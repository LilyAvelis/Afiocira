//! Raycast-style footer and bottom-right action menu; search stays at the bottom.
use super::*;
use photo_core::actions::{ActionId, ActionInfo};

/// Shared SVG icon vocabulary for every action surface. Keep this exhaustive
/// so a new command cannot silently inherit an unrelated navigation arrow.
pub(super) fn icon(action: ActionId) -> Lucide {
    match action {
        ActionId::EnterAlbum => Lucide::FolderOpen,
        ActionId::Copy => Lucide::Copy,
        ActionId::Duplicate => Lucide::CopyPlus,
        ActionId::Paste => Lucide::ClipboardPaste,
        ActionId::Rename => Lucide::Pencil,
        ActionId::Delete => Lucide::Trash2,
        ActionId::Search => Lucide::Search,
        ActionId::ResizeImage => Lucide::Scaling,
        ActionId::ConvertImage => Lucide::Replace,
        ActionId::Open => Lucide::Image,
        ActionId::Back => Lucide::Images,
        ActionId::Next => Lucide::ArrowRight,
        ActionId::Previous => Lucide::ArrowLeft,
        ActionId::ClearMarks => Lucide::ListX,
        ActionId::Refresh => Lucide::RefreshCw,
        ActionId::Fit => Lucide::Maximize2,
        ActionId::PlayPause => Lucide::Play,
    }
}

#[derive(Default)]
pub(super) struct State {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub focus_search: bool,
    pub scroll_selected: bool,
    pub enter_armed: bool,
}

fn take_enter(i: &mut egui::InputState, opening: bool, armed: &mut bool) -> bool {
    let fresh_enter = i.events.iter().any(|event| {
        matches!(event,
        egui::Event::Key { key: Key::Enter, pressed: true, repeat: false, modifiers, .. }
        if *modifiers == egui::Modifiers::NONE)
    });
    if opening {
        *armed = !i.key_down(Key::Enter) && !fresh_enter;
    }
    let enter = i.consume_key(egui::Modifiers::NONE, Key::Enter) && fresh_enter && *armed;
    // An Enter which opened a popup must be released before it can
    // activate its selected command. Repeated key events never run it.
    if !i.key_down(Key::Enter) && !fresh_enter {
        *armed = true;
    }
    enter
}

impl Workshop {
    pub(super) fn filtered_actions(&self) -> Vec<ActionInfo> {
        let words: Vec<_> = self
            .palette
            .query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        self.available_actions()
            .into_iter()
            .filter(|a| {
                let haystack =
                    format!("{} {} {:?} {}", a.label, a.group, a.id, a.shortcut).to_lowercase();
                words.iter().all(|word| haystack.contains(word))
            })
            .collect()
    }
    pub(super) fn palette_snapshot(&self) -> photo_core::api::PaletteState {
        photo_core::api::PaletteState {
            open: self.palette.open,
            mode: if self
                .gallery
                .as_ref()
                .is_some_and(|g| g.view() == View::Photo)
            {
                "space"
            } else {
                "gallery"
            }
            .into(),
            query: self.palette.query.clone(),
            selected: self.palette.selected,
        }
    }
    pub(super) fn actions_footer(&mut self, ui: &mut egui::Ui) {
        let mut toggle = false;
        egui::Panel::bottom("actions-footer")
            .exact_size(46.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(48, 48, 52))
                    .inner_margin(egui::Margin::symmetric(14, 7)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add(Lucide::Images.size(17.0).image());
                    let count = self.action_paths().len();
                    let label = if self.operation.as_ref().is_some_and(|j| j.running) {
                        "Выполняется…".to_owned()
                    } else if self
                        .gallery
                        .as_ref()
                        .is_some_and(|g| g.view() == View::Photo)
                    {
                        "Space · текущий файл".into()
                    } else {
                        format!("Gallery · выбрано {count}")
                    };
                    ui.label(egui::RichText::new(label).color(Color32::from_gray(180)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        toggle = crate::ui_style::actions_button(ui, self.palette.open).clicked();
                    });
                });
            });
        if toggle {
            let command = if self.palette.open {
                Command::CloseCommands
            } else {
                Command::ShowCommands
            };
            if let Some(error) = self.execute(command).error {
                self.message = error.message;
            }
        }
    }
    pub(super) fn actions_popup(&mut self, ctx: &egui::Context) {
        if !self.palette.open {
            return;
        }
        let actions = self.filtered_actions();
        self.palette.selected = self.palette.selected.min(actions.len().saturating_sub(1));
        let mut picked = None;
        let opening = self.palette.focus_search;
        let mut refocus_search = false;
        let enter = ctx.input_mut(|i| {
            let enter = take_enter(i, opening, &mut self.palette.enter_armed);
            refocus_search = i.consume_key(egui::Modifiers::NONE, Key::Tab)
                | i.consume_key(egui::Modifiers::SHIFT, Key::Tab);
            if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) && !actions.is_empty() {
                self.palette.selected = (self.palette.selected + 1) % actions.len();
                self.palette.scroll_selected = true;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) && !actions.is_empty() {
                self.palette.selected = (self.palette.selected + actions.len() - 1) % actions.len();
                self.palette.scroll_selected = true;
            }
            enter
        });
        let width = (ctx.content_rect().width() - 28.0).clamp(240.0, 420.0);
        let area = egui::Modal::default_area(egui::Id::new("action-palette"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -58.0));
        let result = egui::Modal::new(egui::Id::new("action-palette"))
            .area(area)
            .backdrop_color(Color32::from_black_alpha(24))
            .frame(
                egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                    .fill(Color32::from_rgb(69, 69, 74))
                    .stroke(egui::Stroke::new(1.0, Color32::from_gray(118)))
                    .corner_radius(12)
                    .inner_margin(12),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.label(
                    egui::RichText::new(
                        if self
                            .gallery
                            .as_ref()
                            .is_some_and(|g| g.view() == View::Photo)
                        {
                            "Действия с файлом"
                        } else {
                            "Действия с выделением и папкой"
                        },
                    )
                    .strong()
                    .size(13.0),
                );
                ui.add_space(2.0);
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 180.0).clamp(80.0, 320.0))
                    .show(ui, |ui| {
                        let mut group = String::new();
                        for (index, action) in actions.iter().enumerate() {
                            if group != action.group {
                                group = action.group.clone();
                                ui.add_space(3.0);
                                ui.label(
                                    egui::RichText::new(&group)
                                        .size(11.0)
                                        .color(Color32::from_gray(185)),
                                );
                            }
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), 38.0),
                                egui::Sense::click(),
                            );
                            if index == self.palette.selected || response.hovered() {
                                ui.painter().rect_filled(
                                    rect,
                                    7.0,
                                    Color32::from_rgb(100, 100, 108),
                                );
                            }
                            if index == self.palette.selected && self.palette.scroll_selected {
                                self.palette.scroll_selected = false;
                                response.scroll_to_me(Some(egui::Align::Center));
                            }
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(rect.shrink2(egui::vec2(9.0, 3.0)))
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                |ui| {
                                    let icon = icon(action.id);
                                    if action.id == ActionId::Delete {
                                        ui.visuals_mut().override_text_color =
                                            Some(Color32::LIGHT_RED);
                                    }
                                    ui.add(icon.size(16.0).image());
                                    ui.label(egui::RichText::new(&action.label).size(14.0).color(
                                        if action.id == ActionId::Delete {
                                            Color32::LIGHT_RED
                                        } else {
                                            ui.visuals().text_color()
                                        },
                                    ));
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if !action.shortcut.is_empty() {
                                                crate::ui_style::keycap(ui, &action.shortcut);
                                            }
                                        },
                                    );
                                },
                            );
                            if response.clicked() {
                                picked = Some(action.id);
                            }
                        }
                        if actions.is_empty() {
                            ui.add_space(12.0);
                            ui.label("Ничего не найдено");
                            ui.add_space(12.0);
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.add(Lucide::Search.size(16.0).image());
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.palette.query)
                            .hint_text("Поиск действий…")
                            .frame(egui::Frame::NONE)
                            .desired_width(width - 80.0),
                    );
                    if self.palette.focus_search || refocus_search {
                        response.request_focus();
                        self.palette.focus_search = false;
                    }
                    if response.changed() {
                        self.palette.selected = 0;
                        self.palette.scroll_selected = true;
                    }
                    crate::ui_style::keycap(ui, "Esc");
                });
            });
        // TextEdit processes this frame's text events during show(). Resolve
        // Enter against that current query, never the list rendered beforehand.
        if enter && picked.is_none() {
            picked = self
                .filtered_actions()
                .get(self.palette.selected)
                .map(|a| a.id);
        }
        if let Some(id) = picked {
            if let Some(error) = self.execute(Command::RunAction { id }).error {
                self.message = error.message;
            }
        } else if result.should_close() {
            self.close_palette();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_enter_and_repeats_require_release_then_fresh_press() {
        let ctx = egui::Context::default();
        let mut armed = false;
        let mut frame = |opening, pressed, repeat| {
            let mut fired = false;
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed,
                    repeat,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                fired = ui.ctx().input_mut(|i| take_enter(i, opening, &mut armed));
            });
            output.textures_delta.clear();
            fired
        };
        assert!(!frame(true, true, false));
        assert!(!frame(false, true, true));
        assert!(!frame(false, false, false));
        assert!(frame(false, true, false));
        assert!(!frame(false, true, true));
    }
}
