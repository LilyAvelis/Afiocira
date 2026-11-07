//! Duplicate originals without overwriting anything, including on name races.
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SERIAL: AtomicU64 = AtomicU64::new(0);
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
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
    // MoveFileW fails if destination exists. Unlike fs::rename it never replaces.
    if unsafe { MoveFileW(source.as_ptr(), destination.as_ptr()) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn publish(source: &Path, destination: &Path) -> io::Result<()> {
    fs::hard_link(source, destination)?;
    Ok(())
}

pub fn duplicate(source: &Path) -> io::Result<PathBuf> {
    if fs::symlink_metadata(source)?.file_type().is_symlink() {
        return Err(io::Error::other(
            "Duplicating symbolic links is not supported",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    // Hold a read-only sharing lock against writes/replacement while copying.
    let mut input = options.open(source)?;
    let metadata = input.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::other("Source is not a regular file"));
    }
    let folder = source
        .parent()
        .ok_or_else(|| io::Error::other("Missing parent folder"))?;
    let (temporary, mut output) = loop {
        let path = folder.join(format!(
            ".afio-copy-{}-{}.part",
            std::process::id(),
            SERIAL.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => break (Temporary(path), file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let bytes = io::copy(&mut input, &mut output)?;
    let after = input.metadata()?;
    if bytes != metadata.len()
        || after.len() != metadata.len()
        || after.modified()? != metadata.modified()?
    {
        return Err(io::Error::other("Source changed during duplication"));
    }
    output.sync_all()?;
    drop(output);
    let stem = source
        .file_stem()
        .ok_or_else(|| io::Error::other("Missing filename"))?;
    for count in 1..=10000 {
        let mut name = stem.to_os_string();
        name.push(if count == 1 {
            " - Copy".to_owned()
        } else {
            format!(" - Copy ({count})")
        });
        if let Some(extension) = source.extension() {
            name.push(".");
            name.push(extension);
        }
        let destination = folder.join(name);
        match publish(&temporary.0, &destination) {
            Ok(()) => return Ok(destination),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::other("No unused duplicate filename found"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concurrent_duplicates_do_not_overwrite_and_leave_no_partial_files() {
        let folder = std::env::temp_dir().join(format!(
            "afio-duplicates-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&folder).unwrap();
        let source = folder.join("кадр.mp4");
        fs::write(&source, b"original").unwrap();
        fs::write(folder.join("кадр - Copy.mp4"), b"existing").unwrap();
        let jobs: Vec<_> = (0..4)
            .map(|_| {
                let source = source.clone();
                std::thread::spawn(move || duplicate(&source).unwrap())
            })
            .collect();
        let paths: std::collections::BTreeSet<_> =
            jobs.into_iter().map(|j| j.join().unwrap()).collect();
        assert_eq!(paths.len(), 4);
        for path in paths {
            assert_eq!(fs::read(path).unwrap(), b"original");
        }
        assert_eq!(
            fs::read(folder.join("кадр - Copy.mp4")).unwrap(),
            b"existing"
        );
        assert_eq!(fs::read(&source).unwrap(), b"original");
        assert!(
            fs::read_dir(&folder).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".part"))
        );
        assert!(duplicate(&folder.join("missing.mp4")).is_err());
        fs::remove_dir_all(folder).unwrap();
    }
}
