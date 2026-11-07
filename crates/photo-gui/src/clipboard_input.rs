//! Read-only clipboard snapshots. Image decoding happens after releasing the clipboard.
use std::{io::Cursor, path::PathBuf};

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

const MAX_PIXELS: u64 = 32_000_000;
const MAX_BYTES: u64 = 256 * 1024 * 1024;

pub(super) enum Payload {
    Files(Vec<PathBuf>),
    Png(Vec<u8>),
}

enum Snapshot {
    Files(Vec<PathBuf>),
    Image(Vec<u8>, ImageFormat),
    Dib(Vec<u8>),
}

pub(super) fn read() -> Result<Payload, String> {
    normalize(snapshot()?)
}

fn snapshot() -> Result<Snapshot, String> {
    use clipboard_win::{Clipboard, formats, raw};

    // One immediate attempt: do not stall the UI while another application owns it.
    let _clipboard =
        Clipboard::new().map_err(|_| "Clipboard is busy. Try pasting again.".to_owned())?;
    if raw::is_format_avail(formats::CF_HDROP) {
        // This API uses DragQueryFileW and OsString::from_wide, avoiding lossy Unicode conversion.
        let mut paths = Vec::<PathBuf>::new();
        raw::get_file_list_path(&mut paths)
            .map_err(|e| format!("Could not read clipboard files: {e}"))?;
        return Ok(Snapshot::Files(paths));
    }
    if let Some(png) = raw::register_format("PNG")
        && raw::is_format_avail(png.get())
    {
        return Ok(Snapshot::Image(copy_raw(png.get())?, ImageFormat::Png));
    }
    for format in [17, 8] {
        // CF_DIBV5, CF_DIB
        if raw::is_format_avail(format) {
            return Ok(Snapshot::Dib(copy_raw(format)?));
        }
    }
    // Windows synthesizes CF_BITMAP from ordinary DIB clipboard formats. clipboard-win
    // copies it to a BMP file; this fallback cannot recover alpha lost by GDI conversion.
    if raw::is_format_avail(formats::CF_BITMAP) {
        preflight_bitmap()?;
        let mut bytes = Vec::new();
        raw::get_bitmap(&mut bytes).map_err(|e| format!("Could not read clipboard bitmap: {e}"))?;
        return Ok(Snapshot::Image(bytes, ImageFormat::Bmp));
    }
    Err("Clipboard contains no supported image or copied files.".to_owned())
}

fn copy_raw(format: u32) -> Result<Vec<u8>, String> {
    use clipboard_win::raw;
    let size = raw::size(format).ok_or("Clipboard image is empty or inaccessible.")?;
    if size.get() as u64 > MAX_BYTES {
        return Err("Clipboard image exceeds the 256 MiB limit.".to_owned());
    }
    let mut bytes = Vec::new();
    raw::get_vec(format, &mut bytes).map_err(|e| format!("Could not read clipboard image: {e}"))?;
    Ok(bytes)
}

fn preflight_bitmap() -> Result<(), String> {
    #[repr(C)]
    struct Bitmap {
        kind: i32,
        width: i32,
        height: i32,
        width_bytes: i32,
        planes: u16,
        bits_pixel: u16,
        bits: *mut std::ffi::c_void,
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn GetObjectW(
            object: *mut std::ffi::c_void,
            size: i32,
            output: *mut std::ffi::c_void,
        ) -> i32;
    }
    let handle = clipboard_win::raw::get_clipboard_data(clipboard_win::formats::CF_BITMAP)
        .map_err(|e| format!("Could not inspect clipboard bitmap: {e}"))?;
    let mut bitmap = std::mem::MaybeUninit::<Bitmap>::uninit();
    // Clipboard remains open; GetObjectW copies the fixed-size BITMAP description.
    let count = unsafe {
        GetObjectW(
            handle.as_ptr(),
            std::mem::size_of::<Bitmap>() as i32,
            bitmap.as_mut_ptr().cast(),
        )
    };
    if count != std::mem::size_of::<Bitmap>() as i32 {
        return Err("Could not inspect clipboard bitmap dimensions.".to_owned());
    }
    // The successful exact-size GetObjectW call initialized the whole structure.
    let bitmap = unsafe { bitmap.assume_init() };
    if bitmap.width <= 0 || bitmap.height <= 0 {
        return Err("Invalid clipboard bitmap dimensions.".to_owned());
    }
    check_dimensions(bitmap.width as u32, bitmap.height as u32)
}

fn dib_to_bmp(dib: Vec<u8>) -> Result<Vec<u8>, String> {
    let invalid = || "Clipboard DIB header is malformed or unsupported.".to_owned();
    if dib.len() < 40 || dib.len() as u64 > MAX_BYTES {
        return Err(invalid());
    }
    let u32_at = |offset| u32::from_le_bytes(dib[offset..offset + 4].try_into().unwrap());
    let header = u32_at(0) as usize;
    if !matches!(header, 40 | 108 | 124) || dib.len() < header {
        return Err(invalid());
    }
    let width = u32_at(4) as i32;
    let height = u32_at(8) as i32;
    let planes = u16::from_le_bytes(dib[12..14].try_into().unwrap());
    let bits = u16::from_le_bytes(dib[14..16].try_into().unwrap());
    let compression = u32_at(16);
    if width <= 0
        || height == i32::MIN
        || planes != 1
        || !matches!(bits, 1 | 4 | 8 | 16 | 24 | 32)
        || !matches!(compression, 0 | 3 | 6)
        || (compression != 0 && !matches!(bits, 16 | 32))
    {
        return Err(invalid());
    }
    check_dimensions(width as u32, height.unsigned_abs())?;
    let colors = u32_at(32) as u64;
    if bits <= 8 && colors > (1u64 << bits) {
        return Err(invalid());
    }
    let colors = if colors == 0 && bits <= 8 {
        1u64 << bits
    } else {
        colors
    };
    let masks = if header == 40 {
        match compression {
            3 => 12,
            6 => 16,
            _ => 0,
        }
    } else {
        0
    };
    let offset = header as u64 + masks + colors * 4;
    let stride = (width as u64 * u64::from(bits)).div_ceil(32) * 4;
    let required = offset + stride * u64::from(height.unsigned_abs());
    if required > dib.len() as u64 || offset + 14 > u32::MAX as u64 {
        return Err(invalid());
    }
    // Packed clipboard DIBs cannot reference an external color profile.
    if header == 124 && (u32_at(112) != 0 || u32_at(116) != 0) {
        return Err(invalid());
    }
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&((offset + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(&dib);
    Ok(bmp)
}

fn check_dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("Clipboard image must contain between 1 and 32 million pixels.".to_owned());
    }
    Ok(())
}

fn normalize(snapshot: Snapshot) -> Result<Payload, String> {
    match snapshot {
        Snapshot::Dib(dib) => normalize(Snapshot::Image(dib_to_bmp(dib)?, ImageFormat::Bmp)),
        Snapshot::Files(paths) if paths.is_empty() => {
            Err("Clipboard file list is empty.".to_owned())
        }
        Snapshot::Files(paths) => Ok(Payload::Files(paths)),
        Snapshot::Image(bytes, format) => {
            if bytes.len() as u64 > MAX_BYTES {
                return Err("Clipboard image exceeds the 256 MiB limit.".to_owned());
            }
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            let mut limits = Limits::default();
            limits.max_alloc = Some(MAX_BYTES);
            reader.limits(limits);
            let mut decoder = reader
                .into_decoder()
                .map_err(|e| format!("Could not decode clipboard image: {e}"))?;
            let (width, height) = decoder.dimensions();
            check_dimensions(width, height)?;
            let orientation = decoder
                .orientation()
                .map_err(|e| format!("Could not read clipboard image orientation: {e}"))?;
            let mut image = DynamicImage::from_decoder(decoder)
                .map_err(|e| format!("Could not decode clipboard image: {e}"))?;
            image.apply_orientation(orientation);
            let mut png = Cursor::new(Vec::new());
            image
                .write_to(&mut png, ImageFormat::Png)
                .map_err(|e| format!("Could not encode clipboard image: {e}"))?;
            Ok(Payload::Png(png.into_inner()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transparent_dib() -> Vec<u8> {
        let mut dib = vec![0; 124];
        for (offset, value) in [
            (0, 124u32),
            (4, 1),
            (8, (-2i32) as u32),
            (16, 3),
            (20, 8),
            (40, 0x00ff0000),
            (44, 0x0000ff00),
            (48, 0x000000ff),
            (52, 0xff000000),
        ] {
            dib[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&[30, 20, 10, 40, 70, 60, 50, 80]);
        dib
    }

    #[test]
    fn dib_v5_preserves_alpha_and_top_down_orientation() {
        let Payload::Png(png) = normalize(Snapshot::Dib(transparent_dib())).unwrap() else {
            panic!("expected PNG");
        };
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (1, 2));
        assert_eq!(image.get_pixel(0, 0).0, [10, 20, 30, 40]);
        assert_eq!(image.get_pixel(0, 1).0, [50, 60, 70, 80]);
    }

    #[test]
    fn dib_rejects_malformed_headers() {
        assert!(dib_to_bmp(vec![0; 39]).is_err());
        for (offset, value) in [
            (0, 41u32),
            (4, u32::MAX),
            (8, i32::MIN as u32),
            (16, 1),
            (32, u32::MAX),
            (112, 124),
        ] {
            let mut dib = transparent_dib();
            dib[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert!(dib_to_bmp(dib).is_err(), "offset {offset}");
        }
        let mut dib = transparent_dib();
        dib.pop();
        assert!(dib_to_bmp(dib).is_err());
    }

    #[test]
    fn preserves_file_list_and_unicode() {
        let paths = vec![
            PathBuf::from("C:\\Фото\\雪.png"),
            PathBuf::from("C:\\folder"),
        ];
        let Payload::Files(actual) = normalize(Snapshot::Files(paths.clone())).unwrap() else {
            panic!("expected files");
        };
        assert_eq!(actual, paths);
        assert!(normalize(Snapshot::Files(Vec::new())).is_err());
    }

    #[test]
    fn preserves_png_alpha() {
        let pixels = image::RgbaImage::from_pixel(2, 1, image::Rgba([12, 34, 56, 78]));
        let mut input = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(pixels.clone())
            .write_to(&mut input, ImageFormat::Png)
            .unwrap();
        let Payload::Png(output) =
            normalize(Snapshot::Image(input.into_inner(), ImageFormat::Png)).unwrap()
        else {
            panic!("expected PNG");
        };
        assert_eq!(image::load_from_memory(&output).unwrap().to_rgba8(), pixels);
    }

    #[test]
    fn normalizes_bitmap_and_rejects_invalid_images() {
        let mut input = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(2, 3)
            .write_to(&mut input, ImageFormat::Bmp)
            .unwrap();
        let Payload::Png(output) =
            normalize(Snapshot::Image(input.into_inner(), ImageFormat::Bmp)).unwrap()
        else {
            panic!("expected PNG");
        };
        assert_eq!(image::guess_format(&output).unwrap(), ImageFormat::Png);
        assert!(normalize(Snapshot::Image(vec![], ImageFormat::Png)).is_err());
        assert!(check_dimensions(8_000, 4_000).is_ok());
        assert!(check_dimensions(8_001, 4_000).is_err());
        assert!(check_dimensions(u32::MAX, u32::MAX).is_err());
        assert!(check_dimensions(0, 1).is_err());
    }
}
