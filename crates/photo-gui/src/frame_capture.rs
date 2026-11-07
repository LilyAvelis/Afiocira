//! Explicit native API frame capture for repeatable visual checks; no desktop input.
use eframe::egui;
use std::path::PathBuf;
pub(super) fn poll(ctx: &egui::Context) {
    let captures = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| {
                if let egui::Event::Screenshot {
                    image, user_data, ..
                } = e
                {
                    user_data
                        .data
                        .as_ref()?
                        .downcast_ref::<PathBuf>()
                        .map(|p| (p.clone(), image.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    });
    for (path, image) in captures {
        let bytes: Vec<_> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
        if let Err(e) = image::save_buffer(
            path,
            &bytes,
            image.size[0] as u32,
            image.size[1] as u32,
            image::ColorType::Rgba8,
        ) {
            eprintln!("Frame capture: {e}");
        }
    }
}
