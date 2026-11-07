use super::*;

impl Workshop {
    pub(super) fn queue_mark_sync(&mut self) {
        self.mark_sync_after = Some(Instant::now() + Duration::from_millis(60));
        self.mark_sync_attempts = 0;
        self.ctx.request_repaint_after(Duration::from_millis(60));
    }

    pub(super) fn poll_mark_sync(&mut self) {
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
        match yazi.apply_selection(&cursor.path, &marks, true) {
            Ok(()) => {
                self.selection_dirty = false;
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
        let yazi = self.yazi.as_mut().ok_or("Navigator unavailable")?;
        if !yazi.is_running() {
            return Err("Navigator has exited; restart Yazi".into());
        }
        yazi.request_snapshot()?;
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(snapshot) = yazi.poll_snapshot() {
                return Ok(snapshot);
            }
            if Instant::now() >= deadline {
                return Err("Yazi selection timed out".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    pub(super) fn focus_gallery(&mut self) -> Result<(), String> {
        let mut restored = false;
        if self.yazi.as_mut().is_some_and(YaziPane::is_running) {
            let snapshot = self.navigator_snapshot()?;
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
                yazi.apply_selection(&folder, &marks, true)?;
            }
            yazi.reveal_folder(&folder)?;
            self.nested.returned_folder = Some(folder);
        } else {
            self.nested.returned_folder = None;
            if let Some(g) = &self.gallery
                && let Some(cursor) = g.cursor_photo()
            {
                let marks: Vec<_> = g.marked_paths().iter().cloned().collect();
                yazi.apply_selection(&cursor.path, &marks, self.selection_dirty)?;
            }
        }
        self.selection_dirty = false;
        self.mark_sync_after = None;
        self.show_yazi = true;
        self.active_pane = ActivePane::Yazi;
        yazi.request_focus();
        Ok(())
    }
    pub(super) fn follow_navigator_folder(&mut self, folder: PathBuf) {
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
        let folder = self
            .gallery
            .as_ref()
            .map_or_else(default_folder, |gallery| gallery.folder().to_path_buf());
        match YaziPane::start(&folder) {
            Ok(pane) => {
                self.yazi = Some(pane);
                self.yazi_error = None;
                self.active_pane = ActivePane::Yazi;
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
