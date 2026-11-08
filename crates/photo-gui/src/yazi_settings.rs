//! Transactional Yazi configuration: prepare a replacement, verify, then publish.
use super::*;
use photo_core::settings::{
    Quicklink, YaziBinding, YaziProfile, YaziRequest, YaziSettingsSnapshot, YaziThemeMode,
};
use yazi_profile as profile;

pub(super) struct State {
    pub view: YaziSettingsSnapshot,
    applied: YaziProfile,
    prepared_save: Option<(YaziProfile, YaziProfile)>,
    phase: Option<Phase>,
    install: Option<Receiver<Result<String, String>>>,
    pub page: &'static str,
    pub filter: String,
    pub link_edit: Option<Quicklink>,
    pub binding_edit: Option<YaziBinding>,
    pub replace: bool,
    pub install_source: String,
    pub binding_keys: String,
    pub binding_run: String,
    pub link_path: String,
    pub picker: Option<Receiver<Result<Option<PathBuf>, String>>>,
}
enum Phase {
    HotTheme {
        backup: Vec<u8>,
        expected: serde_json::Value,
        saved: yazi::NavigatorSnapshot,
        deadline: Instant,
        next_request: Instant,
        revision: u64,
    },

    Snapshot {
        deadline: Instant,
        revision: u64,
        waiting_tasks: bool,
    },
    Preparing {
        receiver: Receiver<Result<yazi::PreparedYazi, String>>,
        saved: yazi::NavigatorSnapshot,
        revision: u64,
    },
    Candidate {
        pane: Box<YaziPane>,
        saved: yazi::NavigatorSnapshot,
        revision: u64,
        deadline: Instant,
        next_request: Instant,
        restored: bool,
    },
}
impl Default for State {
    fn default() -> Self {
        let loaded = profile::load();
        let applied = loaded.as_ref().cloned().unwrap_or_default();
        Self {
            view: YaziSettingsSnapshot {
                profile: applied.clone(),
                error: loaded.err(),
                ..Default::default()
            },
            applied,
            prepared_save: None,
            phase: None,
            install: None,
            page: "Appearance",
            filter: String::new(),
            link_edit: None,
            binding_edit: None,
            replace: false,
            install_source: String::new(),
            binding_keys: String::new(),
            binding_run: String::new(),
            link_path: String::new(),
            picker: None,
        }
    }
}
impl State {
    pub fn busy(&self) -> bool {
        self.phase.is_some() || self.install.is_some()
    }
}

impl Workshop {
    pub(super) fn inspect_yazi_settings(&mut self) -> Result<(), String> {
        let pane = self.yazi.as_ref().ok_or("Yazi unavailable")?;
        let config = pane.base_config_directory();
        let (values, sources) =
            profile::effective_general(&config, &self.yazi_settings.view.profile)?;
        let bindings = profile::effective_bindings(&config)?;
        let installed = profile::flavors(&pane.config_directory());
        let busy = self.yazi_settings.busy();
        let view = &mut self.yazi_settings.view;
        if view.version.is_empty() {
            view.version = pane.version()?;
        }
        view.config_directory = Some(pane.config_directory());
        view.effective_general = values;
        view.general_sources = sources;
        view.inherited_bindings = bindings.clone();
        let mut own = view
            .profile
            .bindings
            .iter()
            .filter(|b| b.enabled)
            .cloned()
            .map(|b| (b, "Afio:"))
            .collect::<Vec<_>>();
        for link in view
            .profile
            .quicklinks
            .iter()
            .filter(|l| l.enabled && !l.deleted)
        {
            if let Ok(binding) = profile::quicklink_binding(link) {
                own.push((binding, "Quicklink:"));
            }
        }
        let mut effective = Vec::new();
        for (binding, prefix) in own {
            effective.push(photo_core::settings::EffectiveYaziBinding {
                layer: binding.layer,
                on: binding.on,
                run: binding.run,
                description: binding.description,
                source: format!("{prefix}{}", binding.id),
            });
        }
        let mut seen = HashSet::new();
        effective.extend(bindings);
        effective.retain(|b| {
            seen.insert((
                b.layer.clone(),
                b.on.iter().take(2).cloned().collect::<Vec<_>>(),
            ))
        });
        view.bindings = effective;
        view.flavors = installed;
        // Downloaded flavors are kept outside ephemeral Yazi sessions.
        for flavor in profile::flavors(&profile::root()?) {
            if !view.flavors.iter().any(|f| f.name == flavor.name) {
                view.flavors.push(flavor);
            }
        }
        view.dirty = view.profile != self.yazi_settings.applied;
        view.busy = busy;
        Ok(())
    }
    fn binding_conflict(&self, binding: &YaziBinding, replace: bool) -> Result<(), String> {
        if !binding.enabled {
            return Ok(());
        }
        let mut own = self
            .yazi_settings
            .view
            .profile
            .bindings
            .iter()
            .filter(|b| b.enabled && b.id != binding.id)
            .cloned()
            .collect::<Vec<_>>();
        for link in self
            .yazi_settings
            .view
            .profile
            .quicklinks
            .iter()
            .filter(|l| l.enabled && !l.deleted && l.id != binding.id)
        {
            own.push(profile::quicklink_binding(link)?);
        }
        let overlaps = |on: &[String]| {
            on == binding.on || on.starts_with(&binding.on) || binding.on.starts_with(on)
        };
        if let Some(other) = own
            .iter()
            .find(|b| b.layer == binding.layer && overlaps(&b.on))
        {
            return Err(format!(
                "Conflict with Afio override: {} ({})",
                other.description,
                other.on.join(" → ")
            ));
        }
        let already_owned = self
            .yazi_settings
            .view
            .profile
            .bindings
            .iter()
            .any(|b| b.id == binding.id && b.layer == binding.layer && b.on == binding.on)
            || self.yazi_settings.view.profile.quicklinks.iter().any(|l| {
                l.id == binding.id
                    && binding.layer == "mgr"
                    && binding.on == ["g".to_owned(), l.key.clone()]
            });
        if !replace
            && !already_owned
            && let Some(other) = self
                .yazi_settings
                .view
                .inherited_bindings
                .iter()
                .find(|b| b.layer == binding.layer && overlaps(&b.on))
        {
            return Err(format!(
                "Already assigned: {} → {}. Enable explicit replacement.",
                other.on.join(" → "),
                other.description
            ));
        }
        Ok(())
    }
    pub(super) fn configure_yazi(&mut self, request: YaziRequest) -> Result<(), String> {
        let result = self.configure_yazi_inner(request);
        self.yazi_settings.view.error = result.as_ref().err().cloned();
        result
    }
    fn configure_yazi_inner(&mut self, request: YaziRequest) -> Result<(), String> {
        if self.yazi_settings.busy() {
            return Err("Yazi settings operation is running".into());
        }
        self.inspect_yazi_settings()?;
        match request {
            YaziRequest::ApplyThemeMode { mode } => {
                let draft = &self.yazi_settings.view.profile;
                let applied = &self.yazi_settings.applied;
                if draft.general != applied.general
                    || draft.quicklinks != applied.quicklinks
                    || draft.bindings != applied.bindings
                {
                    return Err("Сначала примените или отмените изменения Yazi. Затем переключите светлый/тёмный режим.".into());
                }
                self.yazi_settings.view.profile.theme_mode = mode;
                return self.configure_yazi_inner(YaziRequest::Apply);
            }
            YaziRequest::ApplyAppearance { dark, light, icons } => {
                let draft = &self.yazi_settings.view.profile;
                let applied = &self.yazi_settings.applied;
                if draft.general != applied.general
                    || draft.quicklinks != applied.quicklinks
                    || draft.bindings != applied.bindings
                {
                    return Err("Сначала примените или отмените изменения в других разделах Yazi. Затем примените тему на её карточке.".into());
                }
                self.configure_yazi_inner(YaziRequest::SetAppearance { dark, light, icons })?;
                return self.configure_yazi_inner(YaziRequest::Apply);
            }
            YaziRequest::TestQuicklink { id } => {
                let link = self
                    .yazi_settings
                    .applied
                    .quicklinks
                    .iter()
                    .find(|l| l.id == id && l.enabled && !l.deleted)
                    .ok_or("Apply an enabled Quicklink before testing it")?;
                self.yazi
                    .as_mut()
                    .ok_or("Yazi unavailable")?
                    .quicklink_keys(&link.key)?;
                self.active_pane = ActivePane::Yazi;
                self.interaction_revision = self.interaction_revision.wrapping_add(1);
                return Ok(());
            }
            YaziRequest::PickFolder => {
                if self.yazi_settings.picker.is_some() {
                    return Err("Folder picker is already open".into());
                }
                let (sender, receiver) = mpsc::channel();
                let ctx = self.ctx.clone();
                std::thread::spawn(move || {
                    let _ = sender.send(photo_core::settings::choose_folder());
                    ctx.request_repaint();
                });
                self.yazi_settings.picker = Some(receiver);
                return Ok(());
            }
            YaziRequest::Inspect => return Ok(()),
            YaziRequest::SetGeneral { key, value } => {
                if let Some(value) = value {
                    profile::set_general(&mut toml_edit::DocumentMut::new(), &key, &value)?;
                    self.yazi_settings.view.profile.general.insert(key, value);
                } else {
                    self.yazi_settings.view.profile.general.remove(&key);
                }
            }
            YaziRequest::SaveQuicklink { link, replace } => {
                if self
                    .yazi_settings
                    .view
                    .profile
                    .bindings
                    .iter()
                    .any(|b| b.id == link.id)
                {
                    return Err("This ID belongs to a keybinding".into());
                }
                if !link.deleted {
                    let binding = profile::quicklink_binding(&link)?;
                    profile::validate_binding(&binding)?;
                    self.binding_conflict(&binding, replace)?;
                }
                let links = &mut self.yazi_settings.view.profile.quicklinks;
                if let Some(old) = links.iter_mut().find(|l| l.id == link.id) {
                    *old = link;
                } else {
                    links.push(link);
                }
            }
            YaziRequest::DeleteQuicklink { id } => {
                self.yazi_settings
                    .view
                    .profile
                    .quicklinks
                    .iter_mut()
                    .find(|l| l.id == id)
                    .ok_or("Unknown Quicklink")?
                    .deleted = true;
            }
            YaziRequest::RestoreQuicklink { id } => {
                let mut link = self
                    .yazi_settings
                    .view
                    .profile
                    .quicklinks
                    .iter()
                    .find(|l| l.id == id)
                    .ok_or("Unknown Quicklink")?
                    .clone();
                link.deleted = false;
                self.binding_conflict(&profile::quicklink_binding(&link)?, true)?;
                *self
                    .yazi_settings
                    .view
                    .profile
                    .quicklinks
                    .iter_mut()
                    .find(|l| l.id == id)
                    .unwrap() = link;
            }
            YaziRequest::SaveBinding { binding, replace } => {
                if self
                    .yazi_settings
                    .view
                    .profile
                    .quicklinks
                    .iter()
                    .any(|l| l.id == binding.id)
                {
                    return Err("This ID belongs to a Quicklink".into());
                }
                profile::validate_binding(&binding)?;
                self.binding_conflict(&binding, replace)?;
                let bindings = &mut self.yazi_settings.view.profile.bindings;
                if let Some(old) = bindings.iter_mut().find(|b| b.id == binding.id) {
                    *old = binding;
                } else {
                    bindings.push(binding);
                }
            }
            YaziRequest::DeleteBinding { id } => self
                .yazi_settings
                .view
                .profile
                .bindings
                .retain(|b| b.id != id),
            YaziRequest::SetAppearance { dark, light, icons } => {
                for name in [&dark, &light]
                    .into_iter()
                    .flatten()
                    .filter(|s| !s.is_empty())
                {
                    if !self
                        .yazi_settings
                        .view
                        .flavors
                        .iter()
                        .any(|f| &f.name == name && f.compatible)
                    {
                        return Err("Choose an installed compatible flavor".into());
                    }
                }
                if icons
                    .as_deref()
                    .is_some_and(|v| !["inherit", "minimal"].contains(&v))
                {
                    return Err("Unknown icon set".into());
                }
                let p = &mut self.yazi_settings.view.profile;
                p.dark_flavor = dark;
                p.light_flavor = light;
                p.icons = icons;
            }
            YaziRequest::InstallFlavor { source } => {
                let (sender, receiver) = mpsc::channel();
                let ctx = self.ctx.clone();
                std::thread::spawn(move || {
                    let result = crate::yazi_flavors::install(&source);
                    let _ = sender.send(result);
                    ctx.request_repaint();
                });
                self.yazi_settings.install = Some(receiver);
                self.yazi_settings.view.busy = true;
                self.yazi_settings.view.message = "Скачиваем и проверяем flavor…".into();
                return Ok(());
            }
            YaziRequest::Discard => {
                self.yazi_settings.view.profile = self.yazi_settings.applied.clone();
            }
            YaziRequest::Apply => {
                if self.navigation.busy() {
                    return Err(
                        "Finish the pending navigator handoff before applying settings".into(),
                    );
                }
                if !self
                    .yazi_settings
                    .view
                    .version
                    .starts_with(&format!("Yazi {} ", profile::VERSION))
                {
                    return Err("This settings adapter requires Yazi 26.5.6".into());
                }
                let (latest, merged) = profile::prepare_save(
                    &self.yazi_settings.applied,
                    &self.yazi_settings.view.profile,
                )?;
                self.yazi_settings.view.profile = merged.clone();
                self.yazi_settings.prepared_save = Some((latest, merged));
                let pane = self.yazi.as_mut().ok_or("Yazi unavailable")?;
                pane.begin_snapshot()?;
                self.yazi_settings.phase = Some(Phase::Snapshot {
                    deadline: Instant::now() + Duration::from_secs(5),
                    revision: self.interaction_revision,
                    waiting_tasks: false,
                });
                self.yazi_settings.view.busy = true;
                self.yazi_settings.view.message =
                    "Сохраняем контекст. Будет заменён только встроенный Yazi…".into();
                return Ok(());
            }
        }
        self.inspect_yazi_settings()?;
        self.yazi_settings.view.message =
            "Изменения готовы. «Применить к Yazi» сохранит их и восстановит контекст.".into();
        Ok(())
    }

    pub(super) fn poll_yazi_settings(&mut self) {
        if let Some(receiver) = &self.yazi_settings.picker {
            match receiver.try_recv() {
                Ok(result) => {
                    self.yazi_settings.picker = None;
                    match result {
                        Ok(Some(path)) => self.yazi_settings.link_path = friendly_path(&path),
                        Ok(None) => {}
                        Err(error) => self.yazi_settings.view.error = Some(error),
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.yazi_settings.picker = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(receiver) = &self.yazi_settings.install {
            match receiver.try_recv() {
                Ok(result) => {
                    self.yazi_settings.install = None;
                    match result {
                        Ok(name) => {
                            self.yazi_settings.view.message =
                                format!("Установлен {name}. Выберите его в Appearance.")
                        }
                        Err(e) => self.yazi_settings.view.error = Some(e),
                    }
                    self.yazi_settings.view.busy = false;
                    let _ = self.inspect_yazi_settings();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.yazi_settings.install = None;
                    self.yazi_settings.view.busy = false;
                    self.yazi_settings.view.error = Some("Flavor worker stopped".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let Some(phase) = self.yazi_settings.phase.take() else {
            return;
        };
        let result = self.advance_yazi_apply(phase);
        match result {
            Ok(next) => self.yazi_settings.phase = next,
            Err(error) => {
                if let Some(active) = &mut self.yazi {
                    active.cancel_snapshot();
                }
                self.yazi_settings.prepared_save = None;
                self.yazi_settings.view.error = Some(format!("{error}. Прежний Yazi сохранён."));
                self.yazi_settings.view.message = "Изменения не применены".into();
            }
        }
        self.yazi_settings.view.busy = self.yazi_settings.busy();
    }
    fn advance_yazi_apply(&mut self, phase: Phase) -> Result<Option<Phase>, String> {
        match phase {
            Phase::HotTheme {
                backup,
                expected,
                saved,
                deadline,
                mut next_request,
                revision,
            } => {
                let active = self.yazi.as_mut().ok_or("Yazi unavailable")?;
                if revision != self.interaction_revision || Instant::now() > deadline {
                    let _ = std::fs::write(active.config_directory().join("theme.toml"), backup);
                    let _ = active.emit("app:theme", &[]);
                    return Err(format!(
                        "Hot theme reload did not confirm; theme configuration rolled back: {}",
                        self.yazi_settings.view.message
                    ));
                }
                if let Some(snapshot) = active.poll_snapshot() {
                    if snapshot.metadata["theme"] == expected
                        && snapshot.folder == saved.folder
                        && snapshot.hovered == saved.hovered
                        && paths_equal(&snapshot.marked, &saved.marked)
                    {
                        if let Err(error) = self
                            .yazi_settings
                            .prepared_save
                            .as_ref()
                            .ok_or_else(|| "Missing prepared Yazi settings transaction".to_string())
                            .and_then(|(latest, merged)| profile::save_prepared(latest, merged))
                        {
                            let _ = std::fs::write(
                                active.config_directory().join("theme.toml"),
                                backup,
                            );
                            let _ = active.emit("app:theme", &[]);
                            return Err(error);
                        }
                        self.yazi_settings.applied = self.yazi_settings.view.profile.clone();
                        self.yazi_settings.prepared_save = None;
                        self.yazi_settings.view.dirty = false;
                        self.yazi_settings.view.message =
                            "Тема применена горячо. Yazi и Afio не перезапускались.".into();
                        self.yazi_settings.view.error = None;
                        let _ = self.inspect_yazi_settings();
                        return Ok(None);
                    }
                    self.yazi_settings.view.message =
                        "Ожидаем подтверждение новой темы от Yazi…".into();
                }

                if Instant::now() >= next_request {
                    let _ = active.request_snapshot();
                    next_request = Instant::now() + Duration::from_millis(200);
                }
                Ok(Some(Phase::HotTheme {
                    backup,
                    expected,
                    saved,
                    deadline,
                    next_request,
                    revision,
                }))
            }
            Phase::Snapshot {
                deadline,
                revision,
                waiting_tasks,
            } => {
                if let Some(error) = self.yazi.as_mut().and_then(YaziPane::take_navigation_error) {
                    return Err(error);
                }
                if Instant::now() > deadline {
                    return Err(if waiting_tasks {
                        "Yazi tasks did not finish before the configuration deadline"
                    } else {
                        "Yazi context timed out"
                    }
                    .into());
                }
                let Some(saved) = self.yazi.as_mut().and_then(YaziPane::poll_snapshot) else {
                    return Ok(Some(Phase::Snapshot {
                        deadline,
                        revision,
                        waiting_tasks,
                    }));
                };
                if saved.metadata["layer"]
                    .as_str()
                    .is_some_and(|s| !s.eq_ignore_ascii_case("mgr"))
                {
                    return Err("Finish Yazi dialogs before applying configuration".into());
                }
                if saved.metadata["tasks"].as_u64().unwrap_or(0) > 0 {
                    self.yazi
                        .as_mut()
                        .ok_or("Yazi unavailable")?
                        .request_snapshot()?;
                    self.yazi_settings.view.message = "Ожидаем завершения задач Yazi…".into();
                    self.ctx.request_repaint_after(Duration::from_millis(50));
                    return Ok(Some(Phase::Snapshot {
                        deadline,
                        revision,
                        waiting_tasks: true,
                    }));
                }
                let old = &self.yazi_settings.applied;
                let new = &self.yazi_settings.view.profile;
                let replaces_yazi = old.general != new.general
                    || old.quicklinks != new.quicklinks
                    || old.bindings != new.bindings
                    || old.theme_mode != new.theme_mode;
                if replaces_yazi && saved.metadata["yanked"].as_u64().unwrap_or(0) > 0 {
                    return Err("В Yazi есть отложенное копирование/перенос. Завершите его или снимите yank перед применением.".into());
                }
                if saved.tabs != 1 {
                    return Err(
                        "Apply settings with one Yazi tab; multiple tabs are left intact".into(),
                    );
                }
                if saved.preferences["sort_by"] == "random" {
                    return Err("Choose a deterministic Yazi sort before applying settings".into());
                }
                let draft = self.yazi_settings.view.profile.clone();
                let ctx = self.ctx.clone();
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let _ = sender.send(YaziPane::prepare(&draft));
                    ctx.request_repaint();
                });
                Ok(Some(Phase::Preparing {
                    receiver,
                    saved,
                    revision,
                }))
            }
            Phase::Preparing {
                receiver,
                saved,
                revision,
            } => match receiver.try_recv() {
                Ok(prepared) => {
                    let pane = YaziPane::spawn_prepared(&saved.folder, prepared?)?;
                    Ok(Some(Phase::Candidate {
                        pane: Box::new(pane),
                        saved,
                        revision,
                        deadline: Instant::now() + Duration::from_secs(12),
                        next_request: Instant::now(),
                        restored: false,
                    }))
                }
                Err(mpsc::TryRecvError::Empty) => Ok(Some(Phase::Preparing {
                    receiver,
                    saved,
                    revision,
                })),
                Err(mpsc::TryRecvError::Disconnected) => Err("Yazi preparation stopped".into()),
            },
            Phase::Candidate {
                mut pane,
                mut saved,
                revision,
                deadline,
                mut next_request,
                mut restored,
            } => {
                pane.poll_background();
                if let Some(error) = pane.take_navigation_error() {
                    return Err(format!("Replacement Yazi transport failed: {error}"));
                }
                if revision != self.interaction_revision {
                    return Err(
                        "Navigation changed during apply; retry with the new context".into(),
                    );
                }
                if Instant::now() > deadline || !pane.is_running() {
                    return Err(format!(
                        "Replacement Yazi could not verify its configuration/context: {}",
                        self.yazi_settings.view.message
                    ));
                }
                if let Some(snapshot) = pane.poll_snapshot() {
                    self.yazi_settings.view.message = if restored {
                        "Проверяем восстановленный контекст Yazi…"
                    } else {
                        "Новый Yazi запущен; восстанавливаем контекст…"
                    }
                    .into();
                    if !restored {
                        // Retain the runtime sort unless this particular setting changed.
                        let (effective, _) = profile::effective_general(
                            &pane.base_config_directory(),
                            &self.yazi_settings.view.profile,
                        )?;
                        project_runtime_preferences(
                            &mut saved.preferences,
                            &effective,
                            &self.yazi_settings.applied,
                            &self.yazi_settings.view.profile,
                        );
                        let p = &saved.preferences;
                        let mut args = vec![p["sort_by"].as_str().unwrap_or("alphabetical").into()];
                        for (name, key) in [
                            ("reverse", "sort_reverse"),
                            ("sensitive", "sort_sensitive"),
                            ("dir-first", "sort_dir_first"),
                            ("translit", "sort_translit"),
                        ] {
                            args.push(format!(
                                "--{name}={}",
                                if p[key].as_bool().unwrap_or(false) {
                                    "yes"
                                } else {
                                    "no"
                                }
                            ));
                        }
                        args.push(format!(
                            "--fallback={}",
                            p["sort_fallback"].as_str().unwrap_or("alphabetical")
                        ));
                        pane.emit("sort", &args)?;
                        pane.emit(
                            "hidden",
                            &[if p["show_hidden"].as_bool().unwrap_or(false) {
                                "show"
                            } else {
                                "hide"
                            }
                            .into()],
                        )?;
                        pane.emit(
                            "linemode",
                            &[p["linemode"].as_str().unwrap_or("none").into()],
                        )?;
                        if let Some(cursor) = &saved.hovered {
                            pane.restore_context_selection(
                                cursor,
                                &saved.marked,
                                &snapshot.marked,
                            )?;
                        }
                        restored = true;
                    } else if same_folder(&snapshot.folder, &saved.folder)
                        && snapshot.hovered.as_ref().map(|p| friendly_path(p))
                            == saved.hovered.as_ref().map(|p| friendly_path(p))
                        && paths_equal(&snapshot.marked, &saved.marked)
                        && snapshot.preferences == saved.preferences
                        && snapshot.metadata["light"].as_bool()
                            == Some(
                                self.yazi_settings.view.profile.theme_mode == YaziThemeMode::Light,
                            )
                    {
                        let old = &self.yazi_settings.applied;
                        let new = &self.yazi_settings.view.profile;
                        if old.general == new.general
                            && old.quicklinks == new.quicklinks
                            && old.bindings == new.bindings
                            && old.theme_mode == new.theme_mode
                        {
                            let active = self.yazi.as_ref().ok_or("Yazi unavailable")?;
                            let live = active.config_directory();
                            let backup = std::fs::read(live.join("theme.toml")).unwrap_or_default();
                            copy_active_flavors(&pane.config_directory(), &live)?;
                            let theme = std::fs::read(pane.config_directory().join("theme.toml"))
                                .map_err(|e| e.to_string())?;
                            if let Err(error) = std::fs::write(live.join("theme.toml"), theme)
                                .map_err(|e| e.to_string())
                                .and_then(|_| active.emit("app:theme", &[]))
                            {
                                let _ = std::fs::write(live.join("theme.toml"), backup);
                                return Err(error);
                            }
                            self.yazi_settings.view.message =
                                "Применяем тему горячо через app:theme…".into();
                            return Ok(Some(Phase::HotTheme {
                                backup,
                                expected: snapshot.metadata["theme"].clone(),
                                saved,
                                deadline: Instant::now() + Duration::from_secs(5),
                                next_request: Instant::now(),
                                revision,
                            }));
                        }
                        // Do not discard an old process until the new live process confirms state.
                        let (latest, merged) = self
                            .yazi_settings
                            .prepared_save
                            .as_ref()
                            .ok_or("Missing prepared Yazi settings transaction")?;
                        profile::save_prepared(latest, merged)?;
                        self.yazi_settings.applied = self.yazi_settings.view.profile.clone();
                        self.yazi_settings.prepared_save = None;
                        if self.active_pane == ActivePane::Yazi {
                            pane.request_focus();
                        }
                        // Initial startup/reveal reports are not user navigation.
                        let _ = pane.changed_folder();
                        let _ = pane.poll_order();
                        if let Some(active) = &self.yazi {
                            pane.inherit_selection_ownership(active);
                        }
                        self.yazi = Some(*pane);
                        self.yazi_settings.view.dirty = false;
                        self.yazi_settings.view.message =
                            "Применено. Контекст Yazi восстановлен; Afio продолжает ту же сессию."
                                .into();
                        self.yazi_settings.view.error = None;
                        let _ = self.inspect_yazi_settings();
                        return Ok(None);
                    }
                }
                if Instant::now() >= next_request {
                    let _ = pane.request_snapshot();
                    next_request = Instant::now() + Duration::from_millis(200);
                }
                Ok(Some(Phase::Candidate {
                    pane,
                    saved,
                    revision,
                    deadline,
                    next_request,
                    restored,
                }))
            }
        }
    }
}
fn copy_active_flavors(candidate: &Path, live: &Path) -> Result<(), String> {
    let theme = profile::read(&candidate.join("theme.toml"))?;
    let mut names = std::collections::BTreeSet::new();
    for slot in ["dark", "light"] {
        if let Some(name) = theme
            .get("flavor")
            .and_then(|f| f.get(slot))
            .and_then(toml_edit::Item::as_str)
            .filter(|name| !name.is_empty())
        {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err(format!("Неподдерживаемое имя оформления: {name}"));
            }
            names.insert(name);
        }
    }
    for name in names {
        let directory = format!("{name}.yazi");
        yazi::copy_directory(
            &candidate.join("flavors").join(&directory),
            &live.join("flavors").join(&directory),
        )
        .map_err(|e| format!("Подготовка оформления {name} для живого Yazi: {e}"))?;
    }
    Ok(())
}
fn paths_equal(a: &[PathBuf], b: &[PathBuf]) -> bool {
    let normalize = |paths: &[PathBuf]| {
        paths
            .iter()
            .map(|p| friendly_path(p).to_lowercase())
            .collect::<std::collections::BTreeSet<_>>()
    };
    normalize(a) == normalize(b)
}

fn project_runtime_preferences(
    saved: &mut serde_json::Value,
    effective: &std::collections::BTreeMap<String, String>,
    old: &YaziProfile,
    new: &YaziProfile,
) {
    for (key, value) in effective {
        // Startup-only General fields (scrolloff, ratios, symlink display, etc.)
        // cannot be compared to a runtime snapshot that never reports them.
        if saved.get(key).is_some() && old.general.get(key) != new.general.get(key) {
            saved[key] = serde_json::from_str(value)
                .unwrap_or_else(|_| serde_json::Value::String(value.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_general_changes_do_not_poison_runtime_preference_verification() {
        let old = YaziProfile::default();
        let mut new = old.clone();
        new.general.insert("scrolloff".into(), "5".into());
        new.general.insert("sort_reverse".into(), "true".into());
        let effective = [
            ("scrolloff", "5"),
            ("sort_by", "alphabetical"),
            ("sort_reverse", "true"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let mut saved = serde_json::json!({"sort_by":"natural", "sort_reverse":false, "show_hidden":true, "linemode":"size"});
        project_runtime_preferences(&mut saved, &effective, &old, &new);
        assert_eq!(
            saved,
            serde_json::json!({"sort_by":"natural", "sort_reverse":true, "show_hidden":true, "linemode":"size"})
        );
    }
}
