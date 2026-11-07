//! Self-contained visual check; renders fixture bytes, saves its own frame and exits.
#[path = "../src/search_icons.rs"]
mod search_icons;
use eframe::egui::{self, Color32};
use egui_tty::{Terminal, TerminalStyle, Tty, TtyStream};
use std::sync::{Arc, mpsc};
struct NullTty;
impl Tty for NullTty {
    fn write(&self, _: &[u8]) -> egui_tty::Result<()> {
        Ok(())
    }
    fn resize(&self, _: u16, _: u16) -> egui_tty::Result<()> {
        Ok(())
    }
}
struct Probe {
    old: Terminal,
    new: Terminal,
    frames: usize,
    writers: Vec<mpsc::Sender<Vec<u8>>>,
}
const FIXTURE: &str = "\x1b[?25l\x1b[48;2;23;20;18m\x1b[2J\x1b[H\x1b[1;38;2;199;106;79m\u{e0b6}\x1b[38;2;23;20;18;48;2;199;106;79m Лилит Панорама (1).jpg   \x1b[38;2;199;106;79;48;2;23;20;18m\u{e0b4}\x1b[0m";
fn terminal() -> (Terminal, mpsc::Sender<Vec<u8>>) {
    let (sender, output) = mpsc::channel();
    // Powerline caps use the same foreground as the middle cells' background.
    sender.send(FIXTURE.as_bytes().to_vec()).unwrap();
    let terminal = Terminal::new(TtyStream {
        output,
        tty: Arc::new(NullTty),
    })
    .unwrap();
    (terminal, sender)
}
impl eframe::App for Probe {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        for writer in &self.writers {
            let _ = writer.send(FIXTURE.as_bytes().to_vec());
        }
        let screenshot = ctx.input(|i| {
            i.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(image) = screenshot {
            let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            image::save_buffer(
                std::env::var("AFIO_RENDER_PROBE").unwrap(),
                &bytes,
                image.size[0] as u32,
                image.size[1] as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.heading("Yazi: identical terminal bytes");
        ui.label("Before: simulated bold changes ink colors");
        let style = TerminalStyle {
            font: egui::FontId::new(18.0, egui::FontFamily::Name("probe".into())),
            ..TerminalStyle::default()
        };
        ui.allocate_ui(egui::vec2(690.0, 75.0), |ui| {
            self.old.ui(ui, &style);
        });
        ui.label("After: exact foreground and background colors");
        ui.allocate_ui(egui::vec2(690.0, 75.0), |ui| {
            self.new.ui(
                ui,
                &TerminalStyle {
                    brighten_bold: false,
                    ..style
                },
            );
        });
        ui.separator();
        ui.heading("Everything: vector icons");
        for (index, name) in [
            "Папка",
            "фото.png",
            "видео.mp4",
            "заметка.txt",
            "архив.zip",
            "неизвестный.xyz",
        ]
        .iter()
        .enumerate()
        {
            let item = photo_core::api::SearchResult {
                path: std::path::PathBuf::from(name),
                is_folder: index == 0,
            };
            search_icons::result_row(ui, &item, &format!("C:/Visual fixture/{name}"), index == 1);
        }
        self.frames += 1;
        if self.frames == 12 {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
    }
}
fn main() -> eframe::Result {
    eframe::run_native(
        "Afio rendering fixture",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([750.0, 720.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            let mut fonts = egui::FontDefinitions::default();
            let bytes =
                std::fs::read("C:/Windows/Fonts/JetBrainsMonoNerdFontMono-Regular.ttf").unwrap();
            fonts
                .font_data
                .insert("probe".into(), Arc::new(egui::FontData::from_owned(bytes)));
            fonts
                .families
                .insert(egui::FontFamily::Name("probe".into()), vec!["probe".into()]);
            cc.egui_ctx.set_fonts(fonts);
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            cc.egui_ctx.style_mut_of(egui::Theme::Dark, |s| {
                s.visuals.panel_fill = Color32::from_rgb(54, 54, 58)
            });
            let (old, old_writer) = terminal();
            let (new, new_writer) = terminal();
            Ok(Box::new(Probe {
                old,
                new,
                frames: 0,
                writers: vec![old_writer, new_writer],
            }))
        }),
    )
}
