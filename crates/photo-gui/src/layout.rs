//! Layout math shared by thumbnails and visual regression probes.
use eframe::egui::{Rect, Vec2};
pub(super) fn visible_canvas(logical: Rect, clip: Rect) -> Rect {
    logical.intersect(clip)
}
pub(super) fn fit_rect(cell: Rect, natural: Vec2) -> Rect {
    let scale = (cell.width() / natural.x.max(1.0))
        .min(cell.height() / natural.y.max(1.0))
        .max(0.0);
    Rect::from_center_size(cell.center(), natural * scale)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_toolbar_cannot_push_portrait_outside_the_visible_panel() {
        let logical =
            Rect::from_min_size(eframe::egui::pos2(600.0, 200.0), Vec2::new(1000.0, 500.0));
        let clip = Rect::from_min_max(eframe::egui::Pos2::ZERO, eframe::egui::pos2(900.0, 700.0));
        let canvas = visible_canvas(logical, clip);
        let image = fit_rect(canvas, Vec2::new(300.0, 900.0));
        assert_eq!(canvas.width(), 300.0);
        assert!(canvas.expand(0.001).contains_rect(image));
        assert_eq!(image.center().x, 750.0);
    }
    #[test]
    fn portrait_square_and_wide_stay_centered_when_narrowed() {
        for width in [90.0, 220.0, 500.0] {
            let cell =
                Rect::from_min_size(eframe::egui::pos2(600.0, 200.0), Vec2::new(width, 480.0));
            for natural in [
                Vec2::new(100.0, 200.0),
                Vec2::splat(200.0),
                Vec2::new(1600.0, 900.0),
            ] {
                let image = fit_rect(cell, natural);
                assert_eq!(image.center(), cell.center());
                assert!(cell.expand(0.001).contains_rect(image));
            }
        }
    }
}
