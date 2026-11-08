//! One short, non-interactive confirmation shared by clipboard text commands.
use eframe::egui;
use std::time::{Duration, Instant};
const LIFETIME: Duration = Duration::from_secs(2);
#[derive(Clone)]
struct State {
    enabled: bool,
    latest: Option<(String, Instant)>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            enabled: true,
            latest: None,
        }
    }
}
impl State {
    fn copied(&mut self, message: &str, now: Instant) {
        if self.enabled {
            self.latest = Some((message.chars().take(240).collect(), now));
        }
    }
    fn message(&mut self, now: Instant) -> Option<(String, Duration)> {
        let (message, start) = self.latest.as_ref()?;
        let elapsed = now.saturating_duration_since(*start);
        if elapsed >= LIFETIME {
            self.latest = None;
            return None;
        }
        Some((message.clone(), LIFETIME - elapsed))
    }
}
fn mutate<T>(ctx: &egui::Context, update: impl FnOnce(&mut State) -> T) -> T {
    ctx.data_mut(|data| {
        let id = egui::Id::new("copy-feedback-state");
        let mut state = data.get_temp::<State>(id).unwrap_or_default();
        let result = update(&mut state);
        data.insert_temp(id, state);
        result
    })
}
pub(crate) fn configure(ctx: &egui::Context, enabled: bool) {
    mutate(ctx, |state| {
        state.enabled = enabled;
        if !enabled {
            state.latest = None;
        }
    });
}
pub(crate) fn copied(ctx: &egui::Context, message: &str) {
    mutate(ctx, |state| state.copied(message, Instant::now()));
    ctx.request_repaint();
}
pub(crate) fn render(ctx: &egui::Context) {
    let Some((message, remaining)) = mutate(ctx, |state| state.message(Instant::now())) else {
        return;
    };
    ctx.request_repaint_after(remaining);
    egui::Area::new(egui::Id::new("copy-feedback-toast"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -50.0))
        .interactable(false)
        .sense(egui::Sense::empty())
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(egui::RichText::new(message).strong());
            });
        });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latest_confirmation_replaces_and_expires_after_two_seconds() {
        let now = Instant::now();
        let mut state = State::default();
        assert!(state.enabled);
        state.copied("folder", now);
        state.copied("file", now + Duration::from_secs(1));
        assert_eq!(
            state.message(now + Duration::from_secs(2)).unwrap().0,
            "file"
        );
        assert!(state.message(now + Duration::from_secs(3)).is_none());
        assert!(state.latest.is_none());
    }
    #[test]
    fn disabling_clears_and_suppresses_pending_confirmation() {
        let ctx = egui::Context::default();
        copied(&ctx, "first");
        configure(&ctx, false);
        copied(&ctx, "second");
        assert!(mutate(&ctx, |state| state.latest.is_none()));
        configure(&ctx, true);
        copied(&ctx, "third");
        assert_eq!(
            mutate(&ctx, |state| state.latest.as_ref().unwrap().0.clone()),
            "third"
        );
    }
}
