//! Video post-processing effects engine: Color LUT presets, Portrait Bokeh blur,
//! and Auto-Framing digital pan/crop.

use serde::{Deserialize, Serialize};

/* ------------------------------------------------------------------ */
/* Color LUT Presets (§Pro Controls)                                 */
/* ------------------------------------------------------------------ */

/// Preset color grading filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LutPreset {
    #[default]
    Natural,
    WarmStudio,
    CoolCorporate,
    VibrantStreamer,
    Monochrome,
}

impl LutPreset {
    pub fn all() -> &'static [Self] {
        &[
            Self::Natural,
            Self::WarmStudio,
            Self::CoolCorporate,
            Self::VibrantStreamer,
            Self::Monochrome,
        ]
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Natural => "Natural",
            Self::WarmStudio => "Warm Studio",
            Self::CoolCorporate => "Cool Corporate",
            Self::VibrantStreamer => "Vibrant Streamer",
            Self::Monochrome => "Monochrome",
        }
    }
}

/// Applies a color grading filter preset in-place to a premultiplied BGRA pixel buffer.
/// Channel order: [B, G, R, A]. Alpha channel is preserved.
pub fn apply_lut(bgra: &mut [u8], preset: LutPreset) {
    if preset == LutPreset::Natural {
        return;
    }

    match preset {
        LutPreset::Natural => {}
        LutPreset::WarmStudio => {
            // Warm studio: boost reds and warm tones, gently raise contrast
            for chunk in bgra.chunks_exact_mut(4) {
                let b = chunk[0] as f32;
                let g = chunk[1] as f32;
                let r = chunk[2] as f32;

                // Contrast enhancement centered at 128
                let r_c = (r - 128.0) * 1.10 + 128.0 + 10.0;
                let g_c = (g - 128.0) * 1.05 + 128.0 + 3.0;
                let b_c = (b - 128.0) * 1.02 + 128.0 - 6.0;

                chunk[0] = b_c.clamp(0.0, 255.0) as u8;
                chunk[1] = g_c.clamp(0.0, 255.0) as u8;
                chunk[2] = r_c.clamp(0.0, 255.0) as u8;
            }
        }
        LutPreset::CoolCorporate => {
            // Cool corporate: crisp blue tone, slight lift to shadows, clean whites
            for chunk in bgra.chunks_exact_mut(4) {
                let b = chunk[0] as f32;
                let g = chunk[1] as f32;
                let r = chunk[2] as f32;

                let r_c = (r - 128.0) * 1.03 + 128.0 - 5.0;
                let g_c = (g - 128.0) * 1.04 + 128.0 + 2.0;
                let b_c = (b - 128.0) * 1.08 + 128.0 + 12.0;

                chunk[0] = b_c.clamp(0.0, 255.0) as u8;
                chunk[1] = g_c.clamp(0.0, 255.0) as u8;
                chunk[2] = r_c.clamp(0.0, 255.0) as u8;
            }
        }
        LutPreset::VibrantStreamer => {
            // Vibrant streamer: punchy colors with saturation boost
            for chunk in bgra.chunks_exact_mut(4) {
                let b = chunk[0] as f32;
                let g = chunk[1] as f32;
                let r = chunk[2] as f32;

                // ITU-R BT.601 luma calculation
                let luma = 0.299 * r + 0.587 * g + 0.114 * b;
                let sat = 1.35; // 35% saturation boost

                let r_sat = luma + (r - luma) * sat;
                let g_sat = luma + (g - luma) * sat;
                let b_sat = luma + (b - luma) * sat;

                chunk[0] = b_sat.clamp(0.0, 255.0) as u8;
                chunk[1] = g_sat.clamp(0.0, 255.0) as u8;
                chunk[2] = r_sat.clamp(0.0, 255.0) as u8;
            }
        }
        LutPreset::Monochrome => {
            // Pure black & white luminance
            for chunk in bgra.chunks_exact_mut(4) {
                let b = chunk[0] as u32;
                let g = chunk[1] as u32;
                let r = chunk[2] as u32;

                // Fixed point ITU-R BT.601: (77*R + 150*G + 29*B) >> 8
                let y = ((77 * r + 150 * g + 29 * b) >> 8).min(255) as u8;
                chunk[0] = y;
                chunk[1] = y;
                chunk[2] = y;
            }
        }
    }
}

/* ------------------------------------------------------------------ */
/* Portrait Bokeh / Background Blur Filter                            */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BlurIntensity {
    #[default]
    Soft,
    Studio,
    Dreamy,
}

impl BlurIntensity {
    pub fn radius(&self) -> usize {
        match self {
            Self::Soft => 3,
            Self::Studio => 6,
            Self::Dreamy => 10,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Soft => "Soft",
            Self::Studio => "Studio",
            Self::Dreamy => "Dreamy",
        }
    }
}

/// Applies a portrait bokeh effect to a BGRA buffer. Keeps an elliptical central region
/// (person subject) in sharp focus and smoothly blurs the background.
pub fn apply_portrait_blur(
    bgra: &mut [u8],
    width: usize,
    height: usize,
    intensity: BlurIntensity,
    amount: f32,
) {
    if amount <= 0.001 || width == 0 || height == 0 {
        return;
    }
    let radius = intensity.radius();
    if radius == 0 {
        return;
    }

    let total_pixels = width * height;
    if bgra.len() < total_pixels * 4 {
        return;
    }

    // Allocate temporary buffer for separable box blur
    let mut blurred = bgra.to_vec();
    let mut temp = vec![0u8; bgra.len()];

    // Horizontal 1D box blur pass
    let r = radius as isize;
    for y in 0..height {
        let row_start = y * width * 4;
        for x in 0..width {
            let mut sum_b = 0u32;
            let mut sum_g = 0u32;
            let mut sum_r = 0u32;
            let mut count = 0u32;

            let x_start = (x as isize - r).max(0) as usize;
            let x_end = (x as isize + r).min(width as isize - 1) as usize;

            for ix in x_start..=x_end {
                let idx = row_start + ix * 4;
                sum_b += blurred[idx] as u32;
                sum_g += blurred[idx + 1] as u32;
                sum_r += blurred[idx + 2] as u32;
                count += 1;
            }

            let out_idx = row_start + x * 4;
            temp[out_idx] = (sum_b / count) as u8;
            temp[out_idx + 1] = (sum_g / count) as u8;
            temp[out_idx + 2] = (sum_r / count) as u8;
            temp[out_idx + 3] = blurred[out_idx + 3];
        }
    }

    // Vertical 1D box blur pass
    for x in 0..width {
        for y in 0..height {
            let mut sum_b = 0u32;
            let mut sum_g = 0u32;
            let mut sum_r = 0u32;
            let mut count = 0u32;

            let y_start = (y as isize - r).max(0) as usize;
            let y_end = (y as isize + r).min(height as isize - 1) as usize;

            for iy in y_start..=y_end {
                let idx = (iy * width + x) * 4;
                sum_b += temp[idx] as u32;
                sum_g += temp[idx + 1] as u32;
                sum_r += temp[idx + 2] as u32;
                count += 1;
            }

            let out_idx = (y * width + x) * 4;
            blurred[out_idx] = (sum_b / count) as u8;
            blurred[out_idx + 1] = (sum_g / count) as u8;
            blurred[out_idx + 2] = (sum_r / count) as u8;
        }
    }

    // Blend: Portrait vignette mask centered on (0.50, 0.45)
    let center_x = width as f32 * 0.50;
    let center_y = height as f32 * 0.45;
    let semi_x = width as f32 * 0.30;
    let semi_y = height as f32 * 0.40;

    let eff_amount = amount.clamp(0.0, 1.0);

    for y in 0..height {
        let dy = (y as f32 - center_y) / semi_y;
        let dy2 = dy * dy;
        let row_offset = y * width * 4;

        for x in 0..width {
            let dx = (x as f32 - center_x) / semi_x;
            let d = (dx * dx + dy2).sqrt();

            // Smooth feathering: 0.0 inside subject, transitions 1.0 to 1.6 to background
            let blend = if d < 0.85 {
                0.0
            } else if d > 1.40 {
                eff_amount
            } else {
                let t = (d - 0.85) / (1.40 - 0.85);
                // Hermite smoothstep: 3*t^2 - 2*t^3
                let s = t * t * (3.0 - 2.0 * t);
                s * eff_amount
            };

            if blend > 0.005 {
                let idx = row_offset + x * 4;
                let inv = 1.0 - blend;
                bgra[idx] = (bgra[idx] as f32 * inv + blurred[idx] as f32 * blend) as u8;
                bgra[idx + 1] =
                    (bgra[idx + 1] as f32 * inv + blurred[idx + 1] as f32 * blend) as u8;
                bgra[idx + 2] =
                    (bgra[idx + 2] as f32 * inv + blurred[idx + 2] as f32 * blend) as u8;
            }
        }
    }
}

/* ------------------------------------------------------------------ */
/* Auto-Framing Digital Pan/Crop Math                                 */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoFramingTracker {
    pub target_center_x: f32,
    pub target_center_y: f32,
    pub current_center_x: f32,
    pub current_center_y: f32,
    pub zoom: f32,
    pub smoothing: f32,
}

impl Default for AutoFramingTracker {
    fn default() -> Self {
        Self::new(1.20, 0.12)
    }
}

impl AutoFramingTracker {
    pub fn new(zoom: f32, smoothing: f32) -> Self {
        Self {
            target_center_x: 0.50,
            target_center_y: 0.50,
            current_center_x: 0.50,
            current_center_y: 0.50,
            zoom: zoom.clamp(1.0, 2.5),
            smoothing: smoothing.clamp(0.01, 1.0),
        }
    }

    /// Sets the target center normalized coordinates (0.0 to 1.0).
    pub fn set_target(&mut self, x: f32, y: f32) {
        self.target_center_x = x.clamp(0.15, 0.85);
        self.target_center_y = y.clamp(0.15, 0.85);
    }

    /// Advances the smoothing interpolation toward the target center.
    pub fn step(&mut self) {
        self.current_center_x += (self.target_center_x - self.current_center_x) * self.smoothing;
        self.current_center_y += (self.target_center_y - self.current_center_y) * self.smoothing;
    }

    /// Computes the cropped sub-rectangle matching the original aspect ratio.
    pub fn compute_crop(&self, src_width: u32, src_height: u32) -> CropRect {
        if self.zoom <= 1.001 || src_width == 0 || src_height == 0 {
            return CropRect {
                x: 0,
                y: 0,
                width: src_width,
                height: src_height,
            };
        }

        let crop_w = ((src_width as f32 / self.zoom).round() as u32).max(16) & !1;
        let crop_h = ((src_height as f32 / self.zoom).round() as u32).max(16) & !1;

        let center_px_x = (self.current_center_x * src_width as f32).round() as i32;
        let center_px_y = (self.current_center_y * src_height as f32).round() as i32;

        let mut x = center_px_x - (crop_w as i32 / 2);
        let mut y = center_px_y - (crop_h as i32 / 2);

        // Clamp crop rectangle within image boundaries
        if x < 0 {
            x = 0;
        } else if x + crop_w as i32 > src_width as i32 {
            x = (src_width as i32 - crop_w as i32).max(0);
        }

        if y < 0 {
            y = 0;
        } else if y + crop_h as i32 > src_height as i32 {
            y = (src_height as i32 - crop_h as i32).max(0);
        }

        CropRect {
            x: (x as u32) & !1,
            y: (y as u32) & !1,
            width: crop_w,
            height: crop_h,
        }
    }

    /// Crops and rescales a BGRA source image into a destination buffer.
    pub fn crop_bgra(
        src: &[u8],
        src_w: usize,
        src_h: usize,
        crop: &CropRect,
        dst: &mut [u8],
        dst_w: usize,
        dst_h: usize,
    ) {
        if dst_w == 0 || dst_h == 0 || crop.width == 0 || crop.height == 0 {
            return;
        }

        let crop_x = crop.x as usize;
        let crop_y = crop.y as usize;
        let crop_w = crop.width as usize;
        let crop_h = crop.height as usize;

        for dy in 0..dst_h {
            let sy = crop_y + (dy * crop_h) / dst_h;
            let src_row = sy.min(src_h - 1) * src_w * 4;
            let dst_row = dy * dst_w * 4;

            for dx in 0..dst_w {
                let sx = crop_x + (dx * crop_w) / dst_w;
                let src_idx = src_row + sx.min(src_w - 1) * 4;
                let dst_idx = dst_row + dx * 4;

                dst[dst_idx] = src[src_idx];
                dst[dst_idx + 1] = src[src_idx + 1];
                dst[dst_idx + 2] = src[src_idx + 2];
                dst[dst_idx + 3] = src[src_idx + 3];
            }
        }
    }
}

/* ------------------------------------------------------------------ */
/* Unit Tests                                                         */
/* ------------------------------------------------------------------ */

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lut_natural_leaves_pixels_unchanged() {
        let mut pixels = vec![100, 150, 200, 255];
        let original = pixels.clone();
        apply_lut(&mut pixels, LutPreset::Natural);
        assert_eq!(pixels, original);
    }

    #[test]
    fn lut_monochrome_converts_to_luma() {
        let mut pixels = vec![0, 0, 255, 255]; // Pure red in BGRA: B=0, G=0, R=255
        apply_lut(&mut pixels, LutPreset::Monochrome);
        // BT.601 red weight is ~77/256 = 76 or 77
        assert_eq!(pixels[0], pixels[1]);
        assert_eq!(pixels[1], pixels[2]);
        assert_eq!(pixels[3], 255);
        assert!(pixels[0] > 70 && pixels[0] < 80);
    }

    #[test]
    fn lut_warm_and_cool_shift_channels() {
        let mut warm_px = vec![128, 128, 128, 255];
        apply_lut(&mut warm_px, LutPreset::WarmStudio);
        assert!(
            warm_px[2] > warm_px[0],
            "Red channel should exceed Blue in WarmStudio"
        );

        let mut cool_px = vec![128, 128, 128, 255];
        apply_lut(&mut cool_px, LutPreset::CoolCorporate);
        assert!(
            cool_px[0] > cool_px[2],
            "Blue channel should exceed Red in CoolCorporate"
        );
    }

    #[test]
    fn portrait_blur_preserves_center_focus() {
        let (w, h) = (32, 32);
        let mut bgra = vec![120u8; w * h * 4];
        // Set center pixel
        let center_idx = (16 * w + 16) * 4;
        bgra[center_idx] = 200;
        bgra[center_idx + 1] = 200;
        bgra[center_idx + 2] = 200;

        apply_portrait_blur(&mut bgra, w, h, BlurIntensity::Soft, 1.0);
        // Center pixel should remain unchanged or nearly unchanged (feathered focus)
        assert_eq!(bgra[center_idx], 200);
    }

    #[test]
    fn auto_framing_crop_computation() {
        let mut tracker = AutoFramingTracker::new(1.50, 0.50);
        let crop = tracker.compute_crop(1920, 1080);
        assert_eq!(crop.width, 1280);
        assert_eq!(crop.height, 720);
        assert_eq!(crop.x, 320);
        assert_eq!(crop.y, 180);

        // Pan to right
        tracker.set_target(0.80, 0.50);
        tracker.step();
        let panned = tracker.compute_crop(1920, 1080);
        assert!(panned.x > crop.x);
    }

    #[test]
    fn auto_framing_crop_rescale() {
        let src_w = 4;
        let src_h = 4;
        let mut src = vec![0u8; src_w * src_h * 4];
        for (i, p) in src.chunks_exact_mut(4).enumerate() {
            p[0] = i as u8;
            p[3] = 255;
        }

        let crop = CropRect {
            x: 1,
            y: 1,
            width: 2,
            height: 2,
        };
        let mut dst = vec![0u8; 2 * 2 * 4];
        AutoFramingTracker::crop_bgra(&src, src_w, src_h, &crop, &mut dst, 2, 2);
        assert_eq!(dst.len(), 16);
    }
}
