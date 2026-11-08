use crate::ui_foundation::tokens;
use eframe::egui::{self, Color32};

pub(super) fn install(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(39, 39, 42);
    visuals.window_fill = Color32::from_rgb(54, 54, 58);
    visuals.extreme_bg_color = Color32::from_rgb(32, 32, 35);
    visuals.faint_bg_color = Color32::from_rgb(49, 49, 53);
    visuals.override_text_color = Some(Color32::from_rgb(235, 235, 238));
    visuals.selection.bg_fill = Color32::from_rgb(91, 91, 99);
    visuals.selection.stroke.color = Color32::WHITE;
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = egui::CornerRadius::same(tokens::CONTROL_RADIUS);
        widget.bg_stroke = egui::Stroke::NONE;
    }
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(61, 61, 66);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(79, 79, 85);
    visuals.widgets.active.bg_fill = Color32::from_rgb(95, 95, 103);
    visuals.window_corner_radius = egui::CornerRadius::same(tokens::WINDOW_RADIUS);
    style.visuals = visuals;
    style.spacing.interact_size.y = tokens::CONTROL_HEIGHT;
    style.spacing.item_spacing = egui::vec2(tokens::SM, tokens::SM);
    style.spacing.button_padding = egui::vec2(tokens::SM, tokens::SM);
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::proportional(tokens::BODY),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(tokens::BODY),
    );
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_style_of(egui::Theme::Dark, style);
}

pub(super) fn keycap(ui: &mut egui::Ui, text: &str) {
    let mut keys: Vec<_> = text.split_whitespace().collect();
    if ui.layout().main_dir() == egui::Direction::RightToLeft {
        keys.reverse();
    }
    let spacing = ui.spacing().item_spacing.x;
    ui.spacing_mut().item_spacing.x = 3.0;
    for key in keys {
        egui::Frame::new()
            .fill(Color32::from_white_alpha(12))
            .stroke(egui::Stroke::new(1.0, Color32::from_white_alpha(55)))
            .corner_radius(tokens::XS as u8)
            .inner_margin(egui::Margin::symmetric(tokens::XS as i8, 1))
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new(key)
                        .size(tokens::CAPTION)
                        .color(Color32::from_gray(220)),
                );
            });
    }
    ui.spacing_mut().item_spacing.x = spacing;
}

pub(super) fn actions_button(ui: &mut egui::Ui, open: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(140.0, tokens::CONTROL_HEIGHT),
        egui::Sense::click(),
    );
    ui.painter().rect_filled(
        rect,
        9.0,
        Color32::from_gray(if open || response.hovered() { 100 } else { 78 }),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(11.0, 3.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.label(egui::RichText::new("Actions").strong().size(14.0));
            keycap(ui, "Ctrl K");
        },
    );
    response.on_hover_text("Действия · Ctrl+K")
}
