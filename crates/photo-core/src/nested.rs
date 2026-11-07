//! Demand-driven directory exploration. No recursive call and no image decoding.
use crate::{Photo, is_media_path};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs, io,
    path::{Path, PathBuf},
};
const MAX_FOLDERS: usize = 512;
const MAX_IMAGES: usize = 2000;
struct Section {
    path: PathBuf,
    depth: usize,
    reader: Option<fs::ReadDir>,
}
pub struct Explorer {
    queue: VecDeque<Section>,
    deeper: VecDeque<Section>,
    pub subfolders: usize,
    pub depth: usize,
    pub visited: usize,
    pub loaded: usize,
    pub limited: bool,
    pub warnings: Vec<String>,
    discovered: usize,
    known: HashSet<PathBuf>,
    albums: BTreeMap<PathBuf, crate::api::AlbumState>,
}
impl Explorer {
    pub fn new(root: &Path, cancelled: impl Fn() -> bool) -> io::Result<Self> {
        let mut this = Self {
            queue: VecDeque::new(),
            deeper: VecDeque::new(),
            subfolders: 0,
            depth: 1,
            visited: 0,
            loaded: 0,
            limited: false,
            warnings: Vec::new(),
            discovered: 0,
            known: HashSet::new(),
            albums: BTreeMap::new(),
        };
        for entry in fs::read_dir(root)? {
            if cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Navigation changed",
                ));
            }
            let entry = entry?;
            if directory(&entry)? {
                this.subfolders += 1;
                this.enqueue(entry.path(), 1);
            }
        }
        Ok(this)
    }
    fn enqueue(&mut self, path: PathBuf, depth: usize) {
        if depth > 8 {
            self.limited = true;
            return;
        }
        if self.discovered >= MAX_FOLDERS {
            self.limited = true;
            return;
        }
        if !self.known.insert(path.clone()) {
            return;
        }
        self.discovered += 1;
        self.albums.insert(
            path.clone(),
            crate::api::AlbumState {
                folder: path.clone(),
                depth,
                loaded: 0,
                exhausted: false,
                children: 0,
                error: None,
            },
        );
        let section = Section {
            path,
            depth,
            reader: None,
        };
        if depth <= self.depth {
            self.queue.push_back(section);
        } else {
            self.deeper.push_back(section);
        }
    }
    pub fn recount(&mut self, root: &Path, cancelled: impl Fn() -> bool) -> io::Result<()> {
        let mut count = 0;
        for entry in fs::read_dir(root)? {
            if cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Navigation changed",
                ));
            }
            let entry = entry?;
            if directory(&entry)? {
                count += 1;
                self.enqueue(entry.path(), 1);
            }
        }
        self.subfolders = count;
        Ok(())
    }
    pub fn albums(&self) -> Vec<crate::api::AlbumState> {
        self.albums.values().cloned().collect()
    }
    /// Continue this album only. Other directory iterators are never advanced.
    pub fn more_album(&mut self, folder: &Path, cancelled: impl Fn() -> bool) -> Vec<Photo> {
        let section = if let Some(i) = self.queue.iter().position(|s| s.path == folder) {
            self.queue.remove(i)
        } else if let Some(i) = self.deeper.iter().position(|s| s.path == folder) {
            self.deeper.remove(i)
        } else {
            None
        };
        let Some(section) = section else {
            return Vec::new();
        };
        self.depth = self.depth.max(section.depth);
        let others = std::mem::take(&mut self.queue);
        self.queue.push_back(section);
        let photos = self.more(false, cancelled);
        self.queue.extend(others);
        photos
    }
    pub fn can_more(&self) -> bool {
        !self.queue.is_empty() && self.loaded < MAX_IMAGES
    }
    pub fn can_deepen(&self) -> bool {
        !self.deeper.is_empty() && self.depth < 8 && self.loaded < MAX_IMAGES
    }
    pub fn more(&mut self, deepen: bool, cancelled: impl Fn() -> bool) -> Vec<Photo> {
        if deepen && self.queue.is_empty() && self.can_deepen() {
            self.depth += 1;
            let mut rest = VecDeque::new();
            while let Some(section) = self.deeper.pop_front() {
                if section.depth <= self.depth {
                    self.queue.push_back(section);
                } else {
                    rest.push_back(section);
                }
            }
            self.deeper = rest;
        }
        let mut photos = Vec::new();
        let mut scanned = 0;
        let mut sections = 0;
        let section_budget = self.queue.len().min(8);
        while scanned < 512
            && photos.len() < 25
            && sections < section_budget
            && self.loaded < MAX_IMAGES
            && !cancelled()
        {
            let Some(mut section) = self.queue.pop_front() else {
                break;
            };
            sections += 1;
            if section.reader.is_none() {
                match fs::read_dir(&section.path) {
                    Ok(reader) => {
                        section.reader = Some(reader);
                        self.visited += 1;
                    }
                    Err(e) => {
                        if let Some(album) = self.albums.get_mut(&section.path) {
                            album.exhausted = true;
                            album.error = Some(e.to_string());
                        }
                        self.warn(format!("{}: {e}", section.path.display()));
                        continue;
                    }
                }
            }
            let mut in_section = 0;
            let mut exhausted = false;
            while scanned < 512
                && in_section < 5
                && photos.len() < 25
                && self.loaded < MAX_IMAGES
                && !cancelled()
            {
                let Some(entry) = section.reader.as_mut().unwrap().next() else {
                    exhausted = true;
                    break;
                };
                scanned += 1;
                let entry = match entry {
                    Ok(e) => e,
                    Err(e) => {
                        self.warn(e.to_string());
                        continue;
                    }
                };
                match directory(&entry) {
                    Ok(true) => {
                        if let Some(album) = self.albums.get_mut(&section.path) {
                            album.children += 1;
                        }
                        self.enqueue(entry.path(), section.depth + 1);
                    }
                    Ok(false) => {
                        let path = entry.path();
                        if entry.file_type().is_ok_and(|t| t.is_file()) && is_media_path(&path) {
                            photos.push(Photo {
                                name: entry.file_name().to_string_lossy().into_owned(),
                                extension: path
                                    .extension()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_ascii_lowercase(),
                                path,
                            });
                            in_section += 1;
                            self.loaded += 1;
                        }
                    }
                    Err(e) => self.warn(e.to_string()),
                }
            }
            if let Some(album) = self.albums.get_mut(&section.path) {
                album.loaded += in_section;
                album.exhausted = exhausted;
            }
            if !exhausted {
                self.queue.push_back(section);
            }
        }
        if self.depth >= 8 && !self.deeper.is_empty() {
            self.limited = true;
        }
        if self.loaded >= MAX_IMAGES {
            self.limited = true;
            self.queue.clear();
            self.deeper.clear();
        }
        photos
    }
    fn warn(&mut self, text: String) {
        if self.warnings.len() < 8 {
            self.warnings.push(text);
        }
    }
}
fn directory(entry: &fs::DirEntry) -> io::Result<bool> {
    let ty = entry.file_type()?;
    if !ty.is_dir() || ty.is_symlink() {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if entry.metadata()?.file_attributes() & 0x400 != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuing_one_album_never_advances_its_neighbour() {
        let root =
            std::env::temp_dir().join(format!("afio-album-isolation-{}", std::process::id()));
        for folder in ["a", "b"] {
            fs::create_dir_all(root.join(folder)).unwrap();
            for i in 0..20 {
                fs::write(root.join(folder).join(format!("{i}.png")), []).unwrap();
            }
        }
        let mut explorer = Explorer::new(&root, || false).unwrap();
        for expected in [5, 10] {
            let page = explorer.more_album(&root.join("a"), || false);
            assert_eq!(page.len(), 5);
            assert!(
                page.iter()
                    .all(|p| p.path.parent() == Some(root.join("a").as_path()))
            );
            let rows = explorer.albums();
            assert_eq!(
                rows.iter()
                    .find(|r| r.folder.ends_with("a"))
                    .unwrap()
                    .loaded,
                expected
            );
            assert_eq!(
                rows.iter()
                    .find(|r| r.folder.ends_with("b"))
                    .unwrap()
                    .loaded,
                0
            );
        }
        assert_eq!(explorer.visited, 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bounded_batches_and_explicit_depth_never_scan_grandchildren_early() {
        let root = std::env::temp_dir().join(format!("afio-nested-{}", std::process::id()));
        fs::create_dir_all(root.join("a/deep")).unwrap();
        for i in 0..20 {
            fs::write(root.join(format!("a/{i}.jpg")), []).unwrap();
        }
        fs::write(root.join("a/deep/deep.png"), []).unwrap();
        let mut e = Explorer::new(&root, || false).unwrap();
        assert_eq!(e.subfolders, 1);
        assert_eq!(e.visited, 0);
        let first = e.more(false, || false);
        assert!(first.len() <= 25);
        assert!(
            first
                .iter()
                .all(|p| p.path.parent() == Some(root.join("a").as_path()))
        );
        while e.can_more() {
            e.more(false, || false);
        }
        assert_eq!(e.loaded, 20);
        assert!(e.can_deepen());
        assert_eq!(e.visited, 1);
        let deeper = e.more(true, || false);
        assert_eq!(deeper.len(), 1);
        assert_eq!(e.depth, 2);
        assert_eq!(e.loaded, 21);
        fs::remove_dir_all(root).unwrap();
    }
}
