//! Initialize only our attached pseudoconsole before Yazi queries its defaults.
use std::{
    ffi::OsString,
    io,
    path::PathBuf,
    process::{Command, ExitCode},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{Console::*, JobObjects::*},
};

pub fn run(args: &[OsString]) -> ExitCode {
    let option = |key: &str| {
        args.chunks_exact(2)
            .find(|pair| pair[0] == key)
            .map(|pair| pair[1].clone())
    };
    let Some(status) = option("--status").map(PathBuf::from) else {
        return ExitCode::FAILURE;
    };
    let result = (|| -> io::Result<i32> {
        if args.len() != 10
            || args.chunks_exact(2).any(|p| {
                !["--mode", "--status", "--yazi", "--cwd", "--client-id"]
                    .iter()
                    .any(|key| p[0] == *key)
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid private bootstrap arguments",
            ));
        }
        let required = |key| {
            option(key).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "Missing bootstrap option")
            })
        };
        let mode = required("--mode")?;
        if mode != "light" && mode != "dark" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid theme mode",
            ));
        }
        let console = initialize_console(mode == "light")?;
        let job = ChildJob::new()?;
        // Keep the self-assigned job alive until OS process teardown, including
        // error paths, so a failure receipt can still be written before exiting.
        std::mem::forget(job);
        let initial = PathBuf::from(required("--cwd")?);
        let directory = if initial.is_file() {
            initial.parent().unwrap_or(&initial)
        } else {
            &initial
        };
        let mut child = Command::new(required("--yazi")?)
            .current_dir(directory)
            .arg(&initial)
            .args([OsString::from("--client-id"), required("--client-id")?])
            .spawn()?;
        // The child inherits the helper's console, environment and job membership.
        publish(
            &status,
            &serde_json::json!({"id":0,"ok":true,"pid":child.id(),"console":console}),
        )?;
        let code = child.wait()?.code().unwrap_or(1);
        Ok(code)
    })();
    match result {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            let _ = publish(
                &status,
                &serde_json::json!({"id":0,"ok":false,"error":error.to_string()}),
            );
            ExitCode::FAILURE
        }
    }
}

fn publish(path: &std::path::Path, value: &serde_json::Value) -> io::Result<()> {
    let temporary = path.with_extension(format!("bootstrap-{}", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec(value)?)?;
    std::fs::rename(&temporary, path)
}

fn initialize_console(light: bool) -> io::Result<serde_json::Value> {
    unsafe {
        let output = GetStdHandle(STD_OUTPUT_HANDLE);
        if output.is_null() || output == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let mut mode = 0;
        if GetConsoleMode(output, &mut mode) == 0
            || SetConsoleMode(output, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) == 0
        {
            return Err(io::Error::last_os_error());
        }
        // ConPTY's Win32 query responder reads the screen-buffer palette and
        // default attributes rather than the VT passthrough stream.
        let mut info: CONSOLE_SCREEN_BUFFER_INFOEX = std::mem::zeroed();
        info.cbSize = std::mem::size_of_val(&info) as u32;
        if GetConsoleScreenBufferInfoEx(output, &mut info) == 0 {
            return Err(io::Error::last_os_error());
        }
        info.ColorTable[0] = if light { 0x00fa_fafa } else { 0x0018_1818 };
        info.ColorTable[7] = if light { 0x0020_2020 } else { 0x00e0_e0e0 };
        info.wAttributes = 7; // foreground slot 7, background slot 0
        // The setter treats the window rectangle's right/bottom as exclusive.
        info.srWindow.Right = info.srWindow.Right.saturating_add(1);
        info.srWindow.Bottom = info.srWindow.Bottom.saturating_add(1);
        if SetConsoleScreenBufferInfoEx(output, &info) == 0
            || SetConsoleTextAttribute(output, 7) == 0
        {
            return Err(io::Error::last_os_error());
        }
        // OSC 10/11 change default color aliases, which ConPTY uses to answer
        // the child's OSC 11 query. No registry or other console is modified.
        let colors = if light {
            "\x1b]10;rgb:2020/2020/2020\x1b\\\x1b]11;rgb:fafa/fafa/fafa\x1b\\"
        } else {
            "\x1b]10;rgb:e0e0/e0e0/e0e0\x1b\\\x1b]11;rgb:1818/1818/1818\x1b\\"
        };
        let wide: Vec<u16> = colors.encode_utf16().collect();
        let mut written = 0;
        if WriteConsoleW(
            output,
            wide.as_ptr().cast(),
            wide.len() as u32,
            &mut written,
            std::ptr::null(),
        ) == 0
            || written != wide.len() as u32
        {
            return Err(io::Error::last_os_error());
        }
        let mut readback: CONSOLE_SCREEN_BUFFER_INFOEX = std::mem::zeroed();
        readback.cbSize = std::mem::size_of_val(&readback) as u32;
        if GetConsoleScreenBufferInfoEx(output, &mut readback) == 0 {
            return Err(io::Error::last_os_error());
        }
        // Older ConPTY swallows OSC 11 queries without returning their answer.
        // Seed that one response before spawning the pinned Yazi: its first
        // input read is Emulator::detect, which consumes OSC 11 before the
        // native DA1 reply. Do not seed DA1 or inject anything after startup.
        let background = readback.ColorTable[((readback.wAttributes >> 4) & 15) as usize];
        let report = format!(
            "\x1b]11;rgb:{:02x}{:02x}/{:02x}{:02x}/{:02x}{:02x}\x1b\\",
            background & 255,
            background & 255,
            (background >> 8) & 255,
            (background >> 8) & 255,
            (background >> 16) & 255,
            (background >> 16) & 255
        );
        let input = GetStdHandle(STD_INPUT_HANDLE);
        let events: Vec<INPUT_RECORD> = report
            .encode_utf16()
            .map(|character| {
                let mut record: INPUT_RECORD = std::mem::zeroed();
                record.EventType = KEY_EVENT as u16;
                record.Event.KeyEvent = KEY_EVENT_RECORD {
                    bKeyDown: 1,
                    wRepeatCount: 1,
                    wVirtualKeyCode: 0,
                    wVirtualScanCode: 0,
                    uChar: KEY_EVENT_RECORD_0 {
                        UnicodeChar: character,
                    },
                    dwControlKeyState: 0,
                };
                record
            })
            .collect();
        let mut count = 0;
        if WriteConsoleInputW(input, events.as_ptr(), events.len() as u32, &mut count) == 0
            || count != events.len() as u32
        {
            return Err(io::Error::last_os_error());
        }
        Ok(serde_json::json!({"attributes":readback.wAttributes,
            "background":readback.ColorTable[0],"foreground":readback.ColorTable[7]}))
    }
}

struct ChildJob(HANDLE);
impl ChildJob {
    fn new() -> io::Result<Self> {
        unsafe {
            let job = Self(CreateJobObjectW(std::ptr::null(), std::ptr::null()));
            if job.0.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            // Assign before spawning, so there is no orphan race if the helper
            // is terminated between child creation and the readiness receipt.
            if AssignProcessToJobObject(
                job.0,
                windows_sys::Win32::System::Threading::GetCurrentProcess(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(job)
        }
    }
}
impl Drop for ChildJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
