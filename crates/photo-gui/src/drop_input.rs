//! Drop is an opening gesture: never copy or move files implicitly.
use super::*;

fn prepare(paths: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
    if paths.is_empty() {
        return Err("Drop one folder or local media files".into());
    }
    let paths = paths
        .into_iter()
        .map(|p| p.canonicalize().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    if paths.len() == 1 && paths[0].is_dir() {
        return Ok(paths);
    }
    for path in &paths {
        if !path.is_file() {
            return Err("Drop one folder, or files from a single folder".into());
        }
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !photo_core::is_video_path(path)
            && !matches!(
                ext.as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "webp"
                    | "gif"
                    | "bmp"
                    | "tif"
                    | "tiff"
                    | "avif"
                    | "heic"
                    | "heif"
            )
        {
            return Err(format!("Unsupported media file: {}", path.display()));
        }
    }
    if paths.iter().any(|p| p.parent() != paths[0].parent()) {
        return Err("Drop files from one folder at a time".into());
    }
    Ok(paths)
}
impl Workshop {
    pub(super) fn open_dropped(&mut self, paths: Vec<PathBuf>) -> Result<(), String> {
        let paths = prepare(paths)?;
        if paths.len() == 1 {
            return self
                .execute(Command::Open {
                    path: paths[0].clone(),
                })
                .error
                .map_or(Ok(()), |e| Err(e.message));
        }
        let folder = paths[0]
            .parent()
            .ok_or("Missing parent folder")?
            .to_path_buf();
        if let Some(error) = self.execute(Command::Open { path: folder }).error {
            return Err(error.message);
        }
        self.execute(Command::SetMarked {
            paths: paths.clone(),
        });
        self.execute(Command::SelectPath {
            path: paths[0].clone(),
        });
        Ok(())
    }
    pub(super) fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        if self.image_dialog.is_some()
            || self.search.snapshot.open
            || self.palette.open
            || self.rename_dialog.is_some()
            || self.delete_dialog.is_some()
        {
            return;
        }
        if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            egui::Area::new(egui::Id::new("drop-open-hint"))
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 70.0))
                .order(egui::Order::Foreground)
                .interactable(false)
                .show(ctx, |ui| {
                    egui::Frame::popup(&ctx.style_of(egui::Theme::Dark)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.add(Lucide::FolderOpen.size(18.0).image());
                            ui.label("Открыть в Afio");
                        });
                    });
                });
        }
        let files = ctx.input(|i| i.raw.dropped_files.clone());
        if files.is_empty() {
            return;
        }
        let paths = files.into_iter().map(|f| f.path().to_path_buf()).collect();
        if let Some(error) = self.execute(Command::OpenDropped { paths }).error {
            self.message = error.message;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn drop_validation_opens_only_local_media_and_preserves_files() {
        let root = std::env::temp_dir().join(format!(
            "afio-drop-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        for name in ["one.png", "two.mp4", "no.txt"] {
            std::fs::write(root.join(name), b"original").unwrap();
        }
        assert!(prepare(vec![root.clone()]).is_ok());
        assert!(prepare(vec![root.join("one.png"), root.join("two.mp4")]).is_ok());
        assert!(prepare(vec![root.join("no.txt")]).is_err());
        assert!(prepare(vec![root.clone(), root.join("one.png")]).is_err());
        assert!(prepare(Vec::new()).is_err());
        assert_eq!(std::fs::read(root.join("one.png")).unwrap(), b"original");
        std::fs::remove_dir_all(root).unwrap();
    }
}
