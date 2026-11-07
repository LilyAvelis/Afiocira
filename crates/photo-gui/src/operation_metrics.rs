//! Monotonic job timings. Preview readiness is CPU-side texture availability,
//! not a claim about physical monitor presentation time.
use super::*;

pub(super) struct VisualProbe {
    id: u64,
    submitted: Instant,
    path: PathBuf,
}
impl Workshop {
    pub(super) fn render_snapshot(&self) -> photo_core::api::RenderState {
        let Some(gallery) = &self.gallery else {
            return Default::default();
        };
        let index = if gallery.view() == View::Photo {
            gallery.selected_index().unwrap_or(gallery.grid_anchor())
        } else {
            gallery.grid_anchor()
        };
        photo_core::api::RenderState {
            preview_ready: self.previews.contains_key(&index),
            thumbnail_ready: self.thumbnails.contains_key(&index),
            next_preview_ready: self.previews.contains_key(&(index + 1)),
            cached_previews: self.previews.len(),
            preview_textures: self
                .previews
                .iter()
                .filter_map(|(i, t)| {
                    Some((
                        gallery.photos().get(*i)?.path.clone(),
                        match t.id() {
                            egui::TextureId::Managed(n) | egui::TextureId::User(n) => n,
                        },
                    ))
                })
                .collect(),
            thumbnail_textures: self
                .thumbnails
                .iter()
                .filter_map(|(i, t)| {
                    Some((
                        gallery.photos().get(*i)?.path.clone(),
                        match t.id() {
                            egui::TextureId::Managed(n) | egui::TextureId::User(n) => n,
                        },
                    ))
                })
                .collect(),
        }
    }
    pub(super) fn begin_visual_probe(&mut self, id: u64, submitted: Instant) {
        self.visual_probe = self
            .gallery
            .as_ref()
            .filter(|g| g.view() == View::Photo)
            .and_then(Gallery::selected_photo)
            .filter(|p| !p.is_video())
            .map(|p| VisualProbe {
                id,
                submitted,
                path: p.path.clone(),
            });
    }
    pub(super) fn poll_visual_probe(&mut self) {
        let Some(probe) = &self.visual_probe else {
            return;
        };
        let current = self
            .gallery
            .as_ref()
            .filter(|g| g.view() == View::Photo)
            .and_then(Gallery::selected_photo);
        if current.is_none_or(|p| p.path != probe.path)
            || self.operation.as_ref().is_none_or(|op| op.id != probe.id)
        {
            self.visual_probe = None;
            return;
        }
        if self.render_snapshot().preview_ready {
            if let Some(timing) = self.operation.as_mut().and_then(|op| op.timing.as_mut()) {
                timing.preview_ready_ms = Some(probe.submitted.elapsed().as_secs_f64() * 1000.0);
            }
            self.visual_probe = None;
        }
    }
}
