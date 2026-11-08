//! Bounded external tool probes, process shutdown, and terminal input.
use std::os::windows::process::CommandExt;
#[cfg(test)]
use std::path::PathBuf;
use std::{
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

pub(super) fn version(executable: &Path) -> Result<String, String> {
    let bytes = run(
        executable,
        &["--version".into()],
        Duration::from_secs(2),
        true,
    )?;
    Ok(String::from_utf8_lossy(&bytes).trim().into())
}

pub(super) fn quit(executable: &Path, client: &str) {
    let _ = run(
        executable,
        &[
            "emit-to".into(),
            client.into(),
            "quit".into(),
            "--no-cwd-file".into(),
        ],
        Duration::from_secs(2),
        false,
    );
}
fn run(
    executable: &Path,
    args: &[String],
    timeout: Duration,
    output: bool,
) -> Result<Vec<u8>, String> {
    let mut child = Command::new(executable)
        .creation_flags(0x0800_0000)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(if output {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .spawn()
        .map_err(|e| e.to_string())?;
    let reader = child.stdout.take().map(|stdout| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stdout.take(4096).read_to_end(&mut bytes);
            bytes
        })
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err(format!("Yazi helper failed: {status}"));
                }
                return Ok(reader
                    .map(|r| r.join().unwrap_or_default())
                    .unwrap_or_default());
            }
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.to_string());
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Yazi helper timed out; the viewer remains available".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn input_worker(
    mut writer: Box<dyn std::io::Write + Send>,
) -> (
    mpsc::SyncSender<Vec<u8>>,
    std::sync::Arc<std::sync::Mutex<Option<String>>>,
) {
    let (sender, receiver) = mpsc::sync_channel::<Vec<u8>>(64);
    let error = std::sync::Arc::new(std::sync::Mutex::new(None));
    let report = error.clone();
    std::thread::spawn(move || {
        while let Ok(bytes) = receiver.recv() {
            if let Err(error) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                if let Ok(mut slot) = report.lock() {
                    *slot = Some(format!("Yazi input failed: {error}"));
                }
                break;
            }
        }
    });
    (sender, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stalled_pty_writer_cannot_block_the_ui_sender() {
        use std::sync::{Arc, Condvar, Mutex};
        struct Stalled(Arc<(Mutex<bool>, Condvar)>);
        impl std::io::Write for Stalled {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                let (lock, wake) = &*self.0;
                let mut ready = lock.lock().unwrap();
                while !*ready {
                    ready = wake.wait(ready).unwrap();
                }
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        let (sender, _) = input_worker(Box::new(Stalled(signal.clone())));
        let start = Instant::now();
        sender.try_send(b"terminal reply".to_vec()).unwrap();
        assert!(start.elapsed() < Duration::from_millis(100));
        let (lock, wake) = &*signal;
        *lock.lock().unwrap() = true;
        wake.notify_all();
    }
    #[test]
    fn hung_helper_is_bounded() {
        let powershell = PathBuf::from(std::env::var_os("WINDIR").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let args = vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            "Start-Sleep -Seconds 20".into(),
        ];
        let start = Instant::now();
        assert!(
            run(&powershell, &args, Duration::from_millis(100), false)
                .unwrap_err()
                .contains("timed out")
        );
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
