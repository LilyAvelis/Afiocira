use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Condvar, Mutex};

use eframe::egui::{self, ColorImage};
use image::{DynamicImage, ImageDecoder};

const QUEUE_LIMIT: usize = 24;

pub struct DecodeTask {
    pub generation: u64,
    pub index: usize,
    pub path: PathBuf,
    pub preview: bool,
}

pub struct DecodeResult {
    pub generation: u64,
    pub index: usize,
    pub preview: bool,
    pub image: Result<ColorImage, String>,
}

#[derive(Default)]
struct State {
    tasks: VecDeque<DecodeTask>,
    closed: bool,
}

pub struct DecodeQueue {
    state: Mutex<State>,
    wake: Condvar,
    generation: AtomicU64,
}

impl DecodeQueue {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            wake: Condvar::new(),
            generation: AtomicU64::new(0),
        })
    }

    pub fn set_generation(&self, generation: u64) {
        self.generation.store(generation, Ordering::Release);
        let mut state = self.state.lock().unwrap();
        state.tasks.clear();
    }

    pub fn submit(&self, task: DecodeTask, urgent: bool) -> Result<Option<usize>, ()> {
        let mut state = self.state.lock().unwrap();
        if state.closed || task.generation != self.generation.load(Ordering::Acquire) {
            return Err(());
        }
        let evicted = if state.tasks.len() >= QUEUE_LIMIT {
            if urgent || task.preview {
                state.tasks.pop_back()
            } else {
                state.tasks.pop_front()
            }
            .map(|old| old.index)
        } else {
            None
        };
        if urgent {
            state.tasks.push_front(task);
        } else {
            state.tasks.push_back(task);
        }
        self.wake.notify_one();
        Ok(evicted)
    }

    pub fn promote(&self, index: usize) {
        let mut state = self.state.lock().unwrap();
        if let Some(position) = state.tasks.iter().position(|task| task.index == index)
            && let Some(task) = state.tasks.remove(position)
        {
            state.tasks.push_front(task);
        }
    }

    fn next(&self) -> Option<DecodeTask> {
        let mut state = self.state.lock().unwrap();
        loop {
            if state.closed {
                return None;
            }
            if let Some(task) = state.tasks.pop_front() {
                return Some(task);
            }
            state = self.wake.wait(state).unwrap();
        }
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.tasks.clear();
        self.wake.notify_all();
    }
}

pub fn spawn_worker(
    queue: Arc<DecodeQueue>,
    results: SyncSender<DecodeResult>,
    repaint: egui::Context,
) {
    std::thread::spawn(move || {
        while let Some(task) = queue.next() {
            if task.generation != queue.generation.load(Ordering::Acquire) {
                continue;
            }
            let image = decode(&task.path, if task.preview { 2560 } else { 320 });
            if task.generation != queue.generation.load(Ordering::Acquire) {
                continue;
            }
            if results
                .send(DecodeResult {
                    generation: task.generation,
                    index: task.index,
                    preview: task.preview,
                    image,
                })
                .is_err()
            {
                break;
            }
            repaint.request_repaint();
        }
    });
}

fn decode(path: &Path, max_side: u32) -> Result<ColorImage, String> {
    if photo_core::is_video_path(path) {
        return video_thumbnail(path, max_side);
    }
    let mut decoder = image::ImageReader::open(path)
        .map_err(|error| format!("Cannot read {}: {error}", path.display()))?
        .into_decoder()
        .map_err(|error| format!("Cannot decode {}: {error}", path.display()))?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    decoder
        .set_limits(limits)
        .map_err(|error| format!("Image exceeds decode limit {}: {error}", path.display()))?;
    let orientation = decoder
        .orientation()
        .map_err(|error| format!("Cannot read orientation of {}: {error}", path.display()))?;
    let mut image = DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("Cannot decode {}: {error}", path.display()))?;
    image.apply_orientation(orientation);
    let image = image.thumbnail(max_side, max_side).to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Ok(ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
}

fn video_thumbnail(path: &Path, max_side: u32) -> Result<ColorImage, String> {
    #[cfg(windows)]
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let sibling = std::env::current_exe()
        .ok()
        .map(|p| p.with_file_name("ffmpeg.exe"));
    let executable = std::env::var_os("AFIO_FFMPEG")
        .map(std::path::PathBuf::from)
        .or_else(|| sibling.filter(|p| p.is_file()))
        .unwrap_or_else(|| "ffmpeg".into());
    let output = std::env::temp_dir().join(format!(
        "afio-video-{}-{}.png",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _temporary = Temporary(output.clone());
    let mut command = Command::new(executable);
    command
        .args(["-nostdin", "-v", "error", "-y", "-threads", "1", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-frames:v", "1", "-vf"])
        .arg(format!(
            "scale={max_side}:{max_side}:force_original_aspect_ratio=decrease"
        ))
        .args(["-threads", "1", "-update", "1"])
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Video thumbnail requires FFmpeg: {e}"))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err("Cannot decode video thumbnail".into());
                }
                break;
            }
            Ok(None) if started.elapsed() < Duration::from_secs(8) => {
                std::thread::sleep(Duration::from_millis(20))
            }
            result => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Video thumbnail timed out or failed: {result:?}"));
            }
        }
    }
    decode(&output, max_side)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_generation_replaces_old_queue_and_current_photo_takes_priority() {
        let queue = DecodeQueue::new();
        queue
            .submit(
                DecodeTask {
                    generation: 0,
                    index: 1,
                    path: "old".into(),
                    preview: true,
                },
                false,
            )
            .unwrap();
        queue.set_generation(1);
        queue
            .submit(
                DecodeTask {
                    generation: 1,
                    index: 2,
                    path: "neighbor".into(),
                    preview: true,
                },
                false,
            )
            .unwrap();
        queue
            .submit(
                DecodeTask {
                    generation: 1,
                    index: 3,
                    path: "current".into(),
                    preview: true,
                },
                true,
            )
            .unwrap();
        assert_eq!(queue.next().unwrap().index, 3);
        assert_eq!(queue.next().unwrap().index, 2);
        queue.close();
        assert!(queue.next().is_none());
    }

    #[test]
    fn pending_work_is_bounded() {
        let queue = DecodeQueue::new();
        for index in 0..100 {
            queue
                .submit(
                    DecodeTask {
                        generation: 0,
                        index,
                        path: "image".into(),
                        preview: false,
                    },
                    false,
                )
                .unwrap();
        }
        assert_eq!(queue.state.lock().unwrap().tasks.len(), QUEUE_LIMIT);
        assert_eq!(queue.next().unwrap().index, 76);
    }
}
