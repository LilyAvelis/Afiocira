//! Vector result icons: independent of terminal fonts and Nerd Font coverage.
use eframe::egui::{self, Color32};
use egui_lucide::Lucide;
use photo_core::api::SearchResult;

fn icon(item: &SearchResult) -> Lucide {
    if item.is_folder {
        return Lucide::Folder;
    }
    if photo_core::is_video_path(&item.path) {
        return Lucide::Video;
    }
    if photo_core::is_media_path(&item.path) {
        return Lucide::Image;
    }
    match item
        .path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" | "wav" | "flac" | "ogg" | "aac" | "m4a" => Lucide::Music,
        "zip" | "7z" | "rar" | "tar" | "gz" => Lucide::Archive,
        "txt" | "md" | "pdf" | "doc" | "docx" | "rtf" => Lucide::FileText,
        "rs" | "py" | "js" | "ts" | "json" | "toml" | "lua" | "html" | "css" => Lucide::FileCode,
        _ => Lucide::File,
    }
}

pub(super) fn result_row(
    ui: &mut egui::Ui,
    item: &SearchResult,
    path: &str,
    selected: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 58.0), egui::Sense::click());
    if selected || response.hovered() {
        ui.painter().rect_filled(
            rect,
            7.0,
            if selected {
                Color32::from_rgb(91, 91, 99)
            } else {
                Color32::from_rgb(77, 77, 83)
            },
        );
    }
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(egui::vec2(10.0, 6.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.add(icon(item).size(22.0).image().tint(if item.is_folder {
                Color32::from_rgb(224, 188, 127)
            } else {
                Color32::from_rgb(199, 205, 219)
            }));
            ui.add_space(4.0);
            ui.vertical(|ui| {
                let name = item
                    .path
                    .file_name()
                    .unwrap_or(item.path.as_os_str())
                    .to_string_lossy();
                ui.add(egui::Label::new(egui::RichText::new(name).size(15.0)).truncate());
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(path)
                            .size(12.0)
                            .color(Color32::from_gray(183)),
                    )
                    .truncate(),
                );
            });
        },
    );
    response.on_hover_text(path)
}
