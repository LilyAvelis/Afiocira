//! Small, deterministic adjustments for normalized sRGB pixels.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewState {
    pub tool: crate::actions::ActionId,
    pub finishing: bool,
    pub path: std::path::PathBuf,
    pub requested_revision: u64,
    pub ready_revision: Option<u64>,
    pub pending: bool,
    pub options: AdjustmentOptions,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdjustmentOptions {
    /// Exposure in stops, applied in linear light.
    pub exposure: f32,
    /// Relative warm/cool and green/magenta balance; not a Kelvin calibration.
    pub temperature: f32,
    pub tint: f32,
    pub contrast: f32,
    pub shadows: f32,
    pub highlights: f32,
    pub sharpness: f32,
    pub definition: f32,
    pub noise_reduction: f32,
    pub vignette: f32,
    pub brightness: f32,
    /// Positive values deepen blacks; negative values lift them.
    pub black_point: f32,
    pub saturation: f32,
    pub vibrance: f32,
    /// Luminance-weighted shadow lift and highlight compression.
    pub brilliance: f32,
    /// Piecewise linear master curve in encoded sRGB, with fixed endpoints.
    pub curve: Option<Vec<CurvePoint>>,
}

impl AdjustmentOptions {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value, bound) in [
            ("exposure", self.exposure, 5.0),
            ("temperature", self.temperature, 1.0),
            ("tint", self.tint, 1.0),
            ("contrast", self.contrast, 1.0),
            ("shadows", self.shadows, 1.0),
            ("highlights", self.highlights, 1.0),
            ("brightness", self.brightness, 1.0),
            ("black_point", self.black_point, 1.0),
            ("saturation", self.saturation, 1.0),
            ("vibrance", self.vibrance, 1.0),
            ("brilliance", self.brilliance, 1.0),
        ] {
            if !value.is_finite() || value.abs() > bound {
                return Err(format!(
                    "{name} must be finite and between -{bound} and {bound}"
                ));
            }
        }
        for (name, value) in [
            ("sharpness", self.sharpness),
            ("definition", self.definition),
            ("noise_reduction", self.noise_reduction),
            ("vignette", self.vignette),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!("{name}: допустимое значение от 0 до 1"));
            }
        }
        if let Some(points) = &self.curve
            && (!(2..=32).contains(&points.len())
                || points.first() != Some(&CurvePoint { x: 0.0, y: 0.0 })
                || points.last() != Some(&CurvePoint { x: 1.0, y: 1.0 })
                || points.iter().any(|p| {
                    !p.x.is_finite()
                        || !p.y.is_finite()
                        || !(0.0..=1.0).contains(&p.x)
                        || !(0.0..=1.0).contains(&p.y)
                })
                || points.windows(2).any(|p| p[0].x >= p[1].x))
        {
            return Err(
                "Кривая должна содержать 2–32 точки с возрастающими входными координатами; выходная яркость — от 0 до 1"
                    .into(),
            );
        }
        Ok(())
    }

    /// Caller validates once before processing. Alpha is returned byte-for-byte.
    pub fn transform_pixel(&self, pixel: [u8; 4]) -> [u8; 4] {
        let gains = [
            2.0_f32.powf(self.exposure + self.temperature * 0.5 + self.tint * 0.25),
            2.0_f32.powf(self.exposure - self.tint * 0.5),
            2.0_f32.powf(self.exposure - self.temperature * 0.5 + self.tint * 0.25),
        ];
        let mut out = pixel;
        for c in 0..3 {
            let encoded = pixel[c] as f32 / 255.0;
            let linear = if encoded <= 0.04045 {
                encoded / 12.92
            } else {
                ((encoded + 0.055) / 1.055).powf(2.4)
            };
            let linear = (linear * gains[c]).clamp(0.0, 1.0);
            let mut v = if linear <= 0.0031308 {
                linear * 12.92
            } else {
                1.055 * linear.powf(1.0 / 2.4) - 0.055
            };
            // Smooth tonal weights vanish at black/white, preserving endpoints.
            v += self.shadows * 0.6 * v * (1.0 - v).powi(3)
                + self.highlights * 0.6 * v.powi(3) * (1.0 - v);
            v = ((v - 0.5) * 2.0_f32.powf(self.contrast) + 0.5).clamp(0.0, 1.0);
            v += self.brightness * 0.4 * (1.0 - (2.0 * v - 1.0).powi(2));
            v -= self.black_point * 0.35 * (1.0 - v).powi(3);
            v = v.clamp(0.0, 1.0);
            if let Some(points) = &self.curve
                && let Some(pair) = points.windows(2).find(|p| v <= p[1].x)
            {
                let t = (v - pair[0].x) / (pair[1].x - pair[0].x);
                v = pair[0].y + t * (pair[1].y - pair[0].y);
            }
            out[c] = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        out
    }

    /// Whether a second RGB stage is needed after the per-channel LUT.
    pub fn has_color_mix(&self) -> bool {
        self.saturation != 0.0 || self.vibrance != 0.0 || self.brilliance != 0.0
    }

    /// Apply once after `transform_pixel` (or its equivalent per-channel LUT).
    /// This encoded-sRGB approximation is deterministic and retains alpha.
    pub fn transform_color_pixel(&self, pixel: [u8; 4]) -> [u8; 4] {
        if !self.has_color_mix() {
            return pixel;
        }
        let rgb = [
            pixel[0] as f32 / 255.0,
            pixel[1] as f32 / 255.0,
            pixel[2] as f32 / 255.0,
        ];
        let luma = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        let max = rgb.iter().copied().fold(0.0_f32, f32::max);
        let min = rgb.iter().copied().fold(1.0_f32, f32::min);
        let chroma = max - min;
        // Vibrance concentrates its effect on muted colors. Saturation -1
        // remains a complete grayscale conversion even with vibrance enabled.
        let scale = (1.0 + self.saturation) * (1.0 + self.vibrance * (1.0 - chroma));
        let lift = self.brilliance * 0.8 * luma * (1.0 - luma) * (1.0 - 2.0 * luma);
        let mut out = pixel;
        for c in 0..3 {
            let value = luma + (rgb[c] - luma) * scale + lift;
            out[c] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn user_curve_with_falling_output_is_valid_and_preserves_alpha() {
        let options = AdjustmentOptions {
            curve: Some(vec![
                CurvePoint { x: 0.0, y: 0.0 },
                CurvePoint { x: 0.25, y: 0.56 },
                CurvePoint { x: 0.5, y: 0.63 },
                CurvePoint { x: 0.75, y: 0.61 },
                CurvePoint { x: 1.0, y: 1.0 },
            ]),
            ..Default::default()
        };
        options.validate().unwrap();
        let middle = options.transform_pixel([128, 128, 128, 73]);
        let upper = options.transform_pixel([191, 191, 191, 73]);
        assert!(middle[0] > upper[0]);
        assert_eq!(middle[3], 73);
    }
    #[test]
    fn identity_and_alpha_are_exact() {
        let o = AdjustmentOptions::default();
        for v in 0..=255 {
            assert_eq!(
                o.transform_pixel([v, 255 - v, v / 2, v]),
                [v, 255 - v, v / 2, v]
            );
            assert_eq!(
                o.transform_color_pixel([v, 255 - v, v / 2, v]),
                [v, 255 - v, v / 2, v]
            );
        }
    }

    #[test]
    fn color_controls_preserve_gray_and_alpha_and_target_muted_colors() {
        let vivid = AdjustmentOptions {
            vibrance: 1.0,
            ..Default::default()
        };
        for v in 0..=255 {
            assert_eq!(vivid.transform_color_pixel([v, v, v, 17]), [v, v, v, 17]);
        }
        assert_eq!(vivid.transform_color_pixel([255, 0, 0, 9]), [255, 0, 0, 9]);
        let muted = [150, 120, 100, 9];
        let result = vivid.transform_color_pixel(muted);
        assert!(result[0] - result[2] > muted[0] - muted[2]);
        assert_eq!(result[3], 9);
        let gray = AdjustmentOptions {
            saturation: -1.0,
            ..Default::default()
        }
        .transform_color_pixel([210, 80, 25, 11]);
        assert_eq!(gray[0], gray[1]);
        assert_eq!(gray[1], gray[2]);
        assert_eq!(gray[3], 11);
    }

    #[test]
    fn extra_tones_are_directional_and_new_values_are_validated() {
        let bright = AdjustmentOptions {
            brightness: 1.0,
            ..Default::default()
        };
        assert!(bright.transform_pixel([100, 100, 100, 3])[0] > 100);
        let black = AdjustmentOptions {
            black_point: 1.0,
            ..Default::default()
        };
        assert!(black.transform_pixel([48, 48, 48, 3])[0] < 48);
        let brilliance = AdjustmentOptions {
            brilliance: 1.0,
            ..Default::default()
        };
        assert!(brilliance.transform_color_pixel([60, 60, 60, 3])[0] > 60);
        assert!(brilliance.transform_color_pixel([200, 200, 200, 3])[0] < 200);
        for value in [f32::NAN, f32::INFINITY, -1.01, 1.01] {
            for field in 0..5 {
                let mut options = AdjustmentOptions::default();
                match field {
                    0 => options.brightness = value,
                    1 => options.black_point = value,
                    2 => options.saturation = value,
                    3 => options.vibrance = value,
                    _ => options.brilliance = value,
                }
                assert!(options.validate().is_err());
            }
        }
        for value in [-1.0, 1.0] {
            let options = AdjustmentOptions {
                brightness: value,
                black_point: value,
                saturation: value,
                vibrance: value,
                brilliance: value,
                ..Default::default()
            };
            options.validate().unwrap();
            for v in 0..=255 {
                let result =
                    options.transform_color_pixel(options.transform_pixel([v, 255 - v, v / 2, v]));
                assert_eq!(result[3], v);
            }
        }
        let old: AdjustmentOptions = serde_json::from_str("{\"exposure\":0.5}").unwrap();
        assert_eq!(old.saturation, 0.0);
        assert_eq!(old.brightness, 0.0);
    }
    #[test]
    fn exposure_uses_linear_light_and_balance_is_directional() {
        let o = AdjustmentOptions {
            exposure: 1.0,
            ..Default::default()
        };
        assert_eq!(o.transform_pixel([128, 128, 128, 7]), [176, 176, 176, 7]);
        let o = AdjustmentOptions {
            temperature: 1.0,
            ..Default::default()
        };
        let p = o.transform_pixel([128, 128, 128, 9]);
        assert!(p[0] > p[1] && p[1] > p[2]);
    }
    #[test]
    fn invalid_values_and_curves_reject() {
        assert!(
            AdjustmentOptions {
                exposure: f32::NAN,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        for curve in [
            vec![],
            vec![CurvePoint { x: 0.0, y: 0.0 }, CurvePoint { x: 0.0, y: 1.0 }],
            vec![
                CurvePoint { x: 0.0, y: 0.0 },
                CurvePoint {
                    x: 0.5,
                    y: f32::INFINITY,
                },
                CurvePoint { x: 1.0, y: 1.0 },
            ],
        ] {
            assert!(
                AdjustmentOptions {
                    curve: Some(curve),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }

    #[test]
    fn tonal_controls_and_curve_have_visible_predictable_effects() {
        let shadows = AdjustmentOptions {
            shadows: 1.0,
            ..Default::default()
        };
        let highlights = AdjustmentOptions {
            highlights: -1.0,
            ..Default::default()
        };
        assert!(shadows.transform_pixel([48, 48, 48, 255])[0] > 48);
        assert!(highlights.transform_pixel([208, 208, 208, 255])[0] < 208);
        let curve = AdjustmentOptions {
            curve: Some(vec![
                CurvePoint { x: 0.0, y: 0.0 },
                CurvePoint { x: 0.5, y: 0.75 },
                CurvePoint { x: 1.0, y: 1.0 },
            ]),
            ..Default::default()
        };
        curve.validate().unwrap();
        assert_eq!(
            curve.transform_pixel([102, 102, 102, 3]),
            [153, 153, 153, 3]
        );
    }
}
