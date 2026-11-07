#![cfg_attr(windows, windows_subsystem = "windows")]

fn launch() -> Result<(), Box<dyn std::error::Error>> {
    let launcher = std::env::current_exe()?;
    let root = launcher.parent().ok_or("Missing launcher directory")?;
    let relative = std::fs::read_to_string(root.join("current-version.txt"))?;
    let executable = root.join(relative.trim()).canonicalize()?;
    if !executable.starts_with(root.canonicalize()?) {
        return Err("Working version must stay inside the application directory".into());
    }
    let mut command = std::process::Command::new(&executable);
    command
        .args(std::env::args_os().skip(1))
        .current_dir(executable.parent().unwrap());
    if let Ok(ffmpeg) = std::fs::read_to_string(root.join("ffmpeg-path.txt"))
        && std::path::Path::new(ffmpeg.trim()).is_file()
    {
        command.env("AFIO_FFMPEG", ffmpeg.trim());
    }
    command.spawn()?;
    Ok(())
}

fn main() {
    if let Err(error) = launch() {
        let message = format!("Cannot start the working version of Afiocira:\n{error}");
        #[cfg(windows)]
        {
            #[link(name = "user32")]
            unsafe extern "system" {
                fn MessageBoxW(
                    window: *mut std::ffi::c_void,
                    text: *const u16,
                    title: *const u16,
                    kind: u32,
                ) -> i32;
            }
            let text: Vec<_> = message.encode_utf16().chain(Some(0)).collect();
            let title: Vec<_> = "Afiocira".encode_utf16().chain(Some(0)).collect();
            // Both null-terminated UTF-16 buffers remain alive during the call.
            unsafe {
                MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), 0x10);
            }
        }
        #[cfg(not(windows))]
        eprintln!("{message}");
        std::process::exit(1);
    }
}
