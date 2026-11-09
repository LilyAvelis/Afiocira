//! Bounded background exploration, separate from image decoding and Yazi.
use super::*;
use photo_core::{api::NestedState, nested::Explorer};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::SyncSender,
};
pub(super) enum Task {
    Start(u64, PathBuf),
    More(u64, bool),
    Album(u64, PathBuf),
    Count(u64, PathBuf),
}
struct Reply {
    epoch: u64,
    photos: Vec<photo_core::Photo>,
    state: NestedState,
    album: Option<PathBuf>,
}
pub(super) struct State {
    pub force_grid: bool,
    pub returned_folder: Option<PathBuf>,
    pub snapshot: NestedState,
    pub(super) epoch: Arc<AtomicU64>,
    pub(super) sender: SyncSender<Task>,
    receiver: Receiver<Reply>,
    pending: Option<PathBuf>,
    pub recount: Option<PathBuf>,
    pub requested: bool,
    pub watched: HashSet<PathBuf>,
    pub cursor: Option<PathBuf>,
    pub positions: HashMap<PathBuf, usize>,
    pub advance: Option<PathBuf>,
    pub scroll_album: bool,
    pub pending_album: Option<PathBuf>,
    pub loading_album: Option<PathBuf>,
}
impl State {
    pub fn new(ctx: egui::Context) -> Self {
        let (sender, requests) = mpsc::sync_channel(1);
        let (replies, receiver) = mpsc::channel();
        let epoch = Arc::new(AtomicU64::new(0));
        let current = epoch.clone();
        std::thread::spawn(move || {
            let mut explorer: Option<Explorer> = None;
            let mut session = 0;
            let mut was_expanded = false;
            while let Ok(task) = requests.recv() {
                let album = match &task {
                    Task::Album(_, folder) => Some(folder.clone()),
                    _ => None,
                };
                let (id, photos, expanded, error) = match task {
                    Task::Start(id, root) => {
                        if current.load(Ordering::Relaxed) != id {
                            continue;
                        }
                        session = id;
                        was_expanded = false;
                        match Explorer::new(&root, || current.load(Ordering::Relaxed) != id) {
                            Ok(e) => {
                                explorer = Some(e);
                                (id, Vec::new(), false, None)
                            }
                            Err(e) => {
                                explorer = None;
                                (id, Vec::new(), false, Some(e.to_string()))
                            }
                        }
                    }
                    Task::Count(id, root) => {
                        if session != id || current.load(Ordering::Relaxed) != id {
                            continue;
                        }
                        let error = explorer
                            .as_mut()
                            .and_then(|e| {
                                e.recount(&root, || current.load(Ordering::Relaxed) != id)
                                    .err()
                            })
                            .map(|e| e.to_string());
                        (id, Vec::new(), was_expanded, error)
                    }
                    Task::Album(id, folder) => {
                        if session != id || current.load(Ordering::Relaxed) != id {
                            continue;
                        }
                        was_expanded = true;
                        let photos = explorer
                            .as_mut()
                            .map(|e| {
                                e.more_album(&folder, || current.load(Ordering::Relaxed) != id)
                            })
                            .unwrap_or_default();
                        (id, photos, true, None)
                    }
                    Task::More(id, deeper) => {
                        if session != id || current.load(Ordering::Relaxed) != id {
                            continue;
                        }
                        was_expanded = true;
                        let photos = explorer
                            .as_mut()
                            .map(|e| e.more(deeper, || current.load(Ordering::Relaxed) != id))
                            .unwrap_or_default();
                        (id, photos, true, None)
                    }
                };
                let state = if let Some(e) = &explorer {
                    NestedState {
                        overview: false,
                        active_album: None,
                        album_positions: Vec::new(),
                        albums: e.albums(),
                        subfolders: Some(e.subfolders),
                        loading: false,
                        expanded,
                        can_load_more: e.can_more(),
                        can_go_deeper: e.can_deepen(),
                        depth: e.depth,
                        loaded: e.loaded,
                        visited: e.visited,
                        limited: e.limited,
                        warnings: e.warnings.clone(),
                        error,
                    }
                } else {
                    NestedState {
                        error,
                        ..Default::default()
                    }
                };
                if replies
                    .send(Reply {
                        epoch: id,
                        photos,
                        state,
                        album,
                    })
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            force_grid: false,
            returned_folder: None,
            snapshot: NestedState::default(),
            epoch,
            sender,
            receiver,
            pending: None,
            recount: None,
            requested: false,
            watched: HashSet::new(),
            cursor: None,
            positions: HashMap::new(),
            advance: None,
            scroll_album: false,
            pending_album: None,
            loading_album: None,
        }
    }
    pub fn reset(&mut self, root: PathBuf) {
        self.force_grid = false;
        self.returned_folder = None;
        self.epoch.fetch_add(1, Ordering::Relaxed);
        self.snapshot = NestedState {
            loading: true,
            ..Default::default()
        };
        self.pending = Some(root);
        self.watched.clear();
        self.recount = None;
        self.requested = false;
        self.cursor = None;
        self.positions.clear();
        self.advance = None;
        self.pending_album = None;
        self.loading_album = None;
        self.scroll_album = false;
    }
}
impl Workshop {
    pub(super) fn load_nested(&mut self, deeper: bool) -> Result<(), String> {
        if self.nested.snapshot.loading {
            self.nested.requested = true;
            return Ok(());
        }
        if !(self.nested.snapshot.can_load_more || deeper && self.nested.snapshot.can_go_deeper) {
            return Ok(());
        }
        self.nested
            .sender
            .try_send(Task::More(
                self.nested.epoch.load(Ordering::Relaxed),
                deeper,
            ))
            .map_err(|e| e.to_string())?;
        self.nested.snapshot.loading = true;
        self.nested.snapshot.expanded = true;
        Ok(())
    }
    pub(super) fn poll_nested(&mut self) {
        if let Some(root) = self.nested.pending.take()
            && let Err(mpsc::TrySendError::Full(Task::Start(_, root))) = self
                .nested
                .sender
                .try_send(Task::Start(self.nested.epoch.load(Ordering::Relaxed), root))
        {
            self.nested.pending = Some(root);
        }
        if self.nested.pending.is_none()
            && !self.nested.snapshot.loading
            && let Some(root) = self.nested.recount.take()
        {
            match self
                .nested
                .sender
                .try_send(Task::Count(self.nested.epoch.load(Ordering::Relaxed), root))
            {
                Ok(()) => self.nested.snapshot.loading = true,
                Err(mpsc::TrySendError::Full(Task::Count(_, root))) => {
                    self.nested.recount = Some(root)
                }
                Err(mpsc::TrySendError::Disconnected(_)) => {
                    self.nested.snapshot.error = Some("Обход папок недоступен".into())
                }
                Err(_) => {}
            }
        }
        while let Ok(reply) = self.nested.receiver.try_recv() {
            if reply.epoch != self.nested.epoch.load(Ordering::Relaxed) {
                continue;
            }
            self.nested.snapshot = reply.state;
            if !reply.photos.is_empty() {
                for photo in &reply.photos {
                    if let Some(folder) = photo.path.parent()
                        && self.nested.watched.insert(folder.to_path_buf())
                        && let Some(watcher) = &mut self.watcher
                    {
                        let _ = watcher.watch(folder, RecursiveMode::NonRecursive);
                    }
                }
                if let Some(gallery) = &mut self.gallery {
                    gallery.append_nested(reply.photos);
                }
                self.group_album_members();
            }
            if let Some(folder) = reply.album {
                self.nested.loading_album = None;
                self.finish_album_request(&folder);
            }
        }
        if !self.nested.snapshot.loading
            && let Some(folder) = self.nested.pending_album.take()
        {
            let _ = self.load_album(folder);
        }
        if self.nested.requested && !self.nested.snapshot.loading {
            self.nested.requested = false;
            if self.album_mode() {
                self.activate_album();
            } else {
                let _ = self.load_nested(false);
            }
        }
    }
    pub(super) fn nested_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if let Some(count) = self.nested.snapshot.subfolders {
                ui.weak(format!("Подпапок на этом уровне: {count}"));
            } else {
                ui.weak("Альбомы");
            }
            if self.album_mode() {
                ui.add(Lucide::MoveHorizontal.size(14.0).image());
                ui.weak("кадры");
                ui.add(Lucide::MoveVertical.size(14.0).image());
                ui.weak("альбомы · Enter войти · Space просмотр");
            }
            if self.nested.snapshot.limited {
                ui.weak("Предел просмотра. Продолжить в папке через Yazi.");
            }
        });
        if let Some(error) = &self.nested.snapshot.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }
}
