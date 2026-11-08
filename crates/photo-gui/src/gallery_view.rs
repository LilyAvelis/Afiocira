use super::*;

impl Workshop {
    pub(super) fn grid(&mut self, ui: &mut egui::Ui) {
        self.gallery_scale_wheel(ui);
        self.nested_header(ui);
        if self.album_mode() {
            self.album_grid(ui);
            return;
        }
        let count = self.gallery.as_ref().map_or(0, |g| g.photos().len());
        if count == 0 {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.label("На этом уровне нет изображений или видео");
                    if self.nested.snapshot.subfolders.is_some_and(|n| n > 0)
                        && !self.nested.snapshot.loading
                        && ui.button("Показать изображения из подпапок").clicked()
                    {
                        let _ = self.load_nested(false);
                    }
                });
            });
            return;
        }
        let layout = grid_layout(
            ui.available_width(),
            ui.spacing().item_spacing,
            self.settings.effective_gallery_scale(),
        );
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
        let output = scroll.show_rows(ui, layout.row_height, rows, |ui, row_range| {
            for row in row_range {
                ui.horizontal(|ui| {
                    for column in 0..layout.columns {
                        let index = row * layout.columns + column;
                        if index >= count {
                            break;
                        }
                        self.gallery_tile(ui, index, layout);
                    }
                });
            }
        });
        let at_bottom =
            output.state.offset.y + output.inner_rect.height() >= output.content_size.y - 24.0;
        if at_bottom
            && self.nested.snapshot.expanded
            && !self.nested.snapshot.loading
            && self.nested.snapshot.can_load_more
            && ui.input(|i| i.smooth_scroll_delta.y < 0.0)
            && ui.rect_contains_pointer(output.inner_rect)
        {
            let _ = self.load_nested(false);
        }
    }
    pub(super) fn gallery_tile(&mut self, ui: &mut egui::Ui, index: usize, layout: GridLayout) {
        let item = &self.gallery.as_ref().unwrap().photos()[index];
        let relative = item
            .path
            .strip_prefix(self.gallery.as_ref().unwrap().folder())
            .unwrap_or(&item.path)
            .to_string_lossy();
        let relative = if self.album_mode() {
            std::borrow::Cow::Borrowed(item.name.as_str())
        } else {
            relative
        };
        let name = if item.is_video() {
            format!("[Video] {relative}")
        } else {
            relative.into_owned()
        };
        self.request_thumbnail(index);
        ui.vertical(|ui| {
            ui.set_width(layout.tile_width);
            let (cell, response) = ui.allocate_exact_size(
                Vec2::new(layout.tile_width, layout.image_height),
                egui::Sense::click(),
            );
            if let Some(texture) = self.thumbnails.get(&index) {
                let image = crate::layout::fit_rect(cell, texture.size_vec2());
                ui.painter_at(cell).image(
                    texture.id(),
                    image,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                let text = if self.gallery.as_ref().unwrap().photos()[index].is_video() {
                    "Video"
                } else if self.errors.contains_key(&index) {
                    "Cannot preview"
                } else {
                    "Loading…"
                };
                ui.painter_at(cell).text(
                    cell.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(13.0),
                    Color32::GRAY,
                );
            }
            if response.clicked() {
                release_keyboard_focus(ui);
                self.active_pane = ActivePane::Gallery;
                if ui.input(|input| input.modifiers.shift) {
                    self.execute(Command::ExtendSelection { index });
                } else if ui.input(|input| input.modifiers.ctrl) {
                    self.execute(Command::ToggleSelection { index });
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
}
