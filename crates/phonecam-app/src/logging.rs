//! Structured tracing subscriber for PhoneCam desktop.
//!
//! Captures all `tracing` events across workspace crates, redacts secrets with
//! `phonecam_protocol::redact`, appends formatted lines to
//! `%APPDATA%\PhoneCam\desktop.log` with 2 MB automatic rotation (`desktop.log.1`),
//! forwards events into the active [`Runtime`] log ring buffer, and keeps the
//! stderr `fmt` layer active.

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use phonecam_protocol::{redact, LogLevel};
use phonecam_ui::views::state::{format_log_time, now_ms, LogLine, LogSource};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::reload;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry};

use crate::runtime::Runtime;

/// Maximum log file size before rotating `desktop.log` to `desktop.log.1`.
pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const EARLY_BUFFER_LIMIT: usize = 500;

static INITIALIZED: OnceLock<()> = OnceLock::new();
static RELOAD_HANDLE: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static RUNTIME_SINK: OnceLock<Mutex<Option<Weak<Runtime>>>> = OnceLock::new();
static EARLY_LOGS: OnceLock<Mutex<Vec<LogLine>>> = OnceLock::new();
static FILE_WRITER: OnceLock<Mutex<RotatingLogWriter>> = OnceLock::new();

fn runtime_sink() -> &'static Mutex<Option<Weak<Runtime>>> {
    RUNTIME_SINK.get_or_init(|| Mutex::new(None))
}

fn early_logs() -> &'static Mutex<Vec<LogLine>> {
    EARLY_LOGS.get_or_init(|| Mutex::new(Vec::new()))
}

fn file_writer() -> &'static Mutex<RotatingLogWriter> {
    FILE_WRITER.get_or_init(|| Mutex::new(RotatingLogWriter::new(default_log_path(), MAX_LOG_BYTES)))
}

/// Returns `%APPDATA%\PhoneCam\desktop.log` (beside `desktop-settings.json`).
pub fn default_log_path() -> PathBuf {
    crate::settings::settings_path().with_file_name("desktop.log")
}

/// Returns whether [`init`] has been called in this process.
pub fn is_initialized() -> bool {
    INITIALIZED.get().is_some()
}

/// Initializes the global `tracing` subscriber with stderr formatting,
/// rotating file logging, and runtime UI sink forwarding.
pub fn init(level: LogLevel) {
    INITIALIZED.get_or_init(|| {
        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(level_directive(level)));
        let (filter_layer, handle) = reload::Layer::new(filter);
        let _ = RELOAD_HANDLE.set(handle);

        let subscriber = tracing_subscriber::registry()
            .with(filter_layer)
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .with(DesktopTracingLayer);

        let _ = subscriber.try_init();
    });
}

/// Updates the active tracing filter level at runtime.
pub fn set_level(level: LogLevel) {
    if let Some(handle) = RELOAD_HANDLE.get() {
        let _ = handle.reload(EnvFilter::new(level_directive(level)));
    }
}

/// Registers the active [`Runtime`] to receive captured tracing events and
/// drains any startup events buffered prior to binding.
pub fn bind_runtime(runtime: Arc<Runtime>) {
    if let Ok(mut slot) = runtime_sink().lock() {
        *slot = Some(Arc::downgrade(&runtime));
    }

    let buffered = match early_logs().lock() {
        Ok(mut buf) => std::mem::take(&mut *buf),
        Err(_) => Vec::new(),
    };

    if !buffered.is_empty() {
        {
            let mut state = runtime.state.lock();
            for line in buffered {
                state.logs.push(line);
            }
            if state.logs.len() > 500 {
                let excess = state.logs.len() - 500;
                state.logs.drain(..excess);
            }
        }
        runtime.bump_state_version();
    }
}

/// Appends an already-formatted log line directly to the rotating desktop log file.
pub fn append_file_line(level: LogLevel, source: LogSource, message: &str) {
    let redacted = redact(message);
    let ts_ms = now_ms();
    let line = format!(
        "{} {:<5} [{}] {}\n",
        format_log_time(ts_ms),
        level_str(level),
        source.label(),
        redacted
    );
    if let Ok(mut writer) = file_writer().lock() {
        writer.write_line(&line);
    }
}

fn level_directive(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warn",
        LogLevel::Info => "info",
        LogLevel::Debug => "debug",
        LogLevel::Trace => "trace",
    }
}

fn level_str(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "ERROR",
        LogLevel::Warn => "WARN",
        LogLevel::Info => "INFO",
        LogLevel::Debug => "DEBUG",
        LogLevel::Trace => "TRACE",
    }
}

fn to_protocol_level(level: &Level) -> LogLevel {
    match *level {
        Level::ERROR => LogLevel::Error,
        Level::WARN => LogLevel::Warn,
        Level::INFO => LogLevel::Info,
        Level::DEBUG => LogLevel::Debug,
        Level::TRACE => LogLevel::Trace,
    }
}

/// Maps a tracing event target onto a [`LogSource`].
pub fn source_for_target(target: &str) -> LogSource {
    if target.starts_with("phonecam_core") || target == "core" {
        LogSource::Core
    } else if target.starts_with("phonecam_media")
        || target.starts_with("phonecam_framebus")
        || target == "media"
    {
        LogSource::Media
    } else if target.starts_with("phonecam_ui") || target == "ui" || target == "renderer" {
        LogSource::Renderer
    } else if target == "phone" || target.starts_with("phone:") || target.contains("::phone") {
        LogSource::Phone
    } else {
        LogSource::App
    }
}

struct MessageVisitor {
    message: String,
    fields: String,
}

impl MessageVisitor {
    fn new() -> Self {
        Self { message: String::new(), fields: String::new() }
    }

    fn finish(self) -> String {
        if self.message.is_empty() {
            self.fields.trim_start().to_string()
        } else if self.fields.is_empty() {
            self.message
        } else {
            format!("{}{}", self.message, self.fields)
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={value}", field.name());
        }
    }
}

struct DesktopTracingLayer;

impl<S: Subscriber> Layer<S> for DesktopTracingLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let level = to_protocol_level(meta.level());
        let source = source_for_target(meta.target());

        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);
        let raw = visitor.finish();
        if raw.is_empty() {
            return;
        }
        let message = redact(&raw);
        let ts_ms = now_ms();

        let line = format!(
            "{} {:<5} [{}] {}\n",
            format_log_time(ts_ms),
            level_str(level),
            source.label(),
            message
        );
        if let Ok(mut writer) = file_writer().lock() {
            writer.write_line(&line);
        }

        let entry = LogLine { ts_ms, level, source, message };
        let runtime = runtime_sink()
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().and_then(Weak::upgrade));

        if let Some(runtime) = runtime {
            {
                let mut state = runtime.state.lock();
                state.logs.push(entry);
                if state.logs.len() > 500 {
                    let excess = state.logs.len() - 500;
                    state.logs.drain(..excess);
                }
            }
            runtime.bump_state_version();
        } else if let Ok(mut early) = early_logs().lock() {
            early.push(entry);
            if early.len() > EARLY_BUFFER_LIMIT {
                let excess = early.len() - EARLY_BUFFER_LIMIT;
                early.drain(..excess);
            }
        }
    }
}

/// Append-only file writer that rotates `desktop.log` -> `desktop.log.1` at `max_bytes`.
pub struct RotatingLogWriter {
    path: PathBuf,
    max_bytes: u64,
    written: Option<u64>,
}

impl RotatingLogWriter {
    pub fn new(path: PathBuf, max_bytes: u64) -> Self {
        Self { path, max_bytes, written: None }
    }

    pub fn backup_path(&self) -> PathBuf {
        backup_path_for(&self.path)
    }

    pub fn write_line(&mut self, line: &str) {
        let bytes = line.as_bytes();
        let current = match self.written {
            Some(len) => len,
            None => {
                let len = fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
                self.written = Some(len);
                len
            }
        };

        if current > 0 && current.saturating_add(bytes.len() as u64) > self.max_bytes {
            let backup = self.backup_path();
            let _ = fs::remove_file(&backup);
            let _ = fs::rename(&self.path, &backup);
            self.written = Some(0);
        }

        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&self.path) {
            if file.write_all(bytes).is_ok() {
                let next = self.written.unwrap_or(0).saturating_add(bytes.len() as u64);
                self.written = Some(next);
            }
        }
    }
}

fn backup_path_for(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".1");
    PathBuf::from(os)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_maps_to_expected_log_source() {
        assert_eq!(source_for_target("phonecam_core::session"), LogSource::Core);
        assert_eq!(source_for_target("phonecam_media::flicker"), LogSource::Media);
        assert_eq!(source_for_target("phonecam_framebus::writer"), LogSource::Media);
        assert_eq!(source_for_target("phonecam_ui::views::panels"), LogSource::Renderer);
        assert_eq!(source_for_target("phone"), LogSource::Phone);
        assert_eq!(source_for_target("phonecam_app::runtime"), LogSource::App);
    }

    #[test]
    fn rotating_writer_moves_to_dot_1_above_limit() {
        let dir = std::env::temp_dir().join(format!("phonecam-log-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let log_path = dir.join("desktop.log");

        let mut writer = RotatingLogWriter::new(log_path.clone(), 32);
        writer.write_line("first line 1234567890\n");
        assert!(log_path.is_file());
        assert!(!writer.backup_path().is_file());

        // Exceeding 32 bytes triggers rotation to desktop.log.1
        writer.write_line("second line 1234567890\n");
        assert!(writer.backup_path().is_file());
        let backup_text = fs::read_to_string(writer.backup_path()).unwrap();
        let active_text = fs::read_to_string(&log_path).unwrap();
        assert!(backup_text.contains("first line"));
        assert!(active_text.contains("second line"));

        let _ = fs::remove_dir_all(&dir);
    }
}
