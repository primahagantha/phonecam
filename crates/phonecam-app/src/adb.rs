//! adb: the USB transport.
//!
//! USB is not a second code path. The phone still dials `127.0.0.1:48721`, and
//! `adb reverse tcp:48721 tcp:48721` makes that loopback address reach this PC
//! (docs §2). That is the whole design: one connection path, two ways to make it
//! reachable.
//!
//! Two facts are reported separately and must never be collapsed (see
//! `usb_readiness` in the UI rules):
//!
//! * `adb_available` — the platform tools were found and can be run.
//! * `available` — a phone is actually on the cable.
//!
//! The shipped app read the second as the first, so a PC on Wi-Fi with no cable
//! told the user to install a tool it already had.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// A phone listed by `adb devices`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdbDevice {
    pub serial: String,
    pub model: String,
    /// `device`, `unauthorized`, `offline`, ...
    pub state: String,
}

impl AdbDevice {
    /// Whether this device is usable: present, and the user has approved the
    /// debugging key.
    pub fn is_ready(&self) -> bool {
        self.state == "device"
    }
}

/// The adb executable, once located.
#[derive(Debug, Clone)]
pub struct Adb {
    path: PathBuf,
}

impl Adb {
    /// Finds `adb`, or reports why it could not.
    ///
    /// The order matters: the copy shipped beside the exe is the one that matches
    /// this build, and a developer's PATH copy may be a different version.
    pub fn locate(exe_dir: Option<&std::path::Path>) -> Result<Self, String> {
        let mut candidates: Vec<PathBuf> = Vec::new();

        if let Some(dir) = exe_dir {
            candidates.push(dir.join("adb.exe"));
            candidates.push(dir.join("platform-tools").join("adb.exe"));
        }
        for variable in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
            if let Ok(root) = std::env::var(variable) {
                candidates.push(PathBuf::from(root).join("platform-tools").join("adb.exe"));
            }
        }
        // Last: whatever PATH resolves, which is the least predictable copy.
        if let Ok(path) = std::env::var("PATH") {
            for entry in std::env::split_paths(&path) {
                candidates.push(entry.join("adb.exe"));
            }
        }

        for candidate in candidates {
            if candidate.is_file() {
                return Ok(Self { path: candidate });
            }
        }

        Err("adb wasn't found. USB mode needs Android platform-tools installed.".into())
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Runs one adb command and returns its combined output.
    ///
    /// A timeout is enforced because `adb` can block indefinitely on a wedged
    /// device or a pending authorization prompt, and a blocked UI thread is worse
    /// than a reported failure.
    fn run(&self, args: &[&str], timeout: Duration) -> Result<String, String> {
        let mut child = Command::new(&self.path)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags_no_window()
            .spawn()
            .map_err(|e| format!("could not run adb: {e}"))?;

        let deadline = std::time::Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        return Err(format!("adb {} timed out", args.join(" ")));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(format!("adb failed: {e}")),
            }
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("could not read adb output: {e}"))?;

        let mut text = String::from_utf8_lossy(&output.stdout).to_string();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        Ok(text)
    }

    /// Lists the phones on the cable.
    pub fn devices(&self) -> Result<Vec<AdbDevice>, String> {
        let output = self.run(&["devices", "-l"], Duration::from_secs(8))?;
        Ok(parse_devices(&output))
    }

    /// Opens the tunnel the phone dials through.
    pub fn enable_reverse(&self, port: u16) -> Result<(), String> {
        let spec = format!("tcp:{port}");
        let output = self.run(&["reverse", &spec, &spec], Duration::from_secs(8))?;
        // adb reports most failures on stdout, and the two commands that matter
        // here phrase them differently.
        let lowered = output.to_lowercase();
        if lowered.contains("error") || lowered.contains("cannot") {
            return Err(output.trim().to_string());
        }
        Ok(())
    }

    pub fn disable_reverse(&self, port: u16) -> Result<(), String> {
        let spec = format!("tcp:{port}");
        self.run(&["reverse", "--remove", &spec], Duration::from_secs(8)).map(|_| ())
    }

    /// Whether a reverse tunnel for `port` is already open.
    pub fn is_reverse_active(&self, port: u16) -> bool {
        let spec = format!("tcp:{port}");
        self.run(&["reverse", "--list"], Duration::from_secs(8))
            .map(|output| output.contains(&spec))
            .unwrap_or(false)
    }

    /// Starts the phone's stream service, pointing it at the tunnel.
    ///
    /// The debug package is tried first because a development build installs
    /// alongside the release one, and only one of them is running.
    pub fn start_phone_session(&self, port: u16) -> Result<(), String> {
        let mut last_error = String::from("no PhoneCam package answered");

        for package in ["com.phonecam.camera.debug", "com.phonecam.camera"] {
            let component = format!("{package}/com.phonecam.camera.service.CameraStreamService");
            let port_text = port.to_string();
            let output = self.run(
                &[
                    "shell",
                    "am",
                    "start-foreground-service",
                    "-n",
                    &component,
                    "-a",
                    "com.phonecam.camera.action.CONNECT_MANUAL",
                    "--es",
                    "host",
                    "127.0.0.1",
                    "--ei",
                    "port",
                    &port_text,
                ],
                Duration::from_secs(10),
            )?;

            // Capitalised here, unlike `enable_reverse`: the two adb surfaces
            // word their failures differently and both spellings are real.
            if !output.contains("Error") && !output.contains("Exception") {
                return Ok(());
            }
            last_error = output.trim().to_string();
        }

        Err(last_error)
    }
}

/// Parses `adb devices -l`.
///
/// Lines that are not device rows (the header, blanks) are skipped rather than
/// treated as failures.
pub fn parse_devices(output: &str) -> Vec<AdbDevice> {
    let mut devices = Vec::new();

    for line in output.lines() {
        let line = line.trim_end_matches(['\r', ' ']);
        if line.is_empty() || line.starts_with("List of devices") {
            continue;
        }

        let mut fields = line.split_whitespace();
        let Some(serial) = fields.next() else {
            continue;
        };
        let Some(state) = fields.next() else {
            continue;
        };

        // `model:2201117TG` appears among the trailing fields, not at a fixed
        // position, because adb adds and removes fields between versions.
        let model = fields
            .find_map(|field| field.strip_prefix("model:"))
            .unwrap_or("")
            .to_string();

        devices.push(AdbDevice {
            serial: serial.to_string(),
            model,
            state: state.to_string(),
        });
    }

    devices
}

/// Hides the console window a child process would otherwise flash.
trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for Command {
    #[cfg(windows)]
    fn creation_flags_no_window(&mut self) -> &mut Self {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        self.creation_flags(0x0800_0000)
    }

    #[cfg(not(windows))]
    fn creation_flags_no_window(&mut self) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_normal_device_list_parses() {
        let output = "List of devices attached\r\n\
                      1234567890   device product:lineage model:2201117TG device:camellia transport_id:1\r\n";
        let devices = parse_devices(output);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].serial, "1234567890");
        assert_eq!(devices[0].model, "2201117TG");
        assert_eq!(devices[0].state, "device");
        assert!(devices[0].is_ready());
    }

    #[test]
    fn an_empty_list_is_empty_not_an_error() {
        assert!(parse_devices("List of devices attached\r\n\r\n").is_empty());
        assert!(parse_devices("").is_empty());
    }

    #[test]
    fn an_unauthorized_device_is_listed_but_not_ready() {
        let output = "List of devices attached\n\
                      abc123\tunauthorized\n";
        let devices = parse_devices(output);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].state, "unauthorized");
        assert!(
            !devices[0].is_ready(),
            "an unauthorized phone must not be treated as usable"
        );
    }

    #[test]
    fn several_phones_are_all_listed() {
        let output = "List of devices attached\n\
                      a1\tdevice model:Pixel\n\
                      b2\tdevice model:Redmi\n";
        let devices = parse_devices(output);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[1].model, "Redmi");
    }

    #[test]
    fn a_device_without_a_model_still_parses() {
        let output = "List of devices attached\n1234\tdevice\n";
        let devices = parse_devices(output);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].model, "");
    }

    #[test]
    fn an_offline_device_is_not_ready() {
        let output = "List of devices attached\n1234\toffline\n";
        let devices = parse_devices(output);
        assert!(!devices[0].is_ready());
    }

    #[test]
    fn a_missing_adb_names_the_fix() {
        let error = Adb::locate(Some(std::path::Path::new(r"Z:\definitely\not\here")));
        // PATH may still resolve a real adb on a developer machine, so only the
        // failure wording is pinned.
        if let Err(message) = error {
            assert!(message.contains("platform-tools"), "message was {message}");
            assert!(message.contains("wasn't found"), "message was {message}");
        }
    }
}
