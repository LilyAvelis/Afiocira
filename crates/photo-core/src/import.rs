//! Import files without replacing an existing destination or modifying originals.
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
fn is_link(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}
fn destination_folder(folder: &Path) -> io::Result<PathBuf> {
    let metadata = fs::symlink_metadata(folder)?;
    if is_link(&metadata) || !metadata.is_dir() {
        return Err(invalid(
            "Destination must be a directory, not a symbolic link",
        ));
    }
    fs::canonicalize(folder)
}
fn validate_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err(invalid("Filename must be a single safe basename"));
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches(' ')
        .to_uppercase();
    let reserved = matches!(
        base.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        base.strip_prefix(prefix).is_some_and(|n| {
            matches!(
                n,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    });
    if reserved {
        return Err(invalid("Filename is reserved by Windows"));
    }
    Ok(())
}
fn temporary(folder: &Path) -> io::Result<(Temporary, File)> {
    loop {
        let path = folder.join(format!(
            ".afio-import-{}-{}.part",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((Temporary(path), file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}
#[cfg(windows)]
fn publish(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileW(source: *const u16, destination: *const u16) -> i32;
    }
    let source: Vec<_> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<_> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // MoveFileW deliberately omits replacement semantics.
    if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn publish(source: &Path, destination: &Path) -> io::Result<()> {
    fs::hard_link(source, destination)
}
fn finish(temporary: &Temporary, folder: &Path, name: &str) -> io::Result<PathBuf> {
    let path = Path::new(name);
    for count in 0..=10000 {
        let candidate = if count == 0 {
            name.to_owned()
        } else {
            let mut candidate = path.file_stem().unwrap().to_string_lossy().into_owned();
            candidate.push_str(&if count == 1 {
                " - Copy".to_owned()
            } else {
                format!(" - Copy ({count})")
            });
            if let Some(extension) = path.extension() {
                candidate.push('.');
                candidate.push_str(&extension.to_string_lossy());
            }
            candidate
        };
        let destination = folder.join(candidate);
        match publish(&temporary.0, &destination) {
            Ok(()) => return Ok(destination),
            // Windows can report AccessDenied for an existing directory.
            Err(e)
                if e.kind() == io::ErrorKind::AlreadyExists
                    || fs::symlink_metadata(&destination).is_ok() =>
            {
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("No unused import filename found"))
}
/// Copy a regular file into `folder`, using a unique name on conflicts.
/// Windows holds a read-only sharing lock for the entire copy.
pub fn copy_file(source: &Path, folder: &Path) -> io::Result<PathBuf> {
    let before_open = fs::symlink_metadata(source)?;
    if is_link(&before_open) || !before_open.is_file() {
        return Err(invalid(
            "Source must be a regular file, not a symbolic link",
        ));
    }
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("Source has no valid filename"))?;
    validate_name(name)?;
    let folder = destination_folder(folder)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x00200000);
    }
    let mut input = options.open(source)?;
    let before = input.metadata()?;
    if !before.is_file() || is_link(&before) || is_link(&fs::symlink_metadata(source)?) {
        return Err(invalid("Source must be a regular file"));
    }
    let (temporary, mut output) = temporary(&folder)?;
    let copied = io::copy(&mut input, &mut output)?;
    let after = input.metadata()?;
    if copied != before.len()
        || after.len() != before.len()
        || after.modified()? != before.modified()?
    {
        return Err(io::Error::other("Source changed during import"));
    }
    output.sync_all()?;
    drop(output);
    finish(&temporary, &folder, name)
}
/// Atomically write bytes under a safe basename, selecting a unique name if occupied.
pub fn write_bytes(folder: &Path, preferred_name: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    validate_name(preferred_name)?;
    let folder = destination_folder(folder)?;
    let (temporary, mut output) = temporary(&folder)?;
    output.write_all(bytes)?;
    output.sync_all()?;
    drop(output);
    finish(&temporary, &folder, preferred_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "afio-import-test-{}-{}",
                std::process::id(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn no_parts(p: &Path) {
        assert!(
            fs::read_dir(p).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part"))
        );
    }
    #[test]
    fn copies_preserve_conflicts_and_original() {
        let f = Folder::new();
        let source = f.0.join("кадр.jpg");
        fs::write(&source, b"original").unwrap();
        fs::write(f.0.join("кадр - Copy.jpg"), b"existing").unwrap();
        let imported = copy_file(&source, &f.0).unwrap();
        assert_eq!(imported.file_name().unwrap(), "кадр - Copy (2).jpg");
        assert_eq!(fs::read(imported).unwrap(), b"original");
        assert_eq!(fs::read(source).unwrap(), b"original");
        assert_eq!(fs::read(f.0.join("кадр - Copy.jpg")).unwrap(), b"existing");
        no_parts(&f.0);
    }
    #[test]
    fn unicode_bytes_and_free_basename() {
        let f = Folder::new();
        let p = write_bytes(&f.0, "Фото 🌷.png", &[0, 255, 42]).unwrap();
        assert_eq!(p.file_name().unwrap(), "Фото 🌷.png");
        assert_eq!(fs::read(p).unwrap(), [0, 255, 42]);
        let target = f.0.join("target");
        fs::create_dir(&target).unwrap();
        let source = f.0.join("source.jpg");
        fs::write(&source, b"x").unwrap();
        assert_eq!(
            copy_file(&source, &target).unwrap().file_name().unwrap(),
            "source.jpg"
        );
    }
    #[test]
    fn rejects_invalid_inputs_and_cleans_temporary() {
        let f = Folder::new();
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "C:x",
            "NUL.png",
            "com1",
            "LPT².jpg",
            "a.",
            "a ",
            "x\n",
            "a?",
        ] {
            assert!(write_bytes(&f.0, name, b"x").is_err(), "{name:?}");
        }
        assert!(copy_file(&f.0, &f.0).is_err());
        assert!(write_bytes(&f.0.join("missing"), "x", b"x").is_err());
        // Simulate a failure after temporary creation.
        {
            let (_temporary, _output) = temporary(&f.0).unwrap();
        }
        no_parts(&f.0);
    }
    #[test]
    fn concurrent_publication_preserves_every_payload() {
        let f = Folder::new();
        fs::write(f.0.join("image.png"), b"existing").unwrap();
        let jobs: Vec<_> = (0..8u8)
            .map(|n| {
                let p = f.0.clone();
                std::thread::spawn(move || (write_bytes(&p, "image.png", &[n]).unwrap(), n))
            })
            .collect();
        let mut paths = std::collections::BTreeSet::new();
        for job in jobs {
            let (p, n) = job.join().unwrap();
            assert_eq!(fs::read(&p).unwrap(), [n]);
            assert!(paths.insert(p));
        }
        assert_eq!(fs::read(f.0.join("image.png")).unwrap(), b"existing");
        no_parts(&f.0);
    }
    #[test]
    fn directory_conflict_is_preserved() {
        let f = Folder::new();
        fs::create_dir(f.0.join("image.png")).unwrap();
        assert_eq!(
            write_bytes(&f.0, "image.png", b"x")
                .unwrap()
                .file_name()
                .unwrap(),
            "image - Copy.png"
        );
        assert!(f.0.join("image.png").is_dir());
        no_parts(&f.0);
    }
}
