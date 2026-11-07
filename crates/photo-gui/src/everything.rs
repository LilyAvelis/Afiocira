//! Read-only IPC searches through the pinned, official voidtools ES helper.
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub use photo_core::api::SearchResult;

#[derive(Clone, Debug)]
pub struct SearchResults {
    pub items: Vec<SearchResult>,
    pub total: u32,
}

/// Search Everything's existing index. Errors are distinct from an empty result set.
pub fn search(query: &str) -> Result<SearchResults, String> {
    if query.contains('\0') || query.encode_utf16().count() > 16_000 {
        return Err("Everything query is invalid or too long".into());
    }
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let helper = executable.with_file_name("es.exe");
    if !helper.is_file() {
        return Err("Компонент поиска Everything отсутствует в этой сборке. Просмотр файлов остаётся доступен.".into());
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let output = run(
        &helper,
        &["-json", "-n", "100", "-attributes"],
        query,
        deadline,
    )?;
    // ES emits no bytes (rather than []) for a successful empty JSON search.
    let values: Vec<serde_json::Value> = if output.iter().all(u8::is_ascii_whitespace) {
        Vec::new()
    } else {
        serde_json::from_slice(&output).map_err(|e| format!("Invalid Everything results: {e}"))?
    };
    if values.len() > 100 {
        return Err("Everything exceeded the result limit".into());
    }
    let items = values
        .into_iter()
        .map(|value| {
            let path = value
                .get("filename")
                .and_then(|v| v.as_str())
                .ok_or("Everything result has no filename")?;
            let attributes = value
                .get("attributes")
                .and_then(|v| v.as_u64())
                .ok_or("Everything result has no attributes")?;
            Ok(SearchResult {
                path: PathBuf::from(path),
                is_folder: attributes & 16 != 0,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let count = run(
        &helper,
        &["-get-result-count", "-no-digit-grouping"],
        query,
        deadline,
    )?;
    let total = std::str::from_utf8(&count)
        .map_err(|e| e.to_string())?
        .trim()
        .parse::<u32>()
        .map_err(|e| format!("Invalid Everything result count: {e}"))?;
    // The index can change between these two read-only IPC requests.
    Ok(SearchResults {
        total: total.max(items.len() as u32),
        items,
    })
}

fn run(helper: &Path, options: &[&str], query: &str, deadline: Instant) -> Result<Vec<u8>, String> {
    if Instant::now() >= deadline {
        return Err("Everything не ответил вовремя. Повторите запрос.".into());
    }
    let mut command = Command::new(helper);
    command
        .args(["-argv", "-timeout", "1500", "-no-pause", "-no-highlight"])
        // A hidden process has no console code page; ES otherwise falls back
        // to OEM encoding, which loses Unicode paths. Force UTF-8 explicitly.
        .args(["-code-page", "65001"])
        .args(options)
        .arg("--")
        .arg(query)
        // ES reads defaults from APPDATA. Give this child an isolated, unused
        // settings namespace so user ES defaults cannot alter our query/output.
        .env(
            "APPDATA",
            helper.parent().unwrap().join("afio-everything-config"),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Everything helper unavailable: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    // Drain concurrently so even a maximum-length Unicode result cannot fill
    // the pipe and deadlock the child. Bound memory independently of ES.
    const MAX_OUTPUT: u64 = 32 * 1024 * 1024;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(MAX_OUTPUT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => break Err("Everything не ответил вовремя. Повторите запрос.".to_owned()),
            Err(e) => break Err(format!("Everything helper failed: {e}")),
        }
    };
    if status.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let bytes = reader
        .join()
        .map_err(|_| "Everything output reader failed")?
        .map_err(|e| format!("Cannot read Everything results: {e}"))?;
    let status = status?;
    if !status.success() {
        return Err(match status.code() {
            Some(8) => "Everything не запущен. Запустите Everything и повторите запрос.".into(),
            Some(7) => {
                "Everything недоступен. Нужна запущенная полная версия с поддержкой IPC.".into()
            }
            code => format!("Everything search failed (exit {code:?})"),
        });
    }
    if bytes.len() as u64 > MAX_OUTPUT {
        return Err("Everything output exceeded the size limit".into());
    }
    Ok(bytes
        .strip_prefix(&[0xef, 0xbb, 0xbf])
        .unwrap_or(&bytes)
        .to_vec())
}
