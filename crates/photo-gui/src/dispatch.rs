use super::*;
impl Workshop {
    pub(super) fn execute(&mut self, command: Command) -> Response {
        if self.adjustments_panel.is_some()
            && matches!(
                &command,
                Command::Open { .. }
                    | Command::PreviewFolder { .. }
                    | Command::Select { .. }
                    | Command::SelectPath { .. }
                    | Command::OpenSelected
                    | Command::Next
                    | Command::Previous
                    | Command::Back
                    | Command::OpenDropped { .. }
            )
        {
            return Response::error(
                ErrorCode::InvalidState,
                "Примените или отмените коррекцию перед переходом",
                self.gallery.as_ref(),
            );
        }
        self.navigation_dispatch_started();
        self.cancel_navigator_continuation(&command);
        if matches!(command, Command::NavigatorSnapshot)
            || (self.active_pane == ActivePane::Yazi && navigator::needs_snapshot(&command))
        {
            return match self.queue_navigator_command(command) {
                Ok(()) => self.execute_ready(Command::Status),
                Err(error) => Response::error(ErrorCode::Busy, error, None),
            };
        }
        self.execute_ready(command)
    }
    pub(super) fn execute_ready(&mut self, command: Command) -> Response {
        let settings_details = matches!(
            &command,
            Command::ShowSettings
                | Command::SelectSettings { .. }
                | Command::ConfigureYazi {
                    request: photo_core::settings::YaziRequest::Inspect
                }
        );
        if matches!(
            &command,
            Command::Open { .. }
                | Command::PreviewFolder { .. }
                | Command::Select { .. }
                | Command::SelectPath { .. }
                | Command::ExtendSelection { .. }
                | Command::ToggleSelection { .. }
                | Command::SetMarked { .. }
                | Command::OpenSelected
                | Command::Next
                | Command::Previous
                | Command::Back
                | Command::FocusPane { .. }
                | Command::SetNavigatorVisible { .. }
        ) {
            self.interaction_revision = self.interaction_revision.wrapping_add(1);
        }
        let include_order = matches!(command, Command::ListOrder);
        let include_commands = matches!(
            command,
            Command::ListCommands | Command::ShowCommands | Command::SearchCommands { .. }
        );
        let error_code = match &command {
            Command::Open { .. } | Command::PreviewFolder { .. } => ErrorCode::InvalidPath,
            Command::Select { .. }
            | Command::SelectPath { .. }
            | Command::ExtendSelection { .. }
            | Command::ToggleSelection { .. }
            | Command::SetMarked { .. }
            | Command::SetOrder { .. }
            | Command::OpenSelected => ErrorCode::InvalidSelection,
            Command::Refresh => ErrorCode::Internal,
            _ => ErrorCode::InvalidState,
        };
        let mut navigator_location = None;
        let mut metadata_job = None;
        let result: Result<(), String> = match command {
            Command::ShowMetadata => self.open_metadata(),
            Command::CloseMetadata => self.close_metadata(),
            Command::CancelMetadata => {
                self.cancel_metadata();
                Ok(())
            }
            Command::SubmitMetadata { metadata } => self.submit_panel_metadata(metadata),
            Command::DiscardMetadata => self.discard_panel_metadata(),
            Command::ShowHistory => self.history_command(photo_core::history::Operation::Inspect),
            Command::CloseHistory => {
                self.history_panel.open = false;
                self.history_panel.reset_thumbnails();
                Ok(())
            }
            Command::PreviewHistory { index } => {
                self.history_command(photo_core::history::Operation::Preview(index))
            }
            Command::UndoHistory => self.history_command(photo_core::history::Operation::Undo),
            Command::RedoHistory => self.history_command(photo_core::history::Operation::Redo),
            Command::CommitHistory => self.history_command(photo_core::history::Operation::Commit),
            Command::ShowAdjustments => self.begin_adjustments(),
            Command::ApplyAdjustments => self.apply_adjustments(),
            Command::PreviewAdjustments { options } => self.preview_adjustments(options),
            Command::SubmitAdjustments { options } => self.submit_adjustments(options),
            Command::CancelAdjustments => {
                self.cancel_adjustments();
                Ok(())
            }
            Command::Metadata { request } => self.metadata.submit(request, &self.ctx).map(|job| {
                metadata_job = Some(job);
            }),
            Command::MetadataStatus { request_id } => self.metadata.status(request_id).map(|job| {
                metadata_job = Some(job);
            }),
            Command::SelectSettings { section, page } => {
                self.select_settings(&section, page.as_deref())
            }
            Command::SetShortcut {
                action,
                context,
                binding,
                reset,
                replace,
            } => self.set_shortcut(action, context, binding, reset, replace),
            Command::SetGalleryScale { scale } => self.set_gallery_scale(scale),
            Command::SetGalleryAutoSize { enabled } => self.set_gallery_auto_size(enabled),
            Command::SetCopyNotifications { enabled } => self.set_copy_notifications(enabled),
            Command::ConfigureYazi { request } => self.configure_yazi(request),
            Command::ShowSettings => self.open_settings(),
            Command::CloseSettings => {
                self.close_settings();
                Ok(())
            }
            Command::SetYaziTextSize { size } => self.set_yazi_text_size(size),
            Command::EnterAlbum => self.enter_album(),
            Command::NavigatorSnapshot => self.navigator_snapshot().map(|snapshot| {
                navigator_location = Some(snapshot);
            }),
            Command::CaptureFrame { path, viewport } => {
                if matches!(viewport, photo_core::api::AppViewport::Settings) {
                    Err("Settings frame capture is unavailable in the current immediate-viewport renderer".into())
                } else if path.is_absolute() {
                    let viewport = match viewport {
                        photo_core::api::AppViewport::Root => egui::ViewportId::ROOT,
                        photo_core::api::AppViewport::Settings => settings::viewport_id(),
                    };
                    self.ctx.send_viewport_cmd_to(
                        viewport,
                        egui::ViewportCommand::Screenshot(egui::UserData::new(path)),
                    );
                    Ok(())
                } else {
                    Err("Screenshot path must be absolute".into())
                }
            }
            Command::ResizeViewport {
                width,
                height,
                viewport,
            } => {
                if (680..=4096).contains(&width) && (460..=4096).contains(&height) {
                    let viewport = match viewport {
                        photo_core::api::AppViewport::Root => egui::ViewportId::ROOT,
                        photo_core::api::AppViewport::Settings => settings::viewport_id(),
                    };
                    self.ctx.send_viewport_cmd_to(
                        viewport,
                        egui::ViewportCommand::InnerSize(Vec2::new(width as f32, height as f32)),
                    );
                    Ok(())
                } else {
                    Err("Viewport must be within680–4096 ×460–4096".into())
                }
            }
            Command::Status | Command::ListOrder | Command::ListCommands => Ok(()),
            Command::SubmitImage { options } => self.submit_image(options),
            Command::CancelImage => {
                self.image_dialog = None;
                Ok(())
            }
            Command::LoadAlbum { folder } => self.load_album(folder),
            Command::AlbumMove {
                horizontal,
                vertical,
            } => self.album_move(horizontal, vertical),
            Command::LoadNested { deeper } => self.load_nested(deeper),
            Command::ShowSearch => self.open_search(),
            Command::CloseSearch => {
                self.close_search();
                Ok(())
            }
            Command::SearchFiles { query } => self.search_files(query),
            Command::OpenSearchResult { index } => self.open_search_result(index),
            Command::ShowCommands => self.open_palette(),
            Command::CloseCommands => {
                self.close_palette();
                Ok(())
            }
            Command::SearchCommands { query } => {
                self.palette.query = query;
                self.palette.selected = 0;
                Ok(())
            }
            Command::RunAction { id } => self.run_action(id),
            Command::ConfirmDelete => self.confirm_delete(),
            Command::CancelDelete => {
                self.delete_dialog = None;
                Ok(())
            }
            Command::BeginRename => self.begin_rename(),
            Command::CancelRename => {
                self.rename_dialog = None;
                Ok(())
            }
            Command::Rename { new_name } => self.commit_rename(new_name),
            Command::ImportFiles { paths } => self.start_import(paths),
            Command::OpenDropped { paths } => self.open_dropped(paths),
            Command::VideoPause { paused } => {
                self.video_command(&["set", "pause", if paused { "yes" } else { "no" }])
            }
            Command::VideoSeek { seconds } => {
                if seconds.is_finite() && seconds >= 0.0 {
                    self.video_command(&["seek", &seconds.to_string(), "absolute+exact"])
                } else {
                    Err("Video position must be finite and nonnegative".into())
                }
            }
            Command::VideoVolume { volume } => {
                if volume.is_finite() && (0.0..=100.0).contains(&volume) {
                    self.video_command(&["set", "volume", &volume.to_string()])
                } else {
                    Err("Volume must be between 0 and 100".into())
                }
            }
            Command::Refresh => self.refresh_now(),
            Command::SetNavigatorVisible { visible } => {
                if visible {
                    self.focus_navigator()
                } else {
                    self.focus_gallery().map(|()| {
                        self.show_yazi = false;
                    })
                }
            }
            Command::FocusPane {
                pane: Pane::Gallery,
            } => self.focus_gallery(),
            Command::FocusPane {
                pane: Pane::Navigator,
            } => self.focus_navigator(),
            Command::Open { path } => {
                if self.open_path(&path) {
                    if let Some(yazi) = &mut self.yazi {
                        let _ = yazi.navigate(&path);
                    } else {
                        self.restart_yazi();
                    }
                    self.active_pane = ActivePane::Gallery;
                    self.ctx.memory_mut(|memory| {
                        if let Some(id) = memory.focused() {
                            memory.surrender_focus(id);
                        }
                    });
                    Ok(())
                } else {
                    Err(self.message.clone())
                }
            }
            Command::PreviewFolder { path } => self
                .open_path(&path)
                .then_some(())
                .ok_or_else(|| self.message.clone()),
            Command::Select { index } => {
                if self
                    .gallery
                    .as_mut()
                    .is_some_and(|gallery| gallery.set_grid_anchor(index))
                {
                    self.scroll_to_grid_anchor = true;
                    self.selection_dirty = true;
                    Ok(())
                } else {
                    Err("No such gallery index".into())
                }
            }
            Command::SelectPath { path } => {
                let result = self.gallery.as_mut().is_some_and(|gallery| {
                    navigator_paths_in_gallery(gallery.folder(), [path])
                        .pop()
                        .is_some_and(|path| gallery.select_path(&path))
                });
                if result {
                    self.scroll_to_grid_anchor = true;
                    self.selection_dirty = true;
                    Ok(())
                } else {
                    Err("Photo is not in this gallery".into())
                }
            }
            Command::ExtendSelection { index } => {
                let result = self
                    .gallery
                    .as_mut()
                    .is_some_and(|gallery| gallery.extend_selection(index));
                if result {
                    self.scroll_to_grid_anchor = true;
                    self.selection_dirty = true;
                    self.queue_mark_sync();
                    Ok(())
                } else {
                    Err("No such gallery index".into())
                }
            }
            Command::ToggleSelection { index } => {
                if self
                    .gallery
                    .as_mut()
                    .is_some_and(|gallery| gallery.toggle_selection(index))
                {
                    self.scroll_to_grid_anchor = true;
                    self.selection_dirty = true;
                    self.queue_mark_sync();
                    Ok(())
                } else {
                    Err("Selection toggle requires a gallery item".into())
                }
            }
            Command::SetMarked { paths } => {
                if let Some(gallery) = &mut self.gallery {
                    let paths = navigator_paths_in_gallery(gallery.folder(), paths);
                    gallery.set_marked_paths(paths);
                    self.selection_dirty = true;
                    self.queue_mark_sync();
                    Ok(())
                } else {
                    Err("No open gallery".into())
                }
            }
            Command::SetOrder { paths } => {
                if let Some(gallery) = &mut self.gallery {
                    let old_paths: Vec<_> = gallery
                        .photos()
                        .iter()
                        .map(|photo| photo.path.clone())
                        .collect();
                    let paths = navigator_paths_in_gallery(gallery.folder(), paths);
                    if gallery.set_order(&paths) {
                        self.remap_cached_images(&old_paths);
                    }
                    Ok(())
                } else {
                    Err("No open gallery".into())
                }
            }
            Command::OpenSelected => self
                .gallery
                .as_mut()
                .map(|gallery| gallery.open_photo(gallery.grid_anchor()))
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "No selected photo".into()),
            Command::Next => self
                .gallery
                .as_mut()
                .map(Gallery::next_photo)
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "No next photo".into()),
            Command::Previous => self
                .gallery
                .as_mut()
                .map(Gallery::previous_photo)
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "No previous photo".into()),
            Command::Back => self
                .gallery
                .as_mut()
                .map(|gallery| {
                    gallery.back_to_grid();
                    self.scroll_to_grid_anchor = true;
                })
                .ok_or_else(|| "No open gallery".into()),
            Command::ZoomAt {
                multiplier,
                anchor,
                viewport,
                fitted,
            } => self
                .gallery
                .as_mut()
                .map(|gallery| gallery.zoom_at(multiplier, anchor, viewport, fitted))
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "Cannot zoom at this point".into()),
            Command::PanBy {
                delta,
                viewport,
                fitted,
            } => self
                .gallery
                .as_mut()
                .map(|gallery| gallery.pan_by(delta, viewport, fitted))
                .filter(|success| *success)
                .map(|_| ())
                .ok_or_else(|| "Cannot pan this image".into()),
            Command::ResetZoom => self
                .gallery
                .as_mut()
                .map(Gallery::reset_zoom)
                .ok_or_else(|| "No open gallery".into()),
        };
        let mut response = match result {
            Ok(()) => Response::ok(self.gallery.as_ref()),
            Err(error) => Response::error(error_code, error, self.gallery.as_ref()),
        };
        self.sync_video();
        response.metadata = metadata_job;
        response.metadata_panel = Some(self.metadata_panel.panel_state());
        response.history = Some(self.history_panel.snapshot());
        response.adjustments_open = self.adjustments_panel.is_some();
        response.adjustments_preview = self.adjustments_preview_state();
        response.image_dialog = self.image_dialog.as_ref().map(|d| d.snapshot.clone());
        let mut nested = self.nested.snapshot.clone();
        nested.overview = self.album_mode()
            && self
                .gallery
                .as_ref()
                .is_some_and(|g| g.view() == View::Grid);
        nested.active_album = self.nested.cursor.clone();
        nested.album_positions = self
            .nested
            .positions
            .iter()
            .map(|(p, i)| (p.clone(), *i))
            .collect();
        response.nested = Some(nested);
        response.search = Some(self.search.snapshot.clone());
        response.rename = self.rename_snapshot();
        response.delete_confirmation = self.delete_snapshot();
        response.render = Some(self.render_snapshot());
        response.native_paste_shortcut = self.native_keys.is_some();
        response.operation = self.operation.clone();
        response.palette = Some(self.palette_snapshot());
        response.settings = Some(self.settings.snapshot());
        if let Some(settings) = &mut response.settings {
            settings.page = Some(
                if settings.section == "yazi" {
                    self.yazi_settings.page
                } else {
                    "Shortcuts"
                }
                .into(),
            );
            let mut yazi = self.yazi_settings.view.clone();
            if !settings_details {
                yazi.bindings.clear();
                yazi.inherited_bindings.clear();
                yazi.flavors.clear();
                settings.shortcuts.clear();
            }
            settings.yazi = Some(yazi);
        }
        if include_commands {
            response.commands = Some(self.filtered_actions());
        }
        response.video = self.video.as_ref().map(|v| v.lock().unwrap().state.clone());
        response.session_id = Some(self.session_id.clone());
        if include_order
            && let Some(snapshot) = &mut response.snapshot
            && let Some(gallery) = &self.gallery
        {
            snapshot.ordered_paths = Some(
                gallery
                    .photos()
                    .iter()
                    .map(|photo| photo.path.clone())
                    .collect(),
            );
        }
        response.navigator = Some(NavigatorState {
            runtime: navigator_location.as_ref().map(|snapshot| {
                photo_core::api::NavigatorRuntimeState {
                    marked_paths: snapshot.marked.clone(),
                    ordered_paths: snapshot.ordered.clone(),
                    preferences: snapshot.preferences.clone(),
                    metadata: snapshot.metadata.clone(),
                    tabs: snapshot.tabs,
                }
            }),
            pending: self.navigator_pending_request().is_some()
                || (self.selection_dirty && self.mark_sync_after.is_some()),
            request_id: self.navigator_pending_request(),
            folder: navigator_location.as_ref().map(|s| s.folder.clone()),
            hovered: navigator_location.and_then(|s| s.hovered),
            visible: self.navigator_rendered(),
            focus: if self.active_pane == ActivePane::Gallery {
                Pane::Gallery
            } else {
                Pane::Navigator
            },
        });
        response
    }
    pub(super) fn poll_api(&mut self) {
        let started = Instant::now();
        for _ in 0..4 {
            let request = self
                .api_requests
                .as_ref()
                .and_then(|receiver| receiver.try_recv().ok());
            let Some(request) = request else { break };
            if Instant::now() >= request.deadline {
                let _ = request.reply.send(Response::error(
                    ErrorCode::Busy,
                    "API reply timed out",
                    None,
                ));
                continue;
            }
            let response = self.execute(request.command);
            if !self.defer_navigator_reply(request.reply.clone(), request.deadline) {
                let _ = request.reply.send(response);
            }
            if started.elapsed() >= Duration::from_millis(4) {
                break;
            }
        }
    }
}
