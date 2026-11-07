//! One bounded background worker; requests capture paths and destination folders.
use super::*;
use photo_core::actions::ActionId;
use photo_core::api::{FileResult, OperationState};
use std::sync::mpsc::SyncSender;

pub(super) struct Worker {
    tasks: SyncSender<Task>,
    results: Receiver<Completed>,
}
struct Completed {
    state: OperationState,
    submitted: Instant,
}
struct Task {
    submitted: Instant,
    id: u64,
    kind: TaskKind,
}
pub(super) enum TaskKind {
    Image {
        source: PathBuf,
        version: photo_core::rename::SourceVersion,
        options: photo_core::api::ImageOptions,
        convert: bool,
    },
    Delete(Vec<photo_core::recycle::Target>),
    Duplicate(Vec<PathBuf>),
    Copy(Vec<PathBuf>),
    Paste {
        folder: PathBuf,
    },
    Import {
        folder: PathBuf,
        paths: Vec<PathBuf>,
    },
    Rename {
        source: PathBuf,
        new_name: String,
        version: photo_core::rename::SourceVersion,
    },
}
impl TaskKind {
    fn action(&self) -> ActionId {
        match self {
            Self::Image { convert, .. } => {
                if *convert {
                    ActionId::ConvertImage
                } else {
                    ActionId::ResizeImage
                }
            }
            Self::Delete(_) => ActionId::Delete,
            Self::Duplicate(_) => ActionId::Duplicate,
            Self::Copy(_) => ActionId::Copy,
            Self::Paste { .. } | Self::Import { .. } => ActionId::Paste,
            Self::Rename { .. } => ActionId::Rename,
        }
    }
    fn paths(&self) -> Vec<PathBuf> {
        match self {
            Self::Image { source, .. } => vec![source.clone()],
            Self::Delete(targets) => targets.iter().map(|t| t.path.clone()).collect(),
            Self::Duplicate(paths) | Self::Copy(paths) | Self::Import { paths, .. } => {
                paths.clone()
            }
            Self::Rename { source, .. } => vec![source.clone()],
            Self::Paste { .. } => Vec::new(),
        }
    }
}
pub(super) struct Origin {
    folder: PathBuf,
    video: Option<photo_core::api::VideoState>,
    marks: std::collections::BTreeSet<PathBuf>,
    view: View,
    pub(super) revision: u64,
    pub(super) delete_order: Vec<PathBuf>,
    pub(super) delete_cursor: usize,
}
fn outcome(source: PathBuf, result: std::io::Result<PathBuf>) -> FileResult {
    let (destination, error) = match result {
        Ok(path) => (Some(path), None),
        Err(e) => (None, Some(e.to_string())),
    };
    FileResult {
        source,
        destination,
        error,
    }
}
fn import_files(folder: &Path, paths: Vec<PathBuf>) -> Vec<FileResult> {
    paths
        .into_iter()
        .map(|source| {
            let result = photo_core::import::copy_file(&source, folder);
            outcome(source, result)
        })
        .collect()
}
impl Worker {
    pub fn start(ctx: egui::Context) -> Self {
        let (tasks, receiver) = mpsc::sync_channel::<Task>(1);
        let (sender, results) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(task) = receiver.recv() {
                let work_started = Instant::now();
                let mut state = OperationState {
                    id: task.id,
                    action: task.kind.action(),
                    running: false,
                    files: Vec::new(),
                    warnings: Vec::new(),
                    timing: Some(photo_core::api::OperationTiming {
                        prepare_and_queue_ms: task.submitted.elapsed().as_secs_f64() * 1000.0,
                        ..Default::default()
                    }),
                };
                match task.kind {
                    TaskKind::Image {
                        source,
                        version,
                        options,
                        convert,
                    } => {
                        let result =
                            crate::image_actions::process(&source, &version, &options, convert);
                        let (destination, error) = match result {
                            Ok((path, notice)) => {
                                state.warnings.push(notice);
                                (Some(path), None)
                            }
                            Err(e) => (None, Some(e)),
                        };
                        state.files.push(FileResult {
                            source,
                            destination,
                            error,
                        });
                    }
                    TaskKind::Delete(targets) => {
                        state.files = targets
                            .into_iter()
                            .map(|target| {
                                let result = photo_core::recycle::recycle_checked(&target);
                                FileResult {
                                    source: target.path,
                                    destination: None,
                                    error: result.err().map(|e| e.to_string()),
                                }
                            })
                            .collect();
                    }
                    TaskKind::Duplicate(paths) => {
                        state.files = paths
                            .into_iter()
                            .map(|source| {
                                let result = photo_core::file_ops::duplicate(&source);
                                outcome(source, result)
                            })
                            .collect()
                    }
                    TaskKind::Copy(paths) => {
                        let error = match crate::clipboard::copy(&paths) {
                            Ok(warnings) => {
                                state.warnings = warnings;
                                None
                            }
                            Err(e) => Some(e),
                        };
                        state.files = paths
                            .into_iter()
                            .map(|source| FileResult {
                                source,
                                destination: None,
                                error: error.clone(),
                            })
                            .collect();
                    }
                    TaskKind::Import { folder, paths } => {
                        state.files = import_files(&folder, paths)
                    }
                    TaskKind::Paste { folder } => match crate::clipboard_input::read() {
                        Ok(crate::clipboard_input::Payload::Files(paths)) => {
                            state.files = import_files(&folder, paths)
                        }
                        Ok(crate::clipboard_input::Payload::Png(bytes)) => {
                            state.files.push(outcome(
                                PathBuf::from("[clipboard image]"),
                                photo_core::import::write_bytes(
                                    &folder,
                                    "Pasted image.png",
                                    &bytes,
                                ),
                            ))
                        }
                        Err(error) => state.files.push(FileResult {
                            source: PathBuf::from("[clipboard]"),
                            destination: None,
                            error: Some(error),
                        }),
                    },
                    TaskKind::Rename {
                        source,
                        new_name,
                        version,
                    } => {
                        let started = Instant::now();
                        let result = loop {
                            let result =
                                photo_core::rename::rename_checked(&source, &new_name, &version);
                            if result
                                .as_ref()
                                .err()
                                .is_some_and(|e| matches!(e.raw_os_error(), Some(32 | 33)))
                                && started.elapsed() < Duration::from_millis(700)
                            {
                                std::thread::sleep(Duration::from_millis(20));
                            } else {
                                break result;
                            }
                        };
                        state.files.push(outcome(source, result));
                    }
                }
                if let Some(timing) = &mut state.timing {
                    timing.work_ms = work_started.elapsed().as_secs_f64() * 1000.0;
                }
                if sender
                    .send(Completed {
                        state,
                        submitted: task.submitted,
                    })
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self { tasks, results }
    }
}
impl Workshop {
    pub(super) fn action_paths(&self) -> Vec<PathBuf> {
        let Some(gallery) = &self.gallery else {
            return Vec::new();
        };
        scoped_paths(gallery)
    }
    pub(super) fn start_operation(&mut self, action: ActionId) -> Result<(), String> {
        let kind = match action {
            ActionId::Paste => TaskKind::Paste {
                folder: self
                    .gallery
                    .as_ref()
                    .ok_or("No open folder")?
                    .folder()
                    .to_path_buf(),
            },
            ActionId::Copy | ActionId::Duplicate => {
                let paths = self.action_paths();
                if paths.is_empty() {
                    return Err("No files selected".into());
                }
                if action == ActionId::Copy {
                    TaskKind::Copy(paths)
                } else {
                    TaskKind::Duplicate(paths)
                }
            }
            _ => return Err("Action requires its own arguments".into()),
        };
        self.start_task(kind)
    }
    pub(super) fn start_import(&mut self, paths: Vec<PathBuf>) -> Result<(), String> {
        if paths.is_empty() {
            return Err("No files to import".into());
        }
        let folder = self
            .gallery
            .as_ref()
            .ok_or("No open folder")?
            .folder()
            .to_path_buf();
        self.start_task(TaskKind::Import { folder, paths })
    }
    pub(super) fn start_task(&mut self, kind: TaskKind) -> Result<(), String> {
        let submitted = Instant::now();
        self.visual_probe = None;
        if self.operation.as_ref().is_some_and(|j| j.running) {
            return Err("A file action is already running".into());
        }
        let gallery = self.gallery.as_ref().ok_or("No open gallery")?;
        let origin = Origin {
            folder: gallery.folder().to_path_buf(),
            video: if matches!(&kind, TaskKind::Rename { .. } | TaskKind::Delete(_))
                && gallery.view() == View::Photo
            {
                self.video
                    .as_ref()
                    .map(|v| v.lock().unwrap().state.clone())
                    .filter(|v| v.path.is_some())
            } else {
                None
            },
            marks: gallery.marked_paths().clone(),
            view: gallery.view(),
            revision: self.interaction_revision,
            delete_order: gallery.photos().iter().map(|p| p.path.clone()).collect(),
            delete_cursor: gallery
                .selected_index()
                .filter(|_| gallery.view() == View::Photo)
                .unwrap_or(gallery.grid_anchor()),
        };
        let action = kind.action();
        let paths = kind.paths();
        self.next_operation += 1;
        let id = self.next_operation;
        // Stop an affected video before the worker can ask Shell to recycle it.
        if action == ActionId::Delete
            && let Some(video) = &self.video
        {
            let mut video = video.lock().unwrap();
            if video.state.path.as_ref().is_some_and(|p| paths.contains(p)) {
                video.sync_path(None);
            }
        }
        // Publish the running source snapshot before sync_video observes the job.
        self.worker
            .tasks
            .try_send(Task {
                id,
                kind,
                submitted,
            })
            .map_err(|e| e.to_string())?;
        self.operation_origin = Some(origin);
        self.operation = Some(OperationState {
            id,
            action,
            running: true,
            files: paths
                .into_iter()
                .map(|source| FileResult {
                    source,
                    destination: None,
                    error: None,
                })
                .collect(),
            warnings: Vec::new(),
            timing: None,
        });
        self.message = "Выполняется…".into();
        Ok(())
    }
    pub(super) fn poll_operations(&mut self) {
        while let Ok(completed) = self.worker.results.try_recv() {
            let mut result = completed.state;
            if let Some(timing) = &mut result.timing {
                timing.ui_received_ms = completed.submitted.elapsed().as_secs_f64() * 1000.0;
            }
            let successes: Vec<_> = result
                .files
                .iter()
                .filter_map(|f| f.destination.clone())
                .collect();
            if result.action != ActionId::Copy
                && let Some(origin) = self.operation_origin.take()
                && self
                    .gallery
                    .as_ref()
                    .is_some_and(|g| same_folder(g.folder(), &origin.folder))
            {
                let apply = self.interaction_revision == origin.revision;
                if apply
                    && result.action == ActionId::Rename
                    && let Some(video) = &origin.video
                    && let Some(source) = &video.path
                {
                    let path = successes.first().cloned().unwrap_or_else(|| source.clone());
                    self.video_restore = Some((
                        path,
                        video.position,
                        video.paused,
                        Instant::now() + Duration::from_secs(8),
                    ));
                }
                let refresh_started = Instant::now();
                if matches!(result.action, ActionId::Rename | ActionId::ConvertImage) {
                    for file in &result.files {
                        if let Some(destination) = &file.destination {
                            self.gallery
                                .as_mut()
                                .unwrap()
                                .rename_path(&file.source, destination);
                        }
                    }
                }
                let _ = self.refresh_after_delete();
                if matches!(
                    result.action,
                    ActionId::ResizeImage | ActionId::ConvertImage
                ) {
                    // Processing changes pixels even if a future encoder keeps a name.
                    let changed: Vec<_> = self
                        .gallery
                        .as_ref()
                        .unwrap()
                        .photos()
                        .iter()
                        .enumerate()
                        .filter(|(_, p)| successes.contains(&p.path))
                        .map(|(i, _)| i)
                        .collect();
                    for i in changed {
                        self.previews.remove(&i);
                        self.thumbnails.remove(&i);
                        self.errors.remove(&i);
                        self.preview_order.retain(|n| *n != i);
                        self.thumbnail_order.retain(|n| *n != i);
                    }
                }
                self.gallery.as_mut().unwrap().include_files(&successes);
                if let Some(timing) = &mut result.timing {
                    timing.refresh_ms = refresh_started.elapsed().as_secs_f64() * 1000.0;
                }
                if apply && result.action == ActionId::Delete {
                    let gallery = self.gallery.as_mut().unwrap();
                    let survivors: std::collections::BTreeSet<_> =
                        gallery.photos().iter().map(|p| p.path.clone()).collect();
                    if let Some(path) = crate::delete_action::successor(
                        &origin.delete_order,
                        origin.delete_cursor,
                        &survivors,
                    ) {
                        gallery.select_path(&path);
                        if origin.view == View::Photo
                            && let Some(index) =
                                gallery.photos().iter().position(|p| p.path == path)
                        {
                            gallery.open_photo(index);
                        }
                    }
                    gallery.set_marked_paths(
                        origin
                            .marks
                            .iter()
                            .filter(|p| survivors.contains(*p))
                            .cloned(),
                    );
                    self.selection_dirty = true;
                    self.queue_mark_sync();
                    self.scroll_to_grid_anchor = true;
                }
                if apply && !successes.is_empty() {
                    let gallery = self.gallery.as_mut().unwrap();
                    gallery.select_path(&successes[0]);
                    if origin.view == View::Photo
                        && let Some(index) =
                            gallery.photos().iter().position(|p| p.path == successes[0])
                    {
                        gallery.open_photo(index);
                    }
                    if result.action == ActionId::Rename {
                        let mut marks = origin.marks;
                        for file in &result.files {
                            if let Some(destination) = &file.destination
                                && marks.remove(&file.source)
                            {
                                marks.insert(destination.clone());
                            }
                        }
                        gallery.set_marked_paths(marks);
                    } else if origin.view == View::Grid {
                        let failed = result
                            .files
                            .iter()
                            .filter(|f| f.error.is_some())
                            .map(|f| f.source.clone());
                        gallery.set_marked_paths(successes.iter().cloned().chain(failed));
                    }
                    self.selection_dirty = true;
                    self.queue_mark_sync();
                    self.scroll_to_grid_anchor = true;
                }
            }
            if matches!(
                result.action,
                ActionId::ResizeImage | ActionId::ConvertImage
            ) {
                if let Some(error) = result.files.iter().find_map(|f| f.error.clone()) {
                    if let Some(dialog) = &mut self.image_dialog {
                        dialog.snapshot.error = Some(error);
                    }
                } else {
                    self.image_dialog = None;
                }
            }
            if result.action == ActionId::Rename {
                if let Some(error) = result.files.iter().find_map(|f| f.error.clone()) {
                    if let Some(dialog) = &mut self.rename_dialog {
                        dialog.error = Some(error);
                    }
                } else {
                    self.rename_dialog = None;
                }
            }
            let failed = result.files.iter().filter(|f| f.error.is_some()).count();
            self.message = if failed > 0 {
                format!(
                    "Готово: {} · ошибок: {failed} · {}",
                    result.files.len() - failed,
                    result
                        .files
                        .iter()
                        .find_map(|f| f.error.as_deref())
                        .unwrap_or("")
                )
            } else if !result.warnings.is_empty() {
                result.warnings.join("; ")
            } else {
                format!(
                    "{}: {}",
                    match result.action {
                        ActionId::Copy => "Скопировано",
                        ActionId::Duplicate => "Дубликатов",
                        ActionId::Paste => "Вставлено",
                        ActionId::Rename => "Переименовано",
                        _ => "Готово",
                    },
                    result.files.len()
                )
            };
            if let Some(timing) = &mut result.timing {
                timing.ui_applied_ms = completed.submitted.elapsed().as_secs_f64() * 1000.0;
            }
            if result.action == ActionId::Delete && result.files.iter().any(|f| f.error.is_none()) {
                self.begin_visual_probe(result.id, completed.submitted);
            }
            self.operation_origin = None;
            self.operation = Some(result);
        }
    }
}

fn scoped_paths(gallery: &Gallery) -> Vec<PathBuf> {
    if gallery.view() == View::Photo {
        return gallery
            .selected_photo()
            .map(|p| vec![p.path.clone()])
            .unwrap_or_default();
    }
    if !gallery.marked_paths().is_empty() {
        return gallery
            .photos()
            .iter()
            .filter(|p| gallery.marked_paths().contains(&p.path))
            .map(|p| p.path.clone())
            .collect();
    }
    gallery
        .cursor_photo()
        .map(|p| vec![p.path.clone()])
        .unwrap_or_default()
}

#[cfg(test)]
mod delete_scope_tests {
    use super::*;
    #[test]
    fn gallery_marks_and_space_current_are_different_scopes() {
        let folder = std::env::temp_dir().join(format!(
            "photo-delete-scope-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&folder).unwrap();
        let _paths: Vec<_> = ["a.jpg", "b.mp4", "c.jpg"]
            .iter()
            .map(|n| {
                let p = folder.join(n);
                std::fs::write(&p, b"synthetic").unwrap();
                p
            })
            .collect();
        let mut gallery = Gallery::open(&folder).unwrap();
        let paths: Vec<_> = gallery.photos().iter().map(|p| p.path.clone()).collect();
        gallery.set_marked_paths(paths.clone());
        assert_eq!(scoped_paths(&gallery).len(), 3);
        let captured = scoped_paths(&gallery);
        gallery.set_marked_paths([paths[2].clone()]);
        assert_eq!(captured, paths);
        assert_eq!(scoped_paths(&gallery), vec![paths[2].clone()]);
        gallery.open_photo(1);
        assert_eq!(scoped_paths(&gallery), vec![paths[1].clone()]);
        gallery.set_marked_paths(paths.clone());
        assert_eq!(scoped_paths(&gallery), vec![paths[1].clone()]);
        drop(gallery);
        for path in paths {
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(folder).unwrap();
    }
}
