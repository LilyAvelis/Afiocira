//! Single-file F2 dialogue. The draft binds its source to a captured file version.
use super::*;
use photo_core::actions::ActionId;

pub(super) struct Dialog {
    source: PathBuf,
    version: photo_core::rename::SourceVersion,
    name: String,
    pub error: Option<String>,
    focus: bool,
    focus_index: usize,
    frame: u64,
}

impl Workshop {
    pub(super) fn begin_rename(&mut self) -> Result<(), String> {
        if self.image_dialog.is_some()
            || self.delete_dialog.is_some()
            || self.operation.as_ref().is_some_and(|j| j.running)
        {
            return Err("A file action is already running".into());
        }
        if self.active_pane == ActivePane::Yazi {
            return self.queue_navigator_command(Command::BeginRename);
        }
        let paths = self.action_paths();
        if paths.len() != 1 {
            return Err("Select one file to rename".into());
        }
        let source = paths[0].clone();
        let version = photo_core::rename::capture(&source).map_err(|e| e.to_string())?;
        let name = source
            .file_name()
            .ok_or("Missing filename")?
            .to_string_lossy()
            .into_owned();
        self.close_palette();
        self.rename_dialog = Some(Dialog {
            source,
            version,
            name,
            error: None,
            focus: true,
            focus_index: 0,
            frame: self.ctx.cumulative_frame_nr(),
        });
        Ok(())
    }
    pub(super) fn commit_rename(&mut self, new_name: String) -> Result<(), String> {
        if self.rename_dialog.is_none() {
            self.begin_rename()?;
        }
        let dialog = self.rename_dialog.as_mut().unwrap();
        dialog.name = new_name.clone();
        dialog.error = None;
        let kind = operations::TaskKind::Rename {
            source: dialog.source.clone(),
            new_name,
            version: dialog.version.clone(),
        };
        self.start_task(kind)
    }
    pub(super) fn rename_snapshot(&self) -> Option<photo_core::api::RenameState> {
        self.rename_dialog
            .as_ref()
            .map(|d| photo_core::api::RenameState {
                source: d.source.clone(),
                new_name: d.name.clone(),
                error: d.error.clone(),
                busy: self
                    .operation
                    .as_ref()
                    .is_some_and(|j| j.running && j.action == ActionId::Rename),
            })
    }
    pub(super) fn rename_popup(&mut self, ctx: &egui::Context) {
        let busy = self
            .operation
            .as_ref()
            .is_some_and(|j| j.running && j.action == ActionId::Rename);
        let Some(dialog) = &mut self.rename_dialog else {
            return;
        };
        let mut submit = None;
        let mut cancel = false;
        let armed = ctx.cumulative_frame_nr() > dialog.frame;
        let enter = ctx.input_mut(|i| delete_action::take_fresh_press(i, Key::Enter));
        let tab = ctx.input_mut(delete_action::take_tab);
        if let Some(backwards) = tab {
            dialog.focus_index = (dialog.focus_index + if backwards { 2 } else { 1 }) % 3;
        }
        let move_focus = dialog.focus || tab.is_some();
        let result = egui::Modal::new(egui::Id::new("rename-file"))
            .frame(
                egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                    .corner_radius(12)
                    .inner_margin(16),
            )
            .show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 64.0).clamp(220.0, 440.0));
                ui.horizontal(|ui| {
                    ui.add(Lucide::Pencil.size(18.0).image());
                    ui.heading("Переименовать");
                });
                ui.label(
                    egui::RichText::new(
                        dialog
                            .source
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                    )
                    .small()
                    .color(Color32::from_gray(175)),
                );
                ui.add_space(4.0);
                let response = ui.add_enabled(
                    !busy,
                    egui::TextEdit::singleline(&mut dialog.name)
                        .id(egui::Id::new("rename-name"))
                        .desired_width(f32::INFINITY),
                );
                if move_focus && dialog.focus_index == 0 {
                    response.request_focus();
                }
                if !move_focus && response.has_focus() {
                    dialog.focus_index = 0;
                }
                if dialog.focus {
                    response.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ctx, response.id) {
                        let stem = Path::new(&dialog.name)
                            .file_stem()
                            .map(|s| s.to_string_lossy().chars().count())
                            .unwrap_or(dialog.name.chars().count());
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::two(
                                egui::text::CCursor::new(0),
                                egui::text::CCursor::new(stem),
                            )));
                        state.store(ctx, response.id);
                    }
                    dialog.focus = false;
                }
                if let Some(error) = &dialog.error {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let cancel_button = ui
                        .push_id("rename-cancel", |ui| {
                            ui.add_enabled(!busy, egui::Button::new("Отмена · Esc"))
                        })
                        .inner;
                    if move_focus && dialog.focus_index == 1 {
                        cancel_button.request_focus();
                    }
                    if !move_focus && cancel_button.has_focus() {
                        dialog.focus_index = 1;
                    }
                    if cancel_button.clicked() {
                        cancel = true;
                    }
                    let save_button = ui
                        .push_id("rename-save", |ui| {
                            ui.add_enabled(
                                !busy,
                                egui::Button::new(if busy {
                                    "Переименование…"
                                } else {
                                    "Сохранить · Enter"
                                }),
                            )
                        })
                        .inner;
                    if move_focus && dialog.focus_index == 2 {
                        save_button.request_focus();
                    }
                    if !move_focus && save_button.has_focus() {
                        dialog.focus_index = 2;
                    }
                    if save_button.clicked() {
                        submit = Some(dialog.name.clone());
                    }
                });
                if !busy && armed && enter {
                    if dialog.focus_index == 1 {
                        cancel = true;
                    } else {
                        submit = Some(dialog.name.clone());
                    }
                }
            });
        if let Some(new_name) = submit {
            if let Some(error) = self.execute(Command::Rename { new_name }).error
                && let Some(dialog) = &mut self.rename_dialog
            {
                dialog.error = Some(error.message);
            }
        } else if !busy && (cancel || result.should_close()) {
            self.rename_dialog = None;
        }
    }
}
