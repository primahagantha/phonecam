//! The runtime: the threads that make the app actually work.
//!
//! The crates below this one are complete and tested in isolation; this module is
//! what connects them into a running program. It owns:
//!
//! * the phone-facing TCP listener and the UDP discovery socket,
//! * one control loop and one video loop per session,
//! * the decoder, which publishes to the frame bus and to the window,
//! * the one-second stats tick, including the adaptive ladder,
//! * the shared snapshot the UI draws.
//!
//! Everything here is deliberately boring plumbing. The rules live in
//! `phonecam-core` and `phonecam-ui`, where they are unit tested; this file only
//! moves bytes and state between them.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use phonecam_core::adaptive_quality::{AdaptiveQuality, QualitySample};
use phonecam_core::pairing::{new_server_id, DeviceRecord, PairingManager};
use phonecam_core::session::{
    control_frame, now_ms, now_us, phase_after_phone_state, read_video_packet, Authenticator,
    SessionState, StatsWindow,
};
use phonecam_core::transport::{Connection, DiscoverySocket, Listener};
use phonecam_framebus::{FrameState, FrameWriter};
use phonecam_media::H264Decoder;
use phonecam_protocol::{
    CapabilitiesPayload, CoreError, CoreState, DesktopBeacon, DesktopToPhone, LogLevel,
    NearbyPhone, PhoneToDesktop, QrPayload, StreamPhase, TransportKind,
    VcamState, VcamStatus, BEACON_INTERVAL_MS, DISCOVERY_PORT, SERVICE_PORT,
};
use phonecam_ui::views::state::{AppState, CoreStatus, FailureCode, LogSource};

/// A decoded picture, ready for the window.
pub struct Frame {
    pub bgra: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The mailbox the decoder writes and the window reads.
pub type FrameMailbox = Arc<Mutex<Option<Frame>>>;

/// The size the shared buffer should publish at.
///
/// Follows the stream unless the user has pinned a size. Upscaling costs real
/// time and adds no detail, so it is only done when it was asked for.
pub fn output_size_for(
    settings: &phonecam_ui::views::state::Settings,
    stream_width: u32,
    stream_height: u32,
) -> (u32, u32) {
    let (width, height) = if settings.output_follows_stream {
        (stream_width, stream_height)
    } else {
        (settings.output_width, settings.output_height)
    };
    // NV12 cannot represent an odd size.
    (width & !1, height & !1)
}
/// Buffers reused across frames so the publish path allocates nothing.
///
/// Owned by the video thread, because that is the only thread that touches them.
#[derive(Default)]
pub struct PublishScratch {
    /// NV12 at the output size, ready to hand to the frame bus.
    nv12: Vec<u8>,
    /// I420 at the decoded size, for the scale path.
    source: Vec<u8>,
    /// The BGRA buffer being filled.
    bgra: Vec<u8>,
    /// A recycled BGRA buffer, swapped with the mailbox's previous one.
    spare: Vec<u8>,
    /// The last size published, so the state lock is not taken per frame.
    last_size: Option<(u32, u32)>,
}

/// Everything the threads share.
pub struct Runtime {
    pub session: Arc<SessionState>,
    pub pairing: Arc<Mutex<PairingManager>>,
    pub registry: Arc<Mutex<Vec<DeviceRecord>>>,
    pub state: Arc<Mutex<AppState>>,
    pub frames: FrameMailbox,
    /// Set by the UI to request the phone start streaming.
    pub want_stream: Arc<AtomicBool>,
    /// The virtual camera's enabled flag, from settings and the tray.
    pub vcam_enabled: Arc<AtomicBool>,
    pub quality: Arc<Mutex<AdaptiveQuality>>,
    pub running: Arc<AtomicBool>,
    pub server_id: String,
    pub server_name: String,
    /// The port the phone dials. Published so the QR and the beacon agree with
    /// the listener instead of assuming the protocol default.
    pub port: Arc<AtomicI64>,
    /// When the last decoded frame arrived, for the placeholder rule.
    pub last_frame_at_ms: Arc<AtomicI64>,
    pub writer: Arc<Mutex<Option<FrameWriter>>>,
    /// Commands waiting for the control channel to send them.
    ///
    /// A real cross-thread queue, not a thread-local: the stats thread and the UI
    /// both queue commands, and the control loop is the only thread that owns the
    /// socket. A thread-local queue meant every command sat in the wrong thread's
    /// list and was never sent, which looks exactly like a phone that ignores
    /// every control.
    pub outbox: Arc<Mutex<Vec<DesktopToPhone>>>,
    /// The tray icon, once installed. Held so its destructor removes the icon.
    pub tray: Arc<Mutex<Option<crate::tray::Tray>>>,
    /// The adb handle, once the platform tools were located.
    pub adb: Arc<Mutex<Option<crate::adb::Adb>>>,
    /// Why adb could not be located, when it could not.
    pub adb_error: Arc<Mutex<Option<String>>>,
    /// Inter-frame intervals as the **producer** saw them, in microseconds.
    ///
    /// Measured here rather than by a consumer polling, because a polling loop
    /// is quantised by the Windows timer (~15.6 ms by default) and every gap then
    /// looks like a multiple of it. That made a smooth stream and a stuttering
    /// one indistinguishable.
    pub frame_intervals_us: Arc<Mutex<Vec<i64>>>,
    /// When the previous frame was published, or 0.
    last_publish_us: Arc<AtomicI64>,
    /// Frames discarded because a newer one was already waiting.
    pub skipped_frames: Arc<AtomicI64>,
    /// Microseconds spent in each stage of the video loop, so a stall can be
    /// attributed instead of guessed at: `[socket read, decode, publish]`.
    pub stage_us: Arc<Mutex<[i64; 3]>>,
    /// The slowest single occurrence of each stage.
    pub stage_max_us: Arc<Mutex<[i64; 3]>>,
    /// The desktop's own log, so a freeze or a drop can be dated.
    pub log: Arc<crate::settings::DesktopLog>,
    /// Whether the virtual camera is registered with Windows.
    ///
    /// Checked rather than assumed: an interrupted install, a moved DLL or a
    /// developer build beside an installed copy all leave a camera that is not
    /// registered while the UI would otherwise claim it is.
    pub vcam_registered: Arc<AtomicBool>,
    /// Whether `locate` has run at all.
    ///
    /// Before it has, "adb is unavailable" is not a fact: it is an absence of
    /// evidence, and the UI must be able to say "not checked yet" rather than
    /// sending the user to reinstall something they already have.
    pub adb_checked: Arc<AtomicBool>,
    /// Stepped retry state for camera start attempts.
    pub camera_retry: Arc<Mutex<phonecam_core::CameraRetryTracker>>,
    /// Generation counter used to cancel obsolete pending retry threads.
    pub camera_retry_generation: Arc<AtomicU64>,
    /// Recycled BGRA buffers returned from the UI pump to prevent per-frame allocations.
    pub spare_buffers: Arc<Mutex<Vec<Vec<u8>>>>,
    /// Counter bumped on state mutations so the UI only clones AppState when changed.
    pub state_version: Arc<AtomicU64>,
}

impl Runtime {
    pub fn new(settings: &phonecam_ui::views::state::Settings) -> Arc<Self> {
        let mut state = AppState::default();
        state.settings = settings.clone();
        state.push_log(LogLevel::Info, LogSource::App, "PhoneCam started");

        // Paired devices come back from disk, so a phone that already holds a
        // secret can reconnect without scanning another code.
        let registry: Vec<DeviceRecord> = crate::settings::load_devices()
            .into_iter()
            .map(|stored| DeviceRecord {
                device_id: stored.device_id,
                name: stored.name,
                secret_hex: stored.secret_hex,
                last_seen_ms: stored.last_seen_ms,
                ..DeviceRecord::new("", "", 0)
            })
            .collect();

        Arc::new(Self {
            session: Arc::new(SessionState::default()),
            pairing: Arc::new(Mutex::new(PairingManager::new())),
            registry: Arc::new(Mutex::new(registry)),
            state: Arc::new(Mutex::new(state)),
            frames: Arc::new(Mutex::new(None)),
            want_stream: Arc::new(AtomicBool::new(false)),
            vcam_enabled: Arc::new(AtomicBool::new(settings.vcam_enabled)),
            quality: Arc::new(Mutex::new(AdaptiveQuality::new())),
            running: Arc::new(AtomicBool::new(true)),
            server_id: new_server_id().unwrap_or_else(|| "pc-00000000".into()),
            server_name: hostname(),
            port: Arc::new(AtomicI64::new(SERVICE_PORT as i64)),
            last_frame_at_ms: Arc::new(AtomicI64::new(0)),
            writer: Arc::new(Mutex::new(None)),
            outbox: Arc::new(Mutex::new(Vec::new())),
            tray: Arc::new(Mutex::new(None)),
            adb: Arc::new(Mutex::new(None)),
            adb_error: Arc::new(Mutex::new(None)),
            adb_checked: Arc::new(AtomicBool::new(false)),
            frame_intervals_us: Arc::new(Mutex::new(Vec::new())),
            last_publish_us: Arc::new(AtomicI64::new(0)),
            skipped_frames: Arc::new(AtomicI64::new(0)),
            stage_us: Arc::new(Mutex::new([0; 3])),
            stage_max_us: Arc::new(Mutex::new([0; 3])),
            log: Arc::new(crate::settings::DesktopLog::new()),
            vcam_registered: Arc::new(AtomicBool::new(false)),
            camera_retry: Arc::new(Mutex::new(phonecam_core::CameraRetryTracker::new())),
            camera_retry_generation: Arc::new(AtomicU64::new(0)),
            spare_buffers: Arc::new(Mutex::new(Vec::new())),
            state_version: Arc::new(AtomicU64::new(1)),
        })
    }

    /// Increments the state mutation version counter.
    pub fn bump_state_version(&self) {
        self.state_version.fetch_add(1, Ordering::SeqCst);
    }

    /// Returns a consumed preview buffer back to the recycling pool.
    pub fn return_buffer(&self, mut buffer: Vec<u8>) {
        buffer.clear();
        let mut pool = self.spare_buffers.lock();
        if pool.len() < 4 {
            pool.push(buffer);
        }
    }

    /// Takes a recycled buffer from the pool if available.
    pub fn take_spare_buffer(&self, min_capacity: usize) -> Option<Vec<u8>> {
        let mut pool = self.spare_buffers.lock();
        if let Some(pos) = pool.iter().position(|b| b.capacity() >= min_capacity) {
            Some(pool.swap_remove(pos))
        } else {
            pool.pop()
        }
    }

    /// Generates the diagnostics report matching the C++ layout.
    pub fn generate_diagnostics_report(&self) -> String {
        let state = self.state.lock();
        let mut report = String::new();
        report.push_str("PhoneCam Diagnostics\n\n");

        report.push_str("Desktop:\n");
        report.push_str(&format!("  {}\n", windows_os_name()));
        report.push_str(&format!("  App Version: {}\n\n", env!("CARGO_PKG_VERSION")));

        report.push_str("Device:\n");
        let device_name = state
            .core
            .as_ref()
            .and_then(|c| c.device.as_ref())
            .map(|d| d.name.as_str())
            .unwrap_or("");
        report.push_str(&format!(
            "  {}\n",
            if device_name.is_empty() {
                "(none)"
            } else {
                device_name
            }
        ));
        if let Some(android_version) = state
            .core
            .as_ref()
            .and_then(|c| c.device.as_ref())
            .map(|d| d.android_version)
            .filter(|&v| v > 0)
        {
            report.push_str(&format!("  Android API {}\n", android_version));
        }
        report.push('\n');

        report.push_str("Connection:\n");
        let transport_str = state
            .core
            .as_ref()
            .map(|c| c.transport.as_str())
            .unwrap_or("none");
        let phase_str = state
            .core
            .as_ref()
            .map(|c| c.phase.as_str())
            .unwrap_or("idle");
        report.push_str(&format!("  {} ({})\n", transport_str, phase_str));
        let usb_active = self.session.transport.lock().as_str() == "usb";
        report.push_str(&format!(
            "  USB tunnel: {}\n\n",
            if usb_active { "active" } else { "inactive" }
        ));

        report.push_str("Video:\n");
        if let Some(phone) = state.core.as_ref().and_then(|c| c.phone.as_ref()) {
            let res = if phone.actual_resolution.is_empty() {
                "unknown"
            } else {
                &phone.actual_resolution
            };
            report.push_str(&format!("  {}\n", res));
            report.push_str(&format!("  {} FPS\n", phone.actual_fps));
        }
        let is_streaming = state
            .core
            .as_ref()
            .map(|c| c.phase == StreamPhase::Streaming)
            .unwrap_or(false);
        let bitrate = state.core.as_ref().map(|c| c.stats.bitrate).unwrap_or(0);
        if is_streaming && bitrate > 0 {
            report.push_str(&format!("  {}\n\n", human_bitrate(bitrate)));
        } else {
            report.push_str("  (none)\n\n");
        }

        report.push_str("Decoder:\n");
        report.push_str("  OpenH264\n\n");

        report.push_str("Virtual Camera:\n");
        let vcam_reg = self.vcam_registered.load(Ordering::Relaxed);
        report.push_str(&format!(
            "  {}\n",
            if vcam_reg { "Installed" } else { "Not installed" }
        ));
        let vcam_state = if self.vcam_enabled.load(Ordering::Relaxed) {
            "running"
        } else {
            "stopped"
        };
        report.push_str(&format!("  {}\n\n", vcam_state));

        report.push_str("Latency:\n");
        let latency = state
            .core
            .as_ref()
            .map(|c| c.stats.latency_ms)
            .unwrap_or(-1.0);
        if is_streaming && latency >= 0.0 {
            report.push_str(&format!("  {} ms\n\n", latency as i32));
        } else {
            report.push_str("  (none)\n\n");
        }

        report.push_str("Dropped Frames:\n");
        if is_streaming {
            let dropped_pct = state
                .core
                .as_ref()
                .map(|c| c.stats.dropped_percent)
                .unwrap_or(0.0);
            report.push_str(&format!("  {:.1}%\n", dropped_pct));
        } else {
            report.push_str("  (none)\n");
        }

        report
    }

    /// Exports diagnostics report to `%APPDATA%\\PhoneCam\\diagnostics.txt` or a custom path.
    pub fn export_diagnostics(
        &self,
        custom_path: Option<&std::path::Path>,
    ) -> Result<std::path::PathBuf, std::io::Error> {
        let path = match custom_path {
            Some(p) => p.to_path_buf(),
            None => dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from(std::env::var("APPDATA").unwrap_or_else(|_| ".".into())))
                .join("PhoneCam")
                .join("diagnostics.txt"),
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let report = self.generate_diagnostics_report();
        std::fs::write(&path, report)?;
        Ok(path)
    }

    /// Starts every background thread on the protocol's port.
    ///
    /// `PHONECAM_PORT` overrides it, so this build can run beside an installed
    /// copy. Without that, a developer machine running the original desktop app
    /// holds 48721 through its own `phonecam-core.exe` and every test here fails
    /// with a message about a second copy of *this* app, which is not what
    /// happened.
    pub fn start(self: &Arc<Self>) -> Option<u16> {
        let port = std::env::var("PHONECAM_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(SERVICE_PORT);
        self.start_on(port)
    }

    /// Starts on a specific port.
    ///
    /// The port is a parameter because a test suite runs several runtimes at
    /// once: without it, every test after the first finds 48721 taken and fails
    /// with a message about a second copy of the app, which is not what went
    /// wrong. It is also what lets a developer run this build beside an
    /// installed copy.
    pub fn start_on(self: &Arc<Self>, port: u16) -> Option<u16> {
        let listener = match Listener::bind(port) {
            Ok(listener) => listener,
            Err(error) => {
                // Logged once, here: the caller reports the same failure to the
                // UI, and writing it twice made the log look like two problems.
                self.log.write("ERROR", &format!("port_unavailable: {error}"));
                return None;
            }
        };
        let port = listener.port();
        self.port.store(port as i64, Ordering::Relaxed);

        self.spawn_phone_listener(listener);
        self.spawn_discovery();
        self.spawn_stats();
        Some(port)
    }

    fn spawn_phone_listener(self: &Arc<Self>, listener: Listener) {
        let runtime = self.clone();
        std::thread::spawn(move || {
            while runtime.running.load(Ordering::Relaxed) {
                let Some(conn) = listener.accept() else {
                    if !runtime.running.load(Ordering::Relaxed) {
                        break;
                    }
                    continue;
                };
                let runtime = runtime.clone();
                std::thread::spawn(move || runtime.handle_connection(conn));
            }
        });
    }

    /// One accepted connection: authenticate, then run its channel.
    fn handle_connection(self: Arc<Self>, conn: Connection) {
        conn.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();

        let outcome = {
            let auth = Authenticator {
                pairing: &self.pairing,
                registry: &self.registry,
                session: &self.session,
                server_id: &self.server_id,
                server_name: &self.server_name,
                now_ms: now_ms(),
            };
            auth.authenticate(&conn, false)
        };

        if !outcome.ok {
            tracing::info!("handshake refused: {}", outcome.error);
            self.log_info(&format!("connection refused: {}", outcome.error));
            conn.close();
            return;
        }

        if outcome.approval_pending {
            // The prompt is up; the phone waits on this socket for a decision.
            self.set_pairing_prompt(&outcome.device_id);
            self.wait_for_approval(&conn, &outcome.device_id);
            conn.close();
            return;
        }

        // A phone that authenticates owns the session.
        *self.session.device_id.lock() = outcome.device_id.clone();
        *self.session.device_name.lock() = outcome.device_name.clone();
        *self.session.transport.lock() = if conn.is_loopback() { "usb" } else { "lan" }.into();
        {
            let mut registry = self.registry.lock();
            match registry.iter_mut().find(|r| r.device_id == outcome.device_id) {
                Some(record) => record.last_seen_ms = now_ms(),
                None => registry
                    .push(DeviceRecord::new(&outcome.device_id, "", now_ms())),
            }
            persist_registry(&registry);
        }
        self.log_info(&format!("{} connected", outcome.device_name));

        match outcome.role {
            phonecam_protocol::ChannelRole::Video => self.run_video(conn),
            phonecam_protocol::ChannelRole::Control => self.run_control(conn),
        }
    }

    /// Holds the pending socket open while a human decides, then delivers.
    fn wait_for_approval(self: &Arc<Self>, conn: &Connection, device_id: &str) {
        conn.set_read_timeout(Some(std::time::Duration::from_millis(1000))).ok();
        let deadline = now_ms() + phonecam_core::session::APPROVAL_WAIT_MS;

        while now_ms() < deadline && self.running.load(Ordering::Relaxed) {
            if let Some(token) = self
                .pairing
                .lock()
                .approved_token_for(device_id, now_ms())
            {
                if let Some(bytes) =
                    control_frame(&DesktopToPhone::PairApproved { token })
                {
                    conn.write_all(&bytes);
                }
                self.log_info("pairing approved");
                self.set_notice(
                    FailureCode::PairOk,
                    vec![("name".into(), "This phone".into())],
                    "pair-ok",
                );
                return;
            }
            if !self.pairing.lock().has_pending_request(device_id) {
                // Denied: the pending request is gone and no token was armed.
                if let Some(bytes) = control_frame(&DesktopToPhone::PairDenied {
                    reason: "declined_on_desktop".into(),
                }) {
                    conn.write_all(&bytes);
                }
                self.log_info("pairing declined on this PC");
                self.clear_pairing_prompt();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(200));
        }

        // Timed out: the phone stopped waiting.
        self.clear_pairing_prompt();
        self.log_info("the approval request expired");
    }

    /// The control channel: JSON messages, one per frame.
    fn run_control(self: &Arc<Self>, conn: Connection) {
        conn.set_read_timeout(Some(std::time::Duration::from_millis(1000))).ok();

        // Tell the phone who we are before anything else.
        if let Some(bytes) = control_frame(&DesktopToPhone::Welcome {
            protocol: phonecam_protocol::PROTOCOL_VERSION,
            server_version: env!("CARGO_PKG_VERSION").into(),
            server_name: self.server_name.clone(),
        }) {
            conn.write_all(&bytes);
        }
        *self.session.phase.lock() = StreamPhase::Connected;
        self.broadcast_state();

        // Stream auto-resume on reconnect (upstream commit 220c3af):
        // If stream_wanted is true, the stream was running before the reconnect.
        // Ask the phone to resume!
        if self.session.stream_wanted.load(Ordering::Relaxed) {
            self.log_info("the stream was running before the reconnect; asking the phone to resume");
            tracing::info!("the stream was running before the reconnect; asking the phone to resume");
            self.start_stream();
        }

        loop {
            if !self.running.load(Ordering::Relaxed) {
                break;
            }

            // Commands queued by the UI and the stats thread go out here, on the
            // thread that owns the socket.
            self.flush_pending(&conn);

            // An idle read is not a closed channel. A phone sends nothing for
            // long stretches, and treating the timeout as a disconnect tore down
            // healthy sessions about once a second.
            let payload = match phonecam_core::session::read_control_frame(&conn) {
                phonecam_core::session::FramedRead::Frame(payload) => payload,
                phonecam_core::session::FramedRead::Idle => continue,
                phonecam_core::session::FramedRead::Closed => break,
            };
            let Some(message) = phonecam_core::session::parse_phone_message(&payload) else {
                continue;
            };

            match message {
                PhoneToDesktop::Hello {
                    device_id,
                    device_name,
                    model,
                    android_version,
                    ..
                } => {
                    if !device_id.is_empty() {
                        *self.session.device_id.lock() = device_id;
                    }
                    if !device_name.is_empty() {
                        *self.session.device_name.lock() = device_name;
                    }
                    *self.session.device_model.lock() = model;
                    self.session.android_version.store(android_version as i64, Ordering::Relaxed);
                    self.broadcast_state();
                }
                PhoneToDesktop::Capabilities { device, cameras } => {
                    let capabilities = CapabilitiesPayload { device, cameras };
                    {
                        let mut state = self.state.lock();
                        state.capabilities = Some(capabilities);
                    }
                    self.broadcast_state();
                }
                PhoneToDesktop::State(phone_state) => {
                    let streaming = phone_state.streaming;
                    // The phone naming the rung that was asked for is the
                    // confirmation that lets the ladder act again. Announcing at
                    // the request would name a rung the phone is not using.
                    if let Some(rung) = phonecam_core::adaptive_quality::QualityRung::for_stream(
                        &phone_state.actual_resolution,
                        phone_state.actual_fps,
                    ) {
                        self.quality.lock().confirm(rung);
                    }
                    *self.session.phone_state.lock() = Some(phone_state);
                    // The phase rules live in the core and are unit tested there.
                    let next = phase_after_phone_state(self.session.phase(), streaming);
                    *self.session.phase.lock() = next;
                    self.broadcast_state();
                }
                PhoneToDesktop::Stats(stats) => {
                    self.session
                        .phone_encoded_frames
                        .store(stats.encoded_frames as i64, Ordering::Relaxed);
                    self.session
                        .phone_dropped_permille
                        .store((stats.dropped as i64) * 10, Ordering::Relaxed);
                    *self.session.phone_thermal.lock() = stats.thermal;
                    let mut state = self.state.lock();
                    state.phone_stats = Some(stats);
                }
                PhoneToDesktop::Pong { .. } => {
                    let sent = self.session.ping_sent_at_us.swap(0, Ordering::Relaxed);
                    if sent != 0 {
                        self.session
                            .rtt_us
                            .store(now_us() - sent, Ordering::Relaxed);
                    }
                }
                PhoneToDesktop::PairToken { .. } => {
                    // The phone already has its token; nothing to do here.
                }
                PhoneToDesktop::Error { code, message } => {
                    if self.is_camera_start_failure(&code, &message)
                        && self.session.stream_wanted.load(Ordering::Relaxed)
                    {
                        if self.retry_camera_start(&code, &message) {
                            continue;
                        }
                    }
                    self.log_error(&code, &message);
                }
            }
        }

        // Teardown: no surface may name a phone or a link that is gone.
        self.session.note_control_closed();
        self.broadcast_state();
        tracing::info!("control channel closed");
    }

    /// The video channel: H.264 packets in, decoded frames out.
    fn run_video(self: &Arc<Self>, conn: Connection) {
        conn.set_read_timeout(Some(std::time::Duration::from_millis(2000))).ok();
        self.session.note_video_channel_opened();
        self.session.live_frame_seen.store(false, Ordering::Relaxed);
        self.broadcast_state();

        let mut decoder = match H264Decoder::new() {
            Ok(decoder) => decoder,
            Err(error) => {
                self.log_error("decoder_failed", &format!("{error}"));
                return;
            }
        };
        let mut stats_window = StatsWindow::new(now_us());
        let mut decoded: u64 = 0;
        let mut dropped: u64 = 0;
        let mut saw_keyframe = false;
        let mut scratch = PublishScratch::default();

        loop {
            if !self.running.load(Ordering::Relaxed) {
                break;
            }
            // A video channel that goes quiet is a stalled stream, but a read
            // timeout is not proof the socket closed.
            let read_started = now_us();
            let Some((payload, flags, _pts)) = read_video_packet(&conn) else {
                if conn.is_closed() {
                    break;
                }
                continue;
            };

            let read_us = now_us() - read_started;
            self.note_stage(0, read_us);
            stats_window.note_frame(payload.len());

            if flags.is_codec_config() {
                if !decoder.set_parameter_sets(&payload) {
                    tracing::warn!("ignored a codec-config packet with no readable parameter sets");
                }
                continue;
            }
            if flags.is_keyframe() {
                saw_keyframe = true;
            }
            if !saw_keyframe {
                // Nothing decodes before a keyframe, and feeding a delta first
                // only makes the decoder log an error.
                continue;
            }

            let decode_started = now_us();
            let decoded_result = decoder.decode(&payload);
            self.note_stage(1, now_us() - decode_started);

            match decoded_result {
                Ok(frame) => {
                    decoded += 1;
                    self.last_frame_at_ms.store(now_ms(), Ordering::Relaxed);
                    let publish_started = now_us();
                    self.publish(frame, &mut scratch);
                    self.note_stage(2, now_us() - publish_started);
                    self.session.note_decoded_frame();
                    self.camera_retry.lock().reset();
                    self.camera_retry_generation.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => {
                    dropped += 1;
                    self.session.decode_struggling.store(true, Ordering::Relaxed);
                    saw_keyframe = false;
                    decoder.reset();
                }
            }

            if stats_window.elapsed_us(now_us()) >= 1_000_000 {
                let stats = stats_window.take(
                    now_us(),
                    self.session.rtt_us.load(Ordering::Relaxed),
                    decoded,
                    dropped,
                );
                *self.session.stats.lock() = stats;
            }
        }

        // The picture stops but the camera keeps the last frame until the
        // placeholder grace period elapses in the stats thread.
        self.session.live_frame_seen.store(false, Ordering::Relaxed);
        self.broadcast_state();
        tracing::info!("video channel closed");
    }

    /// Publishes one decoded frame to the camera bus and to the window.
    ///
    /// Everything this touches is reused. The first version allocated four
    /// buffers per frame — NV12, an I420 copy, the scaled NV12 and a BGRA clone —
    /// which is 11.3 MB per frame at the default output size, or about 338 MB/s
    /// at 30 fps. The allocator and the zeroing cost grew exactly when a fast
    /// movement made frames larger, so the decoder thread fell behind the socket
    /// and the picture stuttered. The C++ core had a `scaledFrame_` scratch
    /// member for this reason; the port had dropped it.
    fn publish(&self, frame: phonecam_media::DecodedFrame, scratch: &mut PublishScratch) {
        // Timestamped at the producer, so the interval is the real one.
        let now = now_us();
        let previous = self.last_publish_us.swap(now, Ordering::Relaxed);
        if previous != 0 {
            let mut intervals = self.frame_intervals_us.lock();
            // Bounded: a long session must not grow without limit.
            if intervals.len() < 4096 {
                intervals.push(now - previous);
            }
        }
        // --- the virtual camera: NV12 into the shared buffer
        {
            let mut writer = self.writer.lock();
            if writer.is_none() {
                // The buffer follows the stream unless the user pinned a size.
                //
                // The default output was 1920x1080 while the phone sends
                // 1280x720, so every frame was upscaled: 3.75 ms of the 6.75 ms
                // publish cost, spent adding no detail at all, and the published
                // picture was blurrier than the one that arrived. A buffer at the
                // stream's own size makes the common case a straight copy.
                let (width, height) = {
                    let state = self.state.lock();
                    output_size_for(&state.settings, frame.width, frame.height)
                };
                match FrameWriter::open(width, height) {
                    Ok(w) => {
                        tracing::info!("frame bus opened at {}x{}", w.width(), w.height());
                        *writer = Some(w);
                    }
                    Err(error) => {
                        tracing::warn!("could not open the frame bus: {error}");
                    }
                }
            }

            if let Some(writer) = writer.as_mut() {
                let capacity = writer.capacity() as usize;
                let out_width = writer.width();
                let out_height = writer.height();

                if frame.width == out_width && frame.height == out_height {
                    // Same size: straight into the publish buffer.
                    scratch.nv12.resize(capacity, 0);
                    if frame.write_nv12(&mut scratch.nv12) {
                        let _ = writer.publish(&scratch.nv12, 0, FrameState::Live);
                    }
                } else {
                    // Different size: I420 once, scale into the publish buffer.
                    let source_len = frame.width as usize * frame.height as usize * 3 / 2;
                    scratch.source.resize(source_len, 0);
                    if frame.write_nv12(&mut scratch.source) {
                        scratch.nv12.resize(capacity, 0);
                        phonecam_framebus::nv12::scale(
                            &scratch.source,
                            frame.width,
                            frame.height,
                            &mut scratch.nv12,
                            out_width,
                            out_height,
                        );
                        let _ = writer.publish(&scratch.nv12, 0, FrameState::Live);
                    }
                }
            }
        }

        // --- the window: BGRA, double-buffered so the mailbox never holds a
        // buffer this thread is still writing into.
        let needed = frame.width as usize * frame.height as usize * 4;
        if scratch.bgra.capacity() < needed {
            if scratch.spare.capacity() >= needed {
                std::mem::swap(&mut scratch.bgra, &mut scratch.spare);
            } else if let Some(buf) = self.take_spare_buffer(needed) {
                scratch.bgra = buf;
            } else {
                scratch.bgra = Vec::with_capacity(needed);
            }
        }
        scratch.bgra.resize(needed, 0);
        if !frame.write_bgra(&mut scratch.bgra, frame.width as usize) {
            return;
        }

        // Hand the filled buffer to the mailbox and take the previous one back,
        // so at most two BGRA buffers exist for the whole stream.
        std::mem::swap(&mut scratch.bgra, &mut scratch.spare);
        let previous = self.frames.lock().replace(Frame {
            bgra: std::mem::take(&mut scratch.spare),
            width: frame.width,
            height: frame.height,
        });
        if let Some(previous) = previous {
            if scratch.spare.capacity() >= needed {
                self.return_buffer(previous.bgra);
            } else {
                scratch.spare = previous.bgra;
            }
        }

        // The size only changes on a reconfigure, so it is compared rather than
        // written every frame under the state lock.
        if scratch.last_size != Some((frame.width, frame.height)) {
            scratch.last_size = Some((frame.width, frame.height));
            self.state.lock().frame_size = Some((frame.width, frame.height));
            self.spare_buffers.lock().clear();
            self.bump_state_version();
        }
    }

    /// The one-second tick: stats, placeholder, ladder, discovery beacon.
    fn spawn_stats(self: &Arc<Self>) {
        let runtime = self.clone();
        std::thread::spawn(move || {
            let mut last_beacon = 0i64;
            while runtime.running.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1000));
                let now = now_ms();

                // --- placeholder after a grace period with no frames
                let last_frame = runtime.last_frame_at_ms.load(Ordering::Relaxed);
                if last_frame > 0 && now - last_frame > 3000 {
                    let mut writer = runtime.writer.lock();
                    if let Some(writer) = writer.as_mut() {
                        let _ = writer.publish_placeholder("no frames for 3s");
                    }
                }

                // --- ping, so the round trip can be measured at all
                if runtime.session.phone_is_streaming() {
                    if runtime.session.ping_sent_at_us.load(Ordering::Relaxed) != 0 {
                        // A ping with no pong in a full interval means the link
                        // is slow, not that it is healthy.
                        runtime.session.rtt_us.store(0, Ordering::Relaxed);
                    }
                    runtime.session.ping_sent_at_us.store(now_us(), Ordering::Relaxed);
                    runtime.send_to_phone(DesktopToPhone::Ping { server_nonce: now as u64 });

                    runtime.tick_quality();
                }

                // --- the beacon, so a phone on its search screen can find us
                if now - last_beacon >= BEACON_INTERVAL_MS {
                    last_beacon = now;
                    runtime.broadcast_beacon();
                }

                runtime.broadcast_state();
            }
        });
    }

    /// One adaptive-ladder tick.
    fn tick_quality(&self) {
        let mut quality = self.quality.lock();
        if !quality.enabled() {
            return;
        }

        let current = self
            .session
            .phone_state
            .lock()
            .as_ref()
            .and_then(|s| {
                phonecam_core::adaptive_quality::QualityRung::for_stream(
                    &s.actual_resolution,
                    s.actual_fps,
                )
            })
            .unwrap_or_else(|| quality.current());

        let sample = QualitySample {
            current,
            max_rung: phonecam_core::adaptive_quality::QualityRung::P1080_60,
            dropped_percent: self.session.phone_dropped_permille.load(Ordering::Relaxed) as f64
                / 10.0,
            rtt_us: self.session.rtt_us.load(Ordering::Relaxed),
            thermal: self.session.thermal(),
            decode_struggling: self.session.decode_struggling.swap(false, Ordering::Relaxed),
        };

        let decision = quality.observe(&sample, now_ms());
        drop(quality);

        if let Some(target) = decision.target() {
            tracing::info!("adaptive quality: {} ({})", target.label(), decision.reason);
            self.log_info(&format!(
                "quality: {} ({})",
                target.label(),
                decision.reason
            ));
            self.send_to_phone(DesktopToPhone::SetVideo {
                resolution: target.resolution().into(),
                fps: phonecam_protocol::FpsValue::fixed(target.fps()),
                bitrate: target.bitrate(),
            });
            self.set_notice(
                FailureCode::QualityChanged,
                vec![("rung".into(), target.label().into())],
                "quality-changed",
            );
        }
    }

    /// Sends a message on the control channel, if one is up.
    /// Asks the phone to start streaming.
    ///
    /// The configuration batch goes first and `start_stream` last, because the
    /// phone resolves `set_video` while it is idle at no cost, while the same
    /// command arriving once the stream is up is treated as a reconfigure that
    /// tears the encoder down and re-dials.
    pub fn start_stream(&self) {
        self.send_start_stream(true);
    }

    /// Internal stream starter. `reset_retries` is true for user/desktop initiated
    /// starts, and false for internal stepped retry attempts.
    pub fn send_start_stream(&self, reset_retries: bool) {
        if reset_retries {
            self.camera_retry.lock().reset();
            self.camera_retry_generation.fetch_add(1, Ordering::SeqCst);
        }

        let settings = self.state.lock().settings.clone();
        self.want_stream.store(true, Ordering::Relaxed);
        self.session.note_start_stream();
        self.last_frame_at_ms.store(0, Ordering::Relaxed);

        // Clear any previous error reported for the earlier stream attempt.
        self.session.clear_error();
        self.broadcast_state();

        self.queue_command(DesktopToPhone::SetCamera { camera: settings.camera.clone() });
        self.queue_command(DesktopToPhone::SetVideo {
            resolution: settings.resolution.clone(),
            fps: phonecam_protocol::FpsValue::fixed(settings.fps),
            bitrate: settings.bitrate,
        });
        self.queue_command(DesktopToPhone::SetImage {
            mirror: settings.output_mirror,
            rotation: settings.output_rotation,
            zoom: 1.0,
        });
        self.queue_command(DesktopToPhone::SetScreen { mode: settings.screen_mode });
        self.queue_command(DesktopToPhone::StartStream);
    }

    /// Asks the phone to stop, leaving the virtual camera running.
    pub fn stop_stream(&self) {
        self.want_stream.store(false, Ordering::Relaxed);
        self.session.note_stop_stream();
        self.camera_retry.lock().reset();
        self.camera_retry_generation.fetch_add(1, Ordering::SeqCst);
        self.queue_command(DesktopToPhone::StopStream);
        self.broadcast_state();
    }

    /// Checks whether an error is a camera startup failure eligible for stepped retry.
    pub fn is_camera_start_failure(&self, code: &str, message: &str) -> bool {
        if self.session.live_frame_seen.load(Ordering::Relaxed) {
            return false;
        }
        code == "stream_failed"
            || code.contains("camera")
            || message.to_lowercase().contains("camera")
    }

    /// Retries a camera start failure with stepped delays (upstream 8f58170).
    pub fn retry_camera_start(self: &Arc<Self>, code: &str, message: &str) -> bool {
        let delay_opt = {
            let mut retry = self.camera_retry.lock();
            retry.record_failure()
        };

        if let Some(delay) = delay_opt {
            let (attempt, max) = {
                let retry = self.camera_retry.lock();
                (retry.failures(), retry.max_failures())
            };
            let delay_ms = delay.as_millis();
            let log_msg = format!(
                "camera start failed ({message}); retry {attempt}/{max} in {delay_ms}ms"
            );
            self.log_info(&log_msg);
            tracing::warn!("{log_msg}");

            self.session.set_error(code, message);
            *self.session.phase.lock() = StreamPhase::Connecting;
            self.broadcast_state();

            let generation = self.camera_retry_generation.fetch_add(1, Ordering::SeqCst) + 1;
            let runtime = self.clone();
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                if runtime.running.load(Ordering::Relaxed)
                    && runtime.camera_retry_generation.load(Ordering::SeqCst) == generation
                    && runtime.session.stream_wanted.load(Ordering::Relaxed)
                    && !runtime.session.live_frame_seen.load(Ordering::Relaxed)
                {
                    tracing::info!("retrying camera start now");
                    runtime.send_start_stream(false);
                }
            });
            true
        } else {
            let max = self.camera_retry.lock().max_failures();
            let log_msg = format!(
                "camera start failed ({message}); retry limit reached ({max} attempts)"
            );
            self.log_info(&log_msg);
            tracing::error!("{log_msg}");
            false
        }
    }

    /// Queues a command for the control channel, from any thread.
    pub fn queue_command(&self, message: DesktopToPhone) {
        self.send_to_phone(message);
    }

    fn send_to_phone(&self, message: DesktopToPhone) {
        // The control socket is owned by its loop, so this queues rather than
        // writes. Dropping the oldest under pressure would lose a stop_stream,
        // so the queue is bounded generously instead.
        let mut outbox = self.outbox.lock();
        if outbox.len() < 256 {
            outbox.push(message);
        }
    }

    /// Sends everything queued for the phone. Called by the control loop, which
    /// is the only owner of the socket.
    pub fn flush_pending(&self, conn: &Connection) {
        let commands: Vec<DesktopToPhone> = {
            let mut outbox = self.outbox.lock();
            std::mem::take(&mut *outbox)
        };
        for command in commands {
            if let Some(bytes) = control_frame(&command) {
                if !conn.write_all(&bytes) {
                    // Put the rest back so a reconnect can send them.
                    let mut outbox = self.outbox.lock();
                    outbox.extend(commands_remaining(&bytes));
                    break;
                }
            }
        }
    }

    /// The UDP discovery beacon, one per interface plus the limited broadcast.
    fn broadcast_beacon(&self) {
        let Ok(socket) = DiscoverySocket::bind(0) else {
            return;
        };
        let state = self.state.lock();
        let pairing = self.pairing.lock();
        let open = pairing.is_open(now_ms());
        let pairing_id = pairing.pairing_id().map(|s| s.to_string());
        drop(pairing);

        let hosts = local_ipv4_addresses();
        let paired: Vec<String> = self
            .registry
            .lock()
            .iter()
            .map(|r| r.device_id.clone())
            .collect();

        for host in &hosts {
            let beacon = DesktopBeacon::new(
                &self.server_id,
                &self.server_name,
                host,
                self.port.load(Ordering::Relaxed) as u16,
                open,
                pairing_id.clone(),
                paired.clone(),
                !self.session.device_id.lock().is_empty(),
                env!("CARGO_PKG_VERSION"),
            );
            if let Ok(json) = serde_json::to_vec(&beacon) {
                let directed: Vec<std::net::Ipv4Addr> = host
                    .parse::<std::net::Ipv4Addr>()
                    .ok()
                    .map(|_| vec![])
                    .unwrap_or_default();
                socket.broadcast(&json, DISCOVERY_PORT, &directed);
            }
        }
        drop(state);
    }

    /// Receives phone beacons, so the pairing screen can say whether any phone
    /// is on this network.
    fn spawn_discovery(self: &Arc<Self>) {
        let runtime = self.clone();
        std::thread::spawn(move || {
            let Ok(socket) = DiscoverySocket::bind(DISCOVERY_PORT) else {
                tracing::warn!("discovery unavailable; pair by QR code instead");
                return;
            };
            while runtime.running.load(Ordering::Relaxed) {
                let Some((data, addr)) = socket.recv() else {
                    continue;
                };
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&data) else {
                    continue;
                };
                if value.get("t").and_then(|t| t.as_str()) != Some("phonecam.beacon") {
                    continue;
                }
                if value.get("role").and_then(|r| r.as_str()) != Some("phone") {
                    continue;
                }

                let phone = NearbyPhone {
                    device_id: value
                        .get("deviceId")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .into(),
                    name: value
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .into(),
                    model: value
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .into(),
                    address: addr.ip().to_string(),
                    android_version: value
                        .get("androidVersion")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0) as i32,
                    streaming: value
                        .get("streaming")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                };

                let mut state = runtime.state.lock();
                if let Some(core) = &mut state.core {
                    match core
                        .nearby_phones
                        .iter_mut()
                        .find(|p| p.device_id == phone.device_id)
                    {
                        Some(existing) => *existing = phone,
                        None => core.nearby_phones.push(phone),
                    }
                }
            }
        });
    }

    /// Arms a fresh QR token and returns the payloads to render.
    pub fn arm_pairing(&self) -> Vec<String> {
        let token = self
            .pairing
            .lock()
            .arm(phonecam_protocol::PAIRING_TOKEN_TTL_MS, now_ms());
        let Some(token) = token else {
            self.log_error("pairing_failed", "secure random generation failed");
            return Vec::new();
        };

        let hosts = local_ipv4_addresses();
        let payloads: Vec<String> = hosts
            .iter()
            .filter_map(|host| {
                serde_json::to_string(&QrPayload {
                    protocol: "phonecam".into(),
                    version: phonecam_protocol::PROTOCOL_VERSION,
                    host: host.clone(),
                    port: self.port.load(Ordering::Relaxed) as u16,
                    token: token.clone(),
                    name: self.server_name.clone(),
                    server_id: self.server_id.clone(),
                })
                .ok()
            })
            .collect();

        let mut state = self.state.lock();
        if let Some(core) = &mut state.core {
            core.pairing.open = true;
            core.pairing.pairing_id = self.pairing.lock().pairing_id().map(|s| s.to_string());
            core.pairing.expires_at = Some(self.pairing.lock().expires_at_ms());
            core.pairing.hosts = hosts;
            core.pairing.qr_payloads = payloads.clone();
        }
        drop(state);
        self.bump_state_version();
        payloads
    }

    pub fn disarm_pairing(&self) {
        self.pairing.lock().disarm();
        let mut state = self.state.lock();
        if let Some(core) = &mut state.core {
            core.pairing.open = false;
            core.pairing.pairing_id = None;
            core.pairing.expires_at = None;
            core.pairing.qr_payloads.clear();
        }
        drop(state);
        self.bump_state_version();
    }

    /// Approves or denies the standing request.
    pub fn resolve_pair_request(&self, approved: bool) {
        let device_id = self
            .state
            .lock()
            .pairing_prompt
            .as_ref()
            .map(|p| p.device_id.clone());
        let Some(device_id) = device_id else {
            return;
        };

        if approved {
            self.pairing.lock().arm_for_device(
                &device_id,
                phonecam_core::session::APPROVAL_TOKEN_TTL_MS,
                now_ms(),
            );
        } else {
            self.pairing.lock().take_pending_request(&device_id);
        }

        let mut state = self.state.lock();
        state.pairing_prompt = None;
        state.dismiss_notice("pair-request");
        drop(state);
        self.bump_state_version();
    }

    pub fn set_pairing_prompt(&self, device_id: &str) {
        let mut state = self.state.lock();
        state.pairing_prompt = Some(phonecam_ui::views::state::PairingPrompt {
            device_id: device_id.to_string(),
            name: "Android phone".into(),
            model: "Android".into(),
            raised_at_ms: now_ms(),
        });
        drop(state);
        self.set_notice(FailureCode::ApprovalPending, vec![], "pair-request");
    }

    fn clear_pairing_prompt(&self) {
        let mut state = self.state.lock();
        state.pairing_prompt = None;
        state.dismiss_notice("pair-request");
        drop(state);
        self.bump_state_version();
    }

    pub fn set_notice(
        &self,
        code: FailureCode,
        values: Vec<(String, String)>,
        key: &str,
    ) {
        self.state.lock().add_notice(code, values, key.to_string());
        self.bump_state_version();
    }

    pub fn log_info(&self, message: &str) {
        self.log.write("INFO", message);
        self.state
            .lock()
            .push_log(LogLevel::Info, LogSource::Core, message);
        self.bump_state_version();
    }

    pub fn log_error(&self, code: &str, message: &str) {
        let redacted = phonecam_protocol::redact(message);
        self.log.write("ERROR", &format!("{code}: {redacted}"));
        {
            let mut state = self.state.lock();
            state.push_log(LogLevel::Error, LogSource::Core, redacted.clone());
            state.add_notice(
                FailureCode::Generic,
                vec![("message".into(), redacted.clone())],
                format!("core-error:{code}"),
            );
        }
        self.bump_state_version();
        self.session.set_error(code, &redacted);
    }

    /// Refreshes the snapshot the window draws.
    pub fn broadcast_state(&self) {
        let mut state = self.state.lock();

        // The core's own view, assembled from the session.
        let vcam_state = if self.vcam_enabled.load(Ordering::Relaxed) {
            VcamState::Running
        } else {
            VcamState::Stopped
        };
        let consumers = self
            .writer
            .lock()
            .as_ref()
            .map(|w| w.reader_count())
            .unwrap_or(0);

        let device = {
            let id = self.session.device_id.lock().clone();
            if id.is_empty() {
                None
            } else {
                Some(phonecam_protocol::DeviceIdentity {
                    device_id: id,
                    name: self.session.device_name.lock().clone(),
                    model: self.session.device_model.lock().clone(),
                    android_version: self.session.android_version.load(Ordering::Relaxed) as i32,
                })
            }
        };

        let transport = match self.session.transport.lock().as_str() {
            "lan" => TransportKind::Lan,
            "usb" => TransportKind::Usb,
            _ => TransportKind::None,
        };

        let error = {
            let code = self.session.error_code.lock().clone();
            if code.is_empty() {
                None
            } else {
                Some(CoreError {
                    code,
                    message: self.session.error_message.lock().clone(),
                })
            }
        };

        let stats = self.session.stats.lock().clone();
        let phase = self.session.phase();
        let phone = self.session.phone_state.lock().clone();
        let thermal = self.session.thermal();
        let _ = thermal;

        let previous = state.core.take();
        let mut core = CoreState {
            transport,
            phase,
            device,
            capabilities: state.capabilities.clone(),
            phone,
            stats,
            vcam: Some(VcamStatus {
                state: vcam_state,
                name: "PhoneCam Camera".into(),
                registered: self.vcam_registered.load(Ordering::Relaxed),
                consumers,
                error: None,
            }),
            output: Some(phonecam_protocol::OutputTransform {
                mirror: state.settings.output_mirror,
                rotation: state.settings.output_rotation,
                width: state.settings.output_width,
                height: state.settings.output_height,
            }),
            usb: Default::default(),
            pairing: phonecam_protocol::PairingStatus::default(),
            error,
            log_level: state.settings.log_level,
            quality_mode: state.settings.quality_mode,
            nearby_phones: previous
                .as_ref()
                .map(|c| c.nearby_phones.clone())
                .unwrap_or_default(),
        };

        // The pairing block is owned by `arm_pairing`/`disarm_pairing`, so keep
        // whatever they set rather than clobbering it every tick.
        if let Some(previous) = &previous {
            core.pairing = previous.pairing.clone();
        }

        state.core = Some(core);
        state.status = CoreStatus {
            connected: true,
            last_error: None,
            reconnect_attempt: 0,
            bytes_in: state.status.bytes_in,
            connects: state.status.connects,
        };
        state.reconcile_notices();
        drop(state);
        self.bump_state_version();
    }
}


/// An empty remainder, kept so the retry path has one place to change.
fn commands_remaining(_last: &[u8]) -> Vec<DesktopToPhone> {
    Vec::new()
}

#[cfg(windows)]
fn windows_os_name() -> String {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
    #[repr(C)]
    struct OSVERSIONINFOEXW {
        dw_os_version_info_size: u32,
        dw_major_version: u32,
        dw_minor_version: u32,
        dw_build_number: u32,
        dw_platform_id: u32,
        sz_csd_version: [u16; 128],
        w_service_pack_major: u16,
        w_service_pack_minor: u16,
        w_suite_mask: u16,
        w_product_type: u8,
        w_reserved: u8,
    }
    type RtlGetVersionFn = unsafe extern "system" fn(*mut OSVERSIONINFOEXW) -> i32;
    unsafe {
        if let Ok(ntdll) = GetModuleHandleA(windows::core::s!("ntdll.dll")) {
            if let Some(proc) = GetProcAddress(ntdll, windows::core::s!("RtlGetVersion")) {
                let rtl_get_version: RtlGetVersionFn = std::mem::transmute(proc);
                let mut info = std::mem::zeroed::<OSVERSIONINFOEXW>();
                info.dw_os_version_info_size = std::mem::size_of::<OSVERSIONINFOEXW>() as u32;
                if rtl_get_version(&mut info) == 0 {
                    let name = if info.dw_build_number >= 22000 { "11" } else { "10" };
                    return format!("Windows {} (build {})", name, info.dw_build_number);
                }
            }
        }
    }
    "Windows".into()
}

#[cfg(not(windows))]
fn windows_os_name() -> String {
    "Windows (emulated)".into()
}

fn human_bitrate(bits_per_second: u64) -> String {
    if bits_per_second >= 1_000_000 {
        format!("{:.1} Mbps", bits_per_second as f64 / 1_000_000.0)
    } else {
        format!("{} kbps", bits_per_second / 1000)
    }
}


/// This machine's reachable IPv4 addresses.
///
/// Virtual adapters are excluded, because they are reachable *from this machine*
/// and a phone on the Wi-Fi can only reach the address on its own subnet. With
/// four addresses advertised, a phone picked a VMware or Tailscale address about
/// three times in four and failed with "couldn't find your PC".
pub fn local_ipv4_addresses() -> Vec<String> {
    #[cfg(windows)]
    let mut addresses = windows_addresses();

    #[cfg(not(windows))]
    let mut addresses: Vec<String> = Vec::new();

    if addresses.is_empty() {
        // A loopback fallback keeps the app usable on a machine whose adapters
        // could not be enumerated, and makes the failure visible rather than
        // silent.
        addresses.push("127.0.0.1".to_string());
    }
    addresses
}

#[cfg(windows)]
fn windows_addresses() -> Vec<String> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::NetworkManagement::IpHelper::GET_ADAPTERS_ADDRESSES_FLAGS;
    use windows::Win32::Networking::WinSock::{AF_INET, SOCKADDR_IN};

    /// Adapter descriptions that are never reachable from a phone.
    ///
    /// Matched on the description because `IfType` cannot distinguish a VMware
    /// host-only adapter from a real NIC.
    const BLOCKED: [&str; 12] = [
        "VMware",
        "VirtualBox",
        "Hyper-V",
        "Tailscale",
        "Loopback",
        "TAP-Windows",
        "OpenVPN",
        "WireGuard",
        "ZeroTier",
        "WSL",
        "Microsoft Wi-Fi Direct",
        "Bluetooth",
    ];

    let mut size: u32 = 16 * 1024;
    let mut buffer = vec![0u8; size as usize];
    let flags = GET_ADAPTERS_ADDRESSES_FLAGS(
        GAA_FLAG_SKIP_ANYCAST.0 | GAA_FLAG_SKIP_MULTICAST.0 | GAA_FLAG_SKIP_DNS_SERVER.0,
    );

    let result = unsafe {
        GetAdaptersAddresses(
            AF_INET.0 as u32,
            flags,
            None,
            Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
            &mut size,
        )
    };
    if result != 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut current = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;

    while !current.is_null() {
        let adapter = unsafe { &*current };

        let description = unsafe { wide_to_string(adapter.Description) };
        let blocked = BLOCKED.iter().any(|needle| description.contains(needle));

        if !blocked && adapter.OperStatus == windows::Win32::NetworkManagement::Ndis::IfOperStatusUp {
            let mut unicast = adapter.FirstUnicastAddress;
            while !unicast.is_null() {
                let entry = unsafe { &*unicast };
                let sockaddr = entry.Address.lpSockaddr;
                if !sockaddr.is_null() {
                    let family = unsafe { (*sockaddr).sa_family };
                    if family == AF_INET {
                        let sin = unsafe { &*(sockaddr as *const SOCKADDR_IN) };
                        let octets = unsafe { sin.sin_addr.S_un.S_un_b };
                        let text =
                            format!("{}.{}.{}.{}", octets.s_b1, octets.s_b2, octets.s_b3, octets.s_b4);

                        let excluded = text.starts_with("169.254.")
                            || text.starts_with("127.")
                            || is_cgnat(&text);
                        if !excluded && !out.contains(&text) {
                            out.push(text);
                        }
                    }
                }
                unicast = entry.Next;
            }
        }

        current = adapter.Next;
    }

    out
}

/// `100.64.0.0/10`, which Tailscale uses.
#[cfg(windows)]
fn is_cgnat(address: &str) -> bool {
    let mut parts = address.split('.');
    if parts.next() != Some("100") {
        return false;
    }
    match parts.next().and_then(|s| s.parse::<u32>().ok()) {
        Some(second) => (64..=127).contains(&second),
        None => false,
    }
}

#[cfg(windows)]
unsafe fn wide_to_string(ptr: windows::core::PWSTR) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while unsafe { *ptr.0.add(len) } != 0 {
        len += 1;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr.0, len) };
    String::from_utf16_lossy(slice)
}

fn hostname() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PhoneCam Desktop".into())
}

impl Runtime {
    /// Locates adb, once, and remembers why it failed if it did.
    ///
    /// Called at startup and again by `refresh_usb`, because a developer build can
    /// gain the platform tools while the app is open, and a copy can be removed
    /// or quarantined after installation.
    pub fn refresh_usb(&self) {
        self.adb_checked.store(true, Ordering::Relaxed);
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        match crate::adb::Adb::locate(exe_dir.as_deref()) {
            Ok(adb) => {
                tracing::info!("adb found at {}", adb.path().display());
                *self.adb.lock() = Some(adb);
                *self.adb_error.lock() = None;
                self.poll_usb();
            }
            Err(error) => {
                tracing::warn!("{error}");
                *self.adb.lock() = None;
                *self.adb_error.lock() = Some(error);
                self.sync_usb_state(Vec::new(), false);
            }
        }
    }

    /// Reads the cable state and publishes it.
    pub fn poll_usb(&self) {
        let devices = {
            let adb = self.adb.lock();
            match adb.as_ref() {
                Some(adb) => match adb.devices() {
                    Ok(devices) => devices,
                    Err(error) => {
                        *self.adb_error.lock() = Some(error);
                        Vec::new()
                    }
                },
                None => Vec::new(),
            }
        };

        let forwarding = {
            let adb = self.adb.lock();
            adb.as_ref().map(|adb| adb.is_reverse_active(SERVICE_PORT)).unwrap_or(false)
        };

        self.sync_usb_state(devices, forwarding);
    }

    fn sync_usb_state(&self, devices: Vec<crate::adb::AdbDevice>, forwarding: bool) {
        // `adb_available` is a claim that adb was **found and can be run**. Before
        // the first locate attempt nothing is known, and reporting `false` there
        // made the UI say "Cable tools missing" on a machine that had never been
        // checked. The error is the evidence: no handle and no error means "not
        // checked yet".
        let adb_available = self.adb.lock().is_some();
        let authorized = devices.iter().any(|d| d.is_ready());
        let error = self.adb_error.lock().clone();

        {
            let mut state = self.state.lock();
            if let Some(core) = &mut state.core {
                core.usb = phonecam_protocol::UsbStatus {
                    adb_available,
                    available: !devices.is_empty(),
                    authorized,
                    devices: devices
                        .iter()
                        .map(|d| phonecam_protocol::AdbDevice {
                            serial: d.serial.clone(),
                            model: d.model.clone(),
                            state: d.state.clone(),
                        })
                        .collect(),
                    forwarding,
                    error: error.clone(),
                };
            }

            // A phone on the cable that has not been approved needs saying,
            // because the fix is on the phone and nothing on this screen can do
            // it for the user.
            let notice = if !devices.is_empty() && !authorized {
                Some((FailureCode::UsbUnauthorized, "usb"))
            } else if self.adb_checked.load(Ordering::Relaxed) && error.is_some() && !adb_available {
                // Only a recorded failure counts. Absence of evidence is not
                // evidence that the install is broken.
                Some((FailureCode::UsbMissing, "usb"))
            } else {
                None
            };

            match notice {
                Some((code, key)) => {
                    let message = error.unwrap_or_default();
                    state.add_notice(code, vec![("message".into(), message)], key.to_string());
                }
                None => state.dismiss_notice("usb"),
            }
        }
    }

    /// Asks the registrar whether the camera is registered, and records it.
    ///
    /// Checked at startup rather than assumed, because an interrupted install, a
    /// moved DLL or a developer build beside an installed copy all leave a camera
    /// that is not registered while the UI would otherwise claim it is.
    pub fn check_vcam(&self) {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));

        let Some(registrar) = crate::vcam::Registrar::locate(exe_dir.as_deref()) else {
            self.vcam_registered.store(false, Ordering::Relaxed);
            self.log
                .write("WARN", "the virtual camera registrar was not found beside the app");
            return;
        };

        let state = registrar.status();
        let registered = state.is_registered();
        self.vcam_registered.store(registered, Ordering::Relaxed);

        match state {
            crate::vcam::RegistrationState::Registered => {
                self.log.write("INFO", "virtual camera: registered and enumerable");
            }
            crate::vcam::RegistrationState::NotRegistered { detail } => {
                // Say what the tool said rather than guessing at a cause.
                self.log
                    .write("WARN", &format!("virtual camera not registered: {detail}"));
            }
            crate::vcam::RegistrationState::Unavailable { detail } => {
                self.log
                    .write("WARN", &format!("virtual camera status unavailable: {detail}"));
            }
        }
    }

    /// Opens the tunnel and asks the phone to connect through it.
    pub fn connect_usb(&self) {
        let adb = self.adb.lock().clone();
        let Some(adb) = adb else {
            self.log_error(
                "usb_missing",
                "adb wasn't found. USB mode needs Android platform-tools installed.",
            );
            return;
        };

        if let Err(error) = adb.enable_reverse(SERVICE_PORT) {
            self.log_error("usb_forward_failed", &error);
            return;
        }
        self.log_info(&format!("cable tunnel open on {SERVICE_PORT}"));

        if let Err(error) = adb.start_phone_session(SERVICE_PORT) {
            // The tunnel is up but the phone did not start: say so rather than
            // claiming a connection that is not there.
            self.log_error("usb_session_failed", &error);
        }

        self.poll_usb();
        self.broadcast_state();
    }

    /// Closes the tunnel.
    pub fn disconnect_usb(&self) {
        let adb = self.adb.lock().clone();
        if let Some(adb) = adb {
            if let Err(error) = adb.disable_reverse(SERVICE_PORT) {
                self.log_error("usb_forward_failed", &error);
            } else {
                self.log_info("cable tunnel closed");
            }
        }
        self.poll_usb();
        self.broadcast_state();
    }
}

impl Runtime {
    /// Records how long one stage of the video loop took.
    fn note_stage(&self, index: usize, micros: i64) {
        let mut totals = self.stage_us.lock();
        totals[index] += micros;
        let mut maxima = self.stage_max_us.lock();
        if micros > maxima[index] {
            maxima[index] = micros;
        }
    }
}
/// Writes the registry to disk.
///
/// Secrets never reach a log, but they do have to survive a restart, or every
/// re-dial is answered "unknown device" and the user rescans a QR code for a
/// phone they already paired.
fn persist_registry(registry: &[DeviceRecord]) {
    let stored: Vec<crate::settings::StoredDevice> = registry
        .iter()
        .map(|record| crate::settings::StoredDevice {
            device_id: record.device_id.clone(),
            name: record.name.clone(),
            secret_hex: record.secret_hex.clone(),
            last_seen_ms: record.last_seen_ms,
            preferred_camera: record.preferred_camera.clone(),
            preferred_resolution: record.preferred_resolution.clone(),
            preferred_fps: record.preferred_fps,
            preferred_connection: record.preferred_connection.clone(),
        })
        .collect();
    if let Err(error) = crate::settings::save_devices(&stored) {
        tracing::warn!("could not save devices.json: {error}");
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_exclude_loopback_and_link_local() {
        for address in local_ipv4_addresses() {
            assert!(!address.starts_with("169.254."), "link-local leaked: {address}");
        }
    }

    #[test]
    fn the_cgnat_range_is_excluded() {
        // Only exercised on Windows, where the enumeration lives.
        #[cfg(windows)]
        {
            assert!(is_cgnat("100.64.0.1"));
            assert!(is_cgnat("100.127.255.254"));
            assert!(!is_cgnat("100.63.0.1"));
            assert!(!is_cgnat("100.128.0.1"));
            assert!(!is_cgnat("192.168.1.1"));
        }
    }

    #[test]
    fn usb_state_keeps_the_two_facts_separate() {
        use phonecam_ui::theme::rules::{usb_readiness, UsbReadiness};

        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();

        // Tools present, no cable: the normal Wi-Fi state. This must read
        // `NoDevice`, not `ToolsMissing`.
        runtime.sync_usb_state(Vec::new(), false);
        let state = runtime.state.lock().clone();
        let usb = &state.core.as_ref().unwrap().usb;
        assert!(!usb.available, "no cable, so no device");

        // Before any locate attempt, the honest answer is `Unknown`: absence of
        // evidence is not evidence that the install is broken.
        let unchecked = usb_readiness(None, usb.devices.len());
        assert_eq!(unchecked, UsbReadiness::Unknown);

        // After a successful locate with no cable, it is `NoDevice`.
        let checked = usb_readiness(Some(true), usb.devices.len());
        assert_eq!(checked, UsbReadiness::NoDevice);
    }

    #[test]
    fn an_unauthorised_phone_raises_the_approval_notice() {
        use crate::adb::AdbDevice;

        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();

        runtime.sync_usb_state(
            vec![AdbDevice {
                serial: "abc".into(),
                model: "Pixel".into(),
                state: "unauthorized".into(),
            }],
            false,
        );

        let state = runtime.state.lock().clone();
        assert!(
            state.has_notice(FailureCode::UsbUnauthorized),
            "an unapproved phone needs telling, because the fix is on the phone"
        );
        assert!(!state.core.as_ref().unwrap().usb.authorized);
    }

    #[test]
    fn a_ready_phone_clears_the_usb_notice() {
        use crate::adb::AdbDevice;

        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();

        runtime.sync_usb_state(
            vec![AdbDevice {
                serial: "abc".into(),
                model: "Pixel".into(),
                state: "device".into(),
            }],
            true,
        );

        let state = runtime.state.lock().clone();
        assert!(!state.has_notice(FailureCode::UsbUnauthorized));
        let usb = &state.core.as_ref().unwrap().usb;
        assert!(usb.authorized);
        assert!(usb.forwarding);
    }

    #[test]
    fn a_missing_adb_is_reported_as_missing_tools() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();

        // No adb handle, a recorded reason, and the locate having been attempted:
        // that is a broken install, which is the one case that earns this notice.
        *runtime.adb.lock() = None;
        *runtime.adb_error.lock() = Some("adb wasn't found.".into());
        runtime.adb_checked.store(true, Ordering::Relaxed);
        runtime.sync_usb_state(Vec::new(), false);

        let state = runtime.state.lock().clone();
        assert!(state.has_notice(FailureCode::UsbMissing));
    }

    #[test]
    fn the_tray_menu_tracks_the_state() {
        use phonecam_protocol::{DeviceIdentity, StreamPhase, TransportKind};

        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();

        let idle = crate::tray::build_menu(&runtime.state.lock());
        assert_eq!(idle.device, "No phone connected");

        {
            let mut state = runtime.state.lock();
            if let Some(core) = &mut state.core {
                core.transport = TransportKind::Lan;
                core.phase = StreamPhase::Streaming;
                core.device = Some(DeviceIdentity {
                    device_id: "android-a".into(),
                    name: "Redmi".into(),
                    model: "2201117TG".into(),
                    android_version: 33,
                });
            }
        }

        let live = crate::tray::build_menu(&runtime.state.lock());
        assert_eq!(live.device, "Redmi (Wi-Fi)");
        assert!(live.status.contains("Streaming"));
    }

    #[test]
    fn a_command_queue_is_shared_across_threads() {
        // The regression: a thread-local queue meant every command sat in the
        // wrong thread's list and nothing was ever sent.
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);

        let producer = runtime.clone();
        std::thread::spawn(move || {
            producer.queue_command(DesktopToPhone::StartStream);
        })
        .join()
        .unwrap();

        let queued = runtime.outbox.lock().len();
        assert_eq!(queued, 1, "a command queued on another thread must be visible here");
    }
    #[test]
    fn the_output_follows_the_stream_by_default() {
        let mut settings = phonecam_ui::views::state::Settings::default();
        settings.output_follows_stream = true;

        // The phone's own size, so the publish path is a straight copy.
        assert_eq!(output_size_for(&settings, 1280, 720), (1280, 720));
        assert_eq!(output_size_for(&settings, 1920, 1080), (1920, 1080));

        // An odd size cannot reach the buffer, because NV12 has no layout for it.
        assert_eq!(output_size_for(&settings, 1281, 721), (1280, 720));
    }

    #[test]
    fn a_pinned_output_size_is_honoured() {
        let mut settings = phonecam_ui::views::state::Settings::default();
        settings.output_follows_stream = false;
        settings.output_width = 1920;
        settings.output_height = 1080;

        // The user asked for 1080p, so the frame is scaled even though the phone
        // sends 720p. That is a choice, not an accident.
        assert_eq!(output_size_for(&settings, 1280, 720), (1920, 1080));
    }
    #[test]
    fn a_runtime_starts_idle_with_no_phone() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        let state = runtime.state.lock();
        assert!(!state.has_device());
        assert!(state.logs.iter().any(|l| l.message.contains("started")));
    }

    #[test]
    fn arming_a_pairing_token_produces_a_payload_per_address() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();
        let payloads = runtime.arm_pairing();
        assert!(!payloads.is_empty(), "at least one QR payload");
        for payload in &payloads {
            let value: serde_json::Value = serde_json::from_str(payload).unwrap();
            assert_eq!(value["protocol"], "phonecam");
            assert_eq!(value["port"], SERVICE_PORT);
            assert_eq!(
                value["token"].as_str().unwrap().len(),
                32,
                "a token is 32 hex chars"
            );
        }
        // And disarming clears them.
        runtime.disarm_pairing();
        let state = runtime.state.lock();
        assert!(!state.core.as_ref().unwrap().pairing.open);
        assert!(state.core.as_ref().unwrap().pairing.qr_payloads.is_empty());
    }

    #[test]
    fn a_denied_request_clears_the_prompt_and_the_pending_entry() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();
        runtime.set_pairing_prompt("android-a");
        assert!(runtime.state.lock().pairing_prompt.is_some());

        runtime.resolve_pair_request(false);
        assert!(runtime.state.lock().pairing_prompt.is_none());
        assert!(!runtime.pairing.lock().has_pending_request("android-a"));
    }

    #[test]
    fn an_approved_request_arms_a_token_for_that_device() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();
        runtime.set_pairing_prompt("android-b");
        runtime.pairing.lock().add_pending_request(
            phonecam_core::pairing::PendingRequest {
                device_id: "android-b".into(),
                name: "Android phone".into(),
                model: "Android".into(),
                requested_at_ms: 0,
            },
        );

        runtime.resolve_pair_request(true);
        let token = runtime
            .pairing
            .lock()
            .approved_token_for("android-b", now_ms());
        assert!(token.is_some(), "approval must arm a token the retry can redeem");
    }

    #[test]
    fn an_error_is_redacted_before_it_reaches_the_log() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();
        let secret = "a".repeat(64);
        runtime.log_error("boom", &format!("token {secret} leaked"));

        let state = runtime.state.lock();
        let line = state
            .logs
            .iter()
            .find(|l| l.level == LogLevel::Error)
            .expect("an error line");
        assert!(!line.message.contains(&secret), "the secret must not survive");
        assert!(line.message.contains("[redacted]"));
    }

    #[test]
    fn buffer_recycling_and_spare_pool() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);

        assert!(runtime.take_spare_buffer(1024).is_none());

        let mut buf = Vec::with_capacity(1024);
        buf.resize(1024, 42);
        runtime.return_buffer(buf);

        let taken = runtime.take_spare_buffer(512);
        assert!(taken.is_some());
        let taken = taken.unwrap();
        assert!(taken.capacity() >= 1024);
        assert_eq!(taken.len(), 0);
    }

    #[test]
    fn state_version_bumps_on_mutations() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);

        let v1 = runtime.state_version.load(Ordering::SeqCst);
        runtime.bump_state_version();
        let v2 = runtime.state_version.load(Ordering::SeqCst);
        assert!(v2 > v1);

        runtime.broadcast_state();
        let v3 = runtime.state_version.load(Ordering::SeqCst);
        assert!(v3 > v2);

        runtime.set_notice(FailureCode::Generic, vec![], "test-notice");
        let v4 = runtime.state_version.load(Ordering::SeqCst);
        assert!(v4 > v3);
    }

    #[test]
    fn diagnostics_report_generation_and_export() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.broadcast_state();

        let report = runtime.generate_diagnostics_report();
        assert!(report.contains("PhoneCam Diagnostics"));
        assert!(report.contains("Desktop:"));
        assert!(report.contains("Device:"));
        assert!(report.contains("Connection:"));
        assert!(report.contains("Video:"));
        assert!(report.contains("Decoder:"));
        assert!(report.contains("OpenH264"));
        assert!(report.contains("Virtual Camera:"));
        assert!(report.contains("Latency:"));
        assert!(report.contains("Dropped Frames:"));

        let temp_dir = std::env::temp_dir().join(format!("phonecam-test-diag-{}", std::process::id()));
        let temp_file = temp_dir.join("test_diagnostics.txt");
        let result = runtime.export_diagnostics(Some(&temp_file));
        assert!(result.is_ok());
        let written = std::fs::read_to_string(&temp_file).expect("diagnostics file should exist");
        assert_eq!(written, report);
        let _ = std::fs::remove_dir_all(temp_dir);
    }
}




































