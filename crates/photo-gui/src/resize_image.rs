//! Bounded still-image resizing. Call from a worker, never from the UI thread.
//! EXIF orientation is baked into pixels. Other EXIF/XMP metadata is deliberately
//! removed; ICC profiles are retained. No color-space conversion is performed.
use image::{DynamicImage, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

const MAX_PIXELS: u64 = 32_000_000;
const MAX_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Png,
    Jpeg,
    WebP,
}
impl OutputFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::WebP => "webp",
        }
    }
}
#[derive(Clone, Debug)]
pub struct ResizeOptions {
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// When true, width and height define a bounding box; one dimension is enough.
    pub preserve_aspect: bool,
    pub format: OutputFormat,
    /// Maximum output length, not an exact size. Dimensions are never reduced to meet it.
    pub target_bytes: Option<u64>,
}
impl Default for ResizeOptions {
    fn default() -> Self {
        Self {
            width: None,
            height: None,
            preserve_aspect: true,
            format: OutputFormat::Png,
            target_bytes: None,
        }
    }
}
#[derive(Debug)]
pub struct ResizeResult {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub jpeg_quality: Option<u8>,
    /// Suitable for displaying alongside success, since camera metadata is removed.
    pub metadata_notice: &'static str,
}
fn dimensions(w: u32, h: u32) -> Result<(), String> {
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        Err("Image dimensions must contain 1–32 million pixels.".into())
    } else {
        Ok(())
    }
}
fn requested_size(w: u32, h: u32, o: &ResizeOptions) -> Result<(u32, u32), String> {
    if o.width == Some(0) || o.height == Some(0) {
        return Err("Width and height must be positive.".into());
    }
    let size = if o.preserve_aspect {
        let scale = match (o.width, o.height) {
            (Some(a), Some(b)) => (a as f64 / w as f64).min(b as f64 / h as f64),
            (Some(a), None) => a as f64 / w as f64,
            (None, Some(b)) => b as f64 / h as f64,
            (None, None) => 1.0,
        };
        (
            (w as f64 * scale).round().max(1.0) as u32,
            (h as f64 * scale).round().max(1.0) as u32,
        )
    } else {
        (o.width.unwrap_or(w), o.height.unwrap_or(h))
    };
    dimensions(size.0, size.1)?;
    Ok(size)
}
/// Parse container chunks rather than searching pixel payloads for animation tags.
fn reject_animation(bytes: &[u8], format: ImageFormat) -> Result<(), String> {
    let (mut offset, png) = match format {
        ImageFormat::Png => (8usize, true),
        ImageFormat::WebP => (12, false),
        _ => return Ok(()),
    };
    while offset + 8 <= bytes.len() {
        let len = if png {
            u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
        } else {
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap())
        } as usize;
        let tag = if png {
            &bytes[offset + 4..offset + 8]
        } else {
            &bytes[offset..offset + 4]
        };
        if tag == b"acTL" || tag == b"ANIM" || tag == b"ANMF" {
            return Err("Animated images cannot be resized without losing frames.".into());
        }
        let extra = if png { 12 } else { 8 + (len & 1) };
        offset = offset
            .checked_add(len)
            .and_then(|n| n.checked_add(extra))
            .ok_or("Invalid image chunk size.")?;
        if offset > bytes.len() {
            return Err("Truncated image container.".into());
        }
    }
    Ok(())
}
fn encode(
    image: &DynamicImage,
    format: OutputFormat,
    quality: u8,
    icc: &[u8],
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let result = match format {
        OutputFormat::Png => {
            let mut encoder = image::codecs::png::PngEncoder::new_with_quality(
                &mut bytes,
                image::codecs::png::CompressionType::Best,
                image::codecs::png::FilterType::Adaptive,
            );
            if !icc.is_empty() {
                encoder
                    .set_icc_profile(icc.to_vec())
                    .map_err(|e| e.to_string())?;
            }
            encoder.write_image(
                image.as_bytes(),
                image.width(),
                image.height(),
                image.color().into(),
            )
        }
        OutputFormat::Jpeg => {
            let mut encoder =
                image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality);
            if !icc.is_empty() {
                encoder
                    .set_icc_profile(icc.to_vec())
                    .map_err(|e| e.to_string())?;
            }
            encoder.encode_image(image)
        }
        OutputFormat::WebP => {
            let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(&mut bytes);
            if !icc.is_empty() {
                encoder
                    .set_icc_profile(icc.to_vec())
                    .map_err(|e| e.to_string())?;
            }
            encoder.encode(
                image.as_bytes(),
                image.width(),
                image.height(),
                image.color().into(),
            )
        }
    };
    result.map_err(|e| format!("Could not encode resized image: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Encoded image exceeds 256 MiB.".into());
    }
    Ok(bytes)
}
/// Always publishes a new unique file via photo-core's atomic no-replace writer.
pub fn resize_copy(
    source: &Path,
    destination: &Path,
    options: &ResizeOptions,
) -> Result<ResizeResult, String> {
    if options.target_bytes == Some(0) {
        return Err("Target weight must be positive.".into());
    }
    let meta = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("Source must be a regular image file.".into());
    }
    let mut open = fs::OpenOptions::new();
    open.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        open.share_mode(1).custom_flags(0x00200000);
    }
    let mut file = open.open(source).map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if before.len() > MAX_BYTES {
        return Err("Source exceeds 256 MiB.".into());
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let after = file.metadata().map_err(|e| e.to_string())?;
    if bytes.len() as u64 != before.len()
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
    {
        return Err("Source changed while reading.".into());
    }
    let format = image::guess_format(&bytes).map_err(|e| e.to_string())?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err("Resize supports still PNG, JPEG and WebP images.".into());
    }
    reject_animation(&bytes, format)?;
    // image's JPEG decoder converts CMYK/YCCK to RGB without an ICC transform.
    // Refuse those files rather than attach a CMYK profile to RGB output.
    if format == ImageFormat::Jpeg {
        let mut offset = 2;
        while offset + 4 <= bytes.len() {
            if bytes[offset] != 0xff {
                return Err("Malformed JPEG marker.".into());
            }
            while offset < bytes.len() && bytes[offset] == 0xff {
                offset += 1;
            }
            if offset >= bytes.len() {
                break;
            }
            let marker = bytes[offset];
            offset += 1;
            if marker == 0xda || marker == 0xd9 {
                break;
            }
            if marker == 0x01 || (0xd0..=0xd8).contains(&marker) {
                continue;
            }
            if offset + 2 > bytes.len() {
                return Err("Truncated JPEG marker.".into());
            }
            let length = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize;
            if length < 2 || offset + length > bytes.len() {
                return Err("Invalid JPEG segment.".into());
            }
            if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
                if length < 8 {
                    return Err("Invalid JPEG frame.".into());
                }
                if bytes[offset + 7] == 4 {
                    return Err("CMYK/YCCK JPEG requires a color-managed conversion engine.".into());
                }
                break;
            }
            offset += length;
        }
    }
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions();
    dimensions(w, h)?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let icc = decoder
        .icc_profile()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let decoded_bytes = decoder.total_bytes();
    if bytes.len() as u64 + decoded_bytes * 2 > MAX_BYTES {
        return Err("Decode working memory would exceed 256 MiB.".into());
    }
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    let (width, height) = requested_size(image.width(), image.height(), options)?;
    // Include input, decoded pixels and resampling/output working buffers.
    let working = bytes.len() as u64
        + decoded_bytes
        + u64::from(width) * u64::from(height) * 24
        + u64::from(image.width()) * u64::from(height) * 16;
    if working > MAX_BYTES {
        return Err(
            "Resize working memory would exceed 256 MiB; choose smaller dimensions.".into(),
        );
    }
    drop(bytes);
    if options.format == OutputFormat::Jpeg && image.color().has_alpha() {
        return Err("JPEG cannot preserve alpha. Choose PNG or WebP.".into());
    }
    if options.format != OutputFormat::Png
        && image.color().bits_per_pixel() / image.color().channel_count() as u16 > 8
    {
        return Err("Choose PNG to preserve high bit depth.".into());
    }
    image = image.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
    let mut quality = 95;
    let mut encoded = encode(&image, options.format, quality, &icc)?;
    if let Some(target) = options.target_bytes {
        if options.format == OutputFormat::Jpeg {
            while encoded.len() as u64 > target && quality > 1 {
                quality -= 1;
                encoded = encode(&image, options.format, quality, &icc)?;
            }
        }
        if encoded.len() as u64 > target {
            return Err(format!(
                "Cannot meet target of {target} bytes at {width}×{height}; smallest attempted output is {} bytes. Choose smaller dimensions or another format.",
                encoded.len()
            ));
        }
    }
    let name = format!(
        "{} - resized.{}",
        source.file_stem().unwrap_or_default().to_string_lossy(),
        options.format.extension()
    );
    let path =
        photo_core::import::write_bytes(destination, &name, &encoded).map_err(|e| e.to_string())?;
    Ok(ResizeResult {
        path,
        width,
        height,
        bytes: encoded.len() as u64,
        jpeg_quality: (options.format == OutputFormat::Jpeg).then_some(quality),
        metadata_notice: "EXIF orientation applied; ICC profile retained; other EXIF/XMP metadata removed.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aspect_and_limits() {
        let mut o = ResizeOptions {
            width: Some(100),
            height: Some(100),
            preserve_aspect: true,
            format: OutputFormat::Png,
            target_bytes: None,
        };
        assert_eq!(requested_size(400, 200, &o).unwrap(), (100, 50));
        o.preserve_aspect = false;
        assert_eq!(requested_size(400, 200, &o).unwrap(), (100, 100));
        o.width = Some(0);
        assert!(requested_size(400, 200, &o).is_err());
    }
    #[test]
    fn copies_and_target_failures() {
        let folder = std::env::temp_dir().join(format!(
            "afio-resize-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&folder).unwrap();
        for format in [OutputFormat::Png, OutputFormat::Jpeg] {
            let source = folder.join(format!("source.{}", format.extension()));
            let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(40, 20, |x, y| {
                image::Rgb([(x * 6) as u8, (y * 12) as u8, 77])
            }));
            let original = encode(&image, format, 90, &[]).unwrap();
            fs::write(&source, &original).unwrap();
            let mut o = ResizeOptions {
                width: Some(20),
                height: None,
                preserve_aspect: true,
                format,
                target_bytes: None,
            };
            let a = resize_copy(&source, &folder, &o).unwrap();
            let b = resize_copy(&source, &folder, &o).unwrap();
            assert_ne!(a.path, b.path);
            assert_eq!((a.width, a.height), (20, 10));
            assert_eq!(a.bytes, fs::metadata(a.path).unwrap().len());
            assert_eq!(fs::read(&source).unwrap(), original);
            o.target_bytes = Some(1);
            assert!(resize_copy(&source, &folder, &o).is_err());
        }
        fs::remove_dir_all(folder).unwrap();
    }
}
