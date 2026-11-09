//! Bounded same-file sRGB editing. Run exclusively on an operation worker.
use crate::resize_image::{self, OutputFormat, SavedPreview};
use image::{DynamicImage, GenericImageView, ImageDecoder, ImageFormat, ImageReader, Limits};
use photo_core::{adjustments::AdjustmentOptions, rename::SourceVersion};
use std::{
    fs,
    io::{Cursor, Read},
    path::Path,
};

const MAX_FILE: usize = 64 * 1024 * 1024;
// Full-resolution editing needs several live buffers. 256 MiB rejected ordinary
// 4K/12 MP photos even when ample memory was available; retain a finite budget.
const MAX_WORK: u64 = 1024 * 1024 * 1024;

fn check_working_memory(encoded_bytes: usize, pixels: u64) -> Result<(), String> {
    let estimate = (encoded_bytes as u64)
        .saturating_mul(2)
        .saturating_add(pixels.saturating_mul(32))
        .saturating_add(16 * 1024 * 1024);
    if pixels == 0 || pixels > 32_000_000 || estimate > MAX_WORK {
        return Err(format!(
            "Изображение слишком велико для редактирования: оценка памяти {} МиБ, лимит {} МиБ, не более 32 мегапикселей. Оригинал не изменён.",
            estimate.div_ceil(1024 * 1024),
            MAX_WORK / (1024 * 1024)
        ));
    }
    Ok(())
}
pub(super) struct PreviewBase {
    pixels: DynamicImage,
    scale: f32,
}

/// Immutable, orientation-correct base; never publishes or registers a file edit.
pub(super) fn prepare_preview(
    source: &Path,
    version: &SourceVersion,
) -> Result<PreviewBase, String> {
    let check = || {
        if photo_core::rename::capture(source).map_err(|e| e.to_string())? != *version {
            Err("Source changed since the adjustment panel opened".to_owned())
        } else {
            Ok(())
        }
    };
    check()?;
    let file = fs::File::open(source).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE as u64 {
        return Err("Adjustment source exceeds 64 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE {
        return Err("Adjustment source exceeds 64 MiB".into());
    }
    let format = image::guess_format(&bytes).map_err(|e| e.to_string())?;
    resize_image::reject_animation(&bytes, format)?;
    // Run the publication metadata guard before showing an edit that cannot save.
    let mut metadata = resize_image::adjustment_metadata(&bytes, format)?;
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_WORK);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions();
    let count = u64::from(w) * u64::from(h);
    check_working_memory(bytes.len(), count)?;
    let color = decoder.color_type();
    if color.bits_per_pixel() / u16::from(color.channel_count()) != 8 {
        return Err("Adjustments support 8-bit still images only; original unchanged".into());
    }
    let icc = decoder
        .icc_profile()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    check_color(&bytes, format, &icc)?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    check_xmp_orientation(&metadata, orientation)?;
    let mut pixels = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    pixels.apply_orientation(orientation);
    if format == ImageFormat::Jpeg {
        resize_image::normalize_jpeg_exif(&mut metadata, pixels.width(), pixels.height())?;
    }
    check()?;
    let original_dimensions = pixels.dimensions();
    let preview = if pixels.width() > 1280 || pixels.height() > 1280 {
        pixels.thumbnail(1280, 1280)
    } else {
        pixels
    };
    let scale = (preview.width() as f32 / original_dimensions.0 as f32)
        .min(preview.height() as f32 / original_dimensions.1 as f32);
    Ok(PreviewBase {
        pixels: preview,
        scale,
    })
}

fn apply_lut(rgba: &mut image::RgbaImage, options: &AdjustmentOptions) {
    let mut lut = [[0u8; 256]; 3];
    for value in 0..=255u8 {
        let result = options.transform_pixel([value, value, value, 255]);
        for channel in 0..3 {
            lut[channel][value as usize] = result[channel];
        }
    }
    for pixel in rgba.pixels_mut() {
        for channel in 0..3 {
            pixel.0[channel] = lut[channel][pixel.0[channel] as usize];
        }
        if options.has_color_mix() {
            pixel.0 = options.transform_color_pixel(pixel.0);
        }
    }
}
fn check_xmp_orientation(
    metadata: &[u8],
    orientation: image::metadata::Orientation,
) -> Result<(), String> {
    if orientation != image::metadata::Orientation::NoTransforms
        && metadata
            .windows(crate::embedded_metadata::JPEG_XMP_HEADER.len())
            .any(|w| w == crate::embedded_metadata::JPEG_XMP_HEADER)
    {
        return Err("Для этого JPEG требуется поворот по EXIF. Обновление геометрических полей XMP при повороте пока не поддерживается; исходник не изменён.".into());
    }
    Ok(())
}

pub(super) fn apply_preview(
    base: &PreviewBase,
    options: &AdjustmentOptions,
) -> Result<eframe::egui::ColorImage, String> {
    options.validate()?;
    let mut rgba = base.pixels.to_rgba8();
    apply_lut(&mut rgba, options);
    crate::detail_adjustments::apply(&mut rgba, options, base.scale)?;
    Ok(eframe::egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

/// Conservatively require sRGB encoding: no unmanaged ICC or custom PNG transfer
/// function/primaries. Preserving a profile alone would not make edits correct.
fn check_color(bytes: &[u8], format: ImageFormat, icc: &[u8]) -> Result<(), String> {
    if !icc.is_empty() {
        return Err("Adjustments currently require unprofiled sRGB; ICC images need a color-managed engine. Original unchanged.".into());
    }
    if format == ImageFormat::Png {
        let mut p = 8;
        while p + 12 <= bytes.len() {
            let n = u32::from_be_bytes(bytes[p..p + 4].try_into().unwrap()) as usize;
            let end = p
                .checked_add(n)
                .and_then(|v| v.checked_add(12))
                .filter(|v| *v <= bytes.len())
                .ok_or("Invalid PNG chunk")?;
            let tag = &bytes[p + 4..p + 8];
            if tag == b"cHRM"
                || tag == b"gAMA"
                    && bytes.get(p + 8..p + 12) != Some(45455u32.to_be_bytes().as_slice())
            {
                return Err(
                    "Custom PNG color encoding is unsupported for adjustments; original unchanged."
                        .into(),
                );
            }
            p = end;
        }
    }
    Ok(())
}

/// JPEG edits re-encode at quality 95; the dialog must disclose this before save.
pub fn process(
    source: &Path,
    version: &SourceVersion,
    options: &AdjustmentOptions,
) -> Result<SavedPreview, String> {
    options.validate()?;
    if photo_core::rename::capture(source).map_err(|e| e.to_string())? != *version {
        return Err("Source changed since the command was submitted".into());
    }
    let file = fs::File::open(source).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE as u64 {
        return Err("Adjustment source exceeds 64 MiB".into());
    }
    let mut original = Vec::new();
    file.take(MAX_FILE as u64 + 1)
        .read_to_end(&mut original)
        .map_err(|e| e.to_string())?;
    if original.len() > MAX_FILE {
        return Err("Adjustment source exceeds 64 MiB".into());
    }
    let format = image::guess_format(&original).map_err(|e| e.to_string())?;
    resize_image::reject_animation(&original, format)?;
    let mut metadata = resize_image::adjustment_metadata(&original, format)?;
    let mut reader = ImageReader::with_format(Cursor::new(&original), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_WORK);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions();
    let count = u64::from(w) * u64::from(h);
    // Include metadata copies, encoder buffers, decoded verification and its
    // lossless RGBA comparison, rather than budgeting just the initial decode.
    check_working_memory(original.len(), count)?;
    let color = decoder.color_type();
    if color.bits_per_pixel() / u16::from(color.channel_count()) != 8 {
        return Err("Adjustments support 8-bit still images only; original unchanged".into());
    }
    let icc = decoder
        .icc_profile()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    check_color(&original, format, &icc)?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    check_xmp_orientation(&metadata, orientation)?;
    let mut pixels = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    pixels.apply_orientation(orientation);
    if *options == AdjustmentOptions::default() {
        if photo_core::rename::capture(source).map_err(|e| e.to_string())? != *version
            || fs::read(source).map_err(|e| e.to_string())? != original
        {
            return Err("Source changed while preparing adjustment preview".into());
        }
        return Ok(SavedPreview {
            published: crate::convert_image::PublishedFile {
                path: source.to_owned(),
                version: version.clone(),
            },
            image: crate::decode::preview_from_pixels(pixels),
        });
    }
    let alpha = pixels.color().has_alpha();
    let mut rgba = pixels.into_rgba8();
    // These controls are independent per channel. Evaluate their nonlinear
    // sRGB/curve math once for each 8-bit input, rather than per source pixel.
    apply_lut(&mut rgba, options);
    crate::detail_adjustments::apply(&mut rgba, options, 1.0)?;
    let pixels = if alpha {
        DynamicImage::ImageRgba8(rgba)
    } else {
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(rgba).into_rgb8())
    };
    let mut encoded = match format {
        ImageFormat::Png => resize_image::encode(&pixels, OutputFormat::Png, 95, &[])?,
        ImageFormat::Jpeg => resize_image::encode(&pixels, OutputFormat::Jpeg, 95, &[])?,
        ImageFormat::WebP => resize_image::encode(&pixels, OutputFormat::WebP, 95, &[])?,
        _ => return Err("Adjustments support static PNG, JPEG and WebP only".into()),
    };
    if format == ImageFormat::Png {
        encoded.splice(33..33, metadata);
    } else if format == ImageFormat::Jpeg {
        resize_image::normalize_jpeg_exif(&mut metadata, pixels.width(), pixels.height())?;
        if encoded.get(2..4) == Some(&[255, 224]) {
            let n = u16::from_be_bytes([encoded[4], encoded[5]]) as usize;
            encoded.drain(2..4 + n);
        }
        encoded.splice(2..2, metadata);
    }
    if encoded.len() > MAX_FILE {
        return Err("Adjusted output exceeds 64 MiB".into());
    }
    let verified =
        image::load_from_memory_with_format(&encoded, format).map_err(|e| e.to_string())?;
    if verified.dimensions() != pixels.dimensions()
        || format != ImageFormat::Jpeg && verified.to_rgba8() != pixels.to_rgba8()
    {
        return Err("Adjusted output failed pixel verification".into());
    }
    drop(pixels);
    let published = crate::convert_image::replace_same_file(source, version, original, encoded)
        .map_err(|e| e.to_string())?;
    Ok(SavedPreview {
        published,
        image: crate::decode::preview_from_pixels(verified),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordinary_large_photos_fit_but_working_memory_remains_bounded() {
        assert!(check_working_memory(12 * 1024 * 1024, 4096 * 4096).is_ok());
        assert!(check_working_memory(12 * 1024 * 1024, 6000 * 4000).is_ok());
        assert!(check_working_memory(64 * 1024 * 1024, 32_000_000).is_err());
        assert!(check_working_memory(1, u64::MAX).is_err());
        assert!(check_working_memory(1, 0).is_err());
    }
    #[test]
    fn jpeg_xmp_survives_photometric_edit_and_source_preview_is_read_only() {
        let dir = std::env::temp_dir().join(format!("afio-xmp-edit-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("generated.jpg");
        image::RgbImage::from_pixel(16, 16, image::Rgb([80, 120, 150]))
            .save(&source)
            .unwrap();
        let xml = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/" photoshop:Credit="Made with Google AI"/></rdf:RDF></x:xmpmeta>"#;
        let mut body = crate::embedded_metadata::JPEG_XMP_HEADER.to_vec();
        body.extend_from_slice(xml);
        let mut segment = vec![255, 225];
        segment.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        segment.extend_from_slice(&body);
        let mut original = fs::read(&source).unwrap();
        original.splice(2..2, segment.clone());
        fs::write(&source, &original).unwrap();
        let version = photo_core::rename::capture(&source).unwrap();
        let base = prepare_preview(&source, &version).unwrap();
        let options = AdjustmentOptions {
            exposure: 0.5,
            ..Default::default()
        };
        apply_preview(&base, &options).unwrap();
        assert_eq!(fs::read(&source).unwrap(), original);
        process(&source, &version, &options).unwrap();
        let changed = fs::read(&source).unwrap();
        assert_ne!(changed, original);
        assert!(changed.windows(segment.len()).any(|bytes| bytes == segment));
        let inspected = crate::embedded_metadata::inspect(&source);
        assert!(
            inspected
                .properties
                .iter()
                .any(|p| p.value == "Made with Google AI")
        );
        assert!(check_xmp_orientation(&segment, image::metadata::Orientation::Rotate90).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn tall_preview_uses_actual_reduction_scale() {
        let dir = std::env::temp_dir().join(format!("afio-tall-preview-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("tall.png");
        image::RgbaImage::from_pixel(1, 5000, image::Rgba([80, 90, 100, 255]))
            .save(&source)
            .unwrap();
        let version = photo_core::rename::capture(&source).unwrap();
        let base = prepare_preview(&source, &version).unwrap();
        assert_eq!(base.pixels.dimensions(), (1, 1280));
        assert!((base.scale - 0.256).abs() < 0.0001);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn new_controls_share_exact_small_image_preview_and_commit() {
        let dir = std::env::temp_dir().join(format!(
            "afio-extra-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("synthetic.png");
        let image = image::RgbaImage::from_fn(17, 17, |x, y| {
            image::Rgba([
                40 + (x * 7) as u8,
                30 + (y * 8) as u8,
                if (x + y) % 2 == 0 { 80 } else { 150 },
                255,
            ])
        });
        let mut original = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut original, ImageFormat::Png)
            .unwrap();
        let original = original.into_inner();
        for field in [
            "brightness",
            "black_point",
            "brilliance",
            "saturation",
            "vibrance",
            "sharpness",
            "definition",
            "noise_reduction",
            "vignette",
        ] {
            fs::write(&source, &original).unwrap();
            let mut options = AdjustmentOptions::default();
            match field {
                "brightness" => options.brightness = 0.6,
                "black_point" => options.black_point = 0.6,
                "brilliance" => options.brilliance = 0.6,
                "saturation" => options.saturation = 0.6,
                "vibrance" => options.vibrance = 0.6,
                "sharpness" => options.sharpness = 0.6,
                "definition" => options.definition = 0.6,
                "noise_reduction" => options.noise_reduction = 0.6,
                _ => options.vignette = 0.6,
            }
            let version = photo_core::rename::capture(&source).unwrap();
            let base = prepare_preview(&source, &version).unwrap();
            let expected = apply_preview(&base, &options).unwrap();
            assert_eq!(fs::read(&source).unwrap(), original);
            process(&source, &version, &options).unwrap();
            let committed = image::open(&source).unwrap().to_rgba8();
            let committed = eframe::egui::ColorImage::from_rgba_unmultiplied(
                [committed.width() as usize, committed.height() as usize],
                committed.as_raw(),
            );
            assert_eq!(committed.size, expected.size, "{field}");
            assert!(committed.pixels == expected.pixels, "{field}");
            assert_ne!(fs::read(&source).unwrap(), original, "{field}");
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn png_adjustment_retains_alpha_and_checked_source() {
        let dir = std::env::temp_dir().join(format!(
            "afio-adjust-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("synthetic.png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([128, 64, 32, 71]))
            .save(&source)
            .unwrap();
        let version = photo_core::rename::capture(&source).unwrap();
        let before = fs::read(&source).unwrap();
        process(&source, &version, &AdjustmentOptions::default()).unwrap();
        assert_eq!(fs::read(&source).unwrap(), before);
        let options = AdjustmentOptions {
            exposure: 1.0,
            ..Default::default()
        };
        let base = prepare_preview(&source, &version).unwrap();
        let preview = apply_preview(&base, &options).unwrap();
        assert_eq!(
            preview.pixels[0],
            eframe::egui::Color32::from_rgba_unmultiplied(176, 90, 47, 71)
        );
        let reset = apply_preview(&base, &AdjustmentOptions::default()).unwrap();
        assert_eq!(
            reset.pixels[0],
            eframe::egui::Color32::from_rgba_unmultiplied(128, 64, 32, 71)
        );
        assert_eq!(fs::read(&source).unwrap(), before);
        process(&source, &version, &options).unwrap();
        assert_eq!(
            image::open(&source).unwrap().to_rgba8().get_pixel(0, 0).0,
            [176, 90, 47, 71]
        );
        assert!(process(&source, &version, &options).is_err());
        let version = photo_core::rename::capture(&source).unwrap();
        let saved = fs::read(&source).unwrap();
        fs::rename(&source, dir.join("old.png")).unwrap();
        fs::write(&source, &saved).unwrap();
        assert!(process(&source, &version, &options).is_err());
        assert_eq!(fs::read(&source).unwrap(), saved);
        let deep = dir.join("deep.png");
        image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::new(2, 2)
            .save(&deep)
            .unwrap();
        let saved = fs::read(&deep).unwrap();
        let version = photo_core::rename::capture(&deep).unwrap();
        assert!(process(&deep, &version, &options).is_err());
        assert_eq!(fs::read(&deep).unwrap(), saved);
        fs::remove_dir_all(dir).unwrap();
    }
}
