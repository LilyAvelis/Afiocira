//! Raycast-style footer and bottom-right action menu; search stays at the bottom.
use super::*;
use photo_core::actions::{ActionId, ActionInfo};

/// Shared SVG icon vocabulary for every action surface. Keep this exhaustive
/// so a new command cannot silently inherit an unrelated navigation arrow.
pub(super) fn icon(action: ActionId) -> Lucide {
    match action {
        ActionId::Metadata => Lucide::Tags,
        ActionId::History => Lucide::Clock,
        ActionId::Undo => Lucide::Undo2,
        ActionId::Redo => Lucide::Redo2,
        ActionId::CommitHistory => Lucide::Save,
        ActionId::AdjustImage | ActionId::AdjustDetails | ActionId::AdjustVignette => {
            Lucide::SlidersHorizontal
        }
        ActionId::RotateLeft => Lucide::RotateCcw,
        ActionId::RotateRight => Lucide::RotateCw,
        ActionId::FlipHorizontal => Lucide::FlipHorizontal2,
        ActionId::FlipVertical => Lucide::FlipVertical2,
        ActionId::EnterAlbum => Lucide::FolderOpen,
        ActionId::CopyVideoFrame => Lucide::Copy,
        ActionId::SaveVideoFrame => Lucide::Image,
        ActionId::Copy | ActionId::CopyPath | ActionId::CopySelectedPath => Lucide::Copy,
        ActionId::Duplicate => Lucide::CopyPlus,
        ActionId::Paste => Lucide::ClipboardPaste,
        ActionId::Rename => Lucide::Pencil,
        ActionId::Delete => Lucide::Trash2,
        ActionId::Search => Lucide::Search,
        ActionId::Settings => Lucide::Settings,
        ActionId::ResizeImage => Lucide::Scaling,
        ActionId::ConvertImage => Lucide::Replace,
        ActionId::Open => Lucide::Image,
        ActionId::Back => Lucide::Images,
        ActionId::Next => Lucide::ArrowRight,
        ActionId::Previous => Lucide::ArrowLeft,
        ActionId::ClearMarks => Lucide::ListX,
        ActionId::Refresh => Lucide::RefreshCw,
        ActionId::Fit => Lucide::Maximize2,
        ActionId::PlayPause => Lucide::Play,
    }
}

#[derive(Default)]
pub(super) struct State {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub focus_search: bool,
    pub scroll_selected: bool,
    pub enter_armed: bool,
    shortcut: Option<InlineShortcut>,
    shortcut_release: bool,
    shortcut_selection: Option<(ActionId, String)>,
}

struct InlineShortcut {
    action: ActionId,
    context: String,
    label: String,
    binding: Option<String>,
    error: Option<String>,
    replaceable: bool,
    current_binding: String,
    selected_button: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShortcutChoice {
    Replace,
    Retry,
    Clear,
    Cancel,
}
enum ShortcutInput {
    Binding(String),
    Choice(ShortcutChoice),
}
impl InlineShortcut {
    fn assignment_failed(&mut self, binding: String, error: String, replaceable: bool) {
        self.replaceable = replaceable;
        self.binding = replaceable.then_some(binding);
        self.error = Some(error);
        self.selected_button = replaceable.then_some(0);
    }
    fn choices(&self) -> Vec<ShortcutChoice> {
        let mut choices = Vec::new();
        if self.replaceable {
            choices.push(ShortcutChoice::Replace);
        }
        if self.binding.is_some() {
            choices.push(ShortcutChoice::Retry);
        }
        if !self.current_binding.is_empty() {
            choices.push(ShortcutChoice::Clear);
        }
        choices.push(ShortcutChoice::Cancel);
        choices
    }
    fn input(&mut self, input: &mut egui::InputState) -> Option<ShortcutInput> {
        let binding = match shortcuts::take_recorded_chord(input)? {
            None => return Some(ShortcutInput::Choice(ShortcutChoice::Cancel)),
            Some(binding) if binding.is_empty() => {
                return Some(ShortcutInput::Choice(ShortcutChoice::Clear));
            }
            Some(binding) => binding,
        };
        let chord = shortcuts::parse(&binding).ok()?.into_iter().next()?;
        let controls = self.binding.is_some() || self.selected_button.is_some();
        let backwards = matches!(chord.logical_key, Key::ArrowLeft | Key::ArrowUp)
            || (chord.logical_key == Key::Tab && chord.modifiers.shift);
        let navigation =
            (chord.logical_key == Key::Tab && !chord.modifiers.ctrl && !chord.modifiers.alt)
                || (controls
                    && chord.modifiers.is_none()
                    && matches!(
                        chord.logical_key,
                        Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown
                    ));
        if navigation {
            let count = self.choices().len();
            self.selected_button = Some(match self.selected_button {
                Some(index) if backwards => (index + count - 1) % count,
                Some(index) => (index + 1) % count,
                None if backwards => count - 1,
                None => 0,
            });
            return None;
        }
        if controls && chord.logical_key == Key::Enter && chord.modifiers.is_none() {
            return Some(ShortcutInput::Choice(
                self.choices()[self.selected_button.unwrap_or(0)],
            ));
        }
        self.binding
            .is_none()
            .then_some(ShortcutInput::Binding(binding))
    }
    fn render(&mut self, ui: &mut egui::Ui) -> Option<ShortcutChoice> {
        ui.strong(format!("Клавиша · {}", self.label));
        ui.weak(match self.context.as_str() {
            "space" => "Просмотр файла",
            "albums" => "Альбомы",
            _ => "Галерея",
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Текущее сочетание:");
            if self.current_binding.is_empty() {
                ui.weak("не назначено");
            } else {
                crate::ui_style::keycap(ui, &self.current_binding);
            }
        });
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
            if !self.replaceable {
                ui.weak("Нажмите другое сочетание, чтобы повторить ввод.");
            }
        } else {
            ui.label("Нажмите сочетание клавиш. Оно сохранится сразу.");
        }
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            for (index, choice) in self.choices().into_iter().enumerate() {
                let label = match choice {
                    ShortcutChoice::Replace => "Заменить назначение",
                    ShortcutChoice::Retry => "Другая клавиша",
                    ShortcutChoice::Clear => "Снять назначение · Ctrl Backspace",
                    ShortcutChoice::Cancel => "Отмена · Esc",
                };
                let response = ui.add(
                    egui::Button::new(label)
                        .selected(self.selected_button == Some(index))
                        .wrap(),
                );
                if response.clicked() {
                    picked = Some(choice);
                }
            }
        });
        ui.weak("Tab — к кнопкам · стрелки — выбор · Enter — подтвердить");
        ui.separator();
        picked
    }
}

fn take_assignment_key(input: &mut egui::InputState) -> bool {
    let pressed = input.events.iter().any(|event| {
        matches!(event,
        egui::Event::Key { key: Key::Comma, pressed: true, repeat: false, modifiers, .. }
        if modifiers.ctrl && modifiers.shift && !modifiers.alt && !modifiers.mac_cmd)
    });
    if pressed {
        shortcuts::discard_recording_events(input);
    }
    pressed
}

fn take_enter(i: &mut egui::InputState, opening: bool, armed: &mut bool) -> bool {
    let fresh_enter = i.events.iter().any(|event| {
        matches!(event,
        egui::Event::Key { key: Key::Enter, pressed: true, repeat: false, modifiers, .. }
        if *modifiers == egui::Modifiers::NONE)
    });
    if opening {
        *armed = !i.key_down(Key::Enter) && !fresh_enter;
    }
    let enter = i.consume_key(egui::Modifiers::NONE, Key::Enter) && fresh_enter && *armed;
    // An Enter which opened a popup must be released before it can
    // activate its selected command. Repeated key events never run it.
    if !i.key_down(Key::Enter) && !fresh_enter {
        *armed = true;
    }
    enter
}

impl Workshop {
    fn begin_palette_shortcut(&mut self) {
        self.palette.shortcut_release = true;
        let Some(action) = self.filtered_actions().get(self.palette.selected).cloned() else {
            return;
        };
        let context = self.shortcut_context().to_owned();
        let editable = shortcuts::catalogue(&self.settings.shortcuts)
            .iter()
            .any(|entry| entry.action == action.id && entry.context == context && entry.rebindable);
        if !editable {
            self.message = "Эта клавиша закреплена в Afio".into();
            return;
        }
        self.settings.shortcut_candidate = None;
        self.palette.shortcut = Some(InlineShortcut {
            action: action.id,
            context,
            label: action.label,
            binding: None,
            error: None,
            replaceable: false,
            current_binding: action.shortcut,
            selected_button: None,
        });
        self.palette.shortcut_release = true;
    }

    /// Called before global shortcuts, including the Ctrl+, Settings shortcut.
    pub(super) fn palette_shortcut_input(&mut self, ctx: &egui::Context) -> bool {
        if !self.palette.open {
            if self.palette.shortcut.take().is_some() {
                self.settings.shortcut_candidate = None;
                self.settings.error = None;
                self.palette.shortcut_release = true;
            }
            self.palette.shortcut_selection = None;
        }
        if self.palette.shortcut_release {
            let released = ctx.input_mut(|input| {
                let released = input.keys_down.is_empty();
                shortcuts::discard_recording_events(input);
                released
            });
            self.palette.shortcut_release = !released;
            return true;
        }
        if let Some(capture) = self.palette.shortcut.as_mut() {
            let action = capture.action;
            let context = capture.context.clone();
            let captured = ctx.input_mut(|input| capture.input(input));
            match captured {
                Some(ShortcutInput::Choice(choice)) => self.palette_shortcut_choice(choice),
                Some(ShortcutInput::Binding(binding)) => {
                    self.apply_palette_shortcut(action, context, binding, false);
                }
                _ => {}
            }
            return true;
        }
        if self.palette.open && ctx.input_mut(take_assignment_key) {
            self.begin_palette_shortcut();
            return true;
        }
        false
    }

    fn apply_palette_shortcut(
        &mut self,
        action: ActionId,
        context: String,
        binding: String,
        replace: bool,
    ) {
        match self.set_shortcut(action, context, Some(binding.clone()), false, replace) {
            Ok(()) => {
                self.palette.shortcut = None;
                self.palette.shortcut_release = true;
                self.palette.enter_armed = false;
                // A query may contain the previous binding. Keep its selected
                // action visible until the user changes that query.
                self.palette.shortcut_selection = Some((action, self.palette.query.clone()));
                if let Some(index) = self
                    .filtered_actions()
                    .iter()
                    .position(|entry| entry.id == action)
                {
                    self.palette.selected = index;
                }
            }
            Err(error) => {
                if let Some(capture) = &mut self.palette.shortcut {
                    capture.assignment_failed(
                        binding,
                        error,
                        self.settings.shortcut_candidate.is_some(),
                    );
                }
                self.palette.shortcut_release = true;
            }
        }
    }

    fn palette_shortcut_editor(&mut self, ui: &mut egui::Ui) {
        if let Some(choice) = self
            .palette
            .shortcut
            .as_mut()
            .and_then(|capture| capture.render(ui))
        {
            self.palette_shortcut_choice(choice);
        }
    }
    fn palette_shortcut_choice(&mut self, choice: ShortcutChoice) {
        let Some(capture) = self.palette.shortcut.as_ref() else {
            return;
        };
        let action = capture.action;
        let context = capture.context.clone();
        match choice {
            ShortcutChoice::Replace if capture.replaceable => {
                let binding = capture.binding.clone().unwrap_or_default();
                self.apply_palette_shortcut(action, context, binding, true);
            }
            ShortcutChoice::Clear => {
                self.apply_palette_shortcut(action, context, String::new(), false)
            }
            ShortcutChoice::Retry => {
                if let Some(capture) = &mut self.palette.shortcut {
                    capture.binding = None;
                    capture.error = None;
                    capture.replaceable = false;
                    capture.selected_button = None;
                }
                self.settings.shortcut_candidate = None;
                self.settings.error = None;
                self.palette.shortcut_release = true;
            }
            ShortcutChoice::Cancel => {
                self.palette.shortcut = None;
                self.settings.shortcut_candidate = None;
                self.settings.error = None;
                self.palette.shortcut_release = true;
            }
            _ => {}
        }
    }
    pub(super) fn filtered_actions(&self) -> Vec<ActionInfo> {
        let words: Vec<_> = self
            .palette
            .query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        self.available_actions()
            .into_iter()
            .filter(|a| {
                let haystack =
                    format!("{} {} {:?} {}", a.label, a.group, a.id, a.shortcut).to_lowercase();
                words.iter().all(|word| haystack.contains(word))
                    || self
                        .palette
                        .shortcut_selection
                        .as_ref()
                        .is_some_and(|(id, query)| *id == a.id && *query == self.palette.query)
            })
            .collect()
    }
    pub(super) fn palette_snapshot(&self) -> photo_core::api::PaletteState {
        photo_core::api::PaletteState {
            open: self.palette.open,
            mode: if self
                .gallery
                .as_ref()
                .is_some_and(|g| g.view() == View::Photo)
            {
                "space"
            } else {
                "gallery"
            }
            .into(),
            query: self.palette.query.clone(),
            selected: self.palette.selected,
        }
    }
    pub(super) fn actions_footer(&mut self, ui: &mut egui::Ui) {
        let mut toggle = false;
        let mut picked = None;
        let actions = self.available_actions();
        egui::Panel::bottom("actions-footer")
            .exact_size(46.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(48, 48, 52))
                    .inner_margin(egui::Margin::symmetric(14, 7)),
            )
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add(Lucide::Images.size(17.0).image());
                    let count = self.action_paths().len();
                    let label = if self.operation.as_ref().is_some_and(|j| j.running) {
                        "Выполняется…".to_owned()
                    } else if self
                        .gallery
                        .as_ref()
                        .is_some_and(|g| g.view() == View::Photo)
                    {
                        "Space · текущий файл".into()
                    } else {
                        format!("Gallery · выбрано {count}")
                    };
                    ui.label(egui::RichText::new(label).color(Color32::from_gray(180)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        toggle = crate::ui_style::actions_button(ui, self.palette.open).clicked();
                        for (id, label) in [
                            (ActionId::Metadata, "Metadata"),
                            (ActionId::History, "History"),
                        ] {
                            if let Some(action) = actions.iter().find(|a| a.id == id) {
                                let shortcut = action.shortcut.replace("Ctrl ", "Ctrl+");
                                if ui
                                    .add(egui::Button::image_and_text(
                                        icon(id).size(15.0).image(),
                                        format!("{label} · {shortcut}"),
                                    ))
                                    .clicked()
                                {
                                    picked = Some(id);
                                }
                            }
                        }
                    });
                });
            });
        if toggle {
            let command = if self.palette.open {
                Command::CloseCommands
            } else {
                Command::ShowCommands
            };
            if let Some(error) = self.execute(command).error {
                self.message = error.message;
            }
        }
        if let Some(id) = picked
            && let Some(error) = self.execute(Command::RunAction { id }).error
        {
            self.message = error.message;
        }
    }
    pub(super) fn actions_popup(&mut self, ctx: &egui::Context) {
        if !self.palette.open {
            return;
        }
        let actions = self.filtered_actions();
        self.palette.selected = self.palette.selected.min(actions.len().saturating_sub(1));
        let mut picked = None;
        let opening = self.palette.focus_search;
        let mut refocus_search = false;
        let enter = !self.palette.shortcut_release
            && self.palette.shortcut.is_none()
            && ctx.input_mut(|i| {
                let enter = take_enter(i, opening, &mut self.palette.enter_armed);
                refocus_search = i.consume_key(egui::Modifiers::NONE, Key::Tab)
                    | i.consume_key(egui::Modifiers::SHIFT, Key::Tab);
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) && !actions.is_empty() {
                    self.palette.selected = (self.palette.selected + 1) % actions.len();
                    self.palette.scroll_selected = true;
                }
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) && !actions.is_empty() {
                    self.palette.selected =
                        (self.palette.selected + actions.len() - 1) % actions.len();
                    self.palette.scroll_selected = true;
                }
                enter
            });
        let width = (ctx.content_rect().width() - 28.0).clamp(240.0, 420.0);
        let area = egui::Modal::default_area(egui::Id::new("action-palette"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -58.0));
        let result = egui::Modal::new(egui::Id::new("action-palette"))
            .area(area)
            .backdrop_color(Color32::from_black_alpha(24))
            .frame(
                egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                    .fill(Color32::from_rgb(69, 69, 74))
                    .stroke(egui::Stroke::new(1.0, Color32::from_gray(118)))
                    .corner_radius(12)
                    .inner_margin(12),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                self.palette_shortcut_editor(ui);
                if self.palette.shortcut.is_none() {
                    ui.label(
                        egui::RichText::new(
                            if self
                                .gallery
                                .as_ref()
                                .is_some_and(|g| g.view() == View::Photo)
                            {
                                "Действия с файлом"
                            } else {
                                "Действия с выделением и папкой"
                            },
                        )
                        .strong()
                        .size(13.0),
                    );
                    ui.add_space(2.0);
                    egui::ScrollArea::vertical()
                        .max_height((ctx.content_rect().height() - 180.0).clamp(80.0, 320.0))
                        .show(ui, |ui| {
                            let mut group = String::new();
                            for (index, action) in actions.iter().enumerate() {
                                if group != action.group {
                                    group = action.group.clone();
                                    ui.add_space(3.0);
                                    ui.label(
                                        egui::RichText::new(&group)
                                            .size(11.0)
                                            .color(Color32::from_gray(185)),
                                    );
                                }
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(ui.available_width(), 38.0),
                                    egui::Sense::click(),
                                );
                                if index == self.palette.selected || response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        7.0,
                                        Color32::from_rgb(100, 100, 108),
                                    );
                                }
                                if index == self.palette.selected && self.palette.scroll_selected {
                                    self.palette.scroll_selected = false;
                                    response.scroll_to_me(Some(egui::Align::Center));
                                }
                                ui.scope_builder(
                                    egui::UiBuilder::new()
                                        .max_rect(rect.shrink2(egui::vec2(9.0, 3.0)))
                                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                    |ui| {
                                        let icon = icon(action.id);
                                        if action.id == ActionId::Delete {
                                            ui.visuals_mut().override_text_color =
                                                Some(Color32::LIGHT_RED);
                                        }
                                        ui.add(icon.size(16.0).image());
                                        ui.label(
                                            egui::RichText::new(&action.label).size(14.0).color(
                                                if action.id == ActionId::Delete {
                                                    Color32::LIGHT_RED
                                                } else {
                                                    ui.visuals().text_color()
                                                },
                                            ),
                                        );
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if !action.shortcut.is_empty() {
                                                    crate::ui_style::keycap(ui, &action.shortcut);
                                                }
                                            },
                                        );
                                    },
                                );
                                if response.clicked() {
                                    picked = Some(action.id);
                                }
                            }
                            if actions.is_empty() {
                                ui.add_space(12.0);
                                ui.label("Ничего не найдено");
                                ui.add_space(12.0);
                            }
                            crate::scroll_navigation::apply(ui);
                        });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.add(Lucide::Search.size(16.0).image());
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.palette.query)
                                .hint_text("Поиск действий…")
                                .frame(egui::Frame::NONE)
                                .desired_width(width - 80.0),
                        );
                        if self.palette.focus_search || refocus_search {
                            response.request_focus();
                            self.palette.focus_search = false;
                        }
                        if response.changed() {
                            self.palette.shortcut_selection = None;
                            self.palette.selected = 0;
                            self.palette.scroll_selected = true;
                        }
                        crate::ui_style::keycap(ui, "Esc");
                    });
                    let editable = actions.get(self.palette.selected).is_some_and(|action| {
                        shortcuts::catalogue(&self.settings.shortcuts)
                            .iter()
                            .any(|entry| {
                                entry.action == action.id
                                    && entry.context == self.shortcut_context()
                                    && entry.rebindable
                            })
                    });
                    if ui
                        .add_enabled(
                            editable,
                            egui::Button::new("Назначить клавишу · Ctrl Shift ,").small(),
                        )
                        .clicked()
                    {
                        self.begin_palette_shortcut();
                    }
                }
            });
        // TextEdit processes this frame's text events during show(). Resolve
        // Enter against that current query, never the list rendered beforehand.
        if enter && picked.is_none() {
            picked = self
                .filtered_actions()
                .get(self.palette.selected)
                .map(|a| a.id);
        }
        if let Some(id) = picked {
            if let Some(error) = self.execute(Command::RunAction { id }).error {
                self.message = error.message;
            }
        } else if result.should_close() {
            if self.palette.shortcut.take().is_some() {
                self.settings.shortcut_candidate = None;
                self.settings.error = None;
                self.palette.shortcut_release = true;
            } else {
                self.close_palette();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn editor(conflict: bool) -> InlineShortcut {
        InlineShortcut { action: ActionId::Copy, context: "space".into(),
            label: "Копировать текущий кадр с очень длинным названием".into(),
            binding: conflict.then(|| "Ctrl H".into()),
            error: conflict.then(|| "Сочетание «Ctrl+H» уже назначено команде «История» (просмотр файла). Замена снимет его с этой команды.".into()),
            replaceable: conflict, current_binding: "Ctrl C".into(), selected_button: conflict.then_some(0) }
    }
    fn key(
        editor: &mut InlineShortcut,
        key: Key,
        modifiers: egui::Modifiers,
    ) -> Option<ShortcutInput> {
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: Some(key),
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        });
        let result = ctx.input_mut(|i| editor.input(i));
        assert!(!ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { .. }))
        }));
        ctx.end_pass().textures_delta.clear();
        result
    }
    #[test]
    fn conflict_buttons_support_arrows_enter_clear_and_escape_without_dispatch_leaks() {
        let mut editor = editor(true);
        assert!(key(&mut editor, Key::ArrowRight, egui::Modifiers::NONE).is_none());
        assert!(matches!(
            key(&mut editor, Key::Enter, egui::Modifiers::NONE),
            Some(ShortcutInput::Choice(ShortcutChoice::Retry))
        ));
        key(&mut editor, Key::ArrowDown, egui::Modifiers::NONE);
        assert!(matches!(
            key(&mut editor, Key::Enter, egui::Modifiers::NONE),
            Some(ShortcutInput::Choice(ShortcutChoice::Clear))
        ));
        key(&mut editor, Key::ArrowLeft, egui::Modifiers::NONE);
        key(&mut editor, Key::ArrowUp, egui::Modifiers::NONE);
        assert!(matches!(
            key(&mut editor, Key::Enter, egui::Modifiers::NONE),
            Some(ShortcutInput::Choice(ShortcutChoice::Replace))
        ));
        assert!(matches!(
            key(&mut editor, Key::Backspace, egui::Modifiers::CTRL),
            Some(ShortcutInput::Choice(ShortcutChoice::Clear))
        ));
        assert!(matches!(
            key(&mut editor, Key::Escape, egui::Modifiers::NONE),
            Some(ShortcutInput::Choice(ShortcutChoice::Cancel))
        ));
    }
    #[test]
    fn recording_keeps_arrows_bindable_until_tab_moves_to_buttons() {
        let mut editor = editor(false);
        assert!(matches!(
            key(&mut editor, Key::ArrowLeft, egui::Modifiers::NONE),
            Some(ShortcutInput::Binding(_))
        ));
        key(&mut editor, Key::Tab, egui::Modifiers::NONE);
        assert_eq!(editor.selected_button, Some(0));
        key(&mut editor, Key::ArrowRight, egui::Modifiers::NONE);
        assert!(matches!(
            key(&mut editor, Key::Enter, egui::Modifiers::NONE),
            Some(ShortcutInput::Choice(ShortcutChoice::Cancel))
        ));
    }
    #[test]
    fn validation_failure_keeps_accepting_letters_and_modified_chords() {
        let mut editor = editor(false);
        editor.assignment_failed(
            "Ctrl ControlLeft".into(),
            "Invalid legacy modifier".into(),
            false,
        );
        assert!(editor.binding.is_none() && editor.selected_button.is_none());
        assert!(
            matches!(key(&mut editor, Key::J, egui::Modifiers::NONE), Some(ShortcutInput::Binding(binding)) if binding == "J")
        );
        assert!(
            matches!(key(&mut editor, Key::J, egui::Modifiers::CTRL), Some(ShortcutInput::Binding(binding)) if binding == "Ctrl J")
        );
    }
    #[test]
    fn shortcut_editor_wraps_at_all_widths_and_dpi() {
        for width in [240.0, 420.0, 650.0] {
            for ppp in [1.0, 1.5, 2.0] {
                for conflict in [false, true] {
                    let ctx = egui::Context::default();
                    let mut editor = editor(conflict);
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
                        editor.render(ui);
                        assert!(
                            ui.min_rect().right() <= width + 1.0,
                            "width={width}, dpi={ppp}, rect={:?}, available={:?}",
                            ui.min_rect(),
                            ui.max_rect()
                        );
                    });
                    output.textures_delta.clear();
                }
            }
        }
    }

    #[test]
    fn inline_assignment_consumes_native_ctrl_shift_comma_before_settings() {
        let ctx = egui::Context::default();
        let modifiers = egui::Modifiers::CTRL | egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::Key {
                        key: Key::Comma,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    },
                    egui::Event::Text(",".into()),
                ],
                ..Default::default()
            },
            |ui| {
                ui.ctx().input_mut(|input| {
                    assert!(take_assignment_key(input));
                    assert!(!input.consume_key(egui::Modifiers::CTRL, Key::Comma));
                    assert!(input.events.is_empty());
                });
            },
        );
        output.textures_delta.clear();
    }

    #[test]
    fn opening_enter_and_repeats_require_release_then_fresh_press() {
        let ctx = egui::Context::default();
        let mut armed = false;
        let mut frame = |opening, pressed, repeat| {
            let mut fired = false;
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key: Key::Enter,
                    physical_key: None,
                    pressed,
                    repeat,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                fired = ui.ctx().input_mut(|i| take_enter(i, opening, &mut armed));
            });
            output.textures_delta.clear();
            fired
        };
        assert!(!frame(true, true, false));
        assert!(!frame(false, true, true));
        assert!(!frame(false, false, false));
        assert!(frame(false, true, false));
        assert!(!frame(false, true, true));
    }
}
