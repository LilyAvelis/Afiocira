//! Centered Everything search; one bounded worker, stale replies never navigate.
use super::*;
use photo_core::api::SearchState;
use std::sync::mpsc::{SyncSender, TrySendError};

struct Request {
    revision: u64,
    query: String,
    ctx: egui::Context,
}
struct Reply {
    revision: u64,
    result: Result<everything::SearchResults, String>,
}
pub(super) struct State {
    pub snapshot: SearchState,
    revision: u64,
    due: Option<Instant>,
    sender: SyncSender<Request>,
    receiver: Receiver<Reply>,
    focus: bool,
    scroll: bool,
    opening: Option<Receiver<Result<Gallery, String>>>,
    open_target: Option<(PathBuf, bool, Instant, u64, u64)>,
    opened_frame: u64,
}
impl Default for State {
    fn default() -> Self {
        let (sender, requests) = mpsc::sync_channel::<Request>(1);
        let (replies, receiver) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while let Ok(request) = requests.recv() {
                let result = everything::search(&request.query);
                if replies
                    .send(Reply {
                        revision: request.revision,
                        result,
                    })
                    .is_err()
                {
                    break;
                }
                request.ctx.request_repaint();
            }
        });
        Self {
            snapshot: SearchState::default(),
            revision: 0,
            due: None,
            sender,
            receiver,
            focus: false,
            scroll: false,
            opening: None,
            open_target: None,
            opened_frame: 0,
        }
    }
}
impl Workshop {
    pub(super) fn open_search(&mut self) -> Result<(), String> {
        if self.image_dialog.is_some()
            || self.rename_dialog.is_some()
            || self.delete_dialog.is_some()
        {
            return Err("Сначала завершите текущий диалог".into());
        }
        self.close_palette();
        self.search.snapshot.open = true;
        self.search.opened_frame = self.ctx.cumulative_frame_nr();
        if !self.search.snapshot.query.trim().is_empty() {
            self.search_files(self.search.snapshot.query.clone())?;
        }
        self.search.focus = true;
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        Ok(())
    }
    pub(super) fn close_search(&mut self) {
        self.search.snapshot.open = false;
        self.search.revision = self.search.revision.wrapping_add(1);
        self.search.due = None;
        self.search.snapshot.loading = false;
        self.search.snapshot.opening = false;
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }
    pub(super) fn search_files(&mut self, query: String) -> Result<(), String> {
        if self.search.snapshot.opening {
            return Err("Дождитесь открытия папки или нажмите Esc".into());
        }
        if !self.search.snapshot.open {
            self.open_search()?;
        }
        if query.len() > 8192 {
            return Err("Слишком длинный запрос".into());
        }
        self.search.revision = self.search.revision.wrapping_add(1);
        self.search.snapshot.query = query;
        self.search.snapshot.results.clear();
        self.search.snapshot.selected = 0;
        self.search.snapshot.total = 0;
        self.search.snapshot.error = None;
        let nonempty = !self.search.snapshot.query.trim().is_empty();
        self.search.snapshot.loading = nonempty;
        self.search.due = nonempty.then(|| Instant::now() + Duration::from_millis(120));
        self.ctx.request_repaint_after(Duration::from_millis(120));
        Ok(())
    }
    pub(super) fn poll_search(&mut self) {
        if let Some(receiver) = &self.search.opening {
            match receiver.try_recv() {
                Ok(result) => {
                    self.search.opening = None;
                    self.search.snapshot.opening = false;
                    if let Some((path, nonmedia, started, revision, interaction)) =
                        self.search.open_target.take()
                        && revision == self.search.revision
                        && interaction == self.interaction_revision
                        && self.search.snapshot.open
                    {
                        self.search.snapshot.opening = false;
                        match result {
                            Ok(gallery) => {
                                self.install_gallery(gallery);
                                self.interaction_revision =
                                    self.interaction_revision.wrapping_add(1);
                                self.active_pane = ActivePane::Gallery;
                                self.close_search();
                                self.show_yazi = true;
                                if let Some(yazi) = &mut self.yazi {
                                    if let Err(error) = yazi.navigate(&path) {
                                        self.message = error;
                                    }
                                    if nonmedia {
                                        self.active_pane = ActivePane::Yazi;
                                        yazi.request_focus();
                                    }
                                }
                                self.search.snapshot.open_ms =
                                    Some(started.elapsed().as_secs_f64() * 1000.0);
                            }
                            Err(error) => self.search.snapshot.error = Some(error),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.search.opening = None;
                    self.search.snapshot.opening = false;
                    self.search.snapshot.error = Some("Не удалось открыть папку".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        while let Ok(reply) = self.search.receiver.try_recv() {
            if reply.revision != self.search.revision || !self.search.snapshot.open {
                continue;
            }
            self.search.snapshot.loading = false;
            match reply.result {
                Ok(result) => {
                    self.search.snapshot.results = result.items;
                    self.search.snapshot.total = result.total;
                }
                Err(error) => self.search.snapshot.error = Some(error),
            }
        }
        if self.search.due.is_some_and(|due| Instant::now() >= due) {
            let request = Request {
                revision: self.search.revision,
                query: self.search.snapshot.query.clone(),
                ctx: self.ctx.clone(),
            };
            match self.search.sender.try_send(request) {
                Ok(()) => self.search.due = None,
                Err(TrySendError::Full(_)) => {
                    self.ctx.request_repaint_after(Duration::from_millis(20))
                }
                Err(TrySendError::Disconnected(_)) => {
                    self.search.due = None;
                    self.search.snapshot.loading = false;
                    self.search.snapshot.error = Some("Поиск недоступен: worker завершился".into());
                }
            }
        }
    }
    pub(super) fn open_search_result(&mut self, index: usize) -> Result<(), String> {
        if !self.search.snapshot.open
            || self.search.snapshot.loading
            || self.search.snapshot.opening
        {
            return Err("Дождитесь результатов поиска".into());
        }
        let result = self
            .search
            .snapshot
            .results
            .get(index)
            .ok_or("Нет такого результата")?
            .clone();
        if !result.path.exists() {
            return Err("Файл больше не существует; повторите поиск".into());
        }
        let media = photo_core::is_media_path(&result.path);
        let target = if result.is_folder || media {
            result.path.clone()
        } else {
            result
                .path
                .parent()
                .ok_or("Нет родительской папки")?
                .to_path_buf()
        };
        if self.search.opening.is_some() {
            return Err("Предыдущее открытие ещё завершается".into());
        }
        let (sender, receiver) = mpsc::channel();
        let ctx = self.ctx.clone();
        self.search.snapshot.opening = true;
        self.search.open_target = Some((
            result.path,
            !result.is_folder && !media,
            Instant::now(),
            self.search.revision,
            self.interaction_revision,
        ));
        self.search.opening = Some(receiver);
        std::thread::spawn(move || {
            let result = if target.is_file() {
                Gallery::open_file(&target)
            } else {
                Gallery::open(&target)
            }
            .map_err(|e| e.to_string());
            let _ = sender.send(result);
            ctx.request_repaint();
        });
        Ok(())
    }
    pub(super) fn search_popup(&mut self, ctx: &egui::Context) {
        if !self.search.snapshot.open {
            return;
        }
        let count = self.search.snapshot.results.len();
        let mut picked = None;
        let tab = ctx.input_mut(delete_action::take_tab).is_some();
        let armed = ctx.cumulative_frame_nr() > self.search.opened_frame;
        ctx.input_mut(|i| {
            if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) && count > 0 {
                self.search.snapshot.selected = (self.search.snapshot.selected + 1) % count;
                self.search.scroll = true;
            }
            if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) && count > 0 {
                self.search.snapshot.selected = (self.search.snapshot.selected + count - 1) % count;
                self.search.scroll = true;
            }
            if delete_action::take_fresh_press(i, Key::Enter) && armed && count > 0 {
                picked = Some(self.search.snapshot.selected);
            }
        });
        let mut query = self.search.snapshot.query.clone();
        let width = (ctx.content_rect().width() - 48.0).clamp(220.0, 700.0);
        let modal = egui::Modal::new(egui::Id::new("everything-search"))
            .frame(
                egui::Frame::popup(&ctx.style_of(egui::Theme::Dark))
                    .fill(Color32::from_rgb(59, 59, 64))
                    .corner_radius(12)
                    .inner_margin(16),
            )
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.add(Lucide::Search.size(20.0).image());
                    ui.label(egui::RichText::new("Everything").strong().size(18.0));
                });
                ui.add_space(8.0);
                let field = ui.add(
                    egui::TextEdit::singleline(&mut query)
                        .id(egui::Id::new("everything-query"))
                        .hint_text("Найти файл или папку…")
                        .desired_width(f32::INFINITY),
                );
                if self.search.focus || tab {
                    field.request_focus();
                    self.search.focus = false;
                }
                ui.separator();
                if self.search.snapshot.opening {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Открываем папку…");
                    });
                }
                egui::ScrollArea::vertical()
                    .max_height((ctx.content_rect().height() - 240.0).clamp(80.0, 420.0))
                    .show(ui, |ui| {
                        for (index, item) in self.search.snapshot.results.iter().enumerate() {
                            let row = crate::search_icons::result_row(
                                ui,
                                item,
                                &friendly_path(&item.path),
                                index == self.search.snapshot.selected,
                            );
                            if index == self.search.snapshot.selected && self.search.scroll {
                                row.scroll_to_me(Some(egui::Align::Center));
                                self.search.scroll = false;
                            }
                            if row.clicked() {
                                picked = Some(index);
                            }
                        }
                        if self.search.snapshot.loading {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Ищем…");
                            });
                        } else if let Some(error) = &self.search.snapshot.error {
                            ui.colored_label(Color32::LIGHT_RED, error);
                        } else if count == 0 {
                            ui.label(if query.trim().is_empty() {
                                "Имена, пути и синтаксис Everything"
                            } else {
                                "Ничего не найдено"
                            });
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.weak(format!(
                        "Показано {} из {}",
                        count, self.search.snapshot.total
                    ));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        crate::ui_style::keycap(ui, "Esc");
                        crate::ui_style::keycap(ui, "Enter · открыть");
                    });
                });
            });
        if query != self.search.snapshot.query && !self.search.snapshot.opening {
            let _ = self.search_files(query);
            picked = None;
        }
        if let Some(index) = picked {
            if let Err(error) = self.open_search_result(index) {
                self.search.snapshot.error = Some(error);
            }
        } else if modal.should_close() {
            self.close_search();
        }
    }
}
