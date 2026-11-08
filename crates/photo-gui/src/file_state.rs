use super::*;

impl Workshop {
    pub(super) fn open_path(&mut self, path: &Path) -> bool {
        let result = if path.is_file() {
            Gallery::open_file(path)
        } else {
            Gallery::open(path)
        };
        match result {
            Ok(gallery) => {
                self.install_gallery(gallery);
                true
            }
            Err(error) => {
                self.message = format!("Could not open: {error}");
                false
            }
        }
    }

    pub(super) fn install_gallery(&mut self, gallery: Gallery) {
        self.install_gallery_cache(gallery, false);
    }
    pub(super) fn install_local_gallery(&mut self, gallery: Gallery) {
        self.install_gallery_cache(gallery, true);
    }
    fn install_gallery_cache(&mut self, gallery: Gallery, preserve: bool) {
        let old_paths: Vec<_> = self
            .gallery
            .as_ref()
            .map(|g| g.photos().iter().map(|p| p.path.clone()).collect())
            .unwrap_or_default();
        self.folder_input = friendly_path(gallery.folder());
        self.message = format!("{} items", gallery.photos().len());
        self.gallery = Some(gallery);
        self.selection_dirty = false;
        self.mark_sync_after = None;
        if preserve {
            self.remap_cached_images(&old_paths);
        } else {
            self.generation += 1;
            self.thumbnail_tasks.set_generation(self.generation);
            self.preview_tasks.set_generation(self.generation);
            self.clear_cached_images();
        }
        let folder = self.gallery.as_ref().unwrap().folder().to_path_buf();
        self.watch_folder(&folder);
        self.nested.reset(folder);
        self.scroll_to_grid_anchor = true;
    }

    pub(super) fn refresh_now(&mut self) -> Result<(), String> {
        self.nested.recount = self.gallery.as_ref().map(|g| g.folder().to_path_buf());
        let gallery = self.gallery.as_mut().ok_or("No open gallery")?;
        gallery.refresh().map_err(|error| error.to_string())?;
        self.generation += 1;
        self.thumbnail_tasks.set_generation(self.generation);
        self.preview_tasks.set_generation(self.generation);
        self.clear_cached_images();
        self.scroll_to_grid_anchor = true;
        self.message = format!("{} items", self.gallery.as_ref().unwrap().photos().len());
        Ok(())
    }

    /// Deletion does not modify surviving pixels. Keep cached neighbours by
    /// path identity rather than making the next already-prefetched frame cold.
    pub(super) fn refresh_after_delete(&mut self) -> Result<(), String> {
        let gallery = self.gallery.as_mut().ok_or("No open gallery")?;
        let old_paths: Vec<_> = gallery.photos().iter().map(|p| p.path.clone()).collect();
        gallery.refresh().map_err(|e| e.to_string())?;
        self.remap_cached_images(&old_paths);
        self.message = format!("{} items", self.gallery.as_ref().unwrap().photos().len());
        Ok(())
    }

    pub(super) fn watch_folder(&mut self, folder: &Path) {
        let (sender, receiver) = mpsc::channel();
        match notify::recommended_watcher(sender).and_then(|mut watcher| {
            watcher.watch(folder, RecursiveMode::NonRecursive)?;
            Ok(watcher)
        }) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.file_events = Some(receiver);
            }
            Err(error) => {
                self.watcher = None;
                self.file_events = None;
                self.message = format!("File changes will need manual refresh: {error}");
            }
        }
        self.refresh_after = None;
        self.dirty_names.clear();
        self.dirty_all = false;
    }

    pub(super) fn poll_file_changes(&mut self) {
        if let Some(events) = &self.file_events {
            let mut saw_event = false;
            for event in events.try_iter() {
                let event = match event {
                    Ok(mut event) => {
                        if matches!(event.kind, notify::EventKind::Access(_)) {
                            continue;
                        }
                        let membership = matches!(
                            event.kind,
                            notify::EventKind::Create(_)
                                | notify::EventKind::Remove(_)
                                | notify::EventKind::Modify(notify::event::ModifyKind::Name(_))
                        );
                        let prefix = format!(".photo-convert-{}-", std::process::id());
                        event.paths.retain(|path| {
                            if path
                                .file_name()
                                .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
                            {
                                return false;
                            }
                            !(membership && operations::own_transform_membership(path))
                        });
                        if event.paths.is_empty() {
                            continue;
                        }
                        Ok(event)
                    }
                    Err(error) => Err(error),
                };
                saw_event = true;
                match event {
                    Ok(event) if !event.paths.is_empty() => {
                        // A create/rename/remove on an existing path can be an atomic
                        // replacement. Invalidate that path too; unaffected paths
                        // retain their ready textures through the refresh.
                        let pixels_changed = matches!(
                            event.kind,
                            notify::EventKind::Any
                                | notify::EventKind::Other
                                | notify::EventKind::Create(_)
                                | notify::EventKind::Remove(_)
                                | notify::EventKind::Modify(
                                    notify::event::ModifyKind::Any
                                        | notify::event::ModifyKind::Name(_)
                                        | notify::event::ModifyKind::Data(_)
                                        | notify::event::ModifyKind::Metadata(_)
                                )
                        );
                        for path in event.paths {
                            if !pixels_changed {
                                continue;
                            }
                            if let Some(name) = path.file_name() {
                                self.dirty_names.insert(name.to_owned());
                            }
                        }
                    }
                    _ => self.dirty_all = true,
                }
            }
            if saw_event {
                self.refresh_after = Some(Instant::now() + Duration::from_millis(350));
            }
        }
        if self
            .refresh_after
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.refresh_after = None;
            self.nested.recount = self.gallery.as_ref().map(|g| g.folder().to_path_buf());
            let old_paths: Vec<_> = self
                .gallery
                .as_ref()
                .map(|g| g.photos().iter().map(|p| p.path.clone()).collect())
                .unwrap_or_default();
            let refreshed = self.gallery.as_mut().map(Gallery::refresh);
            if refreshed.as_ref().is_some_and(|r| matches!(r, Ok(true))) {
                self.remap_cached_images(&old_paths);
            }
            if self.dirty_all {
                self.generation += 1;
                self.thumbnail_tasks.set_generation(self.generation);
                self.preview_tasks.set_generation(self.generation);
                self.clear_cached_images();
            }
            match refreshed {
                Some(Ok(_)) => {
                    let dirty: Vec<_> = self
                        .gallery
                        .as_ref()
                        .unwrap()
                        .photos()
                        .iter()
                        .enumerate()
                        .filter(|(_, photo)| {
                            photo
                                .path
                                .file_name()
                                .is_some_and(|name| self.dirty_names.contains(name))
                        })
                        .map(|(index, _)| index)
                        .collect();
                    if !dirty.is_empty() {
                        self.generation += 1;
                        self.thumbnail_tasks.set_generation(self.generation);
                        self.preview_tasks.set_generation(self.generation);
                        self.pending_thumbnails.clear();
                        self.pending_previews.clear();
                        for index in dirty {
                            self.invalidate_image(index);
                        }
                    }
                }
                Some(Err(error)) => self.message = format!("Could not refresh gallery: {error}"),
                _ => {}
            }
            self.dirty_names.clear();
            self.dirty_all = false;
        }
    }
}
