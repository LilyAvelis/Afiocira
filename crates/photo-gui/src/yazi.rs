use std::ffi::OsStr;
use std::fs;
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
#[cfg(test)]
use std::time::Duration;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId};
use egui_tty::{ColorScheme, Terminal, TerminalStyle, Tty, TtyStream};
use photo_core::settings::YaziThemeMode;
use portable_pty::{Child, ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value, value};

#[path = "yazi_protocol.rs"]
mod protocol;

const BRIDGE_INIT: &str = r#"
-- Photo Workshop's private Yazi session. The user's init.lua is loaded above.
local bridge = os.getenv("PHOTO_WORKSHOP_YAZI_BRIDGE")
if bridge then
  -- On Windows Yazi 26.5 can miss a reset when the DDS host exits.
  -- A no-op outbound message lets its existing reconnect path recover.
  -- The private kind has no navigation, selection, or user-config effects.
  ps.sub_remote("workshop-heartbeat", function() end)
  ya.async(function()
    while true do
      ya.sleep(1)
      ps.pub_to(0, "workshop-heartbeat", {})
    end
  end)
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
  local function navigate(paths)
    if not paths[1] then return end
    navigation_target = paths[2]
    if paths[3] == "folder" then
      ya.emit("cd", { Url(paths[1]) })
    else
      ya.emit("reveal", { Url(paths[1]) })
    end
  end
  ps.sub_remote("workshop-navigate", navigate)
  ps.sub("workshop-navigate", navigate)
  -- One immutable ordered stream for all managed work. A request remains owned
  -- by the consumer until the plugin's state barrier has acknowledged its ID.
  ya.async(function()
    local sequence = 1
    while true do
      ya.sleep(0.025)
      local name = bridge .. ".request." .. tostring(sequence)
      local claimed = bridge .. ".processing." .. tostring(sequence)
      local file = os.rename(name, claimed) and io.open(claimed, "rb")
      if file then
        local text = file:read("*a")
        file:close()
        local ok, request = pcall(ya.json_decode, text)
        if not ok or not request or request.id ~= sequence then
          fs.write(Url(bridge .. ".ack." .. tostring(sequence)), ya.json_encode({ id = sequence, ok = false, error = "Invalid private request" }))
        else
          ya.emit("plugin", { "workshop-sync", ya.json_encode(text) })
        end
        -- No following request may overtake an unacknowledged mutation.
        while true do
          local ack = io.open(bridge .. ".ack." .. tostring(sequence), "rb")
          if ack then ack:close(); break end
          ya.sleep(0.01)
        end
        os.remove(claimed)
        sequence = sequence + 1
      end
    end
  end)
end
"#;
const SYNC_PLUGIN: &str = r#"--- @sync entry
local function write(path, text)
  local ok, failure = fs.write(Url(path), text)
  if not ok then error("Private response write failed: " .. tostring(failure)) end
end
local function rename(from, to)
  local ok, failure = os.rename(from, to)
  if not ok then error("Private response publish failed: " .. tostring(failure)) end
end
local snapshot = ya.sync(function()
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
  local p = tab.pref
  local pref = { sort_by = p.sort_by, sort_reverse = p.sort_reverse, sort_sensitive = p.sort_sensitive,
    sort_dir_first = p.sort_dir_first, sort_translit = p.sort_translit, sort_fallback = p.sort_fallback,
    show_hidden = p.show_hidden, linemode = p.linemode }
  return tostring(tab.current.cwd), hovered and tostring(hovered.url) or "", selected, ordered, ya.json_encode(pref), #cx.tabs, ya.json_encode({ light = rt.term.light, theme = { th.mgr.cwd:raw(), th.mgr.border_style:raw(), th.app.overall:raw(), th.mgr.border_symbol }, layer = tostring(cx.layer), tasks = cx.tasks.summary.total or 0, yanked = #cx.yanked })
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
    -- Protocol work is not a user-scheduled Yazi task. A short synchronous
    -- entry starts an async callback; snapshots must not count their own job.
    ya.async(function()
    if target == "order" then
      local bridge = os.getenv("PHOTO_WORKSHOP_YAZI_BRIDGE")
      if not bridge then return end
      local serial, lines = order_snapshot()
      fs.write(Url(bridge .. ".order." .. tostring(serial)), table.concat(lines, "\n") .. "\n")
      return
    end
    if not target then return end
    local request = nil
    local ok, decoded = pcall(ya.json_decode, target)
    if ok and type(decoded) == "table" then request = decoded end
    if request then
      local bridge = os.getenv("PHOTO_WORKSHOP_YAZI_BRIDGE")
      if not bridge then return end
      local ack = bridge .. ".ack." .. tostring(request.id)
      local cancelled = bridge .. ".cancel." .. tostring(request.id)
      local cancel_file = io.open(cancelled, "rb")
      if cancel_file then
        cancel_file:close()
        write(ack, ya.json_encode({ id = request.id, ok = true, cancelled = true }))
        return
      end
      local success, failure = pcall(function()
        local function key(path) return tostring(path):gsub("\\", "/"):lower() end
        local function set(paths)
          local result = {}
          for _, path in ipairs(paths or {}) do result[key(path)] = path end
          return result
        end
        local desired = set(request.marked)
        local scope = set(request.scope)
        local function apply_delta()
          local _, _, selected = snapshot()
          local actual = set(selected)
          for path, original in pairs(actual) do
            if scope[path] and not desired[path] then ya.emit("toggle", { Url(original), state = "off" }) end
          end
          for path, original in pairs(desired) do
            if not actual[path] then ya.emit("toggle", { Url(original), state = "on" }) end
          end
        end
        if request.operation == "apply" then
          -- Yazi 26.5.6 toggle accepts an explicit URL. Diff actual selection,
          -- so loaded thumbnails never cause thousands of reveals or toggles.
          if request.replace then apply_delta() end
          ya.emit("reveal", { Url(request.cursor) })
        elseif request.operation == "navigate" then
          ya.emit(request.paths[3] == "folder" and "cd" or "reveal", { Url(request.paths[1]) })
        elseif request.operation == "emit" then
          local args = {}
          for key, value in pairs(request.args or {}) do args[tonumber(key) or key] = value end
          ya.exec(request.action, args)
        elseif request.operation == "order" then
          local serial, lines = order_snapshot()
          write(bridge .. ".order." .. tostring(serial), table.concat(lines, "\n") .. "\n")
        end
        -- Acknowledgement proves the observed result, rather than assuming a
        -- fixed delay means emitted commands have run. Explicit state mutations
        -- are idempotent; retries remain inside this same logical request ID.
        local verified = request.operation == "snapshot" or request.operation == "order"
        for attempt = 1, 100 do
          if verified then break end
          ya.sleep(0.01)
          local cwd, hovered, selected, _, preferences = snapshot()
          if request.operation == "apply" then
            verified = key(hovered) == key(request.cursor)
            if request.replace then
              local actual = set(selected)
              for path in pairs(scope) do
                if (actual[path] ~= nil) ~= (desired[path] ~= nil) then verified = false end
              end
              if not verified and attempt % 10 == 0 then apply_delta() end
            end
          elseif request.operation == "navigate" then
            verified = key(cwd) == key(request.paths[2])
              and (request.paths[3] == "folder" or key(hovered) == key(request.paths[1]))
          elseif request.operation == "emit" then
            local pref = ya.json_decode(preferences)
            local args = request.args or {}
            verified = true
            if request.action == "sort" then
              if args["1"] and pref.sort_by ~= args["1"] then verified = false end
              for _, name in ipairs({ "reverse", "sensitive", "dir_first", "translit", "fallback" }) do
                if args[name] ~= nil and pref["sort_" .. name] ~= args[name] then verified = false end
              end
            elseif request.action == "hidden" then
              verified = pref.show_hidden == (args["1"] == "show")
            elseif request.action == "linemode" then
              verified = pref.linemode == args["1"]
            end
          end
        end
        if not verified then error("Private mutation did not reach its expected Yazi state") end
        if request.operation == "snapshot" then
          local cwd, hovered, selected, ordered, preferences, tabs, metadata = snapshot()
          local lines = { cwd, hovered, preferences, tostring(tabs), metadata }
          for _, path in ipairs(selected) do lines[#lines + 1] = path end
          lines[#lines + 1] = "\0"
          for _, path in ipairs(ordered) do lines[#lines + 1] = path end
          local temp = request.target .. ".tmp"
          write(temp, table.concat(lines, "\n") .. "\n")
          rename(temp, request.target)
        else
          snapshot()
        end
      end)
      local temp = ack .. ".tmp"
      write(temp, ya.json_encode({ id = request.id, ok = success, error = success and nil or tostring(failure) }))
      rename(temp, ack)
      return
    end
    local cwd, hovered, selected, ordered, preferences, tabs, metadata = snapshot()
    local lines = { cwd, hovered, preferences, tostring(tabs), metadata }
    for _, path in ipairs(selected) do lines[#lines + 1] = path end
    lines[#lines + 1] = "\0"
    for _, path in ipairs(ordered) do lines[#lines + 1] = path end
    fs.write(Url(target), table.concat(lines, "\n") .. "\n")
    end)
  end,
}
"#;
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);
const NERD_FONT_FAMILY: &str = "Yazi Nerd Font";

pub fn install_terminal_font(ctx: &egui::Context) -> bool {
    let system_fonts =
        std::env::var_os("WINDIR").map(|windows| PathBuf::from(windows).join("Fonts"));
    let bundled = std::env::current_exe().ok().map(|p| {
        p.with_file_name("fonts")
            .join("JetBrainsMonoNerdFontMono-Regular.ttf")
    });
    let nerd = system_fonts
        .as_ref()
        .and_then(|directory| {
            fs::read(directory.join("JetBrainsMonoNerdFontMono-Regular.ttf")).ok()
        })
        .or_else(|| bundled.and_then(|path| fs::read(path).ok()));
    let nerd_loaded = nerd.is_some();
    let mut definitions = FontDefinitions::default();
    let mut fallbacks = Vec::new();
    if let Some(bytes) = nerd {
        definitions.font_data.insert(
            NERD_FONT_FAMILY.to_owned(),
            Arc::new(FontData::from_owned(bytes)),
        );
        fallbacks.push(NERD_FONT_FAMILY.to_owned());
    }
    // egui does not consult Windows' font fallback automatically. Use one explicit
    // fallback order in Settings and the terminal, without installing system fonts.
    // Missing optional fonts simply leave the built-in faces available.
    if let Some(directory) = system_fonts {
        for filename in [
            "segoeui.ttf",
            "seguisym.ttf",
            "seguiemj.ttf",
            "msyh.ttc",
            "malgun.ttf",
        ] {
            if let Ok(bytes) = fs::read(directory.join(filename)) {
                definitions
                    .font_data
                    .insert(filename.to_owned(), Arc::new(FontData::from_owned(bytes)));
                fallbacks.push(filename.to_owned());
            }
        }
    }
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        definitions
            .families
            .entry(family)
            .or_default()
            .extend(fallbacks.iter().cloned());
    }
    if nerd_loaded {
        let mut terminal_fonts = vec![NERD_FONT_FAMILY.to_owned()];
        terminal_fonts.extend(
            definitions.families[&FontFamily::Monospace]
                .iter()
                .filter(|name| name.as_str() != NERD_FONT_FAMILY)
                .cloned(),
        );
        definitions
            .families
            .insert(FontFamily::Name(NERD_FONT_FAMILY.into()), terminal_fonts);
    }
    ctx.set_fonts(definitions);
    nerd_loaded
}

struct Pty {
    input: mpsc::SyncSender<Vec<u8>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    input_encoding: Mutex<ConptyInput>,
}

#[derive(Debug)]
struct BootstrapChild {
    bootstrap: Box<dyn Child + Send + Sync>,
    receipt: PathBuf,
}
impl ChildKiller for BootstrapChild {
    fn kill(&mut self) -> std::io::Result<()> {
        self.bootstrap.kill()
    }
    fn clone_killer(&self) -> Box<dyn ChildKiller + Send + Sync> {
        self.bootstrap.clone_killer()
    }
}
impl Child for BootstrapChild {
    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        self.bootstrap.try_wait()
    }
    fn wait(&mut self) -> std::io::Result<portable_pty::ExitStatus> {
        self.bootstrap.wait()
    }
    fn process_id(&self) -> Option<u32> {
        let status = bootstrap_status(&self.receipt).ok()??;
        status["pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
    }
    fn as_raw_handle(&self) -> Option<std::os::windows::io::RawHandle> {
        None
    }
}

fn bootstrap_status(path: &Path) -> Result<Option<serde_json::Value>, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Yazi console receipt: {error}")),
    };
    let mut bytes = Vec::new();
    file.take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 4096 {
        return Err("Yazi console receipt exceeds its size limit".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("Yazi console receipt: {e}"))?;
    if value["id"] != 0 {
        return Err("Yazi console receipt has the wrong request ID".into());
    }
    Ok(Some(value))
}

fn bootstrap_executable() -> Result<PathBuf, String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let directory = executable
        .parent()
        .ok_or("Application directory unavailable")?;
    let mut candidates = vec![directory.join("photo-cli.exe")];
    if directory.file_name().is_some_and(|name| name == "deps")
        && let Some(debug) = directory.parent()
    {
        candidates.push(debug.join("photo-cli.exe"));
        #[cfg(test)]
        if let Some(target) = debug.parent() {
            candidates.push(target.join("release/photo-cli.exe"));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "photo-cli.exe is required beside Afio to initialize its private Yazi console".into()
        })
}

impl Tty for Pty {
    fn write(&self, bytes: &[u8]) -> egui_tty::Result<()> {
        let encoded = self
            .input_encoding
            .lock()
            .map_err(egui_tty::Error::msg)?
            .encode(bytes);
        self.input.try_send(encoded).map_err(egui_tty::Error::msg)
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

#[derive(Default)]
struct ConptyInput {
    paste: bool,
    tail: std::collections::VecDeque<u8>,
}
impl ConptyInput {
    fn observe(&mut self, byte: u8) {
        self.tail.push_back(byte);
        if self.tail.len() > 6 {
            self.tail.pop_front();
        }
        if self.tail.iter().copied().eq(b"\x1b[200~".iter().copied()) {
            self.paste = true;
        }
        if self.tail.iter().copied().eq(b"\x1b[201~".iter().copied()) {
            self.paste = false;
        }
    }
    fn encode(&mut self, bytes: &[u8]) -> Vec<u8> {
        // Crossterm on Windows receives ConPTY's Win32 key events. ConPTY does not
        // translate Kitty CSI-u Tab into that event, even when Yazi negotiated CSI-u
        // with the emulator. BackTab is the native equivalent, including in layers.
        // Preserve bracketed paste verbatim; terminal text is never a key binding.
        const KITTY_SHIFT_TAB: &[&[u8]] = &[b"\x1b[9;2u", b"\x1b[9;2:1u", b"\x1b[9;2:2u"];
        let mut output = Vec::with_capacity(bytes.len());
        let mut cursor = 0;
        while cursor < bytes.len() {
            let remaining = &bytes[cursor..];
            if !self.paste && remaining.starts_with(b"\x1b[9;2:3u") {
                for byte in b"\x1b[9;2:3u" {
                    self.observe(*byte);
                }
                cursor += b"\x1b[9;2:3u".len();
            } else if let Some(sequence) = KITTY_SHIFT_TAB
                .iter()
                .find(|sequence| !self.paste && remaining.starts_with(sequence))
            {
                output.extend_from_slice(b"\x1b[Z");
                for byte in *sequence {
                    self.observe(*byte);
                }
                cursor += sequence.len();
            } else {
                output.push(bytes[cursor]);
                self.observe(bytes[cursor]);
                cursor += 1;
            }
        }
        output
    }
}

pub struct YaziPane {
    terminal: Terminal,
    tty: Arc<Pty>,
    child: Option<Box<dyn Child + Send + Sync>>,
    ya_executable: PathBuf,
    protocol: protocol::Protocol,
    version: String,
    theme_mode: YaziThemeMode,
    io_errors: Arc<Mutex<Option<String>>>,
    session: PathBuf,
    last_folder: Option<PathBuf>,
    last_serial: u64,
    last_order_serial: u64,
    client_id: String,
    snapshot_serial: u64,
    pending_snapshot: Option<(u64, PathBuf)>,
    owned_selection: Mutex<std::collections::BTreeSet<PathBuf>>,
    navigation_target: Option<PathBuf>,
    navigation_generation: Arc<AtomicU64>,
    navigation_errors: Option<mpsc::Receiver<String>>,
    mouse_down: Option<egui::PointerButton>,
}

#[derive(Clone)]
pub struct NavigatorSnapshot {
    pub folder: PathBuf,
    pub hovered: Option<PathBuf>,
    pub marked: Vec<PathBuf>,
    pub ordered: Vec<PathBuf>,
    pub preferences: serde_json::Value,
    pub tabs: usize,
    pub metadata: serde_json::Value,
}

pub struct NavigatorOrder {
    pub folder: PathBuf,
    pub paths: Vec<PathBuf>,
}

pub struct PreparedYazi {
    yazi_executable: PathBuf,
    ya_executable: PathBuf,
    session: PathBuf,
    config: PathBuf,
    version: String,
    theme_mode: YaziThemeMode,
}

impl YaziPane {
    pub fn theme_mode(&self) -> YaziThemeMode {
        self.theme_mode
    }
    pub fn poll_background(&mut self) {
        self.terminal.poll();
        self.protocol.poll();
    }
    pub fn quicklink_keys(&mut self, key: &str) -> Result<(), String> {
        self.navigation_target = None;
        self.terminal
            .send(format!("g{key}").as_bytes())
            .map_err(|e| e.to_string())
    }
    pub fn version(&self) -> Result<String, String> {
        Ok(self.version.clone())
    }
    pub fn base_config_directory(&self) -> PathBuf {
        self.session.join("base-config")
    }
    pub fn emit(&self, action: &str, args: &[String]) -> Result<(), String> {
        let mut lua_args = serde_json::Map::new();
        let mut positional = 1;
        for arg in args {
            if let Some(named) = arg.strip_prefix("--") {
                let (key, value) = named.split_once('=').unwrap_or((named, "yes"));
                let value = match value {
                    "yes" | "true" => serde_json::Value::Bool(true),
                    "no" | "false" => serde_json::Value::Bool(false),
                    value => serde_json::Value::String(value.into()),
                };
                lua_args.insert(key.replace('-', "_"), value);
            } else {
                lua_args.insert(positional.to_string(), arg.clone().into());
                positional += 1;
            }
        }
        self.local_request(
            "emit",
            serde_json::json!({"action": action, "args": lua_args}),
        )
        .map(|_| ())
    }
    pub fn start(initial: &Path) -> Result<Self, String> {
        let profile = crate::yazi_profile::load().unwrap_or_default();
        Self::spawn_prepared(initial, Self::prepare(&profile)?)
    }
    pub fn prepare(profile: &photo_core::settings::YaziProfile) -> Result<PreparedYazi, String> {
        let (yazi_executable, ya_executable) = find_yazi_tools()?;
        let version = crate::yazi_transport::version(&yazi_executable)?;
        let session_number = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let session = std::env::temp_dir().join(format!(
            "photo-workshop-yazi-{}-{}-{}",
            std::process::id(),
            session_number,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos()
        ));
        let config = session.join("config");
        let user_config = std::env::var_os("YAZI_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("APPDATA").map(|root| PathBuf::from(root).join("yazi/config"))
            })
            .ok_or("APPDATA is missing")?;
        copy_directory(&user_config, &config).map_err(|error| error.to_string())?;
        patch_config(&config).map_err(|error| error.to_string())?;
        let base = session.join("base-config");
        fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        for name in ["yazi.toml", "keymap.toml", "theme.toml"] {
            if config.join(name).is_file() {
                fs::copy(config.join(name), base.join(name)).map_err(|e| e.to_string())?;
            }
        }
        let own_flavors = crate::yazi_profile::root()?.join("flavors");
        copy_directory(&own_flavors, &config.join("flavors")).map_err(|e| e.to_string())?;
        crate::yazi_profile::apply(&config, profile)?;
        let plugin = config.join("plugins/workshop-sync.yazi");
        fs::create_dir_all(&plugin).map_err(|error| error.to_string())?;
        fs::write(plugin.join("main.lua"), SYNC_PLUGIN).map_err(|error| error.to_string())?;
        Ok(PreparedYazi {
            yazi_executable,
            ya_executable,
            session,
            config,
            version,
            theme_mode: profile.theme_mode,
        })
    }
    pub fn spawn_prepared(initial: &Path, prepared: PreparedYazi) -> Result<Self, String> {
        let PreparedYazi {
            yazi_executable,
            ya_executable,
            session,
            config,
            version,
            theme_mode,
        } = prepared;
        let session_number = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
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
        let receipt = session.join("folder.txt.ack.0");
        let mut command = CommandBuilder::new(bootstrap_executable()?);
        command.arg("yazi-bootstrap");
        command.arg("--mode");
        command.arg(match theme_mode {
            YaziThemeMode::Dark => "dark",
            YaziThemeMode::Light => "light",
        });
        command.arg("--status");
        command.arg(&receipt);
        command.arg("--yazi");
        command.arg(yazi_executable);
        command.arg("--cwd");
        command.arg(super::friendly_path(initial));
        command.arg("--client-id");
        command.arg(&client_id);
        command.env("TERM", "xterm-256color");
        command.env("YAZI_CONFIG_HOME", &config);
        command.env("PHOTO_WORKSHOP_YAZI_BRIDGE", &bridge);
        let bootstrap = pty
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        let child: Box<dyn Child + Send + Sync> = Box::new(BootstrapChild { bootstrap, receipt });
        drop(pty.slave);

        let mut reader = pty
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pty
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;
        let (sender, output) = mpsc::sync_channel(128);
        let mut log = if cfg!(test) || std::env::var_os("AFIO_YAZI_TRACE_PTY").is_some() {
            fs::File::create(session.join("pty.log")).ok()
        } else {
            None
        };
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            while let Ok(count) = reader.read(&mut buffer) {
                if let Some(log) = &mut log {
                    let _ = log.write_all(&buffer[..count]);
                }
                if count == 0 {
                    break;
                }
                // Keep draining while background shutdown sends the targeted
                // quit. Closing ConPTY here would hard-reset a DDS server.
                let _ = sender.send(buffer[..count].to_vec());
            }
        });
        let (input, io_errors) = crate::yazi_transport::input_worker(writer);
        let tty = Arc::new(Pty {
            input,
            master: Mutex::new(pty.master),
            input_encoding: Mutex::new(ConptyInput::default()),
        });
        let mut terminal = Terminal::new(TtyStream {
            output,
            tty: tty.clone(),
        })
        .map_err(|error| error.to_string())?
        .with_label("Yazi navigator");
        // Yazi samples OSC 11 exactly once at startup. Set the emulator's reply
        // before the first poll can answer that query; app:theme cannot change it.
        terminal.set_color_scheme(match theme_mode {
            YaziThemeMode::Dark => ColorScheme::Dark,
            YaziThemeMode::Light => ColorScheme::Light,
        });
        Ok(Self {
            terminal,
            tty,
            child: Some(child),
            protocol: protocol::Protocol::new(&session),
            version,
            theme_mode,
            io_errors,
            ya_executable,
            session,
            last_folder: None,
            last_serial: 0,
            last_order_serial: 0,
            client_id,
            snapshot_serial: 0,
            pending_snapshot: None,
            owned_selection: Mutex::new(std::collections::BTreeSet::new()),
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
        self.cancel_snapshot();
        // Reports already queued for the old gallery must not reopen it while
        // the DDS request is in flight. Late writes are filtered by target below.
        if let Ok(entries) = fs::read_dir(&self.session) {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .strip_prefix("folder.txt.")
                    .is_some_and(|suffix| suffix.parse::<u64>().is_ok())
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        self.navigation_target = Some(target.clone());
        self.last_folder = Some(target.clone());
        self.navigation_generation.fetch_add(1, Ordering::Relaxed);
        self.local_request(
            "navigate",
            serde_json::json!({"paths": [cursor,
            super::friendly_path(&target), if is_folder { "folder" } else { "file" }]}),
        )
        .map(|_| ())
    }

    pub fn take_navigation_error(&mut self) -> Option<String> {
        match bootstrap_status(&self.session.join("folder.txt.ack.0")) {
            Ok(Some(status)) if status["ok"] != true => {
                return Some(format!(
                    "Yazi console initialization failed: {}",
                    status["error"].as_str().unwrap_or("unknown error")
                ));
            }
            Err(error) => return Some(error),
            _ => {}
        }
        self.protocol
            .take_error()
            .or_else(|| {
                self.io_errors
                    .lock()
                    .ok()
                    .and_then(|mut error| error.take())
            })
            .or_else(|| {
                self.navigation_errors
                    .as_ref()
                    .and_then(|errors| errors.try_recv().ok())
            })
    }

    pub fn request_snapshot(&mut self) -> Result<(), String> {
        if self.pending_snapshot.is_some() {
            return Ok(());
        }
        self.begin_snapshot().map(|_| ())
    }

    /// A new logical read always receives a fresh response path and request ID.
    pub fn begin_snapshot(&mut self) -> Result<u64, String> {
        self.cancel_snapshot();
        self.snapshot_serial += 1;
        let path = self
            .session
            .join(format!("snapshot.{}", self.snapshot_serial));
        let id = self.local_request(
            "snapshot",
            serde_json::json!({"target": path.to_string_lossy().replace('\\', "/")}),
        )?;
        self.pending_snapshot = Some((id, path));
        Ok(id)
    }
    pub fn cancel_snapshot(&mut self) {
        if let Some((id, path)) = self.pending_snapshot.take() {
            self.protocol.cancel(id);
            let _ = fs::remove_file(path);
        }
    }
    pub fn selection_applied(&self, id: u64) -> bool {
        self.protocol.acknowledged(id) && self.protocol.take_error().is_none()
    }
    pub fn pending_request_id(&self) -> Option<u64> {
        self.protocol.pending_request_id()
    }

    pub fn request_order(&self) -> Result<(), String> {
        self.local_request("order", serde_json::json!({}))
            .map(|_| ())
    }

    pub fn poll_snapshot(&mut self) -> Option<NavigatorSnapshot> {
        let (id, path) = self.pending_snapshot.as_ref()?;
        if !self.protocol.acknowledged(*id) {
            return None;
        }
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) => {
                return None;
            }
        };
        if !text.ends_with('\n') {
            return None;
        }
        let lines: Vec<_> = text.lines().collect();
        if lines.len() < 5 {
            return None;
        }
        let boundary = lines[5..].iter().position(|line| *line == "\0")? + 5;
        let snapshot = NavigatorSnapshot {
            folder: PathBuf::from(lines[0]),
            hovered: (!lines[1].is_empty()).then(|| PathBuf::from(lines[1])),
            marked: lines[5..boundary].iter().map(PathBuf::from).collect(),
            ordered: lines[boundary + 1..].iter().map(PathBuf::from).collect(),
            preferences: serde_json::from_str(lines[2]).ok()?,
            tabs: lines[3].parse().ok()?,
            metadata: serde_json::from_str(lines[4]).ok()?,
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

    #[cfg(test)]
    pub fn apply_selection(
        &self,
        cursor: &Path,
        marked: &[PathBuf],
        replace: bool,
    ) -> Result<(), String> {
        self.apply_gallery_selection(cursor, marked, replace, marked)
            .map(|_| ())
    }
    pub fn apply_gallery_selection(
        &self,
        cursor: &Path,
        marked: &[PathBuf],
        replace: bool,
        loaded: &[PathBuf],
    ) -> Result<u64, String> {
        let mut owned = self
            .owned_selection
            .lock()
            .map_err(|_| "Yazi selection scope lock failed")?;
        let mut scope = owned.clone();
        scope.extend(loaded.iter().cloned());
        scope.extend(marked.iter().cloned());
        let id = self.local_request(
            "apply",
            serde_json::json!({
                "cursor": super::friendly_path(cursor), "replace": replace,
                "marked": marked.iter().map(|p| super::friendly_path(p)).collect::<Vec<_>>(),
                "scope": scope.iter().map(|p| super::friendly_path(p)).collect::<Vec<_>>()
            }),
        )?;
        if replace {
            owned.extend(marked.iter().cloned());
        }
        Ok(id)
    }
    pub fn remember_imported_selection(&self, marked: &[PathBuf]) {
        if let Ok(mut owned) = self.owned_selection.lock() {
            owned.extend(marked.iter().cloned());
        }
    }
    pub fn restore_context_selection(
        &self,
        cursor: &Path,
        marked: &[PathBuf],
        previous: &[PathBuf],
    ) -> Result<(), String> {
        let scope: Vec<_> = previous
            .iter()
            .chain(marked)
            .map(|p| super::friendly_path(p))
            .collect();
        self.local_request("apply", serde_json::json!({"cursor": super::friendly_path(cursor), "replace":true,
            "marked":marked.iter().map(|p| super::friendly_path(p)).collect::<Vec<_>>(), "scope":scope})).map(|_| ())
    }
    pub fn inherit_selection_ownership(&self, previous: &Self) {
        if let (Ok(mut owned), Ok(old)) =
            (self.owned_selection.lock(), previous.owned_selection.lock())
        {
            *owned = old.clone();
        }
    }
    fn local_request(&self, operation: &str, request: serde_json::Value) -> Result<u64, String> {
        self.protocol.submit(operation, request)
    }

    pub fn config_directory(&self) -> PathBuf {
        self.session.join("config")
    }

    pub fn ui(
        &mut self,
        ui: &mut eframe::egui::Ui,
        nerd_font_loaded: bool,
        text_size: f32,
    ) -> bool {
        let mut style = TerminalStyle::from_visuals(&match self.theme_mode {
            YaziThemeMode::Dark => egui::Visuals::dark(),
            YaziThemeMode::Light => egui::Visuals::light(),
        });
        // Yazi uses bold on powerline caps and selected text. Their explicit
        // colors must remain exact instead of being mixed toward white.
        style.brighten_bold = false;
        if nerd_font_loaded {
            style.font = FontId::new(text_size, FontFamily::Name(NERD_FONT_FAMILY.into()));
        }
        style.font.size = text_size;
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
            let local_quit = self
                .local_request("emit", serde_json::json!({"action": "quit", "args": []}))
                .is_ok();
            let session = self.session.clone();
            let executable = self.ya_executable.clone();
            let client = self.client_id.clone();
            let tty = self.tty.clone();
            std::thread::spawn(move || {
                if !local_quit {
                    crate::yazi_transport::quit(&executable, &client);
                }
                let deadline = std::time::Instant::now() + std::time::Duration::from_millis(750);
                while std::time::Instant::now() < deadline {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                let _ = child.kill();
                let _ = child.wait();
                drop(tty);
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

pub(super) fn copy_directory(from: &Path, to: &Path) -> std::io::Result<()> {
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
            copy_private_config_file(&entry.path(), &destination)?;
        }
    }
    Ok(())
}

fn copy_private_config_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    // ya pkg marks installed flavor data read-only. These are independent
    // session copies: retain source protection, but make our own copies editable.
    // Avoid touching files already in use by the running Yazi when unchanged.
    if destination.is_file() && same_config_bytes(source, destination)? {
        return Ok(());
    }
    if destination.exists() {
        make_private_writable(destination)?;
    }
    fs::copy(source, destination).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "Не удалось скопировать конфигурацию {} → {}: {error}",
                source.display(),
                destination.display()
            ),
        )
    })?;
    make_private_writable(destination)
}

fn same_config_bytes(source: &Path, destination: &Path) -> std::io::Result<bool> {
    if fs::metadata(source)?.len() != fs::metadata(destination)?.len() {
        return Ok(false);
    }
    let mut left = std::io::BufReader::new(fs::File::open(source)?);
    let mut right = std::io::BufReader::new(fs::File::open(destination)?);
    let mut a = [0u8; 8192];
    let mut b = [0u8; 8192];
    loop {
        let n = left.read(&mut a)?;
        if n == 0 {
            return Ok(true);
        }
        right.read_exact(&mut b[..n])?;
        if a[..n] != b[..n] {
            return Ok(false);
        }
    }
}

// On Windows this clears only FILE_ATTRIBUTE_READONLY on our independent copy;
// it does not grant Unix write permissions or change any ACL.
#[cfg_attr(windows, allow(clippy::permissions_set_readonly_false))]
fn make_private_writable(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "Private config destination must not be a symbolic link",
        ));
    }
    #[cfg(windows)]
    if metadata.permissions().readonly() {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)?;
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
    if let Some(tables) = document
        .get("mgr")
        .and_then(|m| m.get("prepend_keymap"))
        .and_then(Item::as_array_of_tables)
    {
        document["mgr"]["prepend_keymap"] = value(Array::from_iter(
            tables.iter().cloned().map(Table::into_inline_table),
        ));
    }
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
    let mut backtab = InlineTable::new();
    backtab.insert("on", Value::from("<BackTab>"));
    backtab.insert("run", Value::from("spot"));
    backtab.insert("desc", Value::from("File information (Shift+Tab)"));
    entries.insert(0, backtab);
    // Crossterm normalizes legacy Shift+Tab to BackTab (without SHIFT), while
    // Kitty may report Tab+SHIFT. Both native forms toggle the same spot layer.
    ensure_table(&mut document, "spot")?;
    if let Some(tables) = document
        .get("spot")
        .and_then(|s| s.get("prepend_keymap"))
        .and_then(Item::as_array_of_tables)
    {
        document["spot"]["prepend_keymap"] = value(Array::from_iter(
            tables.iter().cloned().map(Table::into_inline_table),
        ));
    }
    let item = &mut document["spot"]["prepend_keymap"];
    if item.is_none() {
        *item = value(Array::new());
    }
    let entries = item
        .as_array_mut()
        .ok_or_else(|| invalid_config("spot.prepend_keymap"))?;
    for key in ["<BackTab>", "<S-Tab>"] {
        let mut close = InlineTable::new();
        close.insert("on", Value::from(key));
        close.insert("run", Value::from("close"));
        close.insert("desc", Value::from("Close file information (Shift+Tab)"));
        entries.insert(0, close);
    }
    Ok(document.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    static NATIVE_TEST_LOCK: Mutex<()> = Mutex::new(());
    #[test]
    fn conpty_shift_tab_translation_preserves_other_keys_and_pasted_text() {
        let conpty_input = |bytes: &[u8]| ConptyInput::default().encode(bytes);
        assert_eq!(conpty_input(b"\x1b[9;2u"), b"\x1b[Z");
        assert_eq!(
            conpty_input(b"x\x1b[9;2uy\x1b[9;2:2u\x1b[A"),
            b"x\x1b[Zy\x1b[Z\x1b[A"
        );
        let paste = b"\x1b[200~literal \x1b[9;2u text\x1b[201~\x1b[9;2u";
        assert_eq!(
            conpty_input(paste),
            b"\x1b[200~literal \x1b[9;2u text\x1b[201~\x1b[Z"
        );
        assert_eq!(
            conpty_input(b"\x1b]11;rgb:ffff/ffff/ffff\x07"),
            b"\x1b]11;rgb:ffff/ffff/ffff\x07"
        );
        assert_eq!(conpty_input(b"\x1b"), b"\x1b");
        assert_eq!(conpty_input(b"\x1b["), b"\x1b[");
        assert_eq!(conpty_input(b"\x1b[123;9~"), b"\x1b[123;9~");
        assert_eq!(conpty_input(b"\x1b[9;2:1u\x1b[9;2:3u"), b"\x1b[Z");
        assert_eq!(
            conpty_input(b"\x1b[200~\x1b[9;2:3u\x1b[201~"),
            b"\x1b[200~\x1b[9;2:3u\x1b[201~"
        );
        let mut encoding = ConptyInput::default();
        assert_eq!(encoding.encode(b"\x1b[20"), b"\x1b[20");
        assert_eq!(encoding.encode(b"0~\x1b[9;2u"), b"0~\x1b[9;2u");
        assert_eq!(encoding.encode(b"\x1b[201"), b"\x1b[201");
        assert_eq!(encoding.encode(b"~\x1b[9;2u"), b"~\x1b[Z");
    }

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
        for family in [FontFamily::Proportional, FontFamily::Monospace, font.family] {
            let font = FontId::new(14.0, family);
            for glyph in ['', '⌘', '←', '⌫', 'Ф', '照', '한'] {
                // epaint 0.36 has_glyph compares the face against the replacement
                // face, giving false negatives for valid glyphs in that same face.
                let (actual, missing) = ctx.fonts_mut(|fonts| {
                    let actual =
                        fonts.layout_no_wrap(glyph.to_string(), font.clone(), egui::Color32::WHITE);
                    let missing = fonts.layout_no_wrap(
                        "\u{10ffff}".into(),
                        font.clone(),
                        egui::Color32::WHITE,
                    );
                    (
                        actual.rows[0].glyphs[0].uv_rect,
                        missing.rows[0].glyphs[0].uv_rect,
                    )
                });
                assert!(
                    actual.min != missing.min || actual.max != missing.max,
                    "missing {glyph} in {font:?}"
                );
            }
        }
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
        assert_eq!(patched.matches("<S-Tab>").count(), 2);
        assert_eq!(patched.matches("<BackTab>").count(), 2);
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
                15
            );
            if original.contains("[input]") {
                assert!(document.get("input").is_some());
            }
        }
    }

    #[test]
    fn native_yazi_light_dark_modes_and_shift_tab_spot_are_real() {
        let _lock = NATIVE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture = std::env::temp_dir().join(format!("afio-mode-spot-{}", std::process::id()));
        fs::create_dir_all(&fixture).unwrap();
        let photo = fixture.join("synthetic.txt");
        fs::write(&photo, "Synthetic file information fixture").unwrap();
        fn read(pane: &mut YaziPane) -> NavigatorSnapshot {
            pane.request_snapshot().unwrap();
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                pane.poll_background();
                if let Some(error) = pane.take_navigation_error() {
                    panic!("{error}");
                }
                if let Some(snapshot) = pane.poll_snapshot() {
                    return snapshot;
                }
                assert!(
                    Instant::now() < deadline,
                    "native mode/spot snapshot timed out"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        fn layer(pane: &mut YaziPane, expected: &str) -> NavigatorSnapshot {
            let deadline = Instant::now() + Duration::from_secs(4);
            loop {
                let snapshot = read(pane);
                if snapshot.metadata["layer"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case(expected))
                {
                    return snapshot;
                }
                assert!(
                    Instant::now() < deadline,
                    "Shift+Tab did not reach {expected}: {:?}",
                    snapshot.metadata
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        let mut fingerprints = Vec::new();
        for mode in [
            YaziThemeMode::Dark,
            YaziThemeMode::Light,
            YaziThemeMode::Dark,
        ] {
            let prepared = YaziPane::prepare(&photo_core::settings::YaziProfile {
                theme_mode: mode,
                ..Default::default()
            })
            .unwrap();
            for (name, color, border) in [
                ("fixture-dark", "#123456", "D"),
                ("fixture-light", "#654321", "L"),
            ] {
                let directory = prepared.config.join(format!("flavors/{name}.yazi"));
                fs::create_dir_all(&directory).unwrap();
                fs::write(
                    directory.join("flavor.toml"),
                    format!("[mgr]\ncwd={{fg=\"{color}\"}}\nborder_symbol=\"{border}\"\n"),
                )
                .unwrap();
            }
            fs::write(
                prepared.config.join("theme.toml"),
                "[flavor]\ndark=\"fixture-dark\"\nlight=\"fixture-light\"\n",
            )
            .unwrap();
            let mut pane = YaziPane::spawn_prepared(&fixture, prepared).unwrap();
            let snapshot = read(&mut pane);
            if mode == YaziThemeMode::Light && snapshot.metadata["light"] != true {
                eprintln!(
                    "Light startup PTY {:?}",
                    String::from_utf8_lossy(
                        &fs::read(pane.session.join("pty.log")).unwrap_or_default()
                    )
                    .chars()
                    .take(1800)
                    .collect::<String>()
                );
            }
            assert_eq!(
                snapshot.metadata["light"],
                mode == YaziThemeMode::Light,
                "Yazi must detect the actual selected terminal mode"
            );
            let theme = snapshot.metadata["theme"].clone();
            assert_eq!(
                theme[3],
                if mode == YaziThemeMode::Light {
                    "L"
                } else {
                    "D"
                }
            );
            fingerprints.push(theme);
            pane.apply_gallery_selection(&photo, &[], false, &[])
                .unwrap();
            read(&mut pane);
            for bytes in [b"\x1b[Z".as_slice(), b"\x1b[9;2u".as_slice()] {
                eprintln!("Native Shift+Tab mode={mode:?} bytes={bytes:?}");
                pane.tty.write(bytes).unwrap();
                let opened = layer(&mut pane, "spot");
                assert_eq!(opened.hovered.as_ref(), Some(&photo));
                pane.tty.write(bytes).unwrap();
                let closed = layer(&mut pane, "mgr");
                assert_eq!(closed.hovered.as_ref(), Some(&photo));
            }
            let ctx = egui::Context::default();
            ctx.set_theme(egui::Theme::Dark);
            ctx.set_visuals(egui::Visuals::dark());
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    pane.ui(ui, false, 12.0);
                },
            );
            output.textures_delta.clear();
            assert!(
                ctx.theme() == egui::Theme::Dark,
                "Yazi mode must not change the host Afio theme"
            );
            drop(pane);
        }
        assert_ne!(fingerprints[0], fingerprints[1]);
        assert_eq!(fingerprints[0], fingerprints[2]);
        let _ = fs::remove_dir_all(fixture);
    }

    #[test]
    fn native_private_protocol_orders_scoped_marks_and_retires_snapshot_ids() {
        let _lock = NATIVE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fixture =
            std::env::temp_dir().join(format!("afio-private-contract-{}", std::process::id()));
        let a = fixture.join("album-a/a.jpg");
        let b = fixture.join("album-b/b.jpg");
        let c = fixture.join("external-c/c.jpg");
        for path in [&a, &b, &c] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"synthetic").unwrap();
        }
        fn read(pane: &mut YaziPane) -> NavigatorSnapshot {
            pane.request_snapshot().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                pane.poll_background();
                assert!(
                    pane.take_navigation_error().is_none(),
                    "private request failed"
                );
                if let Some(snapshot) = pane.poll_snapshot() {
                    return snapshot;
                }
                assert!(
                    Instant::now() < deadline,
                    "private snapshot timed out; PTY: {}",
                    String::from_utf8_lossy(
                        &fs::read(pane.session.join("pty.log")).unwrap_or_default()
                    )
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        struct Paused(*mut std::ffi::c_void);
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        #[link(name = "ntdll")]
        unsafe extern "system" {
            fn NtSuspendProcess(handle: *mut std::ffi::c_void) -> i32;
            fn NtResumeProcess(handle: *mut std::ffi::c_void) -> i32;
        }
        impl Paused {
            fn own_child(pane: &YaziPane) -> Self {
                let pid = pane.child.as_ref().unwrap().process_id().unwrap();
                let handle = unsafe { OpenProcess(0x0800, 0, pid) };
                assert!(!handle.is_null());
                assert_eq!(unsafe { NtSuspendProcess(handle) }, 0);
                Self(handle)
            }
        }
        impl Drop for Paused {
            fn drop(&mut self) {
                unsafe {
                    NtResumeProcess(self.0);
                    CloseHandle(self.0);
                }
            }
        }
        let mut pane = YaziPane::start(&fixture).unwrap();
        read(&mut pane);
        // Mark C directly in the owned Yazi fixture, outside Gallery ownership.
        pane.local_request("emit", serde_json::json!({"action":"toggle", "args":{"1":super::super::friendly_path(&c), "state":"on"}})).unwrap();
        assert!(read(&mut pane).marked.contains(&c));
        let paused = Paused::own_child(&pane);
        let loaded = [a.clone(), b.clone()];
        let first = pane
            .apply_gallery_selection(&a, &loaded, true, &loaded)
            .unwrap();
        let preserve = pane
            .apply_gallery_selection(&b, &[], false, &loaded)
            .unwrap();
        assert!(!pane.selection_applied(first));
        assert!(!pane.selection_applied(preserve));
        drop(paused);
        let applied = read(&mut pane);
        assert_eq!(applied.hovered.as_ref(), Some(&b));
        for path in [&a, &b, &c] {
            assert!(applied.marked.contains(path));
        }
        assert!(pane.selection_applied(first));
        // Replace -> replace remains ordered while the consumer is paused.
        let paused = Paused::own_child(&pane);
        pane.apply_gallery_selection(&b, std::slice::from_ref(&b), true, &loaded)
            .unwrap();
        pane.apply_gallery_selection(&a, std::slice::from_ref(&a), true, &loaded)
            .unwrap();
        drop(paused);
        let reduced = read(&mut pane);
        assert!(reduced.marked.contains(&a) && reduced.marked.contains(&c));
        assert!(!reduced.marked.contains(&b));
        pane.apply_gallery_selection(&a, &[], true, &loaded)
            .unwrap();
        assert_eq!(read(&mut pane).marked, vec![c.clone()]);
        // A retired request may leave a late response. The next logical read
        // has a different ID/path and must observe the subsequent cursor B.
        let paused = Paused::own_child(&pane);
        let old = pane.begin_snapshot().unwrap();
        let old_path = pane.pending_snapshot.as_ref().unwrap().1.clone();
        pane.cancel_snapshot();
        fs::write(
            &old_path,
            format!(
                "{}\n{}\n{{}}\n1\n{{}}\n\0\n",
                fixture.display(),
                a.display()
            ),
        )
        .unwrap();
        pane.apply_gallery_selection(&b, &[], false, &loaded)
            .unwrap();
        let fresh = pane.begin_snapshot().unwrap();
        assert_ne!(old, fresh);
        assert!(pane.poll_snapshot().is_none());
        drop(paused);
        let fresh_snapshot = read(&mut pane);
        assert_eq!(fresh_snapshot.hovered.as_ref(), Some(&b));
        assert!(
            !old_path.exists(),
            "late cancelled response was not collected"
        );
        // Positional and named preferences use this stream, independent of ya.
        pane.emit(
            "sort",
            &[
                "natural".into(),
                "--reverse=yes".into(),
                "--dir-first=no".into(),
            ],
        )
        .unwrap();
        pane.emit("hidden", &["show".into()]).unwrap();
        pane.emit("linemode", &["size".into()]).unwrap();
        let pref = read(&mut pane).preferences;
        assert_eq!(pref["sort_by"], "natural");
        assert_eq!(pref["sort_reverse"], true);
        assert_eq!(pref["sort_dir_first"], false);
        assert_eq!(pref["show_hidden"], true);
        assert_eq!(pref["linemode"], "size");
        // Protocol polling must never become a user task itself. Real plugins
        // still appear in the untouched native task summary used by Apply.
        for _ in 0..12 {
            assert_eq!(read(&mut pane).metadata["tasks"], 0);
            std::thread::sleep(Duration::from_millis(30));
        }
        let real_plugin = pane
            .config_directory()
            .join("plugins/fixture-user-task.yazi");
        fs::create_dir_all(&real_plugin).unwrap();
        fs::write(
            real_plugin.join("main.lua"),
            "return { entry=function() ya.sleep(2) end }",
        )
        .unwrap();
        pane.emit("plugin", &["fixture-user-task".into()]).unwrap();
        let deadline = Instant::now() + Duration::from_millis(1500);
        loop {
            if read(&mut pane).metadata["tasks"].as_u64().unwrap_or(0) > 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "real user plugin task disappeared from the safety snapshot"
            );
            std::thread::sleep(Duration::from_millis(30));
        }
        let mut other = YaziPane::start(&fixture).unwrap();
        assert_ne!(pane.session, other.session);
        assert!(read(&mut other).marked.is_empty());
        assert_eq!(read(&mut pane).marked, vec![c]);
        drop(other);
        drop(pane);
        let _ = fs::remove_dir_all(fixture);
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
        let child_pid = pane.child.as_ref().unwrap().process_id();
        assert!(
            child_pid.is_some(),
            "bootstrap must report the real Yazi PID before readiness"
        );
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
