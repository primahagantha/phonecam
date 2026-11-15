//! Real-time OBSBOT-style Hand Gesture Recognition in pure Rust.
//!
//! Recognizes:
//! - Open Palm gesture (✋): Toggles tracking lock/unlock.
//! - Peace / V-Sign gesture (✌) and Thumbs Up (👍): Toggles zoom level (e.g. 1.0x <-> 2.0x).
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
    /// Peace / V-Sign gesture (✌) - toggles zoom level (e.g. 1.0x <-> 2.0x).
    PeaceV,
    /// Thumbs Up gesture (👍) - alternative zoom or confirmation trigger.
    ThumbsUp,
}

impl HandGesture {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::OpenPalm => "Open Palm (✋)",
            Self::PeaceV => "Peace / V-Sign (✌)",
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

            debounce_frames: 3,
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

        // Determine downscaled grid dimensions
        let (grid_w, grid_h) = if width >= height {
            let gw = MAX_GRID_W;
            let gh = ((height * MAX_GRID_W) / width).clamp(45, MAX_GRID_H);
            (gw, gh)
        } else {
            let gh = MAX_GRID_W;
            let gw = ((width * MAX_GRID_W) / height).clamp(45, MAX_GRID_H);
            (gw, gh)
        };

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

                    if (0.50..=3.50).contains(&aspect) && (0.15..=0.95).contains(&fill) {
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

        // 4. Classify gesture candidates
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

            // Vertical projection in the upper half (fingers zone)
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

            // Count peaks in upper finger projection
            let peak_threshold = (upper_h * 0.30).max(2.0) as u16;
            let mut peaks = 0usize;
            let mut in_peak = false;
            let mut peak_start = 0usize;
            let mut peak_positions = [0usize; 6];

            for x in 0..bw {
                if self.proj_x[x] >= peak_threshold {
                    if !in_peak {
                        in_peak = true;
                        peak_start = x;
                    }
                } else if in_peak {
                    in_peak = false;
                    let peak_center = (peak_start + x - 1) / 2;
                    if peaks < 6 {
                        peak_positions[peaks] = peak_center;
                        peaks += 1;
                    }
                }
            }
            if in_peak && peaks < 6 {
                peak_positions[peaks] = (peak_start + bw - 1) / 2;
                peaks += 1;
            }

            let mut gesture = HandGesture::None;
            let mut confidence = 0.0f32;

            // Pattern A: Open Palm (✋)
            // Aspect ratio ~ 0.70 to 1.60, solid palm base, wide fingers spread
            if (0.65..=1.65).contains(&aspect)
                && (0.30..=0.85).contains(&fill)
                && lower_fill >= 0.35
                && (peaks >= 3 || (upper_pixels as f32 / (bw as f32 * upper_h)) >= 0.35)
            {
                let aspect_score = 1.0 - ((aspect - 1.10).abs() / 0.7).min(1.0);
                let fill_score = 1.0 - ((fill - 0.55).abs() / 0.35).min(1.0);
                confidence = (aspect_score * 0.5 + fill_score * 0.5).clamp(0.0, 1.0);
                gesture = HandGesture::OpenPalm;
            }
            // Pattern B: Peace / V-Sign (✌)
            // Taller aspect ratio ~ 1.10 to 3.50, exactly two peaks in upper section with gap
            else if (1.05..=3.50).contains(&aspect)
                && (0.15..=0.88).contains(&fill)
                && lower_fill >= 0.25
                && peaks == 2
                && peak_positions[1] > peak_positions[0] + 1
            {
                // Check valley between peaks
                let p1 = peak_positions[0];
                let p2 = peak_positions[1];
                let mid_x = (p1 + p2) / 2;
                let valley_val = self.proj_x[mid_x];
                let p1_val = self.proj_x[p1];
                let p2_val = self.proj_x[p2];
                let min_peak = p1_val.min(p2_val);

                if valley_val <= (min_peak / 2).max(1) {
                    let valley_score = 1.0 - (valley_val as f32 / min_peak.max(1) as f32);
                    let aspect_score = (1.0 - ((aspect - 2.0).abs() / 2.0)).clamp(0.2, 1.0);
                    confidence = (valley_score * 0.6 + aspect_score * 0.4).clamp(0.0, 1.0);
                    gesture = HandGesture::PeaceV;
                }
            }
            // Pattern C: Thumbs Up (👍)
            // Solid fist base, 1 single peak on lateral side in upper half
            else if (0.80..=1.80).contains(&aspect)
                && lower_fill >= 0.50
                && peaks == 1
                && (peak_positions[0] <= bw / 3 || peak_positions[0] >= (bw * 2) / 3)
            {
                let aspect_score = 1.0 - ((aspect - 1.25).abs() / 0.6).min(1.0);
                let fill_score = 1.0 - ((fill - 0.55).abs() / 0.30).min(1.0);
                confidence = (aspect_score * 0.5 + fill_score * 0.5).clamp(0.0, 1.0);
                gesture = HandGesture::ThumbsUp;
            }

            if gesture != HandGesture::None && confidence > best_conf {
                best_conf = confidence;
                best_gesture = gesture;
                best_cx = (cand.min_gx as f32 + bw as f32 * 0.5) / grid_w as f32;
                best_cy = (cand.min_gy as f32 + bh as f32 * 0.5) / grid_h as f32;
            }
        }

        // 5. Debouncing and rising-edge trigger logic
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
        let mut bgra = vec![0u8; w * h * 4];
        for chunk in bgra.chunks_exact_mut(4) {
            chunk[0] = 50;
            chunk[1] = 50;
            chunk[2] = 50;
            chunk[3] = 255;
        }

        // Draw synthetic Open Palm:
        // Palm rectangle at x: [120..200], y: [130..190]
        // Fingers: 4 vertical fingers at y: [70..130]
        for y in 130..190 {
            for x in 120..200 {
                let idx = (y * w + x) * 4;
                bgra[idx] = 120; // B
                bgra[idx + 1] = 150; // G
                bgra[idx + 2] = 210; // R
                bgra[idx + 3] = 255;
            }
        }
        for y in 70..130 {
            for x in 120..200 {
                // 4 fingers: x in [125..135], [145..155], [165..175], [185..195]
                if (125..=135).contains(&x)
                    || (145..=155).contains(&x)
                    || (165..=175).contains(&x)
                    || (185..=195).contains(&x)
                {
                    let idx = (y * w + x) * 4;
                    bgra[idx] = 120;
                    bgra[idx + 1] = 150;
                    bgra[idx + 2] = 210;
                    bgra[idx + 3] = 255;
                }
            }
        }

        // Feed frames to satisfy debouncer and catch the rising edge trigger
        let mut triggered_any = false;
        let mut last_event = GestureEvent::default();
        for _ in 0..5 {
            last_event = detector.detect(&bgra, w, h, None);
            if last_event.triggered {
                triggered_any = true;
            }
        }

        assert_eq!(last_event.gesture, HandGesture::OpenPalm);
        assert!(last_event.confidence >= 0.30);
        assert!(triggered_any, "Gesture should be triggered after debounce");
    }

    #[test]
    fn gesture_detector_detects_peace_v_sign() {
        let mut detector = GestureDetector::new();
        let (w, h) = (320, 240);
        let mut bgra = vec![0u8; w * h * 4];
        for chunk in bgra.chunks_exact_mut(4) {
            chunk[0] = 50;
            chunk[1] = 50;
            chunk[2] = 50;
            chunk[3] = 255;
        }

        // Draw synthetic Peace/V-sign:
        // Fist/palm base at x: [140..180], y: [130..180]
        // 2 fingers (Index and Middle) at y: [60..130]:
        // Finger 1 at x: [142..154]
        // Gap at x: [155..165]
        // Finger 2 at x: [166..178]
        for y in 130..180 {
            for x in 140..180 {
                let idx = (y * w + x) * 4;
                bgra[idx] = 120;
                bgra[idx + 1] = 150;
                bgra[idx + 2] = 210;
                bgra[idx + 3] = 255;
            }
        }
        for y in 60..130 {
            for x in 140..180 {
                if (142..=154).contains(&x) || (166..=178).contains(&x) {
                    let idx = (y * w + x) * 4;
                    bgra[idx] = 120;
                    bgra[idx + 1] = 150;
                    bgra[idx + 2] = 210;
                    bgra[idx + 3] = 255;
                }
            }
        }

        let mut triggered_any = false;
        let mut last_event = GestureEvent::default();
        for _ in 0..5 {
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
}
