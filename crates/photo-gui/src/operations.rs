//! One bounded background worker; requests capture paths and destination folders.
use super::*;
use photo_core::actions::ActionId;
use photo_core::api::{FileResult, OperationState};
use std::sync::mpsc::SyncSender;

fn transform_events() -> &'static std::sync::Mutex<std::collections::HashMap<PathBuf, Instant>> {
    static EVENTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<PathBuf, Instant>>,
    > = std::sync::OnceLock::new();
    EVENTS.get_or_init(Default::default)
}
pub(super) fn own_transform_membership(path: &Path) -> bool {
    let mut events = transform_events().lock().unwrap();
    events.retain(|_, deadline| *deadline > Instant::now());
    events.contains_key(path)
}
fn track_transform(source: &Path, duration: Duration) {
    transform_events()
        .lock()
        .unwrap()
        .insert(source.to_path_buf(), Instant::now() + duration);
}

pub(super) struct Worker {
    tasks: SyncSender<Task>,
    results: Receiver<Completed>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let (closed, _) = mpsc::sync_channel(0);
        drop(std::mem::replace(&mut self.tasks, closed));
        if let Some(thread) = self.thread.take() {
            // Finish an admitted file operation and let the session store clean
            // its own snapshots before normal process exit. Drain receipts so
            // the bounded result channel cannot block worker shutdown.
            while !thread.is_finished() {
                while self.results.try_recv().is_ok() {}
                std::thread::sleep(Duration::from_millis(5));
            }
            let _ = thread.join();
        }
    }
}
struct Completed {
    history: Option<photo_core::history::HistorySnapshot>,
    history_preview: Option<(usize, egui::ColorImage)>,
    history_preview_request: Option<(usize, HistoryPreviewRequest)>,
    state: OperationState,
    submitted: Instant,
    session_id: String,
    saved_preview: Option<crate::resize_image::SavedPreview>,
}
#[derive(Clone, Copy)]
pub(super) struct HistoryPreviewRequest {
    pub epoch: u64,
    pub selection: u64,
    pub canvas: bool,
}
struct Task {
    submitted: Instant,
    id: u64,
    kind: TaskKind,
    session_id: String,
}
pub(super) enum TaskKind {
    Adjust {
        source: PathBuf,
        version: photo_core::rename::SourceVersion,
        options: photo_core::adjustments::AdjustmentOptions,
    },
    History {
        source: PathBuf,
        op: photo_core::history::Operation,
        preview: Option<HistoryPreviewRequest>,
    },
    VideoFrame {
        source: PathBuf,
        version: photo_core::rename::SourceVersion,
        action: ActionId,
        capture: crate::video::FrameCapture,
    },
    Transform {
        source: PathBuf,
        version: photo_core::rename::SourceVersion,
        action: ActionId,
    },
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
            Self::Adjust { .. } => ActionId::AdjustImage,
            Self::History { op, .. } => match op {
                photo_core::history::Operation::Undo => ActionId::Undo,
                photo_core::history::Operation::Redo => ActionId::Redo,
                photo_core::history::Operation::Commit => ActionId::CommitHistory,
                _ => ActionId::History,
            },
            Self::VideoFrame { action, .. } => *action,
            Self::Transform { action, .. } => *action,
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
            Self::Adjust { source, .. } | Self::History { source, .. } => vec![source.clone()],
            Self::VideoFrame { source, .. } => vec![source.clone()],
            Self::Transform { source, .. } => vec![source.clone()],
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
    collection_id: u64,
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
fn history_snapshot(
    store: Option<&photo_history::HistoryStore>,
    path: &Path,
) -> photo_core::history::HistorySnapshot {
    let view = store.and_then(|s| s.inspect(path));
    photo_core::history::HistorySnapshot {
        path: path.to_owned(),
        versions: view
            .as_ref()
            .map(|v| {
                v.versions
                    .iter()
                    .map(|n| photo_core::history::HistoryNode {
                        path: n.source_path.clone(),
                        bytes: n.bytes,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        cursor: view.as_ref().map(|v| v.selected),
        can_undo: view.as_ref().is_some_and(|v| v.can_undo),
        can_redo: view.as_ref().is_some_and(|v| v.can_redo),
    }
}
fn prepare_history(
    history: &mut Option<photo_history::HistoryStore>,
    path: &Path,
    version: &photo_core::rename::SourceVersion,
) -> Result<photo_history::Ticket, String> {
    if history.is_none() {
        let root = photo_registry::default_data_home()
            .map_err(|e| e.message)?
            .join("history-sessions");
        *history = Some(
            photo_history::HistoryStore::new(&root, 1024 * 1024 * 1024)
                .map_err(|e| e.to_string())?,
        );
    }
    history
        .as_mut()
        .unwrap()
        .prepare_before(path, version)
        .map_err(|e| e.to_string())
}
fn rebind_metadata(
    source: &Path,
    before: &photo_core::rename::SourceVersion,
    destination: &Path,
    after: &photo_core::rename::SourceVersion,
    warnings: &mut Vec<String>,
) {
    let result = photo_registry::default_data_home().and_then(|home| {
        photo_registry::Registry::new(home).rebind_known(source, before, destination, after)
    });
    if let Err(e) = result {
        warnings.push(format!(
            "Файл сохранён, но привязка метаданных не обновлена: {}",
            e.message
        ));
    }
}
fn finalize_change(
    history: &mut Option<photo_history::HistoryStore>,
    ticket: Option<photo_history::Ticket>,
    source: &Path,
    before: &photo_core::rename::SourceVersion,
    destination: &Path,
    warnings: &mut Vec<String>,
) -> Option<photo_core::history::HistorySnapshot> {
    let after = match photo_core::rename::capture(destination) {
        Ok(v) => v,
        Err(e) => {
            warnings.push(format!(
                "Файл сохранён; невозможно проверить результат: {e}"
            ));
            return None;
        }
    };
    if let Some(store) = history.as_mut() {
        let result = if let Some(ticket) = ticket {
            store.commit_after(ticket, destination, &after)
        } else {
            store.amend_current(source, before, destination, &after)
        };
        if let Err(e) = result {
            warnings.push(format!("Файл сохранён; история: {e}"));
        }
    }
    rebind_metadata(source, before, destination, &after, warnings);
    history
        .as_ref()
        .and_then(|store| store.inspect(destination))
        .map(|_| history_snapshot(history.as_ref(), destination))
}
impl Worker {
    pub fn start(ctx: egui::Context) -> Self {
        let (tasks, receiver) = mpsc::sync_channel::<Task>(1);
        let (sender, results) = mpsc::sync_channel(1);
        let thread = std::thread::spawn(move || {
            let mut history: Option<photo_history::HistoryStore> = None;
            while let Ok(task) = receiver.recv() {
                let history_preview_request = match &task.kind {
                    TaskKind::History {
                        op: photo_core::history::Operation::Preview(index),
                        preview: Some(request),
                        ..
                    } => Some((*index, *request)),
                    _ => None,
                };
                let work_started = Instant::now();
                let mut saved_preview = None;
                let mut history_view = None;
                let mut history_preview = None;
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
                    TaskKind::History {
                        source,
                        op,
                        preview,
                    } => {
                        use photo_core::history::Operation;
                        let result = (|| -> Result<Option<PathBuf>, String> {
                            let Some(store) = &mut history else {
                                if matches!(op, Operation::Inspect) {
                                    return Ok(None);
                                }
                                return Err("У изображения нет истории текущей сессии".into());
                            };
                            match op {
                                Operation::Inspect => Ok(None),
                                Operation::Preview(index) => {
                                    let path =
                                        store.preview(&source, index).ok_or("Версия не найдена")?;
                                    history_preview = Some((
                                        index,
                                        crate::decode::decode(
                                            &path,
                                            if preview.is_some_and(|p| p.canvas) {
                                                1600
                                            } else {
                                                256
                                            },
                                        )?,
                                    ));
                                    Ok(None)
                                }
                                Operation::Commit => {
                                    store.commit_current(&source).map_err(|e| e.to_string())?;
                                    Ok(None)
                                }
                                Operation::Undo | Operation::Redo => {
                                    let before = photo_core::rename::capture(&source)
                                        .map_err(|e| e.to_string())?;
                                    let restored = if matches!(op, Operation::Undo) {
                                        store.undo(&source, &before)
                                    } else {
                                        store.redo(&source, &before)
                                    }
                                    .map_err(|e| e.to_string())?;
                                    rebind_metadata(
                                        &source,
                                        &before,
                                        &restored.path,
                                        &restored.version,
                                        &mut state.warnings,
                                    );
                                    Ok(Some(restored.path))
                                }
                            }
                        })();
                        let path = result
                            .as_ref()
                            .ok()
                            .and_then(|p| p.as_ref())
                            .unwrap_or(&source);
                        history_view = Some(history_snapshot(history.as_ref(), path));
                        let (destination, error) = match result {
                            Ok(path) => (path, None),
                            Err(e) => (None, Some(e)),
                        };
                        state.files.push(FileResult {
                            source,
                            destination,
                            error,
                        });
                    }
                    TaskKind::Adjust {
                        source,
                        version,
                        options,
                    } => {
                        let result = (|| -> Result<crate::resize_image::SavedPreview, String> {
                            let ticket = prepare_history(&mut history, &source, &version)?;
                            let output = crate::adjust_image::process(&source, &version, &options)?;
                            history_view = finalize_change(
                                &mut history,
                                Some(ticket),
                                &source,
                                &version,
                                &output.published.path,
                                &mut state.warnings,
                            );
                            Ok(output)
                        })();
                        let (destination, error) = match result {
                            Ok(output) => {
                                let path = output.published.path.clone();
                                saved_preview = Some(output);
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
                    TaskKind::VideoFrame {
                        source,
                        version,
                        action,
                        capture,
                    } => {
                        let result = (|| -> Result<Option<PathBuf>, String> {
                            capture.wait()?;
                            if photo_core::rename::capture(&source).map_err(|e| e.to_string())?
                                != version
                            {
                                return Err("Source video changed during capture".into());
                            }
                            if action == ActionId::CopyVideoFrame {
                                crate::clipboard::copy_pixels(&capture.path())?;
                                Ok(None)
                            } else {
                                let bytes =
                                    std::fs::read(capture.path()).map_err(|e| e.to_string())?;
                                let stem = source.file_stem().unwrap_or_default().to_string_lossy();
                                let name = format!("{stem} - frame {:.3}s.png", capture.position);
                                photo_core::import::write_bytes(
                                    source.parent().ok_or("No source folder")?,
                                    &name,
                                    &bytes,
                                )
                                .map(Some)
                                .map_err(|e| e.to_string())
                            }
                        })();
                        let (destination, error) = match result {
                            Ok(path) => (path, None),
                            Err(e) => (None, Some(e)),
                        };
                        state.files.push(FileResult {
                            source,
                            destination,
                            error,
                        });
                    }
                    TaskKind::Transform {
                        source,
                        version,
                        action,
                    } => {
                        let result =
                            crate::resize_image::transform_in_place(&source, &version, action);
                        let (destination, error) = match result {
                            Ok(output) => {
                                history_view = finalize_change(
                                    &mut history,
                                    None,
                                    &source,
                                    &version,
                                    &output.path,
                                    &mut state.warnings,
                                );
                                state.warnings.push(output.metadata_notice.into());
                                saved_preview = output.saved_preview;
                                (Some(output.path), None)
                            }
                            Err(error) => (None, Some(error)),
                        };
                        state.files.push(FileResult {
                            source,
                            destination,
                            error,
                        });
                    }
                    TaskKind::Image {
                        source,
                        version,
                        options,
                        convert,
                    } => {
                        let result = (|| {
                            let ticket = if convert {
                                Some(prepare_history(&mut history, &source, &version)?)
                            } else {
                                None
                            };
                            let result = crate::image_actions::process(
                                &source, &version, &options, convert,
                            )?;
                            if convert {
                                history_view = finalize_change(
                                    &mut history,
                                    ticket,
                                    &source,
                                    &version,
                                    &result.0,
                                    &mut state.warnings,
                                );
                            }
                            Ok::<_, String>(result)
                        })();
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
                        if let Ok(destination) = &result {
                            history_view = finalize_change(
                                &mut history,
                                None,
                                &source,
                                &version,
                                destination,
                                &mut state.warnings,
                            );
                        }
                        state.files.push(outcome(source, result));
                    }
                }
                if let Some(timing) = &mut state.timing {
                    timing.work_ms = work_started.elapsed().as_secs_f64() * 1000.0;
                }
                if sender
                    .send(Completed {
                        history: history_view,
                        history_preview,
                        history_preview_request,
                        state,
                        submitted: task.submitted,
                        session_id: task.session_id,
                        saved_preview,
                    })
                    .is_err()
                {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self {
            tasks,
            results,
            thread: Some(thread),
        }
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
        let edits_source = matches!(
            &kind,
            TaskKind::Transform { .. }
                | TaskKind::Adjust { .. }
                | TaskKind::Rename { .. }
                | TaskKind::Image { convert: true, .. }
                | TaskKind::History {
                    op: photo_core::history::Operation::Undo
                        | photo_core::history::Operation::Redo
                        | photo_core::history::Operation::Commit,
                    ..
                }
        );
        if edits_source && (self.metadata.busy() || self.metadata_panel.is_dirty()) {
            return Err("Сначала завершите сохранение метаданных".into());
        }
        let submitted = Instant::now();
        self.visual_probe = None;
        self.retire_transform_animation();
        if self.operation.as_ref().is_some_and(|j| j.running)
            || (matches!(&kind, TaskKind::Transform { .. }) && self.transform_animation.is_some())
        {
            return Err("A file action is already running".into());
        }
        let gallery = self.gallery.as_ref().ok_or("No open gallery")?;
        let origin = Origin {
            folder: gallery.folder().to_path_buf(),
            collection_id: gallery.collection_id(),
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
        if matches!(
            action,
            ActionId::RotateLeft
                | ActionId::RotateRight
                | ActionId::FlipHorizontal
                | ActionId::FlipVertical
        ) {
            for path in &paths {
                track_transform(path, Duration::from_secs(60));
            }
        }
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
                session_id: self.session_id.clone(),
            })
            .map_err(|e| e.to_string())?;
        self.start_transform_animation(id, action, &paths, submitted);
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
    pub(super) fn poll_operations(&mut self, ctx: &egui::Context) {
        while let Ok(completed) = self.worker.results.try_recv() {
            if completed.session_id != self.session_id
                || self
                    .operation
                    .as_ref()
                    .is_none_or(|operation| operation.id != completed.state.id)
            {
                continue;
            }
            let mut result = completed.state;
            if self
                .adjustment_commit_preview
                .as_ref()
                .is_some_and(|(id, _, _)| *id == result.id)
            {
                self.adjustment_commit_preview = None;
            }
            if result.files.iter().any(|f| f.destination.is_some())
                && matches!(
                    result.action,
                    ActionId::AdjustImage
                        | ActionId::ConvertImage
                        | ActionId::RotateLeft
                        | ActionId::RotateRight
                        | ActionId::FlipHorizontal
                        | ActionId::FlipVertical
                        | ActionId::Rename
                        | ActionId::Undo
                        | ActionId::Redo
                )
            {
                self.history_panel.reset_thumbnails();
            }
            let history_target = self.history_panel.target.clone();
            if let Some(snapshot) = completed.history {
                // Never let a late completion open a panel on another image.
                let current = self
                    .gallery
                    .as_ref()
                    .and_then(Gallery::selected_photo)
                    .map(|p| p.path.clone());
                let matching = current.as_ref() == Some(&snapshot.path)
                    || result
                        .files
                        .iter()
                        .any(|f| Some(&f.source) == current.as_ref());
                let was_open = self.history_panel.open;
                if !matching {
                    self.history_panel.open = false;
                }
                self.history_panel.accept(snapshot);
                if !matching {
                    self.history_panel.open = was_open;
                    self.history_panel.target = history_target;
                }
            }
            if let Some((index, pixels)) = completed.history_preview {
                if self.history_panel.open
                    && self
                        .history_panel
                        .target
                        .as_ref()
                        .is_some_and(|p| result.files.iter().any(|f| &f.source == p))
                    && let Some((_, request)) = completed.history_preview_request
                    && self.history_panel.accepts_request(request)
                {
                    let texture = ctx.load_texture(
                        format!("history-preview-{}", result.id),
                        pixels,
                        TextureOptions::LINEAR,
                    );
                    if request.canvas {
                        self.history_panel.accept_canvas(index, request, texture);
                    } else {
                        self.history_panel.accept_preview(index, texture);
                    }
                }
            } else if matches!(
                result.action,
                ActionId::Undo | ActionId::Redo | ActionId::CommitHistory
            ) {
                self.history_panel.preview = None;
            }
            if matches!(result.action, ActionId::History | ActionId::CommitHistory) {
                if let Some((index, request)) = completed.history_preview_request
                    && let Some(error) = result.files.iter().find_map(|f| f.error.clone())
                    && self.history_panel.accepts_request(request)
                {
                    if request.canvas {
                        self.history_panel.fail_canvas(index, request, error);
                    } else {
                        self.history_panel.fail_preview(index, error);
                    }
                }
                let relevant = completed
                    .history_preview_request
                    .is_none_or(|(index, request)| {
                        self.history_panel.accepts_request(request)
                            && (!request.canvas
                                || self.history_panel.current_canvas_request(index, request))
                    });
                if relevant {
                    self.history_panel.error = result.files.iter().find_map(|f| f.error.clone());
                    self.message = self
                        .history_panel
                        .error
                        .clone()
                        .unwrap_or_else(|| "История текущей сессии".into());
                }
                self.operation = Some(result);
                self.operation_origin = None;
                continue;
            }
            if matches!(result.action, ActionId::Undo | ActionId::Redo) {
                self.history_panel.error = result.files.iter().find_map(|f| f.error.clone());
            }
            if let Some(animation) = &mut self.transform_animation {
                animation.complete(
                    result.id,
                    result.files.iter().all(|file| file.error.is_none()),
                    Instant::now(),
                );
            }
            if let Some(timing) = &mut result.timing {
                timing.ui_received_ms = completed.submitted.elapsed().as_secs_f64() * 1000.0;
            }
            let successes: Vec<_> = result
                .files
                .iter()
                .filter_map(|f| f.destination.clone())
                .collect();
            if !matches!(result.action, ActionId::Copy | ActionId::CopyVideoFrame)
                && let Some(origin) = self.operation_origin.take()
                && self.gallery.as_ref().is_some_and(|g| {
                    g.collection_id() == origin.collection_id
                        && same_folder(g.folder(), &origin.folder)
                })
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
                if matches!(
                    result.action,
                    ActionId::Rename | ActionId::ConvertImage | ActionId::Undo | ActionId::Redo
                ) {
                    for file in &result.files {
                        if let Some(destination) = &file.destination {
                            self.gallery
                                .as_mut()
                                .unwrap()
                                .rename_path(&file.source, destination);
                        }
                    }
                }
                let transformed = matches!(
                    result.action,
                    ActionId::RotateLeft
                        | ActionId::RotateRight
                        | ActionId::FlipHorizontal
                        | ActionId::FlipVertical
                        | ActionId::AdjustImage
                );
                if !transformed {
                    let _ = self.refresh_after_delete();
                } else {
                    for file in &result.files {
                        track_transform(&file.source, Duration::from_secs(1));
                    }
                    // Cancel stale in-flight decodes, retaining every unaffected texture.
                    self.generation += 1;
                    self.preview_tasks.set_generation(self.generation);
                    self.thumbnail_tasks.set_generation(self.generation);
                    self.pending_previews.clear();
                    self.pending_thumbnails.clear();
                }
                if matches!(
                    result.action,
                    ActionId::ResizeImage
                        | ActionId::AdjustImage
                        | ActionId::Undo
                        | ActionId::Redo
                        | ActionId::ConvertImage
                        | ActionId::RotateLeft
                        | ActionId::RotateRight
                        | ActionId::FlipHorizontal
                        | ActionId::FlipVertical
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
                        self.invalidate_image(i);
                    }
                }
                if let Some(saved) = completed.saved_preview
                    && let Some(index) = self
                        .gallery
                        .as_ref()
                        .unwrap()
                        .photos()
                        .iter()
                        .position(|photo| photo.path == saved.published.path)
                {
                    // A completion proves which bytes were saved, not that the
                    // path still names them. External edits/replacements must
                    // take the ordinary decode path instead of installing stale pixels.
                    if saved.published.is_current() {
                        let texture = ctx.load_texture(
                            format!("saved-{}-{}-{}", result.id, self.generation, index),
                            saved.image,
                            TextureOptions::LINEAR,
                        );
                        self.insert_ready_preview(index, texture);
                        if let Some(timing) = &mut result.timing {
                            timing.preview_ready_ms =
                                Some(completed.submitted.elapsed().as_secs_f64() * 1000.0);
                        }
                    } else {
                        self.request_preview(index);
                    }
                }
                if !transformed {
                    self.gallery.as_mut().unwrap().include_files(&successes);
                }
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
                if apply && !successes.is_empty() && !transformed {
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
