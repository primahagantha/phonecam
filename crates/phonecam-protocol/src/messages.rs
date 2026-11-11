//! Every message type on the wire (docs/protocol.md §3, §6, §9).
//!
//! The phone-facing messages are `#[deny]`-adjacent: a missing field is a
//! protocol break. Serde is configured to serialize `None` as absent rather than
//! `null` where the phone omits it, and to tolerate unknown fields, because a
//! newer phone may send more than this build knows.

use serde::{Deserialize, Serialize};

/* ------------------------------------------------------------------ */
/* Enums (§5.2, §5.3, §7.1)                                           */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ChannelRole {
    Control = 1,
    Video = 2,
}

impl ChannelRole {
    pub fn from_wire(value: u8) -> Self {
        match value {
            2 => Self::Video,
            _ => Self::Control,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AuthMode {
    /// Accepted only from `127.0.0.1`.
    Loopback = 0,
    /// One-time 32-hex-char token from the QR payload.
    PairingToken = 1,
    /// `deviceId:hexHmac`.
    DeviceSecret = 2,
    /// `deviceId:request:hexHmac` — raises a desktop approval prompt.
    PairRequest = 3,
}

impl AuthMode {
    pub fn from_wire(value: u8) -> Self {
        match value {
            1 => Self::PairingToken,
            2 => Self::DeviceSecret,
            3 => Self::PairRequest,
            _ => Self::Loopback,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HandshakeStatus {
    Ok = 0,
    /// Bad token / bad HMAC / replayed nonce.
    Denied = 1,
    VersionMismatch = 2,
    /// Malformed handshake.
    BadRequest = 3,
    /// Another, already-paired phone holds the session.
    Busy = 4,
    TokenExpired = 5,
    /// `pairRequest` raised; awaiting desktop approval.
    ApprovalPending = 6,
    /// Loopback mode from a non-loopback peer.
    UnauthorizedSource = 7,
}

impl HandshakeStatus {
    pub fn from_wire(value: u8) -> Self {
        match value {
            0 => Self::Ok,
            1 => Self::Denied,
            2 => Self::VersionMismatch,
            4 => Self::Busy,
            5 => Self::TokenExpired,
            6 => Self::ApprovalPending,
            7 => Self::UnauthorizedSource,
            _ => Self::BadRequest,
        }
    }
}

/// Bit flags on a video packet (§7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFlags(pub u8);

impl VideoFlags {
    pub const NONE: Self = Self(0);
    pub const KEYFRAME: Self = Self(1 << 0);
    pub const CODEC_CONFIG: Self = Self(1 << 1);
    pub const END_OF_STREAM: Self = Self(1 << 2);

    pub fn is_keyframe(self) -> bool {
        self.0 & (1 << 0) != 0
    }
    pub fn is_codec_config(self) -> bool {
        self.0 & (1 << 1) != 0
    }
    pub fn is_end_of_stream(self) -> bool {
        self.0 & (1 << 2) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VideoCodec {
    H264 = 1,
}

impl VideoCodec {
    pub fn from_wire(value: u8) -> Self {
        // H.264 is the only defined codec; anything else is still treated as
        // H.264 rather than rejected, matching the phone's own tolerance.
        let _ = value;
        Self::H264
    }
}

/* ------------------------------------------------------------------ */
/* Discovery (§3)                                                      */
/* ------------------------------------------------------------------ */

/// Desktop → broadcast (§3.1). Advertises a PC ready to accept a phone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DesktopBeacon {
    pub t: String,
    pub v: u8,
    pub role: String,
    pub server_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub pairing_open: bool,
    pub pairing_id: Option<String>,
    pub paired_device_ids: Vec<String>,
    /// A control channel is up, so the PC is taken. Advisory for auto-connect;
    /// the handshake's `busy` (§5.3) is the strict rule.
    pub busy: bool,
    pub version: String,
}

impl DesktopBeacon {
    pub fn new(
        server_id: impl Into<String>,
        name: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        pairing_open: bool,
        pairing_id: Option<String>,
        paired_device_ids: Vec<String>,
        busy: bool,
        version: impl Into<String>,
    ) -> Self {
        Self {
            t: "phonecam.beacon".into(),
            v: 1,
            role: "desktop".into(),
            server_id: server_id.into(),
            name: name.into(),
            host: host.into(),
            port,
            pairing_open,
            pairing_id,
            paired_device_ids,
            busy,
            version: version.into(),
        }
    }
}

/// Phone → broadcast (§3.2). Lets the desktop list nearby phones for UX.
///
/// Discovery is **unauthenticated by design** — any device can claim any name.
/// It is therefore a hint for the UI, never a credential.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhoneBeacon {
    pub t: String,
    pub v: u8,
    pub role: String,
    pub device_id: String,
    pub name: String,
    pub model: String,
    pub android_version: i32,
    pub app_version: String,
    pub streaming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveryProbe {
    pub t: String,
    pub v: u8,
    pub role: String,
}

impl DiscoveryProbe {
    pub fn phone() -> Self {
        Self { t: "phonecam.probe".into(), v: 1, role: "phone".into() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "t")]
pub enum DiscoveryMessage {
    #[serde(rename = "phonecam.beacon")]
    Beacon(serde_json::Value),
    #[serde(rename = "phonecam.probe")]
    Probe(DiscoveryProbe),
}

/* ------------------------------------------------------------------ */
/* Pairing (§4.1)                                                      */
/* ------------------------------------------------------------------ */

/// The QR payload the desktop renders and the phone scans.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QrPayload {
    pub protocol: String,
    pub version: u8,
    pub host: String,
    pub port: u16,
    /// 32 lowercase hex chars. Single use, TTL 5 minutes, never logged.
    pub token: String,
    pub name: String,
    #[serde(rename = "serverId")]
    pub server_id: String,
}

impl QrPayload {
    pub fn is_well_formed(&self) -> bool {
        self.protocol == "phonecam"
            && self.version == 1
            && !self.host.is_empty()
            && self.port > 0
            && self.token.len() == 32
            && self.token.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    }
}

/* ------------------------------------------------------------------ */
/* Capabilities (§6.3, PRD §44)                                        */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CameraFacing {
    Back,
    Front,
    External,
    Unknown,
}

impl CameraFacing {
    pub fn from_wire(value: &str) -> Self {
        match value {
            "back" => Self::Back,
            "front" => Self::Front,
            "external" => Self::External,
            _ => Self::Unknown,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Back => "back",
            Self::Front => "front",
            Self::External => "external",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CameraCapability {
    pub id: String,
    pub facing: CameraFacing,
    /// Encoded output sizes as `WxH`, largest first.
    pub resolutions: Vec<String>,
    /// Frame rates supported at the largest resolution, ascending.
    pub fps: Vec<u32>,
    pub torch: bool,
    pub focus: bool,
    pub zoom: bool,
    pub exposure: bool,
    #[serde(rename = "whiteBalance")]
    pub white_balance: bool,
    #[serde(rename = "minZoom")]
    pub min_zoom: f32,
    #[serde(rename = "maxZoom")]
    pub max_zoom: f32,
    /// Sensor rotation relative to the device's natural orientation.
    #[serde(rename = "orientationDegrees")]
    pub orientation_degrees: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceCapabilities {
    pub model: String,
    #[serde(rename = "androidVersion")]
    pub android_version: i32,
    #[serde(rename = "appVersion")]
    pub app_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapabilitiesPayload {
    pub device: DeviceCapabilities,
    pub cameras: Vec<CameraCapability>,
}

/* ------------------------------------------------------------------ */
/* Shared enums                                                        */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ScreenMode {
    KeepOn,
    Dim,
    Black,
}

impl ScreenMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::KeepOn => "keepOn",
            Self::Dim => "dim",
            Self::Black => "black",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum QualityMode {
    /// The core may move the adaptive ladder.
    Auto,
    /// The user's choice is pinned.
    Manual,
}

impl QualityMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ThermalState {
    #[default]
    None,
    Light,
    Moderate,
    Severe,
    Critical,
    Emergency,
    Shutdown,
}

impl ThermalState {
    /// At `critical` and above the platform is about to clamp the encoder, so
    /// the ladder skips its dwell period and steps down immediately (§48).
    pub fn is_critical(self) -> bool {
        matches!(self, Self::Critical | Self::Emergency | Self::Shutdown)
    }

    /// `moderate` and above: enough to count as evidence the stream is
    /// struggling, but not enough to skip the dwell. `"none"` and `"light"`
    /// are neither.
    pub fn is_stressed(self) -> bool {
        matches!(
            self,
            Self::Moderate
                | Self::Severe
                | Self::Critical
                | Self::Emergency
                | Self::Shutdown
        )
    }
}

/* ------------------------------------------------------------------ */
/* Phone → desktop control (§6.1)                                      */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t")]
pub enum PhoneToDesktop {
    #[serde(rename = "hello")]
    Hello {
        protocol: u8,
        #[serde(rename = "deviceId")]
        device_id: String,
        #[serde(rename = "deviceName")]
        device_name: String,
        model: String,
        #[serde(rename = "androidVersion")]
        android_version: i32,
        #[serde(rename = "appVersion")]
        app_version: String,
    },
    #[serde(rename = "capabilities")]
    Capabilities {
        device: DeviceCapabilities,
        cameras: Vec<CameraCapability>,
    },
    #[serde(rename = "state")]
    State(PhoneState),
    #[serde(rename = "stats")]
    Stats(PhoneStats),
    #[serde(rename = "pong")]
    Pong {
        #[serde(rename = "clientNonce")]
        client_nonce: u64,
    },
    /// Sent by the phone when the desktop approved a §4.3 request.
    #[serde(rename = "pair_token")]
    PairToken { token: String },
    /// `message` is already user-facing prose (§6.1).
    #[serde(rename = "error")]
    Error { code: String, message: String },
}

/// The phone's reported stream state (§6.1).
///
/// `actual_resolution` / `actual_fps` are what the encoder really did, which is
/// why every quality announcement is labelled from these and not from the
/// request (§11).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PhoneState {
    pub streaming: bool,
    pub camera: String,
    pub resolution: String,
    pub fps: u32,
    #[serde(rename = "actualResolution")]
    pub actual_resolution: String,
    #[serde(rename = "actualFps")]
    pub actual_fps: u32,
    pub bitrate: u64,
    pub mirror: bool,
    pub rotation: u32,
    pub torch: bool,
    pub zoom: f32,
    pub battery: i32,
    pub charging: bool,
    pub thermal: ThermalState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PhoneStats {
    pub fps: f64,
    pub bitrate: u64,
    pub dropped: u64,
    #[serde(rename = "encodedFrames")]
    pub encoded_frames: u64,
    #[serde(rename = "rttUs")]
    pub rtt_us: i64,
    pub thermal: ThermalState,
}

/* ------------------------------------------------------------------ */
/* Desktop → phone control (§6.2)                                      */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t")]
#[serde(rename_all = "snake_case")]
pub enum DesktopToPhone {
    #[serde(rename = "welcome")]
    Welcome {
        protocol: u8,
        #[serde(rename = "serverVersion")]
        server_version: String,
        #[serde(rename = "serverName")]
        server_name: String,
    },
    #[serde(rename = "set_camera")]
    SetCamera {
        /// `"back"`, `"front"`, or a camera id string.
        camera: String,
    },
    /// Serializes through `SetVideoWire` because `fps` is `int | "auto"` and
    /// serde cannot flatten a scalar into a struct.
    #[serde(rename = "set_video")]
    SetVideo {
        /// `"auto"` or `WxH`.
        resolution: String,
        /// `"auto"` or an integer.
        fps: FpsValue,
        bitrate: u64,
    },
    #[serde(rename = "set_image")]
    SetImage { mirror: bool, rotation: u32, zoom: f32 },
    #[serde(rename = "set_torch")]
    SetTorch { enabled: bool },
    #[serde(rename = "set_screen")]
    SetScreen { mode: ScreenMode },
    #[serde(rename = "set_quality_mode")]
    SetQualityMode { mode: QualityMode },
    #[serde(rename = "start_stream")]
    StartStream,
    #[serde(rename = "stop_stream")]
    StopStream,
    #[serde(rename = "ping")]
    Ping {
        #[serde(rename = "serverNonce")]
        server_nonce: u64,
    },
    #[serde(rename = "pair_approved")]
    PairApproved { token: String },
    #[serde(rename = "pair_denied")]
    PairDenied { reason: String },
    #[serde(rename = "error")]
    Error { code: String, message: String },
}

/// `fps` is `int | "auto"` on the wire.
///
/// `#[serde(flatten)]` cannot carry a non-map, so this serializes itself with
/// explicit impls rather than deriving. The phone rejects a JSON number where
/// it expects the string `"auto"`, and vice versa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FpsValue {
    Auto,
    Fixed(u32),
}

impl FpsValue {
    pub fn auto() -> Self {
        Self::Auto
    }
    pub const fn fixed(value: u32) -> Self {
        Self::Fixed(value)
    }

    pub fn from_wire(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Number(n) => n
                .as_u64()
                .map(|v| Self::Fixed(v as u32))
                .unwrap_or(Self::Auto),
            _ => Self::Auto,
        }
    }
}

impl Serialize for FpsValue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Auto => s.serialize_str("auto"),
            Self::Fixed(v) => s.serialize_u32(*v),
        }
    }
}

impl<'de> Deserialize<'de> for FpsValue {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(d)?;
        Ok(Self::from_wire(&value))
    }
}

/* ------------------------------------------------------------------ */
/* Loopback: desktop UI → core (§9)                                    */
/* ------------------------------------------------------------------ */

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t")]
#[serde(rename_all = "snake_case")]
pub enum DesktopToCore {
    /// Start/stop `0x02` video forwarding.
    #[serde(rename = "sub_video")]
    SubVideo { enabled: bool },
    #[serde(rename = "get_state")]
    GetState,
    #[serde(rename = "arm_pairing")]
    ArmPairing {
        #[serde(rename = "ttlMs")]
        ttl_ms: i64,
    },
    #[serde(rename = "disarm_pairing")]
    DisarmPairing,
    #[serde(rename = "forget_device")]
    ForgetDevice {
        #[serde(rename = "deviceId")]
        device_id: String,
    },
    #[serde(rename = "connect_usb")]
    ConnectUsb { serial: Option<String> },
    #[serde(rename = "disconnect_usb")]
    DisconnectUsb,
    /// Re-locates adb and re-runs device discovery (the UI's "check again").
    #[serde(rename = "refresh_usb")]
    RefreshUsb,
    #[serde(rename = "set_output")]
    SetOutput {
        mirror: bool,
        rotation: u32,
        width: u32,
        height: u32,
    },
    #[serde(rename = "set_vcam")]
    SetVcam { enabled: bool },
    #[serde(rename = "export_diagnostics")]
    ExportDiagnostics { path: String },
    #[serde(rename = "log_level")]
    LogLevel { level: LogLevel },
    #[serde(rename = "pair_decision")]
    PairDecision {
        #[serde(rename = "deviceId")]
        device_id: String,
        approved: bool,
    },
    #[serde(rename = "set_video")]
    SetVideo {
        resolution: String,
        fps: FpsValue,
        bitrate: u64,
    },
    #[serde(rename = "set_camera")]
    SetCamera { camera: String },
    #[serde(rename = "set_image")]
    SetImage { mirror: bool, rotation: u32, zoom: f32 },
    #[serde(rename = "set_torch")]
    SetTorch { enabled: bool },
    #[serde(rename = "set_screen")]
    SetScreen { mode: ScreenMode },
    #[serde(rename = "set_quality_mode")]
    SetQualityMode { mode: QualityMode },
    #[serde(rename = "start_stream")]
    StartStream,
    #[serde(rename = "stop_stream")]
    StopStream,
}

/* ------------------------------------------------------------------ */
/* Loopback: core → desktop UI (§9)                                    */
/* ------------------------------------------------------------------ */

/// The core's session phase (§12).
///
/// This is a claim about whether a picture is flowing, so it is set only when
/// that is true. A video socket being accepted is **not** `streaming`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum StreamPhase {
    /// No phone.
    #[default]
    Idle,
    /// A phone is connected and the stream is being set up.
    Connecting,
    /// The phone is connected, the transport is up, no stream is running.
    Connected,
    /// A decoded frame has been published for the current stream.
    Streaming,
    /// The phone is gone and the core is waiting for it.
    Reconnecting,
    /// The last attempt failed; `error` says why.
    Error,
}

impl StreamPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Streaming => "streaming",
            Self::Reconnecting => "reconnecting",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum VcamState {
    #[default]
    Stopped,
    Starting,
    Running,
    Error,
}

impl VcamState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TransportKind {
    #[default]
    None,
    Lan,
    Usb,
}

impl TransportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Lan => "lan",
            Self::Usb => "usb",
        }
    }
}

/// What the **core** measured (§12).
///
/// None of these is a verdict on the phone's encoder; the phone reports its own
/// drops in `PhoneStats::dropped`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ConnectionStats {
    /// Frames the core published per second, over the last second.
    pub fps: f64,
    /// Bytes received per second, over the last second.
    pub bitrate: u64,
    /// The measured round trip (ping → pong). `-1` means none has been measured
    /// yet: a ping with no pong within one interval resets the measurement, so
    /// there is no 0 ms round trip.
    #[serde(rename = "latencyMs")]
    pub latency_ms: f64,
    /// Frames the **decoder** dropped, as a percentage of frames offered.
    #[serde(rename = "droppedPercent")]
    pub dropped_percent: f64,
    #[serde(rename = "decodedFrames")]
    pub decoded_frames: u64,
    #[serde(rename = "decodeQueueDepth")]
    pub decode_queue_depth: i64,
    /// A verdict on the decode path only: `excellent` < 1%, `good` ≤ 3%,
    /// `unstable` > 3%. It does not consult `latency_ms`.
    pub quality: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AdbDevice {
    pub serial: String,
    pub model: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VcamStatus {
    pub state: VcamState,
    pub name: String,
    /// Whether the COM class resolves machine-wide — whether other apps can
    /// select "PhoneCam Camera" at all.
    pub registered: bool,
    /// How many apps currently hold the camera open.
    pub consumers: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputTransform {
    pub mirror: bool,
    pub rotation: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct UsbStatus {
    /// `adb` was found and can be run.
    ///
    /// Deliberately separate from `available`. The UI used to read `available`
    /// as "the platform tools are installed", so a PC with adb present and
    /// nothing plugged in showed "Platform tools not found".
    #[serde(rename = "adbAvailable")]
    pub adb_available: bool,
    /// A phone is present on the cable (`adb devices` listed one).
    pub available: bool,
    pub authorized: bool,
    pub devices: Vec<AdbDevice>,
    pub forwarding: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct PairingStatus {
    pub open: bool,
    #[serde(rename = "pairingId")]
    pub pairing_id: Option<String>,
    #[serde(rename = "expiresAt")]
    pub expires_at: Option<i64>,
    pub hosts: Vec<String>,
    #[serde(rename = "qrPayloads")]
    pub qr_payloads: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceIdentity {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub name: String,
    pub model: String,
    #[serde(rename = "androidVersion")]
    pub android_version: i32,
}

/// A phone seen on the network via its discovery beacon (§3.2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NearbyPhone {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub name: String,
    pub model: String,
    pub address: String,
    #[serde(rename = "androidVersion")]
    pub android_version: i32,
    pub streaming: bool,
}

/// The full core state, emitted on every `state` event and on attach.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct CoreState {
    pub transport: TransportKind,
    pub phase: StreamPhase,
    pub device: Option<DeviceIdentity>,
    pub capabilities: Option<CapabilitiesPayload>,
    pub phone: Option<PhoneState>,
    pub stats: ConnectionStats,
    pub vcam: Option<VcamStatus>,
    pub output: Option<OutputTransform>,
    pub usb: UsbStatus,
    pub pairing: PairingStatus,
    pub error: Option<CoreError>,
    pub log_level: LogLevel,
    /// Whether the adaptive ladder may move (PRD §48).
    #[serde(rename = "qualityMode")]
    pub quality_mode: QualityMode,
    #[serde(rename = "nearbyPhones")]
    pub nearby_phones: Vec<NearbyPhone>,
}

impl Default for QualityMode {
    fn default() -> Self {
        Self::Auto
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

impl Default for OutputTransform {
    fn default() -> Self {
        Self { mirror: false, rotation: 0, width: 1920, height: 1080 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoreError {
    pub code: String,
    /// User-facing prose (§38).
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t")]
pub enum CoreToDesktop {
    #[serde(rename = "state")]
    State { state: CoreState },
    #[serde(rename = "pairing")]
    Pairing {
        open: bool,
        #[serde(rename = "pairingId")]
        pairing_id: Option<String>,
        #[serde(rename = "expiresAt")]
        expires_at: Option<i64>,
        hosts: Vec<String>,
        #[serde(rename = "qrPayloads")]
        qr_payloads: Vec<String>,
    },
    #[serde(rename = "pairing_result")]
    PairingResult {
        ok: bool,
        #[serde(rename = "deviceId")]
        device_id: Option<String>,
        name: Option<String>,
        error: Option<String>,
    },
    /// The desktop must prompt the user.
    #[serde(rename = "pair_request")]
    PairRequest {
        #[serde(rename = "deviceId")]
        device_id: String,
        name: String,
        model: String,
    },
    #[serde(rename = "phone_state")]
    PhoneState { state: PhoneState },
    #[serde(rename = "phone_stats")]
    PhoneStats { stats: PhoneStats },
    #[serde(rename = "phone_capabilities")]
    PhoneCapabilities { capabilities: CapabilitiesPayload },
    /// The following `state` carries `nearby_phones`.
    #[serde(rename = "nearby_phones_changed")]
    NearbyPhonesChanged,
    /// The adaptive controller moved the ladder (§11).
    #[serde(rename = "quality_changed")]
    QualityChanged {
        rung: String,
        resolution: String,
        fps: u32,
        bitrate: u64,
        reason: String,
        automatic: bool,
    },
    #[serde(rename = "log")]
    Log { level: LogLevel, message: String, ts: i64 },
    #[serde(rename = "error")]
    Error { code: String, message: String },
}

/// The reply the core sends to a UI command, on the control tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandReply {
    pub ok: bool,
    pub error: Option<String>,
}

impl CommandReply {
    pub fn ok() -> Self {
        Self { ok: true, error: None }
    }
    pub fn failed(message: impl Into<String>) -> Self {
        Self { ok: false, error: Some(message.into()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_messages_serialize_to_the_wire_shape() {
        let m = DesktopToPhone::SetVideo {
            resolution: "1920x1080".into(),
            fps: FpsValue::fixed(30),
            bitrate: 8_000_000,
        };
        let v: serde_json::Value = serde_json::to_value(&m).unwrap();
        assert_eq!(v["t"], "set_video");
        assert_eq!(v["resolution"], "1920x1080");
        assert_eq!(v["fps"], 30);
        assert_eq!(v["bitrate"], 8_000_000);
    }

    #[test]
    fn fps_auto_serializes_as_a_string() {
        let m = DesktopToPhone::SetVideo {
            resolution: "auto".into(),
            fps: FpsValue::auto(),
            bitrate: 4_000_000,
        };
        let v: serde_json::Value = serde_json::to_value(&m).unwrap();
        assert_eq!(v["fps"], "auto");
    }

    #[test]
    fn phone_state_parses_camel_case_fields() {
        let json = r#"{"t":"state","streaming":true,"camera":"back","resolution":"1920x1080",
            "fps":30,"actualResolution":"1280x720","actualFps":30,"bitrate":4000000,
            "mirror":false,"rotation":0,"torch":false,"zoom":1.0,"battery":88,
            "charging":true,"thermal":"moderate"}"#;
        let m: PhoneToDesktop = serde_json::from_str(json).unwrap();
        match m {
            PhoneToDesktop::State(s) => {
                assert!(s.streaming);
                assert_eq!(s.actual_resolution, "1280x720");
                assert_eq!(s.thermal, ThermalState::Moderate);
            }
            other => panic!("expected state, got {other:?}"),
        }
    }

    #[test]
    fn thermal_critical_is_detected() {
        assert!(ThermalState::Critical.is_critical());
        assert!(ThermalState::Emergency.is_critical());
        assert!(!ThermalState::Severe.is_critical());
        assert!(!ThermalState::None.is_critical());
    }

    #[test]
    fn phase_strings_match_the_protocol() {
        assert_eq!(StreamPhase::Streaming.as_str(), "streaming");
        assert_eq!(StreamPhase::Reconnecting.as_str(), "reconnecting");
        // The wire value round-trips through serde.
        let v = serde_json::to_value(StreamPhase::Connected).unwrap();
        assert_eq!(v, "connected");
    }

    #[test]
    fn qr_payload_validates() {
        let good = QrPayload {
            protocol: "phonecam".into(),
            version: 1,
            host: "192.168.1.24".into(),
            port: 48721,
            token: "9f2ab4c71de8036f5a1c9e2d4b7f0a63".into(),
            name: "DESKTOP-ABC".into(),
            server_id: "pc-7f3c1a9e".into(),
        };
        assert!(good.is_well_formed());
        let mut bad = good.clone();
        bad.token = "SHORT".into();
        assert!(!bad.is_well_formed());
        let mut bad = good.clone();
        bad.token = "9F2AB4C71DE8036F5A1C9E2D4B7F0A63".into(); // uppercase
        assert!(!bad.is_well_formed());
    }

    #[test]
    fn desktop_beacon_serializes_camel_case() {
        let b = DesktopBeacon::new(
            "pc-1",
            "DESKTOP",
            "192.168.1.5",
            48721,
            true,
            Some("b41c".into()),
            vec!["android-a".into()],
            false,
            "1.0.0",
        );
        let v = serde_json::to_value(&b).unwrap();
        assert_eq!(v["t"], "phonecam.beacon");
        assert_eq!(v["serverId"], "pc-1");
        assert_eq!(v["pairingOpen"], true);
        assert_eq!(v["pairingId"], "b41c");
        assert_eq!(v["pairedDeviceIds"][0], "android-a");
    }

    #[test]
    fn core_to_desktop_round_trips() {
        let e = CoreToDesktop::QualityChanged {
            rung: "720p30".into(),
            resolution: "1280x720".into(),
            fps: 30,
            bitrate: 4_000_000,
            reason: "frames dropped".into(),
            automatic: true,
        };
        let text = serde_json::to_string(&e).unwrap();
        let back: CoreToDesktop = serde_json::from_str(&text).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn screen_mode_serializes_as_camel_case() {
        let m = DesktopToPhone::SetScreen { mode: ScreenMode::KeepOn };
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["mode"], "keepOn");
    }
}
