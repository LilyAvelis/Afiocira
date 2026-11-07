use super::*;
impl Workshop {
    pub(super) fn execute(&mut self, command: Command) -> Response {
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
        let result: Result<(), String> = match command {
            Command::EnterAlbum => self.enter_album(),
            Command::NavigatorSnapshot => self.navigator_snapshot().map(|snapshot| {
                navigator_location = Some(snapshot);
            }),
            Command::CaptureFrame { path } => {
                if path.is_absolute() {
                    self.ctx
                        .send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                            path,
                        )));
                    Ok(())
                } else {
                    Err("Screenshot path must be absolute".into())
                }
            }
            Command::ResizeViewport { width, height } => {
                if (680..=4096).contains(&width) && (460..=4096).contains(&height) {
                    self.ctx
                        .send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                            width as f32,
                            height as f32,
                        )));
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
            folder: navigator_location.as_ref().map(|s| s.folder.clone()),
            hovered: navigator_location.and_then(|s| s.hovered),
            visible: self.show_yazi,
            focus: if self.active_pane == ActivePane::Gallery {
                Pane::Gallery
            } else {
                Pane::Navigator
            },
        });
        response
    }
    pub(super) fn poll_api(&mut self) {
        let requests: Vec<_> = self
            .api_requests
            .as_ref()
            .map(|receiver| receiver.try_iter().collect())
            .unwrap_or_default();
        for request in requests {
            let response = self.execute(request.command);
            let _ = request.reply.send(response);
        }
    }
}
