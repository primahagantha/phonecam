//! Publishing cadence and frame pacing.
//!
//! Ported from `CoreSession::PublishLoop` in `native/core/src/core_session.cpp`
//! (upstream commit ec19949).
//!
//! The H.264 decoder produces frames in bursts per group of pictures (e.g. 15
//! frames every half second), and network arrival exhibits jitter. Publishing
//! frames immediately upon decode causes severe micro-stutter and cadence
//! irregularities (e.g. 27.8 fps instead of 30 fps, or long quiet gaps).
//!
//! `FramePacer` stabilizes publishing cadence to steady 30 fps by pacing frames
//! along their presentation timestamp (PTS) media clock across group boundaries.

use std::time::{Duration, Instant};

/// Default target frames per second.
pub const TARGET_FPS: u32 = 30;

/// Frame duration at 30 fps in microseconds (33,333 us).
pub const TARGET_FRAME_DURATION_US: u64 = 1_000_000 / TARGET_FPS as u64;

/// Action returned by `FramePacer::pace_at` / `pace`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaceAction {
    /// Frame should be published immediately.
    Ready,
    /// Publisher should wait for the given duration to preserve smooth 30fps cadence.
    Sleep(Duration),
    /// Clock re-anchored (e.g. initial frame, large delta, camera switch, or gap).
    Reanchored,
}

/// A media-clock pacer that stabilizes frame publishing cadence.
#[derive(Debug, Clone)]
pub struct FramePacer {
    target_frame_duration: Duration,
    anchor_at: Option<Instant>,
    anchor_pts_us: u64,
    last_pts_us: u64,
    last_publish_at: Option<Instant>,
    paced: bool,
}

impl Default for FramePacer {
    fn default() -> Self {
        Self::new(TARGET_FPS)
    }
}

impl FramePacer {
    /// Creates a new pacer targeting `fps` (defaults to 30 if 0).
    pub fn new(fps: u32) -> Self {
        let fps = if fps > 0 { fps } else { TARGET_FPS };
        let frame_duration_us = 1_000_000 / fps as u64;
        Self {
            target_frame_duration: Duration::from_micros(frame_duration_us),
            anchor_at: None,
            anchor_pts_us: 0,
            last_pts_us: 0,
            last_publish_at: None,
            paced: false,
        }
    }

    pub fn target_frame_duration(&self) -> Duration {
        self.target_frame_duration
    }

    pub fn is_paced(&self) -> bool {
        self.paced
    }

    pub fn reset(&mut self) {
        self.anchor_at = None;
        self.anchor_pts_us = 0;
        self.last_pts_us = 0;
        self.last_publish_at = None;
        self.paced = false;
    }

    /// Evaluates pacing against a specified wall-clock `now`.
    ///
    /// Handles media clock anchoring, 30fps pacing delay, and re-anchoring
    /// on camera switches, encoder restarts, or reconnect gaps.
    pub fn pace_at(&mut self, pts_us: u64, now: Instant) -> PaceAction {
        self.last_pts_us = pts_us;

        // First frame: establish anchor.
        if !self.paced || self.anchor_at.is_none() {
            self.anchor_at = Some(now);
            self.anchor_pts_us = pts_us;
            self.last_publish_at = Some(now);
            self.paced = true;
            return PaceAction::Reanchored;
        }

        // If PTS is continuously 0 (e.g. stream without timestamps),
        // pace on fixed target interval (33.33ms) from previous publish.
        if pts_us == 0 && self.anchor_pts_us == 0 {
            let last = self.last_publish_at.unwrap_or(now);
            let target = last + self.target_frame_duration;
            if target > now {
                let wait = target - now;
                self.last_publish_at = Some(target);
                return PaceAction::Sleep(wait);
            } else {
                self.last_publish_at = Some(now);
                return PaceAction::Ready;
            }
        }

        let anchor_at = self.anchor_at.unwrap();
        let delta = pts_us as i64 - self.anchor_pts_us as i64;

        // PTS backwards jump or jump > 5s: re-anchor.
        if !(0..=5_000_000).contains(&delta) {
            self.anchor_at = Some(now);
            self.anchor_pts_us = pts_us;
            self.last_publish_at = Some(now);
            return PaceAction::Reanchored;
        }

        let target = anchor_at + Duration::from_micros(delta as u64);

        // Forward jump > 120ms (4 frames at 30 fps):
        // Re-anchor rather than sleep through it (prevents long stalls on switch).
        let forward_threshold = Duration::from_micros(120_000);
        // Backward lag > 250ms (8 frames):
        // Re-anchor rather than burst-flush.
        let backward_threshold = Duration::from_micros(250_000);

        if target > now + forward_threshold || (now > target && (now - target) > backward_threshold)
        {
            self.anchor_at = Some(now);
            self.anchor_pts_us = pts_us;
            self.last_publish_at = Some(now);
            return PaceAction::Reanchored;
        }

        if target > now {
            let wait = target - now;
            self.last_publish_at = Some(target);
            PaceAction::Sleep(wait)
        } else {
            self.last_publish_at = Some(now);
            PaceAction::Ready
        }
    }

    /// Evaluates pacing against the current wall clock (`Instant::now()`).
    pub fn pace(&mut self, pts_us: u64) -> PaceAction {
        self.pace_at(pts_us, Instant::now())
    }

    /// Waits (sleeps) if necessary to maintain pacing, then returns.
    pub fn wait_for_pace(&mut self, pts_us: u64) {
        if let PaceAction::Sleep(duration) = self.pace(pts_us) {
            std::thread::sleep(duration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_frame_reanchors() {
        let mut pacer = FramePacer::new(30);
        let now = Instant::now();
        assert_eq!(pacer.pace_at(1_000, now), PaceAction::Reanchored);
        assert!(pacer.is_paced());
    }

    #[test]
    fn frame_burst_is_paced_to_target_cadence() {
        let mut pacer = FramePacer::new(30);
        let start = Instant::now();

        // Frame 1 arrives at t = 0 with PTS = 0
        assert_eq!(pacer.pace_at(0, start), PaceAction::Reanchored);

        // Frame 2 arrives almost immediately (in burst, e.g. at t = 2ms) with PTS = 33_333 us
        let arrival_f2 = start + Duration::from_millis(2);
        match pacer.pace_at(33_333, arrival_f2) {
            PaceAction::Sleep(dur) => {
                // Should sleep approx 33.333 - 2 = 31.333 ms
                assert!(dur >= Duration::from_millis(30) && dur <= Duration::from_millis(33));
            }
            other => panic!("expected sleep for burst frame, got {other:?}"),
        }

        // Frame 3 arrives at t = 66_666 us: should be ready
        let arrival_f3 = start + Duration::from_micros(66_666);
        assert_eq!(pacer.pace_at(66_666, arrival_f3), PaceAction::Ready);
    }

    #[test]
    fn forward_jump_over_120ms_reanchors_instead_of_long_sleep() {
        let mut pacer = FramePacer::new(30);
        let start = Instant::now();
        pacer.pace_at(1_000, start);

        // Forward jump of 200ms (> 120ms threshold): camera switch or encoder jump
        let now = start + Duration::from_millis(10);
        let jump_pts = 1_000 + 200_000;
        assert_eq!(pacer.pace_at(jump_pts, now), PaceAction::Reanchored);
    }

    #[test]
    fn backwards_jump_reanchors() {
        let mut pacer = FramePacer::new(30);
        let start = Instant::now();
        pacer.pace_at(100_000, start);

        let now = start + Duration::from_millis(33);
        assert_eq!(pacer.pace_at(50_000, now), PaceAction::Reanchored);
    }

    #[test]
    fn pts_zero_paces_at_fixed_cadence() {
        let mut pacer = FramePacer::new(30);
        let start = Instant::now();

        assert_eq!(pacer.pace_at(0, start), PaceAction::Reanchored);

        // Frame 2 arrives 10ms later with pts 0
        let t2 = start + Duration::from_millis(10);
        match pacer.pace_at(0, t2) {
            PaceAction::Sleep(wait) => {
                assert!(wait >= Duration::from_millis(20) && wait <= Duration::from_millis(24));
            }
            other => panic!("expected sleep, got {other:?}"),
        }
    }
}
