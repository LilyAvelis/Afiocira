//! Keyboard recording for Yazi sequences. Printable text and modified key events
//! are separate so a native Key+Text pair records once, with the typed layout.
use eframe::egui::{self, Event, Key};

#[derive(Clone, Default)]
struct State {
    anchor: String,
    recording: bool,
    keys: Vec<String>,
    original: Vec<String>,
    changed: bool,
    error: Option<String>,
}
fn id() -> egui::Id {
    egui::Id::new("yazi-binding-editor-capture")
}
fn state(ctx: &egui::Context) -> State {
    ctx.data_mut(|d| d.get_temp(id()).unwrap_or_default())
}
fn store(ctx: &egui::Context, state: State) {
    ctx.data_mut(|d| d.insert_temp(id(), state));
}
pub(super) fn open(ctx: &egui::Context, anchor: String) {
    store(
        ctx,
        State {
            anchor,
            ..Default::default()
        },
    );
}
pub(super) fn anchored(ctx: &egui::Context, anchor: &str) -> bool {
    state(ctx).anchor == anchor
}
pub(super) fn close(ctx: &egui::Context) {
    store(ctx, State::default());
}

pub(super) fn cancel(ctx: &egui::Context, keys: &mut String) {
    let mut state = state(ctx);
    if state.recording {
        *keys = state.original.join(" ");
        state.keys = state.original.clone();
        state.recording = false;
        state.changed = false;
        state.error = None;
    }
    store(ctx, state);
}

fn record(state: &mut State, key: String) {
    if ["<Tab>", "<S-Tab>", "<C-k>", "<C-S-k>", "<C-,>"]
        .iter()
        .any(|reserved| key.eq_ignore_ascii_case(reserved))
    {
        state.error = Some("Эта клавиша принадлежит Afio. Выберите другую.".into());
    } else if state.keys.len() == 8 {
        state.error = Some("Последовательность может содержать до 8 клавиш.".into());
    } else {
        state.keys.push(key);
        state.changed = true;
        state.error = None;
    }
}
fn consume(state: &mut State, input: &mut egui::InputState) {
    if !state.recording {
        return;
    }
    let events = input.events.clone();
    let mut text_allowed =
        !input.modifiers.ctrl && !input.modifiers.alt && !input.modifiers.mac_cmd;
    for event in events {
        match event {
            Event::Key {
                pressed: true,
                repeat: true,
                ..
            } => {
                text_allowed = false;
            }
            Event::Key {
                key: Key::Escape,
                pressed: true,
                repeat: false,
                ..
            } => {
                state.keys = state.original.clone();
                state.changed = true;
                state.recording = false;
                state.error = None;
                break;
            }
            Event::Text(text) if text_allowed => {
                for c in text.chars().filter(|c| !c.is_control()) {
                    record(
                        state,
                        if c == ' ' {
                            "<Space>".into()
                        } else {
                            c.to_string()
                        },
                    );
                }
            }
            Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } => {
                text_allowed = !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd;
                if matches!(
                    key,
                    Key::ShiftLeft
                        | Key::ShiftRight
                        | Key::ControlLeft
                        | Key::ControlRight
                        | Key::AltLeft
                        | Key::AltRight
                        | Key::SuperLeft
                        | Key::SuperRight
                ) {
                    continue;
                }
                let name = match key {
                    Key::ArrowDown => "Down",
                    Key::ArrowUp => "Up",
                    Key::ArrowLeft => "Left",
                    Key::ArrowRight => "Right",
                    Key::Space if !modifiers.ctrl && !modifiers.alt && !modifiers.mac_cmd => {
                        continue;
                    }
                    Key::Comma => ",",
                    Key::Minus => "-",
                    Key::Period => ".",
                    Key::Slash => "/",
                    Key::Backslash => "\\",
                    Key::Semicolon => ";",
                    Key::Quote => "'",
                    Key::OpenBracket => "[",
                    Key::CloseBracket => "]",
                    Key::Backtick => "`",
                    Key::Equals => "=",
                    Key::Plus => "+",
                    _ => key.name(),
                };
                let special = matches!(
                    key,
                    Key::Space
                        | Key::Tab
                        | Key::Enter
                        | Key::Backspace
                        | Key::Delete
                        | Key::Insert
                        | Key::Home
                        | Key::End
                        | Key::PageUp
                        | Key::PageDown
                        | Key::ArrowDown
                        | Key::ArrowUp
                        | Key::ArrowLeft
                        | Key::ArrowRight
                ) || name.starts_with('F') && name[1..].parse::<u8>().is_ok();
                if special || modifiers.ctrl || modifiers.alt || modifiers.mac_cmd {
                    let mut prefix = String::new();
                    if modifiers.ctrl || modifiers.mac_cmd {
                        prefix.push_str("C-");
                    }
                    if modifiers.alt {
                        prefix.push_str("A-");
                    }
                    if modifiers.shift {
                        prefix.push_str("S-");
                    }
                    let name = if special {
                        name.to_owned()
                    } else {
                        name.to_lowercase()
                    };
                    record(state, format!("<{prefix}{name}>"));
                }
            }
            _ => {}
        }
    }
    input.events.retain(|event| {
        !matches!(
            event,
            Event::Key { .. } | Event::Text(_) | Event::Copy | Event::Cut | Event::Paste(_)
        )
    });
}

/// Called before Settings Escape/shortcut handling, while this viewport owns input.
pub(super) fn poll(ctx: &egui::Context, active_editor: bool) {
    let mut state = state(ctx);
    if !active_editor {
        state.recording = false;
    }
    ctx.input_mut(|input| consume(&mut state, input));
    store(ctx, state);
}

pub(super) fn recording(ctx: &egui::Context) -> bool {
    state(ctx).recording
}

pub(super) fn controls(ui: &mut egui::Ui, keys: &mut String) {
    let mut state = state(ui.ctx());
    if state.changed {
        *keys = state.keys.join(" ");
        state.changed = false;
    }
    let recording = state.recording;
    crate::ui_foundation::SettingsRow::show(
        ui,
        "Клавиши",
        "Нажимайте последовательно, например g, затем l. Ctrl/Alt/Shift записываются как сочетание. Esc отменяет запись.",
        |ui| {
            ui.label(if keys.is_empty() {
                "Не назначено"
            } else {
                keys.as_str()
            });
            if recording {
                if ui.button("Готово").clicked() {
                    state.recording = false;
                }
                ui.weak(format!("Запись · {}/8", state.keys.len()));
            } else if ui.button("Записать клавиши").clicked() {
                state.original = keys.split_whitespace().map(str::to_owned).collect();
                state.keys.clear();
                state.recording = true;
                state.changed = false;
                state.error = None;
                *keys = String::new();
                ui.ctx().memory_mut(|memory| {
                    if let Some(focus) = memory.focused() {
                        memory.surrender_focus(focus);
                    }
                });
            }
        },
    );
    if let Some(error) = &state.error {
        crate::ui_foundation::message(ui, error, true);
    }
    if !recording {
        ui.collapsing("Ввести обозначения вручную", |ui| {
            ui.add(egui::TextEdit::singleline(keys).desired_width(ui.available_width()));
        });
    }
    store(ui.ctx(), state);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(key: Key, modifiers: egui::Modifiers, repeat: bool) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers,
        }
    }
    fn feed(initial: State, events: Vec<Event>) -> State {
        let ctx = egui::Context::default();
        store(&ctx, initial);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            // Exercise the recorder's post-backend input boundary, including
            // a repeat event whose initial press occurred in an earlier frame.
            ui.input_mut(|input| input.events = events.clone());
            poll(ui.ctx(), true);
            ui.input_mut(|input| {
                assert!(!input.events.iter().any(|event| matches!(
                    event,
                    Event::Key { .. } | Event::Text(_) | Event::Copy | Event::Cut | Event::Paste(_)
                )));
                assert!(!input.consume_key(egui::Modifiers::NONE, Key::Escape));
            });
        });
        output.textures_delta.clear();
        state(&ctx)
    }
    #[test]
    fn native_key_and_text_pairs_record_sequences_once_and_chords_separately() {
        let next = feed(
            State {
                recording: true,
                ..Default::default()
            },
            vec![
                key(Key::G, egui::Modifiers::NONE, false),
                Event::Text("g".into()),
                key(Key::L, egui::Modifiers::NONE, false),
                Event::Text("l".into()),
                key(Key::A, egui::Modifiers::CTRL, false),
                Event::Copy,
                Event::Paste("ignored".into()),
                key(Key::ArrowRight, egui::Modifiers::ALT, false),
            ],
        );
        assert_eq!(next.keys, ["g", "l", "<C-a>", "<A-Right>"]);
    }
    #[test]
    fn escape_restores_original_sequence_and_consumes_remaining_input() {
        let next = feed(
            State {
                recording: true,
                original: vec!["g".into(), "o".into()],
                keys: vec!["x".into()],
                ..Default::default()
            },
            vec![
                key(Key::Escape, egui::Modifiers::NONE, false),
                Event::Text("ignored".into()),
            ],
        );
        assert!(!next.recording);
        assert_eq!(next.keys, ["g", "o"]);
        assert!(next.changed);
    }
    #[test]
    fn reserved_chords_repeats_and_sequence_limit_are_explicit() {
        let next = feed(
            State {
                recording: true,
                ..Default::default()
            },
            vec![
                key(Key::Tab, egui::Modifiers::SHIFT, false),
                key(Key::K, egui::Modifiers::CTRL, false),
            ],
        );
        assert!(next.keys.is_empty());
        assert!(next.error.is_some());
        let next = feed(
            State {
                recording: true,
                ..Default::default()
            },
            vec![
                key(Key::A, egui::Modifiers::NONE, true),
                Event::Text("a".into()),
            ],
        );
        assert!(next.keys.is_empty());
        let next = feed(
            State {
                recording: true,
                ..Default::default()
            },
            vec![Event::Text("abcdefghx".into())],
        );
        assert_eq!(next.keys.len(), 8);
        assert!(next.error.unwrap().contains('8'));
    }
    #[test]
    fn shifted_layout_text_and_modified_punctuation_use_yazi_notation() {
        let next = feed(
            State {
                recording: true,
                ..Default::default()
            },
            vec![
                key(Key::G, egui::Modifiers::SHIFT, false),
                Event::Text("Ж".into()),
                key(Key::Comma, egui::Modifiers::ALT, false),
                key(Key::Space, egui::Modifiers::CTRL, false),
            ],
        );
        assert_eq!(next.keys, ["Ж", "<A-,>", "<C-Space>"]);
    }
    #[test]
    fn lifecycle_cancel_restores_keys_and_different_rows_never_resume_recording() {
        let ctx = egui::Context::default();
        store(
            &ctx,
            State {
                anchor: "row-a".into(),
                recording: true,
                original: vec!["g".into(), "l".into()],
                keys: vec!["x".into()],
                ..Default::default()
            },
        );
        let mut keys = "x".to_owned();
        cancel(&ctx, &mut keys); // SelectSettings away or CloseSettings.
        assert_eq!(keys, "g l");
        assert!(!recording(&ctx));
        open(&ctx, "row-b".into());
        assert!(anchored(&ctx, "row-b"));
        assert!(!recording(&ctx));
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![Event::Text("search".into())],
                ..Default::default()
            },
            |ui| {
                poll(ui.ctx(), false);
                assert!(ui.input(|input| {
                    input
                        .events
                        .iter()
                        .any(|event| matches!(event, Event::Text(text) if text == "search"))
                }));
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn inline_recording_controls_fit_below_the_selected_row() {
        for width in [240.0, 480.0, 720.0, 1100.0] {
            for ppp in [1.0, 1.5, 2.0] {
                let ctx = egui::Context::default();
                crate::ui_style::install(&ctx);
                open(&ctx, "selected-row".into());
                let mut input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 1800.0),
                    )),
                    ..Default::default()
                };
                input
                    .viewports
                    .get_mut(&egui::ViewportId::ROOT)
                    .unwrap()
                    .native_pixels_per_point = Some(ppp);
                let mut output = ctx.run_ui(input, |ui| {
                    crate::ui_foundation::SettingsSection::show(ui, |ui| {
                        let row = ui
                            .vertical(|ui| {
                                crate::ui_foundation::command_row(
                                    ui,
                                    "Длинное название выбранной команды файлового менеджера",
                                    "g → l · Файлы и навигация",
                                    |ui| {
                                        ui.button("Изменить").clicked();
                                    },
                                )
                            })
                            .response
                            .rect;
                        let editor = ui
                            .vertical(|ui| controls(ui, &mut "g l".to_owned()))
                            .response
                            .rect;
                        assert!(editor.top() >= row.bottom());
                        assert!(editor.right() <= width + 0.5, "{width}/{ppp}: {editor:?}");
                    });
                });
                output.textures_delta.clear();
            }
        }
    }
}
