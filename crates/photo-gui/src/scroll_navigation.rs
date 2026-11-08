//! Hold the middle button and move vertically to autoscroll a Settings pane.
use eframe::egui::{self, PointerButton, Pos2, Vec2};

#[derive(Clone, Default)]
struct State {
    anchor: Option<Pos2>,
    frame: u64,
}

fn velocity(distance: f32) -> f32 {
    let magnitude = (distance.abs() - 12.0).max(0.0);
    (magnitude * 9.0).min(2400.0) * distance.signum()
}

pub(super) fn apply(ui: &mut egui::Ui) {
    let id = ui.id().with("middle-scroll");
    let mut state = ui
        .ctx()
        .data_mut(|d| d.get_temp::<State>(id).unwrap_or_default());
    let frame = ui.ctx().cumulative_frame_nr();
    if state.frame.saturating_add(1) < frame {
        state.anchor = None;
    }
    state.frame = frame;
    let rect = ui.clip_rect();
    let (position, pressed, held, focused, escape, dt) = ui.input(|i| {
        (
            i.pointer.hover_pos(),
            i.pointer.button_pressed(PointerButton::Middle),
            i.pointer.button_down(PointerButton::Middle),
            i.focused,
            i.key_pressed(egui::Key::Escape),
            i.stable_dt.min(0.05),
        )
    });
    if pressed && position.is_some_and(|p| rect.contains(p)) {
        state.anchor = position;
    }
    if !held || !focused || escape {
        state.anchor = None;
    }
    if let (Some(anchor), Some(position)) = (state.anchor, position) {
        ui.scroll_with_delta_animation(
            Vec2::new(0.0, -velocity(position.y - anchor.y) * dt),
            egui::style::ScrollAnimation::none(),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
        let painter = ui.painter_at(rect);
        painter.circle_stroke(
            anchor,
            9.0,
            egui::Stroke::new(1.0, ui.visuals().text_color()),
        );
        painter.line_segment(
            [anchor - egui::vec2(0.0, 5.0), anchor + egui::vec2(0.0, 5.0)],
            egui::Stroke::new(1.0, ui.visuals().text_color()),
        );
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dead_zone_direction_and_speed_limit() {
        assert_eq!(velocity(0.0), 0.0);
        assert_eq!(velocity(12.0), 0.0);
        assert_eq!(velocity(-11.0), 0.0);
        assert_eq!(velocity(100.0), -velocity(-100.0));
        assert!(velocity(100.0) > velocity(40.0));
        assert_eq!(velocity(10000.0), 2400.0);
    }

    #[test]
    fn held_middle_scrolls_and_release_stops_without_swallowing_primary_input() {
        let ctx = egui::Context::default();
        let mut offset = 0.0;
        let mut frame = |events, time| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        Pos2::ZERO,
                        egui::vec2(400.0, 300.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let area = egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            apply(ui);
                            ui.allocate_space(egui::vec2(200.0, 2000.0));
                        });
                    offset = area.state.offset.y;
                },
            );
            output.textures_delta.clear();
            offset
        };
        frame(vec![], 0.0);
        frame(
            vec![
                egui::Event::PointerMoved(egui::pos2(100.0, 80.0)),
                egui::Event::PointerButton {
                    pos: egui::pos2(100.0, 80.0),
                    button: PointerButton::Middle,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            0.02,
        );
        frame(
            vec![egui::Event::PointerMoved(egui::pos2(100.0, 220.0))],
            0.05,
        );
        // egui applies programmatic scroll targets at the next frame boundary.
        let moved = frame(
            vec![egui::Event::PointerMoved(egui::pos2(100.0, 220.0))],
            0.08,
        );
        assert!(moved > 0.0);
        let stopped = frame(
            vec![egui::Event::PointerButton {
                pos: egui::pos2(100.0, 220.0),
                button: PointerButton::Middle,
                pressed: false,
                modifiers: Default::default(),
            }],
            0.11,
        );
        let later = frame(
            vec![egui::Event::PointerMoved(egui::pos2(100.0, 270.0))],
            0.14,
        );
        assert_eq!(later, stopped);
    }
}
