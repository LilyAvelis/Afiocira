//! Session-only, disk-backed history. Calling `inspect` never discovers images.
//! Only `truncate_future` explicitly discards the cursor's forward branch.
use photo_core::rename::{self, SourceVersion};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static NEXT: AtomicU64 = AtomicU64::new(1);
fn name() -> String {
    format!(
        "photo-history-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}
fn error(message: &str) -> io::Error {
    io::Error::other(message)
}
fn digest(path: &Path) -> io::Result<[u8; 32]> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options.open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(*hasher.finalize().as_bytes())
}
fn check(path: &Path, expected: &SourceVersion) -> io::Result<()> {
    if rename::capture(path)? != *expected {
        return Err(error("Source changed"));
    }
    Ok(())
}
fn equal(a: &Path, b: &Path) -> io::Result<bool> {
    let (mut a, mut b) = (File::open(a)?, File::open(b)?);
    let (mut x, mut y) = ([0; 65536], [0; 65536]);
    loop {
        let n = a.read(&mut x)?;
        let m = b.read(&mut y)?;
        if n != m || x[..n] != y[..m] {
            return Ok(false);
        }
        if n == 0 {
            return Ok(true);
        }
    }
}
fn lock_backup(path: &Path, expected: &SourceVersion) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.access_mode(0x80000000 | 0x10000).share_mode(1);
    }
    let file = options.open(path)?;
    check(path, expected)?;
    Ok(file)
}
fn remove_locked_backup(file: File, path: &Path, expected: &SourceVersion) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                class: u32,
                information: *const std::ffi::c_void,
                size: u32,
            ) -> i32;
        }
        // Delete the exact locked object, never a replacement at the old path.
        check(path, expected)?;
        let delete: u8 = 1;
        if unsafe {
            SetFileInformationByHandle(file.as_raw_handle(), 4, (&delete as *const u8).cast(), 1)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        drop(file);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        check(path, expected)?;
        drop(file);
        fs::remove_file(path)
    }
}
#[derive(Debug)]
struct Snapshot {
    file: PathBuf,
    original: PathBuf,
    size: u64,
    digest: [u8; 32],
    budget: Arc<AtomicU64>,
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file);
        self.budget.fetch_sub(self.size, Ordering::Relaxed);
    }
}
/// Dropping a ticket after a failed/cancelled operation discards its snapshot.
#[derive(Debug)]
pub struct Ticket {
    before: Snapshot,
    expected: SourceVersion,
    store: PathBuf,
}
#[derive(Debug)]
struct Stack {
    nodes: Vec<Snapshot>,
    cursor: usize,
    live: PathBuf,
    version: SourceVersion,
}
#[derive(Clone, Debug)]
pub struct HistoryNode {
    pub source_path: PathBuf,
    pub bytes: u64,
}
#[derive(Clone, Debug)]
pub struct HistoryView {
    pub versions: Vec<HistoryNode>,
    pub selected: usize,
    pub can_undo: bool,
    pub can_redo: bool,
}
#[derive(Debug)]
pub struct RestoredFile {
    pub path: PathBuf,
    pub version: SourceVersion,
}
pub struct HistoryStore {
    root: PathBuf,
    quota: u64,
    stacks: Vec<Stack>,
    budget: Arc<AtomicU64>,
    recovery: Vec<Snapshot>,
}
impl HistoryStore {
    /// Reserves a unique session path. Directories are created at first capture.
    pub fn new(parent: &Path, quota: u64) -> io::Result<Self> {
        let parent = std::path::absolute(parent)?;
        let root = parent.join(name());
        Ok(Self {
            root,
            quota,
            stacks: Vec::new(),
            budget: Arc::new(AtomicU64::new(0)),
            recovery: Vec::new(),
        })
    }
    fn snapshot(
        &self,
        path: &Path,
        version: &SourceVersion,
        _reserved: u64,
    ) -> io::Result<Snapshot> {
        check(path, version)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut input = options.open(path)?;
        check(path, version)?;
        let size = input.metadata()?.len();
        let original = std::path::absolute(path)?;
        if self
            .budget
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(size).filter(|&n| n <= self.quota)
            })
            .is_err()
        {
            return Err(error("Session history quota exceeded"));
        }
        let mut snapshot = Snapshot {
            file: self.root.join(name()),
            original,
            size,
            digest: [0; 32],
            budget: self.budget.clone(),
        };
        if !self.root.try_exists()? {
            fs::create_dir_all(
                self.root
                    .parent()
                    .ok_or_else(|| error("Missing session parent"))?,
            )?;
            fs::create_dir(&self.root)?;
        }
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&snapshot.file)?;
        let copied = io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        check(path, version)?;
        if copied != size || !equal(path, &snapshot.file)? {
            return Err(error("Source changed during history capture"));
        }
        snapshot.digest = digest(&snapshot.file)?;
        Ok(snapshot)
    }
    pub fn prepare_before(&mut self, path: &Path, expected: &SourceVersion) -> io::Result<Ticket> {
        if (self.stacks.len() >= 256 && self.find(path).is_none())
            || self.stacks.iter().map(|s| s.nodes.len()).sum::<usize>() >= 4096
        {
            return Err(error("Session history count limit exceeded"));
        }
        if self
            .find(path)
            .is_some_and(|s| s.cursor + 1 < s.nodes.len())
        {
            return Err(error(
                "Commit current point (Ctrl+S) before starting a new edit",
            ));
        }
        let before = self.snapshot(path, expected, 0)?;
        if let Some(s) = self.find(path)
            && (s.version != *expected || !equal(path, &s.nodes[s.cursor].file)?)
        {
            return Err(error("History source changed"));
        }
        Ok(Ticket {
            before,
            expected: expected.clone(),
            store: self.root.clone(),
        })
    }
    pub fn commit_after(
        &mut self,
        ticket: Ticket,
        path: &Path,
        result: &SourceVersion,
    ) -> io::Result<()> {
        if ticket.store != self.root {
            return Err(error("Ticket belongs to another session"));
        }
        let capture_after = (|| {
            if std::path::absolute(path)?.parent() != ticket.before.original.parent() {
                return Err(error(
                    "Session history only supports operations within the same directory",
                ));
            }
            self.snapshot(path, result, ticket.before.size)
        })();
        let after = match capture_after {
            Ok(after) => after,
            Err(e) => {
                let file = ticket.before.file.clone();
                self.recovery.push(ticket.before);
                return Err(error(&format!(
                    "Image operation already published; history capture failed: {e}. Original snapshot retained at {}",
                    file.display()
                )));
            }
        };
        let old = ticket.before.original.clone();
        if let Some(index) = self.stacks.iter().position(|s| s.live == old) {
            let s = &mut self.stacks[index];
            if s.version != ticket.expected || s.cursor + 1 < s.nodes.len() {
                let file = ticket.before.file.clone();
                self.recovery.push(ticket.before);
                return Err(error(&format!(
                    "Image operation already published; history changed while edit was running. Original snapshot retained at {}",
                    file.display()
                )));
            }
            s.nodes.push(after);
            s.cursor += 1;
            s.live = std::path::absolute(path)?;
            s.version = result.clone();
        } else {
            self.stacks.push(Stack {
                nodes: vec![ticket.before, after],
                cursor: 1,
                live: std::path::absolute(path)?,
                version: result.clone(),
            });
        }
        Ok(())
    }
    fn find(&self, path: &Path) -> Option<&Stack> {
        let path = std::path::absolute(path).ok()?;
        self.stacks.iter().find(|s| s.live == path)
    }
    pub fn inspect(&self, path: &Path) -> Option<HistoryView> {
        self.find(path).map(|s| HistoryView {
            versions: s
                .nodes
                .iter()
                .map(|n| HistoryNode {
                    source_path: n.original.clone(),
                    bytes: n.size,
                })
                .collect(),
            selected: s.cursor,
            can_undo: s.cursor > 0,
            can_redo: s.cursor + 1 < s.nodes.len(),
        })
    }
    pub fn preview(&self, path: &Path, index: usize) -> Option<PathBuf> {
        let node = self.find(path)?.nodes.get(index)?;
        (digest(&node.file).ok()? == node.digest).then(|| node.file.clone())
    }
    pub fn truncate_future(&mut self, path: &Path) -> bool {
        let Ok(path) = std::path::absolute(path) else {
            return false;
        };
        if let Some(s) = self.stacks.iter_mut().find(|s| s.live == path) {
            s.nodes.truncate(s.cursor + 1);
            true
        } else {
            false
        }
    }
    /// Verify a tracked live image under a deny-write/delete read lock on Windows.
    pub fn validate_current(&self, path: &Path) -> io::Result<()> {
        self.lock_current(path).map(|_| ())
    }
    fn lock_current(&self, path: &Path) -> io::Result<Option<File>> {
        let Some(s) = self.find(path) else {
            return Ok(None);
        };
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let locked = options.open(path)?;
        check(path, &s.version)?;
        if !equal(path, &s.nodes[s.cursor].file)? {
            return Err(error("History source changed"));
        }
        Ok(Some(locked))
    }
    /// Explicitly commit the cursor, refusing to prune externally changed images.
    pub fn commit_current(&mut self, path: &Path) -> io::Result<bool> {
        let _locked = self.lock_current(path)?;
        Ok(self.truncate_future(path))
    }
    /// Rebase an already tracked current node after an excluded operation, without
    /// creating a new history step or discovering an untracked image.
    pub fn amend_current(
        &mut self,
        path: &Path,
        before: &SourceVersion,
        destination: &Path,
        after: &SourceVersion,
    ) -> io::Result<()> {
        let path = std::path::absolute(path)?;
        let Some(index) = self.stacks.iter().position(|s| s.live == path) else {
            return Ok(());
        };
        if self.stacks[index].version != *before {
            return Err(error("History source changed before excluded operation"));
        }
        if std::path::absolute(destination)?.parent() != path.parent() {
            return Err(error(
                "Session history only supports operations within the same directory",
            ));
        }
        let snapshot = self.snapshot(destination, after, 0)?;
        let s = &mut self.stacks[index];
        s.nodes[s.cursor] = snapshot;
        s.live = std::path::absolute(destination)?;
        s.version = after.clone();
        Ok(())
    }
    pub fn undo(&mut self, path: &Path, expected: &SourceVersion) -> io::Result<RestoredFile> {
        self.restore(path, expected, false)
    }
    pub fn redo(&mut self, path: &Path, expected: &SourceVersion) -> io::Result<RestoredFile> {
        self.restore(path, expected, true)
    }
    fn restore(
        &mut self,
        path: &Path,
        expected: &SourceVersion,
        forward: bool,
    ) -> io::Result<RestoredFile> {
        let path = std::path::absolute(path)?;
        let s = self
            .stacks
            .iter_mut()
            .find(|s| s.live == path)
            .ok_or_else(|| error("No session history"))?;
        let target_index = if forward {
            s.cursor.checked_add(1).filter(|&v| v < s.nodes.len())
        } else {
            s.cursor.checked_sub(1)
        }
        .ok_or_else(|| error("No history version in that direction"))?;
        check(&path, expected)?;
        if *expected != s.version || !equal(&path, &s.nodes[s.cursor].file)? {
            return Err(error("History source changed"));
        }
        let target = &s.nodes[target_index];
        if target.original != path && target.original.try_exists()? {
            return Err(error("History destination already exists"));
        }
        let parent = path.parent().ok_or_else(|| error("Missing parent"))?;
        let stage = parent.join(name());
        let mut input = File::open(&target.file)?;
        let mut staged = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&stage)?;
        if let Err(e) = io::copy(&mut input, &mut staged).and_then(|_| staged.sync_all()) {
            drop(staged);
            let _ = fs::remove_file(&stage);
            return Err(e);
        }
        drop(staged);
        let stage_guard = Snapshot {
            file: stage.clone(),
            original: stage.clone(),
            size: 0,
            digest: [0; 32],
            budget: self.budget.clone(),
        };
        if digest(&stage)? != target.digest || !equal(&target.file, &stage)? {
            return Err(error("Staged history bytes failed verification"));
        }
        let backup = rename::rename_checked(&path, &name(), expected)?;
        let mut locked_backup = None;
        let publish = (|| {
            locked_backup = Some(lock_backup(&backup, expected)?);
            if !equal(&backup, &s.nodes[s.cursor].file)? {
                return Err(error("Source changed during restore"));
            }
            let v = rename::capture(&stage)?;
            rename::rename_checked(
                &stage,
                target
                    .original
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| error("Invalid target name"))?,
                &v,
            )?;
            Ok(v)
        })();
        let version = match publish {
            Ok(v) => v,
            Err(e) => {
                drop(locked_backup.take());
                let recovery = rename::rename_checked(
                    &backup,
                    path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                    expected,
                );
                return Err(error(&format!(
                    "Restore failed: {e}; original recovery: {recovery:?}; backup: {}",
                    backup.display()
                )));
            }
        };
        drop(stage_guard);
        s.cursor = target_index;
        s.live = target.original.clone();
        s.version = version.clone();
        // Publication succeeded. Keep a backup on cleanup failure; never misreport a
        // failed transition after the live file and cursor have advanced.
        if let Some(file) = locked_backup {
            let _ = remove_locked_backup(file, &backup, expected);
        }
        Ok(RestoredFile {
            path: s.live.clone(),
            version,
        })
    }
}
impl Drop for HistoryStore {
    fn drop(&mut self) {
        self.stacks.clear();
        self.recovery.clear();
        if fs::symlink_metadata(&self.root).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            let _ = fs::remove_dir(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(name());
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn image(&self) -> PathBuf {
            let p = self.0.join("image.jpg");
            fs::write(&p, b"original exact jpeg bytes").unwrap();
            p
        }
        fn store(&self) -> HistoryStore {
            HistoryStore::new(&self.0.join("sessions"), 1024 * 1024).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn edit(store: &mut HistoryStore, path: &Path, target: &Path, bytes: &[u8]) -> SourceVersion {
        let ticket = store
            .prepare_before(path, &rename::capture(path).unwrap())
            .unwrap();
        fs::remove_file(path).unwrap();
        fs::write(target, bytes).unwrap();
        let version = rename::capture(target).unwrap();
        store.commit_after(ticket, target, &version).unwrap();
        version
    }
    #[test]
    fn conversion_undo_redo_restore_exact_bytes_and_extensions() {
        let f = Fixture::new();
        let jpg = f.image();
        let png = f.0.join("image.png");
        let mut h = f.store();
        let version = edit(&mut h, &jpg, &png, b"exact png bytes");
        let undone = h.undo(&png, &version).unwrap();
        assert_eq!(undone.path, jpg);
        assert!(!png.exists());
        assert_eq!(fs::read(&jpg).unwrap(), b"original exact jpeg bytes");
        let redone = h.redo(&jpg, &undone.version).unwrap();
        assert_eq!(redone.path, png);
        assert!(!jpg.exists());
        assert_eq!(fs::read(&png).unwrap(), b"exact png bytes");
    }
    #[test]
    fn inspect_is_lazy_and_drop_removes_session_files() {
        let f = Fixture::new();
        let jpg = f.image();
        let mut h = f.store();
        let root = h.root.clone();
        assert!(h.inspect(&jpg).is_none());
        assert!(!root.exists());
        let v = edit(&mut h, &jpg, &jpg, b"edited");
        assert!(h.inspect(&jpg).unwrap().can_undo);
        assert_eq!(h.inspect(&jpg).unwrap().versions.len(), 2);
        assert_eq!(
            fs::read(h.preview(&jpg, 0).unwrap()).unwrap(),
            b"original exact jpeg bytes"
        );
        assert!(h.preview(&jpg, 99).is_none());
        h.undo(&jpg, &v).unwrap();
        drop(h);
        assert!(!root.exists());
    }
    #[test]
    fn undo_conflict_does_not_clobber_or_advance() {
        let f = Fixture::new();
        let jpg = f.image();
        let png = f.0.join("image.png");
        let mut h = f.store();
        let v = edit(&mut h, &jpg, &png, b"png");
        fs::write(&jpg, b"foreign").unwrap();
        assert!(h.undo(&png, &v).is_err());
        assert_eq!(fs::read(&jpg).unwrap(), b"foreign");
        assert_eq!(fs::read(&png).unwrap(), b"png");
        assert_eq!(h.inspect(&png).unwrap().selected, 1);
    }
    #[test]
    fn source_change_rejected_without_cursor_change() {
        let f = Fixture::new();
        let jpg = f.image();
        let mut h = f.store();
        let v = edit(&mut h, &jpg, &jpg, b"edited");
        fs::write(&jpg, b"external modification").unwrap();
        assert!(h.undo(&jpg, &v).is_err());
        assert_eq!(h.inspect(&jpg).unwrap().selected, 1);
        assert_eq!(fs::read(&jpg).unwrap(), b"external modification");
    }
    #[test]
    fn new_edit_after_undo_requires_explicit_truncation() {
        let f = Fixture::new();
        let jpg = f.image();
        let mut h = f.store();
        let v = edit(&mut h, &jpg, &jpg, b"edited");
        let undone = h.undo(&jpg, &v).unwrap();
        assert!(
            h.prepare_before(&jpg, &undone.version)
                .unwrap_err()
                .to_string()
                .contains("Ctrl+S")
        );
        assert_eq!(h.inspect(&jpg).unwrap().versions.len(), 2);
        assert!(h.inspect(&jpg).unwrap().can_redo);
        assert!(h.truncate_future(&jpg));
        assert_eq!(h.inspect(&jpg).unwrap().versions.len(), 1);
        edit(&mut h, &jpg, &jpg, b"new edit");
        assert_eq!(h.inspect(&jpg).unwrap().versions.len(), 2);
    }
    #[test]
    fn quota_and_failed_capture_leave_original_and_history_untouched() {
        let f = Fixture::new();
        let jpg = f.image();
        let mut h = HistoryStore::new(&f.0.join("sessions"), 1).unwrap();
        let v = rename::capture(&jpg).unwrap();
        assert!(h.prepare_before(&jpg, &v).is_err());
        assert!(h.inspect(&jpg).is_none());
        assert_eq!(fs::read(&jpg).unwrap(), b"original exact jpeg bytes");
        h.quota = 100;
        let ticket = h.prepare_before(&jpg, &v).unwrap();
        drop(ticket);
        assert_eq!(h.budget.load(Ordering::Relaxed), 0);
        assert!(h.inspect(&jpg).is_none());
        let ticket = h.prepare_before(&jpg, &v).unwrap();
        let wrong = f.0.join("missing.png");
        assert!(h.commit_after(ticket, &wrong, &v).is_err());
        assert!(h.inspect(&jpg).is_none());
        assert_eq!(h.recovery.len(), 1);
        assert_eq!(
            fs::read(&h.recovery[0].file).unwrap(),
            b"original exact jpeg bytes"
        );
    }
    #[test]
    fn pending_tickets_count_toward_quota() {
        let f = Fixture::new();
        let jpg = f.image();
        let size = fs::metadata(&jpg).unwrap().len();
        let mut h = HistoryStore::new(&f.0, size).unwrap();
        let v = rename::capture(&jpg).unwrap();
        let ticket = h.prepare_before(&jpg, &v).unwrap();
        assert!(h.prepare_before(&jpg, &v).is_err());
        drop(ticket);
        assert!(h.prepare_before(&jpg, &v).is_ok());
    }
    #[test]
    fn excluded_change_amends_current_without_new_step() {
        let f = Fixture::new();
        let jpg = f.image();
        let mut h = f.store();
        let before = edit(&mut h, &jpg, &jpg, b"edited");
        fs::write(&jpg, b"rotated edited bytes").unwrap();
        let after = rename::capture(&jpg).unwrap();
        h.amend_current(&jpg, &before, &jpg, &after).unwrap();
        assert_eq!(h.inspect(&jpg).unwrap().versions.len(), 2);
        let undone = h.undo(&jpg, &after).unwrap();
        assert_eq!(fs::read(&jpg).unwrap(), b"original exact jpeg bytes");
        h.redo(&jpg, &undone.version).unwrap();
        assert_eq!(fs::read(&jpg).unwrap(), b"rotated edited bytes");
        let unrelated = f.0.join("untracked.jpg");
        fs::write(&unrelated, b"new").unwrap();
        h.amend_current(
            &unrelated,
            &before,
            &unrelated,
            &rename::capture(&unrelated).unwrap(),
        )
        .unwrap();
        assert!(h.inspect(&unrelated).is_none());
    }
    #[test]
    fn missing_snapshot_refuses_restore_without_stray_staged_file() {
        let f = Fixture::new();
        let jpg = f.image();
        let mut h = f.store();
        let v = edit(&mut h, &jpg, &jpg, b"edited");
        fs::remove_file(h.preview(&jpg, 0).unwrap()).unwrap();
        let entries = || {
            let mut paths: Vec<_> = fs::read_dir(&f.0)
                .unwrap()
                .map(|e| e.unwrap().path())
                .collect();
            paths.sort();
            paths
        };
        let before = entries();
        assert!(h.undo(&jpg, &v).is_err());
        assert_eq!(entries(), before);
        assert_eq!(fs::read(&jpg).unwrap(), b"edited");
        assert_eq!(h.inspect(&jpg).unwrap().selected, 1);
    }
    #[test]
    fn cross_directory_commit_refused_and_original_snapshot_retained() {
        let f = Fixture::new();
        let jpg = f.image();
        let mut h = f.store();
        let before = rename::capture(&jpg).unwrap();
        let ticket = h.prepare_before(&jpg, &before).unwrap();
        let other = f.0.join("other");
        fs::create_dir(&other).unwrap();
        let png = other.join("image.png");
        fs::write(&png, b"published").unwrap();
        let err = h
            .commit_after(ticket, &png, &rename::capture(&png).unwrap())
            .unwrap_err();
        assert!(err.to_string().contains("same directory"));
        assert!(h.inspect(&png).is_none());
        assert_eq!(h.recovery.len(), 1);
        assert_eq!(
            fs::read(&h.recovery[0].file).unwrap(),
            b"original exact jpeg bytes"
        );
    }
    #[cfg(windows)]
    #[test]
    fn backup_lock_denies_writes_and_delete_targets_exact_handle() {
        let f = Fixture::new();
        let jpg = f.image();
        let v = rename::capture(&jpg).unwrap();
        let locked = lock_backup(&jpg, &v).unwrap();
        assert!(OpenOptions::new().write(true).open(&jpg).is_err());
        assert!(fs::remove_file(&jpg).is_err());
        remove_locked_backup(locked, &jpg, &v).unwrap();
        assert!(!jpg.exists());
    }
    #[test]
    fn commit_current_refuses_external_replacement_without_pruning() {
        let f = Fixture::new();
        let jpg = f.image();
        let mut h = f.store();
        let v = edit(&mut h, &jpg, &jpg, b"edited");
        let undone = h.undo(&jpg, &v).unwrap();
        h.validate_current(&jpg).unwrap();
        fs::remove_file(&jpg).unwrap();
        fs::write(&jpg, b"external replacement").unwrap();
        assert!(h.commit_current(&jpg).is_err());
        assert_eq!(h.inspect(&jpg).unwrap().versions.len(), 2);
        assert!(h.inspect(&jpg).unwrap().can_redo);
        assert!(h.undo(&jpg, &undone.version).is_err());
    }
    #[test]
    fn corrupt_older_snapshot_refuses_undo_and_preview() {
        let f = Fixture::new();
        let jpg = f.image();
        let mut h = f.store();
        let v = edit(&mut h, &jpg, &jpg, b"edited");
        let snapshot = h.preview(&jpg, 0).unwrap();
        fs::write(&snapshot, b"corrupted snapshot bytes").unwrap();
        assert!(h.preview(&jpg, 0).is_none());
        assert!(h.undo(&jpg, &v).is_err());
        assert_eq!(fs::read(&jpg).unwrap(), b"edited");
        assert_eq!(h.inspect(&jpg).unwrap().selected, 1);
        let files: Vec<_> = fs::read_dir(&f.0)
            .unwrap()
            .filter_map(|e| {
                let e = e.unwrap();
                e.file_type().unwrap().is_file().then_some(e.path())
            })
            .collect();
        assert_eq!(files, vec![jpg]);
    }
}
