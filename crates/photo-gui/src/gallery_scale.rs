use super::*;
impl Workshop {
    pub(super) fn set_gallery_scale(&mut self, scale: Option<f32>) -> Result<(), String> {
        if scale.is_some_and(|v| !v.is_finite() || !(0.4..=2.5).contains(&v)) {
            return Err("Gallery scale must be between 0.4 and 2.5".into());
        }
        settings::update_config(&settings::config_path()?, |document| {
            document["gallery_scale"] = serde_json::json!(scale);
            document["gallery_auto_size"] = serde_json::json!(scale.is_none());
            Ok(())
        })?;
        self.settings.gallery_scale = scale;
        self.settings.gallery_auto_size = scale.is_none();
        self.scroll_to_grid_anchor = true;
        Ok(())
    }
    pub(super) fn set_gallery_auto_size(&mut self, enabled: bool) -> Result<(), String> {
        settings::save_property("gallery_auto_size", serde_json::json!(enabled))?;
        self.settings.gallery_auto_size = enabled;
        self.scroll_to_grid_anchor = true;
        Ok(())
    }
    pub(super) fn gallery_scale_wheel(&mut self, ui: &egui::Ui) {
        if !ui.rect_contains_pointer(ui.available_rect_before_wrap()) {
            return;
        }
        let delta = ui.input_mut(|input| {
            let delta: f32 = input
                .events
                .iter()
                .filter_map(|event| {
                    if let egui::Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                        ..
                    } = event
                    {
                        modifiers.ctrl.then_some(
                            delta.y
                                * match unit {
                                    egui::MouseWheelUnit::Line => 40.0,
                                    egui::MouseWheelUnit::Page => 500.0,
                                    egui::MouseWheelUnit::Point => 1.0,
                                },
                        )
                    } else {
                        None
                    }
                })
                .sum();
            if delta == 0.0 {
                return 0.0;
            }
            input.smooth_scroll_delta = Vec2::ZERO;
            input.events.retain(
                |e| !matches!(e, egui::Event::MouseWheel { modifiers, .. } if modifiers.ctrl),
            );
            delta
        });
        if delta != 0.0 {
            let scale = (self.settings.effective_gallery_scale().unwrap_or(1.0)
                * (delta * 0.003).exp())
            .clamp(0.4, 2.5);
            if let Some(error) = self
                .execute(Command::SetGalleryScale { scale: Some(scale) })
                .error
            {
                self.message = error.message;
            }
        }
    }
}
