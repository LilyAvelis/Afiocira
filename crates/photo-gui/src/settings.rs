//! Native settings viewport. Modules own their content; this shell owns navigation.
use super::*;
use crate::ui_foundation::tokens as ui_tokens;
use serde_json::{Value, json};

pub(super) fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("afio-settings")
}

pub(super) struct State {
    pub shortcuts: Value,
    pub gallery_scale: Option<f32>,
    pub gallery_auto_size: bool,
    pub copy_notifications: bool,
    pub recording: Option<(photo_core::actions::ActionId, String)>,
    pub shortcut_candidate: Option<(photo_core::actions::ActionId, String, String)>,
    pub shortcut_filter: String,
    pub open: bool,
    pub text_size: f32,
    query: String,
    selected: &'static str,
    previous_focus: Option<egui::Id>,
    focus_search: bool,
    pub error: Option<String>,
}

struct Module {
    id: &'static str,
    label: &'static str,
    keywords: &'static str,
    icon: Lucide,
    render: fn(&mut Workshop, &mut egui::Ui),
}

const MODULES: &[Module] = &[
    Module {
        id: "afio",
        label: "Afio",
        keywords: "shortcuts keyboard gallery scale copy notifications клавиши масштаб копирование уведомления",
        icon: Lucide::Keyboard,
        render: Workshop::afio_settings,
    },
    Module {
        id: "yazi",
        label: "Yazi",
        keywords: "appearance text font size themes icons quicklinks keybindings general оформление темы иконки переходы клавиши",
        icon: Lucide::FolderOpen,
        render: Workshop::yazi_settings,
    },
];

impl State {
    pub fn load() -> Self {
        let mut state = Self {
            shortcuts: json!({}),
            gallery_scale: None,
            gallery_auto_size: true,
            copy_notifications: true,
            recording: None,
            shortcut_candidate: None,
            shortcut_filter: String::new(),
            open: false,
            text_size: 12.0,
            query: String::new(),
            selected: "yazi",
            previous_focus: None,
            focus_search: false,
            error: None,
        };
        match config_path().and_then(|path| read_config(&path)) {
            Ok(document) => {
                state.copy_notifications = copy_notifications(&document);
                state.shortcuts = document
                    .get("shortcuts")
                    .filter(|v| v.is_object())
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                state.gallery_scale = document
                    .get("gallery_scale")
                    .and_then(Value::as_f64)
                    .map(|v| v as f32)
                    .filter(|v| v.is_finite() && (0.4..=2.5).contains(v));
                state.gallery_auto_size = document
                    .get("gallery_auto_size")
                    .and_then(Value::as_bool)
                    .unwrap_or(state.gallery_scale.is_none());
                if let Some(size) = document.get("yazi_text_size").and_then(Value::as_f64) {
                    if valid_size(size as f32) {
                        state.text_size = size as f32;
                    } else {
                        state.error = Some("Сохранённый размер текста вне диапазона 10–24.".into());
                    }
                }
            }
            Err(error) => state.error = Some(error),
        }
        state
    }
    pub fn snapshot(&self) -> photo_core::api::SettingsState {
        photo_core::api::SettingsState {
            open: self.open,
            section: self.selected.into(),
            page: None,
            yazi_text_size: self.text_size,
            error: self.error.clone(),
            yazi: None,
            shortcuts: crate::shortcuts::catalogue(&self.shortcuts),
            gallery_scale: self.gallery_scale,
            gallery_auto_size: self.gallery_auto_size,
            copy_notifications: self.copy_notifications,
        }
    }
    pub fn effective_gallery_scale(&self) -> Option<f32> {
        (!self.gallery_auto_size).then_some(self.gallery_scale.unwrap_or(1.0))
    }
}

pub(super) fn config_path() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("AFIO_SETTINGS_HOME") {
        return Ok(PathBuf::from(root).join("settings.json"));
    }
    std::env::var_os("APPDATA")
        .map(|root| PathBuf::from(root).join("Afiocira/settings.json"))
        .ok_or_else(|| "Не найдена папка локальных настроек APPDATA.".into())
}

pub(super) fn read_config(path: &Path) -> Result<Value, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(json!({"version":1}));
        }
        Err(error) => return Err(error.to_string()),
    };
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Настройки не прочитаны: {e}"))?;
    if !document.is_object() || document.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Неизвестная версия настроек. Файл оставлен без изменений.".into());
    }
    Ok(document)
}

fn valid_size(size: f32) -> bool {
    size.is_finite() && (10.0..=24.0).contains(&size)
}

fn save_size(path: &Path, size: f32) -> Result<(), String> {
    if !valid_size(size) {
        return Err("Размер текста должен быть от 10 до 24.".into());
    }
    update_config(path, |document| {
        document["yazi_text_size"] = json!(size);
        Ok(())
    })
}

pub(super) fn save_property(key: &str, value: Value) -> Result<(), String> {
    update_config(&config_path()?, |document| {
        document[key] = value;
        Ok(())
    })
}

fn copy_notifications(document: &Value) -> bool {
    document
        .get("copy_notifications")
        .and_then(Value::as_bool)
        .unwrap_or(true)
}

impl Workshop {
    pub(super) fn set_copy_notifications(&mut self, enabled: bool) -> Result<(), String> {
        if let Err(error) = save_property("copy_notifications", json!(enabled)) {
            self.settings.error = Some(error.clone());
            return Err(error);
        }
        self.settings.copy_notifications = enabled;
        crate::copy_feedback::configure(&self.ctx, enabled);
        Ok(())
    }
}

/// All writers reread under this OS lock, which is released even after a crash.
/// Immediate field writes use latest-writer wins; unrelated fields are retained.
pub(super) fn update_config<T>(
    path: &Path,
    mutate: impl FnOnce(&mut Value) -> Result<T, String>,
) -> Result<T, String> {
    std::fs::create_dir_all(path.parent().ok_or("No settings directory")?)
        .map_err(|e| e.to_string())?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path.with_extension("json.lock"))
        .map_err(|e| format!("Settings lock could not be opened: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(20);
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(1))
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(
                    "Settings are busy in another instance; retry the change (20 ms lock timeout)."
                        .into(),
                );
            }
            Err(error) => return Err(format!("Settings lock failed: {error}")),
        }
    }
    let mut document = read_config(path)?;
    let result = mutate(&mut document)?;
    if !document.is_object() || document.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("Invalid settings document; original file preserved.".into());
    }
    save_document(path, &document)?;
    Ok(result)
}

fn save_document(path: &Path, document: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(path.parent().ok_or("Нет папки настроек")?)
        .map_err(|e| e.to_string())?;
    let temporary = path.with_extension(format!(
        "{}-{}.next",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    ));
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // Windows rename replaces an existing file atomically; preserve the last valid version.
        if path.exists() {
            std::fs::copy(path, path.with_extension("json.previous"))?;
        }
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Настройка не сохранена: {e}"))
}

impl Workshop {
    pub(super) fn select_settings(
        &mut self,
        section: &str,
        page: Option<&str>,
    ) -> Result<(), String> {
        let module = MODULES
            .iter()
            .find(|m| m.id == section)
            .ok_or("Unknown Settings module")?;
        let selected_page = if section == "yazi" {
            Some(
                ["Appearance", "Quicklinks", "Keybindings", "General"]
                    .into_iter()
                    .find(|p| Some(*p) == page)
                    .or_else(|| {
                        if page.is_none() {
                            Some(self.yazi_settings.page)
                        } else {
                            None
                        }
                    })
                    .ok_or("Unknown Yazi settings page")?,
            )
        } else {
            if page.is_some_and(|p| p != "Shortcuts") {
                return Err("Unknown Afio settings page".into());
            }
            None
        };
        self.open_settings()?;
        if module.id != "yazi" || selected_page != Some("Keybindings") {
            crate::yazi_key_capture::cancel(&self.ctx, &mut self.yazi_settings.binding_keys);
        }
        self.settings.selected = module.id;
        self.settings.recording = None;
        if let Some(page) = selected_page {
            self.yazi_settings.page = page;
        }
        Ok(())
    }
    pub(super) fn open_settings(&mut self) -> Result<(), String> {
        if let Err(error) = self.inspect_yazi_settings() {
            self.yazi_settings.view.error = Some(error);
        }
        if self.settings.open {
            self.ctx
                .send_viewport_cmd_to(viewport_id(), egui::ViewportCommand::Focus);
            return Ok(());
        }
        self.settings.previous_focus = self.ctx.memory(|m| m.focused());
        self.settings.focus_search = true;
        self.settings.open = true;
        self.ctx
            .send_viewport_cmd_to(viewport_id(), egui::ViewportCommand::Focus);
        Ok(())
    }

    pub(super) fn close_settings(&mut self) {
        if !self.settings.open {
            return;
        }
        self.settings.open = false;
        crate::yazi_key_capture::cancel(&self.ctx, &mut self.yazi_settings.binding_keys);
        self.settings.recording = None;
        self.ctx
            .send_viewport_cmd_to(viewport_id(), egui::ViewportCommand::Close);
        self.ctx
            .send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        if let Some(id) = self.settings.previous_focus.take() {
            self.ctx.memory_mut(|m| m.request_focus(id));
        } else if self.active_pane == ActivePane::Yazi
            && let Some(yazi) = &mut self.yazi
        {
            yazi.request_focus();
        }
    }

    pub(super) fn set_yazi_text_size(&mut self, size: f32) -> Result<(), String> {
        let result = config_path().and_then(|path| save_size(&path, size));
        self.settings.error = result.as_ref().err().cloned();
        result?;
        self.settings.text_size = size;
        Ok(())
    }

    pub(super) fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings.open {
            return;
        }
        let mut close = false;
        ctx.show_viewport_immediate(
            viewport_id(),
            egui::ViewportBuilder::default()
                .with_title("Afiocira Settings")
                .with_inner_size([980.0, 680.0])
                .with_min_inner_size([680.0, 460.0]),
            |ctx, _class| {
                frame_capture::poll(ctx);
                crate::yazi_key_capture::poll(
                    ctx,
                    self.settings.selected == "yazi"
                        && self.yazi_settings.page == "Keybindings"
                        && self.yazi_settings.binding_edit.is_some(),
                );
                self.capture_shortcut(ctx);
                close = ctx.input(|i| i.viewport().close_requested())
                    || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
                if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, Key::Comma)) {
                    self.settings.focus_search = true;
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(Color32::from_gray(69)))
                    .show(ctx, |ui| {
                        egui::Panel::left("settings-sidebar")
                            .exact_size(215.0)
                            .resizable(false)
                            .frame(
                                egui::Frame::new()
                                    .fill(Color32::from_gray(65))
                                    .inner_margin(16),
                            )
                            .show(ui, |ui| {
                                ui.add_space(ui_tokens::SM);
                                ui.label(egui::RichText::new("Settings").size(19.0).strong());
                                ui.add_space(ui_tokens::XL);
                                let search = ui.add_sized(
                                    [ui.available_width(), 34.0],
                                    egui::TextEdit::singleline(&mut self.settings.query)
                                        .hint_text("Search settings…")
                                        .margin(egui::vec2(10.0, 9.0)),
                                );
                                if self.settings.focus_search {
                                    search.request_focus();
                                    self.settings.focus_search = false;
                                }
                                ui.add_space(ui_tokens::XL);
                                ui.label(
                                    egui::RichText::new("WORKSPACE")
                                        .size(10.0)
                                        .color(Color32::from_gray(173)),
                                );
                                ui.add_space(ui_tokens::SM);
                                let query = self.settings.query.trim().to_lowercase();
                                let mut matches = 0;
                                for module in MODULES {
                                    if !query.is_empty()
                                        && !format!("{} {}", module.label, module.keywords)
                                            .to_lowercase()
                                            .contains(&query)
                                    {
                                        continue;
                                    }
                                    matches += 1;
                                    if ui
                                        .add_sized(
                                            [ui.available_width(), 38.0],
                                            egui::Button::image_and_text(
                                                module.icon.size(20.0).image(),
                                                module.label,
                                            )
                                            .selected(self.settings.selected == module.id),
                                        )
                                        .clicked()
                                    {
                                        self.execute(Command::SelectSettings {
                                            section: module.id.into(),
                                            page: None,
                                        });
                                    }
                                }
                                if matches == 0 {
                                    crate::ui_foundation::message(ui, "Ничего не найдено", false);
                                }
                                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                                    ui.weak(format!("Afiocira · {}", env!("CARGO_PKG_VERSION")));
                                });
                            });
                        if self.settings.selected == "yazi" {
                            egui::Panel::bottom("settings-yazi-apply")
                                .resizable(false)
                                .frame(
                                    egui::Frame::new()
                                        .fill(Color32::from_gray(65))
                                        .inner_margin(egui::Margin::symmetric(
                                            crate::ui_foundation::tokens::XL as i8,
                                            crate::ui_foundation::tokens::MD as i8,
                                        )),
                                )
                                .show(ui, |ui| {
                                    let busy = self.yazi_settings.busy();
                                    let dirty = self.yazi_settings.view.dirty;
                                    let mut request = None;
                                    ui.horizontal_wrapped(|ui| {
                                        if ui
                                            .add_enabled(
                                                !busy && dirty,
                                                egui::Button::new("Применить к Yazi"),
                                            )
                                            .clicked()
                                        {
                                            request =
                                                Some(photo_core::settings::YaziRequest::Apply);
                                        }
                                        if ui
                                            .add_enabled(
                                                !busy && dirty,
                                                egui::Button::new("Отменить изменения"),
                                            )
                                            .clicked()
                                        {
                                            request =
                                                Some(photo_core::settings::YaziRequest::Discard);
                                        }
                                        if dirty {
                                            ui.weak("Есть неприменённые изменения");
                                        } else {
                                            ui.weak("Изменения применены");
                                        }
                                    });
                                    if !self.yazi_settings.view.message.is_empty() {
                                        ui.label(&self.yazi_settings.view.message);
                                    }
                                    if let Some(error) = &self.yazi_settings.view.error {
                                        crate::ui_foundation::message(ui, error, true);
                                    }
                                    if let Some(request) = request {
                                        self.execute(Command::ConfigureYazi { request });
                                    }
                                });
                        }
                        egui::ScrollArea::vertical()
                            .id_salt((
                                "settings-content",
                                self.settings.selected,
                                self.yazi_settings.page,
                            ))
                            .show(ui, |ui| {
                                egui::Frame::new()
                                    .inner_margin(crate::ui_foundation::tokens::XL as i8)
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        if let Some(module) =
                                            MODULES.iter().find(|m| m.id == self.settings.selected)
                                        {
                                            (module.render)(self, ui);
                                        }
                                        if let Some(error) = &self.settings.error {
                                            ui.add_space(ui_tokens::MD);
                                            crate::ui_foundation::message(ui, error, true);
                                        }
                                    });
                                crate::scroll_navigation::apply(ui);
                            });
                    });
            },
        );
        if close {
            self.close_settings();
        }
    }
}

/// Keep setting controls on a shared right edge, stacking them on narrow panes.
pub(super) fn row(
    ui: &mut egui::Ui,
    title: &str,
    description: &str,
    controls_width: f32,
    controls: impl FnOnce(&mut egui::Ui),
) {
    let _ = controls_width; // Compatibility argument: the shared column owns geometry.
    crate::ui_foundation::SettingsRow::show(ui, title, description, controls);
}

pub(super) fn card(ui: &mut egui::Ui, content: impl FnOnce(&mut egui::Ui)) {
    crate::ui_foundation::SettingsSection::show(ui, content);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persistence_child_writer() {
        let Some(path) = std::env::var_os("AFIO_TEST_SETTINGS_PATH") else {
            return;
        };
        let key = std::env::var("AFIO_TEST_SETTINGS_KEY").unwrap();
        let path = PathBuf::from(path);
        for _ in 0..100 {
            match update_config(&path, |document| {
                document[&key] = json!(key);
                Ok(())
            }) {
                Ok(()) => return,
                Err(error) if error.contains("busy in another instance") => {}
                Err(error) => panic!("{error}"),
            }
        }
        panic!("settings remained busy");
    }

    #[test]
    fn simultaneous_processes_preserve_top_level_fields() {
        let root = std::env::temp_dir().join(format!(
            "afio-settings-process-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        let exe = std::env::current_exe().unwrap();
        let mut children = ["first", "second"].map(|key| {
            std::process::Command::new(&exe)
                .args(["--exact", "settings::tests::persistence_child_writer"])
                .env("AFIO_TEST_SETTINGS_PATH", &path)
                .env("AFIO_TEST_SETTINGS_KEY", key)
                .spawn()
                .unwrap()
        });
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let document = read_config(&path).unwrap();
        assert_eq!(document["first"], "first");
        assert_eq!(document["second"], "second");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lock_timeout_is_explicit_and_released_on_close() {
        let root = std::env::temp_dir().join(format!("afio-settings-lock-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path.with_extension("json.lock"))
            .unwrap();
        lock.lock().unwrap();
        assert!(
            update_config(&path, |_| Ok(()))
                .unwrap_err()
                .contains("20 ms lock timeout")
        );
        drop(lock);
        update_config(&path, |_| Ok(())).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn save_preserves_unknown_fields_and_rejects_future_or_broken_config() {
        let root = std::env::temp_dir().join(format!(
            "afio-settings-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        std::fs::write(&path, r#"{"version":1,"future":{"keep":true}}"#).unwrap();
        save_size(&path, 18.0).unwrap();
        let saved = read_config(&path).unwrap();
        assert_eq!(saved["future"]["keep"], true);
        assert_eq!(saved["yazi_text_size"], 18.0);
        for text in ["{broken", r#"{"version":2}"#] {
            std::fs::write(&path, text).unwrap();
            assert!(save_size(&path, 14.0).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
        assert!(save_size(&path, f32::NAN).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn copy_notification_preference_defaults_on_and_persists_without_replacing_settings() {
        assert!(copy_notifications(&json!({"version": 1})));
        let root = std::env::temp_dir().join(format!(
            "afio-copy-notifications-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("settings.json");
        std::fs::write(&path, r#"{"version":1,"shortcuts":{"keep":true}}"#).unwrap();
        update_config(&path, |document| {
            document["copy_notifications"] = json!(false);
            Ok(())
        })
        .unwrap();
        let saved = read_config(&path).unwrap();
        assert!(!copy_notifications(&saved));
        assert_eq!(saved["shortcuts"]["keep"], true);
        std::fs::write(&path, "{broken").unwrap();
        assert!(
            update_config(&path, |document| {
                document["copy_notifications"] = json!(true);
                Ok(())
            })
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{broken");
        std::fs::remove_dir_all(root).unwrap();
    }
}
