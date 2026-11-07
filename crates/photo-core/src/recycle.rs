//! Recycle-only deletion. No permanent-delete fallback exists.
use crate::rename::{SourceVersion, capture};
use std::{
    io,
    path::{Path, PathBuf},
};
#[cfg(any(windows, test))]
fn confirmed_recycled(status: i32, has_bin_item: bool) -> bool {
    status >= 0 && has_bin_item
}
#[derive(Clone, Debug)]
pub struct Target {
    pub path: PathBuf,
    pub version: Option<SourceVersion>,
    pub error: Option<String>,
}
pub fn capture_target(path: PathBuf) -> Target {
    let (version, error) = match capture(&path) {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(e.to_string())),
    };
    Target {
        path,
        version,
        error,
    }
}
/// Shell deletion is path based: an external path replacement between this
/// final check and Shell processing cannot be excluded. Metadata is not a hash.
pub fn recycle_checked(target: &Target) -> io::Result<()> {
    checked_with(target, |_| native_recycle(target))
}
fn checked_with(target: &Target, backend: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    if let Some(error) = &target.error {
        return Err(io::Error::other(error.clone()));
    }
    if Some(capture(&target.path)?) != target.version {
        return Err(io::Error::other(
            "Source file changed since it was selected",
        ));
    }
    backend(&target.path)
}
#[cfg(not(windows))]
fn native_recycle(_: &Target) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows Recycle Bin is unavailable",
    ))
}
#[cfg(windows)]
fn native_recycle(target: &Target) -> io::Result<()> {
    native::recycle(target).map_err(|e| io::Error::other(e.to_string()))
}
/// Convert extended DOS/UNC paths to Shell spelling without losing code units.
#[cfg(windows)]
fn shell_path(path: &Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    let normalized = if wide.starts_with(&[92, 92, 63, 92, 85, 78, 67, 92]) {
        [vec![92, 92], wide[8..].to_vec()].concat()
    } else if wide.starts_with(&[92, 92, 63, 92])
        && wide.len() >= 7
        && ((65..=90).contains(&wide[4]) || (97..=122).contains(&wide[4]))
        && wide[5] == 58
        && wide[6] == 92
    {
        wide[4..].to_vec()
    } else {
        wide
    };
    PathBuf::from(std::ffi::OsString::from_wide(&normalized))
}
#[cfg(windows)]
#[allow(non_snake_case, unused_variables)]
mod native {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::{Arc, Mutex};
    use windows::{
        Win32::{Foundation::E_ABORT, System::Com::*, UI::Shell::*},
        core::*,
    };
    #[implement(IFileOperationProgressSink)]
    struct Sink {
        result: Arc<Mutex<Option<(HRESULT, bool)>>>,
        target: Target,
    }
    impl IFileOperationProgressSink_Impl for Sink_Impl {
        fn StartOperations(&self) -> Result<()> {
            Ok(())
        }
        fn FinishOperations(&self, hrresult: HRESULT) -> Result<()> {
            Ok(())
        }
        fn PreRenameItem(
            &self,
            dwflags: u32,
            psiitem: Ref<IShellItem>,
            psznewname: &PCWSTR,
        ) -> Result<()> {
            Ok(())
        }
        fn PostRenameItem(
            &self,
            dwflags: u32,
            psiitem: Ref<IShellItem>,
            psznewname: &PCWSTR,
            hrrename: HRESULT,
            psinewlycreated: Ref<IShellItem>,
        ) -> Result<()> {
            Ok(())
        }
        fn PreMoveItem(
            &self,
            dwflags: u32,
            psiitem: Ref<IShellItem>,
            psidestinationfolder: Ref<IShellItem>,
            psznewname: &PCWSTR,
        ) -> Result<()> {
            Ok(())
        }
        fn PostMoveItem(
            &self,
            dwflags: u32,
            psiitem: Ref<IShellItem>,
            psidestinationfolder: Ref<IShellItem>,
            psznewname: &PCWSTR,
            hrmove: HRESULT,
            psinewlycreated: Ref<IShellItem>,
        ) -> Result<()> {
            Ok(())
        }
        fn PreCopyItem(
            &self,
            dwflags: u32,
            psiitem: Ref<IShellItem>,
            psidestinationfolder: Ref<IShellItem>,
            psznewname: &PCWSTR,
        ) -> Result<()> {
            Ok(())
        }
        fn PostCopyItem(
            &self,
            dwflags: u32,
            psiitem: Ref<IShellItem>,
            psidestinationfolder: Ref<IShellItem>,
            psznewname: &PCWSTR,
            hrcopy: HRESULT,
            psinewlycreated: Ref<IShellItem>,
        ) -> Result<()> {
            Ok(())
        }
        fn PreDeleteItem(&self, dwflags: u32, psiitem: Ref<IShellItem>) -> Result<()> {
            if capture(&self.target.path).ok() != self.target.version {
                return Err(Error::new(E_ABORT, "Source changed or unavailable"));
            }
            if dwflags & 0x80 == 0 {
                return Err(Error::new(E_ABORT, "Refusing non-recycle deletion"));
            }
            Ok(())
        }
        fn PostDeleteItem(
            &self,
            dwflags: u32,
            psiitem: Ref<IShellItem>,
            hrdelete: HRESULT,
            psinewlycreated: Ref<IShellItem>,
        ) -> Result<()> {
            // Shell returns the newly created Recycle Bin item for a recycled
            // file. Positive COPYENGINE success statuses are valid receipts;
            // ignored/pending operations without a Bin item are not success.
            *self.result.lock().unwrap() = Some((hrdelete, !psinewlycreated.is_null()));
            Ok(())
        }
        fn PreNewItem(
            &self,
            dwflags: u32,
            psidestinationfolder: Ref<IShellItem>,
            psznewname: &PCWSTR,
        ) -> Result<()> {
            Ok(())
        }
        fn PostNewItem(
            &self,
            dwflags: u32,
            psidestinationfolder: Ref<IShellItem>,
            psznewname: &PCWSTR,
            psztemplatename: &PCWSTR,
            dwfileattributes: u32,
            hrnew: HRESULT,
            psinewitem: Ref<IShellItem>,
        ) -> Result<()> {
            Ok(())
        }
        fn UpdateProgress(&self, iworktotal: u32, iworksofar: u32) -> Result<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> Result<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> Result<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> Result<()> {
            Ok(())
        }
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    fn stage(name: &'static str) -> impl FnOnce(Error) -> Error {
        move |error| Error::new(error.code(), format!("{name}: {error}"))
    }
    pub fn recycle(target: &Target) -> windows::core::Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(stage("CoInitializeEx"))?;
        }
        let _apartment = Apartment;
        let path =
            std::path::absolute(&target.path).map_err(|e| Error::new(E_ABORT, e.to_string()))?;
        let path = shell_path(&path);
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let operation: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)
                    .map_err(stage("CoCreateInstance"))?;
            // RECYCLEONDELETE mandates recycling (Windows 8+). No retry using
            // weaker flags, SHFileOperation or remove_file is ever attempted.
            operation
                .SetOperationFlags(FILEOPERATION_FLAGS(
                    0x00080000 | 0x00100000 | 0x40 | 0x10 | 0x400 | 0x4 | 0x2000,
                ))
                .map_err(stage("SetOperationFlags"))?;
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None)
                .map_err(stage("SHCreateItemFromParsingName"))?;
            let result = Arc::new(Mutex::new(None));
            let sink: IFileOperationProgressSink = Sink {
                result: result.clone(),
                target: target.clone(),
            }
            .into();
            operation
                .DeleteItem(&item, &sink)
                .map_err(stage("DeleteItem"))?;
            operation
                .PerformOperations()
                .map_err(stage("PerformOperations"))?;
            if operation
                .GetAnyOperationsAborted()
                .map_err(stage("GetAnyOperationsAborted"))?
                .as_bool()
            {
                return Err(Error::new(
                    E_ABORT,
                    "Recycle operation was aborted; no permanent fallback",
                ));
            }
            let status = *result.lock().unwrap();
            match status {
                Some((hr, receipt)) if confirmed_recycled(hr.0, receipt) => Ok(()),
                Some((hr, _)) if hr.is_err() => {
                    Err(Error::new(hr, "File was not confirmed recycled"))
                }
                Some(_) => Err(Error::new(
                    E_ABORT,
                    "Shell returned no Recycle Bin item receipt",
                )),
                None => Err(Error::new(E_ABORT, "Shell did not confirm recycling")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    #[test]
    fn recycling_requires_success_and_bin_item_receipt() {
        assert!(confirmed_recycled(0, true));
        assert!(confirmed_recycled(0x00270008, true));
        assert!(!confirmed_recycled(0, false));
        assert!(!confirmed_recycled(0x00270008, false));
        assert!(!confirmed_recycled(0x00270005, false));
        assert!(!confirmed_recycled(0x80004005_u32 as i32, true));
        assert!(!confirmed_recycled(0x80004005_u32 as i32, false));
    }
    #[cfg(windows)]
    #[test]
    fn shell_names_preserve_unicode_and_surrogates() {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        for (source, expected) in [
            (r"\\?\C:\Фото\日本語.jpg", r"C:\Фото\日本語.jpg"),
            (r"\\?\UNC\server\share\Фото.mp4", r"\\server\share\Фото.mp4"),
            (r"C:\Фото\a.jpg", r"C:\Фото\a.jpg"),
            (r"\\server\share\a.jpg", r"\\server\share\a.jpg"),
            (r"\\?\Volume{abc}\a.jpg", r"\\?\Volume{abc}\a.jpg"),
        ] {
            assert_eq!(shell_path(Path::new(source)), PathBuf::from(expected));
        }
        let mut source: Vec<u16> = r"\\?\C:\test\".encode_utf16().collect();
        source.extend([0xD800, 46, 106, 112, 103]);
        let normalized = shell_path(Path::new(&std::ffi::OsString::from_wide(&source)));
        assert_eq!(
            normalized.as_os_str().encode_wide().collect::<Vec<_>>(),
            source[4..]
        );
    }
    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "photo-recycle-mock-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                SERIAL.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn file(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, b"synthetic").unwrap();
            path
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            for item in fs::read_dir(&self.0).unwrap() {
                fs::remove_file(item.unwrap().path()).unwrap();
            }
            fs::remove_dir(&self.0).unwrap();
        }
    }
    #[test]
    fn changed_replaced_and_initially_missing_never_reach_backend() {
        let folder = Folder::new();
        let path = folder.file("changed.jpg");
        let changed = capture_target(path.clone());
        fs::write(&path, b"changed synthetic contents").unwrap();
        assert!(checked_with(&changed, |_| panic!("changed source reached backend")).is_err());
        let replaced = capture_target(path.clone());
        fs::rename(&path, folder.0.join("saved.jpg")).unwrap();
        fs::write(&path, b"replacement").unwrap();
        assert!(checked_with(&replaced, |_| panic!("replaced source reached backend")).is_err());
        let missing = capture_target(folder.0.join("missing.jpg"));
        folder.file("missing.jpg");
        assert!(checked_with(&missing, |_| panic!("new source reached backend")).is_err());
        let directory = capture_target(folder.0.clone());
        assert!(checked_with(&directory, |_| panic!("directory reached backend")).is_err());
    }
    #[test]
    fn partial_results_preserve_failures_and_do_not_retry_permanently() {
        let folder = Folder::new();
        let targets = [
            capture_target(folder.file("a.jpg")),
            capture_target(folder.file("b.mp4")),
        ];
        let mut calls = 0;
        let results: Vec<_> = targets
            .iter()
            .map(|target| {
                checked_with(target, |path| {
                    calls += 1;
                    if path.ends_with("b.mp4") {
                        Err(io::Error::other("Recycle Bin unavailable"))
                    } else {
                        Ok(())
                    }
                })
            })
            .collect();
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
        assert_eq!(calls, 2);
        assert!(targets.iter().all(|t| t.path.exists()));
    }
}
