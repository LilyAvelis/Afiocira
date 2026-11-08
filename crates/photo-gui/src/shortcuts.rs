//! One resolved command catalogue for Settings, Actions and keyboard dispatch.
use super::*;
use crate::ui_foundation::tokens as ui_tokens;
use photo_core::actions::{ActionId, ActionInfo, Context};
use photo_core::settings::ShortcutInfo;
use serde_json::{Value, json};

fn action_key(action: ActionId) -> String {
    serde_json::to_value(action)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
fn defaults(context: &str) -> Vec<ActionInfo> {
    let mut c = Context {
        album_overview: context == "albums",
        space: context == "space",
        targets: 1,
        video: false,
        busy: false,
        has_gallery: true,
        marked: true,
    };
    let mut result = photo_core::actions::available(&c);
    if c.space {
        c.video = true;
        for action in photo_core::actions::available(&c) {
            if !result.iter().any(|a| a.id == action.id) {
                result.push(action);
            }
        }
    }
    result
}
pub(super) fn effective(
    settings: &Value,
    context: &str,
    action: ActionId,
    default: &str,
) -> String {
    if matches!(action, ActionId::Search | ActionId::Settings) {
        return default.into();
    }
    let key = action_key(action);
    match settings.get(context).and_then(|v| v.get(&key)) {
        Some(Value::Null) => String::new(),
        Some(Value::String(binding)) => binding.clone(),
        _ => default.into(),
    }
}
pub(super) fn catalogue(settings: &Value) -> Vec<ShortcutInfo> {
    ["gallery", "albums", "space"]
        .into_iter()
        .flat_map(|context| {
            defaults(context).into_iter().map(move |a| {
                let binding = effective(settings, context, a.id, &a.shortcut);
                ShortcutInfo {
                    action: a.id,
                    context: context.into(),
                    label: a.label,
                    binding: (!binding.is_empty()).then_some(binding),
                    default_binding: (!a.shortcut.is_empty()).then_some(a.shortcut),
                    rebindable: !matches!(a.id, ActionId::Search | ActionId::Settings),
                }
            })
        })
        .collect()
}
pub(super) fn parse(text: &str) -> Result<Vec<egui::KeyboardShortcut>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    text.split('/')
        .map(|chord| {
            let mut modifiers = egui::Modifiers::NONE;
            let mut key = None;
            for part in chord.split_whitespace() {
                match part.to_lowercase().as_str() {
                    "ctrl" | "control" => modifiers.ctrl = true,
                    "shift" => modifiers.shift = true,
                    "alt" => modifiers.alt = true,
                    _ => {
                        let name = match part {
                            "→" => "ArrowRight",
                            "←" => "ArrowLeft",
                            "↑" => "ArrowUp",
                            "↓" => "ArrowDown",
                            "Esc" => "Escape",
                            "," => "Comma",
                            _ => part,
                        };
                        if key.is_some() {
                            return Err("Use one key per shortcut".into());
                        }
                        key = Some(
                            Key::from_name(name).ok_or_else(|| format!("Unknown key: {part}"))?,
                        );
                    }
                }
            }
            Ok(egui::KeyboardShortcut::new(
                modifiers,
                key.ok_or("A shortcut needs a key")?,
            ))
        })
        .collect()
}
pub(super) fn reserved(chord: &egui::KeyboardShortcut, context: &str) -> bool {
    chord.logical_key == Key::Tab
        || (chord.modifiers.ctrl
            && !chord.modifiers.alt
            && matches!(chord.logical_key, Key::K | Key::Comma))
        || (context != "space"
            && matches!(
                chord.logical_key,
                Key::ArrowUp | Key::ArrowDown | Key::ArrowLeft | Key::ArrowRight
            )
            && !chord.modifiers.ctrl
            && !chord.modifiers.alt)
}
fn normalize(shortcuts: &[egui::KeyboardShortcut]) -> String {
    shortcuts
        .iter()
        .map(|s| {
            let mut parts = Vec::new();
            if s.modifiers.ctrl {
                parts.push("Ctrl");
            }
            if s.modifiers.alt {
                parts.push("Alt");
            }
            if s.modifiers.shift {
                parts.push("Shift");
            }
            parts.push(s.logical_key.name());
            parts.join(" ")
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// Consume recording input before text fields and action dispatch can see it.
/// The outer Option means a fresh key; the inner None means Escape/cancel.
pub(super) fn take_recorded_chord(input: &mut egui::InputState) -> Option<Option<String>> {
    let captured = input.events.iter().find_map(|event| match event {
        egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } => Some(if *key == Key::Escape && modifiers.is_none() {
            None
        } else {
            Some(normalize(&[egui::KeyboardShortcut::new(*modifiers, *key)]))
        }),
        _ => None,
    });
    discard_recording_events(input);
    captured
}

pub(super) fn discard_recording_events(input: &mut egui::InputState) {
    input.events.retain(|event| {
        !matches!(
            event,
            egui::Event::Key { .. }
                | egui::Event::Text(_)
                | egui::Event::Copy
                | egui::Event::Cut
                | egui::Event::Paste(_)
        )
    });
}

pub(super) fn take_action(
    input: &mut egui::InputState,
    actions: Vec<ActionInfo>,
    in_yazi: bool,
    media_paste: bool,
) -> Option<ActionId> {
    for action in actions {
        if in_yazi
            && !matches!(
                action.id,
                ActionId::Copy
                    | ActionId::Duplicate
                    | ActionId::Paste
                    | ActionId::Rename
                    | ActionId::Delete
            )
        {
            continue;
        }
        for shortcut in parse(&action.shortcut).unwrap_or_default() {
            let native_copy = shortcut.logical_key == Key::C
                && shortcut.modifiers == egui::Modifiers::CTRL
                && input.events.iter().any(|e| matches!(e, egui::Event::Copy));
            let native_paste = shortcut.logical_key == Key::V
                && shortcut.modifiers == egui::Modifiers::CTRL
                && media_paste;
            let fresh = input.events.iter().any(|e| matches!(e, egui::Event::Key { key, pressed: true, repeat, modifiers, .. } if *key == shortcut.logical_key && modifiers.matches_exact(shortcut.modifiers) && (!*repeat || matches!(action.id, ActionId::Next | ActionId::Previous))));
            let pressed = if action.id == ActionId::Delete
                && shortcut.logical_key == Key::Delete
                && shortcut.modifiers.is_none()
            {
                fresh && crate::delete_action::take_delete_press(input)
            } else {
                fresh && input.consume_shortcut(&shortcut)
            };
            if native_copy || native_paste || pressed {
                if native_copy {
                    input.events.retain(|e| !matches!(e, egui::Event::Copy));
                }
                if native_paste {
                    input.events.retain(|e| !matches!(e, egui::Event::Paste(_)));
                }
                return Some(action.id);
            }
        }
    }
    None
}

fn plan_change(
    settings: &Value,
    action: ActionId,
    context: &str,
    binding: Option<&str>,
    reset: bool,
    replace: bool,
) -> Result<Value, String> {
    let registry = catalogue(settings);
    let entry = registry
        .iter()
        .find(|e| e.context == context && e.action == action)
        .ok_or("Command is not registered in this context")?;
    if !entry.rebindable {
        return Err("This shortcut is a fixed Afio foundation".into());
    }
    let mut next = settings.clone();
    if !next[context].is_object() {
        next[context] = json!({});
    }
    let key = action_key(action);
    if reset {
        next[context].as_object_mut().unwrap().remove(&key);
    } else {
        let parsed = parse(binding.unwrap_or(""))?;
        if parsed.iter().any(|c| reserved(c, context)) {
            return Err(
                "Tab, Ctrl+K, Ctrl+Shift+K, Ctrl+Comma, Ctrl+Shift+Comma, and Gallery selection arrows are fixed".into(),
            );
        }
        next[context][&key] = if parsed.is_empty() {
            Value::Null
        } else {
            Value::String(normalize(&parsed))
        };
    }
    let proposed = catalogue(&next);
    let own = proposed
        .iter()
        .find(|e| e.context == context && e.action == action)
        .unwrap();
    let chords = parse(own.binding.as_deref().unwrap_or(""))?;
    for other in proposed
        .iter()
        .filter(|e| e.context == context && e.action != action)
    {
        let other_chords = parse(other.binding.as_deref().unwrap_or(""))?;
        if chords.iter().any(|c| other_chords.contains(c)) {
            if !replace || !other.rebindable {
                return Err(format!(
                    "{} conflicts with {} in {context}",
                    own.label, other.label
                ));
            }
            next[context][action_key(other.action)] = Value::Null;
        }
    }
    Ok(next)
}

impl Workshop {
    pub(super) fn shortcut_context(&self) -> &'static str {
        if self
            .gallery
            .as_ref()
            .is_some_and(|g| g.view() == View::Photo)
        {
            "space"
        } else if self.album_mode() {
            "albums"
        } else {
            "gallery"
        }
    }
    pub(super) fn set_shortcut(
        &mut self,
        action: ActionId,
        context: String,
        binding: Option<String>,
        reset: bool,
        replace: bool,
    ) -> Result<(), String> {
        let result = self.set_shortcut_inner(action, &context, binding.as_deref(), reset, replace);
        self.settings.error = result.as_ref().err().cloned();
        self.settings.shortcut_candidate = binding
            .filter(|binding| {
                result
                    .as_ref()
                    .err()
                    .is_some_and(|error| error.contains(" conflicts with "))
                    && plan_change(
                        &self.settings.shortcuts,
                        action,
                        &context,
                        Some(binding),
                        reset,
                        true,
                    )
                    .is_ok()
            })
            .map(|binding| (action, context, binding));
        result
    }
    fn set_shortcut_inner(
        &mut self,
        action: ActionId,
        context: &str,
        binding: Option<&str>,
        reset: bool,
        replace: bool,
    ) -> Result<(), String> {
        let next = settings::update_config(&settings::config_path()?, |document| {
            let current = document
                .get("shortcuts")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({}));
            if !current.is_object() {
                return Err("Invalid shortcuts settings; original file preserved.".into());
            }
            let next = plan_change(&current, action, context, binding, reset, replace)?;
            document["shortcuts"] = next.clone();
            Ok(next)
        })?;
        self.settings.shortcuts = next;
        Ok(())
    }
    pub(super) fn capture_shortcut(&mut self, ctx: &egui::Context) {
        let Some((action, context)) = self.settings.recording.clone() else {
            return;
        };
        let captured = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } = event
                {
                    Some((*key, *modifiers))
                } else {
                    None
                }
            })
        });
        if let Some((key, modifiers)) = captured {
            ctx.input_mut(|i| {
                i.events.retain(|e| {
                    !matches!(
                        e,
                        egui::Event::Key { .. }
                            | egui::Event::Text(_)
                            | egui::Event::Copy
                            | egui::Event::Cut
                            | egui::Event::Paste(_)
                    )
                })
            });
            self.settings.recording = None;
            if key == Key::Escape && modifiers.is_none() {
                return;
            }
            let binding = normalize(&[egui::KeyboardShortcut::new(modifiers, key)]);
            self.execute(Command::SetShortcut {
                action,
                context,
                binding: Some(binding),
                reset: false,
                replace: false,
            });
        }
    }
    pub(super) fn afio_settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add(Lucide::Keyboard.size(25.0).image());
            ui.heading("Afio");
        });
        ui.weak("Shortcuts · команды остаются в Actions даже без клавиши");
        ui.add_space(ui_tokens::LG);
        settings::card(ui, |ui| {
            settings::row(
                ui,
                "Подстраивать карточки под окно",
                "Автоматический размер в Gallery и альбомах. Ручной размер сохраняется.",
                310.0,
                |ui| {
                    let mut enabled = self.settings.gallery_auto_size;
                    if ui.checkbox(&mut enabled, "Включено").changed() {
                        self.execute(Command::SetGalleryAutoSize { enabled });
                    }
                },
            );
            settings::row(
                ui,
                "Размер карточек",
                "Ctrl+колесо в Gallery переключает на ручной размер.",
                310.0,
                |ui| {
                    ui.add_enabled_ui(!self.settings.gallery_auto_size, |ui| {
                        let mut scale = self.settings.gallery_scale.unwrap_or(1.0);
                        ui.spacing_mut().slider_width = 120.0;
                        if ui
                            .add(egui::Slider::new(&mut scale, 0.4..=2.5).suffix(" ×"))
                            .changed()
                        {
                            self.execute(Command::SetGalleryScale { scale: Some(scale) });
                        }
                        if ui
                            .button("Сбросить")
                            .on_hover_text("Автоматически по ширине окна")
                            .clicked()
                        {
                            self.execute(Command::SetGalleryScale { scale: None });
                        }
                    });
                },
            );
        });
        ui.add_space(ui_tokens::LG);
        ui.add(
            egui::TextEdit::singleline(&mut self.settings.shortcut_filter)
                .hint_text("Найти команду или клавишу…")
                .desired_width(f32::INFINITY),
        );
        if let Some((action, context, binding)) = self.settings.shortcut_candidate.clone()
            && ui.button("Заменить конфликтующее назначение").clicked()
        {
            self.execute(Command::SetShortcut {
                action,
                context,
                binding: Some(binding),
                reset: false,
                replace: true,
            });
        }
        let mut command = None;
        let query = self.settings.shortcut_filter.trim().to_lowercase();
        let catalogue = catalogue(&self.settings.shortcuts);
        let mut matched = 0;
        for context in ["gallery", "albums", "space"] {
            let rows: Vec<_> = catalogue
                .iter()
                .filter(|e| {
                    e.context == context
                        && format!(
                            "{} {:?} {}",
                            e.label,
                            e.action,
                            e.binding.as_deref().unwrap_or("")
                        )
                        .to_lowercase()
                        .contains(&query)
                })
                .collect();
            if rows.is_empty() {
                continue;
            }
            matched += rows.len();
            ui.add_space(ui_tokens::MD);
            let heading = match context {
                "gallery" => "Gallery",
                "albums" => "Albums",
                _ => "Space",
            };
            let mut group = egui::CollapsingHeader::new(format!("{heading} ({})", rows.len()))
                .id_salt(("shortcut-group", context))
                .default_open(false);
            if !query.is_empty() {
                group = group.open(Some(true));
            }
            group.show(ui, |ui| {
                settings::card(ui, |ui| {
                    for (index, entry) in rows.into_iter().enumerate() {
                        if index > 0 {
                            ui.add_space(ui_tokens::SM);
                            ui.separator();
                            ui.add_space(ui_tokens::SM);
                        }
                        ui.push_id((context, action_key(entry.action)), |ui| {
                            let recording = self
                                .settings
                                .recording
                                .as_ref()
                                .is_some_and(|(a, c)| *a == entry.action && c == context);
                            let description = if !entry.rebindable {
                                "Фиксированная клавиша"
                            } else if recording {
                                "Нажмите сочетание; Esc отменяет запись"
                            } else {
                                ""
                            };
                            crate::ui_foundation::command_row(
                                ui,
                                &entry.label,
                                description,
                                |ui| {
                                    let binding = if recording {
                                        "Запись..."
                                    } else {
                                        entry.binding.as_deref().unwrap_or("Назначить")
                                    };
                                    if ui
                                        .add_enabled(
                                            entry.rebindable,
                                            egui::Button::new(binding).min_size(egui::vec2(
                                                110.0,
                                                crate::ui_foundation::tokens::CONTROL_HEIGHT,
                                            )),
                                        )
                                        .clicked()
                                    {
                                        self.settings.recording =
                                            Some((entry.action, context.into()));
                                    }
                                    if entry.rebindable {
                                        if ui
                                            .add_sized(
                                                [
                                                    88.0,
                                                    crate::ui_foundation::tokens::CONTROL_HEIGHT,
                                                ],
                                                egui::Button::new("Убрать"),
                                            )
                                            .clicked()
                                        {
                                            command = Some(Command::SetShortcut {
                                                action: entry.action,
                                                context: context.into(),
                                                binding: None,
                                                reset: false,
                                                replace: false,
                                            });
                                        }
                                        if ui
                                            .add_sized(
                                                [
                                                    88.0,
                                                    crate::ui_foundation::tokens::CONTROL_HEIGHT,
                                                ],
                                                egui::Button::new("Сбросить"),
                                            )
                                            .clicked()
                                        {
                                            command = Some(Command::SetShortcut {
                                                action: entry.action,
                                                context: context.into(),
                                                binding: None,
                                                reset: true,
                                                replace: false,
                                            });
                                        }
                                    }
                                },
                            );
                        });
                    }
                });
            });
        }
        if matched == 0 {
            crate::ui_foundation::message(ui, "Команды не найдены", false);
        }
        if let Some(command) = command {
            self.execute(command);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_consumes_text_native_copy_and_key_before_command_dispatch() {
        let ctx = egui::Context::default();
        let modifiers = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::Key {
                        key: Key::C,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                    egui::Event::Copy,
                    egui::Event::Text("c".into()),
                ],
                ..Default::default()
            },
            |ui| {
                ui.ctx().input_mut(|input| {
                    assert_eq!(take_recorded_chord(input), Some(Some("Ctrl C".into())));
                    assert!(input.events.is_empty());
                    assert_eq!(take_action(input, defaults("gallery"), false, false), None);
                });
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn recording_escape_cancels_and_repeats_do_not_capture() {
        let ctx = egui::Context::default();
        for (key, repeat, expected) in [
            (Key::Escape, false, Some(None)),
            (Key::Enter, false, Some(Some("Enter".into()))),
            (Key::Enter, true, None),
        ] {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat,
                        modifiers: egui::Modifiers::NONE,
                    }],
                    ..Default::default()
                },
                |ui| {
                    assert_eq!(ui.ctx().input_mut(take_recorded_chord), expected);
                },
            );
            output.textures_delta.clear();
        }
    }
    #[test]
    fn keyboard_uses_resolved_bindings_and_windows_command_alias() {
        let run = |key, modifiers, events: Vec<egui::Event>, settings: &Value, yazi| {
            let ctx = egui::Context::default();
            let mut actions = defaults("space");
            for action in &mut actions {
                action.shortcut = effective(settings, "space", action.id, &action.shortcut);
            }
            let mut events = events;
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            events.push(egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            });
            ctx.begin_pass(egui::RawInput {
                events,
                ..Default::default()
            });
            let action = ctx.input_mut(|i| take_action(i, actions, yazi, false));
            ctx.end_pass().textures_delta.clear();
            action
        };
        let windows_ctrl = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        assert_eq!(
            run(Key::D, windows_ctrl, vec![], &json!({}), false),
            Some(ActionId::Duplicate)
        );
        let remapped = plan_change(
            &json!({}),
            ActionId::Duplicate,
            "space",
            Some("Ctrl J"),
            false,
            false,
        )
        .unwrap();
        assert_eq!(run(Key::D, windows_ctrl, vec![], &remapped, false), None);
        assert_eq!(
            run(Key::J, windows_ctrl, vec![], &remapped, false),
            Some(ActionId::Duplicate)
        );
        let removed = plan_change(&json!({}), ActionId::Copy, "space", None, false, false).unwrap();
        assert_eq!(
            run(
                Key::C,
                windows_ctrl,
                vec![egui::Event::Copy],
                &removed,
                false
            ),
            None
        );
        assert_eq!(
            run(Key::Q, egui::Modifiers::NONE, vec![], &json!({}), true),
            None
        );
    }
    #[test]
    fn overrides_are_optional_contextual_and_conflicts_need_explicit_replacement() {
        let empty = json!({});
        assert!(plan_change(&empty, ActionId::Next, "space", Some("P"), false, false).is_err());
        let replaced =
            plan_change(&empty, ActionId::Next, "space", Some("P"), false, true).unwrap();
        assert_eq!(effective(&replaced, "space", ActionId::PlayPause, "P"), "");
        let removed = plan_change(&replaced, ActionId::Next, "space", None, false, false).unwrap();
        assert_eq!(effective(&removed, "space", ActionId::Next, "→"), "");
        let reset = plan_change(&removed, ActionId::Next, "space", None, true, false).unwrap();
        assert_eq!(effective(&reset, "space", ActionId::Next, "→"), "→");
        let same = plan_change(&empty, ActionId::Copy, "gallery", Some("Q"), false, false).unwrap();
        assert_eq!(
            effective(&same, "space", ActionId::Copy, "Ctrl C"),
            "Ctrl C"
        );
        assert!(plan_change(&empty, ActionId::Search, "space", Some("J"), false, true).is_err());
    }
    #[test]
    fn defaults_parse_and_fixed_selection_keys_cannot_be_stolen() {
        for entry in catalogue(&json!({})) {
            assert!(
                parse(entry.binding.as_deref().unwrap_or("")).is_ok(),
                "{:?}",
                entry
            );
        }
        for text in [
            "Tab",
            "Ctrl K",
            "Ctrl Shift K",
            "Ctrl Comma",
            "Ctrl Shift Comma",
            "Shift ArrowLeft",
        ] {
            assert!(reserved(&parse(text).unwrap()[0], "gallery"));
        }
        assert!(!reserved(&parse("Q").unwrap()[0], "space"));
        assert_eq!(
            effective(
                &json!({"space":{"next":null}}),
                "space",
                ActionId::Next,
                "ArrowRight"
            ),
            ""
        );
    }
}
