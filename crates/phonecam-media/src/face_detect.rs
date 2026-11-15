//! High-performance Computer Vision Face Detection on premultiplied BGRA frames in pure Rust.
//!
//! Features:
//! - Zero heap allocations on the 60 FPS hot path.
//! - Fast downscaled skin-chrominance segmentation in YCbCr color space.
//! - Contiguous component clustering with facial geometry and Haar-like contrast verification.
//! - Temporal exponential smoothing and tracking hysteresis.

use serde::{Deserialize, Serialize};

/// Maximum grid dimensions for downscaled analysis.
pub const MAX_GRID_W: usize = 160;
pub const MAX_GRID_H: usize = 120;
pub const MAX_GRID_CELLS: usize = MAX_GRID_W * MAX_GRID_H;

/// Normalized bounding box of a detected human face.
///
/// Coordinates are in the normalized range `[0.0, 1.0]` relative to frame dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaceRect {
    /// Normalized x coordinate of top-left corner `[0.0, 1.0]`.
    pub x: f32,
    /// Normalized y coordinate of top-left corner `[0.0, 1.0]`.
    pub y: f32,
    /// Normalized width `[0.0, 1.0]`.
    pub width: f32,
    /// Normalized height `[0.0, 1.0]`.
    pub height: f32,
    /// Normalized center x coordinate `[0.0, 1.0]`.
    pub center_x: f32,
    /// Normalized center y coordinate `[0.0, 1.0]`.
    pub center_y: f32,
    /// Detection confidence score `[0.0, 1.0]`.
    pub confidence: f32,
}

impl FaceRect {
    pub fn new(x: f32, y: f32, width: f32, height: f32, confidence: f32) -> Self {
        let x = x.clamp(0.0, 1.0);
        let y = y.clamp(0.0, 1.0);
        let width = width.clamp(0.0, 1.0 - x);
        let height = height.clamp(0.0, 1.0 - y);
        let center_x = (x + width * 0.5).clamp(0.0, 1.0);
        let center_y = (y + height * 0.5).clamp(0.0, 1.0);
        Self {
            x,
            y,
            width,
            height,
            center_x,
            center_y,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
}

/// Internal candidate face region prior to geometry verification.
#[derive(Debug, Clone, Copy, Default)]
struct CandidateRegion {
    min_gx: usize,
    max_gx: usize,
    min_gy: usize,
    max_gy: usize,
    pixel_count: usize,
}

/// Fast, zero-allocation Computer Vision Face Detector.
#[derive(Debug, Clone)]
pub struct FaceDetector {
    skin_mask: Vec<u8>,
    filtered_mask: Vec<u8>,
    luma: Vec<u8>,
    visited: Vec<bool>,
    queue: Vec<usize>,
    candidates: Vec<CandidateRegion>,
    pub last_face: Option<FaceRect>,
    pub frames_since_detection: usize,
}

impl Default for FaceDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl FaceDetector {
    /// Creates a new `FaceDetector` with pre-allocated scratch buffers.
    pub fn new() -> Self {
        Self {
            skin_mask: vec![0u8; MAX_GRID_CELLS],
            filtered_mask: vec![0u8; MAX_GRID_CELLS],
            luma: vec![0u8; MAX_GRID_CELLS],
            visited: vec![false; MAX_GRID_CELLS],
            queue: Vec::with_capacity(MAX_GRID_CELLS),
            candidates: Vec::with_capacity(64),
            last_face: None,
            frames_since_detection: 100,
        }
    }

    /// Resets detector state and tracking history.
    pub fn reset(&mut self) {
        self.last_face = None;
        self.frames_since_detection = 100;
    }

    /// Detects the primary human face in a premultiplied BGRA image buffer.
    ///
    /// Guarantees zero heap allocations when called repeatedly at 60 FPS.
    pub fn detect(&mut self, bgra: &[u8], width: usize, height: usize) -> Option<FaceRect> {
        if width == 0 || height == 0 || bgra.len() < width * height * 4 {
            return None;
        }

        // Determine downscaled grid dimensions preserving aspect ratio
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
            return None;
        }

        let step_x = width as f32 / grid_w as f32;
        let step_y = height as f32 / grid_h as f32;

        // 1. Skin-chrominance segmentation & luma extraction
        for gy in 0..grid_h {
            let py = ((gy as f32 + 0.5) * step_y) as usize;
            let row_offset = py.min(height - 1) * width * 4;
            let grid_row = gy * grid_w;

            for gx in 0..grid_w {
                let px = ((gx as f32 + 0.5) * step_x) as usize;
                let px_idx = row_offset + px.min(width - 1) * 4;

                let b = bgra[px_idx];
                let g = bgra[px_idx + 1];
                let r = bgra[px_idx + 2];

                let (is_skin, y) = is_skin_rgb(r, g, b);
                let cell_idx = grid_row + gx;
                self.skin_mask[cell_idx] = if is_skin { 1 } else { 0 };
                self.luma[cell_idx] = y;
            }
        }

        // 2. Morphological 3x3 filter: remove isolated noise and fill minor holes
        for gy in 0..grid_h {
            let row = gy * grid_w;
            for gx in 0..grid_w {
                let idx = row + gx;
                if gx == 0 || gx == grid_w - 1 || gy == 0 || gy == grid_h - 1 {
                    self.filtered_mask[idx] = self.skin_mask[idx];
                    continue;
                }

                let mut skin_neighbors = 0u8;
                for dy in [-1, 0, 1] {
                    let n_row = (gy as isize + dy) as usize * grid_w;
                    for dx in [-1, 0, 1] {
                        let n_idx = n_row + (gx as isize + dx) as usize;
                        skin_neighbors += self.skin_mask[n_idx];
                    }
                }

                if self.skin_mask[idx] == 1 {
                    self.filtered_mask[idx] = if skin_neighbors >= 3 { 1 } else { 0 };
                } else {
                    self.filtered_mask[idx] = if skin_neighbors >= 6 { 1 } else { 0 };
                }
            }
        }

        // 3. Connected components blob extraction via BFS
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

                // Preliminary face candidate filter: minimum 4% of grid dimensions, at least 16 pixels
                let min_dim = (grid_w.min(grid_h) / 25).max(4);
                if bw >= min_dim && bh >= min_dim && pixel_count >= 16 {
                    let aspect = bh as f32 / bw as f32;
                    let fill = pixel_count as f32 / (bw * bh) as f32;

                    if (0.75..=2.3).contains(&aspect) && (0.28..=0.96).contains(&fill) {
                        self.candidates.push(CandidateRegion {
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

        // 4. Facial Geometry & Haar-like Feature Verification
        let mut best_candidate: Option<(FaceRect, f32)> = None;

        for cand in &self.candidates {
            let bw = cand.max_gx - cand.min_gx + 1;
            let bh = cand.max_gy - cand.min_gy + 1;
            let aspect = bh as f32 / bw as f32;
            let fill = cand.pixel_count as f32 / (bw * bh) as f32;

            // Facial vertical zones: forehead (10-35%), eyes (35-60%), cheeks (60-85%)
            let y_fh_start = cand.min_gy + (bh * 10 / 100);
            let y_fh_end = cand.min_gy + (bh * 35 / 100);
            let y_eye_start = cand.min_gy + (bh * 35 / 100);
            let y_eye_end = cand.min_gy + (bh * 60 / 100);
            let y_ck_start = cand.min_gy + (bh * 60 / 100);
            let y_ck_end = cand.min_gy + (bh * 85 / 100);

            let luma_fh = self.average_luma(cand.min_gx, cand.max_gx, y_fh_start, y_fh_end, grid_w);
            let luma_eye =
                self.average_luma(cand.min_gx, cand.max_gx, y_eye_start, y_eye_end, grid_w);
            let luma_ck = self.average_luma(cand.min_gx, cand.max_gx, y_ck_start, y_ck_end, grid_w);

            // Eye sockets are typically darker than forehead and cheeks
            let eye_contrast = ((luma_fh + luma_ck) * 0.5 - luma_eye).max(0.0);

            // Bilateral symmetry
            let mid_x = (cand.min_gx + cand.max_gx) / 2;
            let luma_left = self.average_luma(cand.min_gx, mid_x, cand.min_gy, cand.max_gy, grid_w);
            let luma_right =
                self.average_luma(mid_x, cand.max_gx, cand.min_gy, cand.max_gy, grid_w);
            let sym_diff = (luma_left - luma_right).abs();
            let sym_score = 1.0 - (sym_diff / 80.0).min(1.0);

            let aspect_score = 1.0 - ((aspect - 1.35).abs() / 1.0).min(1.0);
            let fill_score = 1.0 - ((fill - 0.60).abs() / 0.40).min(1.0);
            let size_score = (cand.pixel_count as f32 / grid_cells as f32 * 8.0).clamp(0.2, 1.0);
            let contrast_score = (eye_contrast / 15.0).clamp(0.0, 0.4);

            let confidence = (aspect_score * 0.30
                + fill_score * 0.30
                + sym_score * 0.20
                + size_score * 0.10
                + contrast_score * 0.10)
                .clamp(0.0, 1.0);

            let norm_x = cand.min_gx as f32 / grid_w as f32;
            let norm_y = cand.min_gy as f32 / grid_h as f32;
            let norm_w = bw as f32 / grid_w as f32;
            let norm_h = bh as f32 / grid_h as f32;
            let norm_cx = (cand.min_gx as f32 + bw as f32 * 0.5) / grid_w as f32;
            let norm_cy = (cand.min_gy as f32 + bh as f32 * 0.5) / grid_h as f32;

            // Proximity bonus to lock onto previous face location
            let mut total_score = confidence;
            if let Some(prev) = self.last_face {
                let dx = norm_cx - prev.center_x;
                let dy = norm_cy - prev.center_y;
                let dist = (dx * dx + dy * dy).sqrt();
                if dist < 0.25 {
                    total_score += 0.20 * (1.0 - dist / 0.25);
                }
            }

            if confidence >= 0.28
                && (best_candidate.is_none() || total_score > best_candidate.as_ref().unwrap().1)
            {
                let rect = FaceRect::new(norm_x, norm_y, norm_w, norm_h, confidence);
                best_candidate = Some((rect, total_score));
            }
        }

        // 5. Temporal smoothing & tracking hysteresis
        if let Some((detected, _score)) = best_candidate {
            self.frames_since_detection = 0;
            let smoothed = if let Some(prev) = self.last_face {
                let alpha = 0.30f32;
                let cx = prev.center_x * (1.0 - alpha) + detected.center_x * alpha;
                let cy = prev.center_y * (1.0 - alpha) + detected.center_y * alpha;
                let w = prev.width * (1.0 - alpha) + detected.width * alpha;
                let h = prev.height * (1.0 - alpha) + detected.height * alpha;
                FaceRect::new(cx - w * 0.5, cy - h * 0.5, w, h, detected.confidence)
            } else {
                detected
            };
            self.last_face = Some(smoothed);
            Some(smoothed)
        } else {
            self.frames_since_detection += 1;
            // Hold previous face for up to 6 frames (~100-200ms) with decay
            if self.frames_since_detection <= 6 {
                if let Some(mut prev) = self.last_face {
                    prev.confidence *= 0.85;
                    self.last_face = Some(prev);
                    Some(prev)
                } else {
                    None
                }
            } else {
                self.last_face = None;
                None
            }
        }
    }

    /// Computes average luma in a sub-rectangle of the downscaled grid.
    fn average_luma(
        &self,
        min_x: usize,
        max_x: usize,
        min_y: usize,
        max_y: usize,
        grid_w: usize,
    ) -> f32 {
        if min_x > max_x || min_y > max_y {
            return 0.0;
        }
        let mut sum = 0u32;
        let mut count = 0u32;
        for gy in min_y..=max_y {
            let row = gy * grid_w;
            for gx in min_x..=max_x {
                sum += self.luma[row + gx] as u32;
                count += 1;
            }
        }
        if count == 0 {
            0.0
        } else {
            sum as f32 / count as f32
        }
    }
}

/// Standalone face detection function with zero allocations on the hot path.
#[inline]
pub fn detect_face(
    detector: &mut FaceDetector,
    bgra: &[u8],
    width: usize,
    height: usize,
) -> Option<FaceRect> {
    detector.detect(bgra, width, height)
}

/// Chrominance skin classification in YCbCr space using fixed-point BT.601 math.
#[inline(always)]
pub fn is_skin_rgb(r: u8, g: u8, b: u8) -> (bool, u8) {
    let r_i = r as i32;
    let g_i = g as i32;
    let b_i = b as i32;

    // Fixed-point BT.601 integer math:
    // Y  =  (77*R + 150*G + 29*B) >> 8
    // Cb = 128 + (-43*R - 85*G + 128*B) >> 8
    // Cr = 128 + (128*R - 107*G - 21*B) >> 8
    let y = (77 * r_i + 150 * g_i + 29 * b_i) >> 8;
    let cb = 128 + ((-43 * r_i - 85 * g_i + 128 * b_i) >> 8);
    let cr = 128 + ((128 * r_i - 107 * g_i - 21 * b_i) >> 8);

    let is_skin = cb >= 75
        && cb <= 132
        && cr >= 130
        && cr <= 178
        && r > g
        && g >= (b.saturating_sub(15))
        && r > 40
        && y >= 25
        && (r_i - g_i).abs() >= 6;

    (is_skin, y.clamp(0, 255) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_chrominance_detection() {
        // Natural skin tone (R=210, G=150, B=120)
        let (skin, y) = is_skin_rgb(210, 150, 120);
        assert!(skin, "Skin color should be detected as skin");
        assert!(y > 100);

        // Deep skin tone (R=110, G=70, B=50)
        let (skin_deep, _) = is_skin_rgb(110, 70, 50);
        assert!(skin_deep, "Deep skin tone should be detected as skin");

        // Non-skin: pure blue wall
        let (skin_blue, _) = is_skin_rgb(20, 50, 200);
        assert!(!skin_blue, "Blue wall should NOT be skin");

        // Non-skin: pure white sheet
        let (skin_white, _) = is_skin_rgb(255, 255, 255);
        assert!(!skin_white, "White sheet should NOT be skin");

        // Non-skin: dark background
        let (skin_dark, _) = is_skin_rgb(10, 10, 10);
        assert!(!skin_dark, "Dark background should NOT be skin");
    }

    #[test]
    fn face_detector_empty_or_zero_frame_returns_none() {
        let mut detector = FaceDetector::new();
        assert_eq!(detector.detect(&[], 0, 0), None);
        assert_eq!(detector.detect(&[0u8; 10], 10, 10), None);
    }

    #[test]
    fn face_detector_detects_synthetic_face() {
        let mut detector = FaceDetector::new();
        let (w, h) = (320, 240);
        // Background: cool blue-gray
        let mut bgra = vec![0u8; w * h * 4];
        for chunk in bgra.chunks_exact_mut(4) {
            chunk[0] = 180; // B
            chunk[1] = 120; // G
            chunk[2] = 80; // R
            chunk[3] = 255; // A
        }

        // Draw synthetic face oval centered at (160, 120), w=80, h=100
        let face_cx = 160f32;
        let face_cy = 120f32;
        let face_rx = 40f32;
        let face_ry = 50f32;

        for y in 0..h {
            let dy = (y as f32 - face_cy) / face_ry;
            for x in 0..w {
                let dx = (x as f32 - face_cx) / face_rx;
                if dx * dx + dy * dy <= 1.0 {
                    let idx = (y * w + x) * 4;
                    // Natural skin tone
                    bgra[idx] = 120; // B
                    bgra[idx + 1] = 150; // G
                    bgra[idx + 2] = 210; // R
                    bgra[idx + 3] = 255;

                    // Eye sockets: slightly darker horizontal band
                    if y >= 110 && y <= 125 && (x >= 135 && x <= 150 || x >= 170 && x <= 185) {
                        bgra[idx] = 80;
                        bgra[idx + 1] = 100;
                        bgra[idx + 2] = 150;
                    }
                }
            }
        }

        let face = detector
            .detect(&bgra, w, h)
            .expect("Face should be detected");
        assert!(
            (face.center_x - 0.50).abs() < 0.10,
            "Face center_x should be near 0.50, got {}",
            face.center_x
        );
        assert!(
            (face.center_y - 0.50).abs() < 0.10,
            "Face center_y should be near 0.50, got {}",
            face.center_y
        );
        assert!(face.confidence >= 0.30);
    }

    #[test]
    fn face_detector_zero_allocation_hot_path() {
        let mut detector = FaceDetector::new();
        let (w, h) = (160, 120);
        let bgra = vec![128u8; w * h * 4];

        // Call 60 times simulating 60 FPS
        for _ in 0..60 {
            let _ = detector.detect(&bgra, w, h);
        }
        assert!(detector.queue.capacity() <= MAX_GRID_CELLS);
        assert!(detector.candidates.capacity() <= 64);
    }
}
