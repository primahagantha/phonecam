//! Real-time OBSBOT-style Hand Gesture Recognition in pure Rust.
//!
//! Recognizes:
//! - Open Palm gesture (✋): Toggles tracking lock/pause.
//! - Peace / V-Sign gesture (✌): Toggles zoom level (1.0x <-> 2.0x).
//! - Single Index Pointing Up (☝): Sets zoom to 1.5x medium zoom.
//! - Shaka / Hang Loose (🤙): Resets framing to center (0.50, 0.50) and zoom to 1.0x.
//! - Pinch / OK Sign (🤏): Sets zoom to 2.2x close-up macro focal target.
//! - Thumbs Up gesture (👍): Quick confirm / toggles zoom.
//!
//! Guarantees zero heap allocations on the 60 FPS hot path.

use crate::face_detect::{is_skin_rgb, FaceRect, MAX_GRID_CELLS, MAX_GRID_H, MAX_GRID_W};
use serde::{Deserialize, Serialize};

/// Supported hand gesture classifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HandGesture {
    #[default]
    None,
    /// Open Palm gesture (✋) - toggles target tracking lock / unlock.
    OpenPalm,
    /// Peace / V-Sign gesture (✌) - toggles zoom level (1.0x <-> 2.0x).
    PeaceV,
    /// Single index finger pointing up (☝) - sets zoom to 1.5x.
    IndexPoint,
    /// Shaka / Hang Loose gesture (🤙) - resets framing to center (0.50, 0.50) and zoom to 1.0x.
    ShakaCall,
    /// Pinch / OK sign (🤏) - sets zoom to 2.2x macro close-up.
    PinchOk,
    /// Thumbs Up gesture (👍) - quick confirm / toggles zoom.
    ThumbsUp,
}

impl HandGesture {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::OpenPalm => "Open Palm (✋)",
            Self::PeaceV => "Peace / V-Sign (✌)",
            Self::IndexPoint => "Pointing Up (☝)",
            Self::ShakaCall => "Shaka / Call (🤙)",
            Self::PinchOk => "Pinch / OK (🤏)",
            Self::ThumbsUp => "Thumbs Up (👍)",
        }
    }
}

/// Gesture recognition event output.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GestureEvent {
    pub gesture: HandGesture,
    pub confidence: f32,
    pub center_x: f32,
    pub center_y: f32,
    /// True on the exact frame the gesture is confirmed (rising edge trigger),
    /// suitable for triggering toggle actions without rapid repetition.
    pub triggered: bool,
}

impl Default for GestureEvent {
    fn default() -> Self {
        Self {
            gesture: HandGesture::None,
            confidence: 0.0,
            center_x: 0.5,
            center_y: 0.5,
            triggered: false,
        }
    }
}

/// Internal candidate hand region prior to classification.
#[derive(Debug, Clone, Copy, Default)]
struct HandCandidate {
    min_gx: usize,
    max_gx: usize,
    min_gy: usize,
    max_gy: usize,
    pixel_count: usize,
}

/// Fast, zero-allocation Hand Gesture Detector.
#[derive(Debug, Clone)]
pub struct GestureDetector {
    skin_mask: Vec<u8>,
    filtered_mask: Vec<u8>,
    visited: Vec<bool>,
    queue: Vec<usize>,
    candidates: Vec<HandCandidate>,
    proj_x: [u16; MAX_GRID_W],
    first_skin_x: [usize; MAX_GRID_H],
    last_skin_x: [usize; MAX_GRID_H],
    first_skin_y: [usize; MAX_GRID_W],
    last_skin_y: [usize; MAX_GRID_W],

    pub debounce_frames: usize,
    pub cooldown_frames: usize,

    current_candidate: HandGesture,
    consecutive_count: usize,
    cooldown_remaining: usize,
}

impl Default for GestureDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl GestureDetector {
    /// Creates a new `GestureDetector` with pre-allocated scratch buffers.
    pub fn new() -> Self {
        Self {
            skin_mask: vec![0u8; MAX_GRID_CELLS],
            filtered_mask: vec![0u8; MAX_GRID_CELLS],
            visited: vec![false; MAX_GRID_CELLS],
            queue: Vec::with_capacity(MAX_GRID_CELLS),
            candidates: Vec::with_capacity(32),
            proj_x: [0u16; MAX_GRID_W],
            first_skin_x: [0usize; MAX_GRID_H],
            last_skin_x: [0usize; MAX_GRID_H],
            first_skin_y: [0usize; MAX_GRID_W],
            last_skin_y: [0usize; MAX_GRID_W],

            debounce_frames: 7,
            cooldown_frames: 45, // ~0.75 - 1.5 seconds at 30-60 FPS

            current_candidate: HandGesture::None,
            consecutive_count: 0,
            cooldown_remaining: 0,
        }
    }

    /// Resets gesture detection state and debouncers.
    pub fn reset(&mut self) {
        self.current_candidate = HandGesture::None;
        self.consecutive_count = 0;
        self.cooldown_remaining = 0;
    }

    /// Detects hand gestures in a premultiplied BGRA frame, masking out the detected face.
    ///
    /// Guarantees zero heap allocations when called repeatedly at 60 FPS.
    pub fn detect(
        &mut self,
        bgra: &[u8],
        width: usize,
        height: usize,
        face: Option<FaceRect>,
    ) -> GestureEvent {
        if width == 0 || height == 0 || bgra.len() < width * height * 4 {
            if self.cooldown_remaining > 0 {
                self.cooldown_remaining -= 1;
            }
            return GestureEvent::default();
        }

        if self.cooldown_remaining > 0 {
            self.cooldown_remaining -= 1;
        }

        // Determine downscaled grid dimensions clamped strictly within [MAX_GRID_W, MAX_GRID_H]
        let (grid_w, grid_h) = if width >= height {
            let gw = MAX_GRID_W;
            let gh = ((height * MAX_GRID_W) / width).clamp(45, MAX_GRID_H);
            (gw, gh)
        } else {
            let gh = MAX_GRID_H;
            let gw = ((width * MAX_GRID_H) / height).clamp(45, MAX_GRID_W);
            (gw, gh)
        };
        let grid_w = grid_w.min(MAX_GRID_W);
        let grid_h = grid_h.min(MAX_GRID_H);

        let grid_cells = grid_w * grid_h;
        if grid_cells > MAX_GRID_CELLS {
            return GestureEvent::default();
        }

        // Compute face mask in grid coordinates to exclude face from hand detection
        let (face_gx0, face_gx1, face_gy0, face_gy1) = if let Some(f) = face {
            let gx0 = ((f.x - 0.04).max(0.0) * grid_w as f32) as usize;
            let gx1 = (((f.x + f.width + 0.04).min(1.0)) * grid_w as f32) as usize;
            let gy0 = ((f.y - 0.04).max(0.0) * grid_h as f32) as usize;
            let gy1 = (((f.y + f.height + 0.08).min(1.0)) * grid_h as f32) as usize;
            (gx0, gx1, gy0, gy1)
        } else {
            (usize::MAX, usize::MAX, usize::MAX, usize::MAX)
        };

        let step_x = width as f32 / grid_w as f32;
        let step_y = height as f32 / grid_h as f32;

        // 1. Skin-chrominance segmentation masking out face area
        for gy in 0..grid_h {
            let in_face_y = gy >= face_gy0 && gy <= face_gy1;
            let py = ((gy as f32 + 0.5) * step_y) as usize;
            let row_offset = py.min(height - 1) * width * 4;
            let grid_row = gy * grid_w;

            for gx in 0..grid_w {
                let cell_idx = grid_row + gx;
                if in_face_y && gx >= face_gx0 && gx <= face_gx1 {
                    self.skin_mask[cell_idx] = 0;
                    continue;
                }

                let px = ((gx as f32 + 0.5) * step_x) as usize;
                let px_idx = row_offset + px.min(width - 1) * 4;

                let b = bgra[px_idx];
                let g = bgra[px_idx + 1];
                let r = bgra[px_idx + 2];

                let (is_skin, _) = is_skin_rgb(r, g, b);
                self.skin_mask[cell_idx] = if is_skin { 1 } else { 0 };
            }
        }

        // 2. Morphological filter: clean isolated 1-cell noise while preserving slender fingers
        for gy in 0..grid_h {
            let row = gy * grid_w;
            for gx in 0..grid_w {
                let idx = row + gx;
                if gx == 0 || gx == grid_w - 1 || gy == 0 || gy == grid_h - 1 {
                    self.filtered_mask[idx] = self.skin_mask[idx];
                    continue;
                }

                let mut neighbors = 0u8;
                for dy in [-1, 0, 1] {
                    let n_row = (gy as isize + dy) as usize * grid_w;
                    for dx in [-1, 0, 1] {
                        let n_idx = n_row + (gx as isize + dx) as usize;
                        neighbors += self.skin_mask[n_idx];
                    }
                }

                if self.skin_mask[idx] == 1 {
                    self.filtered_mask[idx] = if neighbors >= 2 { 1 } else { 0 };
                } else {
                    self.filtered_mask[idx] = if neighbors >= 6 { 1 } else { 0 };
                }
            }
        }

        // 3. Connected components blob extraction
        self.visited[..grid_cells].fill(false);
        self.candidates.clear();

        for start_gy in 0..grid_h {
            for start_gx in 0..grid_w {
                let start_idx = start_gy * grid_w + start_gx;
                if self.filtered_mask[start_idx] == 0 || self.visited[start_idx] {
                    continue;
                }

                self.queue.clear();
                self.queue.push(start_idx);
                self.visited[start_idx] = true;

                let mut head = 0;
                let mut min_gx = start_gx;
                let mut max_gx = start_gx;
                let mut min_gy = start_gy;
                let mut max_gy = start_gy;
                let mut pixel_count = 0usize;

                while head < self.queue.len() {
                    let curr = self.queue[head];
                    head += 1;
                    let cy = curr / grid_w;
                    let cx = curr % grid_w;

                    pixel_count += 1;
                    min_gx = min_gx.min(cx);
                    max_gx = max_gx.max(cx);
                    min_gy = min_gy.min(cy);
                    max_gy = max_gy.max(cy);

                    if cx > 0 {
                        let n = curr - 1;
                        if self.filtered_mask[n] == 1 && !self.visited[n] {
                            self.visited[n] = true;
                            self.queue.push(n);
                        }
                    }
                    if cx + 1 < grid_w {
                        let n = curr + 1;
                        if self.filtered_mask[n] == 1 && !self.visited[n] {
                            self.visited[n] = true;
                            self.queue.push(n);
                        }
                    }
                    if cy > 0 {
                        let n = curr - grid_w;
                        if self.filtered_mask[n] == 1 && !self.visited[n] {
                            self.visited[n] = true;
                            self.queue.push(n);
                        }
                    }
                    if cy + 1 < grid_h {
                        let n = curr + grid_w;
                        if self.filtered_mask[n] == 1 && !self.visited[n] {
                            self.visited[n] = true;
                            self.queue.push(n);
                        }
                    }
                }

                let bw = max_gx - min_gx + 1;
                let bh = max_gy - min_gy + 1;

                // Minimum candidate hand size: at least 3% of grid dimensions, >= 12 pixels
                let min_dim = (grid_w.min(grid_h) / 30).max(3);
                if bw >= min_dim && bh >= min_dim && pixel_count >= 12 {
                    let aspect = bh as f32 / bw as f32;
                    let fill = pixel_count as f32 / (bw * bh) as f32;

                    if (0.50..=4.00).contains(&aspect) && (0.12..=0.95).contains(&fill) {
                        self.candidates.push(HandCandidate {
                            min_gx,
                            max_gx,
                            min_gy,
                            max_gy,
                            pixel_count,
                        });
                    }
                }
            }
        }

        // 4. Classify gesture candidates with high precision
        let mut best_gesture = HandGesture::None;
        let mut best_conf = 0.0f32;
        let mut best_cx = 0.5f32;
        let mut best_cy = 0.5f32;

        for cand in &self.candidates {
            let bw = cand.max_gx - cand.min_gx + 1;
            let bh = cand.max_gy - cand.min_gy + 1;
            let aspect = bh as f32 / bw as f32;
            let fill = cand.pixel_count as f32 / (bw * bh) as f32;

            let mid_gy = cand.min_gy + bh / 2;

            // Compute upper and lower pixel counts and upper vertical projection
            self.proj_x[..bw].fill(0);
            let mut upper_pixels = 0u32;
            let mut lower_pixels = 0u32;

            for gy in cand.min_gy..=cand.max_gy {
                let row = gy * grid_w;
                let is_upper = gy <= mid_gy;
                for gx in cand.min_gx..=cand.max_gx {
                    if self.filtered_mask[row + gx] == 1 {
                        if is_upper {
                            upper_pixels += 1;
                            self.proj_x[gx - cand.min_gx] += 1;
                        } else {
                            lower_pixels += 1;
                        }
                    }
                }
            }

            let upper_h = (mid_gy - cand.min_gy + 1) as f32;
            let lower_h = (cand.max_gy - mid_gy) as f32;
            let lower_fill = if lower_h > 0.0 {
                lower_pixels as f32 / (bw as f32 * lower_h)
            } else {
                0.0
            };
            let upper_fill = if upper_h > 0.0 {
                upper_pixels as f32 / (bw as f32 * upper_h)
            } else {
                0.0
            };

            // Detect topological hole (loop enclosed by skin pixels) for Pinch / OK
            let bh_clamped = bh.min(MAX_GRID_H);
            let bw_clamped = bw.min(MAX_GRID_W);
            self.first_skin_x[..bh_clamped].fill(usize::MAX);
            self.last_skin_x[..bh_clamped].fill(0);
            self.first_skin_y[..bw_clamped].fill(usize::MAX);
            self.last_skin_y[..bw_clamped].fill(0);

            for gy in cand.min_gy..=cand.max_gy {
                let y_off = gy - cand.min_gy;
                if y_off >= MAX_GRID_H {
                    continue;
                }
                let row = gy * grid_w;
                for gx in cand.min_gx..=cand.max_gx {
                    let x_off = gx - cand.min_gx;
                    if x_off >= MAX_GRID_W {
                        continue;
                    }
                    if self.filtered_mask[row + gx] == 1 {
                        if self.first_skin_x[y_off] == usize::MAX {
                            self.first_skin_x[y_off] = x_off;
                        }
                        self.last_skin_x[y_off] = x_off;

                        if self.first_skin_y[x_off] == usize::MAX {
                            self.first_skin_y[x_off] = y_off;
                        }
                        self.last_skin_y[x_off] = y_off;
                    }
                }
            }

            let mut enclosed_hole_cells = 0usize;
            if bw >= 4 && bh >= 4 {
                for gy in (cand.min_gy + 1)..cand.max_gy {
                    let y_off = gy - cand.min_gy;
                    if y_off >= MAX_GRID_H {
                        continue;
                    }
                    let first_x = self.first_skin_x[y_off];
                    let last_x = self.last_skin_x[y_off];
                    if first_x == usize::MAX || first_x >= last_x {
                        continue;
                    }

                    let row = gy * grid_w;
                    for gx in (cand.min_gx + 1)..cand.max_gx {
                        let x_off = gx - cand.min_gx;
                        if x_off >= MAX_GRID_W {
                            continue;
                        }
                        let first_y = self.first_skin_y[x_off];
                        let last_y = self.last_skin_y[x_off];
                        if first_y == usize::MAX || first_y >= last_y {
                            continue;
                        }

                        if self.filtered_mask[row + gx] == 0
                            && x_off > first_x
                            && x_off < last_x
                            && y_off > first_y
                            && y_off < last_y
                        {
                            enclosed_hole_cells += 1;
                        }
                    }
                }
            }

            // Extract finger peaks in upper half projection using valley-aware peak detection
            let peak_threshold = (upper_h * 0.28).max(2.0) as u16;
            let mut raw_peaks = [0usize; 8];
            let mut raw_count = 0usize;

            let mut seg_start = 0;
            while seg_start < bw {
                if self.proj_x[seg_start] >= peak_threshold {
                    let mut p_val = self.proj_x[seg_start];
                    let mut p_pos = seg_start;
                    let mut k = seg_start + 1;
                    // Ascend to local maximum
                    while k < bw && self.proj_x[k] >= self.proj_x[k - 1] {
                        p_val = self.proj_x[k];
                        p_pos = k;
                        k += 1;
                    }
                    // Descend to local minimum
                    while k < bw
                        && self.proj_x[k] >= peak_threshold
                        && self.proj_x[k] <= self.proj_x[k - 1]
                    {
                        k += 1;
                    }
                    if raw_count < 8 && p_val >= peak_threshold {
                        raw_peaks[raw_count] = p_pos;
                        raw_count += 1;
                    }
                    seg_start = k;
                } else {
                    seg_start += 1;
                }
            }

            // Filter/merge peaks by validating valley depth between them
            let mut peaks = 0usize;
            let mut peak_pos = [0usize; 8];
            let mut peak_val = [0u16; 8];
            let mut peak_w = [0usize; 8];

            for p_idx in 0..raw_count {
                let curr_pos = raw_peaks[p_idx];
                let curr_val = self.proj_x[curr_pos];

                if peaks == 0 {
                    peak_pos[0] = curr_pos;
                    peak_val[0] = curr_val;
                    peaks = 1;
                } else {
                    let prev_pos = peak_pos[peaks - 1];
                    let prev_val = peak_val[peaks - 1];

                    // Measure minimum valley between prev_pos and curr_pos
                    let mut min_valley = u16::MAX;
                    for vx in prev_pos..=curr_pos {
                        min_valley = min_valley.min(self.proj_x[vx]);
                    }

                    let min_peak = prev_val.min(curr_val);
                    // A distinct finger peak requires a valley dip of at least 25%
                    if (min_valley as f32) <= (min_peak as f32 * 0.75).max(min_peak as f32 - 2.0) {
                        if peaks < 8 {
                            peak_pos[peaks] = curr_pos;
                            peak_val[peaks] = curr_val;
                            peaks += 1;
                        }
                    } else if curr_val > prev_val {
                        // Merge into higher peak
                        peak_pos[peaks - 1] = curr_pos;
                        peak_val[peaks - 1] = curr_val;
                    }
                }
            }

            // Compute peak widths
            for p in 0..peaks {
                let pos = peak_pos[p];
                let half_val = peak_val[p] / 2;
                let mut left = pos;
                while left > 0 && self.proj_x[left] >= half_val {
                    left -= 1;
                }
                let mut right = pos;
                while right + 1 < bw && self.proj_x[right] >= half_val {
                    right += 1;
                }
                peak_w[p] = right - left + 1;
            }

            let mut gesture = HandGesture::None;
            let mut confidence = 0.0f32;

            // Classify with priority and precision:
            //
            // 1. Pinch / OK Sign (🤏)
            // Presence of meaningful enclosed hole (circle loop) between thumb and index finger
            let bbox_area = bw * bh;
            let min_hole_cells = (cand.pixel_count / 20).max(6);
            let hole_to_bbox = enclosed_hole_cells as f32 / bbox_area.max(1) as f32;
            if enclosed_hole_cells >= min_hole_cells
                && (0.04..=0.55).contains(&hole_to_bbox)
                && (0.65..=2.30).contains(&aspect)
                && lower_fill >= 0.25
                && fill >= 0.25
            {
                let hole_score = (enclosed_hole_cells as f32 / 16.0).clamp(0.4, 1.0);
                let fill_score = (1.0 - ((fill - 0.50).abs() / 0.40)).clamp(0.2, 1.0);
                confidence = (hole_score * 0.6 + fill_score * 0.4).clamp(0.0, 1.0);
                gesture = HandGesture::PinchOk;
            }
            // 2. Open Palm (✋)
            // Wide hand (aspect 0.65..1.65), solid palm base, at least 3 spread fingers
            else if (0.65..=1.65).contains(&aspect)
                && (0.28..=0.85).contains(&fill)
                && lower_fill >= 0.35
                && peaks >= 3
            {
                let aspect_score = 1.0 - ((aspect - 1.10).abs() / 0.7).min(1.0);
                let fill_score = 1.0 - ((fill - 0.55).abs() / 0.35).min(1.0);
                let palm_score = lower_fill.min(1.0);
                let finger_score = (upper_fill * 0.5 + (peaks as f32 / 5.0) * 0.5).clamp(0.0, 1.0);
                confidence = (aspect_score * 0.30
                    + fill_score * 0.30
                    + palm_score * 0.20
                    + finger_score * 0.20)
                    .clamp(0.0, 1.0);
                gesture = HandGesture::OpenPalm;
            }
            // 3. Shaka / Hang Loose (🤙)
            // Thumb and Pinky extended on opposite lateral edges, middle fingers folded
            // Wide/square aspect 0.55..1.60, 2 peaks with one at left edge and one at right edge
            else if (0.55..=1.65).contains(&aspect)
                && lower_fill >= 0.28
                && peaks == 2
                && peak_pos[0] <= (bw * 35) / 100
                && peak_pos[1] >= (bw * 65) / 100
                && (peak_pos[1] - peak_pos[0]) >= (bw * 40) / 100
            {
                let min_peak = peak_val[0].min(peak_val[1]);
                let mut min_valley = u16::MAX;
                for vx in peak_pos[0]..=peak_pos[1] {
                    min_valley = min_valley.min(self.proj_x[vx]);
                }
                // Middle fingers folded -> valley in center
                if min_valley <= (min_peak * 3 / 4).max(1) {
                    let span_score =
                        ((peak_pos[1] - peak_pos[0]) as f32 / bw as f32).clamp(0.0, 1.0);
                    let valley_score = 1.0 - (min_valley as f32 / min_peak.max(1) as f32);
                    confidence = (span_score * 0.5 + valley_score * 0.5).clamp(0.0, 1.0);
                    gesture = HandGesture::ShakaCall;
                }
            }
            // 4. Peace / V-Sign (✌)
            // Tall aspect ratio 1.05..3.50, exactly 2 peaks in upper section with gap and deep valley
            else if (1.05..=3.50).contains(&aspect)
                && (0.15..=0.88).contains(&fill)
                && lower_fill >= 0.25
                && peaks == 2
                && peak_pos[1] > peak_pos[0] + 1
            {
                let min_peak = peak_val[0].min(peak_val[1]);
                let mut min_valley = u16::MAX;
                for vx in peak_pos[0]..=peak_pos[1] {
                    min_valley = min_valley.min(self.proj_x[vx]);
                }

                if min_valley <= (min_peak / 2).max(1) {
                    let valley_score = 1.0 - (min_valley as f32 / min_peak.max(1) as f32);
                    let aspect_score = (1.0 - ((aspect - 2.0).abs() / 2.0)).clamp(0.2, 1.0);
                    confidence = (valley_score * 0.6 + aspect_score * 0.4).clamp(0.0, 1.0);
                    gesture = HandGesture::PeaceV;
                }
            }
            // 5. Thumbs Up (👍)
            // Solid fist base, 1 single peak on lateral side in upper half, compact aspect
            else if (0.70..=1.90).contains(&aspect)
                && lower_fill >= 0.40
                && peaks == 1
                && (peak_pos[0] <= (bw * 30) / 100 || peak_pos[0] >= (bw * 70) / 100)
            {
                let aspect_score = 1.0 - ((aspect - 1.25).abs() / 0.6).min(1.0);
                let fill_score = 1.0 - ((fill - 0.55).abs() / 0.30).min(1.0);
                confidence = (aspect_score * 0.5 + fill_score * 0.5).clamp(0.0, 1.0);
                gesture = HandGesture::ThumbsUp;
            }
            // 6. Pointing Up / Index Point (☝)
            // Elongated tall aspect ratio 1.20..3.80, single central index finger peak, solid fist base
            else if (1.20..=3.80).contains(&aspect)
                && lower_fill >= 0.35
                && peaks == 1
                && peak_pos[0] > (bw * 25) / 100
                && peak_pos[0] < (bw * 75) / 100
                && peak_w[0] <= (bw * 65) / 100
            {
                let aspect_score = ((aspect - 1.20) / 1.5).clamp(0.3, 1.0);
                let fill_score = lower_fill.clamp(0.3, 1.0);
                let height_score = (peak_val[0] as f32 / upper_h.max(1.0)).clamp(0.3, 1.0);
                confidence =
                    (aspect_score * 0.4 + fill_score * 0.3 + height_score * 0.3).clamp(0.0, 1.0);
                gesture = HandGesture::IndexPoint;
            }

            if gesture != HandGesture::None && confidence > best_conf {
                best_conf = confidence;
                best_gesture = gesture;
                best_cx = (cand.min_gx as f32 + bw as f32 * 0.5) / grid_w as f32;
                best_cy = (cand.min_gy as f32 + bh as f32 * 0.5) / grid_h as f32;
            }
        }

        // 5. Debouncing and rising-edge trigger logic with refractory cooldown
        let mut triggered = false;
        if best_gesture != HandGesture::None && best_conf >= 0.30 {
            if best_gesture == self.current_candidate {
                self.consecutive_count += 1;
            } else {
                self.current_candidate = best_gesture;
                self.consecutive_count = 1;
            }

            if self.consecutive_count >= self.debounce_frames && self.cooldown_remaining == 0 {
                triggered = true;
                self.cooldown_remaining = self.cooldown_frames;
            }
        } else {
            self.consecutive_count = 0;
            self.current_candidate = HandGesture::None;
        }

        GestureEvent {
            gesture: best_gesture,
            confidence: best_conf,
            center_x: best_cx,
            center_y: best_cy,
            triggered,
        }
    }
}

/// Standalone hand gesture detection function with zero allocations on the hot path.
#[inline]
pub fn detect_gesture(
    detector: &mut GestureDetector,
    bgra: &[u8],
    width: usize,
    height: usize,
    face: Option<FaceRect>,
) -> GestureEvent {
    detector.detect(bgra, width, height, face)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_background(w: usize, h: usize) -> Vec<u8> {
        let mut bgra = vec![0u8; w * h * 4];
        for chunk in bgra.chunks_exact_mut(4) {
            chunk[0] = 50;
            chunk[1] = 50;
            chunk[2] = 50;
            chunk[3] = 255;
        }
        bgra
    }

    fn draw_skin_rect(bgra: &mut [u8], w: usize, x0: usize, x1: usize, y0: usize, y1: usize) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let idx = (y * w + x) * 4;
                bgra[idx] = 120; // B
                bgra[idx + 1] = 150; // G
                bgra[idx + 2] = 210; // R
                bgra[idx + 3] = 255;
            }
        }
    }

    #[test]
    fn gesture_detector_empty_frame_returns_default() {
        let mut detector = GestureDetector::new();
        let ev = detector.detect(&[], 0, 0, None);
        assert_eq!(ev.gesture, HandGesture::None);
        assert!(!ev.triggered);
    }

    #[test]
    fn gesture_detector_detects_open_palm() {
        let mut detector = GestureDetector::new();
        let (w, h) = (320, 240);
        let mut bgra = make_background(w, h);

        // Draw synthetic Open Palm:
        // Palm rectangle at x: [120..200], y: [130..190]
        draw_skin_rect(&mut bgra, w, 120, 200, 130, 190);
        // Fingers: 4 vertical fingers at y: [70..130]
        draw_skin_rect(&mut bgra, w, 125, 135, 70, 130);
        draw_skin_rect(&mut bgra, w, 145, 155, 70, 130);
        draw_skin_rect(&mut bgra, w, 165, 175, 70, 130);
        draw_skin_rect(&mut bgra, w, 185, 195, 70, 130);

        let mut triggered_any = false;
        let mut last_event = GestureEvent::default();
        for _ in 0..8 {
            last_event = detector.detect(&bgra, w, h, None);
            if last_event.triggered {
                triggered_any = true;
            }
        }

        assert_eq!(last_event.gesture, HandGesture::OpenPalm);
        assert!(last_event.confidence >= 0.30);
        assert!(
            triggered_any,
            "Open Palm should be triggered after debounce"
        );
    }

    #[test]
    fn gesture_detector_detects_peace_v_sign() {
        let mut detector = GestureDetector::new();
        let (w, h) = (320, 240);
        let mut bgra = make_background(w, h);

        // Draw synthetic Peace/V-sign:
        // Fist/palm base at x: [140..180], y: [130..180]
        draw_skin_rect(&mut bgra, w, 140, 180, 130, 180);
        // 2 fingers (Index and Middle) at y: [60..130]:
        draw_skin_rect(&mut bgra, w, 142, 154, 60, 130);
        draw_skin_rect(&mut bgra, w, 166, 178, 60, 130);

        let mut triggered_any = false;
        let mut last_event = GestureEvent::default();
        for _ in 0..8 {
            last_event = detector.detect(&bgra, w, h, None);
            if last_event.triggered {
                triggered_any = true;
            }
        }

        assert_eq!(last_event.gesture, HandGesture::PeaceV);
        assert!(last_event.confidence >= 0.30);
        assert!(
            triggered_any,
            "Peace gesture should be triggered after debounce"
        );
    }

    #[test]
    fn gesture_detector_detects_index_pointing_up() {
        let mut detector = GestureDetector::new();
        let (w, h) = (320, 240);
        let mut bgra = make_background(w, h);

        // Draw synthetic Pointing Up (☝):
        // Solid fist base at x: [140..180], y: [130..180]
        draw_skin_rect(&mut bgra, w, 140, 180, 130, 180);
        // Single central index finger at x: [154..166], y: [60..130]
        draw_skin_rect(&mut bgra, w, 154, 166, 60, 130);

        let mut triggered_any = false;
        let mut last_event = GestureEvent::default();
        for _ in 0..8 {
            last_event = detector.detect(&bgra, w, h, None);
            if last_event.triggered {
                triggered_any = true;
            }
        }

        assert_eq!(last_event.gesture, HandGesture::IndexPoint);
        assert!(last_event.confidence >= 0.30);
        assert!(
            triggered_any,
            "Index point gesture should trigger after debounce"
        );
    }

    #[test]
    fn gesture_detector_detects_shaka_call() {
        let mut detector = GestureDetector::new();
        let (w, h) = (320, 240);
        let mut bgra = make_background(w, h);

        // Draw synthetic Shaka / Hang Loose (🤙):
        // Curled center fingers / palm base at x: [140..180], y: [120..170]
        draw_skin_rect(&mut bgra, w, 140, 180, 120, 170);
        // Thumb extended far left at x: [115..139], y: [100..140]
        draw_skin_rect(&mut bgra, w, 115, 139, 100, 140);
        // Pinky extended far right at x: [181..205], y: [100..140]
        draw_skin_rect(&mut bgra, w, 181, 205, 100, 140);

        let mut triggered_any = false;
        let mut last_event = GestureEvent::default();
        for _ in 0..8 {
            last_event = detector.detect(&bgra, w, h, None);
            if last_event.triggered {
                triggered_any = true;
            }
        }

        assert_eq!(last_event.gesture, HandGesture::ShakaCall);
        assert!(last_event.confidence >= 0.30);
        assert!(triggered_any, "Shaka gesture should trigger after debounce");
    }

    #[test]
    fn gesture_detector_detects_pinch_ok_sign() {
        let mut detector = GestureDetector::new();
        let (w, h) = (320, 240);
        let mut bgra = make_background(w, h);

        // Draw synthetic Pinch / OK circle loop:
        // Outer skin ring: x in [130..190], y in [110..170]
        draw_skin_rect(&mut bgra, w, 130, 190, 110, 170);
        // Inner non-skin hole: x in [148..172], y in [128..152]
        for y in 128..=152 {
            for x in 148..=172 {
                let idx = (y * w + x) * 4;
                bgra[idx] = 50;
                bgra[idx + 1] = 50;
                bgra[idx + 2] = 50;
                bgra[idx + 3] = 255;
            }
        }

        let mut triggered_any = false;
        let mut last_event = GestureEvent::default();
        for _ in 0..8 {
            last_event = detector.detect(&bgra, w, h, None);
            if last_event.triggered {
                triggered_any = true;
            }
        }

        assert_eq!(last_event.gesture, HandGesture::PinchOk);
        assert!(last_event.confidence >= 0.30);
        assert!(
            triggered_any,
            "Pinch/OK gesture should trigger after debounce"
        );
    }

    #[test]
    fn gesture_detector_detects_thumbs_up() {
        let mut detector = GestureDetector::new();
        let (w, h) = (320, 240);
        let mut bgra = make_background(w, h);

        // Draw synthetic Thumbs Up (👍):
        // Solid fist base: x in [130..190], y in [120..175] (bw 61, bh 56)
        draw_skin_rect(&mut bgra, w, 130, 190, 120, 175);
        // Lateral thumb sticking upwards on left edge: x in [130..145], y in [90..120]
        draw_skin_rect(&mut bgra, w, 130, 145, 90, 120);

        let mut triggered_any = false;
        let mut last_event = GestureEvent::default();
        for _ in 0..8 {
            last_event = detector.detect(&bgra, w, h, None);
            if last_event.triggered {
                triggered_any = true;
            }
        }

        assert_eq!(last_event.gesture, HandGesture::ThumbsUp);
        assert!(last_event.confidence >= 0.30);
        assert!(triggered_any, "Thumbs Up should trigger after debounce");
    }

    #[test]
    fn gesture_detector_cooldown_prevents_rapid_fire() {
        let mut detector = GestureDetector::new();
        detector.debounce_frames = 2;
        detector.cooldown_frames = 10;

        let (w, h) = (320, 240);
        let mut bgra = make_background(w, h);
        draw_skin_rect(&mut bgra, w, 140, 180, 130, 180);
        draw_skin_rect(&mut bgra, w, 154, 166, 60, 130);

        // Frame 1: debounce count 1
        let ev1 = detector.detect(&bgra, w, h, None);
        assert!(!ev1.triggered);

        // Frame 2: debounce count 2 -> triggers!
        let ev2 = detector.detect(&bgra, w, h, None);
        assert!(ev2.triggered);

        // Frame 3..10: cooldown active -> must not trigger
        for _ in 0..8 {
            let ev = detector.detect(&bgra, w, h, None);
            assert!(!ev.triggered);
        }
    }

    #[test]
    fn gesture_detector_zero_allocation_hot_path() {
        let mut detector = GestureDetector::new();
        let (w, h) = (160, 120);
        let bgra = vec![128u8; w * h * 4];

        for _ in 0..60 {
            let _ = detector.detect(&bgra, w, h, None);
        }
        assert!(detector.queue.capacity() <= MAX_GRID_CELLS);
        assert!(detector.candidates.capacity() <= 32);
    }

    #[test]
    fn gesture_detector_portrait_frame_does_not_overflow_grid() {
        let mut detector = GestureDetector::new();
        let (w, h) = (240, 320);
        let mut bgra = make_background(w, h);

        // Open Palm placed near the bottom of a portrait frame (aspect ~1.15)
        draw_skin_rect(&mut bgra, w, 70, 170, 225, 290);
        draw_skin_rect(&mut bgra, w, 75, 88, 175, 225);
        draw_skin_rect(&mut bgra, w, 100, 113, 175, 225);
        draw_skin_rect(&mut bgra, w, 125, 138, 175, 225);
        draw_skin_rect(&mut bgra, w, 150, 163, 175, 225);

        let mut last_event = GestureEvent::default();
        for _ in 0..8 {
            last_event = detector.detect(&bgra, w, h, None);
        }
        assert_eq!(last_event.gesture, HandGesture::OpenPalm);
        assert!(last_event.center_y > 0.5);
    }

    #[test]
    fn gesture_detector_ignores_tiny_hole_in_open_palm_for_pinch_ok() {
        let mut detector = GestureDetector::new();
        let (w, h) = (320, 240);
        let mut bgra = make_background(w, h);

        // Open Palm
        draw_skin_rect(&mut bgra, w, 120, 200, 130, 190);
        draw_skin_rect(&mut bgra, w, 125, 135, 70, 130);
        draw_skin_rect(&mut bgra, w, 145, 155, 70, 130);
        draw_skin_rect(&mut bgra, w, 165, 175, 70, 130);
        draw_skin_rect(&mut bgra, w, 185, 195, 70, 130);

        // Single small 2x2 dark speck inside the palm (must NOT trigger PinchOk!)
        for y in 158..=159 {
            for x in 158..=159 {
                let idx = (y * w + x) * 4;
                bgra[idx] = 20;
                bgra[idx + 1] = 20;
                bgra[idx + 2] = 20;
            }
        }

        let ev = detector.detect(&bgra, w, h, None);
        assert_ne!(ev.gesture, HandGesture::PinchOk);
        assert_eq!(ev.gesture, HandGesture::OpenPalm);
    }

    #[test]
    fn gesture_detector_default_debounce_prevents_short_burst_trigger() {
        let mut detector = GestureDetector::new();
        assert!(detector.debounce_frames >= 6);
        let (w, h) = (320, 240);
        let mut bgra = make_background(w, h);
        draw_skin_rect(&mut bgra, w, 140, 180, 130, 180);
        draw_skin_rect(&mut bgra, w, 154, 166, 60, 130);

        for _ in 0..5 {
            let ev = detector.detect(&bgra, w, h, None);
            assert!(!ev.triggered, "must not trigger before debounce_frames");
        }
        let mut triggered = false;
        for _ in 0..4 {
            if detector.detect(&bgra, w, h, None).triggered {
                triggered = true;
            }
        }
        assert!(
            triggered,
            "should trigger once debounce threshold is reached"
        );
    }
}
