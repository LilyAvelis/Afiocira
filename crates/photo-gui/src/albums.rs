//! Album rows with independent cursors and demand-driven continuation.
use super::*;
use photo_core::api::AlbumState;
#[derive(Clone)]
pub(super) struct Row {
    pub info: AlbumState,
    pub indices: Vec<usize>,
}

fn album_layout(available: f32, spacing: Vec2, scale: Option<f32>) -> GridLayout {
    // Reserve the scrollbar and both album-frame margins before counting cells.
    let width = (available - 32.0).max(1.0);
    let columns = ((width + spacing.x) / (240.0 * scale.unwrap_or(1.0) + spacing.x))
        .floor()
        .max(1.0) as usize;
    let tile_width = scale
        .map(|factor| (TARGET_TILE_WIDTH * factor).min(width))
        .unwrap_or((width - spacing.x * (columns - 1) as f32) / columns as f32)
        .max(1.0);
    GridLayout {
        columns,
        tile_width,
        image_height: (tile_width * 0.75).clamp(64.0, 720.0),
        row_height: 0.0,
    }
}

fn album_window(position: usize, columns: usize) -> std::ops::Range<usize> {
    let start = position / columns * columns;
    start..start.saturating_add(columns)
}

impl Workshop {
    pub(super) fn album_mode(&self) -> bool {
        !self.nested.force_grid && !self.nested.snapshot.albums.is_empty()
    }
    pub(super) fn album_rows(&self) -> Vec<Row> {
        let Some(g) = &self.gallery else {
            return Vec::new();
        };
        let mut groups: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        for (i, p) in g.photos().iter().enumerate() {
            if let Some(folder) = p.path.parent() {
                groups.entry(folder.to_path_buf()).or_default().push(i);
            }
        }
        let mut rows = Vec::new();
        if let Some(indices) = groups.remove(g.folder())
            && !indices.is_empty()
        {
            rows.push(Row {
                info: AlbumState {
                    folder: g.folder().to_path_buf(),
                    depth: 0,
                    loaded: indices.len(),
                    exhausted: true,
                    children: self.nested.snapshot.subfolders.unwrap_or(0),
                    error: None,
                },
                indices,
            });
        }
        for info in &self.nested.snapshot.albums {
            rows.push(Row {
                indices: groups.remove(&info.folder).unwrap_or_default(),
                info: info.clone(),
            });
        }
        rows
    }
    pub(super) fn group_album_members(&mut self) {
        let rows = self.album_rows();
        let Some(gallery) = self.gallery.as_ref() else {
            return;
        };
        let old: Vec<_> = gallery.photos().iter().map(|p| p.path.clone()).collect();
        let order: Vec<_> = rows
            .iter()
            .flat_map(|r| r.indices.iter().map(|&i| gallery.photos()[i].path.clone()))
            .collect();
        if self.gallery.as_mut().unwrap().set_order(&order) {
            self.remap_cached_images(&old);
        }
    }
    pub(super) fn load_album(&mut self, folder: PathBuf) -> Result<(), String> {
        // Resolve only known descriptors, not arbitrary API-supplied directories.
        let folder = std::fs::canonicalize(&folder).map_err(|e| e.to_string())?;
        let info = self
            .nested
            .snapshot
            .albums
            .iter()
            .find(|a| a.folder == folder)
            .ok_or("Нет такого альбома")?;
        if info.exhausted {
            return Ok(());
        }
        if self.nested.snapshot.loading {
            if self.nested.loading_album.as_ref() == Some(&folder) {
                return Ok(());
            }
            self.nested.pending_album = Some(folder);
            return Ok(());
        }
        self.nested
            .sender
            .try_send(super::nested_view::Task::Album(
                self.nested.epoch.load(std::sync::atomic::Ordering::Relaxed),
                folder.clone(),
            ))
            .map_err(|e| e.to_string())?;
        self.nested.loading_album = Some(folder);
        self.nested.snapshot.loading = true;
        self.nested.snapshot.expanded = true;
        Ok(())
    }
    pub(super) fn activate_album(&mut self) {
        let rows = self.album_rows();
        if rows.is_empty() {
            return;
        }
        let folder = self
            .nested
            .cursor
            .clone()
            .filter(|p| rows.iter().any(|r| &r.info.folder == p))
            .unwrap_or_else(|| rows[0].info.folder.clone());
        self.nested.cursor = Some(folder.clone());
        if let Some(row) = rows.iter().find(|r| r.info.folder == folder)
            && row.indices.is_empty()
            && !row.info.exhausted
        {
            let _ = self.load_album(folder);
        }
    }
    pub(super) fn finish_album_request(&mut self, folder: &Path) {
        if self.nested.cursor.as_deref() != Some(folder)
            || self.gallery.as_ref().is_none_or(|g| g.view() != View::Grid)
        {
            return;
        }
        let rows = self.album_rows();
        let Some(row) = rows.iter().find(|r| r.info.folder == folder) else {
            return;
        };
        let pos = self
            .nested
            .positions
            .entry(folder.to_path_buf())
            .or_default();
        if self.nested.advance.as_deref() == Some(folder) {
            if *pos + 1 < row.indices.len() {
                *pos += 1;
            }
            self.nested.advance = None;
        }
        *pos = (*pos).min(row.indices.len().saturating_sub(1));
        if let Some(&index) = row.indices.get(*pos)
            && self
                .gallery
                .as_ref()
                .is_some_and(|g| g.grid_anchor() != index)
        {
            self.execute(Command::Select { index });
        }
    }
    pub(super) fn album_move(&mut self, horizontal: i32, vertical: i32) -> Result<(), String> {
        if !self.album_mode() {
            return Err("Сейчас открыта обычная сетка".into());
        }
        let rows = self.album_rows();
        if rows.is_empty() {
            return Ok(());
        }
        if vertical != 0 || horizontal < 0 {
            self.nested.advance = None;
        }
        let current = rows
            .iter()
            .position(|r| Some(&r.info.folder) == self.nested.cursor.as_ref())
            .unwrap_or(0);
        let target = if vertical < 0 {
            current.saturating_sub(1)
        } else if vertical > 0 {
            (current + 1).min(rows.len() - 1)
        } else {
            current
        };
        let row = &rows[target];
        let folder = row.info.folder.clone();
        self.nested.cursor = Some(folder.clone());
        self.nested.scroll_album = vertical != 0;
        self.scroll_to_grid_anchor = false;
        let pos = self.nested.positions.entry(folder.clone()).or_default();
        if vertical == 0 && horizontal < 0 {
            *pos = pos.saturating_sub(1);
        }
        if vertical == 0 && horizontal > 0 {
            if *pos + 1 < row.indices.len() {
                *pos += 1;
            } else if !row.info.exhausted {
                self.nested.advance = (!row.indices.is_empty()).then(|| folder.clone());
                return self.load_album(folder);
            }
        }
        *pos = (*pos).min(row.indices.len().saturating_sub(1));
        if let Some(&index) = row.indices.get(*pos) {
            self.execute(Command::Select { index });
        } else if !row.info.exhausted {
            let _ = self.load_album(folder);
        }
        Ok(())
    }
    pub(super) fn current_album(&self) -> Option<(PathBuf, Option<PathBuf>)> {
        let rows = self.album_rows();
        let row = rows
            .iter()
            .find(|r| Some(&r.info.folder) == self.nested.cursor.as_ref())
            .or_else(|| rows.first())?;
        let pos = self
            .nested
            .positions
            .get(&row.info.folder)
            .copied()
            .unwrap_or(0);
        let photo = row
            .indices
            .get(pos)
            .and_then(|&i| self.gallery.as_ref()?.photos().get(i))
            .map(|p| p.path.clone());
        Some((row.info.folder.clone(), photo))
    }
    /// Enter opens a real local grid, with the album cursor retained by path.
    pub(super) fn enter_album(&mut self) -> Result<(), String> {
        if !self.album_mode() || self.gallery.as_ref().is_none_or(|g| g.view() != View::Grid) {
            return Err("Выберите альбом в галерее".into());
        }
        let (folder, cursor) = self.current_album().ok_or("Нет выбранного альбома")?;
        let mut gallery = Gallery::open(&folder).map_err(|e| e.to_string())?;
        if let Some(path) = &cursor {
            gallery.select_path(path);
        }
        if let Some(old) = &self.gallery {
            gallery.set_marked_paths(old.marked_paths().iter().cloned());
        }
        let local = !gallery.photos().is_empty();
        self.install_local_gallery(gallery);
        self.nested.force_grid = local;
        self.active_pane = ActivePane::Gallery;
        self.interaction_revision = self.interaction_revision.wrapping_add(1);
        if let Some(yazi) = &mut self.yazi {
            yazi.navigate(cursor.as_deref().unwrap_or(&folder))?;
        }
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        Ok(())
    }
    pub(super) fn album_open(&mut self) {
        let rows = self.album_rows();
        let Some(row) = rows
            .iter()
            .find(|r| Some(&r.info.folder) == self.nested.cursor.as_ref())
        else {
            return;
        };
        let pos = self
            .nested
            .positions
            .get(&row.info.folder)
            .copied()
            .unwrap_or(0);
        if let Some(&index) = row.indices.get(pos) {
            self.execute(Command::Select { index });
            self.execute(Command::OpenSelected);
        } else if !row.info.exhausted {
            let folder = row.info.folder.clone();
            let _ = self.load_album(folder);
        }
    }
    pub(super) fn album_grid(&mut self, ui: &mut egui::Ui) {
        let rows = self.album_rows();
        if rows.is_empty() {
            return;
        }
        if self.scroll_to_grid_anchor
            && let Some(photo) = self.gallery.as_ref().and_then(Gallery::cursor_photo)
            && let Some(folder) = photo.path.parent()
        {
            if let Some(row) = rows.iter().find(|r| r.info.folder == folder) {
                self.nested.cursor = Some(folder.to_path_buf());
                self.nested.positions.insert(
                    folder.to_path_buf(),
                    row.indices
                        .iter()
                        .position(|&i| {
                            self.gallery.as_ref().unwrap().photos()[i].path == photo.path
                        })
                        .unwrap_or(0),
                );
                self.nested.scroll_album = true;
            }
            self.scroll_to_grid_anchor = false;
        }
        if self.nested.cursor.is_none() {
            self.nested.cursor = Some(rows[0].info.folder.clone());
        }
        let available = ui
            .available_width()
            .min((ui.clip_rect().right() - ui.cursor().min.x).max(1.0));
        let layout = album_layout(
            available,
            ui.spacing().item_spacing,
            self.settings.effective_gallery_scale(),
        );
        let columns = layout.columns;
        let height = layout.image_height + 100.0;
        let active = self.nested.cursor.clone();
        let mut seed = None;
        egui::ScrollArea::vertical()
            .id_salt("gallery-albums")
            .show_rows(ui, height, rows.len(), |ui, range| {
                for row in &rows[range] {
                    let selected = active.as_ref() == Some(&row.info.folder);
                    let folder = row.info.folder.clone();
                    let pos = self
                        .nested
                        .positions
                        .get(&folder)
                        .copied()
                        .unwrap_or(0)
                        .min(row.indices.len().saturating_sub(1));
                    let window = album_window(pos, columns);
                    let start = window.start;
                    let end = window.end.min(row.indices.len());
                    let frame = egui::Frame::new()
                        .fill(if selected {
                            Color32::from_rgb(47, 47, 53)
                        } else {
                            Color32::from_rgb(35, 35, 39)
                        })
                        .corner_radius(10)
                        .inner_margin(8);
                    let response = frame
                        .show(ui, |ui| {
                            ui.set_min_height(height - 24.0);
                            ui.horizontal(|ui| {
                                ui.add(
                                    Lucide::Folder
                                        .size(17.0)
                                        .image()
                                        .tint(Color32::from_rgb(224, 188, 127)),
                                );
                                let root = self.gallery.as_ref().unwrap().folder();
                                let name = folder
                                    .strip_prefix(root)
                                    .ok()
                                    .filter(|p| !p.as_os_str().is_empty())
                                    .map(|p| p.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| {
                                        folder
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .into_owned()
                                    });
                                // Reserve the controls before laying out the title. A
                                // truncated label otherwise takes the whole header and
                                // pushes the trailing buttons past the pane's right edge.
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(egui::Button::image(
                                                Lucide::ChevronRight.size(16.0).image(),
                                            ))
                                            .clicked()
                                        {
                                            self.nested.cursor = Some(folder.clone());
                                            let _ = self.album_move(1, 0);
                                            release_keyboard_focus(ui);
                                        }
                                        if ui
                                            .add(egui::Button::image(
                                                Lucide::ChevronLeft.size(16.0).image(),
                                            ))
                                            .clicked()
                                        {
                                            self.nested.cursor = Some(folder.clone());
                                            let _ = self.album_move(-1, 0);
                                            release_keyboard_focus(ui);
                                        }
                                        let range = if row.indices.is_empty() {
                                            "0".into()
                                        } else {
                                            format!("{}–{end}", start + 1)
                                        };
                                        ui.weak(format!(
                                            "{range} / {}{}",
                                            row.indices.len(),
                                            if row.info.exhausted { "" } else { "+" }
                                        ));
                                        ui.allocate_ui_with_layout(
                                            egui::vec2(ui.available_width().max(0.0), ui.available_height()),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.add(egui::Label::new(egui::RichText::new(name).strong()).truncate());
                                            },
                                        );
                                    },
                                );
                            });
                            ui.add_space(5.0);
                            if row.indices.is_empty() {
                                let (rect, r) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), layout.image_height + 24.0),
                                    egui::Sense::click(),
                                );
                                let text = if let Some(error) = &row.info.error {
                                    error.clone()
                                } else if row.info.exhausted {
                                    if row.info.children > 0 {
                                        format!(
                                            "Здесь нет изображений · подпапок: {} · вниз — продолжить",
                                            row.info.children
                                        )
                                    } else {
                                        "Нет изображений или видео".into()
                                    }
                                } else if self.nested.snapshot.loading && selected {
                                    "Загружаем альбом…".into()
                                } else {
                                    "Листать вправо — показать изображения".into()
                                };
                                ui.painter_at(rect).text(
                                    rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    text,
                                    egui::FontId::proportional(13.0),
                                    Color32::GRAY,
                                );
                                if r.clicked() {
                                    self.active_pane = ActivePane::Gallery;
                                    self.nested.cursor = Some(folder.clone());
                                    let _ = self.load_album(folder.clone());
                                    release_keyboard_focus(ui);
                                }
                            } else {
                                ui.horizontal(|ui| {
                                    for &index in &row.indices[start..end] {
                                        self.gallery_tile(ui, index, layout);
                                    }
                                });
                            }
                        })
                        .response;
                    if row.indices.len() < window.end
                        && !row.info.exhausted
                        && row.info.error.is_none()
                        && !self.nested.snapshot.limited
                        && seed.is_none()
                        && (row.info.depth <= 1 || selected)
                        && response.rect.intersects(ui.clip_rect())
                    {
                        seed = Some(folder.clone());
                    }
                    if selected && self.nested.scroll_album {
                        response.scroll_to_me(None);
                        self.nested.scroll_album = false;
                    }
                    ui.add_space(8.0);
                }
            });
        if self.active_pane == ActivePane::Gallery
            && !self.nested.snapshot.loading
            && self.nested.pending_album.is_none()
            && let Some(folder) = seed
        {
            let _ = self.load_album(folder);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_capacity_tracks_width_scale_and_spacing_without_column_caps() {
        let spacing = egui::vec2(8.0, 8.0);
        let normal = album_layout(1800.0, spacing, None);
        let small = album_layout(1800.0, spacing, Some(0.4));
        let wide = album_layout(4096.0, spacing, Some(0.4));
        assert!(normal.columns > 3);
        assert!(small.columns > 12);
        assert!(wide.columns > small.columns);
        assert!(album_layout(1800.0, egui::vec2(32.0, 8.0), Some(0.4)).columns < small.columns);
        assert_eq!(album_layout(120.0, spacing, Some(2.5)).columns, 1);
    }

    #[test]
    fn album_cells_fill_inner_width_and_keep_cursor_in_the_window() {
        for scale in [None, Some(0.4), Some(0.65), Some(2.5)] {
            let layout = album_layout(1470.0, egui::vec2(8.0, 8.0), scale);
            let filled =
                layout.tile_width * layout.columns as f32 + 8.0 * (layout.columns - 1) as f32;
            assert!(filled <= 1470.0 - 32.0 + 0.01);
            if scale.is_none() {
                assert!((filled - (1470.0 - 32.0)).abs() < 0.01);
            }
            for position in [0, 4, 5, 19, 40] {
                let window = album_window(position, layout.columns);
                assert!(window.contains(&position));
                assert_eq!(window.len(), layout.columns);
            }
        }
    }

    #[test]
    fn automatic_cards_fill_resized_windows_while_manual_cards_keep_their_size() {
        let spacing = egui::vec2(8.0, 8.0);
        for layout in [grid_layout, album_layout] {
            let first = layout(820.0, spacing, None);
            let resized = layout(900.0, spacing, None);
            assert_eq!(first.columns, resized.columns);
            assert!(resized.tile_width > first.tile_width);
            let manual = layout(820.0, spacing, Some(0.65));
            let manual_resized = layout(1700.0, spacing, Some(0.65));
            assert_eq!(manual.tile_width, manual_resized.tile_width);
            assert!(manual_resized.columns > manual.columns);
            assert!(layout(100.0, spacing, Some(2.5)).tile_width <= 100.0);
        }
    }
}
