//! Collection identity and captured membership, independent of folder topology.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SelectionScope {
    collection: u64,
    members: BTreeSet<PathBuf>,
}

impl SelectionScope {
    pub(crate) fn new(collection: u64, members: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            collection,
            members: members.into_iter().collect(),
        }
    }

    /// A captured action belongs to this collection, not merely its root path.
    /// Moving the cursor, changing marks, or discovering more members is safe.
    pub fn permits<'a>(&self, collection: u64, paths: impl IntoIterator<Item = &'a Path>) -> bool {
        self.collection == collection && paths.into_iter().all(|path| self.members.contains(path))
    }

    pub fn members(&self) -> &BTreeSet<PathBuf> {
        &self.members
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_scope_accepts_descendants_but_never_another_collection_or_unknown_path() {
        let a = PathBuf::from("root/album-a/a.png");
        let b = PathBuf::from("root/album-b/b.png");
        let scope = SelectionScope::new(7, [a.clone(), b.clone()]);
        assert!(scope.permits(7, [a.as_path(), b.as_path()]));
        assert!(scope.permits(7, [b.as_path()]));
        assert!(!scope.permits(8, [a.as_path()]));
        assert!(!scope.permits(7, [Path::new("root/album-c/c.png")]));
    }

    #[test]
    fn gallery_scope_survives_refresh_and_cursor_changes_but_not_reopening_same_folder() {
        let root = std::env::temp_dir().join(format!(
            "afio-scope-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(root.join("album-a")).unwrap();
        std::fs::create_dir_all(root.join("album-b")).unwrap();
        let a = root.join("album-a/a.png");
        let b = root.join("album-b/b.png");
        std::fs::write(&a, b"synthetic").unwrap();
        std::fs::write(&b, b"synthetic").unwrap();
        let a = std::fs::canonicalize(a).unwrap();
        let b = std::fs::canonicalize(b).unwrap();
        let mut gallery = crate::Gallery::open(&root).unwrap();
        gallery.include_files(&[a.clone(), b.clone()]);
        let captured = gallery.selection_scope();
        gallery.set_marked_paths([a.clone(), b.clone()]);
        gallery.select_path(&b);
        gallery.set_marked_paths([b.clone()]);
        gallery.refresh().unwrap();
        assert!(captured.permits(gallery.collection_id(), [a.as_path(), b.as_path()]));
        let reopened = crate::Gallery::open(&root).unwrap();
        assert!(!captured.permits(reopened.collection_id(), [a.as_path()]));
        std::fs::remove_file(a).unwrap();
        std::fs::remove_file(b).unwrap();
        std::fs::remove_dir(root.join("album-a")).unwrap();
        std::fs::remove_dir(root.join("album-b")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
