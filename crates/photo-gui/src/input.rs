use super::*;

impl Workshop {
    pub(super) fn key_input(&mut self, ctx: &egui::Context, columns: usize) {
        let Some(gallery) = self.gallery.as_ref() else {
            return;
        };
        let pressed = |key| {
            ctx.input(|input| {
                input.key_pressed(key) && !input.modifiers.ctrl && !input.modifiers.alt
            })
        };
        match gallery.view() {
            View::Grid if self.album_mode() => {
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
            }
            View::Grid => {
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
            }
            View::Photo => {}
        }
    }
}
