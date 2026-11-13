//! The rules that decide what the UI is allowed to claim.
//!
//! These are ports of the pure functions in `apps/desktop/shared/`. Each one
//! exists because the shipped app had a specific defect where a surface asserted
//! a state that had not been established, and the fix was made testable by
//! isolating it as a pure function. Re-deriving them inline is how the same bug
//! comes back.

use phonecam_protocol::resolution_label;

/* ------------------------------------------------------------------ */
/* The one status headline                                             */
/* ------------------------------------------------------------------ */

/// The tone of a status surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusTone {
    Ok,
    Warn,
    Danger,
    Neutral,
    Accent,
}

/// What the status headline needs to know.
#[derive(Debug, Clone)]
pub struct StatusInput {
    /// The loopback socket to the core.
    pub connected: bool,
    /// The phase the core last reported.
    pub phase: String,
    /// Whether a phone is paired and connected.
    pub has_device: bool,
    /// Whether a phone has been paired before, even if it is not here now.
    ///
    /// The difference decides the detail line: a first-run user has to be told
    /// to pair, and a returning user whose phone is asleep has to be told to
    /// wake it. Telling the second user to pair again sends them to redo the one
    /// step they have already completed.
    pub has_remembered_phone: bool,
    pub error: Option<(String, String)>,
    pub device_name: Option<String>,
    /// Resolution the decoder is actually producing.
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusSummary {
    pub tone: StatusTone,
    /// Four words at most: the answer to "is it working?".
    pub headline: String,
    /// One clause of supporting fact. Never a repeat of the headline.
    pub detail: Option<String>,
    /// Whether the headline is a problem the user has to act on.
    pub attention: bool,
}

/// The one sentence the app says about itself.
///
/// Priority is by what blocks the user, not by what is technically wrong:
/// the service being unreachable blocks everything; a hard error is the reason
/// the last attempt failed; no phone means there is nothing to stream; and only
/// then does success get a word.
///
/// The virtual camera is deliberately **not** part of the headline. It is a
/// publish destination the user sets once, and making it a peer of "is the
/// picture flowing" is what produced the original contradiction where the app
/// said "Camera ready" next to "No phone".
pub fn summarise_status(input: &StatusInput) -> StatusSummary {
    if !input.connected {
        // "PhoneCam is restarting it" would be a lie: the core is started once
        // at app startup and never respawned. What continues in this state is
        // the retry.
        return StatusSummary {
            tone: StatusTone::Danger,
            headline: "Service not reachable".into(),
            detail: Some("PhoneCam is retrying automatically.".into()),
            attention: true,
        };
    }

    // A phone that is streaming is the success case, and it outranks a stale
    // error: the core clears its own error on a successful start, but a start
    // can also succeed after an error that arrived from a previous attempt.
    if input.phase == "streaming" && input.has_device {
        // Only the stream's *identity* is stated here, never its live numbers.
        // Frame rate and latency used to appear in this line and again on the
        // preview overlay a second apart, so the header read "30 fps" while the
        // overlay read "29.8 fps" — which looks like a bug rather than like two
        // samplings of one counter.
        return StatusSummary {
            tone: StatusTone::Ok,
            headline: "Live".into(),
            detail: input
                .resolution
                .clone()
                .or_else(|| input.device_name.clone()),
            attention: false,
        };
    }

    if let Some((_code, message)) = &input.error {
        return StatusSummary {
            tone: StatusTone::Danger,
            headline: "Stream failed".into(),
            detail: Some(message.clone()),
            attention: true,
        };
    }

    if !input.has_device {
        return StatusSummary {
            tone: StatusTone::Neutral,
            headline: "No phone connected".into(),
            detail: Some(if input.has_remembered_phone {
                "Open PhoneCam on your phone and it reconnects on its own.".into()
            } else {
                "Pair one to get started.".into()
            }),
            attention: false,
        };
    }

    if input.phase == "connecting" {
        return StatusSummary {
            tone: StatusTone::Accent,
            headline: "Starting…".into(),
            detail: input.device_name.clone(),
            attention: false,
        };
    }

    if input.phase == "reconnecting" {
        return StatusSummary {
            tone: StatusTone::Warn,
            headline: "Reconnecting".into(),
            detail: Some(match &input.device_name {
                Some(name) => format!("Waiting for {name}."),
                None => "Waiting for the phone.".into(),
            }),
            attention: true,
        };
    }

    StatusSummary {
        tone: StatusTone::Neutral,
        headline: "Ready to stream".into(),
        detail: input.device_name.clone(),
        attention: false,
    }
}

/* ------------------------------------------------------------------ */
/* The virtual camera                                                  */
/* ------------------------------------------------------------------ */

/// Which USB surface the Setup card renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbReadiness {
    ToolsMissing,
    NoDevice,
    Device,
    Unknown,
}

/// The honest USB state, from two independent facts.
///
/// Missing tools win over a listed device: without adb a device list cannot be
/// trusted, and the fix is different from "plug the cable in". With no state at
/// all the answer is `Unknown`, never `ToolsMissing`: absence of evidence is not
/// evidence that the install is broken.
pub fn usb_readiness(adb_available: Option<bool>, device_count: usize) -> UsbReadiness {
    match adb_available {
        Some(false) => UsbReadiness::ToolsMissing,
        _ => {
            if device_count > 0 {
                UsbReadiness::Device
            } else if adb_available.is_none() {
                UsbReadiness::Unknown
            } else {
                UsbReadiness::NoDevice
            }
        }
    }
}

/// The virtual camera's status chip.
///
/// Order matters: a failure is named first, then a running camera by name, then
/// a registered-but-off camera as "Off" — **not** "Not installed", because a
/// registered camera that is switched off is installed, and the old label said
/// otherwise on every fresh startup.
pub fn vcam_chip_label(state: Option<&str>, registered: bool, name: Option<&str>) -> String {
    if state == Some("error") {
        return "Failed to register".into();
    }
    if state == Some("running") {
        return name.unwrap_or("PhoneCam Camera").to_string();
    }
    if registered {
        return "Off".into();
    }
    "Not installed".into()
}

/// The "N apps open" summary, or `None` when there is nothing honest to say.
///
/// Only while the camera is running and carrying a stream, and only when at
/// least one app has it open: "0 apps open" beside a live picture is a
/// non-sequitur that reads as a complaint.
pub fn vcam_consumer_summary(
    state: Option<&str>,
    streaming: bool,
    consumers: u32,
) -> Option<String> {
    if state != Some("running") || !streaming || consumers == 0 {
        return None;
    }
    Some(if consumers == 1 {
        "1 app open".to_string()
    } else {
        format!("{consumers} apps open")
    })
}

/// The toggle's own detail line.
///
/// Different from [`vcam_consumer_summary`] on purpose: the toggle is a control,
/// so while the count is a measurement it states it even when zero (the user is
/// deciding whether to leave it on), whereas the status chip stays quiet.
///
/// A zero is only a measurement while streaming: `consumers` is written by a
/// ~1 s polling loop and defaults to 0, so a 0 means either "genuinely nobody"
/// or "not measured yet", and the two are indistinguishable.
pub fn vcam_usage_detail(state: Option<&str>, streaming: bool, consumers: u32) -> Option<String> {
    if state != Some("running") {
        return None;
    }
    if consumers > 0 {
        return Some(format!("{consumers} using it"));
    }
    if streaming {
        return Some("no one using it".into());
    }
    None
}

/* ------------------------------------------------------------------ */
/* Readouts                                                            */
/* ------------------------------------------------------------------ */

/// The throughput grid, formatted for a stream in `phase`.
///
/// Outside `streaming` every field reads the placeholder, because a rate is a
/// measurement only while the stream it measures is running. The core always
/// sends a fully populated stats object whose live-rate fields are zeroed the
/// moment a stream ends, so the guard that used to exist here — "is stats
/// non-null?" — printed a confident `0.0` and `0 Mbps` for a stream that was not
/// there.
pub struct ThroughputReadout {
    pub fps: String,
    pub bitrate: String,
    pub latency: String,
    pub dropped: String,
}

pub fn throughput_readout(
    stats: Option<&phonecam_protocol::ConnectionStats>,
    phase: &str,
    empty: &str,
) -> ThroughputReadout {
    match stats {
        Some(stats) if phase == "streaming" => ThroughputReadout {
            fps: format!("{:.1}", stats.fps),
            bitrate: format!("{:.1} Mbps", stats.bitrate as f64 / 1_000_000.0),
            // `-1` is the core's "no round trip measured yet" sentinel, not a
            // measurement of minus one millisecond.
            latency: if stats.latency_ms < 0.0 {
                empty.to_string()
            } else {
                format!("{:.0} ms", stats.latency_ms)
            },
            dropped: format!("{:.2}%", stats.dropped_percent),
        },
        _ => ThroughputReadout {
            fps: empty.to_string(),
            bitrate: empty.to_string(),
            latency: empty.to_string(),
            dropped: empty.to_string(),
        },
    }
}

/// Whether the phone honoured the resolution we asked for.
///
/// Compares the *label* (the shorter side), not the raw string, because a phone
/// with a sideways sensor reports `1080x1920` for a request of `1920x1080` —
/// the same 1080p frame. The old string comparison announced a downgrade that
/// had not happened and named the same resolution on both sides of the
/// sentence.
pub struct ResolutionComparison {
    pub downgraded: bool,
    pub requested_label: String,
    pub actual_label: String,
}

pub fn compare_resolution(actual: &str, requested: &str) -> Option<ResolutionComparison> {
    if actual.is_empty() || requested.is_empty() || requested == "auto" {
        return None;
    }
    let requested_label = resolution_label(requested);
    let actual_label = resolution_label(actual);
    Some(ResolutionComparison {
        downgraded: actual_label != requested_label,
        requested_label,
        actual_label,
    })
}

/* ------------------------------------------------------------------ */
/* Device identity                                                     */
/* ------------------------------------------------------------------ */

/// The identity fields that were actually reported. `None` means "not reported",
/// and a field that was not reported is **absent from the UI** — never blank,
/// and never "Android 0".
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceFacts {
    pub model: Option<String>,
    pub android_version: Option<i32>,
}

pub fn reported_device_facts(model: Option<&str>, android_version: Option<i32>) -> DeviceFacts {
    let model = model.filter(|m| !m.is_empty()).map(|m| m.to_string());
    let android_version = android_version.filter(|&v| v > 0);
    DeviceFacts {
        model,
        android_version,
    }
}

/// True only when a phone is genuinely connected.
///
/// The regression this fixes is `core.device != null` reading true when there
/// was no core at all.
pub fn is_phone_connected(device_present: Option<bool>) -> bool {
    device_present.unwrap_or(false)
}

/* ------------------------------------------------------------------ */
/* Notice reconciliation                                               */
/* ------------------------------------------------------------------ */

/// The standing notices that a core state has made false.
///
/// A notice describes the moment it was posted. When the fact it names later
/// stops being true the notice has to go with it, or the app keeps asserting a
/// state that is over. Three notices had no such sweep: the reconnecting banner
/// outlived its retry and its button stopped a stream that had already stopped;
/// the core-error banner outlived the failure and sat over a healthy
/// connection; and the quality banner outlived the stream it described.
///
/// The pairing notices are deliberately **not** covered: a successful pairing
/// emits its confirmation before the phone's control channel is up, so the core
/// briefly reports no device while the notice is the user's confirmation.
/// Sweeping on "no device" would flash it away.
pub fn stale_notice_keys(keys: &[String], phase: &str, has_error: bool) -> Vec<String> {
    let mut stale = Vec::new();

    if phase != "reconnecting" && keys.iter().any(|k| k == "reconnecting") {
        stale.push("reconnecting".to_string());
    }
    if phase != "streaming" && keys.iter().any(|k| k == "quality-changed") {
        stale.push("quality-changed".to_string());
    }
    if !has_error {
        for key in keys {
            if let Some(rest) = key.strip_prefix("core-error:") {
                stale.push(format!("core-error:{rest}"));
            }
        }
    }
    stale
}

/* ------------------------------------------------------------------ */
/* Start policy                                                        */
/* ------------------------------------------------------------------ */

/// Why a start was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartRefusal {
    Allowed,
    AlreadyStreaming,
    StartInFlight,
}

/// When the desktop may ask the phone to start streaming.
///
/// The rule is about *time*, not the phase alone: the phase is reported by the
/// phone and lags the request by a round trip, so a phase check by itself still
/// lets a second start through in the window that matters. Two concurrent dials
/// produce two video connections, the second replaces the first, and the loser's
/// teardown closes the winner's socket — a stream that starts and stops with no
/// error on either side.
pub fn can_start_stream(phase: &str, last_start_at_ms: u64, now_ms: u64) -> StartRefusal {
    if phase == "streaming" {
        return StartRefusal::AlreadyStreaming;
    }
    // An error is the phone or the core saying the previous attempt is over, so
    // a retry is a new attempt. Without this the Retry button on a failure
    // banner would be inert for the length of the cooldown.
    if phase == "error" {
        return StartRefusal::Allowed;
    }
    // `last_start_at_ms` is 0 before the first ever start, so no special case is
    // needed: the elapsed time is then the age of the epoch.
    if last_start_at_ms != 0 && now_ms.saturating_sub(last_start_at_ms) < 2500 {
        return StartRefusal::StartInFlight;
    }
    StartRefusal::Allowed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::EMPTY_VALUE;

    fn input() -> StatusInput {
        StatusInput {
            connected: true,
            phase: "idle".into(),
            has_device: false,
            has_remembered_phone: false,
            error: None,
            device_name: None,
            resolution: None,
        }
    }

    #[test]
    fn an_unreachable_service_blocks_everything_else() {
        let mut i = input();
        i.connected = false;
        i.phase = "streaming".to_string();
        i.has_device = true;
        let s = summarise_status(&i);
        assert_eq!(s.headline, "Service not reachable");
        assert!(s.attention);
        assert_eq!(s.tone, StatusTone::Danger);
    }

    #[test]
    fn streaming_outranks_a_stale_error() {
        let mut i = input();
        i.phase = "streaming".to_string();
        i.has_device = true;
        i.error = Some(("stream_failed".into(), "boom".into()));
        assert_eq!(summarise_status(&i).headline, "Live");
    }

    #[test]
    fn streaming_states_the_resolution_not_the_live_numbers() {
        let mut i = input();
        i.phase = "streaming".to_string();
        i.has_device = true;
        i.resolution = Some("1280x720".into());
        let s = summarise_status(&i);
        assert_eq!(s.headline, "Live");
        assert_eq!(s.detail.as_deref(), Some("1280x720"));
    }

    #[test]
    fn a_returning_user_is_not_told_to_pair_again() {
        let mut i = input();
        i.has_remembered_phone = true;
        let s = summarise_status(&i);
        assert_eq!(s.headline, "No phone connected");
        assert_eq!(
            s.detail.as_deref(),
            Some("Open PhoneCam on your phone and it reconnects on its own.")
        );

        i.has_remembered_phone = false;
        assert_eq!(
            summarise_status(&i).detail.as_deref(),
            Some("Pair one to get started.")
        );
    }

    #[test]
    fn reconnecting_names_the_phone() {
        let mut i = input();
        i.phase = "reconnecting".to_string();
        i.has_device = true;
        i.device_name = Some("Redmi Note 11".into());
        let s = summarise_status(&i);
        assert_eq!(s.headline, "Reconnecting");
        assert_eq!(s.detail.as_deref(), Some("Waiting for Redmi Note 11."));
        assert!(s.attention);
    }

    #[test]
    fn ready_is_the_resting_state() {
        let mut i = input();
        i.has_device = true;
        i.device_name = Some("Pixel".into());
        let s = summarise_status(&i);
        assert_eq!(s.headline, "Ready to stream");
        assert!(!s.attention);
    }

    #[test]
    fn usb_readiness_keeps_the_two_facts_separate() {
        // Missing tools win even when a device is listed.
        assert_eq!(usb_readiness(Some(false), 1), UsbReadiness::ToolsMissing);
        assert_eq!(usb_readiness(Some(true), 1), UsbReadiness::Device);
        // The normal Wi-Fi case: tools present, no cable.
        assert_eq!(usb_readiness(Some(true), 0), UsbReadiness::NoDevice);
        // No state at all is unknown, never "tools missing".
        assert_eq!(usb_readiness(None, 0), UsbReadiness::Unknown);
        assert_eq!(usb_readiness(None, 2), UsbReadiness::Device);
    }

    #[test]
    fn a_registered_but_off_camera_is_off_not_absent() {
        assert_eq!(
            vcam_chip_label(Some("error"), true, None),
            "Failed to register"
        );
        assert_eq!(
            vcam_chip_label(Some("running"), true, Some("PhoneCam Camera")),
            "PhoneCam Camera"
        );
        assert_eq!(vcam_chip_label(Some("stopped"), true, None), "Off");
        assert_eq!(
            vcam_chip_label(Some("stopped"), false, None),
            "Not installed"
        );
        assert_eq!(vcam_chip_label(None, false, None), "Not installed");
    }

    #[test]
    fn consumer_counts_stay_quiet_when_they_are_not_measurements() {
        assert_eq!(vcam_consumer_summary(Some("running"), true, 0), None);
        assert_eq!(vcam_consumer_summary(Some("running"), false, 2), None);
        assert_eq!(
            vcam_consumer_summary(Some("running"), true, 1),
            Some("1 app open".into())
        );
        assert_eq!(
            vcam_consumer_summary(Some("running"), true, 3),
            Some("3 apps open".into())
        );

        assert_eq!(vcam_usage_detail(Some("running"), false, 0), None);
        assert_eq!(
            vcam_usage_detail(Some("running"), true, 0),
            Some("no one using it".into())
        );
        assert_eq!(
            vcam_usage_detail(Some("running"), true, 2),
            Some("2 using it".into())
        );
        assert_eq!(vcam_usage_detail(Some("stopped"), true, 0), None);
    }

    #[test]
    fn throughput_reads_the_placeholder_outside_streaming() {
        let stats = phonecam_protocol::ConnectionStats::default();
        let r = throughput_readout(Some(&stats), "idle", EMPTY_VALUE);
        assert_eq!(r.fps, EMPTY_VALUE);
        assert_eq!(r.bitrate, EMPTY_VALUE);
        assert_eq!(r.dropped, EMPTY_VALUE);
    }

    #[test]
    fn a_measured_zero_is_still_a_measurement() {
        let mut stats = phonecam_protocol::ConnectionStats::default();
        stats.fps = 0.0;
        stats.bitrate = 0;
        let r = throughput_readout(Some(&stats), "streaming", EMPTY_VALUE);
        assert_eq!(
            r.fps, "0.0",
            "a running stream at zero fps is a measurement, not unknown"
        );
        assert_eq!(r.bitrate, "0.0 Mbps");
    }

    #[test]
    fn an_unmeasured_latency_is_never_printed_as_minus_one() {
        let mut stats = phonecam_protocol::ConnectionStats::default();
        stats.latency_ms = -1.0;
        let r = throughput_readout(Some(&stats), "streaming", EMPTY_VALUE);
        assert_eq!(r.latency, EMPTY_VALUE);
    }

    #[test]
    fn a_sideways_sensor_is_not_a_downgrade() {
        let c = compare_resolution("1080x1920", "1920x1080").unwrap();
        assert!(
            !c.downgraded,
            "the same 1080p frame in the other orientation"
        );
        assert_eq!(c.requested_label, "1080p");
        assert_eq!(c.actual_label, "1080p");
    }

    #[test]
    fn a_real_downgrade_is_reported() {
        let c = compare_resolution("1280x720", "1920x1080").unwrap();
        assert!(c.downgraded);
        assert_eq!(c.requested_label, "1080p");
        assert_eq!(c.actual_label, "720p");
    }

    #[test]
    fn auto_and_empty_are_not_comparable() {
        assert!(compare_resolution("1280x720", "auto").is_none());
        assert!(compare_resolution("", "1920x1080").is_none());
        assert!(compare_resolution("1280x720", "").is_none());
    }

    #[test]
    fn unreported_device_fields_are_absent() {
        let f = reported_device_facts(Some(""), Some(0));
        assert_eq!(f.model, None);
        assert_eq!(f.android_version, None);

        let f = reported_device_facts(Some("2201117TG"), Some(33));
        assert_eq!(f.model.as_deref(), Some("2201117TG"));
        assert_eq!(f.android_version, Some(33));
    }

    #[test]
    fn notices_are_retracted_when_their_fact_is_over() {
        let keys = vec![
            "reconnecting".to_string(),
            "quality-changed".to_string(),
            "core-error:x".to_string(),
            "pair-ok".to_string(),
        ];
        // While reconnecting with an error standing, only the quality banner is
        // stale: it claims a running stream, and nothing is streaming.
        let partial = stale_notice_keys(&keys, "reconnecting", true);
        assert_eq!(partial, vec!["quality-changed".to_string()]);

        let stale = stale_notice_keys(&keys, "connected", false);
        assert!(stale.contains(&"reconnecting".to_string()));
        assert!(stale.contains(&"quality-changed".to_string()));
        assert!(stale.contains(&"core-error:x".to_string()));
        // Pairing success is never swept on a missing device.
        assert!(!stale.contains(&"pair-ok".to_string()));
    }

    #[test]
    fn the_start_cooldown_covers_the_phase_lag() {
        assert_eq!(can_start_stream("connected", 0, 0), StartRefusal::Allowed);
        assert_eq!(
            can_start_stream("streaming", 0, 0),
            StartRefusal::AlreadyStreaming
        );
        assert_eq!(
            can_start_stream("connected", 1000, 2000),
            StartRefusal::StartInFlight
        );
        // At exactly 2500ms the boundary is `<`, so a start is allowed.
        assert_eq!(
            can_start_stream("connected", 1000, 3500),
            StartRefusal::Allowed
        );
        // An error is always retryable, so the Retry button is never inert.
        assert_eq!(can_start_stream("error", 5000, 5001), StartRefusal::Allowed);
    }
}
