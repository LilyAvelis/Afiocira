//! Rename a regular file in its current directory without replacing another file.
#[cfg(any(not(windows), test))]
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

fn validate_name(name: &str) -> io::Result<()> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "Invalid Windows file name");
    if name.is_empty()
        || name.encode_utf16().count() > 255
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err(invalid());
    }
    // Device names remain reserved with an extension, and Windows ignores
    // spaces immediately before that extension when recognizing these names.
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_uppercase();
    let reserved = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if reserved { Err(invalid()) } else { Ok(()) }
}

/// Identity and observable content version captured before a rename request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceVersion {
    identity: (u64, u64),
    size: u64,
    modified: u128,
}
impl SourceVersion {
    /// Exact observable fingerprint, not a logical image ID or content hash.
    /// Binary integers avoid truncating unsigned Windows identities in SQLite.
    pub fn fingerprint(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(40);
        bytes.extend_from_slice(&self.identity.0.to_le_bytes());
        bytes.extend_from_slice(&self.identity.1.to_le_bytes());
        bytes.extend_from_slice(&self.size.to_le_bytes());
        bytes.extend_from_slice(&self.modified.to_le_bytes());
        bytes
    }
}
fn changed() -> io::Error {
    io::Error::other("Source file changed since it was selected")
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    type Handle = *mut c_void;
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct Information {
        attributes: u32,
        creation: FileTime,
        access: FileTime,
        write: FileTime,
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[repr(C)]
    struct RenameInformation {
        replace: u8,
        root: Handle,
        length: u32,
        name: [u16; 1],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *mut c_void,
            disposition: u32,
            flags: u32,
            template: Handle,
        ) -> Handle;
        fn GetFileInformationByHandle(handle: Handle, information: *mut Information) -> i32;
        fn SetFileInformationByHandle(
            handle: Handle,
            class: u32,
            information: *const c_void,
            size: u32,
        ) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    pub(super) struct File(Handle);
    impl Drop for File {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    impl File {
        pub(super) fn open_for_capture(path: &Path) -> io::Result<Self> {
            // Reading metadata must coexist with a decoder that denies deletion.
            Self::open(path, 0x80, 1 | 2 | 4)
        }
        pub(super) fn open_for_rename(path: &Path) -> io::Result<Self> {
            // DELETE | FILE_READ_ATTRIBUTES, sharing only reads: writers and
            // path replacements are excluded until this handle is renamed.
            Self::open(path, 0x10000 | 0x80, 1)
        }
        fn open(path: &Path, access: u32, share: u32) -> io::Result<Self> {
            let path: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            // OPEN_REPARSE_POINT lets us reject links instead of following them.
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    access,
                    share,
                    std::ptr::null_mut(),
                    3,
                    0x00200000 | 0x02000000,
                    std::ptr::null_mut(),
                )
            };
            if handle as isize == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(handle))
            }
        }
        pub(super) fn version(&self) -> io::Result<SourceVersion> {
            let mut info = Information::default();
            if unsafe { GetFileInformationByHandle(self.0, &mut info) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if info.attributes & (0x10 | 0x400) != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Source must be a regular file",
                ));
            }
            Ok(SourceVersion {
                identity: (
                    u64::from(info.volume),
                    (u64::from(info.index_high) << 32) | u64::from(info.index_low),
                ),
                size: (u64::from(info.size_high) << 32) | u64::from(info.size_low),
                modified: u128::from(
                    (u64::from(info.write.high) << 32) | u64::from(info.write.low),
                ),
            })
        }
        pub(super) fn rename(&self, destination: &Path) -> io::Result<()> {
            let destination = std::path::absolute(destination)?;
            // Win32 may inspect FileName as a NUL-terminated path even though
            // FileNameLength excludes the terminator. Always own that extra unit;
            // reading allocator padding made success depend on path alignment.
            let name: Vec<u16> = destination
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let offset = std::mem::offset_of!(RenameInformation, name);
            let size = offset.checked_add(name.len() * 2).ok_or_else(changed)?;
            let size32 = u32::try_from(size).map_err(|_| changed())?;
            // usize storage provides native structure alignment.
            let mut buffer = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
            let info = buffer.as_mut_ptr().cast::<RenameInformation>();
            unsafe {
                (*info).replace = 0;
                (*info).root = std::ptr::null_mut();
                (*info).length = ((name.len() - 1) * 2) as u32;
                std::ptr::copy_nonoverlapping(
                    name.as_ptr(),
                    buffer.as_mut_ptr().cast::<u8>().add(offset).cast::<u16>(),
                    name.len(),
                );
                if SetFileInformationByHandle(self.0, 3, info.cast(), size32) == 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        }
    }
}

/// Capture a regular file's identity, size and last-write timestamp.
pub fn capture(source: &Path) -> io::Result<SourceVersion> {
    #[cfg(windows)]
    {
        native::File::open_for_capture(source)?.version()
    }
    #[cfg(not(windows))]
    {
        let metadata = fs::symlink_metadata(source)?;
        if !metadata.file_type().is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Source must be a regular file",
            ));
        }
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            (metadata.dev(), metadata.ino())
        };
        #[cfg(not(unix))]
        let identity = (0, 0);
        Ok(SourceVersion {
            identity,
            size: metadata.len(),
            modified: metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        })
    }
}

/// Rename a previously captured regular file without replacing a destination.
/// Windows verifies and renames the same locked handle. Other platforms compare
/// metadata first, but retain a path replacement race between checking and linking.
/// Size and last-write time detect ordinary edits, not deliberately restored metadata.
pub fn rename_checked(
    source: &Path,
    new_name: &str,
    expected: &SourceVersion,
) -> io::Result<PathBuf> {
    validate_name(new_name)?;
    #[cfg(windows)]
    let file = native::File::open_for_rename(source)?;
    #[cfg(windows)]
    let actual = file.version()?;
    #[cfg(not(windows))]
    let actual = capture(source)?;
    if &actual != expected {
        return Err(changed());
    }
    let original_name = source
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Source has no file name"))?;
    if original_name == new_name {
        return Ok(source.to_path_buf());
    }
    let parent = source.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "Source has no parent folder")
    })?;
    let destination = parent.join(new_name);
    #[cfg(windows)]
    file.rename(&destination)?;
    #[cfg(not(windows))]
    {
        fs::hard_link(source, &destination)?;
        fs::remove_file(source)?;
    }
    Ok(destination)
}

/// Rename a regular file to a single Windows-compatible name in its folder.
pub fn rename(source: &Path, new_name: &str) -> io::Result<PathBuf> {
    validate_name(new_name)?;
    rename_checked(source, new_name, &capture(source)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            loop {
                let path = std::env::temp_dir().join(format!(
                    "photo-rename-test-{}-{}",
                    std::process::id(),
                    SERIAL.fetch_add(1, Ordering::Relaxed)
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("{error}"),
                }
            }
        }
        fn file(&self, name: &str, content: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, content).unwrap();
            path
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn capture_coexists_with_reader_but_rename_waits_for_release() {
        use std::os::windows::fs::OpenOptionsExt;
        let folder = Folder::new();
        let source = folder.file("source.jpg", b"original");
        let reader = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&source)
            .unwrap();
        let version = capture(&source).unwrap();
        assert_eq!(
            rename_checked(&source, "new.jpg", &version)
                .unwrap_err()
                .raw_os_error(),
            Some(32)
        );
        assert_eq!(fs::read(&source).unwrap(), b"original");
        drop(reader);
        let destination = rename_checked(&source, "new.jpg", &version).unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"original");
    }

    #[cfg(windows)]
    #[test]
    fn destination_names_across_native_buffer_alignment_preserve_bytes() {
        let folder = Folder::new();
        for length in 0..16 {
            let source = folder.file(&format!("source-{length}.jpg"), b"alignment fixture");
            let destination = rename(&source, &format!("new{}.jpg", "x".repeat(length))).unwrap();
            assert_eq!(fs::read(destination).unwrap(), b"alignment fixture");
            assert!(!source.exists());
        }
    }

    #[test]
    fn replaced_source_is_rejected() {
        let folder = Folder::new();
        let source = folder.file("source.jpg", b"original");
        let version = capture(&source).unwrap();
        fs::rename(&source, folder.0.join("saved.jpg")).unwrap();
        fs::write(&source, b"replacement").unwrap();
        assert!(rename_checked(&source, "new.jpg", &version).is_err());
        assert_eq!(fs::read(source).unwrap(), b"replacement");
        assert!(!folder.0.join("new.jpg").exists());
    }

    #[test]
    fn edited_source_is_rejected_even_for_noop() {
        let folder = Folder::new();
        let source = folder.file("source.jpg", b"original");
        let version = capture(&source).unwrap();
        fs::write(&source, b"edited content with different size").unwrap();
        assert!(rename_checked(&source, "source.jpg", &version).is_err());
        assert!(rename_checked(&source, "new.jpg", &version).is_err());
        assert!(!folder.0.join("new.jpg").exists());
    }

    #[test]
    fn unicode_and_spaces_preserve_contents_and_folder() {
        let folder = Folder::new();
        let source = folder.file("source.jpg", b"original bytes");
        let destination = rename(&source, "Фото отпуск 01.jpg").unwrap();
        assert_eq!(destination, folder.0.join("Фото отпуск 01.jpg"));
        assert_eq!(fs::read(destination).unwrap(), b"original bytes");
        assert!(!source.exists());
    }

    #[test]
    fn collision_preserves_both_files() {
        let folder = Folder::new();
        let source = folder.file("source.jpg", b"source");
        let target = folder.file("target.jpg", b"target");
        assert!(rename(&source, "target.jpg").is_err());
        assert_eq!(fs::read(source).unwrap(), b"source");
        assert_eq!(fs::read(target).unwrap(), b"target");
    }

    #[test]
    fn concurrent_destination_race_has_one_winner_and_preserves_losers() {
        let folder = Folder::new();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8u8)
            .map(|index| {
                let source = folder.file(&format!("source-{index}.jpg"), &[index]);
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let result = rename(&source, "winner.jpg");
                    if result.is_err() {
                        assert_eq!(fs::read(&source).unwrap(), [index]);
                    } else {
                        assert!(!source.exists());
                    }
                    (index, result.is_ok())
                })
            })
            .collect();
        let winners: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|(_, won)| *won)
            .collect();
        assert_eq!(winners.len(), 1);
        assert_eq!(
            fs::read(folder.0.join("winner.jpg")).unwrap(),
            [winners[0].0]
        );
    }

    #[test]
    fn invalid_names_cannot_escape_or_change_original() {
        let folder = Folder::new();
        let source = folder.file("source.jpg", b"source");
        for name in [
            "",
            ".",
            "..",
            "../escape.jpg",
            "..\\escape.jpg",
            "/escape.jpg",
            "C:\\escape.jpg",
            "file/child",
            "bad\0.jpg",
            "bad\n.jpg",
            "file.",
            "file ",
            "a:b",
            "a?b",
            "a*b",
            "a|b",
            "a<b",
            "a>b",
            "a\"b",
            "CON",
            "nul.jpg",
            "COM1.jpg",
            "LPT9",
            "COM¹.txt",
            "CON .jpg",
            "CONOUT$",
        ] {
            assert_eq!(
                rename(&source, name).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{name:?}"
            );
            assert_eq!(fs::read(&source).unwrap(), b"source");
        }
        assert!(rename(&source, &"😀".repeat(128)).is_err());
    }

    #[test]
    fn exact_name_is_noop() {
        let folder = Folder::new();
        let source = folder.file("source.jpg", b"source");
        assert_eq!(rename(&source, "source.jpg").unwrap(), source);
        assert_eq!(fs::read(source).unwrap(), b"source");
    }

    #[test]
    fn directories_and_missing_sources_are_rejected() {
        let folder = Folder::new();
        assert!(rename(&folder.0, "new-folder").is_err());
        assert_eq!(
            rename(&folder.0.join("missing.jpg"), "new.jpg")
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn case_only_rename_preserves_contents() {
        let folder = Folder::new();
        let source = folder.file("Photo.jpg", b"photo");
        let destination = rename(&source, "PHOTO.jpg").unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"photo");
        assert!(
            fs::read_dir(&folder.0)
                .unwrap()
                .any(|entry| entry.unwrap().file_name() == "PHOTO.jpg")
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_source_is_rejected() {
        let folder = Folder::new();
        let target = folder.file("target.jpg", b"target");
        let source = folder.0.join("link.jpg");
        std::os::unix::fs::symlink(&target, &source).unwrap();
        assert!(rename(&source, "new.jpg").is_err());
        assert!(
            fs::symlink_metadata(source)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(target).unwrap(), b"target");
    }
}
