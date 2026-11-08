//! Live side-panel editing. Preview never modifies source bytes.
use super::*;
use crate::ui_foundation::{SettingsRow, tokens};
use photo_core::actions::ActionId;
use photo_core::adjustments::{AdjustmentOptions, CurvePoint, PreviewState};

fn tool_title(tool: ActionId) -> &'static str {
    match tool {
        ActionId::AdjustDetails => "Детали",
        ActionId::AdjustVignette => "Виньетка",
        _ => "Свет и цвет",
    }
}

fn is_neutral(options: &AdjustmentOptions) -> bool {
    let mut neutral = options.clone();
    if neutral
        .curve
        .as_ref()
        .is_some_and(|c| c.iter().all(|p| p.x == p.y))
    {
        neutral.curve = None;
    }
    neutral == AdjustmentOptions::default()
}
fn adjustment_slider(
    ui: &mut egui::Ui,
    label: &str,
    description: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) {
    SettingsRow::show(ui, label, description, |ui| {
        ui.spacing_mut().slider_width = (ui.available_width() - 60.0).clamp(60.0, 240.0);
        ui.add(
            egui::Slider::new(value, range)
                .max_decimals(2)
                .clamping(egui::SliderClamping::Edits),
        );
    });
}

pub(super) struct Dialog {
    tool: ActionId,
    finish_requested: bool,
    first_frame: bool,
    pub(super) source: PathBuf,
    pub(super) version: photo_core::rename::SourceVersion,
    options: AdjustmentOptions,
    error: Option<String>,
    curve_enabled: bool,
    curve_y: [f32; 3],
    curve_basis: [f32; 3],
    worker: crate::adjustment_preview::PreviewWorker,
    requested_revision: u64,
    ready_revision: Option<u64>,
    requested_options: AdjustmentOptions,
    texture: Option<TextureHandle>,
    compare_original: bool,
}
fn options_with_curve(d: &Dialog) -> AdjustmentOptions {
    let mut o = d.options.clone();
    o.curve = d.curve_enabled.then(|| {
        if d.curve_y == d.curve_basis
            && let Some(curve) = &d.options.curve
        {
            return curve.clone();
        }
        vec![
            CurvePoint { x: 0.0, y: 0.0 },
            CurvePoint {
                x: 0.25,
                y: d.curve_y[0],
            },
            CurvePoint {
                x: 0.5,
                y: d.curve_y[1],
            },
            CurvePoint {
                x: 0.75,
                y: d.curve_y[2],
            },
            CurvePoint { x: 1.0, y: 1.0 },
        ]
    });
    o
}
impl Workshop {
    pub(super) fn adjustment_tool_action(&self) -> Option<ActionId> {
        self.adjustments_panel.as_ref().map(|d| d.tool)
    }
    pub(super) fn toggle_adjustment_tool(&mut self, tool: ActionId) -> Result<(), String> {
        if let Some(active) = self.adjustment_tool_action() {
            return if active == tool {
                self.apply_adjustments()
            } else {
                Err("Примените или отмените текущий инструмент".into())
            };
        }
        self.begin_adjustment_tool(tool)
    }
    pub(super) fn begin_adjustments(&mut self) -> Result<(), String> {
        self.begin_adjustment_tool(ActionId::AdjustImage)
    }
    fn begin_adjustment_tool(&mut self, tool: ActionId) -> Result<(), String> {
        if self.adjustments_panel.is_some()
            || self.image_dialog.is_some()
            || self.rename_dialog.is_some()
            || self.delete_dialog.is_some()
            || self.operation.as_ref().is_some_and(|j| j.running)
        {
            return Err("Сначала завершите текущую операцию".into());
        }
        if self.metadata.busy() || !self.metadata_panel.close() {
            return Err("Сохраните или отмените изменения метаданных".into());
        }
        let source = self.history_path()?;
        if !source
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| {
                ["png", "jpg", "jpeg", "webp"]
                    .iter()
                    .any(|s| e.eq_ignore_ascii_case(s))
            })
        {
            return Err("Нужно статическое PNG, JPEG или WebP, 8 бит".into());
        }
        let version = photo_core::rename::capture(&source).map_err(|e| e.to_string())?;
        self.history_panel.open = false;
        self.history_panel.reset_thumbnails();
        self.active_pane = ActivePane::Gallery;
        self.close_palette();
        let worker = crate::adjustment_preview::PreviewWorker::new(self.ctx.clone());
        worker.request(
            source.clone(),
            version.clone(),
            AdjustmentOptions::default(),
            1,
        );
        self.adjustments_panel = Some(Dialog {
            tool,
            finish_requested: false,
            first_frame: true,
            source,
            version,
            options: AdjustmentOptions::default(),
            error: None,
            curve_enabled: false,
            curve_y: [0.25, 0.5, 0.75],
            curve_basis: [0.25, 0.5, 0.75],
            worker,
            requested_revision: 1,
            ready_revision: None,
            requested_options: AdjustmentOptions::default(),
            texture: None,
            compare_original: false,
        });
        Ok(())
    }
    pub(super) fn preview_adjustments(&mut self, options: AdjustmentOptions) -> Result<(), String> {
        options.validate()?;
        let d = self
            .adjustments_panel
            .as_mut()
            .ok_or("Сначала откройте коррекцию")?;
        d.options = options.clone();
        d.finish_requested = false;
        d.curve_enabled = options.curve.is_some();
        if let Some(points) = &options.curve {
            for (index, x) in [0.25, 0.5, 0.75].iter().enumerate() {
                if let Some(pair) = points.windows(2).find(|p| *x <= p[1].x) {
                    d.curve_y[index] = pair[0].y
                        + (*x - pair[0].x) / (pair[1].x - pair[0].x) * (pair[1].y - pair[0].y);
                }
            }
        }
        d.curve_basis = d.curve_y;
        if d.requested_options == options {
            if d.ready_revision == Some(d.requested_revision) {
                d.error = None;
            }
            return Ok(());
        }
        d.requested_revision = d
            .requested_revision
            .checked_add(1)
            .ok_or("Preview revision exhausted")?;
        d.requested_options = options.clone();
        d.error = None;
        d.compare_original = false;
        d.worker.request(
            d.source.clone(),
            d.version.clone(),
            options,
            d.requested_revision,
        );
        Ok(())
    }
    pub(super) fn poll_adjustments_preview(&mut self) {
        let Some(d) = &mut self.adjustments_panel else {
            return;
        };
        if let Some(result) = d.worker.poll() {
            if result.path != d.source
                || result.source_version != d.version
                || result.revision != d.requested_revision
            {
                return;
            }
            match result.image {
                Ok(image) => {
                    d.texture = Some(self.ctx.load_texture(
                        format!("adjustment-live-{}", result.revision),
                        image,
                        TextureOptions::LINEAR,
                    ));
                    d.ready_revision = Some(result.revision);
                    d.error = None;
                }
                Err(error) => {
                    d.finish_requested = false;
                    d.error = Some(error);
                    d.texture = None;
                    d.ready_revision = None;
                }
            }
        }
        let finish = d.finish_requested
            && d.error.is_none()
            && d.ready_revision == Some(d.requested_revision);
        if finish {
            d.finish_requested = false;
            if let Err(error) = self.apply_adjustments()
                && let Some(d) = &mut self.adjustments_panel
            {
                d.error = Some(error);
            }
        }
    }
    pub(super) fn adjustments_preview_state(&self) -> Option<PreviewState> {
        self.adjustments_panel.as_ref().map(|d| PreviewState {
            tool: d.tool,
            finishing: d.finish_requested,
            path: d.source.clone(),
            requested_revision: d.requested_revision,
            ready_revision: d.ready_revision,
            pending: d.error.is_none() && d.ready_revision != Some(d.requested_revision),
            options: d.requested_options.clone(),
            error: d.error.clone(),
        })
    }
    pub(super) fn adjustment_texture(&self) -> Option<TextureHandle> {
        let photo = self.gallery.as_ref()?.selected_photo()?;
        if let Some(d) = self.adjustments_panel.as_ref() {
            return if photo.path == d.source && !d.compare_original {
                d.texture.clone()
            } else {
                None
            };
        }
        self.adjustment_commit_preview
            .as_ref()
            .filter(|(id, path, _)| {
                photo.path == *path
                    && self
                        .operation
                        .as_ref()
                        .is_some_and(|o| o.id == *id && o.running)
            })
            .map(|(_, _, texture)| texture.clone())
    }
    pub(super) fn submit_adjustments(&mut self, options: AdjustmentOptions) -> Result<(), String> {
        options.validate()?;
        if is_neutral(&options) {
            return Err("Измените хотя бы один параметр".into());
        }
        let d = self
            .adjustments_panel
            .as_ref()
            .ok_or("Сначала откройте коррекцию")?;
        let preview =
            if d.ready_revision == Some(d.requested_revision) && d.requested_options == options {
                d.texture.clone().map(|t| (d.source.clone(), t))
            } else {
                None
            };
        self.start_task(operations::TaskKind::Adjust {
            source: d.source.clone(),
            version: d.version.clone(),
            options,
        })?;
        self.adjustment_commit_preview =
            preview.map(|(path, texture)| (self.next_operation, path, texture));
        self.adjustments_panel = None;
        Ok(())
    }
    pub(super) fn cancel_adjustments(&mut self) {
        if let Some(d) = self.adjustments_panel.take() {
            d.worker.cancel();
        }
    }
    pub(super) fn apply_adjustments(&mut self) -> Result<(), String> {
        let d = self
            .adjustments_panel
            .as_mut()
            .ok_or("Сначала откройте коррекцию")?;
        if is_neutral(&d.requested_options) {
            self.cancel_adjustments();
            return Ok(());
        }
        if let Some(error) = &d.error {
            return Err(error.clone());
        }
        if d.ready_revision != Some(d.requested_revision) {
            d.finish_requested = true;
            return Ok(());
        }
        let options = d.requested_options.clone();
        self.submit_adjustments(options)
    }
    pub(super) fn render_adjustments_panel(&mut self, ui: &mut egui::Ui) {
        let toggle_binding = self
            .adjustment_tool_action()
            .map(|id| {
                shortcuts::effective(&self.settings.shortcuts, self.shortcut_context(), id, "")
            })
            .unwrap_or_default();
        let Some(d) = &mut self.adjustments_panel else {
            return;
        };
        let before = options_with_curve(d);
        let enter = shortcuts::take_editor_enter(ui.ctx(), !d.first_frame);
        let mut apply = false;
        let mut cancel = false;
        egui::Panel::right("adjustments-panel")
            .default_size(330.0)
            .size_range(260.0..=440.0)
            .resizable(true)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(tokens::SM, tokens::SM);
                ui.heading(tool_title(d.tool));
                ui.add(
                    egui::Label::new(d.source.file_name().unwrap_or_default().to_string_lossy())
                        .truncate(),
                );
                egui::Panel::bottom("adjustment-commit").show(ui, |ui| {
                    ui.separator();
                    if d.error.is_none() && d.ready_revision != Some(d.requested_revision) {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.weak("Обновляем изображение…");
                        });
                    }
                    if let Some(error) = &d.error {
                        crate::ui_foundation::message(ui, error, true);
                    }
                    ui.weak("Исходный файл изменится только после применения.");
                    ui.weak("Enter — применить и закрыть");
                    if !toggle_binding.is_empty() {
                        ui.weak(format!("{toggle_binding} — применить и закрыть"));
                    }
                    if d.source
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| ["jpg", "jpeg"].iter().any(|s| e.eq_ignore_ascii_case(s)))
                    {
                        ui.weak("JPEG: повторное сжатие, качество 95.");
                    }
                    ui.horizontal_wrapped(|ui| {
                        cancel = ui.button("Отмена").clicked();
                        apply = ui
                            .add_enabled(
                                d.error.is_none() && !d.finish_requested,
                                egui::Button::new("Применить"),
                            )
                            .clicked();
                        if ui.button("Сбросить").clicked() {
                            d.options = AdjustmentOptions::default();
                            d.curve_enabled = false;
                            d.curve_y = [0.25, 0.5, 0.75];
                        }
                    });
                });
                ui.checkbox(&mut d.compare_original, "Показать оригинал");
                egui::ScrollArea::vertical()
                    .id_salt("adjustment-controls")
                    .show(ui, |ui| {
                        if d.finish_requested {
                            ui.disable();
                        }
                        if d.tool == ActionId::AdjustImage {
                            egui::CollapsingHeader::new("Свет · 7")
                                .default_open(true)
                                .show(ui, |ui| {
                                    for (label, description, value, range) in [
                                        (
                                            "Экспозиция",
                                            "Ступени EV",
                                            &mut d.options.exposure,
                                            -5.0..=5.0,
                                        ),
                                        (
                                            "Блеск",
                                            "Баланс теней и светов",
                                            &mut d.options.brilliance,
                                            -1.0..=1.0,
                                        ),
                                        (
                                            "Контраст",
                                            "Мягче — контрастнее",
                                            &mut d.options.contrast,
                                            -1.0..=1.0,
                                        ),
                                        (
                                            "Тени",
                                            "Темнее — светлее",
                                            &mut d.options.shadows,
                                            -1.0..=1.0,
                                        ),
                                        (
                                            "Света",
                                            "Темнее — светлее",
                                            &mut d.options.highlights,
                                            -1.0..=1.0,
                                        ),
                                        (
                                            "Яркость",
                                            "Общая яркость",
                                            &mut d.options.brightness,
                                            -1.0..=1.0,
                                        ),
                                        (
                                            "Точка чёрного",
                                            "Глубина тёмных тонов",
                                            &mut d.options.black_point,
                                            -1.0..=1.0,
                                        ),
                                    ] {
                                        SettingsRow::show(ui, label, description, |ui| {
                                            ui.spacing_mut().slider_width =
                                                (ui.available_width() - 60.0).clamp(60.0, 240.0);
                                            ui.add(
                                                egui::Slider::new(value, range)
                                                    .max_decimals(2)
                                                    .clamping(egui::SliderClamping::Edits),
                                            );
                                        });
                                    }
                                });
                            egui::CollapsingHeader::new("Цвет · 4").show(ui, |ui| {
                                for (label, description, value) in [
                                    (
                                        "Насыщенность",
                                        "Интенсивность всех цветов",
                                        &mut d.options.saturation,
                                    ),
                                    ("Красочность", "Приглушённые цвета", &mut d.options.vibrance),
                                    (
                                        "Температура",
                                        "Холоднее — теплее",
                                        &mut d.options.temperature,
                                    ),
                                    ("Оттенок", "Зелёный — пурпурный", &mut d.options.tint),
                                ] {
                                    adjustment_slider(ui, label, description, value, -1.0..=1.0);
                                }
                            });
                        }
                        if d.tool == ActionId::AdjustDetails {
                            for (label, description, value) in [
                                ("Резкость", "Мелкие контуры", &mut d.options.sharpness),
                                ("Чёткость", "Локальный контраст", &mut d.options.definition),
                                (
                                    "Шумоподавление",
                                    "Сглаживание мелкого шума",
                                    &mut d.options.noise_reduction,
                                ),
                            ] {
                                adjustment_slider(ui, label, description, value, 0.0..=1.0);
                            }
                        }
                        if d.tool == ActionId::AdjustVignette {
                            adjustment_slider(
                                ui,
                                "Виньетка",
                                "Затемнение краёв",
                                &mut d.options.vignette,
                                0.0..=1.0,
                            );
                        }
                        if d.tool == ActionId::AdjustImage {
                            ui.checkbox(&mut d.curve_enabled, "Тоновая кривая");
                            if d.curve_enabled {
                                for (index, label) in
                                    ["Точка 25%", "Точка 50%", "Точка 75%"].iter().enumerate()
                                {
                                    SettingsRow::show(
                                        ui,
                                        label,
                                        "Выходная яркость",
                                        |ui| {
                                            ui.spacing_mut().slider_width =
                                                (ui.available_width() - 60.0).clamp(60.0, 240.0);
                                            ui.add(
                                                egui::Slider::new(&mut d.curve_y[index], 0.0..=1.0)
                                                    .max_decimals(2)
                                                    .clamping(egui::SliderClamping::Edits),
                                            );
                                        },
                                    );
                                }
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 100.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    rect,
                                    tokens::CONTROL_RADIUS,
                                    Color32::from_gray(30),
                                );
                                let position = |x: f32, y: f32| {
                                    egui::pos2(
                                        rect.left() + x * rect.width(),
                                        rect.bottom() - y * rect.height(),
                                    )
                                };
                                let points = options_with_curve(d).curve.unwrap();
                                for p in points.windows(2) {
                                    ui.painter().line_segment(
                                        [position(p[0].x, p[0].y), position(p[1].x, p[1].y)],
                                        egui::Stroke::new(2.0, Color32::LIGHT_BLUE),
                                    );
                                }
                            }
                        }
                    });
            });
        let options = options_with_curve(d);
        let changed = before != options;
        d.first_frame = false;
        apply |= enter;
        if cancel || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
            self.execute(Command::CancelAdjustments);
        } else if apply && !changed {
            if let Some(error) = self.execute(Command::ApplyAdjustments).error
                && let Some(d) = &mut self.adjustments_panel
            {
                d.error = Some(error.message);
            }
        } else if changed {
            let result = self.execute(Command::PreviewAdjustments { options });
            if let Some(error) = result.error {
                if let Some(d) = &mut self.adjustments_panel {
                    d.error = Some(error.message);
                }
            } else if apply {
                let result = self.execute(Command::ApplyAdjustments);
                if let Some(error) = result.error
                    && let Some(d) = &mut self.adjustments_panel
                {
                    d.error = Some(error.message);
                }
            }
        }
    }
}
