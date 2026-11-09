//! Space image commands; immutable source snapshots and the shared background worker.
use super::*;
use photo_core::api::{ImageDialogState, ImageOptions};
pub(super) struct Dialog {
    pub snapshot: ImageDialogState,
    version: photo_core::rename::SourceVersion,
    options: ImageOptions,
    width: String,
    height: String,
    weight: String,
    first_frame: bool,
    enter_armed: bool,
    focus_order: Vec<egui::Id>,
}
impl Workshop {
    pub(super) fn image_tool_action(&self) -> Option<photo_core::actions::ActionId> {
        self.image_dialog.as_ref().map(|d| {
            if d.snapshot.convert {
                photo_core::actions::ActionId::ConvertImage
            } else {
                photo_core::actions::ActionId::ResizeImage
            }
        })
    }
    pub(super) fn apply_current_image(&mut self) -> Result<(), String> {
        let d = self
            .image_dialog
            .as_ref()
            .ok_or("Сначала откройте обработку изображения")?;
        let parse = |s: &str| -> Result<Option<u32>, String> {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                s.trim()
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n > 0)
                    .map(Some)
                    .ok_or_else(|| "Введите целые положительные размеры и вес".into())
            }
        };
        let mut options = d.options.clone();
        options.width = parse(&d.width)?;
        options.height = parse(&d.height)?;
        options.target_bytes = parse(&d.weight)?.map(|v| u64::from(v) * 1024);
        self.submit_image(options)
    }
    pub(super) fn begin_image(&mut self, convert: bool) -> Result<(), String> {
        if self.image_dialog.is_some()
            || self.rename_dialog.is_some()
            || self.delete_dialog.is_some()
            || self.operation.as_ref().is_some_and(|j| j.running)
        {
            return Err("Сначала завершите текущую операцию".into());
        }
        let g = self
            .gallery
            .as_ref()
            .filter(|g| g.view() == View::Photo)
            .ok_or("Откройте изображение в Space")?;
        let photo = g
            .selected_photo()
            .filter(|p| !p.is_video())
            .ok_or("Нужно изображение")?;
        let source = photo.path.clone();
        let version = photo_core::rename::capture(&source).map_err(|e| e.to_string())?;
        self.close_palette();
        let mut options = ImageOptions::default();
        if convert
            && source
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        {
            options.format = "webp".into();
        }
        self.image_dialog = Some(Dialog {
            snapshot: ImageDialogState {
                source,
                convert,
                error: None,
            },
            version,
            options,
            width: String::new(),
            height: String::new(),
            weight: String::new(),
            first_frame: true,
            enter_armed: false,
            focus_order: Vec::new(),
        });
        Ok(())
    }
    pub(super) fn submit_image(&mut self, options: ImageOptions) -> Result<(), String> {
        let dialog = self
            .image_dialog
            .as_ref()
            .ok_or("Сначала откройте команду обработки")?;
        if !["png", "jpg", "webp"].contains(&options.format.as_str()) {
            return Err("Выберите PNG, JPEG или WebP".into());
        }
        let task = operations::TaskKind::Image {
            source: dialog.snapshot.source.clone(),
            version: dialog.version.clone(),
            options,
            convert: dialog.snapshot.convert,
        };
        self.start_task(task)
    }
    pub(super) fn image_popup(&mut self, ctx: &egui::Context) {
        let busy = self.operation.as_ref().is_some_and(|j| j.running);
        let Some(d) = &mut self.image_dialog else {
            return;
        };
        // Own modal navigation before widgets process keys. In particular, an Enter
        // opening this form must be released before Enter can apply anything.
        let mut enter = false;
        let mut escape = false;
        let mut tab = None;
        ctx.input_mut(|input| {
            if !input.key_down(Key::Enter) && !d.first_frame {
                d.enter_armed = true;
            }
            input.events.retain(|event| {
                if let egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    modifiers,
                    ..
                } = event
                {
                    match key {
                        Key::Enter => {
                            if enter_allowed(d.first_frame, d.enter_armed, *pressed, *repeat) {
                                enter = true;
                            }
                            return false;
                        }
                        Key::Escape => {
                            escape |= *pressed && !*repeat;
                            return false;
                        }
                        Key::Tab => {
                            if *pressed {
                                tab = Some(modifiers.shift);
                            }
                            return false;
                        }
                        _ => {}
                    }
                }
                true
            });
        });
        if !busy && let Some(reverse) = tab {
            let focused = ctx.memory(|m| m.focused());
            if let Some(id) = next_focus(&d.focus_order, focused, reverse) {
                ctx.memory_mut(|m| m.request_focus(id));
            }
        }
        let mut submit = false;
        let mut cancel = false;
        let mut options = None;
        let mut controls = Vec::new();
        let frame = egui::Frame::window(&ctx.style_of(egui::Theme::Dark))
            .fill(Color32::from_rgb(43, 43, 47))
            .stroke(egui::Stroke::new(1.0, Color32::from_gray(73)))
            .corner_radius(12)
            .inner_margin(18);
        egui::Modal::new(egui::Id::new("image-command")).frame(frame).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 72.0).clamp(260.0, 460.0));
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 12.0);
            ui.horizontal(|ui| {
                ui.add(if d.snapshot.convert { Lucide::RefreshCw } else { Lucide::Scaling }.size(21.0).image());
                ui.label(egui::RichText::new(if d.snapshot.convert { "Конвертировать" } else { "Размер и вес" }).size(20.0).strong());
            });
            ui.add(egui::Label::new(egui::RichText::new(d.snapshot.source.file_name().unwrap_or_default().to_string_lossy())
                .size(12.0).color(Color32::from_gray(170))).truncate());
            ui.add_space(2.0);
            ui.add_enabled_ui(!busy, |ui| {
                egui::Grid::new("image-command-fields").num_columns(2)
                    .spacing(egui::vec2(20.0, 13.0)).min_col_width(104.0).show(ui, |ui| {
                        if !d.snapshot.convert {
                            field_label(ui, "Ширина");
                            controls.push(ui.add(egui::TextEdit::singleline(&mut d.width).desired_width(260.0).hint_text("Исходная · px")).id);
                            ui.end_row();
                            field_label(ui, "Высота");
                            controls.push(ui.add(egui::TextEdit::singleline(&mut d.height).desired_width(260.0).hint_text("Исходная · px")).id);
                            ui.end_row();
                            ui.label("");
                            controls.push(ui.checkbox(&mut d.options.preserve_aspect, "Сохранять пропорции").id);
                            ui.end_row();
                        }
                        field_label(ui, "Формат");
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            for (value, label) in [("png", "PNG"), ("jpg", "JPEG"), ("webp", "WebP")] {
                                let response = ui.add(egui::Button::new(label).selected(d.options.format == value).min_size(egui::vec2(70.0, 30.0)));
                                if response.clicked() { d.options.format = value.into(); }
                                controls.push(response.id);
                            }
                        });
                        ui.end_row();
                        if !d.snapshot.convert {
                            field_label(ui, "Максимум веса");
                            controls.push(ui.add(egui::TextEdit::singleline(&mut d.weight).desired_width(260.0).hint_text("Без ограничения · КиБ")).id);
                            ui.end_row();
                        } else if d.options.format == "jpg" {
                            field_label(ui, "Качество");
                            ui.horizontal(|ui| {
                                controls.push(ui.add(egui::Slider::new(&mut d.options.quality, 1..=100).show_value(false)).id);
                                ui.label(format!("{}%", d.options.quality));
                            });
                            ui.end_row();
                        }
                    });
                if d.snapshot.convert {
                    ui.add_space(1.0);
                    controls.push(ui.checkbox(&mut d.options.strip_metadata, "Удалить метаданные и цветовой профиль").id);
                    hint(ui, if d.options.strip_metadata {
                        "Без цветового профиля оттенки могут измениться."
                    } else { "Метаданные не удаляем без вашего разрешения." });
                } else {
                    hint(ui, if d.options.format == "jpg" {
                        "Лимит веса подбирается качеством JPEG."
                    } else { "PNG и WebP сохраняются без потерь. Для меньшего веса уменьшите размеры." });
                }
            });
            if let Some(error) = &d.snapshot.error {
                egui::Frame::new().fill(Color32::from_rgb(66, 43, 46)).corner_radius(6).inner_margin(9).show(ui, |ui| {
                    ui.label(egui::RichText::new(error).size(12.0).color(Color32::from_rgb(246, 181, 181)));
                });
            }
            ui.separator();
            hint(ui, if d.snapshot.convert { "Исходный файл будет заменён." } else { "Сохраним новую копию рядом. Метаданные камеры удаляются." });
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    let response = ui.button("Отмена"); cancel |= response.clicked(); controls.push(response.id);
                    ui_style::keycap(ui, "Esc");
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui_style::keycap(ui, "Enter");
                    let response = ui.add_enabled(!busy, egui::Button::new(egui::RichText::new("Применить").strong())
                        .fill(Color32::from_rgb(89, 89, 99)).min_size(egui::vec2(112.0, 32.0)));
                    submit |= response.clicked(); controls.push(response.id);
                });
            });
        });
        if !busy {
            // Stable logical order, independent of the footer's right-to-left layout.
            if (d.first_frame
                || !ctx
                    .memory(|m| m.focused())
                    .is_some_and(|id| controls.contains(&id)))
                && let Some(id) = controls.first()
            {
                ctx.memory_mut(|m| m.request_focus(*id));
            }
            submit |= enter;
        }
        if !busy
            && let Some(id) = ctx.memory(|m| m.focused())
            && controls.contains(&id)
            && let Some(response) = ctx.read_response(id)
        {
            ctx.layer_painter(response.layer_id).rect_stroke(
                response.rect.expand(2.0),
                6,
                egui::Stroke::new(1.0, Color32::from_rgb(176, 185, 205)),
                egui::StrokeKind::Outside,
            );
        }
        d.first_frame = false;
        d.focus_order = controls;
        if submit && !busy {
            let parse = |s: &str| -> Result<Option<u32>, String> {
                if s.trim().is_empty() {
                    Ok(None)
                } else {
                    s.trim()
                        .parse::<u32>()
                        .ok()
                        .filter(|n| *n > 0)
                        .map(Some)
                        .ok_or_else(|| "Введите целые положительные размеры и вес".into())
                }
            };
            match (parse(&d.width), parse(&d.height), parse(&d.weight)) {
                (Ok(width), Ok(height), Ok(weight)) => {
                    let mut o = d.options.clone();
                    o.width = width;
                    o.height = height;
                    o.target_bytes = weight.map(|v| u64::from(v) * 1024);
                    options = Some(o);
                }
                _ => d.snapshot.error = Some("Введите целые положительные размеры и вес".into()),
            }
        }
        if let Some(options) = options {
            if let Err(error) = self.submit_image(options)
                && let Some(d) = &mut self.image_dialog
            {
                d.snapshot.error = Some(error);
            }
        } else if !busy && (cancel || escape) {
            self.image_dialog = None;
        }
    }
}
pub(super) fn process(
    source: &Path,
    version: &photo_core::rename::SourceVersion,
    o: &ImageOptions,
    convert: bool,
) -> Result<(PathBuf, String), String> {
    if &photo_core::rename::capture(source).map_err(|e| e.to_string())? != version {
        return Err("Исходный файл изменился; откройте команду заново".into());
    }
    if convert {
        let format = match o.format.as_str() {
            "png" => convert_image::OutputFormat::Png,
            "webp" => convert_image::OutputFormat::WebP,
            "jpg" => convert_image::OutputFormat::Jpeg { quality: o.quality },
            _ => return Err("Неизвестный формат".into()),
        };
        let prepared = convert_image::prepare(
            source,
            format,
            convert_image::ConversionOptions {
                strip_metadata: o.strip_metadata,
            },
        )
        .map_err(|e| e.to_string())?;
        let (w, h) = prepared.dimensions();
        let target = prepared.target().to_path_buf();
        let path = convert_image::commit(prepared).map_err(|e| e.to_string())?;
        let bytes = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        Ok((
            path,
            format!(
                "{} · {w} × {h} · {bytes} байт",
                target.file_name().unwrap_or_default().to_string_lossy()
            ),
        ))
    } else {
        let format = match o.format.as_str() {
            "png" => resize_image::OutputFormat::Png,
            "webp" => resize_image::OutputFormat::WebP,
            "jpg" => resize_image::OutputFormat::Jpeg,
            _ => return Err("Неизвестный формат".into()),
        };
        let r = resize_image::resize_copy(
            source,
            source.parent().ok_or("Нет папки")?,
            &resize_image::ResizeOptions {
                width: o.width,
                height: o.height,
                preserve_aspect: o.preserve_aspect,
                format,
                target_bytes: o.target_bytes,
            },
        )?;
        let quality = r
            .jpeg_quality
            .map(|q| format!(" · JPEG {q}"))
            .unwrap_or_default();
        let _metadata = r.metadata_notice;
        Ok((
            r.path,
            format!("{} × {} · {} байт{quality}", r.width, r.height, r.bytes),
        ))
    }
}

fn field_label(ui: &mut egui::Ui, label: &str) {
    ui.label(
        egui::RichText::new(label)
            .size(13.0)
            .color(Color32::from_gray(193)),
    );
}
fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .size(12.0)
            .color(Color32::from_gray(168)),
    );
}
fn enter_allowed(first_frame: bool, armed: bool, pressed: bool, repeat: bool) -> bool {
    !first_frame && armed && pressed && !repeat
}
fn next_focus(order: &[egui::Id], current: Option<egui::Id>, reverse: bool) -> Option<egui::Id> {
    if order.is_empty() {
        return None;
    }
    let index = current.and_then(|id| order.iter().position(|candidate| *candidate == id));
    Some(
        order[match (index, reverse) {
            (Some(0), true) | (None, true) => order.len() - 1,
            (Some(i), true) => i - 1,
            (Some(i), false) => (i + 1) % order.len(),
            (None, false) => 0,
        }],
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enter_requires_release_after_opening_and_ignores_repeats() {
        assert!(!enter_allowed(true, true, true, false));
        assert!(!enter_allowed(false, false, true, false));
        assert!(!enter_allowed(false, true, true, true));
        assert!(!enter_allowed(false, true, false, false));
        assert!(enter_allowed(false, true, true, false));
    }
    #[test]
    fn modal_focus_wraps_in_both_directions_and_recovers_external_focus() {
        let ids: Vec<_> = (0..9).map(egui::Id::new).collect();
        for i in 0..ids.len() {
            assert_eq!(
                next_focus(&ids, Some(ids[i]), false),
                Some(ids[(i + 1) % ids.len()])
            );
            assert_eq!(
                next_focus(&ids, Some(ids[i]), true),
                Some(ids[(i + ids.len() - 1) % ids.len()])
            );
        }
        assert_eq!(
            next_focus(&ids, Some(egui::Id::new("outside")), false),
            Some(ids[0])
        );
        assert_eq!(next_focus(&ids, None, true), ids.last().copied());
        assert_eq!(next_focus(&[], None, false), None);
    }
}
