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
    #[serde(alias = "off")]
    None,
    Natural,
    WarmStudio,
    CoolCorporate,
    VibrantStreamer,
    Monochrome,
}

impl LutPreset {
    pub fn all() -> &'static [Self] {
        &[
            Self::None,
            Self::Natural,
            Self::WarmStudio,
            Self::CoolCorporate,
            Self::VibrantStreamer,
            Self::Monochrome,
        ]
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::None => "None / Off",
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
    if preset == LutPreset::None || preset == LutPreset::Natural {
        return;
    }

    match preset {
        LutPreset::None | LutPreset::Natural => {}
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
            Self::Soft => 4,
            Self::Studio => 8,
            Self::Dreamy => 14,
        }
    }

    pub fn default_amount(&self) -> f32 {
        match self {
            Self::Soft => 0.65,
            Self::Studio => 0.85,
            Self::Dreamy => 1.0,
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

/// Reusable scratch buffers for zero-allocation post-processing effects on the 60 FPS hot path.
#[derive(Debug, Clone, Default)]
pub struct EffectsScratch {
    blur_src: Vec<u8>,
    blur_tmp: Vec<u8>,
    col_sums: Vec<u32>,
    scale_x0: Vec<usize>,
    scale_x1: Vec<usize>,
    scale_wx: Vec<u16>,
}

impl EffectsScratch {
    /// Creates a new empty `EffectsScratch` buffer pool.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Applies a portrait bokeh effect to a BGRA buffer with zero per-frame allocations once
/// `scratch` is warmed up. Uses a separable O(W*H) sliding-window box blur and centers the
/// sharp foreground region around `face` when provided (falling back to `(0.50, 0.45)` when `None`).
pub fn apply_portrait_blur_with_scratch(
    bgra: &mut [u8],
    width: u32,
    height: u32,
    intensity: BlurIntensity,
    face: Option<crate::face_detect::FaceRect>,
    scratch: &mut EffectsScratch,
) {
    apply_portrait_blur_impl(
        bgra,
        width as usize,
        height as usize,
        intensity,
        intensity.default_amount(),
        face,
        scratch,
    );
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
    thread_local! {
        static LEGACY_SCRATCH: std::cell::RefCell<EffectsScratch> =
            std::cell::RefCell::new(EffectsScratch::new());
    }
    let handled = LEGACY_SCRATCH.with(|cell| {
        if let Ok(mut scratch) = cell.try_borrow_mut() {
            apply_portrait_blur_impl(bgra, width, height, intensity, amount, None, &mut scratch);
            true
        } else {
            false
        }
    });
    if !handled {
        let mut fallback = EffectsScratch::new();
        apply_portrait_blur_impl(bgra, width, height, intensity, amount, None, &mut fallback);
    }
}

fn apply_portrait_blur_impl(
    bgra: &mut [u8],
    width: usize,
    height: usize,
    intensity: BlurIntensity,
    amount: f32,
    face: Option<crate::face_detect::FaceRect>,
    scratch: &mut EffectsScratch,
) {
    if amount <= 0.001 || width == 0 || height == 0 {
        return;
    }
    let radius = intensity.radius();
    if radius == 0 {
        return;
    }

    let Some(total_pixels) = width.checked_mul(height) else {
        return;
    };
    let Some(byte_len) = total_pixels.checked_mul(4) else {
        return;
    };
    if bgra.len() < byte_len {
        return;
    }

    // Reuse scratch buffers without reallocating on steady-state frames.
    scratch.blur_src.resize(byte_len, 0);
    scratch.blur_src.copy_from_slice(&bgra[..byte_len]);
    scratch.blur_tmp.resize(byte_len, 0);
    scratch.col_sums.resize(width * 3, 0);

    let row_bytes = width * 4;
    let r = radius.min(width.max(height));

    // 1. Horizontal O(W*H) sliding-window box blur: blur_src -> blur_tmp
    for y in 0..height {
        let row_off = y * row_bytes;
        let src_row = &scratch.blur_src[row_off..row_off + row_bytes];
        let dst_row = &mut scratch.blur_tmp[row_off..row_off + row_bytes];

        let init_end = r.min(width - 1);
        let mut sum_b = 0u32;
        let mut sum_g = 0u32;
        let mut sum_r = 0u32;
        let mut count = (init_end + 1) as u32;

        for ix in 0..=init_end {
            let i = ix * 4;
            sum_b += src_row[i] as u32;
            sum_g += src_row[i + 1] as u32;
            sum_r += src_row[i + 2] as u32;
        }

        dst_row[0] = (sum_b / count) as u8;
        dst_row[1] = (sum_g / count) as u8;
        dst_row[2] = (sum_r / count) as u8;
        dst_row[3] = src_row[3];

        for x in 1..width {
            if let Some(x_add) = x.checked_add(r).filter(|&v| v < width) {
                let i = x_add * 4;
                sum_b += src_row[i] as u32;
                sum_g += src_row[i + 1] as u32;
                sum_r += src_row[i + 2] as u32;
                count += 1;
            }
            if x > r {
                let x_rem = x - r - 1;
                let i = x_rem * 4;
                sum_b -= src_row[i] as u32;
                sum_g -= src_row[i + 1] as u32;
                sum_r -= src_row[i + 2] as u32;
                count -= 1;
            }

            let out = x * 4;
            dst_row[out] = (sum_b / count) as u8;
            dst_row[out + 1] = (sum_g / count) as u8;
            dst_row[out + 2] = (sum_r / count) as u8;
            dst_row[out + 3] = src_row[out + 3];
        }
    }

    // 2. Vertical O(W*H) sliding-window box blur (row-major for L1 cache locality): blur_tmp -> blur_src
    let col_sums = &mut scratch.col_sums[..width * 3];
    col_sums.fill(0);
    let init_y_end = r.min(height - 1);
    let mut v_count = (init_y_end + 1) as u32;

    for iy in 0..=init_y_end {
        let src_row = &scratch.blur_tmp[iy * row_bytes..(iy + 1) * row_bytes];
        for x in 0..width {
            let si = x * 4;
            let ci = x * 3;
            col_sums[ci] += src_row[si] as u32;
            col_sums[ci + 1] += src_row[si + 1] as u32;
            col_sums[ci + 2] += src_row[si + 2] as u32;
        }
    }

    {
        let out_row = &mut scratch.blur_src[..row_bytes];
        for x in 0..width {
            let ci = x * 3;
            let oi = x * 4;
            out_row[oi] = (col_sums[ci] / v_count) as u8;
            out_row[oi + 1] = (col_sums[ci + 1] / v_count) as u8;
            out_row[oi + 2] = (col_sums[ci + 2] / v_count) as u8;
        }
    }

    for y in 1..height {
        if let Some(y_add) = y.checked_add(r).filter(|&v| v < height) {
            let add_row = &scratch.blur_tmp[y_add * row_bytes..(y_add + 1) * row_bytes];
            for x in 0..width {
                let si = x * 4;
                let ci = x * 3;
                col_sums[ci] += add_row[si] as u32;
                col_sums[ci + 1] += add_row[si + 1] as u32;
                col_sums[ci + 2] += add_row[si + 2] as u32;
            }
            v_count += 1;
        }
        if y > r {
            let y_rem = y - r - 1;
            let rem_row = &scratch.blur_tmp[y_rem * row_bytes..(y_rem + 1) * row_bytes];
            for x in 0..width {
                let si = x * 4;
                let ci = x * 3;
                col_sums[ci] -= rem_row[si] as u32;
                col_sums[ci + 1] -= rem_row[si + 1] as u32;
                col_sums[ci + 2] -= rem_row[si + 2] as u32;
            }
            v_count -= 1;
        }

        let out_row = &mut scratch.blur_src[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..width {
            let ci = x * 3;
            let oi = x * 4;
            out_row[oi] = (col_sums[ci] / v_count) as u8;
            out_row[oi + 1] = (col_sums[ci + 1] / v_count) as u8;
            out_row[oi + 2] = (col_sums[ci + 2] / v_count) as u8;
        }
    }

    // 3. Blend: Protected sharp region centered & scaled around `face` (fallback to (0.50, 0.45))
    let (center_x, center_y, semi_x, semi_y) = if let Some(f) = face {
        let cx = f.center_x.clamp(0.05, 0.95) * width as f32;
        let cy = (f.center_y + f.height * 0.20).clamp(0.05, 0.95) * height as f32;
        let sx = (f.width * 1.35).clamp(0.18, 0.42) * width as f32;
        let sy = (f.height * 1.75).clamp(0.24, 0.52) * height as f32;
        (cx, cy, sx, sy)
    } else {
        (
            width as f32 * 0.50,
            height as f32 * 0.45,
            width as f32 * 0.30,
            height as f32 * 0.40,
        )
    };

    let inv_semi_x = 1.0 / semi_x.max(1.0);
    let inv_semi_y = 1.0 / semi_y.max(1.0);
    let eff_amount = amount.clamp(0.0, 1.0);
    let blurred = &scratch.blur_src[..byte_len];

    const INNER_D: f32 = 0.85;
    const OUTER_D: f32 = 1.40;
    const INNER_D2: f32 = INNER_D * INNER_D;
    const OUTER_D2: f32 = OUTER_D * OUTER_D;
    const INV_SPAN: f32 = 1.0 / (OUTER_D - INNER_D);

    for y in 0..height {
        let dy = (y as f32 - center_y) * inv_semi_y;
        let dy2 = dy * dy;
        let row_offset = y * row_bytes;

        for x in 0..width {
            let dx = (x as f32 - center_x) * inv_semi_x;
            let d2 = dx * dx + dy2;

            if d2 <= INNER_D2 {
                continue;
            }

            let blend = if d2 >= OUTER_D2 {
                eff_amount
            } else {
                let d = d2.sqrt();
                let t = (d - INNER_D) * INV_SPAN;
                // Hermite smoothstep: 3*t^2 - 2*t^3
                (t * t * (3.0 - 2.0 * t)) * eff_amount
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
    #[serde(default)]
    pub target_zoom: f32,
    #[serde(default)]
    pub tracking_locked: bool,
    #[serde(default)]
    pub face_detected: bool,
    #[serde(default)]
    pub last_face: Option<crate::face_detect::FaceRect>,
}

impl Default for AutoFramingTracker {
    fn default() -> Self {
        Self::new(1.20, 0.12)
    }
}

impl AutoFramingTracker {
    pub fn new(zoom: f32, smoothing: f32) -> Self {
        let z = zoom.clamp(1.0, 2.5);
        Self {
            target_center_x: 0.50,
            target_center_y: 0.50,
            current_center_x: 0.50,
            current_center_y: 0.50,
            zoom: z,
            target_zoom: z,
            smoothing: smoothing.clamp(0.01, 1.0),
            tracking_locked: false,
            face_detected: false,
            last_face: None,
        }
    }

    /// Sets the target center normalized coordinates (0.0 to 1.0).
    pub fn set_target(&mut self, x: f32, y: f32) {
        if self.tracking_locked {
            return;
        }
        self.target_center_x = x.clamp(0.15, 0.85);
        self.target_center_y = y.clamp(0.15, 0.85);
    }

    /// Connects a detected face to the auto-framing target.
    ///
    /// When tracking is unlocked and a face is detected, updates `target_center_x`
    /// and `target_center_y` with deadband hysteresis to provide smooth, professional
    /// PTZ tracking without jitter.
    pub fn update_face(&mut self, face: Option<crate::face_detect::FaceRect>) {
        if let Some(f) = face {
            self.face_detected = true;
            self.last_face = Some(f);
            if !self.tracking_locked {
                // Apply 1.5% deadband to eliminate micro-jitter
                let dx = (f.center_x - self.target_center_x).abs();
                let dy = (f.center_y - self.target_center_y).abs();
                if dx > 0.015 || dy > 0.015 {
                    self.set_target(f.center_x, f.center_y);
                }
            }
        } else {
            self.face_detected = false;
        }
    }

    /// Toggles the tracking lock (pause/resume face tracking).
    pub fn toggle_tracking_lock(&mut self) -> bool {
        self.tracking_locked = !self.tracking_locked;
        self.tracking_locked
    }

    /// Sets the tracking lock state.
    pub fn set_tracking_locked(&mut self, locked: bool) {
        self.tracking_locked = locked;
    }

    /// Returns true if face tracking is locked.
    pub fn is_tracking_locked(&self) -> bool {
        self.tracking_locked
    }

    /// Sets the target zoom factor (1.0 to 2.5).
    pub fn set_zoom(&mut self, zoom: f32) {
        self.target_zoom = zoom.clamp(1.0, 2.5);
    }

    /// Toggles zoom between two levels (e.g. 1.0x and 2.0x).
    pub fn toggle_zoom(&mut self, low: f32, high: f32) -> f32 {
        let low = low.clamp(1.0, 2.5);
        let high = high.clamp(1.0, 2.5);
        if (self.zoom - low).abs() < (self.zoom - high).abs() {
            self.set_zoom(high);
            high
        } else {
            self.set_zoom(low);
            low
        }
    }

    /// Resets framing target to uncropped center (0.50, 0.50) and zoom to 1.0x.
    pub fn reset_framing(&mut self) {
        self.target_center_x = 0.50;
        self.target_center_y = 0.50;
        self.set_zoom(1.0);
        self.tracking_locked = false;
    }

    /// Handles a detected hand gesture event:
    /// - OpenPalm: toggles tracking lock/pause.
    /// - PeaceV: toggles zoom level between 1.0x and 2.0x.
    /// - IndexPoint: sets zoom to 1.5x medium zoom.
    /// - ShakaCall: resets framing and zoom to 1.0x uncropped center.
    /// - PinchOk: sets zoom to 2.2x macro close-up.
    /// - ThumbsUp: confirms or toggles zoom between 1.0x and 2.0x.
    ///
    /// Returns an optional action description string for UI feedback.
    pub fn handle_gesture(&mut self, event: &crate::gesture::GestureEvent) -> Option<&'static str> {
        if !event.triggered {
            return None;
        }
        match event.gesture {
            crate::gesture::HandGesture::OpenPalm => {
                let locked = self.toggle_tracking_lock();
                if locked {
                    Some("Tracking Locked")
                } else {
                    Some("Tracking Unlocked")
                }
            }
            crate::gesture::HandGesture::PeaceV => {
                self.toggle_zoom(1.0, 2.0);
                Some("Zoom Toggled (1.0x / 2.0x)")
            }
            crate::gesture::HandGesture::IndexPoint => {
                self.set_zoom(1.5);
                Some("☝ Index: 1.5x Zoom")
            }
            crate::gesture::HandGesture::ShakaCall => {
                self.reset_framing();
                Some("🤙 Shaka: Framing & Zoom Reset")
            }
            crate::gesture::HandGesture::PinchOk => {
                self.set_zoom(2.2);
                Some("🤏 Pinch: 2.2x Macro")
            }
            crate::gesture::HandGesture::ThumbsUp => {
                self.toggle_zoom(1.0, 2.0);
                Some("👍 Thumbs Up: Confirmed")
            }
            crate::gesture::HandGesture::None => None,
        }
    }

    /// Processes a video frame with face detection and updates the target tracking center.
    pub fn process_frame(
        &mut self,
        detector: &mut crate::face_detect::FaceDetector,
        bgra: &[u8],
        width: usize,
        height: usize,
    ) -> Option<crate::face_detect::FaceRect> {
        let face = detector.detect(bgra, width, height);
        self.update_face(face);
        face
    }

    /// Advances the smoothing interpolation toward the target center and target zoom
    /// assuming a nominal 30 FPS step (`dt_secs = 1.0 / 30.0`).
    pub fn step(&mut self) {
        self.step_with_dt(None, 1.0 / 30.0);
    }

    /// Frame-rate independent smoothing step with optional fresh face target and delta time in seconds.
    ///
    /// Normalizes `self.smoothing` (tuned at 30 FPS) via `alpha = 1 - (1 - s)^(dt * 30)`.
    pub fn step_with_dt(&mut self, face: Option<crate::face_detect::FaceRect>, dt_secs: f32) {
        if let Some(f) = face {
            self.update_face(Some(f));
        }

        let dt = dt_secs.clamp(0.0, 1.0);
        if dt <= 0.0 {
            return;
        }
        let alpha = if self.smoothing >= 1.0 {
            1.0
        } else {
            1.0 - (1.0 - self.smoothing).powf(dt * 30.0)
        };

        if !self.tracking_locked {
            self.current_center_x += (self.target_center_x - self.current_center_x) * alpha;
            self.current_center_y += (self.target_center_y - self.current_center_y) * alpha;
        }
        self.zoom += (self.target_zoom - self.zoom) * alpha;
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

    /// Crops and bilinear-rescales a BGRA source image into a destination buffer with zero
    /// per-frame allocations once `scratch` is warmed up.
    #[allow(clippy::too_many_arguments)]
    pub fn crop_bgra_with_scratch(
        src: &[u8],
        src_w: usize,
        src_h: usize,
        crop: &CropRect,
        dst: &mut [u8],
        dst_w: usize,
        dst_h: usize,
        scratch: &mut EffectsScratch,
    ) {
        if src_w == 0
            || src_h == 0
            || dst_w == 0
            || dst_h == 0
            || crop.width == 0
            || crop.height == 0
        {
            return;
        }
        let Some(src_bytes) = src_w.checked_mul(src_h).and_then(|p| p.checked_mul(4)) else {
            return;
        };
        let Some(dst_bytes) = dst_w.checked_mul(dst_h).and_then(|p| p.checked_mul(4)) else {
            return;
        };
        if src.len() < src_bytes || dst.len() < dst_bytes {
            return;
        }

        let crop_x = (crop.x as usize).min(src_w - 1);
        let crop_y = (crop.y as usize).min(src_h - 1);
        let crop_w = (crop.width as usize).min(src_w - crop_x);
        let crop_h = (crop.height as usize).min(src_h - crop_y);
        if crop_w == 0 || crop_h == 0 {
            return;
        }

        // Fast 1:1 crop copy when crop matches destination dimensions
        if crop_w == dst_w && crop_h == dst_h {
            let row_bytes = dst_w * 4;
            for dy in 0..dst_h {
                let s_off = ((crop_y + dy) * src_w + crop_x) * 4;
                let d_off = dy * row_bytes;
                dst[d_off..d_off + row_bytes].copy_from_slice(&src[s_off..s_off + row_bytes]);
            }
            return;
        }

        scratch.scale_x0.resize(dst_w, 0);
        scratch.scale_x1.resize(dst_w, 0);
        scratch.scale_wx.resize(dst_w, 0);

        let max_x = crop_x + crop_w - 1;
        let max_y = crop_y + crop_h - 1;
        let min_x_fp = (crop_x as i64) * 256;
        let max_x_fp = (max_x as i64) * 256;
        let min_y_fp = (crop_y as i64) * 256;
        let max_y_fp = (max_y as i64) * 256;

        for dx in 0..dst_w {
            let fx = (crop_x as i64) * 256
                + (((2 * dx as i64 + 1) * (crop_w as i64) * 256) / (2 * dst_w as i64))
                - 128;
            let clamped = fx.clamp(min_x_fp, max_x_fp);
            let x0 = (clamped >> 8) as usize;
            let wx = (clamped & 0xFF) as u16;
            let x1 = (x0 + 1).min(max_x);
            scratch.scale_x0[dx] = x0 * 4;
            scratch.scale_x1[dx] = x1 * 4;
            scratch.scale_wx[dx] = wx;
        }

        let src_row_bytes = src_w * 4;
        let dst_row_bytes = dst_w * 4;

        for dy in 0..dst_h {
            let fy = (crop_y as i64) * 256
                + (((2 * dy as i64 + 1) * (crop_h as i64) * 256) / (2 * dst_h as i64))
                - 128;
            let clamped_y = fy.clamp(min_y_fp, max_y_fp);
            let y0 = (clamped_y >> 8) as usize;
            let wy = (clamped_y & 0xFF) as u32;
            let inv_wy = 256 - wy;
            let y1 = (y0 + 1).min(max_y);

            let row0 = &src[y0 * src_row_bytes..(y0 + 1) * src_row_bytes];
            let row1 = &src[y1 * src_row_bytes..(y1 + 1) * src_row_bytes];
            let dst_row = &mut dst[dy * dst_row_bytes..(dy + 1) * dst_row_bytes];

            for dx in 0..dst_w {
                let x0_off = scratch.scale_x0[dx];
                let x1_off = scratch.scale_x1[dx];
                let wx = scratch.scale_wx[dx] as u32;
                let inv_wx = 256 - wx;

                let w00 = inv_wx * inv_wy;
                let w10 = wx * inv_wy;
                let w01 = inv_wx * wy;
                let w11 = wx * wy;

                let d_off = dx * 4;
                for c in 0..4 {
                    let v = row0[x0_off + c] as u32 * w00
                        + row0[x1_off + c] as u32 * w10
                        + row1[x0_off + c] as u32 * w01
                        + row1[x1_off + c] as u32 * w11;
                    dst_row[d_off + c] = ((v + 32_768) >> 16) as u8;
                }
            }
        }
    }

    /// Crops and bilinear-rescales a BGRA source image into a destination buffer.
    pub fn crop_bgra(
        src: &[u8],
        src_w: usize,
        src_h: usize,
        crop: &CropRect,
        dst: &mut [u8],
        dst_w: usize,
        dst_h: usize,
    ) {
        thread_local! {
            static CROP_SCRATCH: std::cell::RefCell<EffectsScratch> =
                std::cell::RefCell::new(EffectsScratch::new());
        }
        let handled = CROP_SCRATCH.with(|cell| {
            if let Ok(mut scratch) = cell.try_borrow_mut() {
                Self::crop_bgra_with_scratch(
                    src,
                    src_w,
                    src_h,
                    crop,
                    dst,
                    dst_w,
                    dst_h,
                    &mut scratch,
                );
                true
            } else {
                false
            }
        });
        if !handled {
            let mut fallback = EffectsScratch::new();
            Self::crop_bgra_with_scratch(src, src_w, src_h, crop, dst, dst_w, dst_h, &mut fallback);
        }
    }
}

/* ------------------------------------------------------------------ */
/* BGRA -> NV12 Color Conversion (Virtual Camera Parity)              */
/* ------------------------------------------------------------------ */

/// Converts a premultiplied BGRA buffer (`width * height * 4` bytes) into packed NV12
/// (`width * height * 3 / 2` bytes) so post-processed BGRA frames can be written directly
/// to the virtual camera framebus without preview/vcam drift.
///
/// Uses ITU-R BT.709 limited-range coefficients for HD (`width >= 1280 || height >= 720`)
/// and BT.601 limited-range coefficients for SD.
pub fn bgra_to_nv12(bgra: &[u8], width: u32, height: u32, out_nv12: &mut [u8]) {
    let w = width as usize;
    let h = height as usize;
    if w < 2 || h < 2 || (w & 1) != 0 || (h & 1) != 0 {
        return;
    }
    let Some(luma_len) = w.checked_mul(h) else {
        return;
    };
    let Some(bgra_len) = luma_len.checked_mul(4) else {
        return;
    };
    let nv12_len = luma_len + luma_len / 2;
    if bgra.len() < bgra_len || out_nv12.len() < nv12_len {
        return;
    }

    // 10-bit fixed-point limited-range coefficients (Y in [16, 235], UV in [16, 240]).
    let (yr, yg, yb, ur, ug, ub, vr, vg, vb) = if width >= 1280 || height >= 720 {
        // ITU-R BT.709
        (
            187i32, 629i32, 63i32, -103i32, -347i32, 450i32, 450i32, -409i32, -41i32,
        )
    } else {
        // ITU-R BT.601
        (
            263i32, 516i32, 100i32, -152i32, -298i32, 450i32, 450i32, -377i32, -73i32,
        )
    };

    let (y_plane, uv_plane) = out_nv12[..nv12_len].split_at_mut(luma_len);
    let row_bgra = w * 4;

    for py in 0..h {
        let src_row = &bgra[py * row_bgra..(py + 1) * row_bgra];
        let y_row = &mut y_plane[py * w..(py + 1) * w];
        for px in 0..w {
            let si = px * 4;
            let b = src_row[si] as i32;
            let g = src_row[si + 1] as i32;
            let r = src_row[si + 2] as i32;
            let y = ((yr * r + yg * g + yb * b + 512) >> 10) + 16;
            y_row[px] = y.clamp(16, 235) as u8;
        }
    }

    let pair_rows = h / 2;
    let pair_cols = w / 2;
    for cy in 0..pair_rows {
        let row0 = &bgra[(cy * 2) * row_bgra..(cy * 2 + 1) * row_bgra];
        let row1 = &bgra[(cy * 2 + 1) * row_bgra..(cy * 2 + 2) * row_bgra];
        let uv_row = &mut uv_plane[cy * w..(cy + 1) * w];

        for cx in 0..pair_cols {
            let i0 = cx * 8;
            let i1 = i0 + 4;
            let b_sum = row0[i0] as i32 + row0[i1] as i32 + row1[i0] as i32 + row1[i1] as i32;
            let g_sum = row0[i0 + 1] as i32
                + row0[i1 + 1] as i32
                + row1[i0 + 1] as i32
                + row1[i1 + 1] as i32;
            let r_sum = row0[i0 + 2] as i32
                + row0[i1 + 2] as i32
                + row1[i0 + 2] as i32
                + row1[i1 + 2] as i32;

            let b = (b_sum + 2) >> 2;
            let g = (g_sum + 2) >> 2;
            let r = (r_sum + 2) >> 2;

            let u = ((ur * r + ug * g + ub * b + 512) >> 10) + 128;
            let v = ((vr * r + vg * g + vb * b + 512) >> 10) + 128;

            uv_row[cx * 2] = u.clamp(16, 240) as u8;
            uv_row[cx * 2 + 1] = v.clamp(16, 240) as u8;
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
    fn lut_none_leaves_pixels_unchanged() {
        let mut pixels = vec![100, 150, 200, 255];
        let original = pixels.clone();
        apply_lut(&mut pixels, LutPreset::None);
        assert_eq!(pixels, original);
    }

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

    #[test]
    fn auto_framing_tracks_face_and_respects_lock() {
        let mut tracker = AutoFramingTracker::new(1.50, 0.50);
        let face = crate::face_detect::FaceRect::new(0.65, 0.40, 0.15, 0.20, 0.85);

        // Update with detected face
        tracker.update_face(Some(face));
        assert!(tracker.face_detected);
        assert_eq!(tracker.last_face, Some(face));
        assert!((tracker.target_center_x - face.center_x).abs() < 0.001);

        tracker.step();
        assert!(tracker.current_center_x > 0.50);

        // Lock tracking
        tracker.toggle_tracking_lock();
        assert!(tracker.is_tracking_locked());

        let new_face = crate::face_detect::FaceRect::new(0.20, 0.20, 0.15, 0.20, 0.85);
        tracker.update_face(Some(new_face));
        // Center should not change because tracking is locked
        assert_eq!(tracker.target_center_x, face.center_x);

        // Unlock tracking
        tracker.toggle_tracking_lock();
        tracker.update_face(Some(new_face));
        assert_eq!(tracker.target_center_x, new_face.center_x);
    }

    #[test]
    fn auto_framing_handles_gestures() {
        let mut tracker = AutoFramingTracker::new(1.0, 0.50);

        // Trigger Open Palm gesture -> toggles tracking lock
        let palm_event = crate::gesture::GestureEvent {
            gesture: crate::gesture::HandGesture::OpenPalm,
            confidence: 0.9,
            center_x: 0.5,
            center_y: 0.5,
            triggered: true,
        };
        let res = tracker.handle_gesture(&palm_event);
        assert_eq!(res, Some("Tracking Locked"));
        assert!(tracker.is_tracking_locked());

        // Trigger again -> unlocks
        let res2 = tracker.handle_gesture(&palm_event);
        assert_eq!(res2, Some("Tracking Unlocked"));
        assert!(!tracker.is_tracking_locked());

        // Trigger Peace V gesture -> toggles zoom
        let peace_event = crate::gesture::GestureEvent {
            gesture: crate::gesture::HandGesture::PeaceV,
            confidence: 0.9,
            center_x: 0.5,
            center_y: 0.5,
            triggered: true,
        };
        let zoom_res = tracker.handle_gesture(&peace_event);
        assert_eq!(zoom_res, Some("Zoom Toggled (1.0x / 2.0x)"));
        assert_eq!(tracker.target_zoom, 2.0);

        // Trigger Index Point gesture -> sets zoom to 1.5x
        let index_event = crate::gesture::GestureEvent {
            gesture: crate::gesture::HandGesture::IndexPoint,
            confidence: 0.9,
            center_x: 0.5,
            center_y: 0.5,
            triggered: true,
        };
        let idx_res = tracker.handle_gesture(&index_event);
        assert_eq!(idx_res, Some("☝ Index: 1.5x Zoom"));
        assert_eq!(tracker.target_zoom, 1.5);

        // Trigger Pinch / OK gesture -> sets zoom to 2.2x macro
        let pinch_event = crate::gesture::GestureEvent {
            gesture: crate::gesture::HandGesture::PinchOk,
            confidence: 0.9,
            center_x: 0.5,
            center_y: 0.5,
            triggered: true,
        };
        let pinch_res = tracker.handle_gesture(&pinch_event);
        assert_eq!(pinch_res, Some("🤏 Pinch: 2.2x Macro"));
        assert_eq!(tracker.target_zoom, 2.2);

        // Offset target and lock tracking, then trigger Shaka -> resets to center 1.0x and unlocks
        tracker.set_target(0.80, 0.70);
        tracker.set_tracking_locked(true);
        let shaka_event = crate::gesture::GestureEvent {
            gesture: crate::gesture::HandGesture::ShakaCall,
            confidence: 0.9,
            center_x: 0.5,
            center_y: 0.5,
            triggered: true,
        };
        let shaka_res = tracker.handle_gesture(&shaka_event);
        assert_eq!(shaka_res, Some("🤙 Shaka: Framing & Zoom Reset"));
        assert_eq!(tracker.target_center_x, 0.50);
        assert_eq!(tracker.target_center_y, 0.50);
        assert_eq!(tracker.target_zoom, 1.0);
        assert!(!tracker.is_tracking_locked());

        // Trigger Thumbs Up -> confirms / toggles zoom
        let thumbs_event = crate::gesture::GestureEvent {
            gesture: crate::gesture::HandGesture::ThumbsUp,
            confidence: 0.9,
            center_x: 0.5,
            center_y: 0.5,
            triggered: true,
        };
        let thumb_res = tracker.handle_gesture(&thumbs_event);
        assert_eq!(thumb_res, Some("👍 Thumbs Up: Confirmed"));
        assert_eq!(tracker.target_zoom, 2.0);

        // Untriggered event should do nothing
        let mut inactive = peace_event;
        inactive.triggered = false;
        assert_eq!(tracker.handle_gesture(&inactive), None);
    }

    #[test]
    fn portrait_blur_with_scratch_tracks_face_and_differentiates_intensities() {
        let (w, h) = (64u32, 48u32);
        let mut base = vec![0u8; (w * h * 4) as usize];
        // Checkerboard pattern across the whole frame so blur visibly lowers contrast.
        for y in 0..h as usize {
            for x in 0..w as usize {
                let val = if (x + y) % 2 == 0 { 220 } else { 40 };
                let idx = (y * w as usize + x) * 4;
                base[idx] = val;
                base[idx + 1] = val;
                base[idx + 2] = val;
                base[idx + 3] = 255;
            }
        }

        // Face placed in top-left quadrant (0.25, 0.25)
        let face = crate::face_detect::FaceRect::new(0.15, 0.15, 0.20, 0.20, 0.9);
        let mut scratch = EffectsScratch::new();

        let mut soft_buf = base.clone();
        apply_portrait_blur_with_scratch(
            &mut soft_buf,
            w,
            h,
            BlurIntensity::Soft,
            Some(face),
            &mut scratch,
        );
        // Warmed scratch pointer/capacity stability on subsequent call
        let cap_before = scratch.blur_src.capacity();
        let ptr_before = scratch.blur_src.as_ptr();

        let mut dreamy_buf = base.clone();
        apply_portrait_blur_with_scratch(
            &mut dreamy_buf,
            w,
            h,
            BlurIntensity::Dreamy,
            Some(face),
            &mut scratch,
        );
        assert_eq!(scratch.blur_src.capacity(), cap_before);
        assert_eq!(scratch.blur_src.as_ptr(), ptr_before);

        // Pixel inside face center (16, 12) must remain sharp (220 or 40 unchanged)
        let face_px = (12 * w as usize + 16) * 4;
        assert_eq!(dreamy_buf[face_px], base[face_px]);

        // Pixel far in bottom-right background (56, 40) must be blurred toward ~130
        let bg_px = (40 * w as usize + 56) * 4;
        let orig_diff = (base[bg_px] as i32 - 130).abs();
        let soft_diff = (soft_buf[bg_px] as i32 - 130).abs();
        let dreamy_diff = (dreamy_buf[bg_px] as i32 - 130).abs();
        assert!(soft_diff < orig_diff, "Soft blur should soften background");
        assert!(
            dreamy_diff < soft_diff,
            "Dreamy blur ({dreamy_diff}) should blur more strongly than Soft ({soft_diff})"
        );
    }

    #[test]
    fn auto_framing_step_with_dt_is_framerate_independent() {
        let mut t30 = AutoFramingTracker::new(1.0, 0.20);
        let mut t60 = AutoFramingTracker::new(1.0, 0.20);
        let face = crate::face_detect::FaceRect::new(0.70, 0.30, 0.10, 0.15, 0.9);

        t30.set_zoom(2.0);
        t60.set_zoom(2.0);
        t30.step_with_dt(Some(face), 1.0 / 30.0);

        // Two 60 FPS steps equal one 30 FPS step
        t60.step_with_dt(Some(face), 1.0 / 60.0);
        t60.step_with_dt(None, 1.0 / 60.0);

        assert!((t30.current_center_x - t60.current_center_x).abs() < 1e-5);
        assert!((t30.current_center_y - t60.current_center_y).abs() < 1e-5);
        assert!((t30.zoom - t60.zoom).abs() < 1e-5);
    }

    #[test]
    fn crop_bgra_bilinear_interpolates_smoothly() {
        // 2x2 horizontal gradient: [0, 200] on both rows
        let src = vec![
            0, 0, 0, 255, 200, 200, 200, 255, // row 0
            0, 0, 0, 255, 200, 200, 200, 255, // row 1
        ];
        let crop = CropRect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        };
        let mut dst = vec![0u8; 4 * 4 * 4];
        let mut scratch = EffectsScratch::new();
        AutoFramingTracker::crop_bgra_with_scratch(&src, 2, 2, &crop, &mut dst, 4, 4, &mut scratch);

        // Inner columns (dx=1, dx=2) must be smoothly interpolated between 0 and 200 (50 and 150)
        assert_eq!(dst[0], 0);
        assert_eq!(dst[4], 50);
        assert_eq!(dst[8], 150);
        assert_eq!(dst[12], 200);
    }

    #[test]
    fn bgra_to_nv12_parity_bt601_and_bt709() {
        // 1. SD (BT.601): 4x4 pure red and pure green blocks
        let (sw, sh) = (4u32, 4u32);
        let mut sd_bgra = vec![0u8; (sw * sh * 4) as usize];
        for (i, px) in sd_bgra.chunks_exact_mut(4).enumerate() {
            if i < 8 {
                // Pure Red (BGRA)
                px.copy_from_slice(&[0, 0, 255, 255]);
            } else {
                // Neutral gray
                px.copy_from_slice(&[128, 128, 128, 255]);
            }
        }
        let mut sd_nv12 = vec![0u8; (sw * sh * 3 / 2) as usize];
        bgra_to_nv12(&sd_bgra, sw, sh, &mut sd_nv12);

        // Round-trip back via write_planes_bgra (SD -> BT.601)
        let luma_len = (sw * sh) as usize;
        let cw = (sw / 2) as usize;
        let ch = (sh / 2) as usize;
        let mut u_plane = Vec::with_capacity(cw * ch);
        let mut v_plane = Vec::with_capacity(cw * ch);
        for pair in sd_nv12[luma_len..].chunks_exact(2) {
            u_plane.push(pair[0]);
            v_plane.push(pair[1]);
        }
        let planes = crate::YuvPlanes {
            width: sw,
            height: sh,
            y: &sd_nv12[..luma_len],
            u: &u_plane,
            v: &v_plane,
            y_stride: sw as usize,
            u_stride: cw,
            v_stride: cw,
        };
        let mut rt_bgra = vec![0u8; sd_bgra.len()];
        assert!(crate::write_planes_bgra(&planes, &mut rt_bgra, sw as usize));
        for i in 0..sd_bgra.len() {
            let diff = (sd_bgra[i] as i32 - rt_bgra[i] as i32).abs();
            assert!(diff <= 3, "SD round-trip drift at byte {i}: {diff}");
        }

        // 2. HD (BT.709): 1280x720 pure red -> Y ~63, U ~102, V ~240 (distinct from BT.601 Y ~81, U ~90, V ~240)
        let (hw, hh) = (1280u32, 720u32);
        let mut hd_bgra = vec![0u8; (hw * hh * 4) as usize];
        for px in hd_bgra.chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 0, 255, 255]);
        }
        let mut hd_nv12 = vec![0u8; (hw * hh * 3 / 2) as usize];
        bgra_to_nv12(&hd_bgra, hw, hh, &mut hd_nv12);
        // BT.709 pure red luma is ~63 (BT.601 is ~81)
        assert!(
            (hd_nv12[0] as i32 - 63).abs() <= 2,
            "Expected BT.709 red luma ~63, got {}",
            hd_nv12[0]
        );
        // Round-trip HD via write_planes_bgra (HD -> BT.709)
        let h_luma = (hw * hh) as usize;
        let h_cw = (hw / 2) as usize;
        let h_ch = (hh / 2) as usize;
        let mut hu = Vec::with_capacity(h_cw * h_ch);
        let mut hv = Vec::with_capacity(h_cw * h_ch);
        for pair in hd_nv12[h_luma..].chunks_exact(2) {
            hu.push(pair[0]);
            hv.push(pair[1]);
        }
        let h_planes = crate::YuvPlanes {
            width: hw,
            height: hh,
            y: &hd_nv12[..h_luma],
            u: &hu,
            v: &hv,
            y_stride: hw as usize,
            u_stride: h_cw,
            v_stride: h_cw,
        };
        let mut h_rt = vec![0u8; hd_bgra.len()];
        assert!(crate::write_planes_bgra(&h_planes, &mut h_rt, hw as usize));
        assert!(h_rt[2] >= 252 && h_rt[1] <= 3 && h_rt[0] <= 3);
    }
}
