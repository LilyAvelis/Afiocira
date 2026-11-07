use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
#[cfg(test)]
use std::time::Duration;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId};
use egui_tty::{Terminal, TerminalStyle, Tty, TtyStream};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value, value};

const BRIDGE_INIT: &str = r#"
-- Photo Workshop's private Yazi session. The user's init.lua is loaded above.
local bridge = os.getenv("PHOTO_WORKSHOP_YAZI_BRIDGE")
if bridge then
  local serial = 0
  local navigation_target = nil
  local function report_folder()
    local current = cx.active.current
    local hovered = current.hovered
    local folder = hovered and hovered.cha.is_dir and hovered.url or current.cwd
    if navigation_target then
      folder = current.cwd
      if tostring(current.cwd) == navigation_target then navigation_target = nil end
    end
    serial = serial + 1
    local version = serial
    ya.async(function()
      fs.write(Url(bridge .. "." .. tostring(version)), tostring(folder) .. "\n")
    end)
  end
  ps.sub("cd", report_folder)
  ps.sub("hover", report_folder)
  ps.sub_remote("workshop-navigate", function(paths)
    if not paths[1] then return end
    navigation_target = paths[2]
    if paths[3] == "folder" then
      ya.emit("cd", { Url(paths[1]) })
    else
      ya.emit("reveal", { Url(paths[1]) })
    end
  end)
  ps.sub_remote("workshop-apply", function(paths)
    local cursor = paths[1]
    if not cursor then return end
    ya.emit("reveal", { Url(cursor) })
    if paths[2] == "replace" then
      ya.emit("toggle_all", { state = "off" })
    end
    if paths[2] == "replace" or paths[2] == "add" then
      for i = 3, #paths do
        ya.emit("reveal", { Url(paths[i]) })
        ya.emit("toggle", { state = "on" })
      end
      ya.emit("reveal", { Url(cursor) })
    end
  end)
end
"#;
const SYNC_PLUGIN: &str = r#"local snapshot = ya.sync(function()
  local tab = cx.active
  local hovered = tab.current.hovered
  local selected = {}
  for _, file in pairs(tab.selected) do
    selected[#selected + 1] = tostring(file.url or file)
  end
  local ordered = {}
  for i = 1, #tab.current.files do
    ordered[#ordered + 1] = tostring(tab.current.files[i].url)
  end
  return tostring(tab.current.cwd), hovered and tostring(hovered.url) or "", selected, ordered
end)

local order_snapshot = ya.sync(function(state)
  state.serial = (state.serial or 0) + 1
  local tab = cx.active
  local ordered = { tostring(tab.current.cwd) }
  for i = 1, #tab.current.files do
    ordered[#ordered + 1] = tostring(tab.current.files[i].url)
  end
  return state.serial, ordered
end)

return {
  entry = function(_, job)
    local target = job.args[1]
    if target == "order" then
      local bridge = os.getenv("PHOTO_WORKSHOP_YAZI_BRIDGE")
      if not bridge then return end
      local serial, lines = order_snapshot()
      fs.write(Url(bridge .. ".order." .. tostring(serial)), table.concat(lines, "\n") .. "\n")
      return
    end
    if not target then return end
    local cwd, hovered, selected, ordered = snapshot()
    local lines = { cwd, hovered }
    for _, path in ipairs(selected) do lines[#lines + 1] = path end
    lines[#lines + 1] = "\0"
    for _, path in ipairs(ordered) do lines[#lines + 1] = path end
    fs.write(Url(target), table.concat(lines, "\n") .. "\n")
  end,
}
"#;
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);
const NERD_FONT_FAMILY: &str = "Yazi Nerd Font";

pub fn install_terminal_font(ctx: &egui::Context) -> bool {
    let Some(windows) = std::env::var_os("WINDIR") else {
        return false;
    };
    let system = PathBuf::from(windows)
        .join("Fonts")
        .join("JetBrainsMonoNerdFontMono-Regular.ttf");
    let bundled = std::env::current_exe().ok().map(|p| {
        p.with_file_name("fonts")
            .join("JetBrainsMonoNerdFontMono-Regular.ttf")
    });
    let bytes = fs::read(system).or_else(|e| bundled.as_ref().map_or(Err(e), fs::read));
    let Ok(bytes) = bytes else {
        return false;
    };
    let mut definitions = FontDefinitions::default();
    definitions.font_data.insert(
        NERD_FONT_FAMILY.to_owned(),
        Arc::new(FontData::from_owned(bytes)),
    );
    let mut terminal_fonts = vec![NERD_FONT_FAMILY.to_owned()];
    if let Some(fallbacks) = definitions.families.get(&FontFamily::Monospace) {
        terminal_fonts.extend(fallbacks.iter().cloned());
    }
    definitions
        .families
        .insert(FontFamily::Name(NERD_FONT_FAMILY.into()), terminal_fonts);
    ctx.set_fonts(definitions);
    true
}

struct Pty {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
}

impl Tty for Pty {
    fn write(&self, bytes: &[u8]) -> egui_tty::Result<()> {
        let mut writer = self.writer.lock().map_err(egui_tty::Error::msg)?;
        writer.write_all(bytes).map_err(egui_tty::Error::msg)?;
        writer.flush().map_err(egui_tty::Error::msg)
    }

    fn resize(&self, cols: u16, rows: u16) -> egui_tty::Result<()> {
        self.master
            .lock()
            .map_err(egui_tty::Error::msg)?
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(egui_tty::Error::msg)
    }
}

pub struct YaziPane {
    terminal: Terminal,
    #[cfg(test)]
    tty: Arc<Pty>,
    child: Option<Box<dyn Child + Send + Sync>>,
    ya_executable: PathBuf,
    session: PathBuf,
    last_folder: Option<PathBuf>,
    last_serial: u64,
    last_order_serial: u64,
    client_id: String,
    snapshot_serial: u64,
    pending_snapshot: Option<PathBuf>,
    navigation_target: Option<PathBuf>,
    navigation_generation: Arc<AtomicU64>,
    navigation_errors: Option<mpsc::Receiver<String>>,
    mouse_down: Option<egui::PointerButton>,
}

pub struct NavigatorSnapshot {
    pub folder: PathBuf,
    pub hovered: Option<PathBuf>,
    pub marked: Vec<PathBuf>,
    pub ordered: Vec<PathBuf>,
}

pub struct NavigatorOrder {
    pub folder: PathBuf,
    pub paths: Vec<PathBuf>,
}

impl YaziPane {
    pub fn start(initial: &Path) -> Result<Self, String> {
        let (yazi_executable, ya_executable) = find_yazi_tools()?;
        let session_number = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let session = std::env::temp_dir().join(format!(
            "photo-workshop-yazi-{}-{}",
            std::process::id(),
            session_number
        ));
        let config = session.join("config");
        let user_config = std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or("APPDATA is missing")?
            .join("yazi/config");
        copy_directory(&user_config, &config).map_err(|error| error.to_string())?;
        patch_config(&config).map_err(|error| error.to_string())?;
        let plugin = config.join("plugins/workshop-sync.yazi");
        fs::create_dir_all(&plugin).map_err(|error| error.to_string())?;
        fs::write(plugin.join("main.lua"), SYNC_PLUGIN).map_err(|error| error.to_string())?;
        let bridge = session.join("folder.txt");
        let client_id = format!("{}", std::process::id() as u64 * 1_000_000 + session_number);

        let pty = native_pty_system()
            .openpty(PtySize {
                // Match egui_tty's initial emulator before its first UI resize.
                // Otherwise early output can scroll fixture files off-screen.
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;
        let mut command = CommandBuilder::new(yazi_executable);
        command.arg(super::friendly_path(initial));
        command.arg("--client-id");
        command.arg(&client_id);
        command.env("TERM", "xterm-256color");
        command.env("YAZI_CONFIG_HOME", &config);
        command.env("PHOTO_WORKSHOP_YAZI_BRIDGE", &bridge);
        let child = pty
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        drop(pty.slave);

        let mut reader = pty
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pty
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        let (sender, output) = mpsc::channel();
        #[cfg(test)]
        let mut log = fs::File::create(session.join("pty.log")).ok();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while let Ok(count) = reader.read(&mut buffer) {
                #[cfg(test)]
                if let Some(log) = &mut log {
                    let _ = log.write_all(&buffer[..count]);
                }
                if count == 0 || sender.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        });
        let tty = Arc::new(Pty {
            writer: Mutex::new(writer),
            master: Mutex::new(pty.master),
        });
        let terminal = Terminal::new(TtyStream {
            output,
            tty: tty.clone(),
        })
        .map_err(|error| error.to_string())?
        .with_label("Yazi navigator");
        Ok(Self {
            terminal,
            #[cfg(test)]
            tty,
            child: Some(child),
            ya_executable,
            session,
            last_folder: None,
            last_serial: 0,
            last_order_serial: 0,
            client_id,
            snapshot_serial: 0,
            pending_snapshot: None,
            navigation_target: None,
            navigation_generation: Arc::new(AtomicU64::new(0)),
            navigation_errors: None,
            mouse_down: None,
        })
    }

    /// Navigate the existing Yazi client without copying config or creating a PTY.
    /// Delivery runs off the UI thread and is cancelled by a newer navigation/drop.
    pub fn navigate(&mut self, path: &Path) -> Result<(), String> {
        self.navigate_mode(path, false)
    }
    /// Highlight the directory itself in its parent, instead of entering it.
    pub fn reveal_folder(&mut self, path: &Path) -> Result<(), String> {
        if path.parent().is_none() {
            return self.navigate(path);
        }
        self.navigate_mode(path, true)
    }
    pub fn navigation_untouched(&self) -> bool {
        self.navigation_target.is_some()
    }
    fn navigate_mode(&mut self, path: &Path, reveal_folder: bool) -> Result<(), String> {
        let is_folder = path.is_dir() && !reveal_folder;
        let target = if is_folder {
            path
        } else {
            path.parent()
                .ok_or("Search result has no parent directory")?
        };
        let target = PathBuf::from(super::friendly_path(target));
        let cursor = super::friendly_path(path);
        if let Some(snapshot) = self.pending_snapshot.take() {
            let _ = fs::remove_file(snapshot);
        }
        // Reports already queued for the old gallery must not reopen it while
        // the DDS request is in flight. Late writes are filtered by target below.
        if let Ok(entries) = fs::read_dir(&self.session) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("folder.txt.")
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        self.navigation_target = Some(target.clone());
        self.last_folder = Some(target.clone());
        let generation = self.navigation_generation.fetch_add(1, Ordering::Relaxed) + 1;
        let cancellation = self.navigation_generation.clone();
        let executable = self.ya_executable.clone();
        let client = self.client_id.clone();
        let (errors, receiver) = mpsc::channel();
        self.navigation_errors = Some(receiver);
        std::thread::Builder::new()
            .name("yazi-navigate".into())
            .spawn(move || {
                if cancellation.load(Ordering::Relaxed) != generation {
                    return;
                }
                let mut command = std::process::Command::new(executable);
                #[cfg(windows)]
                command.creation_flags(0x0800_0000);
                let child = command
                    .args(["pub-to", &client, "workshop-navigate", "--list"])
                    .arg(cursor)
                    .arg(super::friendly_path(&target))
                    .arg(if is_folder { "folder" } else { "file" })
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn();
                let mut child = match child {
                    Ok(child) => child,
                    Err(error) => {
                        let _ = errors.send(format!("Yazi navigation failed: {error}"));
                        return;
                    }
                };
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    match child.try_wait() {
                        Ok(Some(status)) => {
                            if !status.success() {
                                let _ = errors.send(format!("Yazi navigation failed: {status}"));
                            }
                            break;
                        }
                        Err(error) => {
                            let _ = errors.send(format!("Yazi navigation failed: {error}"));
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        Ok(None) => {}
                    }
                    if std::time::Instant::now() >= deadline
                        || cancellation.load(Ordering::Relaxed) != generation
                    {
                        let _ = child.kill();
                        let _ = child.wait();
                        if cancellation.load(Ordering::Relaxed) == generation {
                            let _ = errors.send("Yazi navigation timed out".into());
                        }
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn take_navigation_error(&mut self) -> Option<String> {
        self.navigation_errors.as_ref()?.try_recv().ok()
    }

    pub fn request_snapshot(&mut self) -> Result<(), String> {
        self.snapshot_serial += 1;
        let path = self
            .session
            .join(format!("snapshot.{}", self.snapshot_serial));
        self.pending_snapshot = Some(path.clone());
        let mut command = std::process::Command::new(&self.ya_executable);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW for the console-based ya.exe.
        let status = command
            .arg("emit-to")
            .arg(&self.client_id)
            .arg("plugin")
            .arg("workshop-sync")
            .arg(path.to_string_lossy().replace('\\', "/"))
            .status()
            .map_err(|error| error.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("Yazi snapshot command failed: {status}"))
        }
    }

    pub fn request_order(&self) -> Result<(), String> {
        let mut command = std::process::Command::new(&self.ya_executable);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let status = command
            .arg("emit-to")
            .arg(&self.client_id)
            .arg("plugin")
            .arg("workshop-sync")
            .arg("order")
            .status()
            .map_err(|error| error.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("Yazi order command failed: {status}"))
        }
    }

    pub fn poll_snapshot(&mut self) -> Option<NavigatorSnapshot> {
        let path = self.pending_snapshot.as_ref()?;
        let text = fs::read_to_string(path).ok()?;
        if !text.ends_with('\n') {
            return None;
        }
        let lines: Vec<_> = text.lines().collect();
        if lines.len() < 2 {
            return None;
        }
        let boundary = lines[2..].iter().position(|line| *line == "\0")? + 2;
        let snapshot = NavigatorSnapshot {
            folder: PathBuf::from(lines[0]),
            hovered: (!lines[1].is_empty()).then(|| PathBuf::from(lines[1])),
            marked: lines[2..boundary].iter().map(PathBuf::from).collect(),
            ordered: lines[boundary + 1..].iter().map(PathBuf::from).collect(),
        };
        let _ = fs::remove_file(path);
        self.pending_snapshot = None;
        Some(snapshot)
    }

    pub fn poll_order(&mut self) -> Option<NavigatorOrder> {
        let prefix = "folder.txt.order.";
        let mut reports: Vec<_> = fs::read_dir(&self.session)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let serial = name.strip_prefix(prefix)?.parse::<u64>().ok()?;
                (serial > self.last_order_serial).then_some((serial, entry.path()))
            })
            .collect();
        reports.sort_unstable_by_key(|(serial, _)| std::cmp::Reverse(*serial));
        for (serial, path) in reports {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if !text.ends_with('\n') {
                continue;
            }
            let mut lines = text.lines();
            let folder = PathBuf::from(lines.next()?);
            if self
                .navigation_target
                .as_ref()
                .is_some_and(|target| target != &folder)
            {
                let _ = fs::remove_file(&path);
                continue;
            }
            let paths = lines.map(PathBuf::from).collect();
            self.last_order_serial = serial;
            for entry in fs::read_dir(&self.session).ok()?.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name
                    .strip_prefix(prefix)
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_some_and(|version| version <= serial)
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
            return Some(NavigatorOrder { folder, paths });
        }
        None
    }

    pub fn apply_selection(
        &self,
        cursor: &Path,
        marked: &[PathBuf],
        replace: bool,
    ) -> Result<(), String> {
        // Keep each Windows command line bounded even if the gallery marks thousands of files.
        let mut chunks: Vec<Vec<&PathBuf>> = vec![Vec::new()];
        let mut bytes = 0;
        if replace {
            for path in marked {
                let cost = path.to_string_lossy().len() * 2 + 8;
                if bytes + cost > 8_000 && !chunks.last().unwrap().is_empty() {
                    chunks.push(Vec::new());
                    bytes = 0;
                }
                chunks.last_mut().unwrap().push(path);
                bytes += cost;
            }
        }
        for (index, chunk) in chunks.into_iter().enumerate() {
            let mut command = std::process::Command::new(&self.ya_executable);
            #[cfg(windows)]
            command.creation_flags(0x0800_0000);
            command
                .arg("pub-to")
                .arg(&self.client_id)
                .arg("workshop-apply")
                .arg("--list");
            command.arg(super::friendly_path(cursor));
            command.arg(if !replace {
                "preserve"
            } else if index == 0 {
                "replace"
            } else {
                "add"
            });
            command.args(chunk.into_iter().map(|path| super::friendly_path(path)));
            let status = command.status().map_err(|error| error.to_string())?;
            if !status.success() {
                return Err(format!("Yazi selection command failed: {status}"));
            }
        }
        Ok(())
    }

    pub fn ui(&mut self, ui: &mut eframe::egui::Ui, nerd_font_loaded: bool) -> bool {
        let mut style = TerminalStyle::from_visuals(ui.visuals());
        // Yazi uses bold on powerline caps and selected text. Their explicit
        // colors must remain exact instead of being mixed toward white.
        style.brighten_bold = false;
        if nerd_font_loaded {
            style.font = FontId::new(12.0, FontFamily::Name(NERD_FONT_FAMILY.into()));
        }
        self.terminal.set_color_scheme(style.scheme);
        let background = self
            .terminal
            .drawn_colors()
            .map_or(ui.visuals().panel_fill, |(_, background)| background);
        // The terminal paints complete character cells. Cover the fractional edge left by
        // cell sizing so the surrounding panel cannot show through as bright slivers.
        ui.painter()
            .rect_filled(ui.available_rect_before_wrap(), 0.0, background);
        let response = self.terminal.ui(ui, &style);
        // egui_tty 0.2 handles text selection but does not forward pointer buttons to
        // the PTY. Yazi understands SGR mouse reports, like it does in a normal terminal.
        let events = ui.input(|input| input.events.clone());
        // A search result explicitly chooses its gallery directory. Keep that
        // choice through delayed Yazi hover reports, until the user actually
        // operates this terminal. Programmatic focus alone must not release it.
        if events.iter().any(|event| match event {
            egui::Event::Key { pressed: true, .. }
            | egui::Event::Text(_)
            | egui::Event::Paste(_) => response.has_focus(),
            egui::Event::PointerButton {
                pos, pressed: true, ..
            } => response.rect.contains(*pos),
            egui::Event::MouseWheel { .. } => response.hovered(),
            _ => false,
        }) {
            self.navigation_target = None;
        }
        for event in events {
            let egui::Event::PointerButton {
                pos,
                button,
                pressed,
                ..
            } = event
            else {
                continue;
            };
            if pressed && response.rect.contains(pos) {
                let history_key = match button {
                    egui::PointerButton::Extra1 => Some(b"H".as_slice()),
                    egui::PointerButton::Extra2 => Some(b"L".as_slice()),
                    _ => None,
                };
                if let Some(key) = history_key {
                    response.request_focus();
                    let _ = self.terminal.send(key);
                    continue;
                }
            }
            if matches!(
                button,
                egui::PointerButton::Extra1 | egui::PointerButton::Extra2
            ) {
                continue;
            }
            let active = if pressed {
                if !response.rect.contains(pos) {
                    continue;
                }
                self.mouse_down = Some(button);
                true
            } else if self.mouse_down == Some(button) {
                self.mouse_down = None;
                true
            } else {
                false
            };
            if !active {
                continue;
            }
            let cell = egui_tty::cell_size(ui.painter(), &style.font);
            let (cols, rows) = self.terminal.size();
            let column = (((pos.x - response.rect.min.x - style.padding) / cell.x).floor() as i32
                + 1)
            .clamp(1, i32::from(cols));
            let row = (((pos.y - response.rect.min.y - style.padding) / cell.y).floor() as i32 + 1)
                .clamp(1, i32::from(rows));
            let code = match button {
                egui::PointerButton::Primary => 0,
                egui::PointerButton::Middle => 1,
                egui::PointerButton::Secondary => 2,
                _ => continue,
            };
            let suffix = if pressed { 'M' } else { 'm' };
            let report = format!("\x1b[<{code};{column};{row}{suffix}");
            let _ = self.terminal.send(report.as_bytes());
        }
        if response.double_clicked() {
            // Yazi's left click moves its cursor. Its terminal mouse handler does not
            // open on a double click, so complete the familiar gesture with `enter`.
            let _ = self.terminal.send(b"l");
        }
        response.has_focus()
    }

    pub fn request_focus(&mut self) {
        self.terminal.request_focus();
    }

    pub fn is_running(&mut self) -> bool {
        self.child
            .as_mut()
            .is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_none()))
    }

    pub fn changed_folder(&mut self) -> Option<PathBuf> {
        let mut reports: Vec<_> = fs::read_dir(&self.session)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let serial = name.strip_prefix("folder.txt.")?.parse::<u64>().ok()?;
                if serial <= self.last_serial {
                    let _ = fs::remove_file(entry.path());
                    None
                } else {
                    Some((serial, entry.path()))
                }
            })
            .collect();
        reports.sort_unstable_by_key(|(serial, _)| std::cmp::Reverse(*serial));
        for (serial, report) in reports {
            let Ok(text) = fs::read_to_string(&report) else {
                continue;
            };
            let Some(path) = text.strip_suffix('\n') else {
                continue;
            };
            let folder = PathBuf::from(path);
            if self
                .navigation_target
                .as_ref()
                .is_some_and(|target| target != &folder)
            {
                let _ = fs::remove_file(report);
                continue;
            }
            if !folder.is_dir() {
                continue;
            }
            self.last_serial = serial;
            for entry in fs::read_dir(&self.session).ok()?.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name
                    .strip_prefix("folder.txt.")
                    .and_then(|value| value.parse::<u64>().ok())
                    .is_some_and(|version| version <= serial)
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
            if self.last_folder.as_deref() == Some(&folder) {
                return None;
            }
            self.last_folder = Some(folder.clone());
            return Some(folder);
        }
        None
    }
}

impl Drop for YaziPane {
    fn drop(&mut self) {
        self.navigation_generation.fetch_add(1, Ordering::Relaxed);
        if let Some(mut child) = self.child.take() {
            let session = self.session.clone();
            std::thread::spawn(move || {
                let _ = child.kill();
                let _ = child.wait();
                let _ = fs::remove_dir_all(session);
            });
        }
    }
}

fn find_yazi_tools() -> Result<(PathBuf, PathBuf), String> {
    let path = std::env::var_os("PATH");
    find_yazi_tools_with_path(path.as_deref())
}

fn find_yazi_tools_with_path(path: Option<&OsStr>) -> Result<(PathBuf, PathBuf), String> {
    let mut directories: Vec<PathBuf> = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| vec![p.join("yazi"), p.to_path_buf()]))
        .unwrap_or_default();
    directories.extend(path.map(std::env::split_paths).into_iter().flatten());
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let packages = PathBuf::from(local).join("Microsoft/WinGet/Packages");
        if let Ok(entries) = fs::read_dir(packages) {
            for entry in entries.flatten() {
                if !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("sxyazi.yazi_")
                {
                    continue;
                }
                directories.push(entry.path());
                if let Ok(children) = fs::read_dir(entry.path()) {
                    directories.extend(children.flatten().filter_map(|child| {
                        child.file_type().ok()?.is_dir().then_some(child.path())
                    }));
                }
            }
        }
    }
    for directory in directories {
        let yazi = directory.join("yazi.exe");
        let ya = directory.join("ya.exe");
        if yazi.is_file() && ya.is_file() {
            return Ok((yazi, ya));
        }
    }
    Err("Yazi and ya.exe were not found in the same installation".into())
}

fn copy_directory(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    if !from.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let destination = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_directory(&entry.path(), &destination)?;
        } else {
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

fn patch_config(config: &Path) -> std::io::Result<()> {
    let path = config.join("yazi.toml");
    let original = read_optional_text(&path)?;
    let mut document = parse_toml(&original)?;
    ensure_table(&mut document, "mgr")?;
    document["mgr"]["ratio"] = value(Array::from_iter([0, 1, 0]));
    ensure_table(&mut document, "plugin")?;
    for key in ["prepend_previewers", "prepend_preloaders"] {
        let item = &mut document["plugin"][key];
        if item.is_none() {
            *item = Item::ArrayOfTables(ArrayOfTables::new());
        }
        let entries = item
            .as_array_of_tables_mut()
            .ok_or_else(|| invalid_config(key))?;
        entries.insert(
            0,
            Table::from_iter([("mime", value("image/*")), ("run", value("noop"))]),
        );
    }
    fs::write(path, document.to_string())?;
    let init = config.join("init.lua");
    let mut content = read_optional_text(&init)?;
    content.push_str(BRIDGE_INIT);
    fs::write(init, content)?;
    let keymap = config.join("keymap.toml");
    let original = read_optional_text(&keymap)?;
    fs::write(keymap, patch_keymap(&original)?)
}

fn invalid_config(section: &str) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("Invalid Yazi config section: {section}"),
    )
}

fn parse_toml(text: &str) -> std::io::Result<DocumentMut> {
    text.parse()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn ensure_table(document: &mut DocumentMut, key: &str) -> std::io::Result<()> {
    if !document.contains_key(key) {
        document[key] = Item::Table(Table::new());
    }
    document[key]
        .as_table()
        .ok_or_else(|| invalid_config(key))?;
    Ok(())
}

fn read_optional_text(path: &Path) -> std::io::Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error),
    }
}

fn patch_keymap(original: &str) -> std::io::Result<String> {
    let mut document = parse_toml(original)?;
    ensure_table(&mut document, "mgr")?;
    let item = &mut document["mgr"]["prepend_keymap"];
    if item.is_none() {
        *item = value(Array::new());
    }
    let entries = item
        .as_array_mut()
        .ok_or_else(|| invalid_config("mgr.prepend_keymap"))?;
    // Keep the user's own sort bindings, including the Cyrillic mirrors, and add a
    // notification after their original actions. Default English bindings need a private
    // override because Yazi's shipped keymap is not present in the copied config.
    for entry in entries.iter_mut() {
        let Some(mapping) = entry.as_inline_table_mut() else {
            continue;
        };
        let Some(run) = mapping.get("run") else {
            continue;
        };
        let commands: Vec<_> = if let Some(command) = run.as_str() {
            vec![command.to_owned()]
        } else if let Some(array) = run.as_array() {
            array
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        } else {
            continue;
        };
        if commands.iter().any(|command| command.starts_with("sort ")) {
            let mut augmented = Array::new();
            for command in commands {
                augmented.push(command);
            }
            augmented.push("plugin workshop-sync order");
            mapping.insert("run", Value::Array(augmented));
        }
    }
    let sorts = [
        ("m", "mtime", false),
        ("M", "mtime", true),
        ("b", "btime", false),
        ("B", "btime", true),
        ("e", "extension", false),
        ("E", "extension", true),
        ("a", "alphabetical", false),
        ("A", "alphabetical", true),
        ("n", "natural", false),
        ("N", "natural", true),
        ("s", "size", false),
        ("S", "size", true),
        ("r", "random", false),
    ];
    for (key, method, reverse) in sorts {
        let configured = entries.iter().any(|entry| {
            entry
                .as_inline_table()
                .and_then(|mapping| mapping.get("on"))
                .and_then(Value::as_array)
                .is_some_and(|on| {
                    on.get(0).and_then(Value::as_str) == Some(",")
                        && on.get(1).and_then(Value::as_str) == Some(key)
                })
        });
        if configured {
            continue;
        }
        let mut mapping = InlineTable::new();
        mapping.insert("on", Value::Array(Array::from_iter([",", key])));
        let command = format!(
            "sort {method} --reverse={}",
            if reverse { "yes" } else { "no" }
        );
        let mut run = Array::new();
        run.push(command);
        if method == "mtime" || method == "btime" || method == "size" {
            run.push(format!("linemode {method}"));
        }
        run.push("plugin workshop-sync order");
        mapping.insert("run", Value::Array(run));
        mapping.insert(
            "desc",
            Value::from(format!("Sort by {method} (sync gallery)")),
        );
        entries.insert(0, mapping);
    }
    let mut remap = InlineTable::new();
    remap.insert("on", Value::from("<S-Tab>"));
    remap.insert("run", Value::from("spot"));
    remap.insert(
        "desc",
        Value::from("File information (Tab in standalone Yazi)"),
    );
    entries.insert(0, remap);
    Ok(document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    static NATIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn finds_ya_beside_yazi_when_path_is_unavailable() {
        let (yazi, ya) = find_yazi_tools_with_path(None).unwrap();
        assert_eq!(yazi.parent(), ya.parent());
        assert!(yazi.is_file());
        assert!(ya.is_file());
    }
    use std::time::Instant;

    #[test]
    fn installed_nerd_font_contains_yazi_folder_icon() {
        let ctx = egui::Context::default();
        assert!(install_terminal_font(&ctx));
        ctx.begin_pass(egui::RawInput::default());
        let font = FontId::new(12.0, FontFamily::Name(NERD_FONT_FAMILY.into()));
        assert!(ctx.fonts_mut(|fonts| fonts.has_glyph(&font, '')));
        ctx.end_pass().textures_delta.clear();
    }

    #[test]
    fn private_config_hides_preview_without_changing_user_config() {
        let root = std::env::temp_dir().join(format!(
            "photo-workshop-yazi-config-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("yazi.toml"),
            "[mgr]\nratio = [1, 2, 3]\nsort_by = \"natural\"\n",
        )
        .unwrap();
        fs::write(
            root.join("init.lua"),
            "require(\"mime-ext.local\"):setup {}\n",
        )
        .unwrap();
        patch_config(&root).unwrap();
        let config = fs::read_to_string(root.join("yazi.toml")).unwrap();
        config.parse::<DocumentMut>().unwrap();
        assert!(config.contains("ratio = [0, 1, 0]"));
        assert!(!config.contains("ratio = [1, 2, 3]"));
        assert!(config.contains("prepend_preloaders"));
        assert!(
            fs::read_to_string(root.join("init.lua"))
                .unwrap()
                .contains("ps.sub(\"hover\"")
        );
        assert!(
            fs::read_to_string(root.join("keymap.toml"))
                .unwrap()
                .contains("<S-Tab>")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn keymap_patch_does_not_duplicate_existing_manager_section() {
        let original = "[mgr]\nkeymap = []\n[input]\nprepend_keymap = []\n";
        let patched = patch_keymap(original).unwrap();
        patched.parse::<DocumentMut>().unwrap();
        assert_eq!(patched.matches("[mgr]").count(), 1);
        assert!(patched.find("<S-Tab>").unwrap() < patched.find("[input]").unwrap());
        assert_eq!(patched.matches("<S-Tab>").count(), 1);
    }

    #[test]
    fn keymap_patch_keeps_empty_and_unrelated_sections_valid() {
        for original in [
            "",
            "[input]\nprepend_keymap = []\n",
            "[mgr]\n# custom comment\n[input]\nkeymap = []\n",
        ] {
            let patched = patch_keymap(original).unwrap();
            let document = patched.parse::<DocumentMut>().unwrap();
            assert_eq!(
                document["mgr"]["prepend_keymap"].as_array().unwrap().len(),
                14
            );
            if original.contains("[input]") {
                assert!(document.get("input").is_some());
            }
        }
    }

    #[test]
    fn native_yazi_navigates_existing_client() {
        let _lock = NATIVE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture =
            std::env::temp_dir().join(format!("photo-workshop-navigate-{}", std::process::id()));
        let folder = fixture.join("Unicode папка");
        fs::create_dir_all(folder.join("hovered-child")).unwrap();
        let photo = fixture.join("image.jpg");
        fs::write(&photo, []).unwrap();
        let started = Instant::now();
        let mut pane = YaziPane::start(&fixture).unwrap();
        let client_id = pane.client_id.clone();
        let child_pid = pane.child.as_ref().unwrap().process_id();
        eprintln!("Yazi start: {:?}", started.elapsed());
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            pane.terminal.poll();
            let _ = pane.request_snapshot();
            std::thread::sleep(Duration::from_millis(40));
            if pane.poll_snapshot().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "bridge failed to start");
        }
        for target in [&folder, &photo] {
            let started = Instant::now();
            pane.navigate(target).unwrap();
            assert_eq!(pane.client_id, client_id);
            assert_eq!(pane.child.as_ref().unwrap().process_id(), child_pid);
            eprintln!("navigate queued: {:?}", started.elapsed());
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                pane.terminal.poll();
                if let Some(reported) = pane.changed_folder() {
                    let expected = if target == &folder { &folder } else { &fixture };
                    assert_eq!(
                        &reported, expected,
                        "navigation reported a hovered child or old folder"
                    );
                }
                pane.request_snapshot().unwrap();
                std::thread::sleep(Duration::from_millis(30));
                if let Some(snapshot) = pane.poll_snapshot() {
                    let expected = if target == &folder { &folder } else { &fixture };
                    if snapshot.folder == *expected
                        && (target == &folder || snapshot.hovered.as_ref() == Some(&photo))
                    {
                        eprintln!("navigation complete: {:?}", started.elapsed());
                        // Simulate an arbitrarily late hover report after the
                        // requested cd already completed. No timer may let it
                        // replace the explicitly chosen search directory.
                        let late = pane
                            .session
                            .join(format!("folder.txt.{}", pane.last_serial + 10_000));
                        fs::write(
                            &late,
                            format!("{}\n", folder.join("hovered-child").display()),
                        )
                        .unwrap();
                        assert!(pane.changed_folder().is_none());
                        assert_eq!(pane.navigation_target.as_ref(), Some(expected));
                        let settle = Instant::now() + Duration::from_millis(200);
                        while Instant::now() < settle {
                            pane.terminal.poll();
                            if let Some(reported) = pane.changed_folder() {
                                assert_eq!(&reported, expected);
                            }
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        break;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "DDS did not navigate to {target:?}"
                );
            }
        }
        // No delayed report from the previous folder may reopen it after the
        // newest media reveal. Continue polling beyond the DDS completion.
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            pane.terminal.poll();
            if let Some(reported) = pane.changed_folder() {
                assert_eq!(reported, fixture);
            }
            assert!(pane.take_navigation_error().is_none());
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(pane);
        let _ = fs::remove_dir_all(fixture);
    }

    #[test]
    fn native_yazi_reports_hovered_folder() {
        let _lock = NATIVE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture = std::env::temp_dir().join(format!(
            "photo-workshop-yazi-fixture-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&fixture);
        let child_folder = fixture.join("a-subfolder");
        let second_folder = fixture.join("b-subfolder");
        fs::create_dir_all(&child_folder).unwrap();
        fs::create_dir_all(&second_folder).unwrap();
        let mut pane = YaziPane::start(&fixture.canonicalize().unwrap())
            .expect("installed Yazi should start in ConPTY");
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut reported = None;
        while Instant::now() < deadline {
            pane.terminal.poll();
            if let Some(folder) = pane.changed_folder() {
                reported = Some(folder);
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        if reported.is_none() {
            eprintln!(
                "child status: {:?}",
                pane.child.as_mut().unwrap().try_wait()
            );
            eprintln!(
                "bridge: {:?}",
                fs::read_to_string(pane.session.join("folder.txt"))
            );
            let log = fs::read(pane.session.join("pty.log")).unwrap_or_default();
            eprintln!(
                "pty bytes: {}",
                String::from_utf8_lossy(&log)
                    .chars()
                    .take(1500)
                    .collect::<String>()
            );
        }
        assert_eq!(reported, Some(child_folder));
        pane.tty.write(b"j").unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut moved = None;
        while Instant::now() < deadline {
            pane.terminal.poll();
            if let Some(folder) = pane.changed_folder() {
                moved = Some(folder);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(moved, Some(second_folder.clone()));
        pane.request_snapshot().unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut snapshot = None;
        while Instant::now() < deadline {
            pane.terminal.poll();
            if let Some(value) = pane.poll_snapshot() {
                snapshot = Some(value);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(snapshot.unwrap().hovered, Some(second_folder));
        let first_photo = fixture.join("b-photo.jpg");
        let second_photo = fixture.join("c-photo.jpg");
        fs::write(&first_photo, b"test").unwrap();
        fs::write(&second_photo, b"test").unwrap();
        let mut gallery = photo_core::Gallery::open(&fixture).unwrap();
        assert!(gallery.toggle_selection(0));
        assert!(gallery.toggle_selection(1));
        let ctrl_marks: Vec<_> = gallery.marked_paths().iter().cloned().collect();
        pane.apply_selection(&first_photo, &ctrl_marks, true)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut applied = None;
        while Instant::now() < deadline {
            pane.terminal.poll();
            pane.request_snapshot().unwrap();
            std::thread::sleep(Duration::from_millis(80));
            if let Some(value) = pane.poll_snapshot()
                && value.hovered.as_deref() == Some(first_photo.as_path())
                && value.marked.contains(&first_photo)
                && value.marked.contains(&second_photo)
            {
                applied = Some(value);
                break;
            }
        }
        assert!(applied.is_some(), "Yazi did not receive cursor and marks");
        pane.tty.write(b" ").unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut toggled = false;
        while Instant::now() < deadline {
            pane.terminal.poll();
            pane.request_snapshot().unwrap();
            std::thread::sleep(Duration::from_millis(80));
            if let Some(value) = pane.poll_snapshot()
                && !value.marked.contains(&first_photo)
                && value.marked.contains(&second_photo)
            {
                toggled = true;
                break;
            }
        }
        assert!(toggled, "Yazi Space selection was not reported back");
        let outside_photo = fixture.join("a-subfolder/outside.jpg");
        fs::write(&outside_photo, b"test").unwrap();
        pane.apply_selection(&outside_photo, &[], false).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline {
            pane.terminal.poll();
            pane.request_snapshot().unwrap();
            std::thread::sleep(Duration::from_millis(80));
            if pane
                .poll_snapshot()
                .is_some_and(|value| value.hovered.as_deref() == Some(outside_photo.as_path()))
            {
                break;
            }
        }
        pane.tty.write(b" ").unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut outside_marked = false;
        while Instant::now() < deadline {
            pane.terminal.poll();
            pane.request_snapshot().unwrap();
            std::thread::sleep(Duration::from_millis(80));
            if pane
                .poll_snapshot()
                .is_some_and(|value| value.marked.contains(&outside_photo))
            {
                outside_marked = true;
                break;
            }
        }
        assert!(
            outside_marked,
            "Could not select photo in another directory"
        );
        pane.apply_selection(&first_photo, std::slice::from_ref(&first_photo), true)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut preserved = false;
        while Instant::now() < deadline {
            pane.terminal.poll();
            pane.request_snapshot().unwrap();
            std::thread::sleep(Duration::from_millis(80));
            if pane.poll_snapshot().is_some_and(|value| {
                value.hovered.as_deref() == Some(first_photo.as_path())
                    && value.marked.contains(&first_photo)
                    && value.marked.contains(&outside_photo)
            }) {
                preserved = true;
                break;
            }
        }
        assert!(
            preserved,
            "Gallery selection overwrote marks outside its folder"
        );
        drop(pane);
        for _ in 0..20 {
            if fs::remove_dir_all(&fixture).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    fn native_yazi_sort_and_mouse_reach_the_bridge() {
        let _lock = NATIVE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture =
            std::env::temp_dir().join(format!("photo-workshop-sort-mouse-{}", std::process::id()));
        let _ = fs::remove_dir_all(&fixture);
        fs::create_dir_all(&fixture).unwrap();
        fs::create_dir_all(fixture.join("folder-a")).unwrap();
        for name in ["a.jpg", "b.jpg", "c.jpg"] {
            fs::write(fixture.join(name), b"test").unwrap();
        }
        let mut pane = YaziPane::start(&fixture).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut ready = false;
        while Instant::now() < deadline {
            pane.terminal.poll();
            if pane.terminal.visible_text().unwrap().contains("b.jpg") {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(ready, "Yazi did not render fixture files");
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut bridge_ready = false;
        while Instant::now() < deadline {
            pane.terminal.poll();
            if pane.request_snapshot().is_ok() {
                std::thread::sleep(Duration::from_millis(30));
                if pane.poll_snapshot().is_some() {
                    bridge_ready = true;
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        assert!(bridge_ready, "Yazi bridge did not become ready");
        pane.tty.write(b",A").unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut sorted = None;
        while Instant::now() < deadline {
            pane.terminal.poll();
            if let Some(order) = pane.poll_order() {
                sorted = Some(order);
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        let order = sorted.expect("Yazi sort binding did not report order");
        assert_eq!(order.folder, fixture);
        let names: Vec<_> = order
            .paths
            .iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "jpg"))
            .filter_map(|path| path.file_name())
            .collect();
        assert_eq!(
            names,
            [
                OsStr::new("c.jpg"),
                OsStr::new("b.jpg"),
                OsStr::new("a.jpg")
            ]
        );

        let deadline = Instant::now() + Duration::from_secs(4);
        let row = loop {
            pane.terminal.poll();
            let screen = pane.terminal.visible_text().unwrap();
            if let Some(index) = screen.lines().position(|line| line.contains("b.jpg")) {
                break index + 1;
            }
            assert!(
                Instant::now() < deadline,
                "b.jpg did not render after sorting; screen: {screen:?}"
            );
            std::thread::sleep(Duration::from_millis(30));
        };
        pane.tty
            .write(format!("\x1b[<0;5;{row}M\x1b[<0;5;{row}m").as_bytes())
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut hovered = None;
        while Instant::now() < deadline {
            pane.terminal.poll();
            pane.request_snapshot().unwrap();
            std::thread::sleep(Duration::from_millis(30));
            if let Some(snapshot) = pane.poll_snapshot() {
                hovered = snapshot.hovered;
                if hovered.as_deref() == Some(fixture.join("b.jpg").as_path()) {
                    break;
                }
            }
        }
        assert_eq!(hovered, Some(fixture.join("b.jpg")));
        let screen = pane.terminal.visible_text().unwrap();
        let row = screen
            .lines()
            .position(|line| line.contains("folder-a"))
            .unwrap()
            + 1;
        let click = format!("\x1b[<0;5;{row}M\x1b[<0;5;{row}m");
        pane.tty.write(click.as_bytes()).unwrap();
        pane.tty.write(b"l").unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut entered = false;
        while Instant::now() < deadline {
            pane.terminal.poll();
            pane.request_snapshot().unwrap();
            std::thread::sleep(Duration::from_millis(30));
            if let Some(snapshot) = pane.poll_snapshot()
                && snapshot.folder == fixture.join("folder-a")
            {
                entered = true;
                break;
            }
        }
        assert!(entered, "double click did not enter the Yazi folder");
        pane.tty.write(b"H").unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut back = false;
        while Instant::now() < deadline {
            pane.terminal.poll();
            pane.request_snapshot().unwrap();
            std::thread::sleep(Duration::from_millis(30));
            if let Some(snapshot) = pane.poll_snapshot()
                && snapshot.folder == fixture
            {
                back = true;
                break;
            }
        }
        assert!(
            back,
            "Yazi history back did not return to the parent folder"
        );
        pane.tty.write(b"L").unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let mut forward = false;
        while Instant::now() < deadline {
            pane.terminal.poll();
            pane.request_snapshot().unwrap();
            std::thread::sleep(Duration::from_millis(30));
            if let Some(snapshot) = pane.poll_snapshot()
                && snapshot.folder == fixture.join("folder-a")
            {
                forward = true;
                break;
            }
        }
        assert!(
            forward,
            "Yazi history forward did not restore the child folder"
        );
        drop(pane);
        let _ = fs::remove_dir_all(fixture);
    }
}
