//! Mean luma and flicker watchdog.
//!
//! Ported from `native/core/src/core_session.cpp` (upstream commit 5e6b22d).
//!
//! Computes mean luma of published frames and logs a warning when consecutive
//! frames differ significantly (>24/255 at same exposure), identifying
//! lighting (50/60 Hz mains beating against shutter) or AE hunting flicker.
//!
//! Transition events are reported once on enter and exit, never per frame,
//! preventing log spam.

pub const LUMA_JUMP_THRESHOLD: u8 = 24;

/// A state transition event detected by `FlickerDetector`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlickerEvent {
    /// Picture brightness jumped more than `LUMA_JUMP_THRESHOLD` (24/255).
    FlickerStarted { jump: u8 },
    /// Picture brightness returned to stable.
    FlickerStopped,
}

/// Watchdog that tracks mean luma across consecutive frames to detect flicker.
#[derive(Debug, Clone, Default)]
pub struct FlickerDetector {
    last_mean_luma: Option<u8>,
    flicker_reported: bool,
}

impl FlickerDetector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Computes the mean luma (Y plane) of an image buffer by sampling every 16th pixel.
    pub fn compute_mean_luma(luma: &[u8], width: usize, height: usize) -> u8 {
        let samples = width * height;
        if samples == 0 || luma.len() < samples {
            return 0;
        }
        let mut sum: u64 = 0;
        let mut count: u64 = 0;
        for i in (0..samples).step_by(16) {
            sum += luma[i] as u64;
            count += 1;
        }
        if count == 0 {
            0
        } else {
            (sum / count) as u8
        }
    }

    /// Evaluates a luma buffer (first `width * height` bytes) for flicker.
    pub fn observe(&mut self, luma: &[u8], width: usize, height: usize) -> Option<FlickerEvent> {
        let mean = Self::compute_mean_luma(luma, width, height);
        self.observe_mean(mean)
    }

    /// Evaluates a pre-computed mean luma value.
    pub fn observe_mean(&mut self, mean: u8) -> Option<FlickerEvent> {
        let event = if let Some(previous) = self.last_mean_luma {
            let jump = (mean as i16 - previous as i16).abs() as u8;
            let flickering = jump > LUMA_JUMP_THRESHOLD;
            if flickering && !self.flicker_reported {
                self.flicker_reported = true;
                tracing::warn!(
                    "picture brightness jumped {}/255 between frames; this is what flicker looks like (check anti-banding and exposure)",
                    jump
                );
                Some(FlickerEvent::FlickerStarted { jump })
            } else if !flickering && self.flicker_reported {
                self.flicker_reported = false;
                tracing::info!("picture brightness is stable again");
                Some(FlickerEvent::FlickerStopped)
            } else {
                None
            }
        } else {
            None
        };
        self.last_mean_luma = Some(mean);
        event
    }

    pub fn last_mean_luma(&self) -> Option<u8> {
        self.last_mean_luma
    }

    pub fn is_flicker_reported(&self) -> bool {
        self.flicker_reported
    }

    pub fn reset(&mut self) {
        self.last_mean_luma = None;
        self.flicker_reported = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_mean_luma_samples_subgrid() {
        let (w, h) = (32, 16);
        let mut luma = vec![50u8; w * h];
        assert_eq!(FlickerDetector::compute_mean_luma(&luma, w, h), 50);

        for i in (0..(w * h)).step_by(16) {
            luma[i] = 150;
        }
        assert_eq!(FlickerDetector::compute_mean_luma(&luma, w, h), 150);
    }

    #[test]
    fn flicker_detector_reports_only_on_transition() {
        let mut detector = FlickerDetector::new();

        // Baseline frame
        assert_eq!(detector.observe_mean(100), None);
        assert!(!detector.is_flicker_reported());

        // Jump within threshold (20 <= 24)
        assert_eq!(detector.observe_mean(120), None);
        assert!(!detector.is_flicker_reported());

        // Jump exceeding threshold (120 -> 150 = 30 > 24)
        assert_eq!(
            detector.observe_mean(150),
            Some(FlickerEvent::FlickerStarted { jump: 30 })
        );
        assert!(detector.is_flicker_reported());

        // Continuing flicker (150 -> 190 = 40 > 24): no new event / no log spam
        assert_eq!(detector.observe_mean(190), None);
        assert!(detector.is_flicker_reported());

        // Recovery (190 -> 195 = 5 <= 24)
        assert_eq!(detector.observe_mean(195), Some(FlickerEvent::FlickerStopped));
        assert!(!detector.is_flicker_reported());
    }
}
