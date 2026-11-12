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

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use phonecam_core::adaptive_quality::{AdaptiveQuality, QualityRung, QualitySample};
use phonecam_core::pairing::{new_server_id, DeviceRecord, PairingManager};
use phonecam_core::session::{
    control_frame, now_ms, now_us, phase_after_phone_state, Authenticator, SessionState,
    StatsWindow,
};
use phonecam_core::transport::{Connection, DiscoverySocket, Listener, ReadResult};
use phonecam_framebus::{FrameState, FrameWriter};
use phonecam_media::{H264Decoder, YuvPlanes};
use phonecam_protocol::{
    CapabilitiesPayload, CoreError, CoreState, DesktopBeacon, DesktopToPhone, FpsValue, LogLevel,
    NearbyPhone, PhoneToDesktop, QrPayload, QualityMode, ScreenMode, StreamPhase, TransportKind,
    VcamState, VcamStatus, VideoFlags, BEACON_INTERVAL_MS, DISCOVERY_PORT, MAX_VIDEO_PAYLOAD,
    SERVICE_PORT, VIDEO_HEADER_SIZE,
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
    /// I420/NV12 at the decoded size, for the scale path.
    source: Vec<u8>,
    /// The BGRA buffer being filled.
    bgra: Vec<u8>,
    /// Reusable BGRA staging buffer when rotation or mirror transforms are active.
    transformed: Vec<u8>,
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
    /// Whether the desktop preview surface is enabled (kept in sync with settings).
    pub preview_enabled: Arc<AtomicBool>,
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
    /// Last virtual camera registration/unregistration error, if any.
    pub vcam_error: Mutex<Option<String>>,
    /// True while a background virtual camera registration operation is running.
    pub vcam_starting: AtomicBool,
    /// Last known USB / ADB state so `broadcast_state()` never clobbers `core.usb`.
    pub usb_status: Mutex<phonecam_protocol::UsbStatus>,
    /// Serializes background registrar runs so rapid toggles cannot race.
    vcam_op_lock: Mutex<()>,
    /// Generation counter to discard stale virtual camera operations.
    vcam_op_gen: AtomicU64,
    /// Weak self-handle so `&self` methods can spawn background tasks.
    self_weak: Weak<Self>,
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
    /// Total bytes received across authenticated control and video sockets.
    pub bytes_in: Arc<AtomicU64>,
    /// Total authenticated handshakes completed in this runtime lifetime.
    pub connects: Arc<AtomicU32>,
    /// Active or user-selected ADB serial for USB reverse forwarding.
    pub active_usb_serial: Mutex<Option<String>>,
    /// Current digital zoom value stored as `f32::to_bits`.
    zoom_bits: AtomicU32,
    /// Atomic mirror of `settings.output_mirror` for lock-free per-frame reads.
    output_mirror: AtomicBool,
    /// Atomic mirror of `settings.output_rotation % 360` for lock-free per-frame reads.
    output_rotation: AtomicU32,
    /// Atomic mirror of `settings.output_width` for lock-free writer init.
    output_width: AtomicU32,
    /// Atomic mirror of `settings.output_height` for lock-free writer init.
    output_height: AtomicU32,
    /// Atomic mirror of `settings.output_follows_stream` for lock-free writer init.
    output_follows_stream: AtomicBool,
    /// Whether the desktop window is visible (false when minimized or hidden in tray).
    window_visible: AtomicBool,
    /// Whether `state.toast_expires_at_ms` is currently `Some(_)`.
    has_active_toast: AtomicBool,
    /// Set when `bump_state_version()` could not inspect `state` synchronously.
    toast_check_needed: AtomicBool,
    /// Lock-free per-stage accumulator synced to `stage_us` every second & on exit.
    stage_totals_atomic: [AtomicI64; 3],
    /// Lock-free per-stage max synced to `stage_max_us` every second & on exit.
    stage_max_atomic: [AtomicI64; 3],
    /// Cached count of recorded `frame_intervals_us` samples (caps locking after 4096).
    frame_interval_count: AtomicUsize,
    /// Whether a stepped camera-start retry is currently active.
    camera_retry_active: AtomicBool,
    /// Last `(requested, actual)` pair emitted as `ResolutionDowngraded`.
    last_downgraded: Mutex<Option<(String, String)>>,
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

        let mut quality = AdaptiveQuality::new();
        quality.set_enabled(settings.quality_mode == QualityMode::Auto);

        Arc::new_cyclic(|weak| Self {
            session: Arc::new(SessionState::default()),
            pairing: Arc::new(Mutex::new(PairingManager::new())),
            registry: Arc::new(Mutex::new(registry)),
            state: Arc::new(Mutex::new(state)),
            frames: Arc::new(Mutex::new(None)),
            want_stream: Arc::new(AtomicBool::new(false)),
            vcam_enabled: Arc::new(AtomicBool::new(settings.vcam_enabled)),
            preview_enabled: Arc::new(AtomicBool::new(settings.preview_enabled)),
            quality: Arc::new(Mutex::new(quality)),
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
            vcam_error: Mutex::new(None),
            vcam_starting: AtomicBool::new(false),
            usb_status: Mutex::new(phonecam_protocol::UsbStatus::default()),
            vcam_op_lock: Mutex::new(()),
            vcam_op_gen: AtomicU64::new(0),
            self_weak: weak.clone(),
            camera_retry: Arc::new(Mutex::new(phonecam_core::CameraRetryTracker::new())),
            camera_retry_generation: Arc::new(AtomicU64::new(0)),
            spare_buffers: Arc::new(Mutex::new(Vec::new())),
            state_version: Arc::new(AtomicU64::new(1)),
            bytes_in: Arc::new(AtomicU64::new(0)),
            connects: Arc::new(AtomicU32::new(0)),
            active_usb_serial: Mutex::new(None),
            zoom_bits: AtomicU32::new(1.0f32.to_bits()),
            output_mirror: AtomicBool::new(settings.output_mirror),
            output_rotation: AtomicU32::new(settings.output_rotation % 360),
            output_width: AtomicU32::new(settings.output_width),
            output_height: AtomicU32::new(settings.output_height),
            output_follows_stream: AtomicBool::new(settings.output_follows_stream),
            window_visible: AtomicBool::new(true),
            has_active_toast: AtomicBool::new(false),
            toast_check_needed: AtomicBool::new(false),
            stage_totals_atomic: [AtomicI64::new(0), AtomicI64::new(0), AtomicI64::new(0)],
            stage_max_atomic: [AtomicI64::new(0), AtomicI64::new(0), AtomicI64::new(0)],
            frame_interval_count: AtomicUsize::new(0),
            camera_retry_active: AtomicBool::new(false),
            last_downgraded: Mutex::new(None),
        })
    }

    /// Updates `window_visible` so preview BGRA conversion & mailbox publishing
    /// are skipped while the desktop window is minimized or hidden in the tray.
    pub fn set_window_visible(&self, visible: bool) {
        let prev = self.window_visible.swap(visible, Ordering::Relaxed);
        if !visible && prev {
            *self.frames.lock() = None;
        }
    }

    /// Returns whether an active UI toast timer needs expiration polling.
    pub fn has_active_toast(&self) -> bool {
        if self.toast_check_needed.swap(false, Ordering::Relaxed) {
            if let Some(state) = self.state.try_lock() {
                self.sync_cached_settings_from_state(&state);
            } else {
                self.toast_check_needed.store(true, Ordering::Relaxed);
            }
        }
        self.has_active_toast.load(Ordering::Relaxed)
    }

    fn sync_cached_settings_from_state(&self, state: &AppState) {
        self.output_mirror
            .store(state.settings.output_mirror, Ordering::Relaxed);
        self.output_rotation
            .store(state.settings.output_rotation % 360, Ordering::Relaxed);
        self.output_width
            .store(state.settings.output_width, Ordering::Relaxed);
        self.output_height
            .store(state.settings.output_height, Ordering::Relaxed);
        self.output_follows_stream
            .store(state.settings.output_follows_stream, Ordering::Relaxed);
        self.has_active_toast
            .store(state.toast_expires_at_ms.is_some(), Ordering::Relaxed);
    }

    /// Updates `preview_enabled` atomically and clears any queued preview frame when turned off.
    pub fn set_preview_enabled(&self, enabled: bool) {
        self.preview_enabled.store(enabled, Ordering::Relaxed);
        {
            let mut state = self.state.lock();
            state.settings.preview_enabled = enabled;
        }
        if !enabled {
            *self.frames.lock() = None;
        }
        self.bump_state_version();
    }

    /// Synchronizes mirror/rotation onto settings, the active `FrameWriter`, and the phone.
    pub fn sync_output_transform(&self, mirror: bool, rotation: u32) {
        let normalized_rot = rotation % 360;
        self.output_mirror.store(mirror, Ordering::Relaxed);
        self.output_rotation.store(normalized_rot, Ordering::Relaxed);
        {
            let mut state = self.state.lock();
            state.settings.output_mirror = mirror;
            state.settings.output_rotation = normalized_rot;
            let _ = crate::settings::save(&state.settings);
        }
        if let Some(writer) = self.writer.lock().as_mut() {
            writer.set_transform(mirror, normalized_rot);
        }
        let zoom = f32::from_bits(self.zoom_bits.load(Ordering::Relaxed));
        self.send_to_phone(DesktopToPhone::SetImage {
            mirror,
            rotation: normalized_rot,
            zoom,
        });
        self.broadcast_state();
    }

    /// Increments the state mutation version counter.
    pub fn bump_state_version(&self) {
        if let Some(state) = self.state.try_lock() {
            self.sync_cached_settings_from_state(&state);
            self.toast_check_needed.store(false, Ordering::Relaxed);
        } else {
            self.toast_check_needed.store(true, Ordering::Relaxed);
        }
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

    /// Generates the comprehensive diagnostics report covering desktop, listeners,
    /// device/cameras, video/output settings, framebus metrics, ADB/USB, VCam, and log tail.
    pub fn generate_diagnostics_report(&self) -> String {
        self.sync_stage_stats();
        let state = self.state.lock().clone();
        let port = self.port.load(Ordering::Relaxed);
        let addresses = local_ipv4_addresses();
        let stage_us = *self.stage_us.lock();
        let stage_max_us = *self.stage_max_us.lock();
        let skipped = self.skipped_frames.load(Ordering::Relaxed);
        let bytes_in = self.bytes_in.load(Ordering::Relaxed);
        let connects = self.connects.load(Ordering::Relaxed);
        let usb_active = self.session.transport.lock().as_str() == "usb";
        let active_serial = self.active_usb_serial.lock().clone();
        let adb_path = self
            .adb
            .lock()
            .as_ref()
            .map(|a| a.path().display().to_string());
        let adb_error = self.adb_error.lock().clone();
        let bus_metrics = self.writer.lock().as_ref().map(|w| {
            (
                w.width(),
                w.height(),
                w.reader_count(),
                w.frame_index(),
                w.last_mean_luma(),
                w.is_flicker_reported(),
            )
        });

        let mut report = String::new();
        report.push_str("PhoneCam Diagnostics\n\n");

        report.push_str("Desktop:\n");
        report.push_str(&format!(
            "  {} ({})\n",
            windows_os_name(),
            std::env::consts::ARCH
        ));
        report.push_str(&format!("  App Version: {}\n", env!("CARGO_PKG_VERSION")));
        report.push_str(&format!(
            "  Log level: {:?}\n\n",
            state.settings.log_level
        ));

        report.push_str("Service Listeners:\n");
        report.push_str(&format!("  Control/Video: 0.0.0.0:{port}\n"));
        report.push_str(&format!("  Discovery: UDP 0.0.0.0:{DISCOVERY_PORT}\n"));
        if addresses.is_empty() {
            report.push_str("  Local IPv4: (none)\n\n");
        } else {
            report.push_str(&format!("  Local IPv4: {}\n\n", addresses.join(", ")));
        }

        report.push_str("Device:\n");
        let device = state.core.as_ref().and_then(|c| c.device.as_ref());
        let device_name = device.map(|d| d.name.as_str()).unwrap_or("");
        report.push_str(&format!(
            "  {}\n",
            if device_name.is_empty() {
                "(none)"
            } else {
                device_name
            }
        ));
        if let Some(dev) = device {
            if !dev.model.is_empty() {
                report.push_str(&format!("  Model: {}\n", dev.model));
            }
            if !dev.device_id.is_empty() {
                report.push_str(&format!("  ID: {}\n", dev.device_id));
            }
            if dev.android_version > 0 {
                report.push_str(&format!("  Android API {}\n", dev.android_version));
            }
        }
        if let Some(caps) = &state.capabilities {
            report.push_str(&format!(
                "  Phone app version: {}\n",
                if caps.device.app_version.is_empty() {
                    "unknown"
                } else {
                    &caps.device.app_version
                }
            ));
            report.push_str(&format!("  Cameras ({}):\n", caps.cameras.len()));
            for cam in &caps.cameras {
                let res_preview = if cam.resolutions.is_empty() {
                    "none".to_string()
                } else {
                    cam.resolutions
                        .iter()
                        .take(5)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let fps_preview = cam
                    .fps
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join("/");
                report.push_str(&format!(
                    "    - {} ({}) res=[{}] fps=[{}] torch={} focus={} zoom={} ({:.1}..{:.1}x) orient={}deg\n",
                    cam.id,
                    cam.facing.as_str(),
                    res_preview,
                    fps_preview,
                    cam.torch,
                    cam.focus,
                    cam.zoom,
                    cam.min_zoom,
                    cam.max_zoom,
                    cam.orientation_degrees
                ));
            }
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
        report.push_str(&format!(
            "  USB tunnel: {}\n",
            if usb_active { "active" } else { "inactive" }
        ));
        report.push_str(&format!(
            "  Bytes received: {} | Handshakes: {}\n\n",
            bytes_in, connects
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
            report.push_str(&format!(
                "  Phone camera={} torch={} zoom={:.2}x battery={}%{}\n",
                phone.camera,
                phone.torch,
                phone.zoom,
                phone.battery,
                if phone.charging { " (charging)" } else { "" }
            ));
        }
        let is_streaming = state
            .core
            .as_ref()
            .map(|c| c.phase == StreamPhase::Streaming)
            .unwrap_or(false);
        let stats_opt = state.core.as_ref().map(|c| &c.stats);
        let bitrate = stats_opt.map(|s| s.bitrate).unwrap_or(0);
        if is_streaming && bitrate > 0 {
            report.push_str(&format!("  {}\n", human_bitrate(bitrate)));
        } else {
            report.push_str("  (none)\n");
        }
        let configured_fps = if state.settings.fps == 0 {
            "Auto".to_string()
        } else {
            format!("{} FPS", state.settings.fps)
        };
        report.push_str(&format!(
            "  Target: camera={} res={} fps={} bitrate={} quality={:?}\n",
            state.settings.camera,
            state.settings.resolution,
            configured_fps,
            human_bitrate(state.settings.bitrate),
            state.settings.quality_mode
        ));
        let out_size = if state.settings.output_follows_stream {
            "follow stream".to_string()
        } else {
            format!("{}x{}", state.settings.output_width, state.settings.output_height)
        };
        report.push_str(&format!(
            "  Output: {} mirror={} rotation={}deg preview={}\n\n",
            out_size,
            state.settings.output_mirror,
            state.settings.output_rotation,
            state.settings.preview_enabled
        ));

        report.push_str("Decoder:\n");
        report.push_str("  OpenH264\n");
        if let Some((w, h)) = state.frame_size {
            report.push_str(&format!("  Decoded frame size: {}x{}\n", w, h));
        }
        if let Some(s) = stats_opt {
            report.push_str(&format!(
                "  Throughput: {:.1} FPS | decoded={} | quality={} | queue={}\n",
                s.fps, s.decoded_frames, s.quality, s.decode_queue_depth
            ));
        }
        report.push_str(&format!(
            "  Stage totals (us) [read, decode, publish]: {:?} (max: {:?})\n\n",
            stage_us, stage_max_us
        ));

        report.push_str("Frame Bus:\n");
        if let Some((w, h, readers, index, luma, flicker)) = bus_metrics {
            let luma_str = luma
                .map(|v| v.to_string())
                .unwrap_or_else(|| "none".to_string());
            report.push_str(&format!(
                "  {}x{} | consumers={} | frame_index={} | mean_luma={} | flicker={} | skipped={}\n\n",
                w, h, readers, index, luma_str, flicker, skipped
            ));
        } else {
            report.push_str(&format!("  (not open) | skipped={}\n\n", skipped));
        }

        report.push_str("ADB / USB:\n");
        report.push_str(&format!(
            "  ADB path: {}\n",
            adb_path.as_deref().unwrap_or("(not found)")
        ));
        if let Some(err) = adb_error {
            report.push_str(&format!("  ADB error: {}\n", err));
        }
        if let Some(usb) = state.core.as_ref().map(|c| &c.usb) {
            report.push_str(&format!(
                "  Available={} | Authorized={} | Reverse forwarding={} | Selected={}\n",
                usb.adb_available,
                usb.authorized,
                usb.forwarding,
                active_serial.as_deref().unwrap_or("auto")
            ));
            if usb.devices.is_empty() {
                report.push_str("  Attached devices: (none)\n");
            } else {
                for dev in &usb.devices {
                    report.push_str(&format!(
                        "    - {} ({}) [{}]\n",
                        dev.model, dev.serial, dev.state
                    ));
                }
            }
        }
        report.push('\n');

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
        let consumers = bus_metrics.map(|m| m.2).unwrap_or(0);
        report.push_str(&format!("  {} (consumers: {})\n\n", vcam_state, consumers));

        report.push_str("Latency:\n");
        let latency = stats_opt.map(|s| s.latency_ms).unwrap_or(-1.0);
        if is_streaming && latency >= 0.0 {
            report.push_str(&format!("  {} ms\n\n", latency as i32));
        } else {
            report.push_str("  (none)\n\n");
        }

        report.push_str("Dropped Frames:\n");
        if is_streaming {
            let dropped_pct = stats_opt.map(|s| s.dropped_percent).unwrap_or(0.0);
            report.push_str(&format!("  {:.1}%\n\n", dropped_pct));
        } else {
            report.push_str("  (none)\n\n");
        }

        if let Some(err) = state.core.as_ref().and_then(|c| c.error.as_ref()) {
            report.push_str(&format!("Last Error:\n  {}: {}\n\n", err.code, err.message));
        }

        let log_start = state.logs.len().saturating_sub(200);
        let tail = &state.logs[log_start..];
        report.push_str(&format!("Logs (last {}):\n", tail.len()));
        for entry in tail {
            report.push_str(&format!(
                "  {} {:?} [{}] {}\n",
                phonecam_ui::views::state::format_log_time(entry.ts_ms),
                entry.level,
                entry.source.label(),
                entry.message
            ));
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
            self.set_pairing_prompt_with_name(&outcome.device_id, Some(&outcome.device_name));
            self.wait_for_approval(&conn, &outcome.device_id);
            conn.close();
            return;
        }

        // A phone that authenticates owns the session.
        *self.session.device_id.lock() = outcome.device_id.clone();
        *self.session.device_name.lock() = outcome.device_name.clone();
        *self.session.transport.lock() = if conn.is_loopback() { "usb" } else { "lan" }.into();
        self.connects.fetch_add(1, Ordering::Relaxed);
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
            self.bytes_in
                .fetch_add(4 + payload.len() as u64, Ordering::Relaxed);
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
                    self.reconcile_resolution_downgrade(&phone_state);
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
        let mut packet_buf = Vec::with_capacity(256 * 1024);

        loop {
            if !self.running.load(Ordering::Relaxed) {
                break;
            }
            // A video channel that goes quiet is a stalled stream, but a read
            // timeout is not proof the socket closed.
            let read_started = now_us();
            let Some((flags, pts, total_len)) = read_video_packet_into(&conn, &mut packet_buf) else {
                if conn.is_closed() {
                    break;
                }
                continue;
            };
            let payload = &packet_buf[VIDEO_HEADER_SIZE..total_len];
            self.bytes_in.fetch_add(4 + total_len as u64, Ordering::Relaxed);

            let read_us = now_us() - read_started;
            self.note_stage(0, read_us);
            stats_window.note_frame(payload.len());

            if flags.is_codec_config() {
                if !decoder.set_parameter_sets(payload) {
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
            let pts_us = pts.max(0) as u64;
            let decoded_result = decoder.decode_and_process(payload, pts_us, |planes, p_pts| {
                self.note_stage(1, now_us() - decode_started);
                let publish_started = now_us();
                self.publish_planes(planes, p_pts, &mut scratch);
                self.note_stage(2, now_us() - publish_started);
            });

            match decoded_result {
                Ok(()) => {
                    decoded += 1;
                    self.last_frame_at_ms.store(now_ms(), Ordering::Relaxed);
                    if decoded == 1 || !self.session.live_frame_seen.load(Ordering::Relaxed) {
                        self.session.note_decoded_frame();
                    }
                    if self.camera_retry_active.swap(false, Ordering::Relaxed) {
                        self.camera_retry.lock().reset();
                        self.camera_retry_generation.fetch_add(1, Ordering::SeqCst);
                    }
                }
                Err(_) => {
                    self.note_stage(1, now_us() - decode_started);
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
                self.sync_stage_stats();
            }
        }

        self.sync_stage_stats();
        // The picture stops but the camera keeps the last frame until the
        // placeholder grace period elapses in the stats thread.
        self.session.live_frame_seen.store(false, Ordering::Relaxed);
        self.broadcast_state();
        tracing::info!("video channel closed");
    }

    #[cfg(test)]
    fn publish(&self, frame: phonecam_media::DecodedFrame, scratch: &mut PublishScratch) {
        if let Some(planes) = frame.as_planes() {
            self.publish_planes(planes, frame.pts_us, scratch);
        }
    }

    /// Publishes one decoded planar YUV frame to the camera bus and to the window.
    ///
    /// Writes directly into the shared-memory frame bus under the seqlock when
    /// no scaling is needed (zero intermediate NV12 staging copy) and skips
    /// BGRA conversion when the preview is disabled or the window is hidden.
    fn publish_planes(&self, planes: YuvPlanes<'_>, pts_us: u64, scratch: &mut PublishScratch) {
        // Timestamped at the producer, so the interval is the real one.
        let now = now_us();
        let previous = self.last_publish_us.swap(now, Ordering::Relaxed);
        if previous != 0 && self.frame_interval_count.load(Ordering::Relaxed) < 4096 {
            let mut intervals = self.frame_intervals_us.lock();
            // Bounded: a long session must not grow without limit.
            if intervals.len() < 4096 {
                intervals.push(now - previous);
            }
            self.frame_interval_count.store(intervals.len(), Ordering::Relaxed);
        }
        // --- the virtual camera: NV12 into the shared buffer
        let mirror = self.output_mirror.load(Ordering::Relaxed);
        let rotation = self.output_rotation.load(Ordering::Relaxed) % 360;
        if self.vcam_enabled.load(Ordering::Relaxed) {
            let mut writer = self.writer.lock();
            if writer.is_none() {
                let (width, height) = if self.output_follows_stream.load(Ordering::Relaxed) {
                    (planes.width & !1, planes.height & !1)
                } else {
                    (
                        self.output_width.load(Ordering::Relaxed) & !1,
                        self.output_height.load(Ordering::Relaxed) & !1,
                    )
                };
                match FrameWriter::open(width, height) {
                    Ok(mut w) => {
                        w.set_transform(mirror, rotation);
                        tracing::info!("frame bus opened at {}x{}", w.width(), w.height());
                        *writer = Some(w);
                    }
                    Err(error) => {
                        tracing::warn!("could not open the frame bus: {error}");
                    }
                }
            }

            if let Some(writer) = writer.as_mut() {
                let out_width = writer.width();
                let out_height = writer.height();

                if planes.width == out_width && planes.height == out_height {
                    // Same size: interleave straight into the shared-memory publish buffer.
                    let _ = writer.publish_with(pts_us, FrameState::Live, None, |dst| {
                        phonecam_media::write_planes_nv12(&planes, dst)
                    });
                } else {
                    // Different size: interleave once to scratch.source, then scale directly into the frame bus.
                    let source_len = planes.width as usize * planes.height as usize * 3 / 2;
                    scratch.source.resize(source_len, 0);
                    if phonecam_media::write_planes_nv12(&planes, &mut scratch.source) {
                        let _ = writer.publish_with(pts_us, FrameState::Live, None, |dst| {
                            phonecam_framebus::nv12::scale(
                                &scratch.source,
                                planes.width,
                                planes.height,
                                dst,
                                out_width,
                                out_height,
                            );
                            true
                        });
                    }
                }
            }
        }

        if !self.preview_enabled.load(Ordering::Relaxed)
            || !self.window_visible.load(Ordering::Relaxed)
        {
            return;
        }

        // --- the window: BGRA, double-buffered so the mailbox never holds a
        // buffer this thread is still writing into.
        let needed = planes.width as usize * planes.height as usize * 4;
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
        if !phonecam_media::write_planes_bgra(&planes, &mut scratch.bgra, planes.width as usize) {
            return;
        }

        let needs_transform = mirror || rotation != 0;
        let (pub_width, pub_height) = if needs_transform {
            if scratch.transformed.capacity() < needed {
                if scratch.spare.capacity() >= needed {
                    std::mem::swap(&mut scratch.transformed, &mut scratch.spare);
                } else if let Some(buf) = self.take_spare_buffer(needed) {
                    scratch.transformed = buf;
                } else {
                    scratch.transformed = Vec::with_capacity(needed);
                }
            }
            let dims = phonecam_media::transform_bgra_bytes(
                &scratch.bgra,
                &mut scratch.transformed,
                planes.width,
                planes.height,
                mirror,
                rotation,
            );
            std::mem::swap(&mut scratch.transformed, &mut scratch.spare);
            dims
        } else {
            // Hand the filled buffer to the mailbox and take the previous one back,
            // so at most two BGRA buffers exist for the whole stream.
            std::mem::swap(&mut scratch.bgra, &mut scratch.spare);
            (planes.width, planes.height)
        };

        let previous = self.frames.lock().replace(Frame {
            bgra: std::mem::take(&mut scratch.spare),
            width: pub_width,
            height: pub_height,
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
        if scratch.last_size != Some((pub_width, pub_height)) {
            scratch.last_size = Some((pub_width, pub_height));
            self.state.lock().frame_size = Some((pub_width, pub_height));
            self.spare_buffers.lock().clear();
            self.bump_state_version();
        }
    }

    /// The one-second tick: stats, placeholder, ladder, discovery beacon.
    fn spawn_stats(self: &Arc<Self>) {
        let runtime = self.clone();
        std::thread::spawn(move || {
            let mut last_beacon = 0i64;
            let mut tick_count: u64 = 0;
            while runtime.running.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(1000));
                tick_count = tick_count.wrapping_add(1);
                let now = now_ms();

                // --- placeholder after a grace period with no frames
                let last_frame = runtime.last_frame_at_ms.load(Ordering::Relaxed);
                if runtime.vcam_enabled.load(Ordering::Relaxed)
                    && last_frame > 0
                    && now - last_frame > 3000
                {
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

                // --- live USB polling every 2 ticks when adb is located
                if tick_count % 2 == 0 && runtime.adb.lock().is_some() {
                    runtime.poll_usb();
                }

                if runtime.state.lock().expire_toast_if_due(now) {
                    runtime.has_active_toast.store(false, Ordering::Relaxed);
                    runtime.bump_state_version();
                }

                runtime.sync_stage_stats();
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

        let max_rung = {
            let state = self.state.lock();
            effective_max_rung(&state.settings, state.capabilities.as_ref())
        };

        let current = self
            .session
            .phone_state
            .lock()
            .as_ref()
            .and_then(|s| QualityRung::for_stream(&s.actual_resolution, s.actual_fps))
            .unwrap_or_else(|| quality.current());

        let sample = QualitySample {
            current,
            max_rung,
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

        let fps_value = if settings.fps == 0 {
            FpsValue::auto()
        } else {
            FpsValue::fixed(settings.fps)
        };
        let zoom = f32::from_bits(self.zoom_bits.load(Ordering::Relaxed));

        self.queue_command(DesktopToPhone::SetCamera { camera: settings.camera.clone() });
        self.queue_command(DesktopToPhone::SetVideo {
            resolution: settings.resolution.clone(),
            fps: fps_value,
            bitrate: settings.bitrate,
        });
        self.queue_command(DesktopToPhone::SetImage {
            mirror: settings.output_mirror,
            rotation: settings.output_rotation,
            zoom,
        });
        self.queue_command(DesktopToPhone::SetScreen { mode: settings.screen_mode });
        self.queue_command(DesktopToPhone::StartStream);
    }

    /// Selects a phone camera immediately, persisting settings and sending `set_camera`.
    pub fn set_camera(&self, id: String) {
        {
            let mut state = self.state.lock();
            state.settings.camera = id.clone();
            let _ = crate::settings::save(&state.settings);
        }
        self.send_to_phone(DesktopToPhone::SetCamera { camera: id });
        self.broadcast_state();
    }

    /// Updates stream video parameters (`fps: None` or `Some(0)` maps to `"auto"`),
    /// persists settings, and immediately queues `set_video`.
    pub fn set_video(&self, resolution: String, fps: Option<u32>, bitrate: u64) {
        let clamped_fps = fps.filter(|&f| f > 0).unwrap_or(0);
        let fps_value = if clamped_fps == 0 {
            FpsValue::auto()
        } else {
            FpsValue::fixed(clamped_fps)
        };
        {
            let mut state = self.state.lock();
            state.settings.resolution = resolution.clone();
            state.settings.fps = clamped_fps;
            state.settings.bitrate = bitrate;
            let _ = crate::settings::save(&state.settings);
        }
        self.send_to_phone(DesktopToPhone::SetVideo {
            resolution,
            fps: fps_value,
            bitrate,
        });
        self.broadcast_state();
    }

    /// Toggles the phone torch immediately.
    pub fn set_torch(&self, enabled: bool) {
        if let Some(phone) = self.session.phone_state.lock().as_mut() {
            phone.torch = enabled;
        }
        self.send_to_phone(DesktopToPhone::SetTorch { enabled });
        self.broadcast_state();
    }

    /// Updates digital zoom immediately via `set_image`.
    pub fn set_zoom(&self, zoom: f32) {
        let clamped = zoom.max(1.0);
        self.zoom_bits.store(clamped.to_bits(), Ordering::Relaxed);
        let (mirror, rotation) = {
            let state = self.state.lock();
            (
                state.settings.output_mirror,
                state.settings.output_rotation % 360,
            )
        };
        if let Some(phone) = self.session.phone_state.lock().as_mut() {
            phone.zoom = clamped;
        }
        self.send_to_phone(DesktopToPhone::SetImage {
            mirror,
            rotation,
            zoom: clamped,
        });
        self.broadcast_state();
    }

    /// Updates the phone's screen mode immediately and persists settings.
    pub fn set_screen_mode(&self, mode: ScreenMode) {
        {
            let mut state = self.state.lock();
            state.settings.screen_mode = mode;
            let _ = crate::settings::save(&state.settings);
        }
        self.send_to_phone(DesktopToPhone::SetScreen { mode });
        self.broadcast_state();
    }

    /// Switches adaptive quality mode (`auto = true` -> `Auto`, `false` -> `Manual`),
    /// persists settings, and notifies the phone immediately.
    pub fn set_quality_mode(&self, auto: bool) {
        let mode = if auto {
            QualityMode::Auto
        } else {
            QualityMode::Manual
        };
        self.quality.lock().set_enabled(auto);
        {
            let mut state = self.state.lock();
            state.settings.quality_mode = mode;
            let _ = crate::settings::save(&state.settings);
        }
        self.send_to_phone(DesktopToPhone::SetQualityMode { mode });
        self.broadcast_state();
    }

    /// Updates runtime log verbosity, reloads the tracing filter, and persists settings.
    pub fn set_log_level(&self, level: LogLevel) {
        {
            let mut state = self.state.lock();
            state.settings.log_level = level;
            let _ = crate::settings::save(&state.settings);
        }
        crate::logging::set_level(level);
        self.broadcast_state();
    }

    /// Asks the phone to stop, leaving the virtual camera running.
    pub fn stop_stream(&self) {
        self.want_stream.store(false, Ordering::Relaxed);
        self.session.note_stop_stream();
        self.camera_retry_active.store(false, Ordering::Relaxed);
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
        self.camera_retry_active.store(true, Ordering::Relaxed);
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
        self.set_pairing_prompt_with_name(device_id, None);
    }

    fn set_pairing_prompt_with_name(&self, device_id: &str, advertised_name: Option<&str>) {
        let registry_name = self
            .registry
            .lock()
            .iter()
            .find(|r| r.device_id == device_id && !r.name.is_empty())
            .map(|r| r.name.clone());

        let mut state = self.state.lock();
        let nearby = state
            .core
            .as_ref()
            .and_then(|core| core.nearby_phones.iter().find(|p| p.device_id == device_id))
            .cloned();

        let name = advertised_name
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                nearby
                    .as_ref()
                    .map(|p| p.name.clone())
                    .filter(|s| !s.is_empty())
            })
            .or(registry_name)
            .unwrap_or_else(|| "Android phone".into());
        let model = nearby
            .map(|p| p.model)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Android".into());

        state.pairing_prompt = Some(phonecam_ui::views::state::PairingPrompt {
            device_id: device_id.to_string(),
            name,
            model,
            raised_at_ms: now_ms(),
        });
        drop(state);
        self.set_notice(FailureCode::ApprovalPending, vec![], "pair-request");
    }

    fn reconcile_resolution_downgrade(&self, phone_state: &phonecam_protocol::PhoneState) {
        let (requested, actual, downgraded) = {
            let state = self.state.lock();
            let req = state.settings.resolution.trim().to_string();
            let act = phone_state.actual_resolution.trim().to_string();
            let is_explicit = !req.is_empty() && req != "auto";
            // Only flag when streaming at a different resolution than requested and
            // not intentionally stepped down by adaptive quality (`phone_state.resolution`).
            let same_asked_target =
                phone_state.resolution.is_empty() || phone_state.resolution == req;
            let dow = phone_state.streaming
                && is_explicit
                && !act.is_empty()
                && act != req
                && same_asked_target;
            (req, act, dow)
        };

        let mut last = self.last_downgraded.lock();
        if downgraded {
            let pair = (requested.clone(), actual.clone());
            if last.as_ref() != Some(&pair) {
                *last = Some(pair);
                drop(last);
                self.set_notice(
                    FailureCode::ResolutionDowngraded,
                    vec![("requested".into(), requested), ("actual".into(), actual)],
                    "resolution-downgraded",
                );
            }
        } else if last.take().is_some() {
            drop(last);
            self.state.lock().dismiss_notice("resolution-downgraded");
            self.bump_state_version();
        }
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
        let redacted = phonecam_protocol::redact(message);
        if crate::logging::is_initialized() {
            crate::logging::append_file_line(LogLevel::Info, LogSource::Core, &redacted);
        } else {
            self.log.write("INFO", &redacted);
        }
        self.state
            .lock()
            .push_log(LogLevel::Info, LogSource::Core, redacted);
        self.bump_state_version();
    }

    pub fn log_error(&self, code: &str, message: &str) {
        let redacted = phonecam_protocol::redact(message);
        if crate::logging::is_initialized() {
            crate::logging::append_file_line(
                LogLevel::Error,
                LogSource::Core,
                &format!("{code}: {redacted}"),
            );
        } else {
            self.log.write("ERROR", &format!("{code}: {redacted}"));
        }
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
        let vcam_enabled = self.vcam_enabled.load(Ordering::Relaxed);
        let vcam_starting = self.vcam_starting.load(Ordering::Relaxed);
        let vcam_error = self.vcam_error.lock().clone();
        let usb_status = self.usb_status.lock().clone();
        let consumers = self
            .writer
            .lock()
            .as_ref()
            .map(|w| w.reader_count())
            .unwrap_or(0);

        let vcam_state = if vcam_starting {
            VcamState::Starting
        } else if vcam_error.is_some() && vcam_enabled {
            VcamState::Error
        } else if vcam_enabled {
            VcamState::Running
        } else {
            VcamState::Stopped
        };

        let mut state = self.state.lock();

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

        let mut stats = self.session.stats.lock().clone();
        let rtt_us = self.session.rtt_us.load(Ordering::Relaxed);
        stats.latency_ms = if rtt_us > 0 {
            rtt_us as f64 / 1000.0
        } else {
            -1.0
        };
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
                error: vcam_error,
            }),
            output: Some(phonecam_protocol::OutputTransform {
                mirror: state.settings.output_mirror,
                rotation: state.settings.output_rotation,
                width: state.settings.output_width,
                height: state.settings.output_height,
            }),
            usb: usb_status,
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
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            connects: self.connects.load(Ordering::Relaxed),
        };
        state.reconcile_notices();
        self.sync_cached_settings_from_state(&state);
        drop(state);
        self.toast_check_needed.store(false, Ordering::Relaxed);
        self.state_version.fetch_add(1, Ordering::SeqCst);
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
        let adb = self.adb.lock().clone();
        let devices = match adb.as_ref() {
            Some(adb) => match adb.devices() {
                Ok(devices) => {
                    *self.adb_error.lock() = None;
                    devices
                }
                Err(error) => {
                    *self.adb_error.lock() = Some(error);
                    Vec::new()
                }
            },
            None => Vec::new(),
        };

        let target_serial = {
            let selected = self.active_usb_serial.lock().clone();
            selected
                .filter(|s| devices.iter().any(|d| &d.serial == s))
                .or_else(|| {
                    devices
                        .iter()
                        .find(|d| d.is_ready())
                        .map(|d| d.serial.clone())
                })
        };

        let forwarding = adb
            .as_ref()
            .map(|adb| adb.is_reverse_active(SERVICE_PORT, target_serial.as_deref()))
            .unwrap_or(false);

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

        let usb_status = phonecam_protocol::UsbStatus {
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

        *self.usb_status.lock() = usb_status.clone();

        {
            let mut state = self.state.lock();
            if let Some(core) = &mut state.core {
                core.usb = usb_status;
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
        self.bump_state_version();
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
                *self.vcam_error.lock() = None;
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

    /// Enables or disables the virtual camera, running COM registration on a background thread.
    pub fn set_vcam(&self, enabled: bool) {
        self.spawn_vcam_op(enabled, move |rt, registrar, op_gen| {
            let status = registrar.status();
            let already_desired = if enabled {
                status.is_registered()
            } else {
                !status.is_registered()
            };
            if already_desired {
                if rt.vcam_op_gen.load(Ordering::SeqCst) == op_gen {
                    rt.vcam_registered.store(enabled, Ordering::Relaxed);
                    *rt.vcam_error.lock() = None;
                    rt.state.lock().dismiss_notice("vcam");
                    rt.vcam_starting.store(false, Ordering::Relaxed);
                    rt.broadcast_state();
                }
                return;
            }

            let action = if enabled { "register" } else { "unregister" };
            let result = if enabled {
                registrar.register()
            } else {
                registrar.unregister()
            };
            if rt.vcam_op_gen.load(Ordering::SeqCst) == op_gen {
                rt.finish_vcam_op(enabled, action, result);
            }
        });
    }

    /// Unregisters and re-registers the virtual camera on a background thread.
    pub fn restart_vcam(&self) {
        self.spawn_vcam_op(true, |rt, registrar, op_gen| {
            let _ = registrar.unregister();
            std::thread::sleep(std::time::Duration::from_millis(250));
            if rt.vcam_op_gen.load(Ordering::SeqCst) != op_gen {
                return;
            }

            let result = registrar.register();
            if rt.vcam_op_gen.load(Ordering::SeqCst) == op_gen {
                rt.finish_vcam_op(true, "restart", result);
            }
        });
    }

    fn spawn_vcam_op<F>(&self, enabled: bool, work: F)
    where
        F: FnOnce(&Arc<Self>, crate::vcam::Registrar, u64) + Send + 'static,
    {
        let op_gen = self.vcam_op_gen.fetch_add(1, Ordering::SeqCst) + 1;
        self.vcam_enabled.store(enabled, Ordering::Relaxed);
        let settings_snapshot = {
            let mut state = self.state.lock();
            state.settings.vcam_enabled = enabled;
            state.dismiss_notice("vcam");
            state.settings.clone()
        };
        let _ = crate::settings::save(&settings_snapshot);
        *self.vcam_error.lock() = None;
        self.vcam_starting.store(true, Ordering::Relaxed);
        self.broadcast_state();

        let Some(rt) = self.self_weak.upgrade() else {
            self.vcam_starting.store(false, Ordering::Relaxed);
            self.broadcast_state();
            return;
        };

        std::thread::spawn(move || {
            let _guard = rt.vcam_op_lock.lock();
            if rt.vcam_op_gen.load(Ordering::SeqCst) != op_gen {
                return;
            }

            let exe_dir = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|p| p.to_path_buf()));
            let Some(registrar) = crate::vcam::Registrar::locate(exe_dir.as_deref()) else {
                if rt.vcam_op_gen.load(Ordering::SeqCst) == op_gen {
                    if enabled {
                        let msg = "Virtual camera registrar (phonecam-vcam-register.exe) was not found. Reinstall PhoneCam or run the installer as administrator.".to_string();
                        rt.vcam_registered.store(false, Ordering::Relaxed);
                        *rt.vcam_error.lock() = Some(msg.clone());
                        rt.log.write("ERROR", &msg);
                        rt.set_notice(
                            FailureCode::VcamUnavailable,
                            vec![("message".into(), msg)],
                            "vcam",
                        );
                    }
                    rt.vcam_starting.store(false, Ordering::Relaxed);
                    rt.broadcast_state();
                }
                return;
            };

            work(&rt, registrar, op_gen);
        });
    }

    fn finish_vcam_op(&self, enabled: bool, action: &str, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.vcam_registered.store(enabled, Ordering::Relaxed);
                *self.vcam_error.lock() = None;
                self.state.lock().dismiss_notice("vcam");
                self.log
                    .write("INFO", &format!("virtual camera {action} succeeded"));
            }
            Err(err) => {
                let human = format_vcam_error(action, &err);
                if enabled {
                    self.vcam_registered.store(false, Ordering::Relaxed);
                }
                *self.vcam_error.lock() = Some(human.clone());
                self.log.write("ERROR", &human);
                self.set_notice(
                    FailureCode::VcamUnavailable,
                    vec![("message".into(), human)],
                    "vcam",
                );
            }
        }
        self.vcam_starting.store(false, Ordering::Relaxed);
        self.broadcast_state();
    }

    /// Opens the tunnel and asks the phone to connect through it using the first ready device.
    pub fn connect_usb(&self) {
        self.connect_usb_device(None);
    }

    /// Opens the tunnel and asks a specific (or default) USB phone to connect through it.
    pub fn connect_usb_device(&self, serial: Option<String>) {
        let adb = self.adb.lock().clone();
        let Some(adb) = adb else {
            self.log_error(
                "usb_missing",
                "adb wasn't found. USB mode needs Android platform-tools installed.",
            );
            return;
        };

        let target = serial.or_else(|| self.active_usb_serial.lock().clone());
        *self.active_usb_serial.lock() = target.clone();

        if let Err(error) = adb.enable_reverse(SERVICE_PORT, target.as_deref()) {
            self.log_error("usb_forward_failed", &error);
            return;
        }
        self.log_info(&format!("cable tunnel open on {SERVICE_PORT}"));

        if let Err(error) = adb.start_phone_session(SERVICE_PORT, target.as_deref()) {
            // The tunnel is up but the phone did not start: say so rather than
            // claiming a connection that is not there.
            self.log_error("usb_session_failed", &error);
        }

        self.poll_usb();
        self.broadcast_state();
    }

    /// Restarts the ADB server (or re-locates ADB if missing) and refreshes USB state.
    pub fn restart_adb(&self) {
        let adb = {
            let existing = self.adb.lock().clone();
            if let Some(adb) = existing {
                Some(adb)
            } else {
                let exe_dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|p| p.to_path_buf()));
                self.adb_checked.store(true, Ordering::Relaxed);
                match crate::adb::Adb::locate(exe_dir.as_deref()) {
                    Ok(found) => {
                        *self.adb_error.lock() = None;
                        *self.adb.lock() = Some(found.clone());
                        Some(found)
                    }
                    Err(error) => {
                        *self.adb_error.lock() = Some(error);
                        None
                    }
                }
            }
        };

        if let Some(adb) = adb {
            match adb.restart_server() {
                Ok(()) => {
                    *self.adb_error.lock() = None;
                    self.log_info("ADB server restarted");
                }
                Err(error) => {
                    *self.adb_error.lock() = Some(error.clone());
                    self.log_error("usb_restart_failed", &error);
                }
            }
        }

        self.poll_usb();
        self.broadcast_state();
    }

    /// Closes the tunnel.
    pub fn disconnect_usb(&self) {
        let adb = self.adb.lock().clone();
        let serial = self.active_usb_serial.lock().take();
        if let Some(adb) = adb {
            if let Err(error) = adb.disable_reverse(SERVICE_PORT, serial.as_deref()) {
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
    /// Records how long one stage of the video loop took without locking.
    fn note_stage(&self, index: usize, micros: i64) {
        self.stage_totals_atomic[index].fetch_add(micros, Ordering::Relaxed);
        self.stage_max_atomic[index].fetch_max(micros, Ordering::Relaxed);
    }

    /// Flushes lock-free stage accumulators into `stage_us` and `stage_max_us`.
    pub fn sync_stage_stats(&self) {
        let mut totals = self.stage_us.lock();
        let mut maxima = self.stage_max_us.lock();
        for i in 0..3 {
            totals[i] = self.stage_totals_atomic[i].load(Ordering::Relaxed);
            maxima[i] = self.stage_max_atomic[i].load(Ordering::Relaxed);
        }
    }
}

/// Reads one framed H.264 video packet into a caller-reused buffer without
/// per-frame heap allocations. Returns `(flags, pts_us, total_bytes)`.
fn read_video_packet_into(
    conn: &Connection,
    packet: &mut Vec<u8>,
) -> Option<(VideoFlags, i64, usize)> {
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

    packet.resize(total, 0);
    match conn.read_exact(packet) {
        ReadResult::Ok(_) => {}
        _ => return None,
    }

    let flags = VideoFlags(packet[12]);
    let codec = packet[13];
    let payload_len = u32::from_be_bytes([packet[16], packet[17], packet[18], packet[19]]) as usize;
    let pts_us = i64::from_be_bytes([
        packet[4], packet[5], packet[6], packet[7], packet[8], packet[9], packet[10], packet[11],
    ]);

    if codec != 1 || payload_len + VIDEO_HEADER_SIZE != total {
        return None;
    }
    Some((flags, pts_us, total))
}

fn parse_resolution_pixels(resolution: &str) -> Option<u64> {
    let (w, h) = resolution.trim().split_once('x')?;
    let w: u64 = w.parse().ok()?;
    let h: u64 = h.parse().ok()?;
    Some(w.saturating_mul(h))
}

/// Computes the adaptive quality ladder ceiling from the user's configured
/// resolution/fps and the active camera's advertised capabilities.
fn effective_max_rung(
    settings: &phonecam_ui::views::state::Settings,
    capabilities: Option<&CapabilitiesPayload>,
) -> QualityRung {
    let effective_fps = if settings.fps == 0 { 60 } else { settings.fps };

    let settings_ceiling = if settings.resolution.trim().eq_ignore_ascii_case("auto") {
        if effective_fps >= 60 {
            QualityRung::P1080_60
        } else {
            QualityRung::P1080_30
        }
    } else if let Some(req_pixels) = parse_resolution_pixels(&settings.resolution) {
        QualityRung::ALL
            .into_iter()
            .rev()
            .find(|rung| {
                parse_resolution_pixels(rung.resolution()).unwrap_or(0) <= req_pixels
                    && rung.fps() <= effective_fps
            })
            .unwrap_or(QualityRung::P480_30)
    } else if effective_fps >= 60 {
        QualityRung::P1080_60
    } else {
        QualityRung::P1080_30
    };

    let camera_ceiling = capabilities.and_then(|caps| {
        let cam = caps
            .cameras
            .iter()
            .find(|c| c.id == settings.camera)
            .or_else(|| caps.cameras.first())?;
        QualityRung::highest_supported(&cam.resolutions, &cam.fps)
    });

    camera_ceiling.map_or(settings_ceiling, |cam| settings_ceiling.min(cam))
}
fn format_vcam_error(action: &str, err: &str) -> String {
    let trimmed = err.trim();
    if trimmed.is_empty() {
        return format!(
            "Could not {action} the virtual camera. Try running PhoneCam as administrator or reinstalling."
        );
    }
    let lower = trimmed.to_lowercase();
    if lower.contains("elevation")
        || lower.contains("access is denied")
        || lower.contains("administrator")
        || lower.contains("0x80070005")
    {
        format!(
            "Administrator permission is needed to {action} the virtual camera ({trimmed}). Run the PhoneCam installer or restart as administrator."
        )
    } else {
        format!("Could not {action} the virtual camera: {trimmed}")
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

    #[test]
    fn diagnostics_report_includes_expanded_sections() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.bytes_in.store(4096, Ordering::Relaxed);
        runtime.connects.store(3, Ordering::Relaxed);
        runtime.broadcast_state();

        let report = runtime.generate_diagnostics_report();
        assert!(report.contains("Service Listeners:"));
        assert!(report.contains("Frame Bus:"));
        assert!(report.contains("ADB / USB:"));
        assert!(report.contains("Bytes received: 4096 | Handshakes: 3"));
        assert!(report.contains("Logs (last"));
    }

    #[test]
    fn broadcast_state_exposes_bytes_connects_and_latency() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        runtime.bytes_in.store(12_345, Ordering::Relaxed);
        runtime.connects.store(2, Ordering::Relaxed);
        runtime.session.rtt_us.store(18_500, Ordering::Relaxed);

        runtime.broadcast_state();

        let state = runtime.state.lock();
        assert_eq!(state.status.bytes_in, 12_345);
        assert_eq!(state.status.connects, 2);
        let latency = state.core.as_ref().unwrap().stats.latency_ms;
        assert!((latency - 18.5).abs() < 0.01);
    }

    #[test]
    fn live_controls_queue_commands_and_update_state() {
        let settings_path = crate::settings::settings_path();
        let saved_before = std::fs::read(&settings_path).ok();

        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);

        runtime.set_camera("front".to_string());
        runtime.set_video("1920x1080".to_string(), None, 8_000_000);
        runtime.set_torch(true);
        runtime.set_zoom(2.0);
        runtime.set_screen_mode(ScreenMode::Black);
        runtime.set_quality_mode(false);
        runtime.sync_output_transform(true, 90);
        runtime.set_log_level(LogLevel::Debug);

        let outbox = runtime.outbox.lock().clone();
        assert!(matches!(outbox[0], DesktopToPhone::SetCamera { ref camera } if camera == "front"));
        assert!(matches!(
            outbox[1],
            DesktopToPhone::SetVideo {
                ref resolution,
                fps: FpsValue::Auto,
                bitrate: 8_000_000
            } if resolution == "1920x1080"
        ));
        assert!(matches!(outbox[2], DesktopToPhone::SetTorch { enabled: true }));
        assert!(matches!(
            outbox[3],
            DesktopToPhone::SetImage { zoom, .. } if (zoom - 2.0).abs() < 0.01
        ));
        assert!(matches!(outbox[4], DesktopToPhone::SetScreen { mode: ScreenMode::Black }));
        assert!(matches!(
            outbox[5],
            DesktopToPhone::SetQualityMode { mode: QualityMode::Manual }
        ));
        assert!(matches!(
            outbox[6],
            DesktopToPhone::SetImage {
                mirror: true,
                rotation: 90,
                ..
            }
        ));

        {
            let state = runtime.state.lock();
            assert_eq!(state.settings.camera, "front");
            assert_eq!(state.settings.resolution, "1920x1080");
            assert_eq!(state.settings.fps, 0);
            assert_eq!(state.settings.bitrate, 8_000_000);
            assert_eq!(state.settings.screen_mode, ScreenMode::Black);
            assert_eq!(state.settings.quality_mode, QualityMode::Manual);
            assert!(state.settings.output_mirror);
            assert_eq!(state.settings.output_rotation, 90);
            assert_eq!(state.settings.log_level, LogLevel::Debug);
        }

        match saved_before {
            Some(bytes) => {
                let _ = std::fs::write(&settings_path, bytes);
            }
            None => {
                let _ = std::fs::remove_file(&settings_path);
            }
        }
    }

    #[test]
    fn preview_disabled_skips_frame_mailbox_and_transform_rotates_preview() {
        let mut settings = phonecam_ui::views::state::Settings::default();
        settings.vcam_enabled = false;
        settings.preview_enabled = false;
        let runtime = Runtime::new(&settings);

        let frame = phonecam_media::DecodedFrame::new(4, 2, vec![128; 12], 1_000);
        let mut scratch = PublishScratch::default();
        runtime.publish(frame.clone(), &mut scratch);
        assert!(
            runtime.frames.lock().is_none(),
            "preview_enabled=false must skip populating frames mailbox"
        );

        runtime.set_preview_enabled(true);
        runtime.sync_output_transform(false, 90);
        runtime.publish(frame, &mut scratch);

        let guard = runtime.frames.lock();
        let published = guard.as_ref().expect("preview frame should be published");
        assert_eq!((published.width, published.height), (2, 4));
    }

    #[test]
    fn broadcast_state_preserves_usb_adb_available_after_sync_usb_state() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "phonecam_runtime_usb_test_{}_{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("adb.exe"), b"").unwrap();

        let adb = crate::adb::Adb::locate(Some(&dir)).expect("locate dummy adb.exe");
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        *runtime.adb.lock() = Some(adb);
        runtime.adb_checked.store(true, Ordering::Relaxed);

        runtime.sync_usb_state(Vec::new(), false);
        runtime.broadcast_state();

        let state = runtime.state.lock();
        let usb = &state.core.as_ref().unwrap().usb;
        assert!(
            usb.adb_available,
            "broadcast_state() must preserve core.usb.adb_available == true after sync_usb_state"
        );
        drop(state);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broadcast_state_reflects_vcam_states_and_error() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);

        runtime.vcam_enabled.store(true, Ordering::Relaxed);
        runtime.vcam_starting.store(true, Ordering::Relaxed);
        runtime.broadcast_state();
        assert_eq!(
            runtime.state.lock().core.as_ref().unwrap().vcam.as_ref().unwrap().state,
            VcamState::Starting
        );

        runtime.vcam_starting.store(false, Ordering::Relaxed);
        *runtime.vcam_error.lock() = Some("Needs elevation".into());
        runtime.broadcast_state();
        {
            let state = runtime.state.lock();
            let vcam = state.core.as_ref().unwrap().vcam.as_ref().unwrap();
            assert_eq!(vcam.state, VcamState::Error);
            assert_eq!(vcam.error.as_deref(), Some("Needs elevation"));
        }

        *runtime.vcam_error.lock() = None;
        runtime.broadcast_state();
        assert_eq!(
            runtime.state.lock().core.as_ref().unwrap().vcam.as_ref().unwrap().state,
            VcamState::Running
        );

        runtime.vcam_enabled.store(false, Ordering::Relaxed);
        runtime.broadcast_state();
        assert_eq!(
            runtime.state.lock().core.as_ref().unwrap().vcam.as_ref().unwrap().state,
            VcamState::Stopped
        );
    }

    #[test]
    fn publish_skips_frame_bus_when_vcam_disabled() {
        let mut settings = phonecam_ui::views::state::Settings::default();
        settings.vcam_enabled = false;
        settings.preview_enabled = false;
        let runtime = Runtime::new(&settings);

        let frame = phonecam_media::DecodedFrame::new(4, 2, vec![128; 12], 1_000);
        let mut scratch = PublishScratch::default();
        runtime.publish(frame, &mut scratch);

        assert!(
            runtime.writer.lock().is_none(),
            "publish must not open FrameWriter when vcam_enabled is false"
        );
    }

    #[test]
    fn runtime_initializes_quality_from_settings_and_computes_max_rung() {
        let mut manual = phonecam_ui::views::state::Settings::default();
        manual.quality_mode = QualityMode::Manual;
        let rt_manual = Runtime::new(&manual);
        assert!(!rt_manual.quality.lock().enabled());

        let mut auto = phonecam_ui::views::state::Settings::default();
        auto.quality_mode = QualityMode::Auto;
        auto.resolution = "1280x720".into();
        auto.fps = 60;
        let rt_auto = Runtime::new(&auto);
        assert!(rt_auto.quality.lock().enabled());
        assert_eq!(effective_max_rung(&auto, None), QualityRung::P720_30);

        auto.resolution = "1920x1080".into();
        auto.fps = 30;
        assert_eq!(effective_max_rung(&auto, None), QualityRung::P1080_30);

        auto.fps = 60;
        let caps = CapabilitiesPayload {
            device: phonecam_protocol::DeviceCapabilities {
                model: "Pixel".into(),
                android_version: 34,
                app_version: "1.0.0".into(),
            },
            cameras: vec![phonecam_protocol::CameraCapability {
                id: "back-0".into(),
                facing: phonecam_protocol::CameraFacing::Back,
                resolutions: vec!["1280x720".into(), "640x480".into()],
                fps: vec![30],
                torch: true,
                focus: true,
                zoom: true,
                exposure: false,
                white_balance: false,
                min_zoom: 1.0,
                max_zoom: 4.0,
                orientation_degrees: 90,
            }],
        };
        auto.camera = "back-0".into();
        assert_eq!(effective_max_rung(&auto, Some(&caps)), QualityRung::P720_30);
    }

    #[test]
    fn resolution_downgraded_notice_emits_and_retracts() {
        let mut settings = phonecam_ui::views::state::Settings::default();
        settings.resolution = "1920x1080".into();
        settings.quality_mode = QualityMode::Manual;
        let runtime = Runtime::new(&settings);

        let downgraded = phonecam_protocol::PhoneState {
            streaming: true,
            camera: "0".into(),
            resolution: "1920x1080".into(),
            fps: 30,
            actual_resolution: "1280x720".into(),
            actual_fps: 30,
            bitrate: 4_000_000,
            mirror: false,
            rotation: 0,
            torch: false,
            zoom: 1.0,
            battery: 80,
            charging: false,
            thermal: phonecam_protocol::ThermalState::None,
        };
        runtime.reconcile_resolution_downgrade(&downgraded);
        assert!(runtime
            .state
            .lock()
            .notices
            .iter()
            .any(|n| n.code == FailureCode::ResolutionDowngraded));

        let exact = phonecam_protocol::PhoneState {
            actual_resolution: "1920x1080".into(),
            ..downgraded
        };
        runtime.reconcile_resolution_downgrade(&exact);
        assert!(!runtime
            .state
            .lock()
            .notices
            .iter()
            .any(|n| n.code == FailureCode::ResolutionDowngraded));
    }

    #[test]
    fn set_pairing_prompt_uses_nearby_or_advertised_name() {
        let settings = phonecam_ui::views::state::Settings::default();
        let runtime = Runtime::new(&settings);
        {
            let mut state = runtime.state.lock();
            let mut core = CoreState::default();
            core.nearby_phones.push(NearbyPhone {
                device_id: "dev-1".into(),
                name: "Pixel 8 Pro".into(),
                model: "GC3VE".into(),
                address: "192.168.1.50".into(),
                android_version: 34,
                streaming: false,
            });
            state.core = Some(core);
        }
        runtime.set_pairing_prompt("dev-1");
        let prompt = runtime.state.lock().pairing_prompt.clone().unwrap();
        assert_eq!(prompt.name, "Pixel 8 Pro");
        assert_eq!(prompt.model, "GC3VE");

        runtime.set_pairing_prompt_with_name("dev-2", Some("Galaxy S24"));
        let prompt2 = runtime.state.lock().pairing_prompt.clone().unwrap();
        assert_eq!(prompt2.name, "Galaxy S24");
        assert_eq!(prompt2.model, "Android");
    }

    #[test]
    fn hidden_window_skips_preview_mailbox_while_toast_flag_tracks_state() {
        let mut settings = phonecam_ui::views::state::Settings::default();
        settings.vcam_enabled = false;
        settings.preview_enabled = true;
        let runtime = Runtime::new(&settings);

        assert!(!runtime.has_active_toast());
        runtime.state.lock().set_toast("Saved", 3_000);
        runtime.bump_state_version();
        assert!(runtime.has_active_toast());

        runtime.set_window_visible(false);
        let frame = phonecam_media::DecodedFrame::new(4, 2, vec![128; 12], 1_000);
        let mut scratch = PublishScratch::default();
        runtime.publish(frame, &mut scratch);
        assert!(
            runtime.frames.lock().is_none(),
            "hidden/minimized window must skip preview BGRA mailbox"
        );
    }
}




































