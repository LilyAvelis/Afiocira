use super::*;

#[derive(Default)]
pub(super) struct State {
    pending: Option<Pending>,
    ready_snapshot: Option<yazi::NavigatorSnapshot>,
    deferred_reply: bool,
    mark_pending: Option<(u64, u64, Vec<PathBuf>)>,
}
struct Pending {
    command: Command,
    id: u64,
    deadline: Instant,
    revision: u64,
    reply: Option<mpsc::Sender<Response>>,
}
impl State {
    pub(super) fn busy(&self) -> bool {
        self.pending.is_some()
    }
}

pub(super) fn needs_snapshot(command: &Command) -> bool {
    use photo_core::actions::ActionId;
    matches!(
        command,
        Command::NavigatorSnapshot
            | Command::FocusPane {
                pane: Pane::Gallery
            }
            | Command::SetNavigatorVisible { visible: false }
            | Command::ShowCommands
            | Command::BeginRename
            | Command::Rename { .. }
            | Command::RunAction {
                id: ActionId::Copy
                    | ActionId::Duplicate
                    | ActionId::Paste
                    | ActionId::Rename
                    | ActionId::Delete
            }
    )
}

fn cancels_continuation(cancel: &Command, pending: &Command) -> bool {
    use photo_core::actions::ActionId;
    match cancel {
        Command::CancelRename => matches!(
            pending,
            Command::Rename { .. }
                | Command::BeginRename
                | Command::RunAction {
                    id: ActionId::Rename
                }
        ),
        Command::CancelDelete => matches!(
            pending,
            Command::RunAction {
                id: ActionId::Delete
            }
        ),
        Command::CloseCommands => matches!(pending, Command::ShowCommands),
        _ => false,
    }
}

impl Workshop {
    pub(super) fn navigator_pending_request(&self) -> Option<u64> {
        self.navigation
            .pending
            .as_ref()
            .map(|pending| pending.id)
            .or_else(|| self.yazi.as_ref().and_then(YaziPane::pending_request_id))
    }
    pub(super) fn navigation_dispatch_started(&mut self) {
        self.navigation.deferred_reply = false;
    }
    pub(super) fn cancel_navigator_continuation(&mut self, command: &Command) {
        if !self
            .navigation
            .pending
            .as_ref()
            .is_some_and(|pending| cancels_continuation(command, &pending.command))
        {
            return;
        }
        let pending = self.navigation.pending.take().unwrap();
        if let Some(yazi) = &mut self.yazi {
            yazi.cancel_snapshot();
        }
        if let Some(reply) = pending.reply {
            let _ = reply.send(Response::error(
                ErrorCode::InvalidState,
                "Pending navigator command was canceled",
                None,
            ));
        }
    }
    pub(super) fn queue_navigator_command(&mut self, command: Command) -> Result<(), String> {
        if self.yazi_settings.busy() {
            return Err("Yazi settings apply is in progress".into());
        }
        if self.navigation.pending.is_some() {
            return Err("A navigator handoff is already pending".into());
        }
        let yazi = self.yazi.as_mut().ok_or("Navigator unavailable")?;
        if !yazi.is_running() {
            return Err("Navigator has exited; restart Yazi".into());
        }
        let id = yazi.begin_snapshot()?;
        self.navigation.pending = Some(Pending {
            command,
            id,
            deadline: Instant::now() + Duration::from_secs(2),
            revision: self.interaction_revision,
            reply: None,
        });
        self.navigation.deferred_reply = true;
        self.ctx.request_repaint_after(Duration::from_millis(25));
        Ok(())
    }
    pub(super) fn defer_navigator_reply(
        &mut self,
        reply: mpsc::Sender<Response>,
        deadline: Instant,
    ) -> bool {
        if !std::mem::take(&mut self.navigation.deferred_reply) {
            return false;
        }
        if let Some(pending) = &mut self.navigation.pending {
            pending.reply = Some(reply);
            pending.deadline = pending.deadline.min(deadline);
            return true;
        }
        false
    }
    pub(super) fn poll_navigator(&mut self) {
        if let Some(yazi) = &mut self.yazi {
            yazi.poll_background();
        }
        let Some(pending) = self.navigation.pending.take() else {
            return;
        };
        let error = if Instant::now() >= pending.deadline {
            Some(format!("Yazi selection request {} timed out", pending.id))
        } else if pending.revision != self.interaction_revision {
            Some("Navigation changed during the pending handoff; retry".into())
        } else {
            self.yazi.as_mut().and_then(YaziPane::take_navigation_error)
        };
        let result = if let Some(error) = error {
            if let Some(yazi) = &mut self.yazi {
                yazi.cancel_snapshot();
            }
            Err(error)
        } else if let Some(snapshot) = self.yazi.as_mut().and_then(YaziPane::poll_snapshot) {
            if matches!(pending.command, Command::NavigatorSnapshot) {
                self.navigation.ready_snapshot = Some(snapshot);
                Ok(())
            } else {
                self.import_navigator_snapshot(snapshot)
            }
        } else {
            self.navigation.pending = Some(pending);
            self.ctx.request_repaint_after(Duration::from_millis(25));
            return;
        };
        let response = match result {
            Ok(()) => self.execute_ready(pending.command),
            Err(error) => {
                self.message = error.clone();
                Response::error(ErrorCode::InvalidState, error, None)
            }
        };
        if let Some(reply) = pending.reply {
            let _ = reply.send(response);
        }
    }
    pub(super) fn queue_mark_sync(&mut self) {
        self.mark_sync_after = Some(Instant::now() + Duration::from_millis(60));
        self.mark_sync_attempts = 0;
        self.ctx.request_repaint_after(Duration::from_millis(60));
    }

    pub(super) fn poll_mark_sync(&mut self) {
        if self.yazi_settings.busy() {
            return;
        }
        if let Some((id, collection, published)) = &self.navigation.mark_pending {
            let applied = self
                .yazi
                .as_ref()
                .is_some_and(|yazi| yazi.selection_applied(*id));
            if !applied {
                self.ctx.request_repaint_after(Duration::from_millis(25));
                return;
            }
            let current: Vec<_> = self
                .gallery
                .as_ref()
                .map(|g| g.marked_paths().iter().cloned().collect())
                .unwrap_or_default();
            if self
                .gallery
                .as_ref()
                .is_some_and(|g| g.collection_id() == *collection)
                && current == *published
            {
                self.selection_dirty = false;
            }
            self.navigation.mark_pending = None;
        }
        let Some(deadline) = self.mark_sync_after else {
            return;
        };
        if !self.selection_dirty {
            self.mark_sync_after = None;
            return;
        }
        if Instant::now() < deadline {
            return;
        }
        self.mark_sync_after = None;
        let (Some(gallery), Some(yazi)) = (&self.gallery, &mut self.yazi) else {
            return;
        };
        if !yazi.is_running() {
            return;
        }
        let Some(cursor) = gallery.cursor_photo() else {
            return;
        };
        let marks: Vec<_> = gallery.marked_paths().iter().cloned().collect();
        let loaded: Vec<_> = gallery
            .photos()
            .iter()
            .map(|photo| photo.path.clone())
            .collect();
        match yazi.apply_gallery_selection(&cursor.path, &marks, true, &loaded) {
            Ok(id) => {
                self.navigation.mark_pending = Some((id, gallery.collection_id(), marks));
                self.mark_sync_after = Some(Instant::now() + Duration::from_millis(25));
                self.ctx.request_repaint_after(Duration::from_millis(25));
            }
            Err(_) if self.mark_sync_attempts < 8 => {
                self.mark_sync_attempts += 1;
                self.mark_sync_after = Some(Instant::now() + Duration::from_millis(250));
                self.ctx.request_repaint_after(Duration::from_millis(250));
            }
            Err(error) => {
                self.message = format!("Yazi: {error}. Tab повторит синхронизацию.");
            }
        }
    }

    pub(super) fn navigator_snapshot(&mut self) -> Result<yazi::NavigatorSnapshot, String> {
        self.navigation
            .ready_snapshot
            .take()
            .ok_or_else(|| "A fresh navigator snapshot is required".into())
    }
    pub(super) fn focus_gallery(&mut self) -> Result<(), String> {
        if self.active_pane == ActivePane::Yazi {
            return self.queue_navigator_command(Command::FocusPane {
                pane: Pane::Gallery,
            });
        }
        self.finish_gallery_focus(false)
    }
    fn import_navigator_snapshot(
        &mut self,
        snapshot: yazi::NavigatorSnapshot,
    ) -> Result<(), String> {
        let mut restored = false;
        if self.yazi.as_mut().is_some_and(YaziPane::is_running) {
            restored = self.album_mode()
                && self.nested.returned_folder.as_ref().is_some_and(|folder| {
                    restore_album_overview(
                        folder,
                        &snapshot.folder,
                        snapshot.hovered.as_deref(),
                        self.yazi
                            .as_ref()
                            .is_some_and(YaziPane::navigation_untouched),
                    )
                });
            if !restored {
                self.nested.returned_folder = None;
                let target_folder = snapshot
                    .hovered
                    .as_deref()
                    .filter(|p| p.is_dir())
                    .unwrap_or(&snapshot.folder);
                // Merely belonging to the old expanded tree is not a reason to
                // hold that overview after the user actually navigates in Yazi.
                if self
                    .gallery
                    .as_ref()
                    .is_none_or(|g| !same_folder(g.folder(), target_folder))
                    && !self.open_path(target_folder)
                {
                    return Err(self.message.clone());
                }
                let root = self.gallery.as_ref().unwrap().folder().to_path_buf();
                if same_folder(&root, &snapshot.folder) {
                    self.execute(Command::SetOrder {
                        paths: navigator_paths_in_gallery(&root, snapshot.ordered),
                    });
                }
                if let Some(g) = &mut self.gallery {
                    g.set_marked_paths(navigator_paths_in_gallery(&root, snapshot.marked));
                    if let Some(yazi) = &self.yazi {
                        yazi.remember_imported_selection(
                            &g.marked_paths().iter().cloned().collect::<Vec<_>>(),
                        );
                    }
                    if let Some(path) = snapshot.hovered.as_ref().filter(|p| p.is_file()) {
                        if let Some(path) = navigator_paths_in_gallery(&root, [path.clone()]).pop()
                        {
                            g.select_path(&path);
                        }
                        // A file cursor means the ordinary local grid, even if
                        // the directory also happens to contain child folders.
                        self.nested.force_grid = true;
                    }
                }
                self.selection_dirty = false;
            }
        }
        self.finish_gallery_focus(restored)
    }
    fn finish_gallery_focus(&mut self, restored: bool) -> Result<(), String> {
        self.nested.returned_folder = None;
        self.active_pane = ActivePane::Gallery;
        if !self.nested.snapshot.expanded && !self.nested.force_grid {
            if self.album_mode() {
                self.activate_album();
            } else {
                let _ = self.load_nested(false);
            }
        }
        self.scroll_to_grid_anchor = !restored;
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        Ok(())
    }
    pub(super) fn focus_navigator(&mut self) -> Result<(), String> {
        let album = self
            .gallery
            .as_ref()
            .filter(|g| g.view() == View::Grid)
            .filter(|_| self.album_mode())
            .and_then(|_| self.current_album())
            .map(|(folder, _)| folder);
        let yazi = self.yazi.as_mut().ok_or("Navigator unavailable")?;
        if !yazi.is_running() {
            return Err("Navigator has exited; restart Yazi".into());
        }
        if let Some(folder) = album {
            if self.selection_dirty {
                let marks: Vec<_> = self
                    .gallery
                    .as_ref()
                    .unwrap()
                    .marked_paths()
                    .iter()
                    .cloned()
                    .collect();
                let loaded: Vec<_> = self
                    .gallery
                    .as_ref()
                    .unwrap()
                    .photos()
                    .iter()
                    .map(|p| p.path.clone())
                    .collect();
                let id = yazi.apply_gallery_selection(&folder, &marks, true, &loaded)?;
                self.navigation.mark_pending =
                    Some((id, self.gallery.as_ref().unwrap().collection_id(), marks));
            }
            yazi.reveal_folder(&folder)?;
            self.nested.returned_folder = Some(folder);
        } else {
            self.nested.returned_folder = None;
            if let Some(g) = &self.gallery
                && let Some(cursor) = g.cursor_photo()
            {
                let marks: Vec<_> = g.marked_paths().iter().cloned().collect();
                let loaded: Vec<_> = g.photos().iter().map(|p| p.path.clone()).collect();
                let id = yazi.apply_gallery_selection(
                    &cursor.path,
                    &marks,
                    self.selection_dirty,
                    &loaded,
                )?;
                if self.selection_dirty {
                    self.navigation.mark_pending = Some((id, g.collection_id(), marks));
                }
            }
        }
        self.mark_sync_after = self
            .selection_dirty
            .then(|| Instant::now() + Duration::from_millis(25));
        self.show_yazi = true;
        self.active_pane = ActivePane::Yazi;
        yazi.request_focus();
        Ok(())
    }
    pub(super) fn follow_navigator_folder(&mut self, folder: PathBuf) {
        if self.navigation.pending.is_some() {
            return;
        }
        if self.active_pane != ActivePane::Yazi {
            return;
        }
        let roundtrip = self.nested.returned_folder.as_ref().is_some_and(|album| {
            same_folder(album, &folder) || album.parent().is_some_and(|p| same_folder(p, &folder))
        });
        if roundtrip {
            return;
        }
        if self
            .gallery
            .as_ref()
            .is_none_or(|g| !same_folder(g.folder(), &folder))
        {
            self.nested.returned_folder = None;
            self.execute(Command::PreviewFolder { path: folder });
        }
    }

    pub(super) fn restart_yazi(&mut self) {
        if let Some(pending) = self.navigation.pending.take()
            && let Some(reply) = pending.reply
        {
            let _ = reply.send(Response::error(
                ErrorCode::InvalidState,
                "Navigator restarted during pending handoff",
                None,
            ));
        }
        self.navigation = State::default();
        let folder = self
            .gallery
            .as_ref()
            .map_or_else(default_folder, |gallery| gallery.folder().to_path_buf());
        match YaziPane::start(&folder) {
            Ok(pane) => {
                self.yazi = Some(pane);
                self.yazi_error = None;
                self.active_pane = ActivePane::Yazi;
                if self.gallery.is_some() {
                    self.selection_dirty = true;
                    self.queue_mark_sync();
                }
            }
            Err(error) => {
                self.yazi = None;
                self.yazi_error = Some(error);
            }
        }
    }
}

// Only our exact folder handoff may restore the overview. A real cd into that
// folder (or elsewhere) must be allowed to open the corresponding local view.
fn restore_album_overview(
    album: &Path,
    cwd: &Path,
    hovered: Option<&Path>,
    untouched: bool,
) -> bool {
    untouched
        || (album.parent().is_some_and(|p| same_folder(p, cwd))
            && hovered.is_some_and(|p| same_folder(album, p)))
}
#[cfg(test)]
mod album_return_tests {
    use super::*;
    #[test]
    fn explicit_cancel_matches_only_its_pending_continuations() {
        use photo_core::actions::ActionId;
        for rename in [
            Command::BeginRename,
            Command::Rename {
                new_name: "renamed.png".into(),
            },
            Command::RunAction {
                id: ActionId::Rename,
            },
        ] {
            assert!(cancels_continuation(&Command::CancelRename, &rename));
            assert!(!cancels_continuation(&Command::Status, &rename));
            assert!(!cancels_continuation(&Command::CancelDelete, &rename));
        }
        assert!(cancels_continuation(
            &Command::CancelDelete,
            &Command::RunAction {
                id: ActionId::Delete
            }
        ));
        assert!(cancels_continuation(
            &Command::CloseCommands,
            &Command::ShowCommands
        ));
        for read in [
            Command::NavigatorSnapshot,
            Command::FocusPane {
                pane: Pane::Gallery,
            },
        ] {
            for unrelated in [
                Command::Status,
                Command::CancelRename,
                Command::CancelDelete,
                Command::CloseCommands,
            ] {
                assert!(!cancels_continuation(&unrelated, &read));
            }
        }
    }
    #[test]
    fn folder_handoff_restores_but_entering_or_leaving_does_not() {
        let root = std::env::temp_dir().join(format!("afio-album-return-{}", std::process::id()));
        let album = root.join("album");
        std::fs::create_dir_all(&album).unwrap();
        assert!(restore_album_overview(&album, &root, Some(&album), false));
        assert!(!restore_album_overview(
            &album,
            &album,
            Some(&album.join("photo.png")),
            false
        ));
        assert!(!restore_album_overview(
            &album,
            &root,
            Some(&root.join("other")),
            false
        ));
        assert!(restore_album_overview(&album, &album, None, true));
        std::fs::remove_dir_all(root).unwrap();
    }
}
