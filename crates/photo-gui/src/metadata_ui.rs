//! Metadata panel integration through the shared command dispatcher.
use super::*;
use photo_core::metadata::{Metadata, MetadataRequest};

fn take_metadata_escape(ctx: &egui::Context) -> bool {
    let fresh = ctx.input(|i| i.events.iter().any(|e| matches!(e,
        egui::Event::Key { key: Key::Escape, pressed: true, repeat: false, modifiers, .. } if modifiers.is_none()
    )));
    if fresh {
        ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, Key::Escape);
        });
    }
    fresh
}

impl Workshop {
    pub(super) fn metadata_escape(&mut self, ctx: &egui::Context) -> bool {
        if !self.metadata_panel.is_open()
            || self.palette.open
            || self.search.snapshot.open
            || self.image_dialog.is_some()
            || self.adjustments_panel.is_some()
            || self.rename_dialog.is_some()
            || self.delete_dialog.is_some()
        {
            return false;
        }
        let fresh = take_metadata_escape(ctx);
        if fresh {
            self.execute(Command::CancelMetadata);
        }
        fresh
    }
    pub(super) fn cancel_metadata(&mut self) {
        self.metadata_panel.cancel();
        self.active_pane = ActivePane::Gallery;
        self.ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }
    fn metadata_target(&self) -> Result<PathBuf, String> {
        let gallery = self.gallery.as_ref().ok_or("No open gallery")?;
        let photo = match gallery.view() {
            View::Photo => gallery.selected_photo(),
            View::Grid => gallery.cursor_photo(),
        }
        .ok_or("No current image")?;
        if photo.is_video() {
            return Err("Metadata requires a still image".into());
        }
        Ok(photo.path.clone())
    }
    fn dispatch_panel_metadata(&mut self, request: MetadataRequest) -> Result<(), String> {
        let response = self.execute(Command::Metadata { request });
        if let Some(error) = response.error {
            self.metadata_panel.submission_failed(error.message.clone());
            return Err(error.message);
        }
        if let Some(job) = response.metadata {
            self.metadata_panel.attach_request(&job);
            Ok(())
        } else {
            let error = "Metadata dispatcher returned no receipt".to_owned();
            self.metadata_panel.submission_failed(error.clone());
            Err(error)
        }
    }
    pub(super) fn open_metadata(&mut self) -> Result<(), String> {
        if self.image_dialog.is_some()
            || self.adjustments_panel.is_some()
            || self.rename_dialog.is_some()
            || self.delete_dialog.is_some()
            || self.operation.as_ref().is_some_and(|o| o.running)
        {
            return Err("Сначала завершите текущий диалог или операцию".into());
        }
        self.active_pane = ActivePane::Gallery;
        self.history_panel.open = false;
        self.history_panel.preview = None;
        let path = self.metadata_target()?;
        if let Some(request) = self.metadata_panel.open(path.clone()) {
            self.dispatch_panel_metadata(request)
        } else if self.metadata_panel.target() == Some(path.as_path()) {
            Ok(())
        } else {
            Err("Save or discard the metadata draft before changing its target".into())
        }
    }
    pub(super) fn close_metadata(&mut self) -> Result<(), String> {
        if self.metadata_panel.close() {
            Ok(())
        } else {
            Err("Save or discard metadata changes and wait for the pending request".into())
        }
    }
    pub(super) fn finish_metadata(&mut self) -> Result<(), String> {
        if let Some(request) = self.metadata_panel.finish()? {
            self.dispatch_panel_metadata(request)?;
        }
        Ok(())
    }
    pub(super) fn submit_panel_metadata(&mut self, metadata: Metadata) -> Result<(), String> {
        self.metadata_panel.set_document(metadata)?;
        let request = self.metadata_panel.save_request()?;
        self.dispatch_panel_metadata(request)
    }
    pub(super) fn discard_panel_metadata(&mut self) -> Result<(), String> {
        if self.metadata_panel.discard() {
            Ok(())
        } else {
            Err("Metadata request is pending".into())
        }
    }
    pub(super) fn poll_metadata_panel(&mut self) {
        if let Some(id) = self.metadata_panel.pending_request_id() {
            match self.metadata.status(id) {
                Ok(job) => self.metadata_panel.apply_job(&job),
                Err(error) => self.metadata_panel.request_failed(error),
            }
        }
    }
    pub(super) fn render_metadata_panel(&mut self, ui: &mut egui::Ui) {
        if !self.metadata_panel.is_open() {
            return;
        }
        if !self.metadata_panel.is_dirty()
            && self.metadata_panel.pending_request_id().is_none()
            && let Ok(path) = self.metadata_target()
            && let Some(request) = self.metadata_panel.open(path)
        {
            let _ = self.dispatch_panel_metadata(request);
        }
        for intent in self.metadata_panel.render(ui) {
            match intent {
                metadata_panel::Intent::Request(request) => {
                    let _ = self.dispatch_panel_metadata(request);
                }
                metadata_panel::Intent::Closed => self.cancel_metadata(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escape_is_owned_even_when_text_or_terminal_has_focus() {
        for text in [false, true] {
            let ctx = egui::Context::default();
            ctx.begin_pass(egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Escape,
                    physical_key: Some(Key::Escape),
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            });
            let focus = egui::Id::new("previous-terminal-or-text-focus");
            ctx.memory_mut(|memory| memory.request_focus(focus));
            if text {
                egui::text_edit::TextEditState::default().store(&ctx, focus);
            }
            assert!(take_metadata_escape(&ctx));
            assert!(!ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)));
            ctx.end_pass().textures_delta.clear();
        }
    }
}
