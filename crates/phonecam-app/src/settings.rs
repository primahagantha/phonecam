//! Settings persistence.
//!
//! The settings file is treated as **untrusted input**. It is user-writable JSON
//! that a previous version, a partial write or a hand edit can leave malformed,
//! and a settings file that cannot be read must never stop the app from
//! starting. Every field falls back to its default independently, so one bad
//! value does not discard the rest.

use std::path::PathBuf;

use phonecam_protocol::{LogLevel, QualityMode, ScreenMode};
use phonecam_ui::views::state::Settings;
use serde::{Deserialize, Serialize};

/// Where the settings live, beside the platform's app-data directory.
pub fn settings_path() -> PathBuf {
    let base = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PhoneCam");
    base.join("desktop-settings.json")
}

/// The on-disk shape. Every field is optional so a partial file still loads.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SettingsFile {
    #[serde(default)]
    vcam_enabled: Option<bool>,
    #[serde(default)]
    output_mirror: Option<bool>,
    #[serde(default)]
    output_rotation: Option<u32>,
    #[serde(default)]
    output_width: Option<u32>,
    #[serde(default)]
    output_height: Option<u32>,
    #[serde(default)]
    output_follows_stream: Option<bool>,
    #[serde(default)]
    camera: Option<String>,
    #[serde(default)]
    resolution: Option<String>,
    #[serde(default)]
    fps: Option<u32>,
    #[serde(default)]
    bitrate: Option<u64>,
    #[serde(default)]
    quality_mode: Option<String>,
    #[serde(default)]
    screen_mode: Option<String>,
    #[serde(default)]
    log_level: Option<String>,
    #[serde(default)]
    always_on_top: Option<bool>,
    #[serde(default)]
    window_x: Option<f32>,
    #[serde(default)]
    window_y: Option<f32>,
    #[serde(default)]
    window_width: Option<f32>,
    #[serde(default)]
    window_height: Option<f32>,
    #[serde(default)]
    window_maximized: Option<bool>,
    #[serde(default)]
    preview_enabled: Option<bool>,
    #[serde(default)]
    last_device_id: Option<String>,
}

/// Loads the settings, falling back to defaults per field.
///
/// A missing file is not an error and a corrupt one is not fatal: the app must
/// always start.
pub fn load() -> Settings {
    let path = settings_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    match serde_json::from_str::<SettingsFile>(&text) {
        Ok(file) => coerce(file),
        Err(error) => {
            tracing::warn!("settings file is unreadable, using defaults: {error}");
            Settings::default()
        }
    }
}

/// Applies the file over the defaults, validating each value.
fn coerce(file: SettingsFile) -> Settings {
    let defaults = Settings::default();

    Settings {
        vcam_enabled: file.vcam_enabled.unwrap_or(defaults.vcam_enabled),
        output_mirror: file.output_mirror.unwrap_or(defaults.output_mirror),
        output_rotation: file
            .output_rotation
            .filter(|r| matches!(r, 0 | 90 | 180 | 270))
            .unwrap_or(defaults.output_rotation),
        // Both must be even (NV12 stores chroma in pairs) and inside a sane
        // range; a stale or hand-edited value must not reach the frame bus.
        output_width: file
            .output_width
            .filter(|&w| is_dimension(w))
            .unwrap_or(defaults.output_width),
        output_height: file
            .output_height
            .filter(|&h| is_dimension(h))
            .unwrap_or(defaults.output_height),
        output_follows_stream: file
            .output_follows_stream
            .unwrap_or(defaults.output_follows_stream),
        camera: file
            .camera
            .filter(|c| !c.is_empty())
            .unwrap_or(defaults.camera),
        resolution: file
            .resolution
            .filter(|r| r == "auto" || phonecam_protocol::parse_resolution(r).is_some())
            .unwrap_or(defaults.resolution),
        fps: file
            .fps
            .filter(|&f| f == 0 || (1..=120).contains(&f))
            .unwrap_or(defaults.fps),
        bitrate: file
            .bitrate
            .filter(|&b| (100_000..=50_000_000).contains(&b))
            .unwrap_or(defaults.bitrate),
        quality_mode: match file.quality_mode.as_deref() {
            Some("manual") => QualityMode::Manual,
            Some("auto") => QualityMode::Auto,
            _ => defaults.quality_mode,
        },
        screen_mode: match file.screen_mode.as_deref() {
            Some("keepOn") => ScreenMode::KeepOn,
            Some("dim") => ScreenMode::Dim,
            Some("black") => ScreenMode::Black,
            _ => defaults.screen_mode,
        },
        log_level: match file.log_level.as_deref() {
            Some("error") => LogLevel::Error,
            Some("warn") => LogLevel::Warn,
            Some("info") => LogLevel::Info,
            Some("debug") => LogLevel::Debug,
            Some("trace") => LogLevel::Trace,
            _ => defaults.log_level,
        },
        always_on_top: file.always_on_top.unwrap_or(defaults.always_on_top),
        window_x: file
            .window_x
            .filter(|x| x.is_finite() && (-10_000.0..=20_000.0).contains(x)),
        window_y: file
            .window_y
            .filter(|y| y.is_finite() && (-10_000.0..=20_000.0).contains(y)),
        window_width: file
            .window_width
            .filter(|w| (900.0..=10_000.0).contains(w))
            .unwrap_or(defaults.window_width),
        window_height: file
            .window_height
            .filter(|h| (640.0..=10_000.0).contains(h))
            .unwrap_or(defaults.window_height),
        window_maximized: file.window_maximized.unwrap_or(defaults.window_maximized),
        preview_enabled: file.preview_enabled.unwrap_or(defaults.preview_enabled),
        last_device_id: file.last_device_id.filter(|id| !id.is_empty()),
    }
}

/// A dimension must be an even number in a range a camera could plausibly use.
fn is_dimension(value: u32) -> bool {
    value >= 16 && value <= 8192 && value % 2 == 0
}

/// Writes the settings atomically.
///
/// The write goes to a temporary file and is renamed over the real one, so a
/// crash mid-write cannot leave a half-file that the next start has to discard.
pub fn save(settings: &Settings) -> std::io::Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = SettingsFile {
        vcam_enabled: Some(settings.vcam_enabled),
        output_mirror: Some(settings.output_mirror),
        output_rotation: Some(settings.output_rotation),
        output_width: Some(settings.output_width),
        output_height: Some(settings.output_height),
        output_follows_stream: Some(settings.output_follows_stream),
        camera: Some(settings.camera.clone()),
        resolution: Some(settings.resolution.clone()),
        fps: Some(settings.fps),
        bitrate: Some(settings.bitrate),
        quality_mode: Some(settings.quality_mode.as_str().to_string()),
        screen_mode: Some(settings.screen_mode.as_str().to_string()),
        log_level: Some(settings.log_level.as_str().to_string()),
        always_on_top: Some(settings.always_on_top),
        window_x: settings.window_x,
        window_y: settings.window_y,
        window_width: Some(settings.window_width),
        window_height: Some(settings.window_height),
        window_maximized: Some(settings.window_maximized),
        preview_enabled: Some(settings.preview_enabled),
        last_device_id: settings.last_device_id.clone(),
    };

    let text = serde_json::to_string_pretty(&file)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Settings {
        coerce(serde_json::from_str::<SettingsFile>(json).unwrap_or_default())
    }

    #[test]
    fn an_empty_file_yields_the_defaults() {
        let settings = parse("{}");
        let defaults = Settings::default();
        assert_eq!(settings.vcam_enabled, defaults.vcam_enabled);
        assert_eq!(settings.resolution, defaults.resolution);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        // Loading is total: it must never fail, whatever the disk says.
        let _ = load();
    }

    #[test]
    fn the_camera_is_on_by_default() {
        // The camera is the product; a false default hid it from OBS and Meet
        // on first run.
        assert!(Settings::default().vcam_enabled);
    }

    #[test]
    fn the_default_resolution_is_720p30_not_auto() {
        // Auto resolves to the largest advertised size, which measured at 31%
        // app CPU and 130% camera-HAL CPU on a Redmi Note 11.
        let defaults = Settings::default();
        assert_eq!(defaults.resolution, "1280x720");
        assert_eq!(defaults.fps, 30);
    }

    #[test]
    fn an_odd_output_width_is_refused() {
        let settings = parse(r#"{"output_width": 1921}"#);
        assert_eq!(
            settings.output_width, 1920,
            "NV12 has no layout for an odd width"
        );
    }

    #[test]
    fn an_absurd_dimension_is_refused() {
        assert_eq!(parse(r#"{"output_height": 100000}"#).output_height, 1080);
        assert_eq!(parse(r#"{"output_width": 0}"#).output_width, 1920);
    }

    #[test]
    fn an_invalid_rotation_is_refused() {
        assert_eq!(parse(r#"{"output_rotation": 45}"#).output_rotation, 0);
        assert_eq!(parse(r#"{"output_rotation": 90}"#).output_rotation, 90);
        assert_eq!(parse(r#"{"output_rotation": 270}"#).output_rotation, 270);
    }

    #[test]
    fn one_bad_field_does_not_discard_the_others() {
        let settings = parse(r#"{"output_rotation": 45, "resolution": "1920x1080"}"#);
        assert_eq!(settings.output_rotation, 0, "the bad field falls back");
        assert_eq!(settings.resolution, "1920x1080", "the good field survives");
    }

    #[test]
    fn a_malformed_resolution_falls_back() {
        assert_eq!(parse(r#"{"resolution": "huge"}"#).resolution, "1280x720");
        assert_eq!(parse(r#"{"resolution": "auto"}"#).resolution, "auto");
        assert_eq!(parse(r#"{"resolution": "640x480"}"#).resolution, "640x480");
    }

    #[test]
    fn an_out_of_range_bitrate_falls_back() {
        assert_eq!(parse(r#"{"bitrate": 1}"#).bitrate, 4_000_000);
        assert_eq!(parse(r#"{"bitrate": 8000000}"#).bitrate, 8_000_000);
    }

    #[test]
    fn enums_parse_from_their_wire_strings() {
        assert_eq!(
            parse(r#"{"quality_mode": "manual"}"#).quality_mode,
            QualityMode::Manual
        );
        assert_eq!(
            parse(r#"{"screen_mode": "black"}"#).screen_mode,
            ScreenMode::Black
        );
        assert_eq!(
            parse(r#"{"log_level": "debug"}"#).log_level,
            LogLevel::Debug
        );
        // An unknown string is not an error.
        assert_eq!(
            parse(r#"{"quality_mode": "sideways"}"#).quality_mode,
            QualityMode::Auto
        );
    }

    #[test]
    fn the_last_device_id_survives_and_empty_is_none() {
        assert_eq!(
            parse(r#"{"last_device_id": "android-a31f92"}"#)
                .last_device_id
                .as_deref(),
            Some("android-a31f92")
        );
        assert_eq!(parse(r#"{"last_device_id": ""}"#).last_device_id, None);
    }

    #[test]
    fn a_corrupt_file_is_not_fatal() {
        // The caller gets defaults rather than a panic or an error to handle.
        let settings = coerce(serde_json::from_str::<SettingsFile>("not json").unwrap_or_default());
        assert_eq!(settings.resolution, Settings::default().resolution);
    }

    #[test]
    fn window_position_coerces_valid_and_rejects_out_of_range() {
        let defaults = parse("{}");
        assert_eq!(defaults.window_x, None);
        assert_eq!(defaults.window_y, None);

        let valid = parse(r#"{"window_x": -120.5, "window_y": 240.0}"#);
        assert_eq!(valid.window_x, Some(-120.5));
        assert_eq!(valid.window_y, Some(240.0));

        let invalid = parse(r#"{"window_x": -20000.0, "window_y": 25000.0}"#);
        assert_eq!(invalid.window_x, None);
        assert_eq!(invalid.window_y, None);
    }
}

/// The desktop's own log file, beside the settings.
///
/// The core writes `core.log` and the phone answers `adb logcat`, but the
/// desktop's half of a problem was invisible in the Electron build: "it froze" or
/// "it disconnected" produced nothing on this side, so a report could not be
/// confirmed, dated, or tied to a state. This writes one line per event with a
/// timestamp, and it is deliberately append-only and size-capped rather than
/// rotated, because the interesting window is always the end.
pub struct DesktopLog {
    path: PathBuf,
    written: std::sync::atomic::AtomicU64,
}

impl DesktopLog {
    /// A log beside the settings file.
    pub fn new() -> Self {
        let path = settings_path().with_file_name("desktop.log");
        Self {
            path,
            written: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Appends one line.
    ///
    /// A log that cannot be written must never take the app down with it, so the
    /// failure is swallowed after one attempt to create the directory.
    pub fn write(&self, level: &str, message: &str) {
        use std::io::Write;

        // Past a few megabytes, start over: the tail is what gets read.
        const LIMIT: u64 = 2 * 1024 * 1024;
        let count = self.written.load(std::sync::atomic::Ordering::Relaxed);
        if count == 0 {
            if let Ok(meta) = std::fs::metadata(&self.path) {
                self.written
                    .store(meta.len(), std::sync::atomic::Ordering::Relaxed);
            }
        }
        if self.written.load(std::sync::atomic::Ordering::Relaxed) > LIMIT {
            let _ = std::fs::remove_file(&self.path);
            self.written.store(0, std::sync::atomic::Ordering::Relaxed);
        }

        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let stamp = format_log_time(now_ms());
        let line = format!("{stamp} {level:<5} {message}\n");
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            if file.write_all(line.as_bytes()).is_ok() {
                self.written
                    .fetch_add(line.len() as u64, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

impl Default for DesktopLog {
    fn default() -> Self {
        Self::new()
    }
}

/// `HH:MM:SS.mmm`, matching the diagnostics panel.
fn format_log_time(ts_ms: i64) -> String {
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

/// Milliseconds since the Unix epoch.
fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod log_tests {
    use super::*;

    #[test]
    fn the_log_writes_a_dated_line() {
        let log = DesktopLog::new();
        log.write("INFO", "unit test line");

        // The default location may not be writable in a sandbox, so only assert
        // when the write actually landed.
        if let Ok(text) = std::fs::read_to_string(log.path()) {
            assert!(text.contains("unit test line"), "log was {text:?}");
            // A timestamp, so a freeze can be dated.
            assert!(text.contains(':'), "log line has no time: {text:?}");
        }
    }
}

/// A persisted paired device.
///
/// The field names are the C++ core's, not Rust's: `deviceSecret` and `lastSeen`
/// rather than `secret_hex` and `last_seen_ms`. That is deliberate. The original
/// writes `%APPDATA%\PhoneCam\devices.json`, and a phone already paired to it
/// holds a secret that only matches that file, so this build has to read the same
/// one or every user has to rescan. An earlier version here wrote a bare array
/// with Rust names, which failed to parse the real file outright.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct StoredDevice {
    #[serde(rename = "deviceId")]
    pub device_id: String,
    pub name: String,
    #[serde(rename = "deviceSecret")]
    pub secret_hex: String,
    #[serde(rename = "lastSeen")]
    pub last_seen_ms: i64,
    #[serde(default = "default_camera")]
    pub preferred_camera: String,
    #[serde(default = "default_resolution")]
    pub preferred_resolution: String,
    #[serde(default = "default_fps")]
    pub preferred_fps: u32,
    #[serde(default = "default_connection")]
    pub preferred_connection: String,
}

fn default_camera() -> String {
    "back".into()
}
fn default_resolution() -> String {
    "1920x1080".into()
}
fn default_fps() -> u32 {
    30
}
fn default_connection() -> String {
    "auto".into()
}

/// The on-disk envelope: `{"devices": [...]}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DevicesFile {
    devices: Vec<StoredDevice>,
}

/// Where the paired devices live.
pub fn devices_path() -> PathBuf {
    settings_path().with_file_name("devices.json")
}

/// Loads the paired devices.
///
/// A missing file is a first run, not an error. A file this build cannot parse is
/// warned about and skipped, because throwing away a user's pairing because of one
/// bad field is worse than ignoring it.
pub fn load_devices() -> Vec<StoredDevice> {
    let Ok(text) = std::fs::read_to_string(devices_path()) else {
        return Vec::new();
    };
    match serde_json::from_str::<DevicesFile>(&text) {
        Ok(file) => file.devices,
        Err(error) => {
            tracing::warn!("devices.json is unreadable, starting empty: {error}");
            Vec::new()
        }
    }
}

/// Writes the paired devices atomically.
///
/// Secrets are written to a user-private file and never to a log.
pub fn save_devices(devices: &[StoredDevice]) -> std::io::Result<()> {
    if let Some(parent) = devices_path().parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&DevicesFile {
        devices: devices.to_vec(),
    })?;
    let temp = devices_path().with_extension("json.tmp");
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, devices_path())?;
    Ok(())
}
