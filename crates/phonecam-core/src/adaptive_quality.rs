//! The adaptive quality ladder (PRD Â§48, docs Â§11).
//!
//! The controller lives in the core, not the phone, because the evidence is
//! split across both ends: the phone owns the encoder and the thermal sensor,
//! the desktop owns the round trip and the decode path. A single decision has to
//! be made where both are visible, and putting it here means the phone keeps one
//! job (encode what it is asked to encode) and two controllers cannot fight over
//! the same encoder.
//!
//! The ladder is deliberately asymmetric: quick to drop, slow to climb back.
//! A drop protects a stream the user is watching; a climb is a guess about a
//! link that has only just recovered.

use phonecam_protocol::ThermalState;

/// One rung of the ladder, ordered worst-first so `+1` is always an improvement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QualityRung {
    P480_30 = 0,
    P720_30 = 1,
    P1080_30 = 2,
    P1080_60 = 3,
}

pub const QUALITY_RUNG_COUNT: usize = 4;

impl QualityRung {
    pub const ALL: [QualityRung; QUALITY_RUNG_COUNT] = [
        QualityRung::P480_30,
        QualityRung::P720_30,
        QualityRung::P1080_30,
        QualityRung::P1080_60,
    ];

    pub fn resolution(self) -> &'static str {
        match self {
            Self::P480_30 => "640x480",
            Self::P720_30 => "1280x720",
            Self::P1080_30 | Self::P1080_60 => "1920x1080",
        }
    }

    pub fn fps(self) -> u32 {
        match self {
            Self::P1080_60 => 60,
            _ => 30,
        }
    }

    pub fn bitrate(self) -> u64 {
        match self {
            Self::P480_30 => 1_500_000,
            Self::P720_30 => 4_000_000,
            Self::P1080_30 => 8_000_000,
            Self::P1080_60 => 12_000_000,
        }
    }

    /// The label shown in the UI. A silent quality change reads as a bug, so
    /// every change is announced with this.
    pub fn label(self) -> &'static str {
        match self {
            Self::P480_30 => "480p",
            Self::P720_30 => "720p",
            Self::P1080_30 => "1080p30",
            Self::P1080_60 => "1080p60",
        }
    }

    pub fn step_down(self) -> Option<Self> {
        match self {
            Self::P480_30 => None,
            Self::P720_30 => Some(Self::P480_30),
            Self::P1080_30 => Some(Self::P720_30),
            Self::P1080_60 => Some(Self::P1080_30),
        }
    }

    pub fn step_up(self) -> Option<Self> {
        match self {
            Self::P480_30 => Some(Self::P720_30),
            Self::P720_30 => Some(Self::P1080_30),
            Self::P1080_30 => Some(Self::P1080_60),
            Self::P1080_60 => None,
        }
    }

    /// The rung a stream at `resolution` x `fps` corresponds to.
    ///
    /// 1080p30 and 1080p60 share a resolution string, so fps is the
    /// discriminator.
    pub fn for_stream(resolution: &str, fps: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|r| r.resolution() == resolution && r.fps() == fps)
    }

    /// The highest rung a camera can actually serve.
    ///
    /// Used as the ladder's ceiling. Returns `None` when nothing matches, which
    /// the caller treats as "no ceiling" rather than "no support".
    pub fn highest_supported(resolutions: &[String], frame_rates: &[u32]) -> Option<Self> {
        Self::ALL
            .into_iter()
            .rev()
            .find(|r| {
                resolutions.iter().any(|s| s == r.resolution())
                    && frame_rates.contains(&r.fps())
            })
    }
}

impl Default for QualityRung {
    fn default() -> Self {
        Self::P1080_30
    }
}

/* ------------------------------------------------------------------ */
/* Thresholds                                                          */
/* ------------------------------------------------------------------ */

/// How long evidence must persist before stepping down.
pub const STEP_DOWN_DWELL_MS: i64 = 10_000;
/// How long a clean stream must run before stepping back up.
pub const STEP_UP_DWELL_MS: i64 = 30_000;
/// Above this drop rate the stream is struggling.
pub const BAD_DROP_PERCENT: f64 = 3.0;
/// Below this drop rate, and under the RTT ceiling, the stream is clean.
pub const CLEAN_DROP_PERCENT: f64 = 0.5;
/// A round trip at or under this is clean.
pub const CLEAN_RTT_US: i64 = 60_000;
/// Twice the clean threshold counts as a struggling link.
pub const SLOW_RTT_US: i64 = CLEAN_RTT_US * 2;

/* ------------------------------------------------------------------ */
/* Inputs and outputs                                                  */
/* ------------------------------------------------------------------ */

/// One measurement of the link, from both ends.
#[derive(Debug, Clone)]
pub struct QualitySample {
    /// The rung actually encoding, from the phone's own report.
    pub current: QualityRung,
    /// The highest rung the user's settings and the camera allow.
    pub max_rung: QualityRung,
    /// Phone-reported encoder drops.
    pub dropped_percent: f64,
    /// Measured round trip in microseconds. `0` means unmeasured, which is
    /// treated as "no evidence" rather than "a perfect link".
    pub rtt_us: i64,
    pub thermal: ThermalState,
    /// The decoder could not keep up since the last tick.
    pub decode_struggling: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualityAction {
    Hold,
    StepDown(QualityRung),
    StepUp(QualityRung),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityDecision {
    pub action: QualityAction,
    /// Shown to the user. Every automatic change is announced, because a silent
    /// quality change reads as a bug.
    pub reason: &'static str,
}

impl QualityDecision {
    fn hold() -> Self {
        Self { action: QualityAction::Hold, reason: "" }
    }
    pub fn target(&self) -> Option<QualityRung> {
        match self.action {
            QualityAction::Hold => None,
            QualityAction::StepDown(r) | QualityAction::StepUp(r) => Some(r),
        }
    }
    pub fn is_automatic(&self) -> bool {
        !matches!(self.action, QualityAction::Hold)
    }
}

/* ------------------------------------------------------------------ */
/* Controller                                                          */
/* ------------------------------------------------------------------ */

/// The ladder controller. Only touched from the stats thread, so it needs no
/// lock; the evidence it consumes is written by other threads and is atomic.
#[derive(Debug)]
pub struct AdaptiveQuality {
    enabled: bool,
    current: QualityRung,
    /// `-1` means "no streak running". Not `0`, because tests drive from t=0.
    bad_since_ms: i64,
    clean_since_ms: i64,
    /// The rung a change was asked for, until the phone confirms it.
    ///
    /// A request is not a fact until the phone reports the new rung, and the
    /// statistics that arrive in between still describe the **old** stream.
    /// Acting on them asked for a second rung before the first was confirmed:
    /// the pending target was overwritten, the announcement for the rung the
    /// phone actually reached was dropped, and the ladder walked two steps for
    /// one period of trouble.
    pending: Option<QualityRung>,
}

impl Default for AdaptiveQuality {
    fn default() -> Self {
        Self::new()
    }
}

impl AdaptiveQuality {
    pub fn new() -> Self {
        Self {
            enabled: true,
            current: QualityRung::default(),
            bad_since_ms: -1,
            clean_since_ms: -1,
            pending: None,
        }
    }

    /// The rung a change was asked for and not yet confirmed, if any.
    pub fn pending(&self) -> Option<QualityRung> {
        self.pending
    }

    /// Records the phone's confirmation, clearing the pending target.
    ///
    /// Called when the phone's own `state` reports the rung that was requested.
    pub fn confirm(&mut self, rung: QualityRung) {
        if self.pending == Some(rung) {
            self.pending = None;
        }
    }

    pub fn current(&self) -> QualityRung {
        self.current
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// `"auto"` enables the ladder; `"manual"` pins the user's choice.
    ///
    /// Streaks keep tracking while disabled, so switching back to auto does not
    /// start from a stale clean run.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    fn reset_streaks(&mut self) {
        self.bad_since_ms = -1;
        self.clean_since_ms = -1;
    }

    /// One controller tick. `now_ms` must be monotonic, never wall clock.
    pub fn observe(&mut self, sample: &QualitySample, now_ms: i64) -> QualityDecision {
        self.current = sample.current;

        let thermal_critical = sample.thermal.is_critical();

        let dropped = sample.dropped_percent > BAD_DROP_PERCENT;
        // A round trip of 0 is "not measured", which is no evidence either way.
        // Reading it as a perfect link made an unmeasured stream look clean.
        let slow = sample.rtt_us > 0 && sample.rtt_us > SLOW_RTT_US;
        let thermal = sample.thermal.is_stressed();

        let struggling = dropped || slow || thermal || sample.decode_struggling;
        if !struggling {
            self.bad_since_ms = -1;
        } else if self.bad_since_ms < 0 {
            self.bad_since_ms = now_ms;
        }

        let clean = sample.dropped_percent < CLEAN_DROP_PERCENT
            && (sample.rtt_us == 0 || sample.rtt_us < CLEAN_RTT_US)
            && !thermal
            && !sample.decode_struggling;
        if !clean {
            self.clean_since_ms = -1;
        } else if self.clean_since_ms < 0 {
            self.clean_since_ms = now_ms;
        }

        if !self.enabled {
            return QualityDecision::hold();
        }

        // One change at a time: waiting costs at most one dwell and keeps the
        // request and the announcement paired.
        if self.pending.is_some() {
            return QualityDecision::hold();
        }

        let can_step_down = sample.current != QualityRung::P480_30;
        let can_step_up = sample.current != QualityRung::P1080_60
            && sample.current < sample.max_rung;

        // Critical thermal skips the dwell: the platform is about to clamp the
        // encoder anyway, so a controlled step keeps the stream alive instead of
        // letting it stall.
        if thermal_critical {
            if let Some(target) = sample.current.step_down() {
                self.reset_streaks();
                self.pending = Some(target);
                return QualityDecision {
                    action: QualityAction::StepDown(target),
                    reason: "Your phone is running hot. Lowering quality to keep the stream stable.",
                };
            }
        }

        if can_step_down
            && self.bad_since_ms >= 0
            && now_ms - self.bad_since_ms >= STEP_DOWN_DWELL_MS
        {
            if let Some(target) = sample.current.step_down() {
                let reason = if dropped || sample.decode_struggling {
                    "Frames are being dropped. Lowering quality to keep the stream smooth."
                } else {
                    "The link is struggling. Lowering quality to keep the stream smooth."
                };
                self.reset_streaks();
                self.pending = Some(target);
                return QualityDecision { action: QualityAction::StepDown(target), reason };
            }
        }

        if can_step_up
            && self.clean_since_ms >= 0
            && now_ms - self.clean_since_ms >= STEP_UP_DWELL_MS
        {
            if let Some(target) = sample.current.step_up() {
                self.reset_streaks();
                self.pending = Some(target);
                return QualityDecision {
                    action: QualityAction::StepUp(target),
                    reason: "The stream has been stable. Raising quality.",
                };
            }
        }

        QualityDecision::hold()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(current: QualityRung) -> QualitySample {
        QualitySample {
            current,
            max_rung: QualityRung::P1080_60,
            dropped_percent: 0.0,
            rtt_us: 20_000,
            thermal: ThermalState::None,
            decode_struggling: false,
        }
    }

    #[test]
    fn rung_table_matches_the_spec() {
        assert_eq!(QualityRung::P480_30.resolution(), "640x480");
        assert_eq!(QualityRung::P480_30.bitrate(), 1_500_000);
        assert_eq!(QualityRung::P720_30.resolution(), "1280x720");
        assert_eq!(QualityRung::P720_30.bitrate(), 4_000_000);
        assert_eq!(QualityRung::P1080_30.resolution(), "1920x1080");
        assert_eq!(QualityRung::P1080_30.fps(), 30);
        assert_eq!(QualityRung::P1080_30.bitrate(), 8_000_000);
        assert_eq!(QualityRung::P1080_60.fps(), 60);
        assert_eq!(QualityRung::P1080_60.bitrate(), 12_000_000);
        assert_eq!(QualityRung::P1080_30.label(), "1080p30");
        assert_eq!(QualityRung::P1080_60.label(), "1080p60");
    }

    #[test]
    fn ladder_order_is_worst_first() {
        assert!(QualityRung::P480_30 < QualityRung::P720_30);
        assert!(QualityRung::P720_30 < QualityRung::P1080_30);
        assert!(QualityRung::P1080_30 < QualityRung::P1080_60);
    }

    #[test]
    fn step_down_and_up_are_symmetric_and_bounded() {
        assert_eq!(QualityRung::P480_30.step_down(), None);
        assert_eq!(QualityRung::P1080_60.step_up(), None);
        for rung in QualityRung::ALL {
            if let Some(down) = rung.step_down() {
                assert_eq!(down.step_up(), Some(rung));
            }
            if let Some(up) = rung.step_up() {
                assert_eq!(up.step_down(), Some(rung));
            }
        }
    }

    #[test]
    fn for_stream_distinguishes_1080p30_from_1080p60() {
        assert_eq!(QualityRung::for_stream("1920x1080", 30), Some(QualityRung::P1080_30));
        assert_eq!(QualityRung::for_stream("1920x1080", 60), Some(QualityRung::P1080_60));
        assert_eq!(QualityRung::for_stream("1280x720", 30), Some(QualityRung::P720_30));
        assert_eq!(QualityRung::for_stream("640x480", 30), Some(QualityRung::P480_30));
        // A size the ladder does not have is not a rung.
        assert_eq!(QualityRung::for_stream("3840x2160", 30), None);
        assert_eq!(QualityRung::for_stream("1280x720", 24), None);
    }

    #[test]
    fn highest_supported_picks_the_top_matching_rung() {
        let resolutions = vec!["1920x1080".to_string(), "1280x720".to_string()];
        assert_eq!(
            QualityRung::highest_supported(&resolutions, &[30]),
            Some(QualityRung::P1080_30)
        );
        assert_eq!(
            QualityRung::highest_supported(&resolutions, &[30, 60]),
            Some(QualityRung::P1080_60)
        );
        assert_eq!(
            QualityRung::highest_supported(&["640x480".to_string()], &[30]),
            Some(QualityRung::P480_30)
        );
        assert_eq!(QualityRung::highest_supported(&[], &[30]), None);
    }

    #[test]
    fn a_clean_stream_holds() {
        let mut q = AdaptiveQuality::new();
        let d = q.observe(&sample(QualityRung::P1080_30), 0);
        assert_eq!(d.action, QualityAction::Hold);
    }

    #[test]
    fn stepping_down_requires_the_full_dwell() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P1080_30);
        s.dropped_percent = 10.0;

        // The streak starts on the first bad sample but nothing moves yet.
        assert_eq!(q.observe(&s, 0).action, QualityAction::Hold);
        // One millisecond before the dwell elapses: still holding.
        assert_eq!(q.observe(&s, STEP_DOWN_DWELL_MS - 1).action, QualityAction::Hold);
        // At the dwell: one step down, never two.
        assert_eq!(
            q.observe(&s, STEP_DOWN_DWELL_MS).action,
            QualityAction::StepDown(QualityRung::P720_30)
        );
    }

    #[test]
    fn one_change_at_a_time() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P1080_60);
        s.dropped_percent = 50.0;

        assert_eq!(q.observe(&s, 0).action, QualityAction::Hold);
        let d = q.observe(&s, STEP_DOWN_DWELL_MS);
        assert_eq!(d.action, QualityAction::StepDown(QualityRung::P1080_30));

        // The request is not a fact yet: further ticks hold until the phone
        // reports the new rung, so one period of trouble cannot walk two rungs.
        assert_eq!(q.pending(), Some(QualityRung::P1080_30));
        assert_eq!(q.observe(&s, STEP_DOWN_DWELL_MS + 1).action, QualityAction::Hold);
        assert_eq!(q.observe(&s, STEP_DOWN_DWELL_MS * 2).action, QualityAction::Hold);

        // Once confirmed, the ladder may act again.
        q.confirm(QualityRung::P1080_30);
        assert_eq!(q.pending(), None);
    }

    #[test]
    fn critical_thermal_skips_the_dwell() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P1080_30);
        s.thermal = ThermalState::Critical;
        // No dwell has elapsed at all, yet it steps immediately.
        assert_eq!(
            q.observe(&s, 0).action,
            QualityAction::StepDown(QualityRung::P720_30)
        );
    }

    #[test]
    fn severe_thermal_needs_the_dwell_but_still_counts_as_evidence() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P1080_30);
        s.thermal = ThermalState::Severe;
        assert_eq!(q.observe(&s, 0).action, QualityAction::Hold);
        assert_eq!(
            q.observe(&s, STEP_DOWN_DWELL_MS).action,
            QualityAction::StepDown(QualityRung::P720_30)
        );
    }

    #[test]
    fn stepping_up_requires_thirty_clean_seconds() {
        let mut q = AdaptiveQuality::new();
        let s = sample(QualityRung::P720_30);
        assert_eq!(q.observe(&s, 0).action, QualityAction::Hold);
        assert_eq!(q.observe(&s, STEP_UP_DWELL_MS - 1).action, QualityAction::Hold);
        assert_eq!(
            q.observe(&s, STEP_UP_DWELL_MS).action,
            QualityAction::StepUp(QualityRung::P1080_30)
        );
    }

    #[test]
    fn unmeasured_rtt_is_not_a_perfect_link() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P1080_30);
        // A 0 us round trip means "never measured". It must not count as clean,
        // or a stream nobody has measured would climb the ladder.
        s.rtt_us = 0;
        s.dropped_percent = 0.0;
        q.observe(&s, 0);
        // It also must not count as struggling, so it holds rather than drops.
        assert_eq!(q.observe(&s, STEP_UP_DWELL_MS).action, QualityAction::StepUp(QualityRung::P1080_60));
    }

    #[test]
    fn a_slow_link_steps_down() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P1080_30);
        s.rtt_us = SLOW_RTT_US + 1;
        assert_eq!(q.observe(&s, 0).action, QualityAction::Hold);
        assert_eq!(
            q.observe(&s, STEP_DOWN_DWELL_MS).action,
            QualityAction::StepDown(QualityRung::P720_30)
        );
    }

    #[test]
    fn an_rtt_at_the_clean_threshold_is_not_clean() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P720_30);
        s.rtt_us = CLEAN_RTT_US; // strictly less than is required
        q.observe(&s, 0);
        assert_eq!(q.observe(&s, STEP_UP_DWELL_MS).action, QualityAction::Hold);
    }

    #[test]
    fn decode_struggling_steps_down_with_the_drop_reason() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P1080_30);
        s.decode_struggling = true;
        q.observe(&s, 0);
        let d = q.observe(&s, STEP_DOWN_DWELL_MS);
        assert_eq!(d.action, QualityAction::StepDown(QualityRung::P720_30));
        assert!(d.reason.contains("dropped"), "reason was {:?}", d.reason);
    }

    #[test]
    fn manual_mode_never_moves_the_ladder() {
        let mut q = AdaptiveQuality::new();
        q.set_enabled(false);
        let mut s = sample(QualityRung::P1080_30);
        s.dropped_percent = 90.0;
        s.thermal = ThermalState::Critical;
        assert_eq!(q.observe(&s, 0).action, QualityAction::Hold);
        assert_eq!(q.observe(&s, 100_000).action, QualityAction::Hold);
    }

    #[test]
    fn the_ceiling_caps_the_climb_but_never_the_drop() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P480_30);
        s.max_rung = QualityRung::P720_30;

        // At the ceiling, a clean stream holds instead of climbing to 1080p.
        assert_eq!(q.observe(&s, 0).action, QualityAction::Hold);
        assert_eq!(q.observe(&s, STEP_UP_DWELL_MS).action, QualityAction::StepUp(QualityRung::P720_30));

        // Confirm the climb before the next decision, since one change at a time
        // means an unconfirmed target holds the ladder.
        q.confirm(QualityRung::P720_30);

        // A bad stream still steps down from below the ceiling.
        let mut bad = sample(QualityRung::P720_30);
        bad.max_rung = QualityRung::P720_30;
        bad.dropped_percent = 20.0;
        q.observe(&bad, 0);
        assert_eq!(
            q.observe(&bad, STEP_DOWN_DWELL_MS).action,
            QualityAction::StepDown(QualityRung::P480_30)
        );
    }

    #[test]
    fn the_bottom_rung_cannot_step_down() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P480_30);
        s.dropped_percent = 99.0;
        s.thermal = ThermalState::Critical;
        assert_eq!(q.observe(&s, 0).action, QualityAction::Hold);
        assert_eq!(q.observe(&s, 999_999).action, QualityAction::Hold);
    }

    #[test]
    fn a_recovering_stream_does_not_climb_from_a_stale_clean_run() {
        let mut q = AdaptiveQuality::new();
        let mut s = sample(QualityRung::P720_30);
        // Clean for long enough to be eligible...
        q.observe(&s, 0);
        // ...then one bad sample resets the clean streak.
        s.dropped_percent = 10.0;
        q.observe(&s, STEP_UP_DWELL_MS - 1);
        // Now clean again: the 30 s must restart from here.
        s.dropped_percent = 0.0;
        q.observe(&s, STEP_UP_DWELL_MS);
        assert_eq!(
            q.observe(&s, STEP_UP_DWELL_MS + STEP_UP_DWELL_MS - 1).action,
            QualityAction::Hold
        );
        assert_eq!(
            q.observe(&s, STEP_UP_DWELL_MS * 2).action,
            QualityAction::StepUp(QualityRung::P1080_30)
        );
    }
}



