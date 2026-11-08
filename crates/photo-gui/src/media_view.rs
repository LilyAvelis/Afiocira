use super::*;

impl Workshop {
    pub(super) fn sync_video(&mut self) {
        let path = self
            .gallery
            .as_ref()
            .filter(|g| g.view() == View::Photo)
            .and_then(Gallery::selected_photo)
            .filter(|p| p.is_video())
            .map(|p| p.path.clone())
            .filter(|path| {
                !self.operation.as_ref().is_some_and(|op| {
                    op.running
                        && matches!(
                            op.action,
                            photo_core::actions::ActionId::Rename
                                | photo_core::actions::ActionId::Delete
                        )
                        && op.files.iter().any(|f| &f.source == path)
                })
            });
        if let Some(video) = &self.video {
            let mut video = video.lock().unwrap();
            video.poll();
            video.sync_path(path.as_deref());
            if let Some((expected, position, paused, deadline)) = self.video_restore.clone() {
                if path.as_ref() != Some(&expected)
                    || Instant::now() > deadline
                    || video.state.error.is_some()
                {
                    self.video_restore = None;
                } else if video.state.duration > 0.0 {
                    let _ = video.command(&["seek", &position.to_string(), "absolute+exact"]);
                    let _ = video.command(&["set", "pause", if paused { "yes" } else { "no" }]);
                    self.video_restore = None;
                }
            }
        }
    }
    pub(super) fn video_command(&mut self, args: &[&str]) -> Result<(), String> {
        self.sync_video();
        let video = self
            .video
            .as_ref()
            .ok_or_else(|| self.video_error.clone().unwrap_or_default())?;
        let mut video = video.lock().unwrap();
        if video.state.path.is_none() {
            return Err("No video is open".into());
        }
        video.command(args)
    }
    pub(super) fn video_view(&mut self, ui: &mut egui::Ui) {
        if ui.rect_contains_pointer(ui.available_rect_before_wrap())
            && ui.input(|i| i.pointer.any_pressed())
        {
            self.active_pane = ActivePane::Gallery;
            release_keyboard_focus(ui);
        }
        self.sync_video();
        let name = self
            .gallery
            .as_ref()
            .and_then(Gallery::selected_photo)
            .map(|p| p.name.clone())
            .unwrap_or_default();
        ui.label(name);
        let Some(player) = self.video.clone() else {
            ui.colored_label(
                Color32::LIGHT_RED,
                self.video_error.as_deref().unwrap_or("Video unavailable"),
            );
            return;
        };
        let state = player.lock().unwrap().state.clone();
        ui.horizontal_wrapped(|ui| {
            if ui
                .button(if state.paused { "Play" } else { "Pause" })
                .clicked()
            {
                self.execute(Command::VideoPause {
                    paused: !state.paused,
                });
            }
            if ui.button("-5 s").clicked() {
                self.execute(Command::VideoSeek {
                    seconds: (state.position - 5.0).max(0.0),
                });
            }
            if ui.button("+5 s").clicked() {
                self.execute(Command::VideoSeek {
                    seconds: (state.position + 5.0).min(state.duration.max(0.0)),
                });
            }
            let mut volume = state.volume;
            if ui
                .add(egui::Slider::new(&mut volume, 0.0..=100.0).text("Volume"))
                .changed()
            {
                self.execute(Command::VideoVolume { volume });
            }
        });
        let mut position = state.position;
        let response = ui.add(
            egui::Slider::new(&mut position, 0.0..=state.duration.max(0.01))
                .text(format!("{:.0} / {:.0} s", state.position, state.duration)),
        );
        if response.changed() {
            self.execute(Command::VideoSeek { seconds: position });
        }
        if let Some(error) = state.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        video::canvas(ui, player);
    }

    pub(super) fn photo(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(egui::Button::image_and_text(
                    Lucide::Images.size(16.0).image(),
                    "Gallery",
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
            if !self
                .gallery
                .as_ref()
                .and_then(Gallery::selected_photo)
                .is_some_and(|p| p.is_video())
            {
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
            }
        });
        let Some(index) = self.gallery.as_ref().and_then(Gallery::selected_index) else {
            return;
        };
        if self.gallery.as_ref().unwrap().photos()[index].is_video() {
            self.video_view(ui);
            return;
        }
        self.retire_transform_animation();
        self.prefetch_previews(index);
        let name = self.gallery.as_ref().unwrap().photos()[index].name.clone();
        ui.add(egui::Label::new(name).truncate());
        ui.separator();
        let available =
            crate::layout::visible_canvas(ui.available_rect_before_wrap(), ui.clip_rect())
                .size()
                .max(Vec2::splat(1.0));
        let (rect, response) = ui.allocate_exact_size(available, egui::Sense::drag());
        if response.clicked() || response.drag_started() {
            release_keyboard_focus(ui);
            self.active_pane = ActivePane::Gallery;
        }
        if let Some(texture) = self
            .transform_animation
            .as_ref()
            .map(|animation| animation.texture())
            .or_else(|| {
                self.previews
                    .get(&index)
                    .or_else(|| self.thumbnails.get(&index))
                    .or_else(|| self.fallback_previews.get(&index))
                    .cloned()
            })
        {
            let natural = texture.size_vec2();
            let fitted = crate::layout::fit_rect(rect, natural).size();
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
            if let Some(animation) = &self.transform_animation {
                animation.paint(
                    ui,
                    rect,
                    zoom.factor,
                    Vec2::new(zoom.offset.x, zoom.offset.y),
                );
                return;
            }
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
