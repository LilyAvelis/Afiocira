use super::*;

impl Workshop {
    pub(super) fn clear_cached_images(&mut self) {
        self.thumbnails.clear();
        self.thumbnail_order.clear();
        self.pending_thumbnails.clear();
        self.previews.clear();
        self.preview_order.clear();
        self.pending_previews.clear();
        self.errors.clear();
    }

    pub(super) fn remap_cached_images(&mut self, old_paths: &[PathBuf]) {
        let Some(gallery) = &self.gallery else { return };
        let new_indices: HashMap<_, _> = gallery
            .photos()
            .iter()
            .enumerate()
            .map(|(index, photo)| (&photo.path, index))
            .collect();
        let remap: Vec<_> = old_paths
            .iter()
            .map(|path| new_indices.get(path).copied())
            .collect();
        self.generation += 1;
        self.thumbnail_tasks.set_generation(self.generation);
        self.preview_tasks.set_generation(self.generation);
        self.pending_thumbnails.clear();
        self.pending_previews.clear();
        self.thumbnails = std::mem::take(&mut self.thumbnails)
            .into_iter()
            .filter_map(|(index, image)| Some((remap.get(index).copied().flatten()?, image)))
            .collect();
        self.previews = std::mem::take(&mut self.previews)
            .into_iter()
            .filter_map(|(index, image)| Some((remap.get(index).copied().flatten()?, image)))
            .collect();
        self.errors = std::mem::take(&mut self.errors)
            .into_iter()
            .filter_map(|(index, error)| Some((remap.get(index).copied().flatten()?, error)))
            .collect();
        self.thumbnail_order = std::mem::take(&mut self.thumbnail_order)
            .into_iter()
            .filter_map(|index| remap.get(index).copied().flatten())
            .collect();
        self.preview_order = std::mem::take(&mut self.preview_order)
            .into_iter()
            .filter_map(|index| remap.get(index).copied().flatten())
            .collect();
        self.scroll_to_grid_anchor = true;
    }

    pub(super) fn poll_results(&mut self, ctx: &egui::Context) {
        while let Ok(result) = self.results.try_recv() {
            if result.generation != self.generation {
                continue;
            }
            match result.image {
                Ok(image) => {
                    let texture = ctx.load_texture(
                        format!(
                            "photo-{}-{}-{}",
                            result.generation, result.index, result.preview
                        ),
                        image,
                        TextureOptions::LINEAR,
                    );
                    if result.preview {
                        self.previews.insert(result.index, texture);
                        self.preview_order.push_back(result.index);
                        while self.preview_order.len() > PREVIEW_CACHE_LIMIT
                            || (self.previews.len() > 1
                                && self
                                    .previews
                                    .values()
                                    .map(|texture| {
                                        let [width, height] = texture.size();
                                        width.saturating_mul(height).saturating_mul(4)
                                    })
                                    .sum::<usize>()
                                    > PREVIEW_CACHE_BYTES)
                        {
                            if let Some(oldest) = self.preview_order.pop_front() {
                                self.previews.remove(&oldest);
                            }
                        }
                        self.pending_previews.remove(&result.index);
                    } else {
                        self.thumbnails.insert(result.index, texture);
                        self.thumbnail_order.push_back(result.index);
                        while self.thumbnail_order.len() > 160 {
                            if let Some(oldest) = self.thumbnail_order.pop_front() {
                                self.thumbnails.remove(&oldest);
                            }
                        }
                        self.pending_thumbnails.remove(&result.index);
                    }
                }
                Err(error) => {
                    self.errors.insert(result.index, error);
                    if result.preview {
                        self.pending_previews.remove(&result.index);
                    } else {
                        self.pending_thumbnails.remove(&result.index);
                    }
                }
            }
        }
    }

    pub(super) fn request_thumbnail(&mut self, index: usize) {
        if self.thumbnails.contains_key(&index)
            || self.pending_thumbnails.contains(&index)
            || self.errors.contains_key(&index)
        {
            return;
        }
        if let Some(photo) = self.gallery.as_ref().and_then(|g| g.photos().get(index)) {
            let task = DecodeTask {
                generation: self.generation,
                index,
                path: photo.path.clone(),
                preview: false,
            };
            if let Ok(evicted) = self.thumbnail_tasks.submit(task, false) {
                self.pending_thumbnails.insert(index);
                if let Some(evicted) = evicted {
                    self.pending_thumbnails.remove(&evicted);
                }
            } else {
                self.errors.insert(index, "Thumbnail worker stopped".into());
            }
        }
    }

    pub(super) fn request_preview(&mut self, index: usize) {
        if self.previews.contains_key(&index) || self.errors.contains_key(&index) {
            return;
        }
        if self.pending_previews.contains(&index) {
            self.preview_tasks.promote(index);
            return;
        }
        if let Some(photo) = self.gallery.as_ref().and_then(|g| g.photos().get(index)) {
            if photo.is_video() {
                return;
            }
            let task = DecodeTask {
                generation: self.generation,
                index,
                path: photo.path.clone(),
                preview: true,
            };
            let urgent = self.gallery.as_ref().and_then(Gallery::selected_index) == Some(index);
            if let Ok(evicted) = self.preview_tasks.submit(task, urgent) {
                self.pending_previews.insert(index);
                if let Some(evicted) = evicted {
                    self.pending_previews.remove(&evicted);
                }
            } else {
                self.errors.insert(index, "Preview worker stopped".into());
            }
        }
    }

    pub(super) fn prefetch_previews(&mut self, index: usize) {
        let count = self.gallery.as_ref().map_or(0, |g| g.photos().len());
        self.request_preview(index);
        for distance in 1..=2 {
            if index + distance < count {
                self.request_preview(index + distance);
            }
            if let Some(previous) = index.checked_sub(distance) {
                self.request_preview(previous);
            }
        }
    }
}
