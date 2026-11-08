//! GPU-only feedback for checked in-place transforms; pixels are saved by the worker.
use super::*;
use photo_core::actions::ActionId;

const DURATION: Duration = Duration::from_millis(230);

#[derive(Clone, Copy, Debug)]
enum Motion {
    Rotate(f32),
    MirrorX,
    MirrorY,
}
impl Motion {
    fn for_action(action: ActionId) -> Option<Self> {
        match action {
            ActionId::RotateLeft => Some(Self::Rotate(-std::f32::consts::FRAC_PI_2)),
            ActionId::RotateRight => Some(Self::Rotate(std::f32::consts::FRAC_PI_2)),
            ActionId::FlipHorizontal => Some(Self::MirrorX),
            ActionId::FlipVertical => Some(Self::MirrorY),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Forward,
    Rollback { started: Instant, from: f32 },
}

pub(super) struct Animation {
    operation: u64,
    collection: u64,
    path: PathBuf,
    texture: TextureHandle,
    motion: Motion,
    started: Instant,
    phase: Phase,
    saved: bool,
}

fn ease(progress: f32) -> f32 {
    let t = progress.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn progress(started: Instant, now: Instant) -> f32 {
    (now.saturating_duration_since(started).as_secs_f32() / DURATION.as_secs_f32()).clamp(0.0, 1.0)
}

fn strength(phase: Phase, started: Instant, now: Instant) -> f32 {
    match phase {
        Phase::Forward => ease(progress(started, now)),
        Phase::Rollback { started, from } => from * (1.0 - ease(progress(started, now))),
    }
}

fn finished(phase: Phase, started: Instant, now: Instant, saved_and_ready: bool) -> bool {
    match phase {
        Phase::Forward => progress(started, now) >= 1.0 && saved_and_ready,
        Phase::Rollback { started, .. } => progress(started, now) >= 1.0,
    }
}

fn scope_index(gallery: &Gallery, collection: u64, path: &Path) -> Option<usize> {
    if gallery.collection_id() != collection || gallery.view() != View::Photo {
        return None;
    }
    gallery
        .selected_index()
        .filter(|&index| gallery.photos()[index].path == path)
}

/// Quad positions preserve the center and fit every intermediate rotated pose.
/// Mirroring folds around the center without enlarging the collapsing axis.
fn corners(
    viewport: egui::Rect,
    natural: Vec2,
    motion: Motion,
    amount: f32,
    zoom: f32,
    offset: Vec2,
) -> [egui::Pos2; 4] {
    let (angle, axis) = match motion {
        Motion::Rotate(angle) => (angle * amount, Vec2::splat(1.0)),
        Motion::MirrorX => (0.0, Vec2::new((std::f32::consts::PI * amount).cos(), 1.0)),
        Motion::MirrorY => (0.0, Vec2::new(1.0, (std::f32::consts::PI * amount).cos())),
    };
    let (sin, cos) = angle.sin_cos();
    let bounds = Vec2::new(
        natural.x * cos.abs() + natural.y * sin.abs(),
        natural.x * sin.abs() + natural.y * cos.abs(),
    );
    let scale = (viewport.width() / bounds.x.max(1.0))
        .min(viewport.height() / bounds.y.max(1.0))
        .max(0.0)
        * zoom;
    let center = viewport.center() + offset;
    let half = natural * 0.5;
    [
        Vec2::new(-half.x, -half.y),
        Vec2::new(half.x, -half.y),
        Vec2::new(half.x, half.y),
        Vec2::new(-half.x, half.y),
    ]
    .map(|point| {
        let point = point * axis;
        center + Vec2::new(point.x * cos - point.y * sin, point.x * sin + point.y * cos) * scale
    })
}

impl Animation {
    pub(super) fn complete(&mut self, operation: u64, success: bool, now: Instant) {
        if self.operation != operation {
            return;
        }
        if success {
            self.saved = true;
        } else {
            let from = strength(self.phase, self.started, now);
            self.phase = Phase::Rollback { started: now, from };
        }
    }

    pub(super) fn texture(&self) -> TextureHandle {
        self.texture.clone()
    }

    pub(super) fn paint(&self, ui: &egui::Ui, rect: egui::Rect, zoom: f32, offset: Vec2) {
        let now = Instant::now();
        let positions = corners(
            rect,
            self.texture.size_vec2(),
            self.motion,
            strength(self.phase, self.started, now),
            zoom,
            offset,
        );
        let mut mesh = egui::Mesh::with_texture(self.texture.id());
        for (pos, uv) in positions.into_iter().zip([
            egui::pos2(0.0, 0.0),
            egui::pos2(1.0, 0.0),
            egui::pos2(1.0, 1.0),
            egui::pos2(0.0, 1.0),
        ]) {
            mesh.vertices.push(egui::epaint::Vertex {
                pos,
                uv,
                color: Color32::WHITE,
            });
        }
        mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
        ui.painter_at(rect).add(egui::Shape::mesh(mesh));
        let moving = match self.phase {
            Phase::Forward => progress(self.started, now) < 1.0,
            Phase::Rollback { started, .. } => progress(started, now) < 1.0,
        };
        if moving {
            ui.ctx().request_repaint();
        }
    }
}

impl Workshop {
    pub(super) fn start_transform_animation(
        &mut self,
        operation: u64,
        action: ActionId,
        paths: &[PathBuf],
        started: Instant,
    ) {
        let Some(motion) = Motion::for_action(action) else {
            return;
        };
        self.transform_animation = None;
        let Some(gallery) = self.gallery.as_ref().filter(|g| g.view() == View::Photo) else {
            return;
        };
        let Some(index) = gallery.selected_index() else {
            return;
        };
        let path = &gallery.photos()[index].path;
        if !paths.contains(path) {
            return;
        }
        let Some(texture) = self
            .previews
            .get(&index)
            .or_else(|| self.thumbnails.get(&index))
            .or_else(|| self.fallback_previews.get(&index))
            .cloned()
        else {
            return;
        };
        self.transform_animation = Some(Animation {
            operation,
            collection: gallery.collection_id(),
            path: path.clone(),
            texture,
            motion,
            started,
            phase: Phase::Forward,
            saved: false,
        });
        self.ctx.request_repaint();
    }

    pub(super) fn retire_transform_animation(&mut self) {
        let Some(animation) = &self.transform_animation else {
            return;
        };
        let index = self
            .gallery
            .as_ref()
            .and_then(|gallery| scope_index(gallery, animation.collection, &animation.path));
        let Some(index) = index else {
            self.transform_animation = None;
            return;
        };
        let ready = self
            .previews
            .get(&index)
            .is_some_and(|texture| texture.id() != animation.texture.id());
        if finished(
            animation.phase,
            animation.started,
            Instant::now(),
            animation.saved && ready,
        ) || (animation.saved && self.errors.contains_key(&index))
        {
            self.transform_animation = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_rotation_poses_fit_and_finish_with_swapped_aspect() {
        for viewport in [Vec2::new(180.0, 520.0), Vec2::new(900.0, 500.0)] {
            let rect = egui::Rect::from_min_size(egui::pos2(37.0, 91.0), viewport);
            for natural in [Vec2::new(1600.0, 900.0), Vec2::new(900.0, 1600.0)] {
                for angle in [-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2] {
                    for step in 0..=40 {
                        let points = corners(
                            rect,
                            natural,
                            Motion::Rotate(angle),
                            ease(step as f32 / 40.0),
                            1.0,
                            Vec2::ZERO,
                        );
                        assert!(points.iter().all(|p| rect.expand(0.01).contains(*p)));
                        let bounds = egui::Rect::from_points(&points);
                        assert!(bounds.center().distance(rect.center()) < 0.01);
                    }
                    let points =
                        corners(rect, natural, Motion::Rotate(angle), 1.0, 1.0, Vec2::ZERO);
                    let bounds = egui::Rect::from_points(&points);
                    assert!(
                        (bounds.width() / bounds.height() - natural.y / natural.x).abs() < 0.001
                    );
                }
            }
        }
    }

    #[test]
    fn mirroring_changes_only_the_requested_axis_and_keeps_center() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 500.0));
        let natural = Vec2::new(600.0, 300.0);
        for motion in [Motion::MirrorX, Motion::MirrorY] {
            let start = corners(rect, natural, motion, 0.0, 1.0, Vec2::ZERO);
            let middle = corners(rect, natural, motion, 0.5, 1.0, Vec2::ZERO);
            let end = corners(rect, natural, motion, 1.0, 1.0, Vec2::ZERO);
            for i in 0..4 {
                match motion {
                    Motion::MirrorX => {
                        assert!((end[i].x + start[i].x - 2.0 * rect.center().x).abs() < 0.01);
                        assert_eq!(end[i].y, start[i].y);
                        assert!((middle[i].x - rect.center().x).abs() < 0.01);
                    }
                    Motion::MirrorY => {
                        assert!((end[i].y + start[i].y - 2.0 * rect.center().y).abs() < 0.01);
                        assert_eq!(end[i].x, start[i].x);
                        assert!((middle[i].y - rect.center().y).abs() < 0.01);
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    #[test]
    fn early_save_finishes_motion_pending_save_holds_and_failure_rolls_back() {
        let start = Instant::now();
        let middle = start + DURATION / 2;
        assert!((strength(Phase::Forward, start, middle) - 0.5).abs() < 0.001);
        assert!(!finished(Phase::Forward, start, middle, true));
        assert!(!finished(
            Phase::Forward,
            start,
            start + DURATION * 3,
            false
        ));
        assert!(finished(Phase::Forward, start, start + DURATION, true));
        let rollback = Phase::Rollback {
            started: middle,
            from: 0.5,
        };
        assert!((strength(rollback, start, middle) - 0.5).abs() < 0.001);
        assert!(strength(rollback, start, middle + DURATION) <= f32::EPSILON);
        assert!(finished(rollback, start, middle + DURATION, false));
    }

    #[test]
    fn action_directions_match_saved_pixel_transforms() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 500.0));
        let natural = Vec2::new(600.0, 300.0);
        let right = corners(
            rect,
            natural,
            Motion::for_action(ActionId::RotateRight).unwrap(),
            1.0,
            1.0,
            Vec2::ZERO,
        );
        let left = corners(
            rect,
            natural,
            Motion::for_action(ActionId::RotateLeft).unwrap(),
            1.0,
            1.0,
            Vec2::ZERO,
        );
        assert!(right[0].x > rect.center().x && right[0].y < rect.center().y);
        assert!(left[0].x < rect.center().x && left[0].y > rect.center().y);
        assert!(matches!(
            Motion::for_action(ActionId::FlipHorizontal),
            Some(Motion::MirrorX)
        ));
        assert!(matches!(
            Motion::for_action(ActionId::FlipVertical),
            Some(Motion::MirrorY)
        ));
        assert!(Motion::for_action(ActionId::Duplicate).is_none());
    }

    #[test]
    fn navigation_grid_and_reopened_collection_retire_captured_photo_scope() {
        let folder = std::env::temp_dir().join(format!(
            "afio-animation-scope-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("a.png"), []).unwrap();
        std::fs::write(folder.join("b.png"), []).unwrap();
        let mut gallery = Gallery::open(&folder).unwrap();
        gallery.open_photo(0);
        let collection = gallery.collection_id();
        let path = gallery.photos()[0].path.clone();
        assert_eq!(scope_index(&gallery, collection, &path), Some(0));
        gallery.open_photo(1);
        assert!(scope_index(&gallery, collection, &path).is_none());
        gallery.back_to_grid();
        assert!(scope_index(&gallery, collection, &path).is_none());
        let mut reopened = Gallery::open(&folder).unwrap();
        reopened.open_photo(0);
        assert!(scope_index(&reopened, collection, &path).is_none());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
