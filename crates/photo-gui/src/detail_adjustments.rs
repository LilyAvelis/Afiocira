//! Bounded spatial sRGB adjustments, used only by the adjustment worker.
use image::RgbaImage;
use photo_core::adjustments::AdjustmentOptions;

/// `scale` is the preview/source ratio; use 1 for full-resolution output.
/// Two temporary RGBA buffers (8 bytes/pixel) are live during each blur.
pub(super) fn apply(
    pixels: &mut RgbaImage,
    options: &AdjustmentOptions,
    scale: f32,
) -> Result<(), String> {
    if !scale.is_finite() || !(0.0..=1.0).contains(&scale) || scale == 0.0 {
        return Err("Detail adjustment scale must be between zero and one".into());
    }
    for value in [
        options.sharpness,
        options.definition,
        options.noise_reduction,
        options.vignette,
    ] {
        if !value.is_finite() || !(0.0..=1.0).contains(&value) {
            return Err("Detail adjustment strengths must be between zero and one".into());
        }
    }
    if pixels.width() == 0 || pixels.height() == 0 {
        return Ok(());
    }
    if options.noise_reduction > 0.0 {
        let smooth = blur(pixels, (2.0 * scale).round().max(1.0) as u32);
        for (pixel, soft) in pixels.pixels_mut().zip(smooth.pixels()) {
            // Limit smoothing across strong edges. This is a simple spatial
            // filter, not model-based reconstruction of lost detail.
            let difference = (0..3)
                .map(|c| (pixel[c] as f32 - soft[c] as f32).abs())
                .sum::<f32>()
                / 3.0;
            let amount =
                options.noise_reduction * (2.0 * scale).min(1.0) * 0.8 / (1.0 + difference / 32.0);
            for c in 0..3 {
                pixel[c] = byte(pixel[c] as f32 + amount * (soft[c] as f32 - pixel[c] as f32));
            }
        }
    }
    for (strength, radius, gain) in [
        (options.definition, 24.0, 0.7),
        (options.sharpness, 1.2, 1.3),
    ] {
        if strength > 0.0 {
            let soft = blur(pixels, (radius * scale).round().max(1.0) as u32);
            for (pixel, low) in pixels.pixels_mut().zip(soft.pixels()) {
                for c in 0..3 {
                    pixel[c] = byte(
                        pixel[c] as f32
                            + strength
                                * gain
                                * (radius * scale).min(1.0)
                                * (pixel[c] as f32 - low[c] as f32),
                    );
                }
            }
        }
    }
    if options.vignette > 0.0 {
        let (width, height) = pixels.dimensions();
        for (x, y, pixel) in pixels.enumerate_pixels_mut() {
            let nx = (2.0 * (x as f32 + 0.5) / width as f32 - 1.0).abs();
            let ny = (2.0 * (y as f32 + 0.5) / height as f32 - 1.0).abs();
            let radial = ((nx * nx + ny * ny) / 2.0).clamp(0.0, 1.0);
            let fade = radial * radial * (3.0 - 2.0 * radial);
            let factor = 1.0 - options.vignette * 0.75 * fade;
            for c in 0..3 {
                pixel[c] = byte(pixel[c] as f32 * factor);
            }
        }
    }
    Ok(())
}

fn byte(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// Separable box filter with clipped borders and a bounded sliding window.
/// Alpha in the caller is never written by any spatial operation.
fn blur(source: &RgbaImage, radius: u32) -> RgbaImage {
    let (width, height) = source.dimensions();
    let mut horizontal = RgbaImage::new(width, height);
    let mut result = RgbaImage::new(width, height);
    let radius = radius.clamp(1, 256);
    for y in 0..height {
        let mut sums = [0u32; 4];
        let mut left = 0;
        let mut right = 0;
        for x in 0..width {
            let end = x.saturating_add(radius).min(width - 1);
            while right <= end {
                accumulate(&mut sums, source.get_pixel(right, y), true);
                right += 1;
            }
            let start = x.saturating_sub(radius);
            while left < start {
                accumulate(&mut sums, source.get_pixel(left, y), false);
                left += 1;
            }
            *horizontal.get_pixel_mut(x, y) = weighted_pixel(sums, right - left);
        }
    }
    for x in 0..width {
        let mut sums = [0u32; 4];
        let mut top = 0;
        let mut bottom = 0;
        for y in 0..height {
            let end = y.saturating_add(radius).min(height - 1);
            while bottom <= end {
                accumulate(&mut sums, horizontal.get_pixel(x, bottom), true);
                bottom += 1;
            }
            let start = y.saturating_sub(radius);
            while top < start {
                accumulate(&mut sums, horizontal.get_pixel(x, top), false);
                top += 1;
            }
            *result.get_pixel_mut(x, y) = weighted_pixel(sums, bottom - top);
        }
    }
    result
}

// Keep the intermediate unpremultiplied RGB plus averaged alpha in RGBA8.
// This quantizes weights to 8 bits between passes, but constant visible colors
// remain exact and hidden RGB never contributes to a visible neighborhood.
fn accumulate(sums: &mut [u32; 4], pixel: &image::Rgba<u8>, add: bool) {
    let alpha = u32::from(pixel[3]);
    for c in 0..4 {
        let value = if c == 3 {
            alpha
        } else {
            u32::from(pixel[c]) * alpha
        };
        if add {
            sums[c] += value;
        } else {
            sums[c] -= value;
        }
    }
}

fn weighted_pixel(sums: [u32; 4], count: u32) -> image::Rgba<u8> {
    let mut pixel = image::Rgba([0; 4]);
    if sums[3] > 0 {
        for c in 0..3 {
            pixel[c] = (sums[c] + sums[3] / 2).checked_div(sums[3]).unwrap_or(0) as u8;
        }
        pixel[3] = ((sums[3] + count / 2) / count) as u8;
    }
    pixel
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_saturated_rgb_does_not_tint_visible_neighbors() {
        for options in [
            AdjustmentOptions {
                noise_reduction: 1.0,
                ..Default::default()
            },
            AdjustmentOptions {
                sharpness: 1.0,
                ..Default::default()
            },
            AdjustmentOptions {
                definition: 1.0,
                ..Default::default()
            },
        ] {
            let mut pixels = RgbaImage::from_fn(11, 11, |x, y| {
                if x < 4 || y < 4 {
                    image::Rgba([255, 0, 0, 0])
                } else {
                    image::Rgba([70, 120, 150, 255])
                }
            });
            apply(&mut pixels, &options, 1.0).unwrap();
            for pixel in pixels.pixels().filter(|p| p[3] == 255) {
                assert_eq!(pixel.0, [70, 120, 150, 255]);
            }
        }
    }
    fn fixture() -> RgbaImage {
        RgbaImage::from_fn(17, 17, |x, y| {
            image::Rgba([
                if (x + y) % 2 == 0 { 110 } else { 146 },
                100,
                90,
                ((x * 13 + y * 5) % 256) as u8,
            ])
        })
    }
    #[test]
    fn neutral_is_byte_identical_and_effects_preserve_alpha() {
        let original = fixture();
        let mut output = original.clone();
        apply(&mut output, &AdjustmentOptions::default(), 1.0).unwrap();
        assert_eq!(output, original);
        let options = AdjustmentOptions {
            sharpness: 0.7,
            definition: 0.6,
            noise_reduction: 0.8,
            vignette: 0.5,
            ..Default::default()
        };
        apply(&mut output, &options, 0.5).unwrap();
        assert!(
            output
                .pixels()
                .zip(original.pixels())
                .all(|(a, b)| a[3] == b[3])
        );
    }
    #[test]
    fn noise_filter_reduces_variation() {
        let mut output = fixture();
        let original = output.clone();
        apply(
            &mut output,
            &AdjustmentOptions {
                noise_reduction: 1.0,
                ..Default::default()
            },
            1.0,
        )
        .unwrap();
        let variation = |image: &RgbaImage| {
            image
                .pixels()
                .map(|p| (p[0] as f32 - 128.0).powi(2))
                .sum::<f32>()
        };
        assert!(variation(&output) < variation(&original));
    }
    #[test]
    fn sharpness_increases_edge_contrast() {
        let mut output = RgbaImage::from_fn(9, 3, |x, _| {
            image::Rgba([if x < 4 { 80 } else { 160 }, 100, 100, 255])
        });
        apply(
            &mut output,
            &AdjustmentOptions {
                sharpness: 1.0,
                ..Default::default()
            },
            1.0,
        )
        .unwrap();
        assert!(output.get_pixel(3, 1)[0] < 80);
        assert!(output.get_pixel(4, 1)[0] > 160);
    }
    #[test]
    fn vignette_darkens_corners_and_retains_center() {
        let mut output = RgbaImage::from_pixel(21, 21, image::Rgba([160, 160, 160, 77]));
        apply(
            &mut output,
            &AdjustmentOptions {
                vignette: 1.0,
                ..Default::default()
            },
            1.0,
        )
        .unwrap();
        assert_eq!(output.get_pixel(10, 10)[0], 160);
        assert!(output.get_pixel(0, 0)[0] < 100);
    }

    #[test]
    fn definition_changes_broad_contrast_and_single_pixel_is_stable() {
        let mut output = RgbaImage::from_fn(65, 3, |x, _| {
            image::Rgba([if x < 32 { 100 } else { 150 }, 90, 90, 255])
        });
        let options = AdjustmentOptions {
            definition: 1.0,
            ..Default::default()
        };
        apply(&mut output, &options, 1.0).unwrap();
        assert!(output.get_pixel(24, 1)[0] < 100);
        assert!(output.get_pixel(40, 1)[0] > 150);
        let mut single = RgbaImage::from_pixel(1, 1, image::Rgba([90, 120, 150, 43]));
        let original = single.clone();
        apply(&mut single, &options, 0.1).unwrap();
        assert_eq!(single, original);
        assert!(apply(&mut single, &options, f32::NAN).is_err());
    }
}
