//! Windows file clipboard plus full-size pixels for a single supported image.
use image::ImageDecoder;
use std::path::{Path, PathBuf};

fn shell_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix("\\\\?\\UNC\\") {
        PathBuf::from(format!("\\\\{unc}"))
    } else if let Some(path) = text.strip_prefix("\\\\?\\") {
        PathBuf::from(path)
    } else {
        path.to_path_buf()
    }
}

fn file_drop(paths: &[PathBuf]) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    let mut bytes = vec![0u8; 20];
    bytes[..4].copy_from_slice(&20u32.to_le_bytes());
    bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
    for path in paths {
        for unit in shell_path(path).as_os_str().encode_wide().chain(Some(0)) {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes
}

fn image_dib(path: &Path) -> Result<Vec<u8>, String> {
    let mut decoder = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .into_decoder()
        .map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    if width as u64 * height as u64 > 32_000_000 {
        return Err("Image exceeds clipboard pixel budget; files copied instead".into());
    }
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    let rgba = image.into_rgba8();
    Ok(dib(rgba.width(), rgba.height(), rgba.as_raw()))
}

fn dib(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0u8; 124]; // BITMAPV5HEADER, top-down BGRA, explicit alpha.
    for (offset, value) in [
        (0, 124),
        (4, width),
        (8, (-(height as i32)) as u32),
        (16, 3),
        (20, rgba.len() as u32),
        (40, 0x00ff0000),
        (44, 0x0000ff00),
        (48, 0x000000ff),
        (52, 0xff000000),
        (56, 0x73524742),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[12..14].copy_from_slice(&1u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&32u16.to_le_bytes());
    bytes.reserve(rgba.len());
    for pixel in rgba.chunks_exact(4) {
        bytes.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
    }
    bytes
}

pub fn copy(paths: &[PathBuf]) -> Result<Vec<String>, String> {
    if paths.is_empty() {
        return Err("No files selected".into());
    }
    for path in paths {
        if !path.is_file() {
            return Err(format!("File unavailable: {}", path.display()));
        }
    }
    let mut warnings = Vec::new();
    let pixels = if paths.len() == 1 && !photo_core::is_video_path(&paths[0]) {
        match image_dib(&paths[0]) {
            Ok(pixels) => Some(pixels),
            Err(e) => {
                warnings.push(format!("Files copied; pixels unavailable: {e}"));
                None
            }
        }
    } else {
        None
    };
    let files = file_drop(paths);
    let mut clipboard = None;
    for _ in 0..20 {
        if let Ok(open) = clipboard_win::Clipboard::new() {
            clipboard = Some(open);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _clipboard = clipboard.ok_or("Clipboard is busy; try again")?;
    clipboard_win::raw::set(15, &files).map_err(|e| e.to_string())?;
    if let Some(pixels) = pixels
        && let Err(error) = clipboard_win::raw::set_without_clear(17, &pixels)
    {
        warnings.push(format!("Files copied; image clipboard failed: {error}"));
    }
    if let Some(format) = clipboard_win::raw::register_format("Preferred DropEffect") {
        let _ = clipboard_win::raw::set_without_clear(format.get(), &1u32.to_le_bytes());
    }
    Ok(warnings)
}

/// Pixel-only ownership; never publishes a temporary file reference.
pub fn copy_pixels(path: &Path) -> Result<(), String> {
    let pixels = image_dib(path)?;
    let mut clipboard = None;
    for _ in 0..20 {
        if let Ok(open) = clipboard_win::Clipboard::new() {
            clipboard = Some(open);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _clipboard = clipboard.ok_or("Clipboard is busy; try again")?;
    clipboard_win::raw::set(17, &pixels).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_shell_reads_every_file_in_the_multicopy_payload() {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GlobalAlloc(flags: u32, size: usize) -> *mut c_void;
            fn GlobalLock(memory: *mut c_void) -> *mut c_void;
            fn GlobalUnlock(memory: *mut c_void) -> i32;
            fn GlobalFree(memory: *mut c_void) -> *mut c_void;
        }
        #[link(name = "shell32")]
        unsafe extern "system" {
            fn DragQueryFileW(drop: *mut c_void, index: u32, name: *mut u16, length: u32) -> u32;
        }
        struct Memory(*mut c_void);
        impl Drop for Memory {
            fn drop(&mut self) {
                unsafe {
                    GlobalFree(self.0);
                }
            }
        }
        let names = [
            r"C:\Фото\кадр 1.png",
            r"C:\Фото\кадр 2.jpg",
            r"D:\Снимки\третий.webp",
        ];
        let payload = file_drop(&names.iter().map(PathBuf::from).collect::<Vec<_>>());
        // Exercise the native parser used by Windows file consumers, without
        // opening/clearing the user's clipboard or creating any windows.
        unsafe {
            let memory = Memory(GlobalAlloc(0x42, payload.len()));
            assert!(!memory.0.is_null());
            let pointer = GlobalLock(memory.0);
            assert!(!pointer.is_null());
            std::ptr::copy_nonoverlapping(payload.as_ptr(), pointer.cast(), payload.len());
            GlobalUnlock(memory.0);
            assert_eq!(
                DragQueryFileW(memory.0, u32::MAX, std::ptr::null_mut(), 0),
                3
            );
            for (index, expected) in names.iter().enumerate() {
                let length = DragQueryFileW(memory.0, index as u32, std::ptr::null_mut(), 0);
                let mut name = vec![0u16; length as usize + 1];
                assert_eq!(
                    DragQueryFileW(memory.0, index as u32, name.as_mut_ptr(), name.len() as u32),
                    length
                );
                assert_eq!(
                    String::from_utf16(&name[..length as usize]).unwrap(),
                    *expected
                );
            }
        }
    }

    #[test]
    fn clipboard_payloads_preserve_alpha_orientation_unicode_and_file_boundaries() {
        let pixels = dib(1, 2, &[255, 0, 0, 128, 0, 0, 255, 255]);
        assert_eq!(i32::from_le_bytes(pixels[8..12].try_into().unwrap()), -2);
        assert_eq!(&pixels[124..], &[0, 0, 255, 128, 255, 0, 0, 255]);
        let paths = file_drop(&[
            PathBuf::from(r"\\?\C:\кадр 1.png"),
            PathBuf::from(r"D:\two.mp4"),
        ]);
        let text = String::from_utf16(
            &paths[20..]
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(text, "C:\\кадр 1.png\0D:\\two.mp4\0\0");
    }
}
