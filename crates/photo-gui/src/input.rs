use super::*;

impl Workshop {
    pub(super) fn key_input(&mut self, ctx: &egui::Context, columns: usize) {
        if ctx.input(|input| input.key_pressed(Key::F5)) {
            self.execute(Command::Refresh);
            return;
        }
        let Some(gallery) = self.gallery.as_ref() else {
            return;
        };
        let pressed = |key| ctx.input(|input| input.key_pressed(key));
        match gallery.view() {
            View::Grid if self.album_mode() => {
                if pressed(Key::Escape) {
                    self.execute(Command::SetMarked { paths: Vec::new() });
                    return;
                }
                let (horizontal, vertical) = if pressed(Key::ArrowRight) {
                    (1, 0)
                } else if pressed(Key::ArrowLeft) {
                    (-1, 0)
                } else if pressed(Key::ArrowDown) {
                    (0, 1)
                } else if pressed(Key::ArrowUp) {
                    (0, -1)
                } else {
                    (0, 0)
                };
                if horizontal != 0 || vertical != 0 {
                    let _ = self.album_move(horizontal, vertical);
                }
                if pressed(Key::Enter) {
                    if let Some(error) = self.execute(Command::EnterAlbum).error {
                        self.message = error.message;
                    }
                } else if pressed(Key::Space) {
                    self.album_open();
                }
            }
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
                if pressed(Key::P) && gallery.selected_photo().is_some_and(|p| p.is_video()) {
                    let paused = self
                        .video
                        .as_ref()
                        .is_some_and(|v| v.lock().unwrap().state.paused);
                    self.execute(Command::VideoPause { paused: !paused });
                }
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
}
