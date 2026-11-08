//! Captured delete requests; a group can never retarget after confirmation.
use super::*;
use photo_core::recycle::Target;
pub(super) struct Dialog {
    scope: photo_core::selection::SelectionScope,
    targets: Vec<Target>,
    order: Vec<PathBuf>,
    cursor: usize,
    revision: u64,
    frame: u64,
    focus: usize,
    autofocus: bool,
}
pub(super) fn successor(
    order: &[PathBuf],
    cursor: usize,
    survivors: &std::collections::BTreeSet<PathBuf>,
) -> Option<PathBuf> {
    order
        .iter()
        .skip(cursor)
        .find(|p| survivors.contains(*p))
        .or_else(|| {
            order
                .iter()
                .take(cursor)
                .rev()
                .find(|p| survivors.contains(*p))
        })
        .cloned()
}
pub(super) fn take_fresh_press(input: &mut egui::InputState, key: Key) -> bool {
    let pressed = input.events.iter().any(|e| matches!(e, egui::Event::Key { key: event_key, pressed: true, repeat: false, modifiers, .. } if *event_key == key && modifiers.is_none()));
    input.events.retain(|e| !matches!(e, egui::Event::Key { key: event_key, pressed: true, .. } if *event_key == key));
    pressed
}
pub(super) fn take_tab(input: &mut egui::InputState) -> Option<bool> {
    let direction = input.events.iter().find_map(|event| match event {
        egui::Event::Key {
            key: Key::Tab,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } if !modifiers.alt && !modifiers.ctrl && !modifiers.command => Some(modifiers.shift),
        _ => None,
    });
    input.events.retain(|event| {
        !matches!(
            event,
            egui::Event::Key {
                key: Key::Tab,
                pressed: true,
                ..
            }
        )
    });
    direction
}
pub(super) fn take_delete_press(input: &mut egui::InputState) -> bool {
    take_fresh_press(input, Key::Delete)
}
impl Workshop {
    pub(super) fn begin_delete(&mut self) -> Result<(), String> {
        if self.image_dialog.is_some()
            || self.rename_dialog.is_some()
            || self.delete_dialog.is_some()
            || self.operation.as_ref().is_some_and(|j| j.running)
        {
            return Err("A file action or dialogue is already active".into());
        }
        let paths = self.action_paths();
        if paths.is_empty() {
            return Err("No files selected".into());
        }
        let targets = paths
            .into_iter()
            .map(photo_core::recycle::capture_target)
            .collect::<Vec<_>>();
        let gallery = self.gallery.as_ref().ok_or("No open gallery")?;
        let immediate = gallery.view() == View::Photo || targets.len() == 1;
        let dialog = Dialog {
            scope: gallery.selection_scope(),
            targets,
            order: gallery.photos().iter().map(|p| p.path.clone()).collect(),
            cursor: gallery
                .selected_index()
                .filter(|_| gallery.view() == View::Photo)
                .unwrap_or(gallery.grid_anchor()),
            revision: self.interaction_revision,
            frame: self.ctx.cumulative_frame_nr(),
            focus: 0,
            autofocus: true,
        };
        self.close_palette();
        self.delete_dialog = Some(dialog);
        if immediate {
            self.confirm_delete()?;
        }
        Ok(())
    }
    pub(super) fn confirm_delete(&mut self) -> Result<(), String> {
        let dialog = self
            .delete_dialog
            .as_ref()
            .ok_or("No delete confirmation is pending")?;
        let targets = dialog.targets.clone();
        let order = dialog.order.clone();
        let cursor = dialog.cursor;
        let revision = dialog.revision;
        // Loaded descendants are members too. The captured collection identity
        // rejects leaving/reopening even the same folder without retargeting.
        let gallery = self.gallery.as_ref().ok_or("No open gallery")?;
        if !dialog.scope.permits(
            gallery.collection_id(),
            targets.iter().map(|t| t.path.as_path()),
        ) {
            return Err("Delete confirmation belongs to another collection".into());
        }
        self.start_task(operations::TaskKind::Delete(targets))?;
        if let Some(origin) = &mut self.operation_origin {
            origin.revision = revision;
            origin.delete_order = order;
            origin.delete_cursor = cursor;
        }
        self.delete_dialog = None;
        Ok(())
    }
    pub(super) fn delete_snapshot(&self) -> Option<photo_core::api::DeleteState> {
        self.delete_dialog
            .as_ref()
            .map(|d| photo_core::api::DeleteState {
                paths: d.targets.iter().map(|t| t.path.clone()).collect(),
                count: d.targets.len(),
            })
    }
    pub(super) fn delete_popup(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.delete_dialog else {
            return;
        };
        let count = dialog.targets.len();
        let armed = ctx.cumulative_frame_nr() > dialog.frame;
        let tab = ctx.input_mut(take_tab);
        let enter = ctx.input_mut(|i| take_fresh_press(i, Key::Enter));
        if tab.is_some() {
            dialog.focus = (dialog.focus + 1) % 2;
        }
        let move_focus = tab.is_some() || std::mem::take(&mut dialog.autofocus);
        let mut confirm = false;
        let mut cancel = false;
        let response = egui::Modal::new(egui::Id::new("delete-files"))
            .frame(
                egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                    .corner_radius(12)
                    .inner_margin(16),
            )
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.heading(format!("Удалить файлов: {count}?"));
                ui.label("Файлы будут перемещены в корзину Windows.");
                ui.horizontal(|ui| {
                    let cancel_button = ui
                        .push_id("delete-cancel", |ui| ui.button("Отмена · Esc"))
                        .inner;
                    if move_focus && dialog.focus == 0 {
                        cancel_button.request_focus();
                    }
                    if !move_focus && cancel_button.has_focus() {
                        dialog.focus = 0;
                    }
                    cancel = cancel_button.clicked();
                    let delete_button = ui
                        .push_id("delete-confirm", |ui| {
                            ui.add(egui::Button::new(
                                egui::RichText::new("Удалить · Enter").color(Color32::LIGHT_RED),
                            ))
                        })
                        .inner;
                    if move_focus && dialog.focus == 1 {
                        delete_button.request_focus();
                    }
                    if !move_focus && delete_button.has_focus() {
                        dialog.focus = 1;
                    }
                    confirm = delete_button.clicked();
                });
                if armed && enter {
                    confirm = true;
                }
            });
        if confirm && armed {
            if let Some(error) = self.execute(Command::ConfirmDelete).error {
                self.message = error.message;
            }
        } else if cancel || (armed && response.should_close()) {
            self.delete_dialog = None;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modal_tab_consumes_forward_reverse_and_repeat() {
        for (modifiers, repeat, expected) in [
            (egui::Modifiers::NONE, false, Some(false)),
            (egui::Modifiers::SHIFT, false, Some(true)),
            (egui::Modifiers::NONE, true, None),
            (egui::Modifiers::CTRL, false, None),
        ] {
            let ctx = egui::Context::default();
            if repeat {
                // egui derives repeat from its held-key state; a standalone
                // raw repeat flag on the first keydown becomes a fresh press.
                ctx.begin_pass(egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: Key::Tab,
                        physical_key: Some(Key::Tab),
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                });
                assert_eq!(ctx.input_mut(take_tab), Some(false));
                ctx.end_pass().textures_delta.clear();
            }
            ctx.begin_pass(egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Tab,
                    physical_key: Some(Key::Tab),
                    pressed: true,
                    repeat,
                    modifiers,
                }],
                ..Default::default()
            });
            assert_eq!(ctx.input_mut(take_tab), expected);
            assert_eq!(ctx.input_mut(take_tab), None);
            assert!(!ctx.input(|i| i.events.iter().any(|event| matches!(
                event,
                egui::Event::Key {
                    key: Key::Tab,
                    pressed: true,
                    ..
                }
            ))));
            ctx.end_pass().textures_delta.clear();
        }
    }
    #[test]
    fn delete_and_enter_repeat_require_fresh_press() {
        for target in [Key::Delete, Key::Enter] {
            let ctx = egui::Context::default();
            let key = |repeat, modifiers| egui::Event::Key {
                key: target,
                physical_key: Some(target),
                pressed: true,
                repeat,
                modifiers,
            };
            ctx.begin_pass(egui::RawInput {
                events: vec![key(false, egui::Modifiers::NONE)],
                ..Default::default()
            });
            assert!(ctx.input_mut(|i| take_fresh_press(i, target)));
            assert!(!ctx.input_mut(|i| take_fresh_press(i, target)));
            ctx.end_pass().textures_delta.clear();
            ctx.begin_pass(egui::RawInput {
                events: vec![
                    key(true, egui::Modifiers::NONE),
                    key(false, egui::Modifiers::SHIFT),
                ],
                ..Default::default()
            });
            assert!(!ctx.input_mut(|i| take_fresh_press(i, target)));
            ctx.end_pass().textures_delta.clear();
        }
    }
    #[test]
    fn next_then_previous_and_empty() {
        let order: Vec<PathBuf> = ["a", "b", "c", "d"].into_iter().map(Into::into).collect();
        let remaining = [order[0].clone(), order[3].clone()].into_iter().collect();
        assert_eq!(successor(&order, 1, &remaining), Some(order[3].clone()));
        assert_eq!(
            successor(&order, 3, &[order[0].clone()].into_iter().collect()),
            Some(order[0].clone())
        );
        assert_eq!(successor(&order, 0, &Default::default()), None);
    }
}
