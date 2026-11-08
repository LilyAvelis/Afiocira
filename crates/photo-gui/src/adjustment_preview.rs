//! One active preview plus one replaceable pending request, separate from saves.
use eframe::egui::{self, ColorImage};
use photo_core::{adjustments::AdjustmentOptions, rename::SourceVersion};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
};

pub(super) struct PreviewResult {
    pub revision: u64,
    pub path: PathBuf,
    pub source_version: SourceVersion,
    pub image: Result<ColorImage, String>,
}
struct Request {
    serial: u64,
    epoch: u64,
    revision: u64,
    path: PathBuf,
    version: SourceVersion,
    options: AdjustmentOptions,
}
#[derive(Default)]
struct State {
    serial: u64,
    epoch: u64,
    pending: Option<Request>,
    result: Option<PreviewResult>,
    closed: bool,
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

pub(super) struct PreviewWorker {
    shared: Arc<Shared>,
}
impl PreviewWorker {
    pub fn new(ctx: egui::Context) -> Self {
        let shared = Arc::new(Shared::default());
        let worker = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("adjustment-preview".into())
            .spawn(move || run(worker, ctx))
            .expect("Could not start adjustment preview worker");
        Self { shared }
    }
    pub fn request(
        &self,
        path: PathBuf,
        version: SourceVersion,
        options: AdjustmentOptions,
        revision: u64,
    ) {
        let mut state = self.shared.state.lock().unwrap();
        if state.closed {
            return;
        }
        state.serial = state.serial.wrapping_add(1);
        state.result = None;
        state.pending = Some(Request {
            serial: state.serial,
            epoch: state.epoch,
            revision,
            path,
            version,
            options,
        });
        self.shared.wake.notify_one();
    }
    pub fn cancel(&self) {
        let mut state = self.shared.state.lock().unwrap();
        state.serial = state.serial.wrapping_add(1);
        state.epoch = state.epoch.wrapping_add(1);
        state.pending = None;
        state.result = None;
        self.shared.wake.notify_one();
    }
    pub fn poll(&self) -> Option<PreviewResult> {
        self.shared.state.lock().unwrap().result.take()
    }
}
impl Drop for PreviewWorker {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.closed = true;
        state.pending = None;
        state.result = None;
        self.shared.wake.notify_one();
        // Do not join a decoder on the UI thread. It owns no user file handles
        // after decoding and exits as soon as its bounded active task completes.
    }
}
struct Cached {
    epoch: u64,
    path: PathBuf,
    version: SourceVersion,
    base: Result<crate::adjust_image::PreviewBase, String>,
}
fn run(shared: Arc<Shared>, ctx: egui::Context) {
    let mut cached: Option<Cached> = None;
    loop {
        let request = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if state.closed {
                    return;
                }
                if cached.as_ref().is_some_and(|c| c.epoch != state.epoch) {
                    cached = None;
                }
                if let Some(request) = state.pending.take() {
                    break request;
                }
                state = shared.wake.wait(state).unwrap();
            }
        };
        let image = (|| {
            request.options.validate()?;
            if photo_core::rename::capture(&request.path).map_err(|e| e.to_string())?
                != request.version
            {
                cached = None;
                return Err("Source changed since the adjustment panel opened".into());
            }
            if !cached.as_ref().is_some_and(|c| {
                c.epoch == request.epoch && c.path == request.path && c.version == request.version
            }) {
                cached = Some(Cached {
                    epoch: request.epoch,
                    path: request.path.clone(),
                    version: request.version.clone(),
                    base: crate::adjust_image::prepare_preview(&request.path, &request.version),
                });
            }
            let base = cached
                .as_ref()
                .unwrap()
                .base
                .as_ref()
                .map_err(Clone::clone)?;
            let image = crate::adjust_image::apply_preview(base, &request.options)?;
            if photo_core::rename::capture(&request.path).map_err(|e| e.to_string())?
                != request.version
            {
                cached = None;
                return Err("Source changed while preparing adjustment preview".into());
            }
            Ok(image)
        })();
        let mut state = shared.state.lock().unwrap();
        if state.closed {
            return;
        }
        if state.serial == request.serial && state.epoch == request.epoch {
            state.result = Some(PreviewResult {
                revision: request.revision,
                path: request.path,
                source_version: request.version,
                image,
            });
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latest_request_wins_and_cancel_discards_ready_output_without_writes() {
        let dir = std::env::temp_dir().join(format!(
            "afio-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("synthetic.png");
        image::RgbaImage::from_pixel(300, 200, image::Rgba([128, 64, 32, 71]))
            .save(&path)
            .unwrap();
        let original = std::fs::read(&path).unwrap();
        let version = photo_core::rename::capture(&path).unwrap();
        let worker = PreviewWorker::new(egui::Context::default());
        for revision in 0..20 {
            worker.request(
                path.clone(),
                version.clone(),
                AdjustmentOptions {
                    exposure: if revision == 19 { 1.0 } else { 0.0 },
                    ..Default::default()
                },
                revision,
            );
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let result = loop {
            if let Some(result) = worker.poll() {
                break result;
            }
            assert!(std::time::Instant::now() < deadline, "preview timed out");
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        assert_eq!(result.revision, 19);
        assert_eq!(result.path, path);
        assert_eq!(result.source_version, version);
        let actual = result.image.unwrap().pixels[0];
        assert_eq!(
            actual,
            egui::Color32::from_rgba_unmultiplied(176, 90, 47, 71)
        );
        worker.request(
            path.clone(),
            version.clone(),
            AdjustmentOptions::default(),
            20,
        );
        worker.cancel();
        std::thread::sleep(std::time::Duration::from_millis(80));
        assert!(worker.poll().is_none());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        drop(worker);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
