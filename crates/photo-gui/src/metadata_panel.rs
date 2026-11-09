//! Catalogue-driven metadata editor. The parent owns dispatch and worker receipts.
use crate::ui_foundation::{self, SettingsRow, tokens};
use eframe::egui;
use photo_core::metadata::{
    FIELDS, FieldKind, Metadata, MetadataJob, MetadataRequest, MetadataSnapshot,
};
use std::path::{Path, PathBuf};

pub(super) enum Intent {
    Request(MetadataRequest),
    Closed,
}

fn normalized_tags(tags: Vec<String>) -> Vec<String> {
    let mut tags: Vec<_> = tags
        .into_iter()
        .map(|tag| photo_registry::normalize_tag(&tag).unwrap_or(tag))
        .collect();
    tags.sort();
    tags.dedup();
    tags
}

fn tag_color(tag: &str) -> egui::Color32 {
    let palette = [
        egui::Color32::from_rgb(64, 89, 114),
        egui::Color32::from_rgb(86, 76, 112),
        egui::Color32::from_rgb(63, 99, 86),
        egui::Color32::from_rgb(112, 82, 63),
        egui::Color32::from_rgb(106, 68, 87),
    ];
    palette[tag.bytes().fold(0usize, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as usize)
    }) % palette.len()]
}

#[derive(Default)]
pub(super) struct Panel {
    target: Option<PathBuf>,
    snapshot: Option<MetadataSnapshot>,
    fields: Vec<String>,
    tags: Vec<String>,
    tag_input: String,
    tag_options_open: bool,
    pending: Option<u64>,
    awaiting_submission: bool,
    error: Option<String>,
    close_after_save: bool,
    embedded_filter: String,
}
impl Panel {
    pub(super) fn new() -> Self {
        Self::default()
    }
    pub(super) fn is_open(&self) -> bool {
        self.target.is_some()
    }
    pub(super) fn target(&self) -> Option<&Path> {
        self.target.as_deref()
    }
    pub(super) fn pending_request_id(&self) -> Option<u64> {
        self.pending
    }
    pub(super) fn is_dirty(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|s| {
            FIELDS
                .iter()
                .zip(&self.fields)
                .any(|(f, text)| f.kind != FieldKind::Tags && f.text(&s.metadata) != *text)
                || self.tags != s.metadata.tags
        })
    }
    fn busy(&self) -> bool {
        self.pending.is_some() || self.awaiting_submission
    }
    /// A refused switch retains the draft and original target.
    pub(super) fn open(&mut self, path: PathBuf) -> Option<MetadataRequest> {
        if self.target.as_ref() == Some(&path) {
            return None;
        }
        if self.is_dirty() || self.busy() {
            self.error = Some(
                "Сначала сохраните или отмените изменения. Дождитесь завершения текущего запроса."
                    .into(),
            );
            return None;
        }
        self.target = Some(path.clone());
        self.snapshot = None;
        self.fields.clear();
        self.tags.clear();
        self.tag_input.clear();
        self.tag_options_open = false;
        self.error = None;
        self.awaiting_submission = true;
        Some(MetadataRequest::Inspect { path })
    }
    pub(super) fn close(&mut self) -> bool {
        if self.is_dirty() || self.busy() {
            self.error = Some(
                "Сначала сохраните или отмените изменения и дождитесь завершения запроса.".into(),
            );
            return false;
        }
        *self = Self::new();
        true
    }
    /// Cancels the UI draft. An already submitted worker request keeps its own
    /// receipt, but cannot reopen this panel or attach its result to a new draft.
    pub(super) fn cancel(&mut self) {
        *self = Self::new();
    }
    /// Toggle/Enter commits a draft once and closes only after its receipt.
    pub(super) fn finish(&mut self) -> Result<Option<MetadataRequest>, String> {
        self.close_after_save = true;
        if self.busy() {
            return Ok(None);
        }
        if !self.tag_input.trim().is_empty() {
            self.add_tag(&self.tag_input.clone());
            if !self.tag_input.trim().is_empty() {
                self.close_after_save = false;
                return Err(self
                    .error
                    .clone()
                    .unwrap_or("Не удалось добавить тег".into()));
            }
        }
        if !self.is_dirty() {
            self.close();
            return Ok(None);
        }
        match self.save_request() {
            Ok(request) => Ok(Some(request)),
            Err(error) => {
                self.close_after_save = false;
                Err(error)
            }
        }
    }
    pub(super) fn attach_request(&mut self, job: &MetadataJob) {
        if self.target.as_ref() == Some(&job.path) && self.awaiting_submission {
            self.awaiting_submission = false;
            self.pending = Some(job.request_id);
            self.apply_job(job);
        }
    }
    pub(super) fn submission_failed(&mut self, error: String) {
        self.close_after_save = false;
        self.awaiting_submission = false;
        self.error = Some(error);
    }
    pub(super) fn request_failed(&mut self, error: String) {
        self.close_after_save = false;
        self.pending = None;
        self.awaiting_submission = false;
        self.error = Some(format!(
            "Результат запроса неизвестен. Проверьте сохранённые метаданные перед повторным сохранением: {error}"
        ));
    }
    /// Both receipt and path must match: an old completion can never replace a draft.
    pub(super) fn apply_job(&mut self, job: &MetadataJob) {
        if self.pending != Some(job.request_id)
            || self.target.as_ref() != Some(&job.path)
            || job.pending
        {
            return;
        }
        self.pending = None;
        if let Some(error) = &job.error {
            self.close_after_save = false;
            self.error = Some(error.message.clone());
            return;
        }
        if let Some(snapshot) = &job.snapshot
            && snapshot.path == job.path
        {
            self.set_snapshot(snapshot.clone());
            if self.close_after_save {
                self.close();
            }
        }
    }
    fn set_snapshot(&mut self, snapshot: MetadataSnapshot) {
        self.tags = snapshot.metadata.tags.clone();
        self.tag_input.clear();
        self.tag_options_open = false;
        self.fields = FIELDS.iter().map(|f| f.text(&snapshot.metadata)).collect();
        self.snapshot = Some(snapshot);
        self.error = None;
    }
    fn document(&self) -> Metadata {
        let mut metadata = self
            .snapshot
            .as_ref()
            .map(|s| s.metadata.clone())
            .unwrap_or_default();
        for (field, text) in FIELDS.iter().zip(&self.fields) {
            if field.kind != FieldKind::Tags {
                field.set_text(&mut metadata, text);
            }
        }
        metadata.tags = self.tags.clone();
        metadata
    }
    pub(super) fn panel_state(&self) -> photo_core::metadata::MetadataPanelState {
        photo_core::metadata::MetadataPanelState {
            embedded: self.snapshot.as_ref().map(|s| s.embedded.clone()),
            open: self.is_open(),
            path: self.target.clone(),
            dirty: self.is_dirty(),
            pending: self.busy(),
            metadata: self.snapshot.as_ref().map(|_| self.document()),
            revision: self.snapshot.as_ref().and_then(|s| s.revision),
            error: self.error.clone(),
        }
    }
    pub(super) fn set_document(&mut self, metadata: Metadata) -> Result<(), String> {
        self.draft_set(metadata)
    }
    pub(super) fn draft_set(&mut self, metadata: Metadata) -> Result<(), String> {
        if self.busy() || self.snapshot.is_none() {
            return Err("Metadata is not ready".into());
        }
        self.fields = FIELDS.iter().map(|f| f.text(&metadata)).collect();
        self.tags = normalized_tags(metadata.tags);
        Ok(())
    }
    pub(super) fn discard(&mut self) -> bool {
        if self.busy() {
            return false;
        }
        if let Some(snapshot) = self.snapshot.clone() {
            self.set_snapshot(snapshot);
        }
        true
    }
    pub(super) fn save_request(&mut self) -> Result<MetadataRequest, String> {
        if self.busy() {
            return Err("Metadata request is pending".into());
        }
        let snapshot = self.snapshot.as_ref().ok_or("Metadata is not ready")?;
        if snapshot.image_id.is_some() && !snapshot.binding_current {
            return Err("Original binding changed".into());
        }
        let request = MetadataRequest::Save {
            path: snapshot.path.clone(),
            expected_revision: snapshot.revision,
            metadata: self.document(),
        };
        self.awaiting_submission = true;
        self.error = None;
        Ok(request)
    }
    fn add_tag(&mut self, text: &str) {
        match photo_registry::normalize_tag(text) {
            Ok(tag) if self.tags.contains(&tag) => {
                self.tag_input.clear();
            }
            Ok(tag) if self.tags.len() < 128 => {
                self.tags.push(tag);
                self.tags.sort();
                self.tag_input.clear();
                self.error = None;
            }
            Ok(_) => self.error = Some("Можно выбрать не более 128 тегов".into()),
            Err(_) => self.error = Some("Введите тег: до 128 символов, без перевода строки".into()),
        }
    }
    fn toggle_tag(&mut self, tag: &str) {
        if self.tags.iter().any(|selected| selected == tag) {
            self.tags.retain(|selected| selected != tag);
        } else {
            self.add_tag(tag);
        }
    }
    fn render_tags(&mut self, ui: &mut egui::Ui) {
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                for tag in self.tags.clone() {
                    let label =
                        egui::RichText::new(format!("{tag}  ×")).color(egui::Color32::WHITE);
                    if ui
                        .add(
                            egui::Button::new(label)
                                .fill(tag_color(&tag))
                                .corner_radius(tokens::CONTROL_RADIUS)
                                .wrap(),
                        )
                        .on_hover_text("Удалить тег")
                        .clicked()
                    {
                        self.toggle_tag(&tag);
                    }
                }
            });
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.tag_input)
                    .hint_text("Найти или создать тег…")
                    .desired_width(ui.available_width()),
            );
            if response.gained_focus() || response.changed() {
                self.tag_options_open = true;
            }
            if (response.has_focus() || response.lost_focus())
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                self.add_tag(&self.tag_input.clone());
                response.request_focus();
            }
            if ui
                .selectable_label(
                    self.tag_options_open,
                    if self.tag_options_open {
                        "▴ Скрыть теги"
                    } else {
                        "▾ Выбрать теги"
                    },
                )
                .clicked()
            {
                self.tag_options_open = !self.tag_options_open;
            }
            if self.tag_options_open {
                let query = self.tag_input.trim().to_lowercase();
                let options: Vec<_> = self
                    .snapshot
                    .as_ref()
                    .map(|s| {
                        s.available_tags
                            .iter()
                            .filter(|tag| tag.to_lowercase().contains(&query))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .id_salt("tag-options")
                        .max_height(180.0)
                        .show(ui, |ui| {
                            for tag in &options {
                                let mut selected = self.tags.contains(tag);
                                if ui.checkbox(&mut selected, tag).changed() {
                                    self.toggle_tag(tag);
                                }
                            }
                            let candidate = self.tag_input.trim().to_owned();
                            let exists = self
                                .tags
                                .iter()
                                .chain(options.iter())
                                .any(|tag| tag == &candidate);
                            if !candidate.is_empty() && !exists {
                                if ui
                                    .add(
                                        egui::Button::new(format!("+ Создать «{candidate}»"))
                                            .wrap(),
                                    )
                                    .clicked()
                                {
                                    self.add_tag(&candidate);
                                }
                            } else if options.is_empty() {
                                ui.weak("Введите название нового тега");
                            }
                        });
                });
            }
        });
    }
    pub(super) fn render(&mut self, ui: &mut egui::Ui) -> Vec<Intent> {
        let mut intents = Vec::new();
        if !self.is_open() {
            return intents;
        }
        egui::Panel::right("metadata_panel").default_size(340.0).size_range(220.0..=520.0).resizable(true).show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(tokens::SM, tokens::SM);
            ui.heading("Метаданные");
            if let Some(path) = &self.target { ui.add(egui::Label::new(path.file_name().unwrap_or(path.as_os_str()).to_string_lossy()).wrap()); }
            ui.horizontal_wrapped(|ui| {
                let can_save = !self.busy() && self.is_dirty() && self.snapshot.as_ref().is_some_and(|s| s.image_id.is_none() || s.binding_current);
                if ui.add_enabled(can_save, egui::Button::new("Применить")).clicked() {
                    match self.finish() {
                        Ok(Some(request)) => intents.push(Intent::Request(request)),
                        Ok(None) => {},
                        Err(error) => self.error = Some(error),
                    }
                }
                if ui.add_enabled(!self.busy() && self.is_dirty(), egui::Button::new("Отменить изменения")).clicked() {
                    self.discard();
                }
                if ui.button("Отмена · Esc").on_hover_text("Отменить несохранённые изменения и закрыть панель").clicked() {
                    self.cancel(); intents.push(Intent::Closed);
                }
            });
            if !self.busy() && self.snapshot.is_none() && ui.button("Повторить загрузку").clicked() && let Some(path) = self.target.clone() {
                self.awaiting_submission = true;
                self.error = None;
                intents.push(Intent::Request(MetadataRequest::Inspect { path }));
            }
            if let Some(error) = &self.error { ui_foundation::message(ui, error, true); }
            if self.snapshot.as_ref().is_some_and(|s| s.image_id.is_some() && !s.binding_current) {
                ui_foundation::message(ui, "Исходный файл недоступен или изменился. Сохранение недоступно до восстановления привязки.", true);
            }
            let enabled = !self.busy() && self.snapshot.is_some();
            egui::ScrollArea::vertical().id_salt("metadata_fields").show(ui, |ui| {
                egui::CollapsingHeader::new("Заметки Afio").default_open(true).show(ui, |ui| {
                ui.add_enabled_ui(enabled, |ui| {
                    for (index, spec) in FIELDS.iter().take(self.fields.len()).enumerate() {
                        ui.push_id(spec.key, |ui| {
                            SettingsRow::show(ui, spec.label, spec.description, |ui| {
                                match spec.kind {
                                    FieldKind::Tags => self.render_tags(ui),
                                    FieldKind::Multiline => { ui.add(egui::TextEdit::multiline(&mut self.fields[index]).desired_rows(4).desired_width(ui.available_width())); }
                                }
                            });
                        });
                    }
                });
                });
                if let Some(snapshot) = &self.snapshot {
                    ui.push_id(&snapshot.path, |ui| render_embedded(ui, &snapshot.embedded, &mut self.embedded_filter));
                }
            });
        });
        intents
    }
}

fn metadata_group(namespace: &str) -> &'static str {
    match namespace.to_ascii_lowercase().as_str() {
        "exif" => "EXIF · камера и съёмка",
        "iptc" | "iptc-iim" => "IPTC · автор и описание",
        "icc" | "image" => "Файл и цвет",
        "comfyui" | "generation" => "Генерация · ComfyUI и параметры",
        "png_text" => "Текстовые поля PNG",
        _ => "XMP · встроенные свойства",
    }
}
fn render_embedded(
    ui: &mut egui::Ui,
    data: &photo_core::metadata::EmbeddedMetadata,
    filter: &mut String,
) {
    ui.separator();
    ui.strong("Данные файла");
    ui.weak("Чтение из изображения. Заметки Afio сохраняются отдельно.");
    for warning in &data.warnings {
        ui_foundation::message(ui, warning, false);
    }
    if data.c2pa_present {
        ui.weak(
            "Обнаружен блок сведений о происхождении; подлинность подписи здесь не проверяется.",
        );
    }
    if data.properties.is_empty() && data.raw_xmp.is_empty() && data.raw_payloads.is_empty() {
        ui.weak("Поддерживаемые встроенные поля не найдены.");
        return;
    }
    ui.add(
        egui::TextEdit::singleline(filter)
            .hint_text("Поиск по встроенным полям…")
            .desired_width(ui.available_width()),
    );
    let query = filter.to_lowercase();
    for group in [
        "Файл и цвет",
        "EXIF · камера и съёмка",
        "IPTC · автор и описание",
        "XMP · встроенные свойства",
        "Генерация · ComfyUI и параметры",
        "Текстовые поля PNG",
    ] {
        let fields: Vec<_> = data
            .properties
            .iter()
            .filter(|p| {
                metadata_group(&p.namespace) == group
                    && (query.is_empty()
                        || format!("{} {} {}", p.namespace, p.name, p.value)
                            .to_lowercase()
                            .contains(&query))
            })
            .collect();
        if fields.is_empty() {
            continue;
        }
        egui::CollapsingHeader::new(format!("{group} · {}", fields.len()))
            .id_salt(("embedded-v2", group))
            .default_open(false)
            .show(ui, |ui| {
                for (index, field) in fields.into_iter().enumerate() {
                    ui.push_id((&field.namespace, &field.name, index), |ui| {
                        ui.label(egui::RichText::new(embedded_label(&field.name)).strong())
                            .on_hover_text(format!("{}\n{}", field.namespace, field.name));
                        let preview: String = field.value.chars().take(240).collect();
                        let long = preview.len() < field.value.len();
                        ui.add(
                            egui::Label::new(if long {
                                format!("{preview}…")
                            } else {
                                preview
                            })
                            .selectable(true)
                            .wrap(),
                        );
                        if long {
                            egui::CollapsingHeader::new("Показать значение")
                                .default_open(false)
                                .show(ui, |ui| {
                                    code_metadata(ui, &field.value);
                                });
                        }
                        ui.add_space(tokens::XS);
                    });
                }
            });
    }
    for (index, packet) in data.raw_xmp.iter().enumerate() {
        raw_metadata(ui, ("xmp", index), "Исходный XMP", packet);
    }
    for (index, payload) in data.raw_payloads.iter().enumerate() {
        raw_metadata(ui, (&payload.kind, index), &payload.label, &payload.text);
    }
}
fn embedded_label(name: &str) -> String {
    let mut hidden = false;
    name.chars()
        .filter(|c| match c {
            '{' => {
                hidden = true;
                false
            }
            '}' => {
                hidden = false;
                false
            }
            _ => !hidden,
        })
        .collect::<String>()
        .trim_start_matches('/')
        .replace("[0]", "")
        .split('/')
        .filter(|p| {
            !matches!(
                *p,
                "xmpmeta" | "RDF" | "Description" | "Seq" | "Bag" | "Alt"
            )
        })
        .map(|p| p.trim_start_matches('@'))
        .collect::<Vec<_>>()
        .join(" / ")
}
fn raw_metadata(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    text: &str,
) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .corner_radius(tokens::CONTROL_RADIUS)
        .inner_margin(tokens::SM as i8)
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong(label);
                ui.weak(format!("{} КБ", text.len().div_ceil(1024)));
                if ui.button("Скопировать полностью").clicked() {
                    copy_metadata(ui.ctx(), text);
                }
            });
            egui::CollapsingHeader::new("Посмотреть исходник")
                .id_salt(("raw-v2", &id))
                .default_open(false)
                .show(ui, |ui| {
                    let expanded_id = ui.make_persistent_id(("raw-full", &id));
                    let mut expanded =
                        ui.data(|d| d.get_temp::<bool>(expanded_id).unwrap_or(false));
                    let preview: String = text.chars().take(1024).collect();
                    code_metadata(ui, if expanded { text } else { &preview });
                    if preview.len() < text.len() {
                        ui.weak(if expanded {
                            "Показан весь исходник. Прокрутка внутри блока."
                        } else {
                            "… Есть продолжение. Кнопка копирования берёт весь исходник."
                        });
                        if ui
                            .button(if expanded {
                                "Свернуть до начала"
                            } else {
                                "Показать всё"
                            })
                            .clicked()
                        {
                            expanded = !expanded;
                            ui.data_mut(|d| d.insert_temp(expanded_id, expanded));
                        }
                    }
                });
        });
}
fn copy_metadata(ctx: &egui::Context, text: &str) {
    ctx.copy_text(text.to_owned());
    crate::copy_feedback::copied(ctx, "Метаданные скопированы");
}
/// Bounded visual rows let even a single-line megabyte JSON render by viewport,
/// without laying out the entire document. Raw clipboard contents stay exact.
fn code_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut columns = 0;
    for (offset, c) in text.char_indices() {
        if c == '\n' || columns == 120 {
            ranges.push(start..offset);
            start = if c == '\n' { offset + 1 } else { offset };
            columns = 0;
        }
        if c != '\n' {
            columns += 1;
        }
    }
    if start < text.len() {
        ranges.push(start..text.len());
    }
    ranges
}
fn code_metadata(ui: &mut egui::Ui, text: &str) {
    let rows = code_ranges(text);
    let height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::both()
        .id_salt("metadata-code")
        .max_height(220.0)
        .show_rows(ui, height, rows.len(), |ui, visible| {
            for index in visible {
                ui.add(
                    egui::Label::new(egui::RichText::new(&text[rows[index].clone()]).monospace())
                        .selectable(true)
                        .wrap_mode(egui::TextWrapMode::Extend),
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_closes_dirty_or_loading_panel_and_ignores_late_receipt() {
        let mut panel = Panel::new();
        panel.target = Some("a.jpg".into());
        panel.set_snapshot(snapshot("a.jpg"));
        panel.fields[1] = "Unsaved draft".into();
        panel.cancel();
        assert!(!panel.is_open() && !panel.is_dirty());
        panel.open("a.jpg".into());
        let pending = MetadataJob {
            request_id: 42,
            path: "a.jpg".into(),
            pending: true,
            snapshot: None,
            error: None,
        };
        panel.attach_request(&pending);
        panel.cancel();
        panel.apply_job(&MetadataJob {
            pending: false,
            snapshot: Some(snapshot("a.jpg")),
            ..pending
        });
        assert!(!panel.is_open() && panel.snapshot.is_none());
        assert!(panel.open("b.jpg".into()).is_some());
    }
    #[test]
    fn virtual_code_rows_are_unicode_safe_and_copy_keeps_the_entire_document() {
        let text = format!("{{\"workflow\":\"{}\"}}\n", "Тест🗿".repeat(20_000));
        let rows = code_ranges(&text);
        assert!(rows.iter().all(|r| text[r.clone()].chars().count() <= 120));
        let visible: String = rows.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(visible, text.replace('\n', ""));
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| copy_metadata(ui.ctx(), &text));
        assert!(output.platform_output.commands.iter().any(
            |command| matches!(command, egui::OutputCommand::CopyText(copied) if copied == &text)
        ));
        output.textures_delta.clear();
    }
    #[test]
    fn finish_without_changes_closes_without_a_save_request() {
        let mut panel = Panel::new();
        panel.target = Some("a.jpg".into());
        panel.set_snapshot(snapshot("a.jpg"));
        assert!(panel.finish().unwrap().is_none());
        assert!(!panel.is_open());
    }
    #[test]
    fn finish_waits_for_matching_save_receipt_and_preserves_failed_draft() {
        let mut panel = Panel::new();
        panel.target = Some("a.jpg".into());
        panel.set_snapshot(snapshot("a.jpg"));
        panel.fields[1] = "Changed".into();
        assert!(panel.finish().unwrap().is_some());
        assert!(panel.is_open());
        panel.submission_failed("Cannot save".into());
        assert!(panel.is_open() && panel.is_dirty() && !panel.close_after_save);
        assert!(panel.finish().unwrap().is_some());
        let mut saved = snapshot("a.jpg");
        saved.metadata.comment = "Changed".into();
        let pending = MetadataJob {
            request_id: 42,
            path: "a.jpg".into(),
            pending: true,
            snapshot: None,
            error: None,
        };
        panel.attach_request(&pending);
        panel.apply_job(&MetadataJob {
            pending: false,
            snapshot: Some(saved),
            ..pending
        });
        assert!(!panel.is_open());
    }
    #[test]
    fn loading_panel_renders_before_async_snapshot_arrives() {
        let ctx = egui::Context::default();
        let mut panel = Panel::new();
        assert!(panel.open("a.jpg".into()).is_some());
        assert!(panel.fields.is_empty());
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert!(panel.render(ui).is_empty());
        });
        output.textures_delta.clear();
        assert!(panel.panel_state().pending);
        assert!(panel.panel_state().metadata.is_none());
    }
    #[test]
    fn embedded_fields_are_read_only_and_fit_localized_panel_widths() {
        for width in [320.0, 800.0, 1200.0] {
            for ppp in [1.0, 1.5, 2.0] {
                let ctx = egui::Context::default();
                let mut panel = Panel::new();
                panel.target = Some("synthetic.jpg".into());
                let mut snapshot = snapshot("synthetic.jpg");
                for ns in [
                    "EXIF",
                    "IPTC-IIM",
                    "ICC",
                    "comfyui",
                    "http://ns.adobe.com/photoshop/1.0/",
                ] {
                    snapshot.embedded.properties.push(photo_core::metadata::EmbeddedProperty {
                        namespace: ns.into(), name: "Описание изображения и параметры генерации".into(), value: "Длинное значение для проверки переноса и чтения без изменения оригинала".repeat(3),
                    });
                }
                panel.set_snapshot(snapshot);
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(ppp);
                let mut output = ctx.run_ui(input, |ui| {
                    assert!(panel.render(ui).is_empty());
                    assert!(ui.min_rect().right() <= width + 1.0);
                });
                output.textures_delta.clear();
                assert!(!panel.is_dirty());
            }
        }
    }
    #[test]
    fn tag_chips_normalize_deduplicate_and_toggle_without_touching_hidden_text() {
        let mut panel = Panel::new();
        panel.target = Some("a.jpg".into());
        let mut initial = snapshot("a.jpg");
        initial.metadata.caption = "Stored caption".into();
        initial.metadata.prompt = "Stored prompt".into();
        panel.set_snapshot(initial);
        panel.add_tag("  Cafe\u{301}  ");
        panel.add_tag("Café");
        panel.add_tag("Русский тег");
        assert_eq!(panel.tags, vec!["Café", "Русский тег"]);
        panel.toggle_tag("Café");
        assert_eq!(panel.document().tags, vec!["Русский тег"]);
        panel
            .draft_set(Metadata {
                comment: "New comment".into(),
                tags: vec![" test ".into(), "test".into()],
                ..Default::default()
            })
            .unwrap();
        let document = panel.document();
        assert_eq!(document.tags, vec!["test"]);
        assert_eq!(document.caption, "Stored caption");
        assert_eq!(document.prompt, "Stored prompt");
        assert_eq!(document.comment, "New comment");
        assert_eq!(
            FIELDS.iter().map(|f| f.key).collect::<Vec<_>>(),
            vec!["tags", "comment", "description"]
        );
    }
    #[test]
    fn polling_failure_releases_pending_without_losing_draft() {
        let mut panel = Panel::new();
        panel.target = Some("a.jpg".into());
        panel.set_snapshot(snapshot("a.jpg"));
        panel.fields[1] = "Несохранённый комментарий".into();
        panel.pending = Some(42);
        panel.awaiting_submission = true;
        panel.request_failed("Expired receipt".into());
        assert!(!panel.busy());
        assert!(panel.is_dirty());
        assert_eq!(panel.document().comment, "Несохранённый комментарий");
        assert_eq!(panel.target(), Some(Path::new("a.jpg")));
        assert!(panel.panel_state().error.unwrap().contains("неизвестен"));
    }
    #[test]
    fn localized_panel_fits_narrow_and_normal_frames() {
        for width in [320.0, 800.0, 1200.0] {
            for ppp in [1.0, 1.5, 2.0] {
                let ctx = egui::Context::default();
                let mut panel = Panel::new();
                panel.target = Some("Очень длинное русское название изображения.jpg".into());
                let mut initial = snapshot("Очень длинное русское название изображения.jpg");
                initial.metadata.tags = vec![
                    "Очень длинный русский тег для проверки переноса".into(),
                    "Портрет".into(),
                ];
                initial.available_tags = vec![
                    "Очень длинный русский тег для проверки переноса".into(),
                    "Пейзаж".into(),
                ];
                panel.set_snapshot(initial);
                panel.tag_options_open = true;
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(ppp);
                let mut output = ctx.run_ui(input, |ui| {
                    assert!(panel.render(ui).is_empty());
                    assert!(ui.available_rect_before_wrap().min.x >= 0.0);
                    assert!(ui.available_rect_before_wrap().max.x <= width + 1.0);
                });
                output.textures_delta.clear();
            }
        }
    }
    #[test]
    fn new_records_can_save_and_registered_stale_records_cannot() {
        let mut panel = Panel::new();
        panel.target = Some("a.jpg".into());
        let mut initial = snapshot("a.jpg");
        initial.binding_current = false;
        panel.set_snapshot(initial.clone());
        assert!(panel.save_request().is_ok());
        panel.submission_failed("test".into());
        initial.image_id = Some("registered".into());
        panel.set_snapshot(initial);
        assert!(panel.save_request().is_err());
    }
    fn snapshot(path: &str) -> MetadataSnapshot {
        MetadataSnapshot {
            embedded: Default::default(),
            path: path.into(),
            image_id: None,
            revision: None,
            binding_current: true,
            metadata: Metadata::default(),
            available_tags: Vec::new(),
        }
    }
    #[test]
    fn dirty_switch_and_close_retain_draft() {
        let mut panel = Panel::new();
        panel.target = Some("a.jpg".into());
        panel.set_snapshot(snapshot("a.jpg"));
        panel.fields[1] = "Очень длинный русский комментарий".into();
        assert!(panel.open("b.jpg".into()).is_none());
        assert!(!panel.close());
        assert_eq!(panel.target(), Some(Path::new("a.jpg")));
        assert!(panel.is_dirty());
    }
    #[test]
    fn completions_require_receipt_and_target() {
        let mut panel = Panel::new();
        panel.open("a.jpg".into());
        let mut job = MetadataJob {
            request_id: 1,
            path: "a.jpg".into(),
            pending: true,
            snapshot: None,
            error: None,
        };
        panel.attach_request(&job);
        job.pending = false;
        job.snapshot = Some(snapshot("a.jpg"));
        job.request_id = 2;
        panel.apply_job(&job);
        assert!(panel.snapshot.is_none());
        job.request_id = 1;
        job.path = "b.jpg".into();
        panel.apply_job(&job);
        assert!(panel.snapshot.is_none());
        job.path = "a.jpg".into();
        panel.apply_job(&job);
        assert!(panel.snapshot.is_some());
    }
}
