#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod actions;
mod albums;
mod clipboard;
mod clipboard_input;
mod convert_image;
mod decode;
mod delete_action;
mod dispatch;
mod drop_input;
mod everything;
mod file_state;
mod frame_capture;
mod gallery_scale;
mod gallery_view;
mod image_actions;
mod image_cache;
mod input;
mod ipc;
mod layout;
mod media_view;
mod native_shortcuts;
mod navigator;
mod nested_view;
mod operation_metrics;
mod operations;
mod palette;
mod rename_action;
mod resize_image;
mod scroll_navigation;
mod search;
mod search_icons;
mod settings;
mod settings_yazi;
mod shortcuts;
mod transform_animation;
mod ui_foundation;
mod ui_style;
mod video;
mod yazi;
mod yazi_flavors;
mod yazi_key_capture;
mod yazi_profile;
mod yazi_settings;
mod yazi_transport;
use decode::{DecodeQueue, DecodeResult, DecodeTask, spawn_worker};
use eframe::egui::{self, Color32, Key, TextureHandle, TextureOptions, Vec2};
use egui_lucide::Lucide;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use photo_core::api::{Command, ErrorCode, NavigatorState, Pane, Response};
use photo_core::{Gallery, Point, Size, View};
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use yazi::YaziPane;
const TARGET_TILE_WIDTH: f32 = 240.0;
const PREVIEW_CACHE_LIMIT: usize = 12;
const PREVIEW_CACHE_BYTES: usize = 128 * 1024 * 1024;
#[derive(Clone, Copy)]
struct GridLayout {
    columns: usize,
    tile_width: f32,
    image_height: f32,
    row_height: f32,
}
fn grid_layout(available_width: f32, spacing: Vec2, scale: Option<f32>) -> GridLayout {
    // Leave room for the vertical scrollbar so the final column stays visible.
    let width = (available_width - 16.0).max(1.0);
    let columns = ((width + spacing.x) / (TARGET_TILE_WIDTH * scale.unwrap_or(1.0) + spacing.x))
        .floor()
        .max(1.0) as usize;
    let tile_width = scale
        .map(|factor| (TARGET_TILE_WIDTH * factor).min(width))
        .unwrap_or((width - spacing.x * (columns - 1) as f32) / columns as f32)
        .max(1.0);
    let image_height = (tile_width * 0.8).clamp(64.0, 720.0);
    GridLayout {
        columns,
        tile_width,
        image_height,
        row_height: image_height + 24.0 + spacing.y * 2.0,
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum ActivePane {
    Yazi,
    Gallery,
}
struct Workshop {
    transform_animation: Option<transform_animation::Animation>,
    navigation: navigator::State,
    yazi_settings: yazi_settings::State,
    settings: settings::State,
    image_dialog: Option<image_actions::Dialog>,
    nested: nested_view::State,
    search: search::State,
    visual_probe: Option<operation_metrics::VisualProbe>,
    native_keys: Option<native_shortcuts::Hook>,
    rename_dialog: Option<rename_action::Dialog>,
    delete_dialog: Option<delete_action::Dialog>,
    interaction_revision: u64,
    palette: palette::State,
    worker: operations::Worker,
    operation: Option<photo_core::api::OperationState>,
    operation_origin: Option<operations::Origin>,
    next_operation: u64,
    video: Option<Arc<std::sync::Mutex<video::Video>>>,
    video_error: Option<String>,
    video_restore: Option<(PathBuf, f64, bool, Instant)>,
    ctx: egui::Context,
    session_id: String,
    yazi: Option<YaziPane>,
    yazi_error: Option<String>,
    show_yazi: bool,
    nerd_font_loaded: bool,
    active_pane: ActivePane,
    scroll_to_grid_anchor: bool,
    selection_dirty: bool,
    mark_sync_after: Option<Instant>,
    mark_sync_attempts: u8,
    gallery: Option<Gallery>,
    folder_input: String,
    message: String,
    generation: u64,
    thumbnail_tasks: Arc<DecodeQueue>,
    preview_tasks: Arc<DecodeQueue>,
    results: Receiver<DecodeResult>,
    thumbnails: HashMap<usize, TextureHandle>,
    thumbnail_order: VecDeque<usize>,
    pending_thumbnails: HashSet<usize>,
    previews: HashMap<usize, TextureHandle>,
    fallback_previews: HashMap<usize, TextureHandle>,
    preview_order: VecDeque<usize>,
    pending_previews: HashSet<usize>,
    errors: HashMap<usize, String>,
    api_requests: Option<Receiver<ipc::Request>>,
    api_error: Option<String>,
    watcher: Option<RecommendedWatcher>,
    file_events: Option<Receiver<notify::Result<notify::Event>>>,
    refresh_after: Option<Instant>,
    dirty_names: HashSet<OsString>,
    dirty_all: bool,
}
impl Workshop {
    fn new(cc: &eframe::CreationContext<'_>, initial: &Path) -> Self {
        let ctx = &cc.egui_ctx;
        ui_style::install(ctx);
        let (video, video_error) = match video::Video::new(cc) {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };
        let nerd_font_loaded = yazi::install_terminal_font(ctx);
        egui_extras::install_image_loaders(ctx);
        let thumbnail_tasks = DecodeQueue::new();
        let preview_tasks = DecodeQueue::new();
        let (results_tx, results_rx) = mpsc::sync_channel(4);
        spawn_worker(thumbnail_tasks.clone(), results_tx.clone(), ctx.clone());
        spawn_worker(preview_tasks.clone(), results_tx, ctx.clone());
        let yazi_folder = if initial.is_file() {
            initial.parent().unwrap_or(initial)
        } else {
            initial
        };
        let (mut yazi, yazi_error) = match YaziPane::start(yazi_folder) {
            Ok(pane) => (Some(pane), None),
            Err(error) => (None, Some(error)),
        };
        if let Some(pane) = &mut yazi {
            pane.request_focus();
        }
        let active_pane = if yazi.is_some() {
            ActivePane::Yazi
        } else {
            ActivePane::Gallery
        };
        let (api_requests, api_error) = match ipc::start() {
            Ok(requests) => (Some(requests), None),
            Err(error) => (None, Some(format!("Live API unavailable: {error}"))),
        };
        let mut app = Self {
            transform_animation: None,
            navigation: navigator::State::default(),
            yazi_settings: yazi_settings::State::default(),
            settings: settings::State::load(),
            image_dialog: None,
            nested: nested_view::State::new(ctx.clone()),
            search: search::State::default(),
            visual_probe: None,
            native_keys: native_shortcuts::Hook::install().ok(),
            rename_dialog: None,
            delete_dialog: None,
            interaction_revision: 0,
            palette: palette::State::default(),
            worker: operations::Worker::start(ctx.clone()),
            operation: None,
            operation_origin: None,
            next_operation: 0,
            video,
            video_error,
            video_restore: None,
            ctx: ctx.clone(),
            session_id: format!(
                "{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ),
            yazi,
            yazi_error,
            show_yazi: true,
            nerd_font_loaded,
            active_pane,
            scroll_to_grid_anchor: true,
            selection_dirty: false,
            mark_sync_after: None,
            mark_sync_attempts: 0,
            gallery: None,
            folder_input: initial.display().to_string(),
            message: String::new(),
            generation: 0,
            thumbnail_tasks,
            preview_tasks,
            results: results_rx,
            thumbnails: HashMap::new(),
            thumbnail_order: VecDeque::new(),
            pending_thumbnails: HashSet::new(),
            previews: HashMap::new(),
            fallback_previews: HashMap::new(),
            preview_order: VecDeque::new(),
            pending_previews: HashSet::new(),
            errors: HashMap::new(),
            api_requests,
            api_error,
            watcher: None,
            file_events: None,
            refresh_after: None,
            dirty_names: HashSet::new(),
            dirty_all: false,
        };
        app.open_path(initial);
        app
    }
}
impl Drop for Workshop {
    fn drop(&mut self) {
        self.thumbnail_tasks.close();
        self.preview_tasks.close();
    }
}
impl eframe::App for Workshop {
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let (Some(video), Some(gl)) = (&self.video, gl) {
            video.lock().unwrap().shutdown(gl);
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        frame_capture::poll(&ctx);
        self.poll_operations(&ctx);
        self.sync_video();
        self.poll_api();
        self.poll_navigator();
        self.poll_yazi_settings();
        self.poll_search();
        self.poll_nested();
        if let Some(error) = self.yazi.as_mut().and_then(YaziPane::take_navigation_error) {
            self.message = error;
        }
        if !self.palette_shortcut_input(&ctx) {
            self.global_action_keys(&ctx);
        }
        self.handle_dropped_files(&ctx);
        self.actions_footer(ui);
        self.poll_file_changes();
        self.poll_results(&ctx);
        self.retire_transform_animation();
        if let Some(folder) = self.yazi.as_mut().and_then(YaziPane::changed_folder) {
            self.follow_navigator_folder(folder);
            if let Some(yazi) = &self.yazi {
                let _ = yazi.request_order();
            }
        }
        if let Some(order) = self.yazi.as_mut().and_then(YaziPane::poll_order)
            && self
                .gallery
                .as_ref()
                .is_some_and(|gallery| same_folder(gallery.folder(), &order.folder))
        {
            let folder = self.gallery.as_ref().unwrap().folder();
            let paths = navigator_paths_in_gallery(folder, order.paths);
            self.execute(Command::SetOrder { paths });
        }
        ctx.request_repaint_after(Duration::from_millis(100));
        ui.add_space(8.0);
        let mut path_input_focused = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("Afio").strong().size(19.0));
            ui.separator();
            if ui
                .add(egui::Button::new("Yazi").selected(self.show_yazi))
                .clicked()
            {
                self.execute(Command::SetNavigatorVisible {
                    visible: !self.show_yazi,
                });
            }
            let input = ui.add(
                egui::TextEdit::singleline(&mut self.folder_input)
                    .id(egui::Id::new("folder-path"))
                    .desired_width((ui.available_width() - 220.0).clamp(100.0, 430.0)),
            );
            path_input_focused = input.has_focus();
            let submit = ui
                .add(egui::Button::image_and_text(
                    Lucide::FolderOpen.size(16.0).image(),
                    "Open",
                ))
                .clicked()
                || (input.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
            if submit
                && self
                    .execute(Command::Open {
                        path: pasted_path(&self.folder_input),
                    })
                    .error
                    .is_none()
            {
                release_keyboard_focus(ui);
            }
            if ui.button("Обновить · F5").clicked() {
                self.execute(Command::Refresh);
                release_keyboard_focus(ui);
            }
            if ui.button("Поиск · Ctrl Shift K").clicked() {
                self.execute(Command::ShowSearch);
            }
            ui.label(&self.message);
            if let Some(error) = &self.api_error {
                ui.colored_label(Color32::YELLOW, error);
            }
        });
        ui.add_space(8.0);
        ui.separator();
        let yazi_running = self.yazi.as_mut().is_some_and(YaziPane::is_running);
        if yazi_running
            && !path_input_focused
            && !self.search.snapshot.open
            && !self.palette.open
            && self.image_dialog.is_none()
            && self.rename_dialog.is_none()
            && self.delete_dialog.is_none()
            && consume_plain_tab(&ctx)
        {
            match self.active_pane {
                ActivePane::Yazi => {
                    self.execute(Command::FocusPane {
                        pane: Pane::Gallery,
                    });
                }
                ActivePane::Gallery => {
                    self.execute(Command::FocusPane {
                        pane: Pane::Navigator,
                    });
                }
            }
        }
        let mut restart_yazi = false;
        let mut terminal_has_focus = false;
        if self.show_yazi
            && let Some(yazi) = &mut self.yazi
        {
            egui::Panel::left("yazi-navigator")
                .resizable(true)
                .default_size(340.0)
                .size_range(240.0..=560.0)
                .show(ui, |ui| {
                    let heading = if self.active_pane == ActivePane::Yazi {
                        "YAZI · активная панель"
                    } else {
                        "Yazi · Tab"
                    };
                    ui.label(egui::RichText::new(heading).strong().color(
                        if self.active_pane == ActivePane::Yazi {
                            Color32::from_rgb(105, 189, 247)
                        } else {
                            ui.visuals().weak_text_color()
                        },
                    ));
                    ui.separator();
                    if yazi.is_running() {
                        let events = if self.yazi_settings.busy()
                            || self.search.snapshot.open
                            || self.palette.open
                            || self.image_dialog.is_some()
                            || self.rename_dialog.is_some()
                            || self.delete_dialog.is_some()
                        {
                            Some(ui.input_mut(|i| std::mem::take(&mut i.events)))
                        } else {
                            None
                        };
                        terminal_has_focus = ui
                            .add_enabled_ui(
                                !self.yazi_settings.busy()
                                    && !self.search.snapshot.open
                                    && !self.palette.open
                                    && self.image_dialog.is_none()
                                    && self.rename_dialog.is_none()
                                    && self.delete_dialog.is_none(),
                                |ui| yazi.ui(ui, self.nerd_font_loaded, self.settings.text_size),
                            )
                            .inner;
                        if let Some(events) = events {
                            ui.input_mut(|i| i.events = events);
                        }
                    } else if ui.button("Restart Yazi").clicked() {
                        restart_yazi = true;
                    }
                });
        } else if self.show_yazi
            && let Some(error) = &self.yazi_error
        {
            ui.label(format!("Yazi unavailable: {error}"));
            if ui.button("Retry Yazi").clicked() {
                restart_yazi = true;
            }
        }
        if restart_yazi {
            self.restart_yazi();
            if let Some(yazi) = &mut self.yazi {
                yazi.request_focus();
            }
        }
        if terminal_has_focus {
            self.active_pane = ActivePane::Yazi;
        }
        if self.active_pane == ActivePane::Gallery
            && !self.search.snapshot.open
            && !self.palette.open
            && self.image_dialog.is_none()
            && self.rename_dialog.is_none()
            && self.delete_dialog.is_none()
            && !ctx.memory(|memory| memory.focused().is_some())
        {
            let layout = grid_layout(
                ui.available_width(),
                ui.spacing().item_spacing,
                self.settings.effective_gallery_scale(),
            );
            self.key_input(&ctx, layout.columns);
        }
        let gallery_heading = if self.active_pane == ActivePane::Gallery {
            "GALLERY · стрелки — выбор · Ctrl+K — Actions"
        } else {
            "Gallery · Tab"
        };
        ui.label(egui::RichText::new(gallery_heading).strong().color(
            if self.active_pane == ActivePane::Gallery {
                Color32::from_rgb(105, 189, 247)
            } else {
                ui.visuals().weak_text_color()
            },
        ));
        match self.gallery.as_ref().map(Gallery::view) {
            Some(View::Grid) => self.grid(ui),
            Some(View::Photo) => self.photo(ui),
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label("Enter a folder to begin");
                });
            }
        }
        self.actions_popup(&ctx);
        self.search_popup(&ctx);
        self.rename_popup(&ctx);
        self.image_popup(&ctx);
        self.delete_popup(&ctx);
        self.poll_mark_sync();
        self.poll_visual_probe();
        self.settings_window(&ctx);
    }
}
fn pasted_path(text: &str) -> PathBuf {
    let text = text.trim();
    PathBuf::from(
        text.strip_prefix('"')
            .and_then(|path| path.strip_suffix('"'))
            .unwrap_or(text),
    )
}
fn navigator_paths_in_gallery(
    folder: &Path,
    paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<PathBuf> {
    let root = friendly_path(folder);
    paths
        .into_iter()
        .filter_map(|path| {
            let friendly = PathBuf::from(friendly_path(&path));
            let root_path = PathBuf::from(&root);
            let prefix: Vec<_> = root_path.components().collect();
            let components: Vec<_> = friendly.components().collect();
            if components.len() <= prefix.len()
                || !prefix.iter().zip(&components).all(|(a, b)| {
                    a.as_os_str()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
                })
            {
                return None;
            }
            let suffix: PathBuf = components[prefix.len()..]
                .iter()
                .map(|c| c.as_os_str())
                .collect();
            if suffix
                .components()
                .any(|c| !matches!(c, std::path::Component::Normal(_)))
            {
                return None;
            }
            Some(folder.join(suffix))
        })
        .collect()
}
fn release_keyboard_focus(ui: &egui::Ui) {
    ui.ctx().memory_mut(|memory| {
        if let Some(id) = memory.focused() {
            memory.surrender_focus(id);
        }
    });
}
fn consume_plain_tab(ctx: &egui::Context) -> bool {
    ctx.input_mut(|input| {
        let pressed = input.events.iter().any(|event| {
            matches!(event, egui::Event::Key { key: Key::Tab, pressed: true, modifiers, .. } if modifiers.is_none())
        });
        if pressed {
            input.events.retain(|event| {
                !matches!(event, egui::Event::Key { key: Key::Tab, modifiers, .. } if modifiers.is_none())
            });
        }
        pressed
    })
}
fn grid_destination(current: usize, count: usize, columns: usize, key: Key) -> usize {
    if count == 0 {
        return 0;
    }
    match key {
        Key::ArrowRight => current.saturating_add(1).min(count - 1),
        Key::ArrowLeft => current.saturating_sub(1),
        Key::ArrowDown => current.saturating_add(columns.max(1)).min(count - 1),
        Key::ArrowUp => current.saturating_sub(columns.max(1)),
        _ => current,
    }
}
fn friendly_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}
fn same_folder(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
fn default_folder() -> PathBuf {
    if let Some(configured) = std::env::var_os("PHOTO_WORKSHOP_DEFAULT_FOLDER") {
        let folder = PathBuf::from(configured);
        if folder.is_dir() {
            return folder;
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let pictures = PathBuf::from(home).join("Pictures");
        if pictures.is_dir() {
            return pictures;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}
fn main() -> eframe::Result {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(default_folder);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([680.0, 460.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Afiocira",
        options,
        Box::new(move |cc| Ok(Box::new(Workshop::new(cc, &path)))),
    )
}
#[cfg(test)]
mod tests {
    use super::{
        consume_plain_tab, friendly_path, grid_destination, navigator_paths_in_gallery,
        pasted_path, same_folder,
    };
    use eframe::egui::{self, Key};
    use std::path::Path;
    #[test]
    fn accepts_explorer_copy_as_path() {
        assert_eq!(
            pasted_path("  \"C:\\Photos\\Summer 2026\"  ").to_string_lossy(),
            "C:\\Photos\\Summer 2026"
        );
    }
    #[test]
    fn yazi_and_gallery_paths_compare_by_directory_identity() {
        let folder = std::env::temp_dir();
        let equivalent = folder.join(".");
        assert!(same_folder(&folder, &equivalent));
        assert_eq!(
            friendly_path(Path::new(r"\\?\UNC\server\share\photos")),
            r"\\server\share\photos"
        );
    }
    #[test]
    fn yazi_paths_match_gallery_paths_without_reopening_each_file() {
        let gallery = Path::new(r"\\?\C:\Users\example\Pictures");
        let paths = navigator_paths_in_gallery(
            gallery,
            [
                r"C:\Users\example\Pictures\c.jpg".into(),
                r"C:\Users\example\Pictures\a.jpg".into(),
                r"C:\Users\example\Other\a.jpg".into(),
            ],
        );
        assert_eq!(paths, [gallery.join("c.jpg"), gallery.join("a.jpg")]);
    }
    #[test]
    fn grid_arrows_follow_rows_and_clamp_at_edges() {
        assert_eq!(grid_destination(0, 8, 3, Key::ArrowDown), 3);
        assert_eq!(grid_destination(3, 8, 3, Key::ArrowDown), 6);
        assert_eq!(grid_destination(6, 8, 3, Key::ArrowDown), 7);
        assert_eq!(grid_destination(7, 8, 3, Key::ArrowUp), 4);
        assert_eq!(grid_destination(0, 8, 3, Key::ArrowLeft), 0);
        assert_eq!(grid_destination(7, 8, 3, Key::ArrowRight), 7);
    }
    #[test]
    fn tab_switch_does_not_steal_shift_tab_from_yazi() {
        let ctx = egui::Context::default();
        let key = |modifiers| egui::Event::Key {
            key: Key::Tab,
            physical_key: Some(Key::Tab),
            pressed: true,
            repeat: false,
            modifiers,
        };
        ctx.begin_pass(egui::RawInput {
            events: vec![key(egui::Modifiers::NONE), key(egui::Modifiers::SHIFT)],
            ..Default::default()
        });
        assert!(consume_plain_tab(&ctx));
        assert_eq!(ctx.input(|input| input.events.len()), 1);
        assert!(ctx.input(|input| matches!(&input.events[0], egui::Event::Key { modifiers, .. } if modifiers.shift)));
        ctx.end_pass().textures_delta.clear();
    }
}
