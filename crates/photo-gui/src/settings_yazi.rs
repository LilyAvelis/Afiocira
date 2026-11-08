//! Raycast-style Yazi settings pages; all mutations use ConfigureYazi.
use super::*;
use crate::ui_foundation::tokens as ui_tokens;
use photo_core::settings::{Quicklink, YaziBinding, YaziRequest, YaziThemeMode};

impl Workshop {
    pub(super) fn yazi_settings(&mut self, ui: &mut egui::Ui) {
        let mut request = None;
        ui.horizontal_wrapped(|ui| {
            ui.add(Lucide::FolderOpen.size(27.0).image());
            ui.heading("Yazi");
        });
        ui.weak("Настоящий файловый менеджер. Ваши настройки — внутри Afio.");
        ui.add_space(ui_tokens::LG);
        ui.horizontal_wrapped(|ui| {
            for page in ["Appearance", "Quicklinks", "Keybindings", "General"] {
                if ui
                    .selectable_label(self.yazi_settings.page == page, page)
                    .clicked()
                {
                    self.execute(Command::SelectSettings {
                        section: "yazi".into(),
                        page: Some(page.into()),
                    });
                }
            }
        });
        ui.add_space(ui_tokens::XL);
        let busy = self.yazi_settings.busy();
        ui.add_enabled_ui(!busy, |ui| match self.yazi_settings.page {
            "Quicklinks" => self.yazi_quicklinks_page(ui, &mut request),
            "Keybindings" => self.yazi_keybindings_page(ui, &mut request),
            "General" => self.yazi_general_page(ui, &mut request),
            _ => self.yazi_appearance_page(ui, &mut request),
        });
        ui.add_space(ui_tokens::SM);
        ui.collapsing("Конфигурация", |ui| {
            ui.weak(&self.yazi_settings.view.version);
            if let Some(path) = &self.yazi_settings.view.config_directory {
                ui.label(friendly_path(path));
                ui.weak("yazi.toml · keymap.toml · theme.toml — конфигурация живого сеанса");
                if ui.button("Скопировать путь").clicked() { ui.ctx().copy_text(friendly_path(path)); }
            }
            ui.weak("Переопределения Afio хранятся отдельно. Исходные TOML, комментарии и плагины сохраняются.");
        });
        if let Some(request) = request {
            self.execute(Command::ConfigureYazi { request });
        }
    }
    fn yazi_appearance_page(&mut self, ui: &mut egui::Ui, request: &mut Option<YaziRequest>) {
        settings::card(ui, |ui| {
            settings::row(
                ui,
                "Размер текста",
                "Сохраняется и применяется сразу",
                250.0,
                |ui| {
                    let mut size = self.settings.text_size;
                    if ui
                        .add(
                            egui::DragValue::new(&mut size)
                                .range(10.0..=24.0)
                                .speed(0.2)
                                .suffix(" pt"),
                        )
                        .changed()
                    {
                        self.execute(Command::SetYaziTextSize { size });
                    }
                    if ui.button("Сбросить").clicked() {
                        self.execute(Command::SetYaziTextSize { size: 12.0 });
                    }
                },
            );
            ui.add_space(ui_tokens::SM);
            ui.separator();
            ui.add_space(ui_tokens::SM);
            ui.label(if self.nerd_font_loaded {
                "JetBrains Mono · Nerd Font"
            } else {
                "Monospace · Nerd Font не найден"
            });
        });
        ui.add_space(ui_tokens::XL);
        ui.label(egui::RichText::new("Темы оформления").strong());
        ui.add_space(ui_tokens::SM);
        let active_mode = self
            .yazi
            .as_ref()
            .map(YaziPane::theme_mode)
            .unwrap_or_default();
        settings::card(ui, |ui| {
            settings::row(
                ui,
                "Режим Yazi",
                "Меняет только Yazi. Его сеанс будет заменён с сохранением контекста; Afio остаётся тёмным.",
                250.0,
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        for (mode, label) in [
                            (YaziThemeMode::Dark, "Тёмный"),
                            (YaziThemeMode::Light, "Светлый"),
                        ] {
                            if ui.selectable_label(active_mode == mode, label).clicked()
                                && active_mode != mode
                            {
                                *request = Some(YaziRequest::ApplyThemeMode { mode });
                            }
                        }
                    });
                },
            );
        });
        ui.add_space(ui_tokens::SM);
        let profile = &self.yazi_settings.view.profile;
        let mut dark = profile.dark_flavor.clone();
        let mut light = profile.light_flavor.clone();
        let mut icons = profile.icons.clone();
        let original = (dark.clone(), light.clone(), icons.clone());
        let names: Vec<_> = self
            .yazi_settings
            .view
            .flavors
            .iter()
            .filter(|f| f.compatible)
            .map(|f| f.name.clone())
            .collect();
        settings::card(ui, |ui| {
            for (key, label, selected) in [
                ("dark", "Тёмная тема", &mut dark),
                ("light", "Светлая тема", &mut light),
            ] {
                settings::row(ui, label, "", 250.0, |ui| {
                    egui::ComboBox::from_id_salt(key)
                        .width(ui.available_width().min(240.0))
                        .truncate()
                        .selected_text(
                            selected
                                .as_ref()
                                .map(|s| {
                                    if s.is_empty() {
                                        "Yazi default"
                                    } else {
                                        s.as_str()
                                    }
                                })
                                .unwrap_or("Наследовать из Yazi"),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(selected, None, "Наследовать из Yazi");
                            ui.selectable_value(selected, Some(String::new()), "Yazi default");
                            for name in &names {
                                ui.selectable_value(selected, Some(name.clone()), name);
                            }
                        });
                });
            }
            ui.separator();
            settings::row(ui, "Иконки", "", 250.0, |ui| {
                egui::ComboBox::from_id_salt("yazi-icons")
                    .width(ui.available_width().min(240.0))
                    .truncate()
                    .selected_text(if icons.as_deref() == Some("minimal") {
                        "Minimal · ▸ ·"
                    } else {
                        "Из темы · Nerd Font"
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut icons, None, "Из темы · Nerd Font");
                        ui.selectable_value(
                            &mut icons,
                            Some("minimal".into()),
                            "Minimal · без Nerd Font",
                        );
                    });
            });
            ui.weak("Индивидуальные правила темы сохраняются при переключении flavor.");
        });
        if original != (dark.clone(), light.clone(), icons.clone()) {
            *request = Some(YaziRequest::SetAppearance {
                dark: dark.clone(),
                light: light.clone(),
                icons: icons.clone(),
            });
        }
        ui.add_space(ui_tokens::SM);
        let flavors = self.yazi_settings.view.flavors.clone();
        let columns = ((ui.available_width() + 12.0) / 235.0).floor().max(1.0) as usize;
        let card_width = (ui.available_width() - (columns - 1) as f32 * 12.0) / columns as f32;
        for row in flavors.chunks(columns) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for flavor in row {
                    ui.push_id(&flavor.name, |ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(card_width, 0.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                settings::card(ui, |ui| {
                                    ui.set_width((card_width - 32.0).max(80.0));
                                    ui.strong(&flavor.name);
                                    let preview_size = egui::vec2(ui.available_width(), 106.0);
                                    if !ui.is_rect_visible(egui::Rect::from_min_size(
                                        ui.cursor().min,
                                        preview_size,
                                    )) {
                                        ui.allocate_exact_size(preview_size, egui::Sense::hover());
                                    } else if let Some(path) = &flavor.preview {
                                        match cached_preview(ui.ctx(), path) {
                                            Ok(texture) => {
                                                let (rect, _) = ui.allocate_exact_size(
                                                    preview_size,
                                                    egui::Sense::hover(),
                                                );
                                                ui.put(
                                                    rect,
                                                    egui::Image::new(&texture)
                                                        .fit_to_exact_size(preview_size)
                                                        .maintain_aspect_ratio(true),
                                                );
                                            }
                                            Err(error) => {
                                                ui.allocate_ui(preview_size, |ui| {
                                                    ui.weak("Превью недоступно")
                                                        .on_hover_text(error);
                                                });
                                            }
                                        }
                                    } else {
                                        ui.allocate_ui(preview_size, |ui| {
                                            ui.centered_and_justified(|ui| {
                                                ui.weak("Нет превью");
                                            });
                                        });
                                    }
                                    let active_dark = dark.as_deref() == Some(flavor.name.as_str());
                                    let active_light =
                                        light.as_deref() == Some(flavor.name.as_str());
                                    ui.weak(match (active_dark, active_light) {
                                        (true, true) => "Выбрана · тёмная и светлая",
                                        (true, false) => "Выбрана · тёмная",
                                        (false, true) => "Выбрана · светлая",
                                        _ => "Доступна локально",
                                    });
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label("Для:");
                                        let target_id = egui::Id::new("yazi-flavor-target");
                                        let mut for_light = ui.ctx().data_mut(|data| {
                                            data.get_temp::<bool>(target_id).unwrap_or(false)
                                        });
                                        ui.selectable_value(&mut for_light, false, "Тёмной");
                                        ui.selectable_value(&mut for_light, true, "Светлой");
                                        ui.ctx().data_mut(|data| {
                                            data.insert_temp(target_id, for_light)
                                        });
                                        if ui
                                            .add_enabled(
                                                flavor.compatible,
                                                egui::Button::new("Применить тему"),
                                            )
                                            .clicked()
                                        {
                                            *request = Some(YaziRequest::ApplyAppearance {
                                                dark: if for_light {
                                                    dark.clone()
                                                } else {
                                                    Some(flavor.name.clone())
                                                },
                                                light: if for_light {
                                                    Some(flavor.name.clone())
                                                } else {
                                                    light.clone()
                                                },
                                                icons: icons.clone(),
                                            });
                                        }
                                    });
                                    if let Some(problem) = &flavor.problem {
                                        crate::ui_foundation::message(ui, problem, true);
                                    }
                                    ui.collapsing("Источник и лицензия", |ui| {
                                        ui.weak(&flavor.source);
                                        ui.weak(
                                            flavor.revision.chars().take(10).collect::<String>(),
                                        );
                                        ui.label(&flavor.license);
                                    });
                                })
                            },
                        );
                    });
                }
            });
            ui.add_space(ui_tokens::MD);
        }
        ui.add_space(ui_tokens::LG);
        settings::card(ui, |ui| {
            ui.strong("Установить open-source flavor с GitHub");
            ui.add(
                egui::TextEdit::singleline(&mut self.yazi_settings.install_source)
                    .hint_text("owner/repository или owner/repository:flavor.yazi")
                    .desired_width(f32::INFINITY),
            );
            if ui.button("Скачать и проверить").clicked() {
                *request = Some(YaziRequest::InstallFlavor {
                    source: self.yazi_settings.install_source.clone(),
                });
            }
            ui.weak("Только файлы оформления. Скрипты репозитория не запускаются. Скачанные темы доступны офлайн.");
        });
    }
    fn yazi_general_page(&mut self, ui: &mut egui::Ui, request: &mut Option<YaziRequest>) {
        for (key, label, choices) in yazi_profile::GENERAL {
            let mut selected = self
                .yazi_settings
                .view
                .effective_general
                .get(*key)
                .cloned()
                .unwrap_or_default();
            let old = selected.clone();
            ui.push_id(key, |ui| {
                settings::card(ui, |ui| {
                    let source = self
                        .yazi_settings
                        .view
                        .general_sources
                        .get(*key)
                        .map(String::as_str)
                        .unwrap_or("Yazi default");
                    let source = match source {
                        "Afio override" => "Настроено в Afio",
                        "Yazi session" => "Из настроек Yazi",
                        "Yazi default" => "По умолчанию Yazi",
                        other => other,
                    };
                    settings::row(ui, label, source, 300.0, |ui| {
                        egui::ComboBox::from_id_salt("value")
                            .width(ui.available_width().min(180.0))
                            .truncate()
                            .selected_text(general_value_label(key, &selected))
                            .show_ui(ui, |ui| {
                                for choice in *choices {
                                    ui.selectable_value(
                                        &mut selected,
                                        (*choice).into(),
                                        general_value_label(key, choice),
                                    );
                                }
                            });
                        if ui
                            .add_enabled(
                                self.yazi_settings.view.profile.general.contains_key(*key),
                                egui::Button::new("Сбросить"),
                            )
                            .on_hover_text("Наследовать настройку Yazi")
                            .clicked()
                        {
                            *request = Some(YaziRequest::SetGeneral {
                                key: (*key).into(),
                                value: None,
                            });
                        }
                    });
                })
            });
            if selected != old {
                *request = Some(YaziRequest::SetGeneral {
                    key: (*key).into(),
                    value: Some(selected),
                });
            }
            ui.add_space(ui_tokens::SM);
        }
    }
    fn yazi_quicklinks_page(&mut self, ui: &mut egui::Ui, request: &mut Option<YaziRequest>) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("Ваши переходы g…");
            if ui.button("+ Переход").clicked() {
                self.yazi_settings.link_edit = Some(Quicklink {
                    id: format!(
                        "link-{}",
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    ),
                    name: String::new(),
                    key: String::new(),
                    path: PathBuf::new(),
                    enabled: true,
                    deleted: false,
                });
                self.yazi_settings.link_path.clear();
                self.yazi_settings.replace = false;
            }
        });
        ui.add_space(ui_tokens::SM);
        if self.yazi_settings.view.profile.quicklinks.is_empty() {
            crate::ui_foundation::message(
                ui,
                "Переходов пока нет. Добавьте папку и клавишу после g.",
                false,
            );
        }
        for link in self.yazi_settings.view.profile.quicklinks.clone() {
            ui.push_id(&link.id, |ui| {
                settings::card(ui, |ui| {
                    let title = format!("g {} · {}", link.key, link.name);
                    let path = friendly_path(&link.path);
                    crate::ui_foundation::command_row(ui, &title, &path, |ui| {
                        if link.deleted {
                            ui.weak("Удалён");
                            if ui.button("Восстановить").clicked() {
                                *request = Some(YaziRequest::RestoreQuicklink {
                                    id: link.id.clone(),
                                });
                            }
                        } else {
                            if ui.button("Изменить").clicked() {
                                self.yazi_settings.link_edit = Some(link.clone());
                                self.yazi_settings.link_path = friendly_path(&link.path);
                                self.yazi_settings.replace = false;
                            }
                            if ui.button("Проверить").on_hover_text("Нажать настоящие g и клавишу в Yazi; сначала примените изменения").clicked() {
                                *request = Some(YaziRequest::TestQuicklink { id: link.id.clone() });
                            }
                            let mut enabled = link.enabled;
                            if ui.checkbox(&mut enabled, "Включён").changed() {
                                let mut link = link.clone();
                                link.enabled = enabled;
                                *request = Some(YaziRequest::SaveQuicklink {
                                    link,
                                    replace: true,
                                });
                            }
                            if ui.button("Удалить").clicked() {
                                *request = Some(YaziRequest::DeleteQuicklink {
                                    id: link.id.clone(),
                                });
                            }
                        }
                    });
                })
            });
            ui.add_space(ui_tokens::SM);
        }
        if let Some(mut link) = self.yazi_settings.link_edit.take() {
            let mut close = false;
            settings::card(ui, |ui| {
                ui.strong("Quicklink");
                settings::row(
                    ui,
                    "Название",
                    "Имя перехода в списке",
                    360.0,
                    |ui| {
                        ui.text_edit_singleline(&mut link.name);
                    },
                );
                ui.label("Папка Windows");
                ui.add(
                    egui::TextEdit::singleline(&mut self.yazi_settings.link_path)
                        .desired_width(f32::INFINITY),
                );
                if ui
                    .add_enabled(
                        self.yazi_settings.picker.is_none(),
                        egui::Button::new("Выбрать папку…"),
                    )
                    .clicked()
                {
                    *request = Some(YaziRequest::PickFolder);
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label("g →");
                    ui.text_edit_singleline(&mut link.key);
                });
                ui.checkbox(
                    &mut self.yazi_settings.replace,
                    "Явно заменить существующую команду на этих клавишах",
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Сохранить переход").clicked() {
                        link.path = PathBuf::from(&self.yazi_settings.link_path);
                        *request = Some(YaziRequest::SaveQuicklink {
                            link: link.clone(),
                            replace: self.yazi_settings.replace,
                        });
                    }
                    if ui.button("Закрыть редактор").clicked() {
                        close = true;
                    }
                });
            });
            if !close {
                self.yazi_settings.link_edit = Some(link);
            }
        }
    }
    fn yazi_binding_editor(&mut self, ui: &mut egui::Ui, request: &mut Option<YaziRequest>) {
        if let Some(mut binding) = self.yazi_settings.binding_edit.take() {
            let mut close = false;
            settings::card(ui, |ui| {
                ui.strong("Переопределение Yazi");
                egui::ComboBox::from_id_salt("binding-layer")
                    .selected_text(layer_label(&binding.layer))
                    .show_ui(ui, |ui| {
                        for layer in yazi_profile::LAYERS {
                            ui.selectable_value(
                                &mut binding.layer,
                                (*layer).into(),
                                layer_label(layer),
                            );
                        }
                    });
                settings::row(
                    ui,
                    "Название",
                    "Описание команды",
                    360.0,
                    |ui| {
                        ui.text_edit_singleline(&mut binding.description);
                    },
                );
                crate::yazi_key_capture::controls(ui, &mut self.yazi_settings.binding_keys);
                ui.label("Нативные команды Yazi — по одной на строку");
                ui.add(
                    egui::TextEdit::multiline(&mut self.yazi_settings.binding_run)
                        .desired_rows(3)
                        .desired_width(f32::INFINITY),
                );
                ui.checkbox(
                    &mut self.yazi_settings.replace,
                    "Явно заменить конфликтующее назначение",
                );
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !crate::yazi_key_capture::recording(ui.ctx()),
                            egui::Button::new("Сохранить override"),
                        )
                        .clicked()
                    {
                        binding.on = self
                            .yazi_settings
                            .binding_keys
                            .split_whitespace()
                            .map(str::to_owned)
                            .collect();
                        binding.run = self
                            .yazi_settings
                            .binding_run
                            .lines()
                            .filter(|s| !s.trim().is_empty())
                            .map(str::to_owned)
                            .collect();
                        *request = Some(YaziRequest::SaveBinding {
                            binding: binding.clone(),
                            replace: self.yazi_settings.replace,
                        });
                    }
                    if ui.button("Закрыть редактор").clicked() {
                        close = true;
                    }
                });
            });
            if !close {
                self.yazi_settings.binding_edit = Some(binding);
            } else {
                crate::yazi_key_capture::close(ui.ctx());
            }
            ui.add_space(ui_tokens::MD);
        }
    }
    fn yazi_keybindings_page(&mut self, ui: &mut egui::Ui, request: &mut Option<YaziRequest>) {
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.yazi_settings.filter)
                    .hint_text("Поиск: имя, клавиши, режим…"),
            );
            if ui.button("+ Назначение").clicked() {
                crate::yazi_key_capture::open(ui.ctx(), "new".into());
                self.yazi_settings.binding_edit = Some(YaziBinding {
                    id: format!(
                        "binding-{}",
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_nanos()
                    ),
                    layer: "mgr".into(),
                    on: Vec::new(),
                    run: Vec::new(),
                    description: String::new(),
                    enabled: true,
                });
                self.yazi_settings.binding_keys.clear();
                self.yazi_settings.binding_run.clear();
                self.yazi_settings.replace = false;
            }
        });
        ui.weak("Tab и входы в Actions / Settings / Everything остаются за Afio.");
        if crate::yazi_key_capture::anchored(ui.ctx(), "new") {
            self.yazi_binding_editor(ui, request);
        }
        let query = self.yazi_settings.filter.trim().to_lowercase();
        let overrides: Vec<_> = self
            .yazi_settings
            .view
            .profile
            .bindings
            .iter()
            .filter(|b| {
                format!(
                    "{} {} {} {}",
                    layer_label(&b.layer),
                    b.on.join(" "),
                    b.description,
                    b.run.join(" ")
                )
                .to_lowercase()
                .contains(&query)
            })
            .cloned()
            .collect();
        egui::CollapsingHeader::new(format!("Мои переопределения · {}", overrides.len()))
            .id_salt(("yazi-own-bindings", query.is_empty()))
            .default_open(false)
            .open((!query.is_empty()).then_some(true))
            .show(ui, |ui| {
                for binding in overrides {
                    ui.push_id(format!("own-{}", binding.id), |ui| {
                        settings::card(ui, |ui| {
                            crate::ui_foundation::command_row(
                                ui,
                                &binding.description,
                                &format!(
                                    "{} · {}",
                                    binding.on.join(" → "),
                                    layer_label(&binding.layer)
                                ),
                                |ui| {
                                    if ui.button("Изменить").clicked() {
                                        crate::yazi_key_capture::open(
                                            ui.ctx(),
                                            format!("own:{}", binding.id),
                                        );
                                        self.yazi_settings.binding_edit = Some(binding.clone());
                                        self.yazi_settings.binding_keys = binding.on.join(" ");
                                        self.yazi_settings.binding_run = binding.run.join("\n");
                                        self.yazi_settings.replace = true;
                                    }
                                    if ui.button("Сбросить override").clicked() {
                                        *request = Some(YaziRequest::DeleteBinding {
                                            id: binding.id.clone(),
                                        });
                                    }
                                },
                            );
                            if crate::yazi_key_capture::anchored(
                                ui.ctx(),
                                &format!("own:{}", binding.id),
                            ) {
                                self.yazi_binding_editor(ui, request);
                            }
                        })
                    });
                    ui.add_space(ui_tokens::SM);
                }
            });
        let rows: Vec<_> = self
            .yazi_settings
            .view
            .bindings
            .iter()
            .filter(|b| {
                format!(
                    "{} {} {} {} {}",
                    b.layer,
                    layer_label(&b.layer),
                    b.on.join(" "),
                    b.description,
                    b.run.join(" ")
                )
                .to_lowercase()
                .contains(&query)
            })
            .cloned()
            .collect();
        ui.weak(format!("Эффективные назначения: {}", rows.len()));
        if rows.is_empty() && !query.is_empty() {
            crate::ui_foundation::message(
                ui,
                "Назначения не найдены. Попробуйте другие клавиши или название.",
                false,
            );
        }
        for layer in yazi_profile::LAYERS {
            let layer_rows: Vec<_> = rows.iter().filter(|row| row.layer == *layer).collect();
            if layer_rows.is_empty() {
                continue;
            }
            ui.add_space(ui_tokens::SM);
            egui::CollapsingHeader::new(format!("{} · {}", layer_label(layer), layer_rows.len()))
                .id_salt(("yazi-layer", layer, query.is_empty()))
                .default_open(false)
                .open((!query.is_empty()).then_some(true))
                .show(ui, |ui| {
                    for row in &layer_rows {
                        let editor_anchor =
                            format!("effective:{}:{}:{:?}", row.layer, row.source, row.on);
                        ui.push_id(&editor_anchor, |ui| {
                            settings::card(ui, |ui| {
                                crate::ui_foundation::command_row(
                                    ui,
                                    &row.description,
                                    &format!(
                                        "{} · {}",
                                        row.on.join(" → "),
                                        layer_label(&row.layer)
                                    ),
                                    |ui| {
                                        if row.source.starts_with("Quicklink:") {
                                            ui.weak("→ Quicklinks");
                                        }
                                        if !row.source.starts_with("Quicklink:")
                                            && ui.small_button("Изменить").clicked()
                                        {
                                            crate::yazi_key_capture::open(
                                                ui.ctx(),
                                                editor_anchor.clone(),
                                            );
                                            let binding = YaziBinding {
                                                id: row
                                                    .source
                                                    .strip_prefix("Afio:")
                                                    .map(str::to_owned)
                                                    .unwrap_or_else(|| {
                                                        format!(
                                                            "override-{}-{}",
                                                            row.layer,
                                                            row.on.join("-")
                                                        )
                                                    }),
                                                layer: row.layer.clone(),
                                                on: row.on.clone(),
                                                run: row.run.clone(),
                                                description: row.description.clone(),
                                                enabled: true,
                                            };
                                            self.yazi_settings.binding_keys = binding.on.join(" ");
                                            self.yazi_settings.binding_run = binding.run.join("\n");
                                            self.yazi_settings.binding_edit = Some(binding);
                                            self.yazi_settings.replace = false;
                                        }
                                        if !row.source.starts_with("Quicklink:")
                                            && ui.small_button("Отключить клавишу").clicked()
                                        {
                                            *request = Some(YaziRequest::SaveBinding {
                                                binding: YaziBinding {
                                                    id: row
                                                        .source
                                                        .strip_prefix("Afio:")
                                                        .map(str::to_owned)
                                                        .unwrap_or_else(|| {
                                                            format!(
                                                                "override-{}-{}",
                                                                row.layer,
                                                                row.on.join("-")
                                                            )
                                                        }),
                                                    layer: row.layer.clone(),
                                                    on: row.on.clone(),
                                                    run: vec!["noop".into()],
                                                    description: format!(
                                                        "Disabled: {}",
                                                        row.description
                                                    ),
                                                    enabled: true,
                                                },
                                                replace: true,
                                            });
                                        }
                                    },
                                );
                                ui.weak(format!("{} · {}", row.source, row.run.join("; ")));
                                if crate::yazi_key_capture::anchored(ui.ctx(), &editor_anchor) {
                                    self.yazi_binding_editor(ui, request);
                                }
                            })
                        });
                        ui.add_space(ui_tokens::SM);
                    }
                });
        }
    }
}

fn load_preview(ctx: &egui::Context, path: &Path) -> Result<TextureHandle, String> {
    use image::ImageDecoder;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Preview exceeds 4 MiB".into());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions();
    if u64::from(w) * u64::from(h) > 4_000_000 {
        return Err("Preview dimensions are too large".into());
    }
    let pixels = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .thumbnail(480, 240)
        .to_rgba8();
    Ok(ctx.load_texture(
        "yazi-flavor-preview",
        egui::ColorImage::from_rgba_unmultiplied(
            [pixels.width() as usize, pixels.height() as usize],
            pixels.as_raw(),
        ),
        TextureOptions::LINEAR,
    ))
}

fn general_value_label<'a>(key: &str, value: &'a str) -> &'a str {
    match (key, value) {
        (_, "true") => "Включено",
        (_, "false") => "Выключено",
        ("sort_by", "alphabetical") => "По алфавиту",
        ("sort_by", "natural") => "По имени с учётом чисел",
        ("sort_by", "extension") => "По расширению",
        ("sort_by", "size") => "По размеру",
        ("sort_by", "mtime") => "По дате изменения",
        ("sort_by", "btime") => "По дате создания",
        ("sort_by", "none") => "Без сортировки",
        ("linemode", "none") => "Без подписей",
        ("linemode", "size") => "Размер",
        ("linemode", "mtime") => "Дата изменения",
        ("linemode", "btime") => "Дата создания",
        ("linemode", "permissions") => "Права доступа",
        ("linemode", "owner") => "Владелец",
        _ => value,
    }
}

fn layer_label(layer: &str) -> &str {
    match layer {
        "mgr" => "Файлы и навигация",
        "input" => "Ввод текста",
        "confirm" => "Подтверждения",
        "tasks" => "Фоновые задачи",
        "spot" => "Сведения о файле",
        "pick" => "Выбор вариантов",
        "cmp" => "Автодополнение",
        "help" => "Справка",
        _ => layer,
    }
}

fn cached_preview(ctx: &egui::Context, path: &Path) -> Result<TextureHandle, String> {
    type Cache = Vec<(PathBuf, Result<TextureHandle, String>)>;
    let id = egui::Id::new("yazi-flavor-previews");
    let mut cache = ctx.data_mut(|data| data.get_temp::<Cache>(id).unwrap_or_default());
    if let Some(index) = cache.iter().position(|(loaded, _)| loaded == path) {
        let entry = cache.remove(index);
        let result = entry.1.clone();
        cache.push(entry);
        ctx.data_mut(|data| data.insert_temp(id, cache));
        return result;
    }
    let result = load_preview(ctx, path);
    if cache.len() >= 12 {
        let _ = cache.remove(0);
    }
    cache.push((path.to_path_buf(), result.clone()));
    ctx.data_mut(|data| data.insert_temp(id, cache));
    result
}
