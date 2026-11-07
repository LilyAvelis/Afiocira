#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod decode;
mod ipc;
mod yazi;

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

use decode::{DecodeQueue, DecodeResult, DecodeTask, spawn_worker};
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

fn grid_layout(available_width: f32, spacing: Vec2) -> GridLayout {
    // Leave room for the vertical scrollbar so the final column stays visible.
    let width = (available_width - 16.0).max(1.0);
    let columns = ((width + spacing.x) / (TARGET_TILE_WIDTH + spacing.x))
        .floor()
        .max(1.0) as usize;
    let tile_width = ((width - spacing.x * (columns - 1) as f32) / columns as f32).max(1.0);
    let image_height = (tile_width * 0.8).clamp(150.0, 360.0);
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
    ctx: egui::Context,
    session_id: String,
    yazi: Option<YaziPane>,
    yazi_error: Option<String>,
    show_yazi: bool,
    nerd_font_loaded: bool,
    active_pane: ActivePane,
    scroll_to_grid_anchor: bool,
    selection_dirty: bool,
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
    fn new(ctx: &egui::Context, initial: &Path) -> Self {
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
        let (yazi, yazi_error) = match YaziPane::start(yazi_folder) {
            Ok(pane) => (Some(pane), None),
            Err(error) => (None, Some(error)),
        };
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

    fn open_path(&mut self, path: &Path) -> bool {
        let result = if path.is_file() {
            Gallery::open_file(path)
        } else {
            Gallery::open(path)
        };
        match result {
            Ok(gallery) => {
                self.generation += 1;
                self.thumbnail_tasks.set_generation(self.generation);
                self.preview_tasks.set_generation(self.generation);
                self.folder_input = friendly_path(gallery.folder());
                self.message = format!("{} photos", gallery.photos().len());
                self.gallery = Some(gallery);
                self.selection_dirty = false;
                self.clear_cached_images();
                let folder = self.gallery.as_ref().unwrap().folder().to_path_buf();
                self.watch_folder(&folder);
                self.scroll_to_grid_anchor = true;
                true
            }
            Err(error) => {
                self.message = format!("Could not open: {error}");
                false
            }
        }
    }

    fn clear_cached_images(&mut self) {
        self.thumbnails.clear();
        self.thumbnail_order.clear();
        self.pending_thumbnails.clear();
        self.previews.clear();
        self.preview_order.clear();
        self.pending_previews.clear();
        self.errors.clear();
    }

    fn remap_cached_images(&mut self, old_paths: &[PathBuf]) {
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

    fn refresh_now(&mut self) -> Result<(), String> {
        let gallery = self.gallery.as_mut().ok_or("No open gallery")?;
        gallery.refresh().map_err(|error| error.to_string())?;
        self.generation += 1;
        self.thumbnail_tasks.set_generation(self.generation);
        self.preview_tasks.set_generation(self.generation);
        self.clear_cached_images();
        self.scroll_to_grid_anchor = true;
        self.message = format!("{} photos", self.gallery.as_ref().unwrap().photos().len());
        Ok(())
    }

    fn watch_folder(&mut self, folder: &Path) {
        let (sender, receiver) = mpsc::channel();
        match notify::recommended_watcher(sender).and_then(|mut watcher| {
            watcher.watch(folder, RecursiveMode::NonRecursive)?;
            Ok(watcher)
        }) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.file_events = Some(receiver);
            }
            Err(error) => {
                self.watcher = None;
                self.file_events = None;
                self.message = format!("File changes will need manual refresh: {error}");
            }
        }
        self.refresh_after = None;
        self.dirty_names.clear();
        self.dirty_all = false;
    }

    fn poll_file_changes(&mut self) {
        if let Some(events) = &self.file_events {
            let mut saw_event = false;
            for event in events.try_iter() {
                saw_event = true;
                match event {
                    Ok(event) if !event.paths.is_empty() => {
                        for path in event.paths {
                            if let Some(name) = path.file_name() {
                                self.dirty_names.insert(name.to_owned());
                            }
                        }
                    }
                    _ => self.dirty_all = true,
                }
            }
            if saw_event {
                self.refresh_after = Some(Instant::now() + Duration::from_millis(350));
            }
        }
        if self
            .refresh_after
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.refresh_after = None;
            match self.gallery.as_mut().map(Gallery::refresh) {
                Some(Ok(list_changed)) if list_changed || self.dirty_all => {
                    self.generation += 1;
                    self.thumbnail_tasks.set_generation(self.generation);
                    self.preview_tasks.set_generation(self.generation);
                    self.clear_cached_images();
                    self.scroll_to_grid_anchor = true;
                    self.message =
                        format!("{} photos", self.gallery.as_ref().unwrap().photos().len());
                }
                Some(Ok(false)) => {
                    let dirty: Vec<_> = self
                        .gallery
                        .as_ref()
                        .unwrap()
                        .photos()
                        .iter()
                        .enumerate()
                        .filter(|(_, photo)| {
                            photo
                                .path
                                .file_name()
                                .is_some_and(|name| self.dirty_names.contains(name))
                        })
                        .map(|(index, _)| index)
                        .collect();
                    if !dirty.is_empty() {
                        self.generation += 1;
                        self.thumbnail_tasks.set_generation(self.generation);
                        self.preview_tasks.set_generation(self.generation);
                        self.pending_thumbnails.clear();
                        self.pending_previews.clear();
                        for index in dirty {
                            self.thumbnails.remove(&index);
                            self.previews.remove(&index);
                            self.errors.remove(&index);
                            self.thumbnail_order.retain(|item| *item != index);
                            self.preview_order.retain(|item| *item != index);
                        }
                    }
                }
                Some(Err(error)) => self.message = format!("Could not refresh gallery: {error}"),
                _ => {}
            }
            self.dirty_names.clear();
            self.dirty_all = false;
        }
    }

    fn focus_gallery(&mut self) -> Result<(), String> {
        if let Some(yazi) = &mut self.yazi
            && yazi.is_running()
        {
            yazi.request_snapshot()?;
            let deadline = Instant::now() + Duration::from_secs(1);
            let snapshot = loop {
                if let Some(snapshot) = yazi.poll_snapshot() {
                    break snapshot;
                }
                if Instant::now() >= deadline {
                    return Err("Yazi selection timed out".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            let target_folder = snapshot
                .hovered
                .as_deref()
                .filter(|path| path.is_dir())
                .unwrap_or(&snapshot.folder);
            if self
                .gallery
                .as_ref()
                .is_none_or(|gallery| !same_folder(gallery.folder(), target_folder))
                && !self.open_path(target_folder)
            {
                return Err(self.message.clone());
            }
            let gallery_folder = self.gallery.as_ref().unwrap().folder().to_path_buf();
            if same_folder(&gallery_folder, &snapshot.folder) {
                self.execute(Command::SetOrder {
                    paths: navigator_paths_in_gallery(&gallery_folder, snapshot.ordered),
                });
            }
            if let Some(gallery) = &mut self.gallery {
                gallery
                    .set_marked_paths(navigator_paths_in_gallery(&gallery_folder, snapshot.marked));
                if let Some(path) = snapshot.hovered {
                    if let Some(path) = navigator_paths_in_gallery(&gallery_folder, [path]).pop() {
                        gallery.select_path(&path);
                    }
                }
            }
            self.selection_dirty = false;
        }
        self.active_pane = ActivePane::Gallery;
        self.scroll_to_grid_anchor = true;
        self.ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
        Ok(())
    }

    fn focus_navigator(&mut self) -> Result<(), String> {
        let yazi = self.yazi.as_mut().ok_or("Navigator unavailable")?;
        if !yazi.is_running() {
            return Err("Navigator has exited; restart Yazi".into());
        }
        if let Some(gallery) = &self.gallery
            && let Some(cursor) = gallery.cursor_photo()
        {
            let marked: Vec<_> = gallery.marked_paths().iter().cloned().collect();
            yazi.apply_selection(&cursor.path, &marked, self.selection_dirty)?;
            self.selection_dirty = false;
        }
        self.show_yazi = true;
        self.active_pane = ActivePane::Yazi;
        yazi.request_focus();
        Ok(())
    }

    fn execute(&mut self, command: Command) -> Response {
        let include_order = matches!(command, Command::ListOrder);
        let error_code = match &command {
            Command::Open { .. } | Command::PreviewFolder { .. } => ErrorCode::InvalidPath,
            Command::Select { .. }
            | Command::SelectPath { .. }
            | Command::ExtendSelection { .. }
            | Command::SetMarked { .. }
            | Command::SetOrder { .. }
            | Command::OpenSelected => ErrorCode::InvalidSelection,
            Command::Refresh => ErrorCode::Internal,
            _ => ErrorCode::InvalidState,
        };
        let result: Result<(), String> = match command {
            Command::Status | Command::ListOrder => Ok(()),
            Command::Refresh => self.refresh_now(),
            Command::SetNavigatorVisible { visible } => {
                if visible {
                    self.focus_navigator()
                } else {
                    self.focus_gallery().map(|()| {
                        self.show_yazi = false;
                    })
                }
            }
            Command::FocusPane {
                pane: Pane::Gallery,
            } => self.focus_gallery(),
            Command::FocusPane {
                pane: Pane::Navigator,
            } => self.focus_navigator(),
            Command::Open { path } => {
                if self.open_path(&path) {
                    self.restart_yazi();
                    self.active_pane = ActivePane::Gallery;
                    self.ctx.memory_mut(|memory| {
                        if let Some(id) = memory.focused() {
                            memory.surrender_focus(id);
                        }
                    });
                    Ok(())
                } else {
                    Err(self.message.clone())
                }
            }
            Command::PreviewFolder { path } => self
                .open_path(&path)
                .then_some(())
                .ok_or_else(|| self.message.clone()),
            Command::Select { index } => {
                if self
                    .gallery
                    .as_mut()
                    .is_some_and(|gallery| gallery.set_grid_anchor(index))
                {
                    self.scroll_to_grid_anchor = true;
                    self.selection_dirty = true;
                    Ok(())
                } else {
                    Err("No such gallery index".into())
                }
            }
            Command::SelectPath { path } => {
                let result = self
                    .gallery
                    .as_mut()
                    .is_some_and(|gallery| gallery.select_path(&path));
                if result {
                    self.scroll_to_grid_anchor = true;
                    self.selection_dirty = true;
                    Ok(())
                } else {
                    Err("Photo is not in this gallery".into())
                }
            }
            Command::ExtendSelection { index } => {
                let result = self
                    .gallery
                    .as_mut()
                    .is_some_and(|gallery| gallery.extend_selection(index));
                if result {
                    self.scroll_to_grid_anchor = true;
                    self.selection_dirty = true;
                    Ok(())
                } else {
                    Err("No such gallery index".into())
                }
            }
            Command::SetMarked { paths } => {
                if let Some(gallery) = &mut self.gallery {
                    gallery.set_marked_paths(paths);
                    self.selection_dirty = true;
                    Ok(())
                } else {
                    Err("No open gallery".into())
                }
            }
            Command::SetOrder { paths } => {
                if let Some(gallery) = &mut self.gallery {
                    let old_paths: Vec<_> = gallery
                        .photos()
                        .iter()
                        .map(|photo| photo.path.clone())
                        .collect();
                    if gallery.set_order(&paths) {
                        self.remap_cached_images(&old_paths);
                    }
                    Ok(())
                } else {
                    Err("No open gallery".into())
                }
            }
            Command::OpenSelected => self
                .gallery
                .as_mut()
                .map(|gallery| gallery.open_photo(gallery.grid_anchor()))
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "No selected photo".into()),
            Command::Next => self
                .gallery
                .as_mut()
                .map(Gallery::next_photo)
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "No next photo".into()),
            Command::Previous => self
                .gallery
                .as_mut()
                .map(Gallery::previous_photo)
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "No previous photo".into()),
            Command::Back => self
                .gallery
                .as_mut()
                .map(|gallery| {
                    gallery.back_to_grid();
                    self.scroll_to_grid_anchor = true;
                })
                .ok_or_else(|| "No open gallery".into()),
            Command::ZoomAt {
                multiplier,
                anchor,
                viewport,
                fitted,
            } => self
                .gallery
                .as_mut()
                .map(|gallery| gallery.zoom_at(multiplier, anchor, viewport, fitted))
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "Cannot zoom at this point".into()),
            Command::PanBy {
                delta,
                viewport,
                fitted,
            } => self
                .gallery
                .as_mut()
                .map(|gallery| gallery.pan_by(delta, viewport, fitted))
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "Cannot pan this image".into()),
            Command::ResetZoom => self
                .gallery
                .as_mut()
                .map(Gallery::reset_zoom)
                .ok_or_else(|| "No open gallery".into()),
        };
        let mut response = match result {
            Ok(()) => Response::ok(self.gallery.as_ref()),
            Err(error) => Response::error(error_code, error, self.gallery.as_ref()),
        };
        response.session_id = Some(self.session_id.clone());
        if include_order
            && let Some(snapshot) = &mut response.snapshot
            && let Some(gallery) = &self.gallery
        {
            snapshot.ordered_paths = Some(
                gallery
                    .photos()
                    .iter()
                    .map(|photo| photo.path.clone())
                    .collect(),
            );
        }
        response.navigator = Some(NavigatorState {
            visible: self.show_yazi,
            focus: if self.active_pane == ActivePane::Gallery {
                Pane::Gallery
            } else {
                Pane::Navigator
            },
        });
        response
    }

    fn poll_api(&mut self) {
        let requests: Vec<_> = self
            .api_requests
            .as_ref()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default();
        for request in requests {
            let response = self.execute(request.command);
            let _ = request.reply.send(response);
        }
    }

    fn restart_yazi(&mut self) {
        let folder = self
            .gallery
            .as_ref()
            .map_or_else(default_folder, |gallery| gallery.folder().to_path_buf());
        match YaziPane::start(&folder) {
            Ok(pane) => {
                self.yazi = Some(pane);
                self.yazi_error = None;
                self.active_pane = ActivePane::Yazi;
            }
            Err(error) => {
                self.yazi = None;
                self.yazi_error = Some(error);
            }
        }
    }

    fn poll_results(&mut self, ctx: &egui::Context) {
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

    fn request_thumbnail(&mut self, index: usize) {
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

    fn request_preview(&mut self, index: usize) {
        if self.previews.contains_key(&index) || self.errors.contains_key(&index) {
            return;
        }
        if self.pending_previews.contains(&index) {
            self.preview_tasks.promote(index);
            return;
        }
        if let Some(photo) = self.gallery.as_ref().and_then(|g| g.photos().get(index)) {
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

    fn prefetch_previews(&mut self, index: usize) {
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

    fn key_input(&mut self, ctx: &egui::Context, columns: usize) {
        if ctx.input(|input| input.key_pressed(Key::F5)) {
            self.execute(Command::Refresh);
            return;
        }
        let Some(gallery) = self.gallery.as_ref() else {
            return;
        };
        let pressed = |key| ctx.input(|input| input.key_pressed(key));
        match gallery.view() {
            View::Grid => {
                if pressed(Key::Escape) {
                    self.execute(Command::SetMarked { paths: Vec::new() });
                    return;
                }
                let count = gallery.photos().len();
                if count == 0 {
                    return;
                }
                let current = gallery.grid_anchor();
                for key in [
                    Key::ArrowRight,
                    Key::ArrowLeft,
                    Key::ArrowDown,
                    Key::ArrowUp,
                ] {
                    if pressed(key) {
                        let next = grid_destination(current, count, columns, key);
                        if next != current {
                            if ctx.input(|input| input.modifiers.shift) {
                                self.execute(Command::ExtendSelection { index: next });
                            } else {
                                self.execute(Command::Select { index: next });
                            }
                        }
                        break;
                    }
                }
                if pressed(Key::Enter) || pressed(Key::Space) {
                    self.execute(Command::OpenSelected);
                }
            }
            View::Photo => {
                if pressed(Key::ArrowRight) {
                    self.execute(Command::Next);
                }
                if pressed(Key::ArrowLeft) {
                    self.execute(Command::Previous);
                }
                if pressed(Key::Escape) || pressed(Key::Space) {
                    self.execute(Command::Back);
                }
            }
        }
    }

    fn grid(&mut self, ui: &mut egui::Ui) {
        let count = self.gallery.as_ref().map_or(0, |g| g.photos().len());
        if count == 0 {
            ui.centered_and_justified(|ui| {
                ui.label("No images in this folder");
            });
            return;
        }
        let layout = grid_layout(ui.available_width(), ui.spacing().item_spacing);
        let rows = count.div_ceil(layout.columns);
        let mut scroll = egui::ScrollArea::vertical().id_salt("gallery-grid");
        if self.scroll_to_grid_anchor {
            let id = ui.make_persistent_id("gallery-grid");
            let offset = egui::containers::scroll_area::State::load(ui.ctx(), id)
                .map_or(0.0, |state| state.offset.y);
            let row = self.gallery.as_ref().unwrap().grid_anchor() / layout.columns;
            let top = row as f32 * layout.row_height;
            let bottom = top + layout.row_height;
            let height = ui.available_height();
            let target = if top < offset {
                top
            } else if bottom > offset + height {
                bottom - height
            } else {
                offset
            };
            scroll = scroll.vertical_scroll_offset(target.max(0.0));
            self.scroll_to_grid_anchor = false;
        }
        scroll.show_rows(ui, layout.row_height, rows, |ui, row_range| {
            for row in row_range {
                ui.horizontal(|ui| {
                    for column in 0..layout.columns {
                        let index = row * layout.columns + column;
                        if index >= count {
                            break;
                        }
                        let name = self.gallery.as_ref().unwrap().photos()[index].name.clone();
                        self.request_thumbnail(index);
                        ui.vertical(|ui| {
                            ui.set_width(layout.tile_width);
                            let response = if let Some(texture) = self.thumbnails.get(&index) {
                                ui.add(
                                    egui::Image::new(texture)
                                        .fit_to_exact_size(Vec2::new(
                                            layout.tile_width,
                                            layout.image_height,
                                        ))
                                        .sense(egui::Sense::click()),
                                )
                            } else {
                                ui.add_sized(
                                    [layout.tile_width, layout.image_height],
                                    egui::Button::new(if self.errors.contains_key(&index) {
                                        "Cannot preview"
                                    } else {
                                        "Loading…"
                                    }),
                                )
                            };
                            if response.clicked() {
                                release_keyboard_focus(ui);
                                self.active_pane = ActivePane::Gallery;
                                if ui.input(|input| input.modifiers.shift) {
                                    self.execute(Command::ExtendSelection { index });
                                } else {
                                    self.execute(Command::Select { index });
                                    self.execute(Command::OpenSelected);
                                }
                            }
                            if self
                                .gallery
                                .as_ref()
                                .unwrap()
                                .marked_paths()
                                .contains(&self.gallery.as_ref().unwrap().photos()[index].path)
                            {
                                ui.painter().rect_filled(
                                    response.rect.shrink(2.0),
                                    3.0,
                                    Color32::from_rgba_unmultiplied(105, 189, 247, 30),
                                );
                                ui.painter().rect_stroke(
                                    response.rect.expand(1.0),
                                    3.0,
                                    egui::Stroke::new(3.0, Color32::from_rgb(247, 178, 88)),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            if self.gallery.as_ref().unwrap().grid_anchor() == index {
                                let color = if self.active_pane == ActivePane::Gallery {
                                    Color32::from_rgb(105, 189, 247)
                                } else {
                                    Color32::from_rgb(84, 100, 110)
                                };
                                ui.painter().rect_stroke(
                                    response.rect.expand(2.0),
                                    3.0,
                                    egui::Stroke::new(2.0, color),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            ui.add_sized(
                                [layout.tile_width, 24.0],
                                egui::Label::new(egui::RichText::new(&name).size(12.0)).truncate(),
                            )
                            .on_hover_text(name);
                        });
                    }
                });
            }
        });
    }

    fn photo(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::image_and_text(
                    Lucide::Images.size(16.0).image(),
                    "Gallery · Esc/Space",
                ))
                .clicked()
            {
                self.execute(Command::Back);
                self.active_pane = ActivePane::Gallery;
                release_keyboard_focus(ui);
            }
            if ui
                .add(egui::Button::image_and_text(
                    Lucide::ArrowLeft.size(16.0).image(),
                    "Previous",
                ))
                .clicked()
            {
                self.execute(Command::Previous);
                self.active_pane = ActivePane::Gallery;
                release_keyboard_focus(ui);
            }
            if ui
                .add(egui::Button::image_and_text(
                    Lucide::ArrowRight.size(16.0).image(),
                    "Next",
                ))
                .clicked()
            {
                self.execute(Command::Next);
                self.active_pane = ActivePane::Gallery;
                release_keyboard_focus(ui);
            }
            if ui
                .add(egui::Button::image_and_text(
                    Lucide::Maximize2.size(16.0).image(),
                    "Fit",
                ))
                .clicked()
            {
                self.execute(Command::ResetZoom);
                self.active_pane = ActivePane::Gallery;
                release_keyboard_focus(ui);
            }
            ui.label(format!(
                "Fit ×{:.1}",
                self.gallery.as_ref().unwrap().zoom().factor
            ));
        });
        let Some(index) = self.gallery.as_ref().and_then(Gallery::selected_index) else {
            return;
        };
        self.prefetch_previews(index);
        let name = self.gallery.as_ref().unwrap().photos()[index].name.clone();
        ui.label(name);
        ui.separator();
        let available = ui.available_size();
        let (rect, response) = ui.allocate_exact_size(available, egui::Sense::drag());
        if response.clicked() || response.drag_started() {
            release_keyboard_focus(ui);
            self.active_pane = ActivePane::Gallery;
        }
        if let Some(texture) = self
            .previews
            .get(&index)
            .or_else(|| self.thumbnails.get(&index))
            .cloned()
        {
            let natural = texture.size_vec2();
            let fit = (rect.width() / natural.x).min(rect.height() / natural.y);
            let fitted = natural * fit;
            let viewport = Size {
                width: rect.width(),
                height: rect.height(),
            };
            let fitted_size = Size {
                width: fitted.x,
                height: fitted.y,
            };
            if response.hovered() {
                let (wheel, center_zoom, cursor) = ui.input(|input| {
                    (
                        input.smooth_scroll_delta().y,
                        input.zoom_delta(),
                        input.pointer.hover_pos(),
                    )
                });
                if wheel.abs() > 0.01
                    && let Some(cursor) = cursor
                {
                    self.execute(Command::ZoomAt {
                        multiplier: (wheel * 0.003).exp(),
                        anchor: Point {
                            x: cursor.x - rect.min.x,
                            y: cursor.y - rect.min.y,
                        },
                        viewport,
                        fitted: fitted_size,
                    });
                }
                if (center_zoom - 1.0).abs() > 0.001 {
                    self.execute(Command::ZoomAt {
                        multiplier: center_zoom,
                        anchor: Point {
                            x: viewport.width / 2.0,
                            y: viewport.height / 2.0,
                        },
                        viewport,
                        fitted: fitted_size,
                    });
                }
            }
            if response.dragged() {
                let delta = ui.input(|input| input.pointer.delta());
                self.execute(Command::PanBy {
                    delta: Point {
                        x: delta.x,
                        y: delta.y,
                    },
                    viewport,
                    fitted: fitted_size,
                });
            }
            let gallery = self.gallery.as_mut().unwrap();
            gallery.constrain_zoom(viewport, fitted_size);
            let zoom = gallery.zoom();
            let image_rect = egui::Rect::from_center_size(
                rect.center() + Vec2::new(zoom.offset.x, zoom.offset.y),
                fitted * zoom.factor,
            );
            ui.painter_at(rect).image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            return;
        }
        let text = self.errors.get(&index).map_or("Loading…", String::as_str);
        ui.painter_at(rect).text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional(16.0),
            Color32::LIGHT_GRAY,
        );
    }
}

impl Drop for Workshop {
    fn drop(&mut self) {
        self.thumbnail_tasks.close();
        self.preview_tasks.close();
    }
}

impl eframe::App for Workshop {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_api();
        self.poll_file_changes();
        self.poll_results(&ctx);
        if let Some(folder) = self.yazi.as_mut().and_then(YaziPane::changed_folder) {
            if self
                .gallery
                .as_ref()
                .is_none_or(|gallery| !same_folder(gallery.folder(), &folder))
            {
                self.execute(Command::PreviewFolder { path: folder });
            }
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
        ui.horizontal(|ui| {
            ui.heading("Afiocira");
            ui.separator();
            if ui
                .add(egui::Button::new("Yazi").selected(self.show_yazi))
                .clicked()
            {
                self.execute(Command::SetNavigatorVisible {
                    visible: !self.show_yazi,
                });
            }
            ui.label("Folder or image:");
            let input =
                ui.add(egui::TextEdit::singleline(&mut self.folder_input).desired_width(430.0));
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
            if ui.button("Refresh · F5").clicked() {
                self.execute(Command::Refresh);
                release_keyboard_focus(ui);
            }
            ui.label(&self.message);
            if let Some(error) = &self.api_error {
                ui.colored_label(Color32::YELLOW, error);
            }
        });
        ui.add_space(8.0);
        ui.separator();
        let yazi_running = self.yazi.as_mut().is_some_and(YaziPane::is_running);
        if yazi_running && !path_input_focused && consume_plain_tab(&ctx) {
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
                        "YAZI · KEYBOARD ACTIVE"
                    } else {
                        "Yazi · Tab to focus"
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
                        terminal_has_focus = yazi.ui(ui, self.nerd_font_loaded);
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
        }
        if terminal_has_focus {
            self.active_pane = ActivePane::Yazi;
        }
        if self.active_pane == ActivePane::Gallery
            && !ctx.memory(|memory| memory.focused().is_some())
        {
            let layout = grid_layout(ui.available_width(), ui.spacing().item_spacing);
            self.key_input(&ctx, layout.columns);
        }
        let gallery_heading = if self.active_pane == ActivePane::Gallery {
            "GALLERY · KEYBOARD ACTIVE · arrows select · Enter/Space opens"
        } else {
            "Gallery · Tab to focus"
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
            let parent = path.parent()?;
            let name = path.file_name()?;
            friendly_path(parent)
                .eq_ignore_ascii_case(&root)
                .then(|| folder.join(name))
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
        Box::new(move |cc| Ok(Box::new(Workshop::new(&cc.egui_ctx, &path)))),
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
