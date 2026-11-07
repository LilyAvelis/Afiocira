use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub mod api;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Photo {
    pub path: PathBuf,
    pub name: String,
    pub extension: String,
}

pub fn list_photos(folder: &Path) -> io::Result<Vec<Photo>> {
    let mut photos = Vec::new();

    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        let extension = extension.to_ascii_lowercase();
        if !matches!(
            extension.as_str(),
            "jpg"
                | "jpeg"
                | "png"
                | "webp"
                | "heic"
                | "heif"
                | "avif"
                | "bmp"
                | "gif"
                | "tif"
                | "tiff"
        ) {
            continue;
        }
        photos.push(Photo {
            name: entry.file_name().to_string_lossy().into_owned(),
            path,
            extension,
        });
    }

    photos.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(photos)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum View {
    Grid,
    Photo,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    fn valid(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomState {
    pub factor: f32,
    pub offset: Point,
}

impl Default for ZoomState {
    fn default() -> Self {
        Self {
            factor: 1.0,
            offset: Point::default(),
        }
    }
}

impl ZoomState {
    pub fn zoom_at(
        &mut self,
        multiplier: f32,
        anchor: Point,
        viewport: Size,
        fitted: Size,
    ) -> bool {
        if !multiplier.is_finite()
            || multiplier <= 0.0
            || !anchor.x.is_finite()
            || !anchor.y.is_finite()
            || !viewport.valid()
            || !fitted.valid()
        {
            return false;
        }
        let old = self.factor;
        self.factor = (old * multiplier).clamp(1.0, 16.0);
        let ratio = self.factor / old;
        let center = Point {
            x: viewport.width / 2.0,
            y: viewport.height / 2.0,
        };
        self.offset.x = anchor.x - center.x - (anchor.x - center.x - self.offset.x) * ratio;
        self.offset.y = anchor.y - center.y - (anchor.y - center.y - self.offset.y) * ratio;
        self.constrain(viewport, fitted);
        true
    }

    pub fn pan_by(&mut self, delta: Point, viewport: Size, fitted: Size) -> bool {
        if !delta.x.is_finite() || !delta.y.is_finite() || !viewport.valid() || !fitted.valid() {
            return false;
        }
        self.offset.x += delta.x;
        self.offset.y += delta.y;
        self.constrain(viewport, fitted);
        true
    }

    pub fn constrain(&mut self, viewport: Size, fitted: Size) {
        if !viewport.valid() || !fitted.valid() {
            return;
        }
        let max_x = ((fitted.width * self.factor - viewport.width) / 2.0).max(0.0);
        let max_y = ((fitted.height * self.factor - viewport.height) / 2.0).max(0.0);
        self.offset.x = self.offset.x.clamp(-max_x, max_x);
        self.offset.y = self.offset.y.clamp(-max_y, max_y);
    }
}

#[derive(Debug)]
pub struct Gallery {
    folder: PathBuf,
    photos: Vec<Photo>,
    view: View,
    selected: Option<usize>,
    grid_anchor: usize,
    range_anchor: usize,
    range_base: Option<BTreeSet<PathBuf>>,
    marked: BTreeSet<PathBuf>,
    external_order: bool,
    zoom: ZoomState,
}

impl Gallery {
    pub fn open(folder: &Path) -> io::Result<Self> {
        let folder = fs::canonicalize(folder)?;
        let photos = list_photos(&folder)?;
        Ok(Self {
            folder,
            photos,
            view: View::Grid,
            selected: None,
            grid_anchor: 0,
            range_anchor: 0,
            range_base: None,
            marked: BTreeSet::new(),
            external_order: false,
            zoom: ZoomState::default(),
        })
    }

    pub fn open_file(file: &Path) -> io::Result<Self> {
        let file = fs::canonicalize(file)?;
        let folder = file.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "photo has no parent folder")
        })?;
        let mut gallery = Self::open(folder)?;
        let index = gallery
            .photos
            .iter()
            .position(|photo| photo.path == file)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsupported image file"))?;
        gallery.open_photo(index);
        gallery.grid_anchor = index;
        Ok(gallery)
    }

    pub fn refresh(&mut self) -> io::Result<bool> {
        let mut photos = list_photos(&self.folder)?;
        if self.external_order {
            let positions: HashMap<_, _> = self
                .photos
                .iter()
                .enumerate()
                .map(|(index, photo)| (&photo.path, index))
                .collect();
            photos.sort_by_key(|photo| positions.get(&photo.path).copied().unwrap_or(usize::MAX));
        }
        if photos == self.photos {
            return Ok(false);
        }
        let selected_path = self.selected_photo().map(|photo| photo.path.clone());
        let anchor_path = self
            .photos
            .get(self.grid_anchor)
            .map(|photo| photo.path.clone());
        self.photos = photos;
        let known: BTreeSet<_> = self.photos.iter().map(|photo| photo.path.clone()).collect();
        self.marked.retain(|path| known.contains(path));
        self.selected = selected_path
            .as_ref()
            .and_then(|path| self.photos.iter().position(|photo| &photo.path == path));
        self.grid_anchor = anchor_path
            .as_ref()
            .and_then(|path| self.photos.iter().position(|photo| &photo.path == path))
            .unwrap_or_else(|| self.grid_anchor.min(self.photos.len().saturating_sub(1)));
        self.range_anchor = self.grid_anchor;
        self.range_base = None;
        if self.view == View::Photo && self.selected.is_none() {
            self.view = View::Grid;
            self.reset_zoom();
        }
        Ok(true)
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    pub fn photos(&self) -> &[Photo] {
        &self.photos
    }

    pub fn view(&self) -> View {
        self.view
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub fn selected_photo(&self) -> Option<&Photo> {
        self.selected.and_then(|index| self.photos.get(index))
    }

    pub fn grid_anchor(&self) -> usize {
        self.grid_anchor
    }

    pub fn marked_paths(&self) -> &BTreeSet<PathBuf> {
        &self.marked
    }

    /// Use the navigator's actual file order, including reverse and random sorts.
    /// Paths not present in the gallery are ignored; newly discovered photos stay at the end
    /// until the navigator reports its next order.
    pub fn set_order(&mut self, paths: &[PathBuf]) -> bool {
        let positions: HashMap<_, _> = self
            .photos
            .iter()
            .enumerate()
            .map(|(index, photo)| (photo.path.clone(), index))
            .collect();
        let mut seen = vec![false; self.photos.len()];
        let mut indices = Vec::with_capacity(self.photos.len());
        for path in paths {
            if let Some(&index) = positions.get(path)
                && !seen[index]
            {
                seen[index] = true;
                indices.push(index);
            }
        }
        indices.extend((0..self.photos.len()).filter(|&index| !seen[index]));
        self.external_order = true;
        if indices.iter().enumerate().all(|(index, &old)| index == old) {
            return false;
        }
        let selected_path = self.selected_photo().map(|photo| photo.path.clone());
        let anchor_path = self
            .photos
            .get(self.grid_anchor)
            .map(|photo| photo.path.clone());
        self.photos = indices
            .into_iter()
            .map(|index| self.photos[index].clone())
            .collect();
        self.selected =
            selected_path.and_then(|path| self.photos.iter().position(|photo| photo.path == path));
        self.grid_anchor = anchor_path
            .and_then(|path| self.photos.iter().position(|photo| photo.path == path))
            .unwrap_or(0);
        self.range_anchor = self.grid_anchor;
        self.range_base = None;
        true
    }

    pub fn cursor_photo(&self) -> Option<&Photo> {
        let index = if self.view == View::Photo {
            self.selected?
        } else {
            self.grid_anchor
        };
        self.photos.get(index)
    }

    pub fn select_path(&mut self, path: &Path) -> bool {
        let Some(index) = self.photos.iter().position(|photo| photo.path == path) else {
            return false;
        };
        self.grid_anchor = index;
        self.range_anchor = index;
        self.range_base = None;
        if self.view == View::Photo {
            self.open_photo(index);
        }
        true
    }

    pub fn set_marked_paths(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        let known: BTreeSet<_> = self.photos.iter().map(|photo| photo.path.clone()).collect();
        self.marked = paths
            .into_iter()
            .filter(|path| known.contains(path))
            .collect();
        self.range_base = None;
    }

    pub fn extend_selection(&mut self, index: usize) -> bool {
        if index >= self.photos.len() {
            return false;
        }
        let (start, end) = if self.range_anchor <= index {
            (self.range_anchor, index)
        } else {
            (index, self.range_anchor)
        };
        let mut marked = self
            .range_base
            .get_or_insert_with(|| self.marked.clone())
            .clone();
        marked.extend(
            self.photos[start..=end]
                .iter()
                .map(|photo| photo.path.clone()),
        );
        self.marked = marked;
        self.grid_anchor = index;
        true
    }

    pub fn zoom(&self) -> ZoomState {
        self.zoom
    }

    pub fn zoom_at(
        &mut self,
        multiplier: f32,
        anchor: Point,
        viewport: Size,
        fitted: Size,
    ) -> bool {
        self.view == View::Photo && self.zoom.zoom_at(multiplier, anchor, viewport, fitted)
    }

    pub fn pan_by(&mut self, delta: Point, viewport: Size, fitted: Size) -> bool {
        self.view == View::Photo && self.zoom.pan_by(delta, viewport, fitted)
    }

    pub fn constrain_zoom(&mut self, viewport: Size, fitted: Size) {
        self.zoom.constrain(viewport, fitted);
    }

    pub fn reset_zoom(&mut self) {
        self.zoom = ZoomState::default();
    }

    pub fn set_grid_anchor(&mut self, index: usize) -> bool {
        if index >= self.photos.len() {
            return false;
        }
        self.grid_anchor = index;
        self.range_anchor = index;
        self.range_base = None;
        true
    }

    pub fn open_photo(&mut self, index: usize) -> bool {
        if index >= self.photos.len() {
            return false;
        }
        self.selected = Some(index);
        self.view = View::Photo;
        self.reset_zoom();
        true
    }

    pub fn next_photo(&mut self) -> bool {
        self.step(1)
    }

    pub fn previous_photo(&mut self) -> bool {
        self.step(-1)
    }

    pub fn back_to_grid(&mut self) {
        self.view = View::Grid;
        self.reset_zoom();
    }

    fn step(&mut self, direction: isize) -> bool {
        if self.view != View::Photo {
            return false;
        }
        let Some(index) = self.selected else {
            return false;
        };
        let Some(next) = index.checked_add_signed(direction) else {
            return false;
        };
        if next >= self.photos.len() {
            return false;
        }
        self.selected = Some(next);
        self.reset_zoom();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returning_to_grid_preserves_scroll_position_and_selection() {
        let photos = (0..3)
            .map(|index| Photo {
                path: PathBuf::from(format!("{index}.jpg")),
                name: format!("{index}.jpg"),
                extension: "jpg".into(),
            })
            .collect();
        let mut gallery = Gallery {
            folder: PathBuf::from("photos"),
            photos,
            view: View::Grid,
            selected: None,
            grid_anchor: 0,
            range_anchor: 0,
            range_base: None,
            marked: BTreeSet::new(),
            external_order: false,
            zoom: ZoomState::default(),
        };

        assert!(gallery.set_grid_anchor(1));
        assert!(gallery.open_photo(1));
        assert!(gallery.next_photo());
        assert!(!gallery.next_photo());
        gallery.back_to_grid();

        assert_eq!(gallery.view(), View::Grid);
        assert_eq!(gallery.grid_anchor(), 1);
        assert_eq!(gallery.selected_photo().unwrap().name, "2.jpg");
        assert!(!gallery.previous_photo());
    }

    #[test]
    fn range_selection_reverses_without_losing_existing_marks() {
        let photos = (0..5)
            .map(|index| Photo {
                path: PathBuf::from(format!("{index}.jpg")),
                name: format!("{index}.jpg"),
                extension: "jpg".into(),
            })
            .collect();
        let mut gallery = Gallery {
            folder: PathBuf::from("photos"),
            photos,
            view: View::Grid,
            selected: None,
            grid_anchor: 0,
            range_anchor: 0,
            range_base: None,
            marked: BTreeSet::new(),
            external_order: false,
            zoom: ZoomState::default(),
        };
        gallery.set_marked_paths([PathBuf::from("4.jpg")]);
        assert!(gallery.set_grid_anchor(1));
        assert!(gallery.extend_selection(3));
        assert_eq!(gallery.marked_paths().len(), 4);
        assert!(gallery.extend_selection(2));
        assert_eq!(
            gallery.marked_paths(),
            &BTreeSet::from([
                PathBuf::from("1.jpg"),
                PathBuf::from("2.jpg"),
                PathBuf::from("4.jpg")
            ])
        );
        assert!(gallery.select_path(Path::new("0.jpg")));
        assert_eq!(gallery.cursor_photo().unwrap().path, PathBuf::from("0.jpg"));
    }

    #[test]
    fn zoom_keeps_the_point_under_cursor() {
        let mut zoom = ZoomState::default();
        let viewport = Size {
            width: 800.0,
            height: 600.0,
        };
        let fitted = viewport;
        assert!(zoom.zoom_at(2.0, Point { x: 600.0, y: 300.0 }, viewport, fitted));
        assert_eq!(zoom.factor, 2.0);
        assert_eq!(zoom.offset, Point { x: -200.0, y: 0.0 });
        assert!(zoom.zoom_at(0.5, Point { x: 600.0, y: 300.0 }, viewport, fitted));
        assert_eq!(zoom, ZoomState::default());
    }

    #[test]
    fn zoom_clamps_pan_and_rejects_invalid_values() {
        let mut zoom = ZoomState::default();
        let viewport = Size {
            width: 800.0,
            height: 600.0,
        };
        let fitted = viewport;
        assert!(!zoom.zoom_at(f32::NAN, Point::default(), viewport, fitted));
        assert!(zoom.zoom_at(2.0, Point { x: 400.0, y: 300.0 }, viewport, fitted));
        assert!(zoom.pan_by(
            Point {
                x: 1000.0,
                y: -1000.0
            },
            viewport,
            fitted
        ));
        assert_eq!(
            zoom.offset,
            Point {
                x: 400.0,
                y: -300.0
            }
        );
    }

    #[test]
    fn refresh_preserves_file_identity_and_handles_external_removal() {
        let folder =
            std::env::temp_dir().join(format!("photo-workshop-refresh-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let first = folder.join("a.jpg");
        let second = folder.join("b.jpg");
        fs::write(&first, []).unwrap();
        fs::write(&second, []).unwrap();
        let mut gallery = Gallery::open(&folder).unwrap();
        gallery.set_grid_anchor(1);
        gallery.open_photo(1);
        fs::write(folder.join("0.jpg"), []).unwrap();
        assert!(gallery.refresh().unwrap());
        assert_eq!(gallery.selected_photo().unwrap().name, "b.jpg");
        assert_eq!(gallery.grid_anchor(), 2);
        fs::remove_file(second).unwrap();
        assert!(gallery.refresh().unwrap());
        assert_eq!(gallery.view(), View::Grid);
        assert_eq!(gallery.selected_index(), None);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn navigator_order_preserves_cursor_marks_and_photo_across_refresh() {
        let folder =
            std::env::temp_dir().join(format!("photo-workshop-order-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        for name in ["a.jpg", "b.jpg", "c.jpg"] {
            fs::write(folder.join(name), []).unwrap();
        }
        let mut gallery = Gallery::open(&folder).unwrap();
        let root = gallery.folder().to_path_buf();
        gallery.set_grid_anchor(1);
        gallery.set_marked_paths([root.join("a.jpg"), root.join("c.jpg")]);
        gallery.open_photo(1);
        assert!(gallery.set_order(&[
            root.join("c.jpg"),
            root.join("c.jpg"),
            root.join("not-a-photo.txt"),
            root.join("b.jpg"),
            root.join("a.jpg"),
        ]));
        assert_eq!(
            gallery
                .photos()
                .iter()
                .map(|photo| photo.name.as_str())
                .collect::<Vec<_>>(),
            ["c.jpg", "b.jpg", "a.jpg"]
        );
        assert_eq!(gallery.selected_photo().unwrap().name, "b.jpg");
        assert_eq!(gallery.cursor_photo().unwrap().name, "b.jpg");
        assert_eq!(gallery.marked_paths().len(), 2);
        fs::write(folder.join("0.jpg"), []).unwrap();
        assert!(gallery.refresh().unwrap());
        assert_eq!(
            gallery
                .photos()
                .iter()
                .map(|photo| photo.name.as_str())
                .collect::<Vec<_>>(),
            ["c.jpg", "b.jpg", "a.jpg", "0.jpg"]
        );
        assert_eq!(gallery.selected_photo().unwrap().name, "b.jpg");
        fs::remove_dir_all(folder).unwrap();
    }
}
