use super::*;
use photo_core::history::{HistoryPanelState, HistorySnapshot, Operation};

#[derive(Default)]
pub(super) struct State {
    pub open: bool,
    pub target: Option<PathBuf>,
    pub cache: HashMap<PathBuf, HistorySnapshot>,
    pub preview: Option<(usize, TextureHandle)>,
    pub error: Option<String>,
    thumbnails: HashMap<usize, TextureHandle>,
    thumbnail_lru: VecDeque<usize>,
    thumbnail_errors: HashMap<usize, String>,
    pub visible_pending: Option<usize>,
    selected: Option<usize>,
    epoch: u64,
    selection_generation: u64,
    canvas_failed: Option<(usize, u64)>,
    background_dispatch: bool,
    hovered: Option<usize>,
    scroll_to_selected: bool,
}
impl State {
    pub fn snapshot(&self) -> HistoryPanelState {
        HistoryPanelState {
            thumbnail_indices: self.thumbnails.keys().copied().collect(),
            open: self.open,
            snapshot: self
                .target
                .as_ref()
                .and_then(|p| self.cache.get(p))
                .cloned(),
            preview_index: self.selected,
            preview_ready: self
                .selected
                .is_some_and(|index| self.preview.as_ref().is_some_and(|(i, _)| *i == index)),
            error: self.error.clone(),
        }
    }
    pub fn accept(&mut self, snapshot: HistorySnapshot) {
        let changed = self.target.as_ref() != Some(&snapshot.path)
            || self.cache.get(&snapshot.path).is_none_or(|old| {
                old.versions.len() != snapshot.versions.len()
                    || old
                        .versions
                        .iter()
                        .zip(&snapshot.versions)
                        .any(|(a, b)| a.path != b.path || a.bytes != b.bytes)
            });
        if changed {
            self.reset_thumbnails();
        }
        if self.open {
            self.target = Some(snapshot.path.clone());
        }
        if self.cache.len() >= 256 && !self.cache.contains_key(&snapshot.path) {
            self.cache.clear();
        }
        self.cache.insert(snapshot.path.clone(), snapshot);
    }
    pub fn reset_thumbnails(&mut self) {
        self.thumbnails.clear();
        self.thumbnail_lru.clear();
        self.thumbnail_errors.clear();
        self.visible_pending = None;
        self.preview = None;
        self.selected = None;
        self.hovered = None;
        self.canvas_failed = None;
        self.epoch = self.epoch.wrapping_add(1);
        self.selection_generation = self.selection_generation.wrapping_add(1);
    }
    pub fn accept_preview(&mut self, index: usize, texture: TextureHandle) {
        self.visible_pending = None;
        self.thumbnail_errors.remove(&index);
        self.thumbnail_lru.retain(|&i| i != index);
        self.thumbnail_lru.push_back(index);
        self.thumbnails.insert(index, texture);
        while self.thumbnails.len() > 24 {
            if let Some(index) = self.thumbnail_lru.pop_front() {
                self.thumbnails.remove(&index);
            }
        }
    }
    pub fn fail_preview(&mut self, index: usize, error: String) {
        self.visible_pending = None;
        self.thumbnail_errors.insert(index, error);
    }
    fn select(&mut self, index: usize) {
        if self.selected != Some(index) {
            self.selected = Some(index);
            self.selection_generation = self.selection_generation.wrapping_add(1);
            self.canvas_failed = None;
        }
    }
    pub fn preview_request(&self, canvas: bool) -> operations::HistoryPreviewRequest {
        operations::HistoryPreviewRequest {
            epoch: self.epoch,
            selection: self.selection_generation,
            canvas,
        }
    }
    pub fn accepts_request(&self, request: operations::HistoryPreviewRequest) -> bool {
        self.epoch == request.epoch
    }
    pub fn current_canvas_request(
        &self,
        index: usize,
        request: operations::HistoryPreviewRequest,
    ) -> bool {
        self.accepts_request(request)
            && self.selected == Some(index)
            && self.selection_generation == request.selection
    }
    pub fn accept_canvas(
        &mut self,
        index: usize,
        request: operations::HistoryPreviewRequest,
        texture: TextureHandle,
    ) {
        if self.accepts_request(request)
            && self.selected == Some(index)
            && self.selection_generation == request.selection
        {
            self.preview = Some((index, texture));
            self.canvas_failed = None;
        }
    }
    pub fn fail_canvas(
        &mut self,
        index: usize,
        request: operations::HistoryPreviewRequest,
        error: String,
    ) {
        if self.accepts_request(request)
            && self.selected == Some(index)
            && self.selection_generation == request.selection
        {
            self.canvas_failed = Some((index, request.selection));
            self.error = Some(error);
        }
    }
    pub fn canvas_texture(&self, path: &Path) -> Option<TextureHandle> {
        if !self.open || self.target.as_deref() != Some(path) {
            return None;
        }
        let index = self.selected?;
        self.preview
            .as_ref()
            .filter(|(i, _)| *i == index)
            .map(|(_, t)| t.clone())
            .or_else(|| self.thumbnails.get(&index).cloned())
            .or_else(|| self.preview.as_ref().map(|(_, t)| t.clone()))
    }
    fn needs_canvas(&self, index: usize) -> bool {
        self.preview.as_ref().is_none_or(|(i, _)| *i != index)
            && self.canvas_failed != Some((index, self.selection_generation))
    }
    fn step(&mut self, delta: isize) {
        let Some(snapshot) = self.target.as_ref().and_then(|p| self.cache.get(p)) else {
            return;
        };
        if snapshot.versions.is_empty() {
            return;
        }
        let current = self.selected.or(snapshot.cursor).unwrap_or(0);
        self.select(
            current
                .saturating_add_signed(delta)
                .min(snapshot.versions.len() - 1),
        );
        self.scroll_to_selected = true;
    }
}
impl Workshop {
    pub(super) fn history_keyboard(&mut self, ctx: &egui::Context) {
        if !self.history_panel.open
            || ctx.text_edit_focused()
            || self.image_dialog.is_some()
            || self.adjustments_panel.is_some()
        {
            return;
        }
        let up = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp)
                | i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft)
        });
        let down = ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown)
                | i.consume_key(egui::Modifiers::NONE, Key::ArrowRight)
        });
        if up {
            self.history_panel.step(1);
        } else if down {
            self.history_panel.step(-1);
        }
    }
    pub(super) fn history_path(&self) -> Result<PathBuf, String> {
        self.gallery
            .as_ref()
            .filter(|g| g.view() == View::Photo)
            .and_then(Gallery::selected_photo)
            .filter(|p| !p.is_video())
            .map(|p| p.path.clone())
            .ok_or_else(|| "History is available for an image in Space".into())
    }
    pub(super) fn history_command(&mut self, op: Operation) -> Result<(), String> {
        if self.image_dialog.is_some()
            || self.adjustments_panel.is_some()
            || self.rename_dialog.is_some()
            || self.delete_dialog.is_some()
            || self.operation.as_ref().is_some_and(|o| o.running)
            || self.metadata.busy()
            || self.metadata_panel.panel_state().pending
        {
            return Err("Сначала завершите текущий диалог или операцию".into());
        }
        let source = self.history_path()?;
        if self.metadata_panel.is_dirty() {
            return Err("Сохраните или отмените изменения метаданных".into());
        }
        if matches!(op, Operation::Inspect) {
            self.active_pane = ActivePane::Gallery;
            if !self.metadata_panel.close() {
                return Err("Metadata request is pending".into());
            }
            self.history_panel.open = true;
            self.history_panel.reset_thumbnails();
            if self.history_panel.target.as_ref() != Some(&source) {
                self.history_panel.reset_thumbnails();
            }
            self.history_panel.target = Some(source.clone());
            self.history_panel.preview = None;
            self.history_panel.error = None;
        }
        let preview = if let Operation::Preview(index) = op {
            let canvas = !self.history_panel.background_dispatch;
            if canvas {
                self.history_panel.select(index);
            }
            Some(self.history_panel.preview_request(canvas))
        } else {
            None
        };
        self.start_task(operations::TaskKind::History {
            source,
            op,
            preview,
        })
    }
    pub(super) fn render_history_panel(&mut self, ui: &mut egui::Ui) {
        if !self.history_panel.open {
            return;
        }
        if self.history_path().ok() != self.history_panel.target {
            self.history_panel.open = false;
            self.history_panel.reset_thumbnails();
            return;
        }
        let mut command = None;
        let mut automatic_preview = None;
        let mut hovered = None;
        let state = self.history_panel.snapshot();
        let scroll_to_selected = std::mem::take(&mut self.history_panel.scroll_to_selected);
        let busy = self.operation.as_ref().is_some_and(|o| o.running);
        egui::Panel::right("history-panel").default_size(340.0).size_range(240.0..=480.0).resizable(true).show(ui,|ui| {
            ui.heading("История");
            ui.weak("Текущая сессия · SSD");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Закрыть").clicked() { command=Some(Command::CloseHistory); }
                if busy { ui.spinner(); }
            });
            if let Some(error) = &state.error { crate::ui_foundation::message(ui,error,true); }
            let Some(snapshot) = &state.snapshot else { ui.spinner(); return; };
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!busy && snapshot.can_undo,egui::Button::new("Назад · Ctrl Z")).clicked() { command=Some(Command::UndoHistory); }
                if ui.add_enabled(!busy && snapshot.can_redo,egui::Button::new("Вперёд · Ctrl Shift Z")).clicked() { command=Some(Command::RedoHistory); }
            });
            if snapshot.can_redo && ui.add_enabled(!busy,egui::Button::new("Обрезать продолжение · Ctrl S")).clicked() { command=Some(Command::CommitHistory); }
            if snapshot.versions.is_empty() { ui.add_space(16.0); ui.label("У этого изображения пока нет истории."); ui.weak("Версии появятся после конвертации или правки света и цвета. Повороты и отражения не добавляют шагов."); }
            egui::ScrollArea::vertical().id_salt("history-list").show(ui,|ui| {
                for (index,node) in snapshot.versions.iter().enumerate().rev() {
                    ui.push_id(index, |ui| {
                    let current = snapshot.cursor==Some(index);
                    let selected = state.preview_index==Some(index);
                    let width = ui.available_width();
                    let texture = self.history_panel.thumbnails.get(&index).cloned();
                    let failure = self.history_panel.thumbnail_errors.get(&index).cloned();
                    let frame = egui::Frame::new()
                        .fill(if selected { ui.visuals().selection.bg_fill } else { ui.visuals().faint_bg_color })
                        .stroke(if current { ui.visuals().selection.stroke } else { egui::Stroke::NONE })
                        .corner_radius(crate::ui_foundation::tokens::SECTION_RADIUS)
                        .inner_margin(crate::ui_foundation::tokens::SM as i8)
                        .show(ui, |ui| {
                            ui.set_width((width-16.0).max(0.0));
                            ui.horizontal(|ui| {
                                let thumbnail_width = (ui.available_width()*0.42).min(112.0);
                                ui.allocate_ui(egui::vec2(thumbnail_width,88.0), |ui| {
                                    if let Some(texture) = &texture {
                                        let size=texture.size_vec2();
                                        let scale=(thumbnail_width/size.x).min(88.0/size.y).min(1.0);
                                        ui.centered_and_justified(|ui| { ui.add(egui::Image::new(texture).fit_to_exact_size(size*scale)); });
                                    } else { ui.centered_and_justified(|ui| { ui.weak(if failure.is_some() { "Нет превью" } else { "Загрузка…" }); }); }
                                });
                                ui.vertical(|ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(format!("Версия {}",index+1));
                                    if current { ui.weak("Текущая"); }
                                    ui.weak(node.path.extension().unwrap_or_default().to_string_lossy().to_uppercase());
                                    ui.weak(format!("{} байт",node.bytes));
                                });
                            });
                        });
                    let visible=ui.is_rect_visible(frame.response.rect);
                    let response=frame.response.interact(egui::Sense::click());
                    if selected && scroll_to_selected { response.scroll_to_me(Some(egui::Align::Center)); }
                    if response.hovered() { hovered=Some(index); if self.history_panel.hovered!=Some(index) { self.history_panel.select(index); } }
                    if response.clicked() { self.history_panel.select(index); self.history_panel.canvas_failed=None; }
                    if visible && texture.is_some() { self.history_panel.thumbnail_lru.retain(|&i| i != index); self.history_panel.thumbnail_lru.push_back(index); }
                    if visible && !busy && texture.is_none() && failure.is_none() && self.history_panel.visible_pending.is_none() && automatic_preview.is_none() { automatic_preview=Some(index); }
                    if let Some(error)=failure { response.on_hover_text(error); }
                    ui.add_space(crate::ui_foundation::tokens::SM);
                    });
                }
            });
        });
        self.history_panel.hovered = hovered;
        let mut background = false;
        if command.is_none()
            && !busy
            && let Some(index) = self.history_panel.selected
            && self.history_panel.needs_canvas(index)
        {
            command = Some(Command::PreviewHistory { index });
        }
        if command.is_none()
            && let Some(index) = automatic_preview
        {
            command = Some(Command::PreviewHistory { index });
            background = true;
        }
        if background && let Some(Command::PreviewHistory { index }) = &command {
            self.history_panel.visible_pending = Some(*index);
        }
        self.history_panel.background_dispatch = background;
        if let Some(command) = command
            && let Some(e) = self.execute(command).error
        {
            if let Some(index) = self.history_panel.visible_pending.take() {
                self.history_panel.fail_preview(index, e.message.clone());
            }
            self.history_panel.error = Some(e.message);
        }
        self.history_panel.background_dispatch = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn thumbnail_cache_is_bounded_and_errors_do_not_stall_next_row() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        for index in 0..30 {
            state.accept_preview(
                index,
                ctx.load_texture(
                    format!("history-test-{index}"),
                    egui::ColorImage::filled([2, 2], Color32::WHITE),
                    TextureOptions::LINEAR,
                ),
            );
        }
        assert_eq!(state.thumbnails.len(), 24);
        assert!(!state.thumbnails.contains_key(&0));
        assert!(state.thumbnails.contains_key(&29));
        state.visible_pending = Some(4);
        state.fail_preview(4, "Unreadable snapshot".into());
        assert!(state.visible_pending.is_none());
        assert!(state.thumbnail_errors.contains_key(&4));
        state.reset_thumbnails();
        assert!(state.thumbnails.is_empty());
        assert!(state.preview.is_none());
        assert!(state.thumbnail_errors.is_empty());
    }
    fn texture(ctx: &egui::Context, name: &str) -> TextureHandle {
        ctx.load_texture(
            name,
            egui::ColorImage::filled([2, 2], Color32::WHITE),
            TextureOptions::LINEAR,
        )
    }
    #[test]
    fn preload_does_not_select_and_latest_canvas_selection_wins() {
        let ctx = egui::Context::default();
        let mut state = State::default();
        state.accept_preview(0, texture(&ctx, "thumbnail"));
        assert!(state.selected.is_none());
        assert!(state.preview.is_none());
        state.select(0);
        assert!(!state.snapshot().preview_ready);
        let old = state.preview_request(true);
        state.select(1);
        let latest = state.preview_request(true);
        state.accept_canvas(0, old, texture(&ctx, "stale"));
        state.fail_canvas(0, old, "stale failure".into());
        assert!(state.error.is_none());
        assert!(state.preview.is_none());
        assert_eq!(state.selected, Some(1));
        state.accept_canvas(1, latest, texture(&ctx, "latest"));
        assert_eq!(state.preview.as_ref().unwrap().0, 1);
        assert!(!state.needs_canvas(1));
        assert!(state.snapshot().preview_ready);
        state.reset_thumbnails();
        state.select(1);
        state.accept_canvas(1, latest, texture(&ctx, "old-epoch"));
        assert!(state.preview.is_none());
    }
    #[test]
    fn browsing_versions_changes_selection_only_and_preserves_stack_cursor() {
        let mut state = State {
            open: true,
            ..Default::default()
        };
        let path = PathBuf::from("image.jpg");
        state.accept(HistorySnapshot {
            path: path.clone(),
            versions: (0..3)
                .map(|_| photo_core::history::HistoryNode {
                    path: path.clone(),
                    bytes: 1,
                })
                .collect(),
            cursor: Some(1),
            can_undo: true,
            can_redo: true,
        });
        state.step(1);
        assert_eq!(state.selected, Some(2));
        state.step(1);
        assert_eq!(state.selected, Some(2));
        state.step(-1);
        state.step(-1);
        state.step(-1);
        assert_eq!(state.selected, Some(0));
        assert_eq!(state.cache[&path].cursor, Some(1));
        assert!(state.cache[&path].can_undo);
        assert!(state.cache[&path].can_redo);
    }
}
