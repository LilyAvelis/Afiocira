//! Application action service: every entry point reaches this dispatcher.
use super::*;
use photo_core::actions::{ActionId, ActionInfo, Context};
impl Workshop {
    pub(super) fn available_actions(&self) -> Vec<ActionInfo> {
        let gallery = self.gallery.as_ref();
        let mut actions = photo_core::actions::available(&Context {
            album_overview: self.album_mode() && gallery.is_some_and(|g| g.view() == View::Grid),
            space: gallery.is_some_and(|g| g.view() == View::Photo),
            targets: self.action_paths().len(),
            video: gallery
                .and_then(Gallery::selected_photo)
                .is_some_and(|p| p.is_video()),
            busy: self.operation.as_ref().is_some_and(|j| j.running),
            has_gallery: gallery.is_some(),
            marked: gallery.is_some_and(|g| !g.marked_paths().is_empty()),
        });
        let context = self.shortcut_context();
        for action in &mut actions {
            action.shortcut = shortcuts::effective(
                &self.settings.shortcuts,
                context,
                action.id,
                &action.shortcut,
            );
        }
        actions
    }
    pub(super) fn run_action(&mut self, id: ActionId) -> Result<(), String> {
        if id != ActionId::Settings && (self.image_dialog.is_some() || self.search.snapshot.open) {
            return Err("Close Everything search first".into());
        }
        if id != ActionId::Settings && self.delete_dialog.is_some() {
            return Err("Confirm or cancel deletion first".into());
        }
        if !self.palette.open
            && self.active_pane == ActivePane::Yazi
            && matches!(
                id,
                ActionId::Copy
                    | ActionId::Duplicate
                    | ActionId::Paste
                    | ActionId::Rename
                    | ActionId::Delete
            )
        {
            return self.queue_navigator_command(Command::RunAction { id });
        }
        if !self.available_actions().iter().any(|a| a.id == id) {
            return Err("Action is not available in this context".into());
        }
        self.close_palette();
        match id {
            ActionId::RotateLeft
            | ActionId::RotateRight
            | ActionId::FlipHorizontal
            | ActionId::FlipVertical => {
                let source = self
                    .gallery
                    .as_ref()
                    .and_then(Gallery::selected_photo)
                    .ok_or("Open an image first")?
                    .path
                    .clone();
                let version = photo_core::rename::capture(&source).map_err(|e| e.to_string())?;
                self.start_task(operations::TaskKind::Transform {
                    source,
                    version,
                    action: id,
                })
            }
            ActionId::CopyVideoFrame | ActionId::SaveVideoFrame => {
                let source = self
                    .gallery
                    .as_ref()
                    .and_then(Gallery::selected_photo)
                    .ok_or("Open a video first")?
                    .path
                    .clone();
                let version = photo_core::rename::capture(&source).map_err(|e| e.to_string())?;
                let capture = self
                    .video
                    .as_ref()
                    .ok_or("Video engine unavailable")?
                    .lock()
                    .map_err(|_| "Video engine unavailable")?
                    .capture_frame(&source)?;
                self.start_task(operations::TaskKind::VideoFrame {
                    source,
                    version,
                    action: id,
                    capture,
                })
            }
            ActionId::Copy | ActionId::Duplicate | ActionId::Paste => self.start_operation(id),
            ActionId::EnterAlbum => self.enter_album(),
            ActionId::Open if self.album_mode() => {
                self.album_open();
                Ok(())
            }
            ActionId::ResizeImage => self.begin_image(false),
            ActionId::ConvertImage => self.begin_image(true),
            ActionId::Search => self.open_search(),
            ActionId::Rename => self.begin_rename(),
            ActionId::Delete => self.begin_delete(),
            _ => {
                let command = match id {
                    ActionId::Open => Command::OpenSelected,
                    ActionId::Settings => Command::ShowSettings,
                    ActionId::Back => Command::Back,
                    ActionId::Next => Command::Next,
                    ActionId::Previous => Command::Previous,
                    ActionId::ClearMarks => Command::SetMarked { paths: Vec::new() },
                    ActionId::Refresh => Command::Refresh,
                    ActionId::Fit => Command::ResetZoom,
                    ActionId::PlayPause => Command::VideoPause {
                        paused: !self
                            .video
                            .as_ref()
                            .is_some_and(|v| v.lock().unwrap().state.paused),
                    },
                    _ => unreachable!(),
                };
                self.execute(command)
                    .error
                    .map_or(Ok(()), |error| Err(error.message))
            }
        }
    }
    pub(super) fn open_palette(&mut self) -> Result<(), String> {
        if self.image_dialog.is_some() || self.search.snapshot.open {
            return Err("Close Everything search first".into());
        }
        if self.delete_dialog.is_some() {
            return Err("Confirm or cancel deletion first".into());
        }
        if self.active_pane == ActivePane::Yazi {
            return self.queue_navigator_command(Command::ShowCommands);
        }
        self.palette.open = true;
        self.palette.query.clear();
        self.palette.selected = 0;
        self.palette.focus_search = true;
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        Ok(())
    }
    pub(super) fn close_palette(&mut self) {
        self.palette.open = false;
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }
    pub(super) fn global_action_keys(&mut self, ctx: &egui::Context) {
        // Drain the native observer even in text fields; do not retain a paste
        // press to accidentally import after a dialogue closes.
        let native_paste = self
            .native_keys
            .as_ref()
            .is_some_and(|hook| hook.take_paste());
        // The hook is shared by this process's windows. A paste in Settings
        // must never become a media import in the background gallery.
        if !ctx.input(|i| i.focused) {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, Key::Comma)) {
            if let Some(error) = self.execute(Command::ShowSettings).error {
                self.message = error.message;
            }
            return;
        }
        if self.image_dialog.is_some()
            || self.rename_dialog.is_some()
            || self.delete_dialog.is_some()
        {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, Key::K))
        {
            if self.image_dialog.is_some() || self.search.snapshot.open {
                self.close_search();
            } else if let Err(error) = self.open_search() {
                self.message = error;
            }
            return;
        }
        if self.image_dialog.is_some() || self.search.snapshot.open {
            return;
        }
        let native_fallback = self.native_keys.is_none() && ctx.input(|i| i.events.iter().any(|e| matches!(e,egui::Event::Key { key:Key::V, pressed:false, modifiers, .. } if modifiers.ctrl && !modifiers.alt && !modifiers.shift)));
        // The fallback triggers only once, on release; do not also react to
        // text Paste events or a slow tap could import the same payload twice.
        let media_paste = native_paste || native_fallback;
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, Key::K)) {
            let command = if self.palette.open {
                Command::CloseCommands
            } else {
                Command::ShowCommands
            };
            let result = self.execute(command);
            if let Some(error) = result.error {
                self.message = error.message;
            }
            return;
        }
        if self.palette.open {
            return;
        }
        if ctx.memory(|m| m.focused() == Some(egui::Id::new("folder-path"))) {
            return;
        }
        // Text fields keep their own Copy/Ctrl+D behaviour. Terminal focus is
        // special: these media commands first import Yazi's actual selection.
        if self.active_pane != ActivePane::Yazi && ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let actions = self.available_actions();
        let in_yazi = self.active_pane == ActivePane::Yazi;
        let action =
            ctx.input_mut(|input| shortcuts::take_action(input, actions, in_yazi, media_paste));
        if let Some(id) = action
            && let Some(error) = self.execute(Command::RunAction { id }).error
        {
            self.message = error.message;
        }
    }
}
