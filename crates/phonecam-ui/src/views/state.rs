//! The view model: everything the window draws, in one snapshot.
//!
//! The shipped renderer kept this shape deliberately, and the reason still
//! holds. The loopback socket and the tray both push updates that must land in
//! one coherent snapshot, and video packets must **not** re-render the window —
//! they go straight to the preview surface. Keeping the packet path out of the
//! view model is what keeps preview latency low.

use phonecam_protocol::{
    CapabilitiesPayload, CoreState, LogLevel, PhoneStats, QualityMode, ScreenMode, StreamPhase,
};

// Re-exported so callers can name a log level without depending on the protocol
// crate directly.
pub use phonecam_protocol::LogLevel as LogLevelExport;
use crate::theme::rules::{DeviceFacts, StatusSummary};

/// A failure banner.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub id: u64,
    pub code: FailureCode,
    /// Placeholder values substituted into the copy.
    pub values: Vec<(String, String)>,
    /// Stable key so repeats of the same notice collapse instead of stacking.
    pub dedupe_key: String,
}

/// Every failure the desktop can name (§38 copy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureCode {
    CoreUnreachable,
    CoreReconnecting,
    NoDevice,
    Denied,
    TokenExpired,
    Busy,
    VersionMismatch,
    BadRequest,
    ApprovalPending,
    StreamEnded,
    Generic,
    UsbMissing,
    UsbUnauthorized,
    VcamUnavailable,
    ResolutionDowngraded,
    QualityChanged,
    DecoderFailed,
    DecoderUnsupported,
    PairOk,
    PhoneAppMissing,
}

impl FailureCode {
    /// The banner's title. Sentence case, no em-dashes.
    pub fn title(self) -> &'static str {
        match self {
            Self::CoreUnreachable => "Service not reachable",
            Self::CoreReconnecting => "Reconnecting to your phone",
            Self::NoDevice => "No phone connected",
            Self::Denied => "That pairing was declined",
            Self::TokenExpired => "That code expired",
            Self::Busy => "Another phone is using this PC",
            Self::VersionMismatch => "This phone needs a newer PhoneCam",
            Self::BadRequest => "The phone sent something unreadable",
            Self::ApprovalPending => "Waiting for your approval",
            Self::StreamEnded => "The stream stopped",
            Self::Generic => "Something went wrong",
            Self::UsbMissing => "Cable tools missing",
            Self::UsbUnauthorized => "Approve USB debugging on your phone",
            Self::VcamUnavailable => "The virtual camera is not working",
            Self::ResolutionDowngraded => "Using a lower resolution",
            Self::QualityChanged => "Quality changed",
            Self::DecoderFailed => "No preview",
            Self::DecoderUnsupported => "This build cannot show the preview",
            Self::PairOk => "Phone paired",
            Self::PhoneAppMissing => "PhoneCam app not installed",
        }
    }

    /// The supporting sentence. Says what happened, or what to do next.
    pub fn body(self, values: &[(String, String)]) -> String {
        let get = |key: &str| {
            values
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        match self {
            Self::CoreUnreachable => "PhoneCam is retrying automatically.".into(),
            Self::CoreReconnecting => {
                "The last frame stays on the virtual camera while PhoneCam retries.".into()
            }
            Self::NoDevice => {
                "Your phone scans one code, and this PC remembers it from then on.".into()
            }
            Self::Denied => "Approve the next request on this PC to connect.".into(),
            Self::TokenExpired => "Codes work once and last five minutes.".into(),
            Self::Busy => "Stop the stream on the other phone, then try again.".into(),
            Self::VersionMismatch => {
                "Update the PhoneCam app on your phone to match this PC.".into()
            }
            Self::BadRequest => "Try connecting again from your phone.".into(),
            Self::ApprovalPending => "A prompt is open on this PC.".into(),
            Self::StreamEnded => "The phone stopped sending. You can start it again.".into(),
            Self::Generic => get("message"),
            Self::UsbMissing => {
                "PhoneCam ships Google's Android platform tools, but they are missing from this copy. Reinstall PhoneCam, or install them yourself and check again.".into()
            }
            Self::UsbUnauthorized => {
                "USB connected. Approve \"USB debugging\" on your phone.".into()
            }
            Self::VcamUnavailable => get("message"),
            Self::ResolutionDowngraded => format!(
                "{} is not available on this phone. Using {} instead.",
                get("requested"),
                get("actual")
            ),
            Self::QualityChanged => format!("Now streaming at {}.", get("rung")),
            Self::DecoderFailed => {
                "The virtual camera is unaffected; this window cannot show the picture.".into()
            }
            Self::DecoderUnsupported => {
                "The virtual camera is unaffected by this.".into()
            }
            Self::PairOk => format!("{} is connected.", get("name")),
            Self::PhoneAppMissing => {
                let msg = get("message");
                if msg.is_empty() {
                    "The PhoneCam app is not installed on this Android phone. Install PhoneCam on your phone, then reconnect.".into()
                } else {
                    msg
                }
            }
        }
    }

    /// The one action the banner offers, if any.
    pub fn action(self) -> Option<NoticeAction> {
        Some(match self {
            Self::CoreUnreachable => NoticeAction::RefreshState,
            Self::CoreReconnecting => NoticeAction::StopStream,
            Self::Denied | Self::TokenExpired | Self::NoDevice => NoticeAction::ArmPairing,
            Self::StreamEnded => NoticeAction::StartStream,
            Self::UsbMissing | Self::UsbUnauthorized => NoticeAction::ConnectUsb,
            Self::PhoneAppMissing => NoticeAction::InstallPhoneApp,
            Self::VersionMismatch | Self::BadRequest | Self::Busy => NoticeAction::RefreshState,
            Self::VcamUnavailable => NoticeAction::RestartVcam,
            // The prompt itself is the affordance.
            Self::ApprovalPending => NoticeAction::Dismiss,
            Self::ResolutionDowngraded | Self::QualityChanged => NoticeAction::Dismiss,
            Self::Generic | Self::DecoderFailed | Self::DecoderUnsupported | Self::PairOk => {
                return None
            }
        })
    }

    /// Whether the banner's action is the primary control for its state.
    ///
    /// Only two states may carry a primary button, so the app never offers the
    /// same job three times in two visual languages.
    pub fn action_is_primary(self) -> bool {
        matches!(self, Self::StreamEnded | Self::NoDevice | Self::PhoneAppMissing)
    }

    pub fn tone(self) -> crate::theme::Tone {
        use crate::theme::Tone;
        match self {
            Self::PhoneAppMissing
            | Self::CoreUnreachable
            | Self::Denied
            | Self::VersionMismatch
            | Self::BadRequest
            | Self::Busy
            | Self::VcamUnavailable
            | Self::UsbMissing => Tone::Danger,
            Self::CoreReconnecting
            | Self::TokenExpired
            | Self::UsbUnauthorized
            | Self::ResolutionDowngraded
            | Self::DecoderFailed
            | Self::DecoderUnsupported => Tone::Warn,
            Self::NoDevice
            | Self::StreamEnded
            | Self::Generic
            | Self::ApprovalPending => Tone::Neutral,
            Self::QualityChanged | Self::PairOk => Tone::Accent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeAction {
    RefreshState,
    StopStream,
    ArmPairing,
    StartStream,
    ConnectUsb,
    RestartVcam,
    Dismiss,
    InstallPhoneApp,
}

/// One line in the diagnostics log.
#[derive(Debug, Clone, PartialEq)]
pub struct LogLine {
    pub ts_ms: i64,
    pub level: LogLevel,
    pub source: LogSource,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogSource {
    Core,
    App,
    Renderer,
    Media,
    Phone,
}

impl LogSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Core => "Service",
            Self::App => "PhoneCam",
            Self::Renderer => "App",
            Self::Media => "media",
            Self::Phone => "phone",
        }
    }
}

/// A phone asking to be let in.
#[derive(Debug, Clone, PartialEq)]
pub struct PairingPrompt {
    pub device_id: String,
    pub name: String,
    pub model: String,
    pub raised_at_ms: i64,
}

/// Persisted desktop settings.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// On by default: the camera is the product. A false default made a fresh
    /// install hide the camera from OBS and Meet.
    pub vcam_enabled: bool,
    pub output_mirror: bool,
    pub output_rotation: u32,
    pub output_width: u32,
    pub output_height: u32,
    /// Whether the published size follows the stream.
    ///
    /// On by default: matching the stream makes the frame path a straight copy,
    /// while upscaling costs time and adds no detail.
    pub output_follows_stream: bool,
    pub camera: String,
    /// 720p30 rather than "auto": auto resolves to the largest advertised size,
    /// which measured at 31% app CPU and 130% camera-HAL CPU on a Redmi Note 11.
    pub resolution: String,
    pub fps: u32,
    pub bitrate: u64,
    pub quality_mode: QualityMode,
    pub screen_mode: ScreenMode,
    pub log_level: LogLevel,
    pub always_on_top: bool,
    pub window_x: Option<f32>,
    pub window_y: Option<f32>,
    pub window_width: f32,
    pub window_height: f32,
    pub window_maximized: bool,
    /// Whether the window renders the live preview.
    pub preview_enabled: bool,
    /// Whether a phone has been paired here before, **not** whether one is here
    /// now. It decides whether the empty state says "pair one" or "wake yours".
    pub last_device_id: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            vcam_enabled: true,
            output_mirror: false,
            output_rotation: 0,
            output_width: 1920,
            output_height: 1080,
            output_follows_stream: true,
            camera: "back".into(),
            resolution: "1280x720".into(),
            fps: 30,
            bitrate: 4_000_000,
            quality_mode: QualityMode::Auto,
            screen_mode: ScreenMode::KeepOn,
            log_level: LogLevel::Info,
            always_on_top: false,
            window_x: None,
            window_y: None,
            window_width: 1280.0,
            window_height: 840.0,
            window_maximized: false,
            preview_enabled: true,
            last_device_id: None,
        }
    }
}

/// The loopback connection's own state, separate from the core's.
#[derive(Debug, Clone, PartialEq)]
pub struct CoreStatus {
    pub connected: bool,
    pub last_error: Option<String>,
    pub reconnect_attempt: u32,
    pub bytes_in: u64,
    pub connects: u32,
}

impl Default for CoreStatus {
    fn default() -> Self {
        Self {
            connected: false,
            last_error: None,
            reconnect_attempt: 0,
            bytes_in: 0,
            connects: 0,
        }
    }
}

/// Everything the window draws.
#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub core: Option<CoreState>,
    pub status: CoreStatus,
    pub settings: Settings,
    pub capabilities: Option<CapabilitiesPayload>,
    pub notices: Vec<Notice>,
    pub logs: Vec<LogLine>,
    pub log_filter_level: Option<phonecam_protocol::LogLevel>,
    pub log_filter_source: Option<LogSource>,
    pub pairing_prompt: Option<PairingPrompt>,
    pub toast: Option<String>,
    pub toast_expires_at_ms: Option<i64>,
    /// The size of the picture this window is showing, if any.
    pub frame_size: Option<(u32, u32)>,
    pub phone_stats: Option<PhoneStats>,
    /// A camera the user asked for but the phone has not confirmed yet.
    ///
    /// Kept so the control shows the user's own choice while the switch is in
    /// flight: the phone keeps reporting the old camera until the new one is up,
    /// and reading that back made a working switch look ignored.
    pub pending_camera_id: Option<String>,
    /// The primary local Wi-Fi / LAN IP address for phone connection.
    pub local_ip: Option<String>,
}

impl AppState {
    pub fn phase(&self) -> &str {
        match &self.core {
            Some(core) => core.phase.as_str(),
            None => "idle",
        }
    }

    /// The phase to *display*, which is not the core's phase when the socket is
    /// down: a core we cannot reach has no phase worth showing.
    pub fn display_phase(&self) -> String {
        if !self.status.connected {
            return "connecting".into();
        }
        self.phase().to_string()
    }

    pub fn has_device(&self) -> bool {
        self.core.as_ref().map(|c| c.device.is_some()).unwrap_or(false)
    }

    pub fn device_name(&self) -> Option<String> {
        self.core.as_ref().and_then(|c| c.device.as_ref().map(|d| d.name.clone()))
    }

    pub fn is_streaming(&self) -> bool {
        self.phase() == "streaming"
    }

    pub fn status_summary(&self) -> StatusSummary {
        crate::theme::rules::summarise_status(
            &crate::theme::rules::StatusInput {
                connected: self.status.connected,
                phase: self.display_phase(),
                has_device: self.has_device(),
                has_remembered_phone: self.settings.last_device_id.is_some(),
                error: self
                    .core
                    .as_ref()
                    .and_then(|c| c.error.as_ref())
                    .map(|e| {
                        let lower = e.message.to_lowercase();
                        let clean = if lower.contains("no service started") || lower.contains("not found") {
                            "Could not start PhoneCam on the phone. Please open the PhoneCam app on your phone and try again.".to_string()
                        } else if lower.contains("couldn't start video encoding") || lower.contains("encoder_failed") {
                            "The phone could not start video encoding. Try switching camera or lowering the resolution.".to_string()
                        } else {
                            e.message.clone()
                        };
                        (e.code.clone(), clean)
                    }),
                device_name: self.device_name(),
                resolution: self
                    .core
                    .as_ref()
                    .and_then(|c| c.phone.as_ref())
                    .map(|p| p.actual_resolution.clone())
                    .filter(|s| !s.is_empty()),
            },
        )
    }

    pub fn device_facts(&self) -> DeviceFacts {
        let device = self.core.as_ref().and_then(|c| c.device.as_ref());
        crate::theme::rules::reported_device_facts(
            device.map(|d| d.model.as_str()),
            device.map(|d| d.android_version),
        )
    }

    pub fn vcam_state(&self) -> Option<&str> {
        self.core.as_ref().and_then(|c| c.vcam.as_ref()).map(|v| v.state.as_str())
    }

    pub fn vcam_registered(&self) -> bool {
        self.core
            .as_ref()
            .and_then(|c| c.vcam.as_ref())
            .map(|v| v.registered)
            .unwrap_or(false)
    }

    pub fn vcam_consumers(&self) -> u32 {
        self.core
            .as_ref()
            .and_then(|c| c.vcam.as_ref())
            .map(|v| v.consumers)
            .unwrap_or(0)
    }

    /// Adds a notice, replacing one with the same key in place.
    ///
    /// Replacing rather than stacking is what stops a repeated failure from
    /// filling the window with identical banners.
    pub fn add_notice(&mut self, code: FailureCode, values: Vec<(String, String)>, key: String) {
        if let Some(existing) = self.notices.iter_mut().find(|n| n.dedupe_key == key) {
            existing.code = code;
            existing.values = values;
            return;
        }
        let id = self.notices.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        self.notices.push(Notice { id, code, values, dedupe_key: key });
    }

    pub fn dismiss_notice(&mut self, key: &str) {
        self.notices.retain(|n| n.dedupe_key != key);
    }

    pub fn dismiss_notice_by_id(&mut self, id: u64) {
        self.notices.retain(|n| n.id != id);
    }

    pub fn has_notice(&self, code: FailureCode) -> bool {
        self.notices.iter().any(|n| n.code == code)
    }

    /// Applies the sweep: notices whose fact is over are removed.
    pub fn reconcile_notices(&mut self) {
        let keys: Vec<String> = self.notices.iter().map(|n| n.dedupe_key.clone()).collect();
        let has_error = self
            .core
            .as_ref()
            .map(|c| c.error.is_some())
            .unwrap_or(false);
        for key in crate::theme::rules::stale_notice_keys(&keys, self.phase(), has_error) {
            self.dismiss_notice(&key);
        }
    }

    pub fn push_log(&mut self, level: LogLevel, source: LogSource, message: impl Into<String>) {
        const MAX_LOGS: usize = 500;
        self.logs.push(LogLine {
            ts_ms: now_ms(),
            level,
            source,
            message: message.into(),
        });
        if self.logs.len() > MAX_LOGS {
            let excess = self.logs.len() - MAX_LOGS;
            self.logs.drain(..excess);
        }
    }

    /// Shows a transient toast message that expires after `ttl_ms` milliseconds.
    pub fn set_toast(&mut self, msg: impl Into<String>, ttl_ms: i64) {
        self.toast = Some(msg.into());
        self.toast_expires_at_ms = Some(now_ms().saturating_add(ttl_ms.max(0)));
    }

    /// Clears the active toast if `now_ms` has reached its expiration timestamp.
    pub fn expire_toast_if_due(&mut self, now_ms: i64) -> bool {
        if let Some(expires_at) = self.toast_expires_at_ms {
            if now_ms >= expires_at {
                self.toast = None;
                self.toast_expires_at_ms = None;
                return true;
            }
        }
        false
    }
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The label for a stream phase, used by the tray and the throughput grid.
pub fn stream_phase_label(phase: StreamPhase) -> &'static str {
    match phase {
        StreamPhase::Idle => "Idle",
        StreamPhase::Connecting => "Connecting",
        StreamPhase::Connected => "Connected",
        StreamPhase::Streaming => "Streaming",
        StreamPhase::Reconnecting => "Reconnecting",
        StreamPhase::Error => "Error",
    }
}

/// The thermal chip's wording. `None` and `Light` are not worth a chip.
pub fn thermal_label(thermal: phonecam_protocol::ThermalState) -> Option<(&'static str, bool)> {
    use phonecam_protocol::ThermalState as T;
    Some(match thermal {
        T::None => return None,
        T::Light => return None,
        T::Moderate => ("Warm", false),
        T::Severe => ("Hot", false),
        T::Critical => ("Very hot", true),
        T::Emergency => ("Overheating", true),
        T::Shutdown => ("Too hot, stopped", true),
    })
}

/// `M:SS`, for the pairing countdown.
pub fn format_duration(ms: i64) -> String {
    let total_seconds = (ms.max(0) as f64 / 1000.0).ceil() as i64;
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}

/// Bytes as a human-readable size, 1-2 decimal places.
pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// `HH:MM:SS.mmm` for a log line.
pub fn format_log_time(ts_ms: i64) -> String {
    let total_seconds = ts_ms.div_euclid(1000);
    let millis = ts_ms.rem_euclid(1000);
    let seconds = total_seconds.rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        seconds / 3600,
        (seconds % 3600) / 60,
        seconds % 60,
        millis
    )
}

/// A resolution's label, e.g. `1920x1080` → `1080p`.
pub fn resolution_label_of(value: &str) -> String {
    phonecam_protocol::resolution_label(value)
}

/// Formats phone battery percentage and charging state when reported (`>= 0`).
pub fn format_battery(battery: i32, charging: bool) -> Option<String> {
    if battery < 0 {
        return None;
    }
    Some(if charging {
        format!("{battery}% (charging)")
    } else {
        format!("{battery}%")
    })
}

/// Picks a sensible default bitrate for a resolution and frame rate.
pub fn bitrate_for(resolution: &str, fps: u32, current: u64) -> u64 {
    if resolution == "auto" {
        return current;
    }
    let Some((w, h)) = phonecam_protocol::parse_resolution(resolution) else {
        return current;
    };
    let short = w.min(h);
    if short >= 2000 {
        16_000_000
    } else if short >= 1400 {
        12_000_000
    } else if short >= 1000 {
        if fps >= 50 { 12_000_000 } else { 8_000_000 }
    } else if short >= 700 {
        4_000_000
    } else {
        1_500_000
    }
}

/// Human-readable label for a phone camera facing direction.
pub fn camera_facing_label(facing: phonecam_protocol::CameraFacing) -> &'static str {
    match facing {
        phonecam_protocol::CameraFacing::Back => "Back camera",
        phonecam_protocol::CameraFacing::Front => "Front camera",
        phonecam_protocol::CameraFacing::External => "External camera",
        phonecam_protocol::CameraFacing::Unknown => "Camera",
    }
}

/// Human-readable label for a phone screen mode.
pub fn screen_mode_label(mode: ScreenMode) -> &'static str {
    match mode {
        ScreenMode::KeepOn => "Keep on",
        ScreenMode::Dim => "Dim",
        ScreenMode::Black => "Off",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::effects::Tab;

    #[test]
    fn an_empty_state_says_it_has_no_phone() {
        let state = AppState::default();
        assert!(!state.has_device());
        assert_eq!(state.display_phase(), "connecting", "an unreachable service is not idle");
    }

    #[test]
    fn a_repeated_notice_replaces_in_place() {
        let mut state = AppState::default();
        state.add_notice(FailureCode::Generic, vec![], "core-error:x".into());
        state.add_notice(FailureCode::Generic, vec![], "core-error:x".into());
        assert_eq!(state.notices.len(), 1, "repeats must not stack");
    }

    #[test]
    fn distinct_notices_coexist() {
        let mut state = AppState::default();
        state.add_notice(FailureCode::NoDevice, vec![], "no-device".into());
        state.add_notice(FailureCode::UsbMissing, vec![], "usb".into());
        assert_eq!(state.notices.len(), 2);
        assert!(state.has_notice(FailureCode::NoDevice));
        state.dismiss_notice("no-device");
        assert!(!state.has_notice(FailureCode::NoDevice));
    }

    #[test]
    fn the_log_is_capped_and_keeps_the_tail() {
        let mut state = AppState::default();
        for i in 0..600 {
            state.push_log(LogLevel::Info, LogSource::Core, format!("line {i}"));
        }
        assert_eq!(state.logs.len(), 500);
        assert_eq!(state.logs.last().unwrap().message, "line 599");
        assert_eq!(state.logs.first().unwrap().message, "line 100");
    }

    #[test]
    fn durations_format_as_minutes_and_seconds() {
        assert_eq!(format_duration(0), "0:00");
        assert_eq!(format_duration(59_000), "0:59");
        assert_eq!(format_duration(60_000), "1:00");
        assert_eq!(format_duration(300_000), "5:00");
        // Rounds up, so the last partial second is shown rather than hidden.
        assert_eq!(format_duration(1_500), "0:02");
        assert_eq!(format_duration(-5), "0:00");
    }

    #[test]
    fn bytes_use_binary_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KiB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.00 GiB");
    }

    #[test]
    fn log_times_are_zero_padded_with_millis() {
        assert_eq!(format_log_time(0), "00:00:00.000");
        assert_eq!(format_log_time(3_661_007), "01:01:01.007");
    }

    #[test]
    fn thermal_chips_are_rationed() {
        use phonecam_protocol::ThermalState as T;
        assert_eq!(thermal_label(T::None), None);
        assert_eq!(thermal_label(T::Light), None);
        assert_eq!(thermal_label(T::Moderate), Some(("Warm", false)));
        assert_eq!(thermal_label(T::Critical), Some(("Very hot", true)));
    }

    #[test]
    fn tabs_are_in_job_order_and_cover_ctrl_1_to_4() {
        let indices: Vec<usize> = Tab::ALL.iter().map(|t| t.index()).collect();
        assert_eq!(indices, vec![0, 1, 2, 3]);
        assert_eq!(Tab::Setup.label(), "Setup");
        assert_eq!(Tab::Capture.label(), "Capture");
        assert_eq!(Tab::Publish.label(), "Publish");
    }

    #[test]
    fn every_failure_code_has_a_title_and_body_without_dashes() {
        for code in [
            FailureCode::CoreUnreachable,
            FailureCode::CoreReconnecting,
            FailureCode::NoDevice,
            FailureCode::Denied,
            FailureCode::TokenExpired,
            FailureCode::Busy,
            FailureCode::VersionMismatch,
            FailureCode::BadRequest,
            FailureCode::ApprovalPending,
            FailureCode::StreamEnded,
            FailureCode::Generic,
            FailureCode::UsbMissing,
            FailureCode::UsbUnauthorized,
            FailureCode::VcamUnavailable,
            FailureCode::ResolutionDowngraded,
            FailureCode::QualityChanged,
            FailureCode::DecoderFailed,
            FailureCode::DecoderUnsupported,
            FailureCode::PairOk,
            FailureCode::PhoneAppMissing,
        ] {
            let values = vec![
                ("message".to_string(), "x".to_string()),
                ("requested".to_string(), "1080p".to_string()),
                ("actual".to_string(), "720p".to_string()),
                ("rung".to_string(), "720p".to_string()),
                ("name".to_string(), "Redmi".to_string()),
            ];
            let title = code.title();
            let body = code.body(&values);
            assert!(!title.is_empty());
            assert!(!body.is_empty(), "{} has no body", title);
            for text in [title, body.as_str()] {
                assert!(!text.contains('\u{2014}'), "em-dash in {text:?}");
                assert!(!text.contains('\u{2013}'), "en-dash in {text:?}");
                assert!(!text.contains("undefined"), "undefined in {text:?}");
                assert!(!text.contains("NaN"), "NaN in {text:?}");
            }
        }
    }

    #[test]
    fn a_resolution_downgrade_names_both_sides() {
        let body = FailureCode::ResolutionDowngraded.body(&[
            ("requested".into(), "1080p".into()),
            ("actual".into(), "720p".into()),
        ]);
        assert!(body.contains("1080p"));
        assert!(body.contains("720p"));
    }

    #[test]
    fn toast_expires_when_due() {
        let mut state = AppState::default();
        assert!(state.toast_expires_at_ms.is_none());
        assert!(state.log_filter_level.is_none());
        assert!(state.log_filter_source.is_none());

        state.set_toast("Copied", 3_000);
        assert_eq!(state.toast.as_deref(), Some("Copied"));
        let expires_at = state.toast_expires_at_ms.unwrap();

        assert!(!state.expire_toast_if_due(expires_at - 1));
        assert!(state.toast.is_some());

        assert!(state.expire_toast_if_due(expires_at));
        assert!(state.toast.is_none());
        assert!(state.toast_expires_at_ms.is_none());
    }

    #[test]
    fn log_sources_have_expected_labels() {
        assert_eq!(LogSource::Media.label(), "media");
        assert_eq!(LogSource::Phone.label(), "phone");
    }

    #[test]
    fn preview_is_enabled_by_default() {
        assert!(Settings::default().preview_enabled);
    }

    #[test]
    fn battery_formatting_respects_reported_and_charging_state() {
        assert_eq!(format_battery(-1, false), None);
        assert_eq!(format_battery(-1, true), None);
        assert_eq!(format_battery(84, false).as_deref(), Some("84%"));
        assert_eq!(format_battery(84, true).as_deref(), Some("84% (charging)"));
        assert_eq!(format_battery(0, false).as_deref(), Some("0%"));
    }

    #[test]
    fn bitrate_for_scales_cleanly_by_resolution_and_fps() {
        assert_eq!(bitrate_for("auto", 30, 5_500_000), 5_500_000);
        assert_eq!(bitrate_for("640x480", 30, 4_000_000), 1_500_000);
        assert_eq!(bitrate_for("1280x720", 30, 8_000_000), 4_000_000);
        assert_eq!(bitrate_for("1920x1080", 30, 4_000_000), 8_000_000);
        assert_eq!(bitrate_for("1920x1080", 60, 4_000_000), 12_000_000);
        assert_eq!(bitrate_for("1080x1920", 30, 4_000_000), 8_000_000);
        assert_eq!(bitrate_for("3840x2160", 30, 4_000_000), 16_000_000);
    }

    #[test]
    fn camera_and_screen_labels_cover_all_variants() {
        use phonecam_protocol::CameraFacing;
        assert_eq!(camera_facing_label(CameraFacing::Back), "Back camera");
        assert_eq!(camera_facing_label(CameraFacing::Front), "Front camera");
        assert_eq!(camera_facing_label(CameraFacing::External), "External camera");
        assert_eq!(camera_facing_label(CameraFacing::Unknown), "Camera");

        assert_eq!(screen_mode_label(ScreenMode::KeepOn), "Keep on");
        assert_eq!(screen_mode_label(ScreenMode::Dim), "Dim");
        assert_eq!(screen_mode_label(ScreenMode::Black), "Off");
    }
}







