//! The server side of the phone-facing channels (docs Ã‚Â§4Ã¢â‚¬â€œÃ‚Â§7, Ã‚Â§12).
//!
//! This is where a dial becomes a session: the greeting, the credential check,
//! the control loop, the video loop, and the phase rules that decide when the UI
//! is allowed to say a picture is flowing.
//!
//! Two of the rules here are the ones the shipped C++ got wrong first and are
//! documented because of it:
//!
//! * **A phase is a claim about a picture.** A video socket being accepted is not
//!   `streaming`; only a decoded frame is. The phone's own `streaming` flag is
//!   not sufficient either, because it is set from the encoder starting, before
//!   this PC has a socket or a decoded frame Ã¢â‚¬â€ it is evidence about the phone's
//!   encoder, not about a picture reaching this PC.
//! * **`busy` must not spend the token.** The refusal means "wait and retry", so
//!   the credential the retry needs has to survive it.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use phonecam_protocol::{
    write_control_frame, write_server_greeting, write_server_response, AuthMode, ChannelRole,
    ClientHandshake, ConnectionStats, DesktopToPhone, HandshakeStatus, PhoneState, PhoneToDesktop,
    StreamPhase, ThermalState, VideoFlags, MAX_CONTROL_JSON, MAX_VIDEO_PAYLOAD, PROTOCOL_VERSION,
    VIDEO_HEADER_SIZE,
};

use crate::pairing::{new_device_id, new_device_secret, new_server_nonce};
use crate::pairing::{DeviceRecord, PairingManager, PendingRequest};
use crate::transport::{Connection, ReadResult};

/// How long an approval prompt may wait for a human (docs Ã‚Â§4.3).
///
/// The UI's countdown must match this; a prompt that has visually expired while
/// the core still waits is a prompt that lies about its own state.
pub const APPROVAL_WAIT_MS: i64 = 120_000;
/// The token armed for a device after approval.
pub const APPROVAL_TOKEN_TTL_MS: i64 = 120_000;
/// The longest auth payload accepted. Real payloads are 81-100 bytes.
pub const MAX_HANDSHAKE_AUTH: usize = 512;

/// Why a handshake was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandshakeOutcome {
    pub ok: bool,
    pub device_id: String,
    pub device_name: String,
    pub role: ChannelRole,
    pub is_loopback: bool,
    /// A `pairRequest` was accepted and a human prompt is pending.
    pub approval_pending: bool,
    /// Safe to log: never a credential.
    pub error: String,
}

impl HandshakeOutcome {
    fn refused(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            device_id: String::new(),
            device_name: String::new(),
            role: ChannelRole::Control,
            is_loopback: false,
            approval_pending: false,
            error: reason.into(),
        }
    }
}

/// Everything the session shares with the rest of the app.
#[derive(Debug)]
pub struct SessionState {
    pub phase: Mutex<StreamPhase>,
    pub transport: Mutex<String>,
    pub device_id: Mutex<String>,
    pub device_name: Mutex<String>,
    pub device_model: Mutex<String>,
    pub android_version: AtomicI64,
    pub phone_state: Mutex<Option<PhoneState>>,
    pub stats: Mutex<ConnectionStats>,
    /// Set when the decoder could not keep up since the last tick, cleared on
    /// read so a burst of errors counts as one problem.
    pub decode_struggling: AtomicBool,
    /// True once a decoded frame has been published for the current stream.
    pub live_frame_seen: AtomicBool,
    /// Whether the user last asked the phone to stream.
    ///
    /// Set by `start_stream` and cleared by `stop_stream`. A reconnect (a Wi-Fi
    /// blink, a phone that went to sleep) reopens the control channel with nothing
    /// asking the phone to encode again, so the desktop would sit at "Connected"
    /// with a black preview until the user pressed Start a second time. The core
    /// resumes on their behalf when this is still set.
    pub stream_wanted: AtomicBool,
    pub error_code: Mutex<String>,
    pub error_message: Mutex<String>,
    /// Latest measured round trip in microseconds; 0 until the first `pong`.
    pub rtt_us: AtomicI64,
    /// Cumulative frames the phone says it has encoded.
    pub phone_encoded_frames: AtomicI64,
    /// Latest encoder drop percentage the phone reported.
    pub phone_dropped_permille: AtomicI64,
    pub phone_thermal: Mutex<ThermalState>,
    /// When the outstanding ping was sent, or 0 when none is outstanding.
    pub ping_sent_at_us: AtomicI64,
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            phase: Mutex::new(StreamPhase::Idle),
            transport: Mutex::new("none".into()),
            device_id: Mutex::new(String::new()),
            device_name: Mutex::new(String::new()),
            device_model: Mutex::new(String::new()),
            android_version: AtomicI64::new(0),
            phone_state: Mutex::new(None),
            stats: Mutex::new(ConnectionStats {
                // `-1` is the "no round trip measured yet" sentinel: a ping with
                // no pong within one interval resets the measurement, so there is
                // no 0 ms round trip.
                latency_ms: -1.0,
                quality: "offline".into(),
                ..Default::default()
            }),
            decode_struggling: AtomicBool::new(false),
            live_frame_seen: AtomicBool::new(false),
            stream_wanted: AtomicBool::new(false),
            error_code: Mutex::new(String::new()),
            error_message: Mutex::new(String::new()),
            rtt_us: AtomicI64::new(0),
            phone_encoded_frames: AtomicI64::new(0),
            phone_dropped_permille: AtomicI64::new(0),
            phone_thermal: Mutex::new(ThermalState::None),
            ping_sent_at_us: AtomicI64::new(0),
        }
    }
}

impl SessionState {
    pub fn set_error(&self, code: &str, message: &str) {
        *self.error_code.lock() = code.to_string();
        *self.error_message.lock() = message.to_string();
    }

    pub fn clear_error(&self) {
        self.error_code.lock().clear();
        self.error_message.lock().clear();
    }

    pub fn has_error(&self) -> bool {
        !self.error_code.lock().is_empty()
    }

    /// Called when starting a stream.
    ///
    /// Records user intent (`stream_wanted = true`) so reconnects will auto-resume
    /// (upstream 220c3af), and resets `live_frame_seen = false` so the first
    /// decoded frame promotes the phase from `connecting` to `streaming`
    /// (upstream a3d8ccd).
    pub fn note_start_stream(&self) {
        self.stream_wanted.store(true, Ordering::Relaxed);
        self.live_frame_seen.store(false, Ordering::Relaxed);
        let mut phase = self.phase.lock();
        if *phase != StreamPhase::Streaming {
            *phase = StreamPhase::Connecting;
        }
    }

    /// Called when stopping a stream.
    ///
    /// Clears user intent (`stream_wanted = false`) so a future reconnect
    /// will not auto-resume.
    pub fn note_stop_stream(&self) {
        self.stream_wanted.store(false, Ordering::Relaxed);
        let mut phase = self.phase.lock();
        if matches!(*phase, StreamPhase::Streaming | StreamPhase::Connecting) {
            *phase = StreamPhase::Connected;
        }
    }

    /// Promotes the phase to `connecting` when a video channel opens.
    ///
    /// It never demotes `streaming`: a reconfigure opens a second video socket
    /// while the first picture is still on screen, and dropping to `connecting`
    /// there would make a working stream look like it had failed.
    pub fn note_video_channel_opened(&self) {
        let mut phase = self.phase.lock();
        if *phase != StreamPhase::Streaming {
            *phase = StreamPhase::Connecting;
        }
    }

    /// Applies the phase rules for a phone `state` message.
    ///
    /// A `streaming: false` from the phone **does** demote, because it means the
    /// phone has stopped encoding and no claim of live video can hold. A
    /// `streaming: true` does not promote: only a decoded frame can.
    pub fn note_phone_state(&self, streaming: bool) {
        let mut phase = self.phase.lock();
        if streaming {
            if *phase == StreamPhase::Idle || *phase == StreamPhase::Connected {
                *phase = StreamPhase::Connecting;
            }
        } else if matches!(*phase, StreamPhase::Streaming | StreamPhase::Connecting) {
            *phase = StreamPhase::Connected;
        }
    }

    /// Promotes to `streaming`. Called only when a frame has actually decoded.
    pub fn note_decoded_frame(&self) {
        self.live_frame_seen.store(true, Ordering::Relaxed);
        let mut phase = self.phase.lock();
        if matches!(*phase, StreamPhase::Connecting | StreamPhase::Connected | StreamPhase::Idle) {
            *phase = StreamPhase::Streaming;
        }
    }

    /// The control channel closed: no surface may name a phone or a link that is
    /// no longer there.
    pub fn note_control_closed(&self) {
        self.device_id.lock().clear();
        self.device_name.lock().clear();
        self.device_model.lock().clear();
        self.android_version.store(0, Ordering::Relaxed);
        *self.transport.lock() = "none".to_string();
        *self.phone_state.lock() = None;
        self.live_frame_seen.store(false, Ordering::Relaxed);
        *self.phase.lock() = StreamPhase::Reconnecting;
    }

    pub fn phase(&self) -> StreamPhase {
        *self.phase.lock()
    }

    /// True while a phone is connected and a picture is flowing.
    pub fn phone_is_streaming(&self) -> bool {
        self.phase() == StreamPhase::Streaming
    }

    pub fn thermal(&self) -> ThermalState {
        *self.phone_thermal.lock()
    }
}

/// Stepped retry delays for a camera that failed to start mid-stream or during startup.
///
/// A wedged HAL needs roughly a second of quiet before it will open a device again;
/// retrying faster than that is what produced the endless open-fail loop (upstream 8f58170).
/// The steps give it that quiet while keeping the first retry fast enough that a
/// switch race is invisible on the desktop.
pub const CAMERA_RETRY_DELAYS_MS: [u64; 4] = [500, 1000, 2000, 4000];
pub const MAX_CAMERA_START_FAILURES: usize = 4;

/// Stepped retry manager for camera start attempts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraRetryTracker {
    failures: usize,
    delays_ms: Vec<u64>,
    max_failures: usize,
}

impl Default for CameraRetryTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraRetryTracker {
    pub fn new() -> Self {
        Self {
            failures: 0,
            delays_ms: CAMERA_RETRY_DELAYS_MS.to_vec(),
            max_failures: MAX_CAMERA_START_FAILURES,
        }
    }

    pub fn with_delays(delays_ms: Vec<u64>, max_failures: usize) -> Self {
        Self {
            failures: 0,
            delays_ms,
            max_failures,
        }
    }

    pub fn failures(&self) -> usize {
        self.failures
    }

    pub fn max_failures(&self) -> usize {
        self.max_failures
    }

    pub fn reset(&mut self) {
        self.failures = 0;
    }

    /// Records a failure. Returns `Some(delay)` for the next retry attempt,
    /// or `None` if the retry limit has been reached.
    pub fn record_failure(&mut self) -> Option<std::time::Duration> {
        if self.failures >= self.max_failures {
            return None;
        }
        let idx = self.failures.min(self.delays_ms.len().saturating_sub(1));
        let delay_ms = self.delays_ms.get(idx).copied().unwrap_or(1000);
        self.failures += 1;
        Some(std::time::Duration::from_millis(delay_ms))
    }

    pub fn can_retry(&self) -> bool {
        self.failures < self.max_failures
    }
}

/// A command the core sends to the phone.
pub fn control_frame(message: &DesktopToPhone) -> Option<Vec<u8>> {
    write_control_frame(message).ok()
}

/// Authenticates one inbound connection (docs Ã‚Â§5.2Ã¢â‚¬â€œÃ‚Â§5.4).
///
/// `allow_loopback` is true only for the UI listener, because auth mode 0 is
/// trusted solely from 127.0.0.1.
pub struct Authenticator<'a> {
    pub pairing: &'a Mutex<PairingManager>,
    pub registry: &'a Mutex<Vec<DeviceRecord>>,
    pub session: &'a Arc<SessionState>,
    pub server_id: &'a str,
    pub server_name: &'a str,
    pub now_ms: i64,
}

impl Authenticator<'_> {
    /// Reads the greeting/handshake exchange and validates the credential.
    ///
    /// On success the connection is left positioned for framed traffic.
    pub fn authenticate(
        &self,
        conn: &Connection,
        allow_loopback: bool,
    ) -> HandshakeOutcome {
        let is_loopback = conn.is_loopback();

        // A nonce we cannot generate is a reason to refuse, never a reason to
        // fall back to something predictable.
        let Some(server_nonce) = new_server_nonce() else {
            return HandshakeOutcome::refused("secure random generation failed");
        };

        let Ok(greeting) = write_server_greeting(HandshakeStatus::Ok, &server_nonce) else {
            return HandshakeOutcome::refused("could not frame the greeting");
        };
        if !conn.write_all(&greeting) {
            return HandshakeOutcome::refused("the peer closed before the greeting");
        }

        let mut fixed = [0u8; 20];
        match conn.read_exact(&mut fixed) {
            ReadResult::Ok(_) => {}
            ReadResult::Timeout => return HandshakeOutcome::refused("the handshake timed out"),
            ReadResult::Closed => return HandshakeOutcome::refused("the peer closed"),
        }

        if &fixed[0..4] != b"PCAM" {
            self.refuse(conn, ChannelRole::Control, HandshakeStatus::BadRequest, "{}");
            return HandshakeOutcome::refused("bad magic in the handshake");
        }

        // The auth body follows the fixed 20 bytes and its length lives inside
        // them, so it has to be read before the handshake can be parsed. Reading
        // only the fixed part and parsing it as a whole handshake is what made
        // every dial fail with `badRequest`.
        let auth_len = u16::from_be_bytes([fixed[16], fixed[17]]) as usize;
        if auth_len > MAX_HANDSHAKE_AUTH {
            // Refused before any allocation, so a hostile length cannot make this
            // process reserve memory.
            return HandshakeOutcome::refused("auth payload too long");
        }

        let mut whole = fixed.to_vec();
        if auth_len > 0 {
            let mut auth = vec![0u8; auth_len];
            match conn.read_exact(&mut auth) {
                ReadResult::Ok(_) => whole.extend_from_slice(&auth),
                ReadResult::Timeout => return HandshakeOutcome::refused("the auth body timed out"),
                ReadResult::Closed => return HandshakeOutcome::refused("the peer closed"),
            }
        }

        let Ok(handshake) = ClientHandshake::parse(&whole) else {
            self.refuse(conn, ChannelRole::Control, HandshakeStatus::BadRequest, "{}");
            return HandshakeOutcome::refused("malformed handshake");
        };

        if handshake.proto_version != PROTOCOL_VERSION {
            let body = serde_json::json!({ "serverVersion": env!("CARGO_PKG_VERSION") });
            if let Ok(bytes) = write_server_response(
                HandshakeStatus::VersionMismatch,
                handshake.role,
                &body,
            ) {
                conn.write_all(&bytes);
            }
            return HandshakeOutcome::refused(format!(
                "protocol version {} is not {PROTOCOL_VERSION}",
                handshake.proto_version
            ));
        }

        // The auth length is checked before the payload is read, so an oversized
        // claim cannot be used to make this process allocate.
        if handshake.auth.len() > MAX_HANDSHAKE_AUTH {
            return HandshakeOutcome::refused("auth payload too long");
        }

        let mut outcome = match handshake.auth_mode {
            AuthMode::Loopback => {
                if !allow_loopback {
                    self.refuse(conn, handshake.role, HandshakeStatus::BadRequest, "{}");
                    return HandshakeOutcome::refused(
                        "the UI channel requires loopback authentication",
                    );
                }
                if !is_loopback {
                    self.refuse(conn, handshake.role, HandshakeStatus::UnauthorizedSource, "{}");
                    return HandshakeOutcome::refused("loopback mode from a non-loopback peer");
                }
                // Every successful handshake must send a response frame, or the
                // client waits forever on a channel it cannot know is open.
                self.refuse(conn, handshake.role, HandshakeStatus::Ok, "{}");
                HandshakeOutcome {
                    ok: true,
                    device_id: "local-ui".into(),
                    device_name: "PhoneCam UI".into(),
                    role: handshake.role,
                    is_loopback: true,
                    approval_pending: false,
                    error: String::new(),
                }
            }
            AuthMode::PairingToken => self.pairing_token(conn, &handshake),
            AuthMode::DeviceSecret => self.device_secret(conn, &handshake, &server_nonce),
            AuthMode::PairRequest => self.pair_request(conn, &handshake, &server_nonce),
        };

        outcome.is_loopback = is_loopback;
        outcome.role = handshake.role;
        outcome
    }

    /// Auth mode 1: the one-time token from the QR code, or an approval token.
    fn pairing_token(&self, conn: &Connection, handshake: &ClientHandshake) -> HandshakeOutcome {
        let mut pairing = self.pairing.lock();

        // Peek first. A `busy` refusal means "wait and retry", so it must not
        // spend the credential the retry needs.
        if let Some(peeked) = pairing.peek(&handshake.auth, self.now_ms) {
            if let Some(bound_to) = &peeked.device_id {
                if self.is_busy_for(bound_to) {
                    drop(pairing);
                    self.refuse(conn, handshake.role, HandshakeStatus::Busy, "{}");
                    return HandshakeOutcome::refused("busy with another phone");
                }
            }
        }

        let Some(consumed) = pairing.consume(&handshake.auth, self.now_ms) else {
            drop(pairing);
            self.refuse(conn, handshake.role, HandshakeStatus::TokenExpired, "{}");
            return HandshakeOutcome::refused("the pairing token expired or was already used");
        };

        // A bound token names the device that asked for it and that id must be
        // reused; an unbound QR token mints a fresh one.
        let device_id = consumed.device_id.unwrap_or_else(|| {
            new_device_id().unwrap_or_else(|| format!("android-{:08x}", self.now_ms as u32))
        });
        drop(pairing);

        let Some(secret) = new_device_secret() else {
            self.refuse(conn, handshake.role, HandshakeStatus::Denied, "{}");
            return HandshakeOutcome::refused("secure random generation failed");
        };

        {
            let mut registry = self.registry.lock();
            let record = DeviceRecord::new(&device_id, &secret, self.now_ms);
            match registry.iter_mut().find(|r| r.device_id == device_id) {
                Some(existing) => *existing = record,
                None => registry.push(record),
            }
        }

        let body = serde_json::json!({
            "deviceId": device_id,
            "deviceSecret": secret,
            "serverId": self.server_id,
            "name": self.server_name,
        });
        if let Ok(bytes) = write_server_response(HandshakeStatus::Ok, handshake.role, &body) {
            conn.write_all(&bytes);
        }

        HandshakeOutcome {
            ok: true,
            device_id,
            device_name: "Android phone".into(),
            role: handshake.role,
            is_loopback: false,
            approval_pending: false,
            error: String::new(),
        }
    }

    /// Auth mode 2: `deviceId:hexHmac` over `serverNonce || clientNonce`.
    fn device_secret(
        &self,
        conn: &Connection,
        handshake: &ClientHandshake,
        server_nonce: &[u8],
    ) -> HandshakeOutcome {
        let parts: Vec<&str> = handshake.auth.split(':').collect();
        if parts.len() != 2 {
            self.refuse(conn, handshake.role, HandshakeStatus::BadRequest, "{}");
            return HandshakeOutcome::refused("malformed device-secret auth");
        }
        let (device_id, presented) = (parts[0], parts[1]);

        let secret = {
            let registry = self.registry.lock();
            registry
                .iter()
                .find(|r| r.device_id == device_id)
                .map(|r| r.secret_hex.clone())
        };
        let Some(secret) = secret else {
            self.refuse(conn, handshake.role, HandshakeStatus::Denied, "{}");
            return HandshakeOutcome::refused("unknown device");
        };

        // Constant-time comparison of the HMAC digests.
        if !phonecam_protocol::verify_device_secret(
            device_id,
            &secret,
            server_nonce,
            handshake.client_nonce,
            presented,
        ) {
            self.refuse(conn, handshake.role, HandshakeStatus::Denied, "{}");
            return HandshakeOutcome::refused("the device secret did not verify");
        }

        // Replay defence: the nonce must strictly increase per device.
        if !self.pairing.lock().accept_nonce(device_id, handshake.client_nonce) {
            self.refuse(conn, handshake.role, HandshakeStatus::Denied, "{}");
            return HandshakeOutcome::refused("replayed client nonce");
        }

        // Only now, with the credential proven, can `busy` be decided: the dial
        // has established that it is a different, known phone.
        if self.is_busy_for(device_id) {
            self.refuse(conn, handshake.role, HandshakeStatus::Busy, "{}");
            return HandshakeOutcome::refused("busy with another phone");
        }

        let name = {
            let registry = self.registry.lock();
            registry
                .iter()
                .find(|r| r.device_id == device_id)
                .map(|r| r.name.clone())
                .unwrap_or_else(|| "Android phone".into())
        };

        self.refuse(conn, handshake.role, HandshakeStatus::Ok, "{}");

        HandshakeOutcome {
            ok: true,
            device_id: device_id.to_string(),
            device_name: name,
            role: handshake.role,
            is_loopback: false,
            approval_pending: false,
            error: String::new(),
        }
    }

    /// Auth mode 3: `deviceId:request:hexHmac`, raising a human prompt.
    fn pair_request(
        &self,
        conn: &Connection,
        handshake: &ClientHandshake,
        server_nonce: &[u8],
    ) -> HandshakeOutcome {
        let parts: Vec<&str> = handshake.auth.split(':').collect();
        if parts.len() != 3 || parts[1] != "request" {
            self.refuse(conn, handshake.role, HandshakeStatus::BadRequest, "{}");
            return HandshakeOutcome::refused("malformed pair request");
        }
        let device_id = parts[0];

        // The HMAC proves the requester saw the fresh nonce, which stops blind
        // spraying. It does **not** authenticate the device: the human approval
        // is what authorises the pairing.
        let known_secret = {
            let registry = self.registry.lock();
            registry
                .iter()
                .find(|r| r.device_id == device_id)
                .map(|r| r.secret_hex.clone())
        };
        if !phonecam_protocol::verify_pair_request(
            device_id,
            known_secret.as_deref(),
            server_nonce,
            handshake.client_nonce,
            &handshake.auth,
        ) {
            self.refuse(conn, handshake.role, HandshakeStatus::Denied, "{}");
            return HandshakeOutcome::refused("the pair request did not verify");
        }

        if self.is_busy_for(device_id) {
            self.refuse(conn, handshake.role, HandshakeStatus::Busy, "{}");
            return HandshakeOutcome::refused("busy with another phone");
        }

        {
            let mut pairing = self.pairing.lock();
            pairing.add_pending_request(PendingRequest {
                device_id: device_id.to_string(),
                name: "Android phone".into(),
                model: "Android".into(),
                requested_at_ms: self.now_ms,
            });
        }

        let body = serde_json::json!({ "deviceId": device_id });
        if let Ok(bytes) =
            write_server_response(HandshakeStatus::ApprovalPending, handshake.role, &body)
        {
            conn.write_all(&bytes);
        }

        HandshakeOutcome {
            ok: true,
            device_id: device_id.to_string(),
            device_name: "Android phone".into(),
            role: handshake.role,
            is_loopback: false,
            approval_pending: true,
            error: String::new(),
        }
    }

    /// Whether another, already-paired phone holds the session.
    ///
    /// The current phone re-dialling is never refused, and an unbound QR token is
    /// never refused because it cannot prove it is a different phone.
    fn is_busy_for(&self, claimed_device_id: &str) -> bool {
        let current = self.session.device_id.lock().clone();
        if current.is_empty() {
            return false;
        }
        if claimed_device_id.is_empty() {
            return false;
        }
        claimed_device_id != current
    }

    fn refuse(&self, conn: &Connection, role: ChannelRole, status: HandshakeStatus, body: &str) {
        let parsed: serde_json::Value =
            serde_json::from_str(body).unwrap_or_else(|_| serde_json::json!({}));
        if let Ok(bytes) = write_server_response(status, role, &parsed) {
            conn.write_all(&bytes);
        }
    }
}

/// The result of trying to read one framed message.
///
/// The three-way distinction is the same one `transport::ReadResult` makes, and
/// it matters just as much here: a control channel is quiet for long stretches,
/// and treating an idle timeout as "closed" tears down a session that is
/// perfectly healthy.
#[derive(Debug)]
pub enum FramedRead {
    /// A complete frame arrived.
    Frame(Vec<u8>),
    /// Nothing arrived within the timeout. The channel is still open.
    Idle,
    /// The peer closed, or the framing was violated.
    Closed,
}

/// Reads one control frame's payload, or reports why it could not.
///
/// A length of 0 or one past the limit is a protocol violation and closes the
/// connection; it is never skipped, because skipping would desynchronise the
/// stream.
pub fn read_control_payload(conn: &Connection) -> Option<Vec<u8>> {
    match read_control_frame(conn) {
        FramedRead::Frame(payload) => Some(payload),
        _ => None,
    }
}

/// Reads one control frame, distinguishing an idle channel from a closed one.
pub fn read_control_frame(conn: &Connection) -> FramedRead {
    let mut len_bytes = [0u8; 4];
    match conn.read_exact(&mut len_bytes) {
        ReadResult::Ok(_) => {}
        ReadResult::Timeout => return FramedRead::Idle,
        ReadResult::Closed => return FramedRead::Closed,
    }
    let len = u32::from_be_bytes(len_bytes) as usize;
    if len == 0 || len > MAX_CONTROL_JSON {
        tracing::warn!("control frame length {len} is out of range; closing");
        return FramedRead::Closed;
    }
    let mut payload = vec![0u8; len];
    match conn.read_exact(&mut payload) {
        ReadResult::Ok(_) => FramedRead::Frame(payload),
        ReadResult::Timeout => FramedRead::Idle,
        ReadResult::Closed => FramedRead::Closed,
    }
}

/// Reads one video packet, returning the payload and its flags.
pub fn read_video_packet(conn: &Connection) -> Option<(Vec<u8>, VideoFlags, i64)> {
    let mut len_bytes = [0u8; 4];
    match conn.read_exact(&mut len_bytes) {
        ReadResult::Ok(_) => {}
        _ => return None,
    }
    let total = u32::from_be_bytes(len_bytes) as usize;
    if total < VIDEO_HEADER_SIZE || total > VIDEO_HEADER_SIZE + MAX_VIDEO_PAYLOAD {
        tracing::warn!("video packet length {total} is out of range; closing");
        return None;
    }

    let mut packet = vec![0u8; total];
    match conn.read_exact(&mut packet) {
        ReadResult::Ok(_) => {}
        _ => return None,
    }

    let flags = VideoFlags(packet[12]);
    let codec = packet[13];
    let payload_len = u32::from_be_bytes([packet[16], packet[17], packet[18], packet[19]]) as usize;
    let pts_us = i64::from_be_bytes(packet[4..12].try_into().ok()?);

    if codec != 1 {
        return None;
    }
    // A payload length that does not fit the packet would read past the buffer.
    if payload_len + VIDEO_HEADER_SIZE != total {
        return None;
    }

    Some((packet[VIDEO_HEADER_SIZE..].to_vec(), flags, pts_us))
}

/// Parses a phone control message, ignoring anything unknown.
pub fn parse_phone_message(payload: &[u8]) -> Option<PhoneToDesktop> {
    serde_json::from_slice(payload).ok()
}

/// The phase a phone `state` message implies, per docs Ã‚Â§12.
pub fn phase_after_phone_state(current: StreamPhase, streaming: bool) -> StreamPhase {
    if streaming {
        // A `streaming: true` is evidence about the phone's encoder, not about a
        // picture reaching this PC, so it cannot promote past `connecting`.
        match current {
            StreamPhase::Idle | StreamPhase::Connected => StreamPhase::Connecting,
            other => other,
        }
    } else {
        match current {
            StreamPhase::Streaming | StreamPhase::Connecting => StreamPhase::Connected,
            other => other,
        }
    }
}

/// Which stats the core reports, computed from its own counters (docs Ã‚Â§12).
pub struct StatsWindow {
    frames: u64,
    bytes: u64,
    started_us: i64,
}

impl StatsWindow {
    pub fn new(now_us: i64) -> Self {
        Self { frames: 0, bytes: 0, started_us: now_us }
    }

    /// When the current window began.
    ///
    /// Exposed because the caller decides when a window has elapsed, and it
    /// cannot do that from a clock reading alone: a "start" helper that returned
    /// the current time made the elapsed check `now - now >= 1s`, which is never
    /// true, so the statistics were never computed at all. The stream ran, the
    /// frames decoded, and every rate read zero.
    pub fn started_us(&self) -> i64 {
        self.started_us
    }

    /// How long the current window has been open, in microseconds.
    pub fn elapsed_us(&self, now_us: i64) -> i64 {
        now_us.saturating_sub(self.started_us)
    }

    pub fn note_frame(&mut self, bytes: usize) {
        self.frames += 1;
        self.bytes += bytes as u64;
    }

    /// Produces the per-second figures and restarts the window.
    pub fn take(&mut self, now_us: i64, rtt_us: i64, decoded: u64, dropped: u64) -> ConnectionStats {
        let elapsed_us = (now_us - self.started_us).max(1);
        let elapsed_s = elapsed_us as f64 / 1_000_000.0;

        let fps = self.frames as f64 / elapsed_s;
        let bitrate = (self.bytes as f64 / elapsed_s) as u64;

        let offered = decoded + dropped;
        let dropped_percent = if offered > 0 {
            dropped as f64 * 100.0 / offered as f64
        } else {
            0.0
        };

        // A verdict on the decode path only. It deliberately does not consult
        // latency, and it is not a verdict on the phone's encoder.
        let quality = if dropped_percent < 1.0 {
            "excellent"
        } else if dropped_percent <= 3.0 {
            "good"
        } else {
            "unstable"
        };

        self.frames = 0;
        self.bytes = 0;
        self.started_us = now_us;

        ConnectionStats {
            fps,
            bitrate,
            latency_ms: if rtt_us > 0 { rtt_us as f64 / 1000.0 } else { -1.0 },
            dropped_percent,
            decoded_frames: decoded,
            decode_queue_depth: 0,
            quality: quality.to_string(),
        }
    }
}

/// Monotonic microseconds, for timing rather than for display.
pub fn now_us() -> i64 {
    use std::time::Instant;
    // A process-lifetime origin is enough: only differences are used.
    thread_local! {
        static ORIGIN: Instant = Instant::now();
    }
    ORIGIN.with(|origin| origin.elapsed().as_micros() as i64)
}

/// Milliseconds since the Unix epoch, for anything shown or persisted.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use phonecam_protocol::SERVER_GREETING_SIZE;

    fn session() -> Arc<SessionState> {
        Arc::new(SessionState::default())
    }

    #[test]
    fn a_new_session_is_idle_with_no_measurement() {
        let state = session();
        assert_eq!(state.phase(), StreamPhase::Idle);
        // `-1` is the documented "no round trip measured" sentinel.
        assert_eq!(state.stats.lock().latency_ms, -1.0);
        assert_eq!(state.stats.lock().quality, "offline");
        assert!(!state.phone_is_streaming());
    }

    #[test]
    fn a_video_socket_alone_is_not_streaming() {
        let state = session();
        state.note_video_channel_opened();
        assert_eq!(
            state.phase(),
            StreamPhase::Connecting,
            "a socket says a channel exists, a decoded frame says there is a picture"
        );
        assert!(!state.phone_is_streaming());
    }

    #[test]
    fn a_decoded_frame_is_what_makes_it_streaming() {
        let state = session();
        state.note_video_channel_opened();
        state.note_decoded_frame();
        assert_eq!(state.phase(), StreamPhase::Streaming);
        assert!(state.phone_is_streaming());
        assert!(state.live_frame_seen.load(Ordering::Relaxed));
    }

    #[test]
    fn a_reconfigure_does_not_demote_a_live_stream() {
        let state = session();
        state.note_decoded_frame();
        assert_eq!(state.phase(), StreamPhase::Streaming);
        // A second video socket opens while the first picture is on screen.
        state.note_video_channel_opened();
        assert_eq!(state.phase(), StreamPhase::Streaming, "a working stream must not look failed");
    }

    #[test]
    fn the_phones_streaming_flag_cannot_promote_the_phase() {
        let state = session();
        state.note_phone_state(true);
        assert_eq!(
            state.phase(),
            StreamPhase::Connecting,
            "the phone's flag is evidence about its encoder, not about a picture here"
        );
    }

    #[test]
    fn the_phones_stop_does_demote() {
        let state = session();
        state.note_decoded_frame();
        state.note_phone_state(false);
        assert_eq!(state.phase(), StreamPhase::Connected);
    }

    #[test]
    fn phase_after_phone_state_matches_the_rules() {
        assert_eq!(phase_after_phone_state(StreamPhase::Idle, true), StreamPhase::Connecting);
        assert_eq!(phase_after_phone_state(StreamPhase::Connected, true), StreamPhase::Connecting);
        assert_eq!(phase_after_phone_state(StreamPhase::Streaming, true), StreamPhase::Streaming);
        assert_eq!(phase_after_phone_state(StreamPhase::Streaming, false), StreamPhase::Connected);
        assert_eq!(phase_after_phone_state(StreamPhase::Reconnecting, false), StreamPhase::Reconnecting);
    }

    #[test]
    fn a_closed_control_channel_names_nothing() {
        let state = session();
        *state.device_id.lock() = "android-a".into();
        *state.device_name.lock() = "Redmi".into();
        *state.transport.lock() = "lan".into();
        state.note_decoded_frame();

        state.note_control_closed();

        assert!(state.device_id.lock().is_empty());
        assert!(state.device_name.lock().is_empty());
        assert_eq!(*state.transport.lock(), "none");
        assert_eq!(state.phase(), StreamPhase::Reconnecting);
        assert!(!state.live_frame_seen.load(Ordering::Relaxed));
        assert!(state.phone_state.lock().is_none());
    }

    #[test]
    fn stats_are_computed_over_the_window() {
        let mut window = StatsWindow::new(0);
        // 30 frames of 10 KB in one second.
        for _ in 0..30 {
            window.note_frame(10_000);
        }
        let stats = window.take(1_000_000, 25_000, 30, 0);
        assert!((stats.fps - 30.0).abs() < 0.01, "fps was {}", stats.fps);
        assert_eq!(stats.bitrate, 300_000);
        assert!((stats.latency_ms - 25.0).abs() < 0.01);
        assert_eq!(stats.quality, "excellent");
    }

    #[test]
    fn an_unmeasured_round_trip_stays_negative() {
        let mut window = StatsWindow::new(0);
        let stats = window.take(1_000_000, 0, 0, 0);
        assert_eq!(stats.latency_ms, -1.0);
    }

    #[test]
    fn the_quality_verdict_tracks_decode_drops() {
        let mut window = StatsWindow::new(0);
        // 2 drops in 100 offered = 2% -> good.
        let stats = window.take(1_000_000, 0, 98, 2);
        assert_eq!(stats.quality, "good");
        assert!((stats.dropped_percent - 2.0).abs() < 0.01);

        // 10 in 100 = 10% -> unstable.
        let stats = window.take(2_000_000, 0, 90, 10);
        assert_eq!(stats.quality, "unstable");
    }

    #[test]
    fn the_window_restarts_after_each_read() {
        let mut window = StatsWindow::new(0);
        window.note_frame(1000);
        let first = window.take(1_000_000, 0, 1, 0);
        assert!((first.fps - 1.0).abs() < 0.01);
        // No new frames: the next window reads zero, not the previous count.
        let second = window.take(2_000_000, 0, 1, 0);
        assert_eq!(second.fps, 0.0);
    }

    #[test]
    fn control_frames_serialise_to_the_wire_shape() {
        let bytes = control_frame(&DesktopToPhone::StartStream).unwrap();
        let len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        assert_eq!(len, bytes.len() - 4);
        let value: serde_json::Value = serde_json::from_slice(&bytes[4..]).unwrap();
        assert_eq!(value["t"], "start_stream");
    }

    #[test]
    fn a_ping_carries_the_wire_field_name() {
        let bytes = control_frame(&DesktopToPhone::Ping { server_nonce: 12345 }).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes[4..]).unwrap();
        assert_eq!(value["t"], "ping");
        assert_eq!(value["serverNonce"], 12345);
    }

    #[test]
    fn an_oversized_control_frame_is_refused() {
        let huge = DesktopToPhone::Error {
            code: "x".into(),
            message: "y".repeat(MAX_CONTROL_JSON + 100),
        };
        assert!(control_frame(&huge).is_none());
    }

    #[test]
    fn phone_messages_parse_and_unknown_ones_are_ignored() {
        let hello = br#"{"t":"hello","protocol":1,"deviceId":"android-a","deviceName":"Redmi","model":"2201117TG","androidVersion":33,"appVersion":"1.0.0"}"#;
        assert!(matches!(
            parse_phone_message(hello),
            Some(PhoneToDesktop::Hello { .. })
        ));
        assert!(parse_phone_message(br#"{"t":"something-new"}"#).is_none());
        assert!(parse_phone_message(b"not json").is_none());
    }

    #[test]
    fn the_greeting_frames_correctly() {
        let nonce = vec![7u8; 32];
        let bytes = write_server_greeting(HandshakeStatus::Ok, &nonce).unwrap();
        assert_eq!(bytes.len(), SERVER_GREETING_SIZE);
        assert_eq!(&bytes[0..4], b"PCAM");
    }

    #[test]
    fn busy_refuses_only_a_different_known_phone() {
        let state = session();
        let pairing = Mutex::new(PairingManager::new());
        let registry = Mutex::new(Vec::new());
        let auth = Authenticator {
            pairing: &pairing,
            registry: &registry,
            session: &state,
            server_id: "pc-1",
            server_name: "DESKTOP",
            now_ms: 0,
        };

        // Nobody connected: never busy.
        assert!(!auth.is_busy_for("android-a"));

        *state.device_id.lock() = "android-a".into();
        // The same phone re-dialling is never refused.
        assert!(!auth.is_busy_for("android-a"));
        // A different phone is.
        assert!(auth.is_busy_for("android-b"));
        // An unbound token cannot prove it is a different phone.
        assert!(!auth.is_busy_for(""));
    }

    #[test]
    fn start_stream_tracks_stream_wanted_and_clears_live_frame_seen() {
        let state = session();
        state.note_decoded_frame();
        assert!(state.live_frame_seen.load(Ordering::Relaxed));
        assert_eq!(state.phase(), StreamPhase::Streaming);

        // Restarting stream clears live_frame_seen and sets stream_wanted to true
        state.note_start_stream();
        assert!(!state.live_frame_seen.load(Ordering::Relaxed));
        assert!(state.stream_wanted.load(Ordering::Relaxed));

        // When the next frame decodes, it properly promotes to streaming
        state.note_decoded_frame();
        assert!(state.live_frame_seen.load(Ordering::Relaxed));
        assert_eq!(state.phase(), StreamPhase::Streaming);
    }

    #[test]
    fn stop_stream_clears_stream_wanted() {
        let state = session();
        state.note_start_stream();
        assert!(state.stream_wanted.load(Ordering::Relaxed));

        state.note_stop_stream();
        assert!(!state.stream_wanted.load(Ordering::Relaxed));
    }

    #[test]
    fn control_channel_close_preserves_stream_wanted_for_reconnect() {
        let state = session();
        state.note_start_stream();
        assert!(state.stream_wanted.load(Ordering::Relaxed));

        // When the control channel drops (e.g. Wi-Fi blink), stream_wanted must stay true
        // so the desktop can automatically resume the stream when it comes back.
        state.note_control_closed();
        assert!(state.stream_wanted.load(Ordering::Relaxed));
        assert_eq!(state.phase(), StreamPhase::Reconnecting);
        assert!(!state.live_frame_seen.load(Ordering::Relaxed));
    }

    #[test]
    fn stepped_camera_retry_tracks_delays_and_exhaustion() {
        let mut tracker = CameraRetryTracker::new();
        assert_eq!(tracker.failures(), 0);
        assert!(tracker.can_retry());

        assert_eq!(tracker.record_failure(), Some(std::time::Duration::from_millis(500)));
        assert_eq!(tracker.failures(), 1);

        assert_eq!(tracker.record_failure(), Some(std::time::Duration::from_millis(1000)));
        assert_eq!(tracker.failures(), 2);

        assert_eq!(tracker.record_failure(), Some(std::time::Duration::from_millis(2000)));
        assert_eq!(tracker.failures(), 3);

        assert_eq!(tracker.record_failure(), Some(std::time::Duration::from_millis(4000)));
        assert_eq!(tracker.failures(), 4);

        // Exhausted after 4 failures
        assert!(!tracker.can_retry());
        assert_eq!(tracker.record_failure(), None);
    }

    #[test]
    fn camera_retry_tracker_resets_failures() {
        let mut tracker = CameraRetryTracker::new();
        tracker.record_failure();
        tracker.record_failure();
        assert_eq!(tracker.failures(), 2);

        tracker.reset();
        assert_eq!(tracker.failures(), 0);
        assert_eq!(tracker.record_failure(), Some(std::time::Duration::from_millis(500)));
    }
}







