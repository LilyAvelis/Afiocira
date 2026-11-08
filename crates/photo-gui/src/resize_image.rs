//! Bounded still-image resizing. Call from a worker, never from the UI thread.
//! Resize copies bake EXIF orientation and remove other EXIF/XMP, retaining ICC.
//! In-place transforms preserve supported metadata and reject unsafe cases.
//! No color-space conversion is performed.
use image::{
    DynamicImage, GenericImageView, ImageDecoder, ImageEncoder, ImageFormat, ImageReader, Limits,
};
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
    /// Display pixels from the verified encoded output, bound to its publication.
    pub saved_preview: Option<SavedPreview>,
}
#[derive(Debug)]
pub struct SavedPreview {
    pub published: crate::convert_image::PublishedFile,
    pub image: eframe::egui::ColorImage,
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
pub(crate) fn reject_animation(bytes: &[u8], format: ImageFormat) -> Result<(), String> {
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
pub(crate) fn encode(
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
    process_copy(source, destination, options, None, None)
}

/// Edits the original format/path with checked staging and rollback. Unsupported
/// metadata is rejected before touching the source, never silently discarded.
pub fn transform_in_place(
    source: &Path,
    version: &photo_core::rename::SourceVersion,
    action: photo_core::actions::ActionId,
) -> Result<ResizeResult, String> {
    use photo_core::actions::ActionId;
    if photo_core::rename::capture(source).map_err(|e| e.to_string())? != *version {
        return Err("Source changed since the command was submitted".into());
    }
    let file = fs::File::open(source).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
        return Err("Transform source exceeds 64 MiB".into());
    }
    let mut original = Vec::new();
    file.take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut original)
        .map_err(|e| e.to_string())?;
    if original.len() > 64 * 1024 * 1024 {
        return Err("Transform source exceeds 64 MiB".into());
    }
    let format = image::guess_format(&original).map_err(|e| e.to_string())?;
    reject_animation(&original, format)?;
    let mut metadata = transform_metadata(&original, format)?;
    let mut reader = ImageReader::with_format(Cursor::new(&original), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions();
    dimensions(w, h)?;
    if original.len() as u64 + decoder.total_bytes() * 4 > MAX_BYTES {
        return Err("Transform working memory exceeds 256 MiB".into());
    }
    let icc = decoder
        .icc_profile()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut pixels = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    pixels.apply_orientation(orientation);
    let pixels = match action {
        ActionId::RotateLeft => pixels.rotate270(),
        ActionId::RotateRight => pixels.rotate90(),
        ActionId::FlipHorizontal => pixels.fliph(),
        ActionId::FlipVertical => pixels.flipv(),
        _ => return Err("Unknown image transform".into()),
    };
    let mut encoded = Vec::new();
    match format {
        ImageFormat::Png => {
            image::codecs::png::PngEncoder::new_with_quality(
                &mut encoded,
                image::codecs::png::CompressionType::Fast,
                image::codecs::png::FilterType::Adaptive,
            )
            .write_image(
                pixels.as_bytes(),
                pixels.width(),
                pixels.height(),
                pixels.color().into(),
            )
            .map_err(|e| e.to_string())?;
            // Preserve original independent ancillary chunks, including the exact
            // ICC/color/text payloads. Insert after IHDR, before encoded pixels.
            encoded.splice(33..33, metadata);
        }
        ImageFormat::Jpeg => {
            normalize_jpeg_exif(&mut metadata, pixels.width(), pixels.height())?;
            encoded = encode(&pixels, OutputFormat::Jpeg, 95, &[])?;
            // Encoder's JFIF header is replaced by original APP0/APP2/COM blocks.
            if encoded.get(2..4) == Some(&[255, 224]) {
                let n = u16::from_be_bytes([encoded[4], encoded[5]]) as usize;
                encoded.drain(2..4 + n);
            }
            encoded.splice(2..2, metadata);
        }
        ImageFormat::WebP => encoded = encode(&pixels, OutputFormat::WebP, 95, &icc)?,
        _ => return Err("In-place transforms support still PNG, JPEG and WebP only".into()),
    }
    let verified =
        image::load_from_memory_with_format(&encoded, format).map_err(|e| e.to_string())?;
    if verified.dimensions() != pixels.dimensions() {
        return Err("Transformed dimensions failed verification".into());
    }
    if format != ImageFormat::Jpeg
        && (verified.color() != pixels.color() || verified.as_bytes() != pixels.as_bytes())
    {
        return Err("Transformed pixels failed verification".into());
    }
    let (width, height) = pixels.dimensions();
    drop(pixels);
    let encoded_len = encoded.len() as u64;
    let published = crate::convert_image::replace_same_file(source, version, original, encoded)
        .map_err(|e| e.to_string())?;
    // Staging has released the original/encoded buffers before allocating a
    // bounded display image. JPEG previews use saved, quality-95 decoded pixels.
    let preview = crate::decode::preview_from_pixels(verified);
    Ok(ResizeResult {
        path: published.path.clone(),
        width,
        height,
        bytes: encoded_len,
        jpeg_quality: (format == ImageFormat::Jpeg).then_some(95),
        metadata_notice: if format == ImageFormat::Jpeg {
            "JPEG сохранён в том же файле; повторное сжатие JPEG, качество 95."
        } else {
            "Сохранено в том же файле."
        },
        saved_preview: Some(SavedPreview {
            published,
            image: preview,
        }),
    })
}

pub(crate) fn transform_metadata(bytes: &[u8], format: ImageFormat) -> Result<Vec<u8>, String> {
    collect_transform_metadata(bytes, format, false)
}
pub(crate) fn adjustment_metadata(bytes: &[u8], format: ImageFormat) -> Result<Vec<u8>, String> {
    collect_transform_metadata(bytes, format, true)
}
fn collect_transform_metadata(
    bytes: &[u8],
    format: ImageFormat,
    photometric: bool,
) -> Result<Vec<u8>, String> {
    let unsupported = || {
        "Встроенный блок метаданных пока не поддерживается для этой операции. Его можно посмотреть в Ctrl+G; исходник не изменён.".to_owned()
    };
    let mut saved = Vec::new();
    match format {
        ImageFormat::Png => {
            let mut p = 8usize;
            while p + 12 <= bytes.len() {
                let n = u32::from_be_bytes(bytes[p..p + 4].try_into().unwrap()) as usize;
                let end = p
                    .checked_add(n)
                    .and_then(|v| v.checked_add(12))
                    .filter(|v| *v <= bytes.len())
                    .ok_or("Invalid PNG chunk")?;
                let tag = &bytes[p + 4..p + 8];
                match tag {
                    b"IHDR" | b"PLTE" | b"IDAT" | b"IEND" | b"tRNS" => {}
                    b"iCCP" | b"sRGB" | b"gAMA" | b"cHRM" | b"tEXt" | b"zTXt" => {
                        saved.extend_from_slice(&bytes[p..end])
                    }
                    b"iTXt" if photometric => saved.extend_from_slice(&bytes[p..end]),
                    _ => return Err(unsupported()),
                }
                p = end;
            }
        }
        ImageFormat::Jpeg => {
            let mut p = 2usize;
            while p + 4 <= bytes.len() {
                if bytes[p] != 255 {
                    return Err("Invalid JPEG marker".into());
                }
                let marker = bytes[p + 1];
                if marker == 0xda || marker == 0xd9 {
                    break;
                }
                let n = u16::from_be_bytes([bytes[p + 2], bytes[p + 3]]) as usize;
                let end = p
                    .checked_add(n + 2)
                    .filter(|v| n >= 2 && *v <= bytes.len())
                    .ok_or("Invalid JPEG segment")?;
                if marker == 0xe1
                    && bytes[p + 4..end].starts_with(crate::embedded_metadata::JPEG_XMP_HEADER)
                {
                    if !photometric {
                        return Err("Этот файл содержит XMP. Чтение в Ctrl+G и коррекции поддерживаются; поворот/отражение с обновлением геометрических XMP-полей пока не реализованы. Исходник не изменён.".into());
                    }
                    let xml = &bytes[p + 4 + crate::embedded_metadata::JPEG_XMP_HEADER.len()..end];
                    crate::embedded_metadata::parse_packet(xml).map_err(|e| {
                        format!(
                            "Не удалось проверить XMP перед сохранением: {e}. Исходник не изменён."
                        )
                    })?;
                    saved.extend_from_slice(&bytes[p..end]);
                    p = end;
                    continue;
                }
                if marker == 0xeb {
                    return Err("JPEG содержит APP11/JUMBF — возможные сведения о происхождении C2PA. Они доступны для просмотра в Ctrl+G; сохранение правки с такими блоками пока не поддерживается. Исходник не изменён.".into());
                }
                if marker == 0xe0 && bytes[p + 4..end].starts_with(b"JFIF\0") {
                    let payload = &bytes[p + 4..end];
                    if payload.len() < 14 || payload[12] != 0 || payload[13] != 0 {
                        return Err("Transform refused: embedded JFIF thumbnail cannot be safely preserved; original unchanged".into());
                    }
                }
                if marker == 0xfe
                    || marker == 0xe1 && bytes[p + 4..end].starts_with(b"Exif\0\0")
                    || marker == 0xe0 && bytes[p + 4..end].starts_with(b"JFIF\0")
                    || marker == 0xe2 && bytes[p + 4..end].starts_with(b"ICC_PROFILE\0")
                {
                    saved.extend_from_slice(&bytes[p..end]);
                } else if (0xe0..=0xef).contains(&marker) {
                    return Err(format!(
                        "JPEG содержит неподдерживаемый для записи блок APP{}. Данные можно посмотреть в Ctrl+G; исходник не изменён.",
                        marker - 0xe0
                    ));
                }
                if (0xc0..=0xcf).contains(&marker)
                    && ![0xc4, 0xc8, 0xcc].contains(&marker)
                    && (n < 8 || bytes[p + 4] != 8 || bytes[p + 9] == 4)
                {
                    return Err("Only 8-bit RGB/grayscale JPEG transforms are supported".into());
                }
                p = end;
            }
        }
        ImageFormat::WebP => {
            let mut p = 12usize;
            while p + 8 <= bytes.len() {
                let n = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap()) as usize;
                let end = p
                    .checked_add(n)
                    .and_then(|v| v.checked_add(8))
                    .filter(|v| *v <= bytes.len())
                    .ok_or("Invalid WebP chunk")?;
                if !matches!(
                    &bytes[p..p + 4],
                    b"VP8 " | b"VP8L" | b"VP8X" | b"ALPH" | b"ICCP"
                ) {
                    return Err(unsupported());
                }
                p = end + (n & 1);
            }
        }
        _ => return Err("In-place transforms support still PNG, JPEG and WebP only".into()),
    }
    Ok(saved)
}

/// Patch standard TIFF orientation/dimensions in place, retaining all other
/// EXIF bytes and MakerNotes at their original offsets. Detach obsolete thumbnail
/// IFD (its bytes remain in the APP1 payload); malformed/unsupported TIFF rejects.
pub(crate) fn normalize_jpeg_exif(
    metadata: &mut [u8],
    width: u32,
    height: u32,
) -> Result<(), String> {
    fn u16_at(b: &[u8], p: usize, le: bool) -> Result<u16, String> {
        let a: [u8; 2] = b.get(p..p + 2).ok_or("Truncated EXIF")?.try_into().unwrap();
        Ok(if le {
            u16::from_le_bytes(a)
        } else {
            u16::from_be_bytes(a)
        })
    }
    fn u32_at(b: &[u8], p: usize, le: bool) -> Result<u32, String> {
        let a: [u8; 4] = b.get(p..p + 4).ok_or("Truncated EXIF")?.try_into().unwrap();
        Ok(if le {
            u32::from_le_bytes(a)
        } else {
            u32::from_be_bytes(a)
        })
    }
    fn ifd(
        b: &mut [u8],
        p: usize,
        le: bool,
        width: u32,
        height: u32,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 4 {
            return Err("EXIF nesting cannot be safely updated".into());
        }
        let count = u16_at(b, p, le)? as usize;
        let end = p
            .checked_add(2 + count * 12)
            .filter(|v| v + 4 <= b.len())
            .ok_or("Truncated EXIF directory")?;
        for e in (p + 2..end).step_by(12) {
            let tag = u16_at(b, e, le)?;
            let kind = u16_at(b, e + 2, le)?;
            let n = u32_at(b, e + 4, le)?;
            if tag == 34665 || tag == 34853 {
                if kind != 4 || n != 1 {
                    return Err("Unsupported EXIF directory pointer".into());
                }
                let next = u32_at(b, e + 8, le)? as usize;
                if next != 0 {
                    ifd(b, next, le, width, height, depth + 1)?;
                }
            } else if matches!(tag, 256 | 257 | 274 | 40962 | 40963) {
                if n != 1 || !matches!(kind, 3 | 4) {
                    return Err("Unsupported EXIF orientation/dimension field".into());
                }
                let value = match tag {
                    274 => 1,
                    256 | 40962 => width,
                    _ => height,
                };
                if kind == 3 {
                    let value =
                        u16::try_from(value).map_err(|_| "EXIF SHORT dimension overflow")?;
                    b[e + 8..e + 10].copy_from_slice(&if le {
                        value.to_le_bytes()
                    } else {
                        value.to_be_bytes()
                    });
                } else {
                    b[e + 8..e + 12].copy_from_slice(&if le {
                        value.to_le_bytes()
                    } else {
                        value.to_be_bytes()
                    });
                }
            }
        }
        // IFD1 is an embedded preview with stale pixels after a transform.
        b[end..end + 4].fill(0);
        Ok(())
    }
    let mut p = 0;
    while p + 4 <= metadata.len() {
        let n = u16::from_be_bytes([metadata[p + 2], metadata[p + 3]]) as usize;
        let end = p
            .checked_add(n + 2)
            .filter(|v| n >= 2 && *v <= metadata.len())
            .ok_or("Invalid preserved JPEG metadata")?;
        if metadata[p + 1] == 0xe1 && metadata[p + 4..end].starts_with(b"Exif\0\0") {
            let b = &mut metadata[p + 10..end];
            let le = match b.get(..2) {
                Some(b"II") => true,
                Some(b"MM") => false,
                _ => return Err("Unsupported EXIF byte order".into()),
            };
            if u16_at(b, 2, le)? != 42 {
                return Err("Unsupported EXIF TIFF header".into());
            }
            let first = u32_at(b, 4, le)? as usize;
            if first != 0 {
                ifd(b, first, le, width, height, 0)?;
            }
        }
        p = end;
    }
    Ok(())
}

fn process_copy(
    source: &Path,
    destination: &Path,
    options: &ResizeOptions,
    transform: Option<photo_core::actions::ActionId>,
    version: Option<&photo_core::rename::SourceVersion>,
) -> Result<ResizeResult, String> {
    use photo_core::actions::ActionId;
    if let Some(version) = version
        && &photo_core::rename::capture(source).map_err(|e| e.to_string())? != version
    {
        return Err("Source changed since the command was submitted".into());
    }
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
    image = match transform {
        Some(ActionId::RotateLeft) => image.rotate270(),
        Some(ActionId::RotateRight) => image.rotate90(),
        Some(ActionId::FlipHorizontal) => image.fliph(),
        Some(ActionId::FlipVertical) => image.flipv(),
        Some(_) => return Err("Unknown image transform".into()),
        None => image,
    };
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
    if image.width() != width || image.height() != height {
        image = image.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
    }
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
    if transform.is_some() {
        let verified = image::load_from_memory_with_format(&encoded, ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        if verified.width() != image.width()
            || verified.height() != image.height()
            || verified.color() != image.color()
            || verified.as_bytes() != image.as_bytes()
        {
            return Err("Transformed PNG failed pixel verification".into());
        }
    }
    if let Some(version) = version
        && &photo_core::rename::capture(source).map_err(|e| e.to_string())? != version
    {
        return Err("Source changed before publication".into());
    }
    let suffix = match transform {
        Some(ActionId::RotateLeft) => "rotated-left",
        Some(ActionId::RotateRight) => "rotated-right",
        Some(ActionId::FlipHorizontal) => "flipped-horizontal",
        Some(ActionId::FlipVertical) => "flipped-vertical",
        _ => "resized",
    };
    let name = format!(
        "{} - {suffix}.{}",
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
        saved_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transform_preview_matches_saved_png_and_lossy_jpeg() {
        let root = std::env::temp_dir().join(format!(
            "afio-saved-preview-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        for extension in ["png", "jpg"] {
            let source = root.join(format!("source.{extension}"));
            DynamicImage::ImageRgb8(image::RgbImage::from_fn(2701, 17, |x, y| {
                image::Rgb([
                    ((x * 37 + y * 53) % 256) as u8,
                    ((x * 71 + y * 19) % 256) as u8,
                    ((x * 13 + y * 97) % 256) as u8,
                ])
            }))
            .save(&source)
            .unwrap();
            let before = image::open(&source).unwrap().rotate90();
            let result = transform_in_place(
                &source,
                &photo_core::rename::capture(&source).unwrap(),
                photo_core::actions::ActionId::RotateRight,
            )
            .unwrap();
            let saved = result.saved_preview.unwrap();
            assert_eq!(saved.published.path, source);
            assert_eq!(
                saved.published.version,
                photo_core::rename::capture(&source).unwrap()
            );
            let expected = crate::decode::preview_from_pixels(image::open(&source).unwrap());
            assert_eq!(saved.image.size, expected.size);
            assert_eq!(saved.image.pixels, expected.pixels);
            assert_eq!(*saved.image.size.iter().max().unwrap(), 2560);
            if extension == "jpg" {
                let pre_encode = crate::decode::preview_from_pixels(before);
                assert_ne!(saved.image.pixels, pre_encode.pixels);
            }
            fs::write(&source, b"externally edited after publication").unwrap();
            assert!(!saved.published.is_current());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn in_place_transforms_are_cumulative_preserve_precision_and_reject_stale() {
        use photo_core::actions::ActionId;
        let root = std::env::temp_dir().join(format!(
            "afio-transform-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        for sixteen in [false, true] {
            let source = root.join(if sixteen { "sixteen.png" } else { "alpha.png" });
            let image = if sixteen {
                DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(3, 2, |x, y| {
                    image::Rgba([x as u16 * 20000, y as u16 * 30000, 42321, 12345])
                }))
            } else {
                DynamicImage::ImageRgba8(image::ImageBuffer::from_fn(3, 2, |x, y| {
                    image::Rgba([x as u8 * 60, y as u8 * 90, 42, 123])
                }))
            };
            image.save(&source).unwrap();
            let initial_count = fs::read_dir(&root).unwrap().count();
            for action in [
                ActionId::RotateRight,
                ActionId::RotateRight,
                ActionId::RotateRight,
                ActionId::RotateRight,
                ActionId::FlipHorizontal,
                ActionId::FlipHorizontal,
                ActionId::FlipVertical,
                ActionId::FlipVertical,
            ] {
                let version = photo_core::rename::capture(&source).unwrap();
                let result = transform_in_place(&source, &version, action).unwrap();
                assert_eq!(result.path, source);
                assert_eq!(fs::read_dir(&root).unwrap().count(), initial_count);
            }
            let after = image::open(&source).unwrap();
            assert_eq!(after.color(), image.color());
            assert_eq!(after.as_bytes(), image.as_bytes());
            let stale = photo_core::rename::capture(&source).unwrap();
            fs::write(&source, b"changed externally").unwrap();
            assert!(transform_in_place(&source, &stale, ActionId::RotateLeft).is_err());
            assert_eq!(fs::read(&source).unwrap(), b"changed externally");
        }
        let jpeg = root.join("same.jpeg");
        DynamicImage::new_rgb8(3, 2).save(&jpeg).unwrap();
        transform_in_place(
            &jpeg,
            &photo_core::rename::capture(&jpeg).unwrap(),
            ActionId::RotateLeft,
        )
        .unwrap();
        assert_eq!(
            image::guess_format(&fs::read(&jpeg).unwrap()).unwrap(),
            ImageFormat::Jpeg
        );
        assert_eq!(image::open(&jpeg).unwrap().dimensions(), (2, 3));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn in_place_rejects_metadata_without_touching_original() {
        use photo_core::actions::ActionId;
        let root = std::env::temp_dir().join(format!("afio-transform-meta-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.jpg");
        DynamicImage::new_rgb8(3, 2).save(&source).unwrap();
        let mut bytes = fs::read(&source).unwrap();
        bytes.splice(2..2, [255, 225, 0, 8, b'E', b'x', b'i', b'f', 0, 0]);
        fs::write(&source, &bytes).unwrap();
        assert!(
            transform_in_place(
                &source,
                &photo_core::rename::capture(&source).unwrap(),
                ActionId::RotateRight
            )
            .unwrap_err()
            .contains("EXIF")
        );
        assert_eq!(fs::read(&source).unwrap(), bytes);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn jpeg_exif_normalizes_once_and_retains_other_tags_and_icc() {
        use photo_core::actions::ActionId;
        let root = std::env::temp_dir().join(format!("afio-exif-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.jpg");
        DynamicImage::new_rgb8(3, 2).save(&source).unwrap();
        let mut tiff = vec![b'I', b'I', 42, 0, 8, 0, 0, 0, 4, 0];
        for (tag, kind, value) in [
            (256u16, 4u16, 3u32),
            (257, 4, 2),
            (274, 3, 6),
            (270, 2, 0x00636261),
        ] {
            tiff.extend(tag.to_le_bytes());
            tiff.extend(kind.to_le_bytes());
            tiff.extend((if tag == 270 { 4u32 } else { 1u32 }).to_le_bytes());
            tiff.extend(value.to_le_bytes());
        }
        tiff.extend(0u32.to_le_bytes());
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend(tiff);
        let mut metadata = vec![255, 225];
        metadata.extend(((payload.len() + 2) as u16).to_be_bytes());
        metadata.extend(payload);
        let icc = b"ICC_PROFILE\0\x01\x01retained-profile";
        metadata.extend([255, 226]);
        metadata.extend(((icc.len() + 2) as u16).to_be_bytes());
        metadata.extend(icc);
        let mut bytes = fs::read(&source).unwrap();
        bytes.splice(2..2, metadata);
        fs::write(&source, bytes).unwrap();
        for expected in [(3, 2), (2, 3)] {
            transform_in_place(
                &source,
                &photo_core::rename::capture(&source).unwrap(),
                ActionId::RotateRight,
            )
            .unwrap();
            let bytes = fs::read(&source).unwrap();
            let mut decoder = ImageReader::open(&source).unwrap().into_decoder().unwrap();
            assert_eq!(decoder.dimensions(), expected);
            assert_eq!(
                decoder.orientation().unwrap(),
                image::metadata::Orientation::NoTransforms
            );
            assert!(bytes.windows(icc.len()).any(|w| w == icc));
            assert!(bytes.windows(4).any(|w| w == b"abc\0"));
        }
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
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
