//! The virtual camera registrar.
//!
//! `PhoneCamVirtualCamera.dll` is a Media Foundation COM server loaded by the
//! Windows Camera Frame Server, so it cannot be registered from inside this
//! process: the machine-wide CLSID key needs elevation, and
//! `MFCreateVirtualCamera` needs to run where the camera store lives. The
//! original C++ tree ships `phonecam-vcam-register.exe` for exactly that, and it
//! stays the tool of record.
//!
//! **`--status` is the check that answers the real question.** Running
//! `--register` and printing success only means the registrar did not error; the
//! camera can still be missing from the device list, because of a stale CLSID key
//! or a Frame Server that has not re-enumerated. A user then finds "PhoneCam
//! camera not found" in OBS from an installer that said it worked. So the app
//! asks `--status`, which exits non-zero unless the camera is actually
//! enumerable, and reports that rather than assuming.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// The registrar tool, once located.
#[derive(Debug, Clone)]
pub struct Registrar {
    path: PathBuf,
}

/// What `--status` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationState {
    /// Registered and enumerable: other apps can select the camera.
    Registered,
    /// The tool ran but the camera is not there.
    NotRegistered { detail: String },
    /// The tool could not be found or run.
    Unavailable { detail: String },
}

impl RegistrationState {
    pub fn is_registered(&self) -> bool {
        matches!(self, Self::Registered)
    }
}

impl Registrar {
    /// Finds `phonecam-vcam-register.exe` beside the exe, or in the tree.
    pub fn locate(exe_dir: Option<&Path>) -> Option<Self> {
        let mut candidates = Vec::new();
        if let Some(dir) = exe_dir {
            candidates.push(dir.join("phonecam-vcam-register.exe"));
            candidates.push(dir.join("resources").join("phonecam-vcam-register.exe"));
            if let Some(parent) = dir.parent() {
                candidates.push(parent.join("virtual-camera").join("phonecam-vcam-register.exe"));
                if let Some(grandparent) = parent.parent() {
                    candidates.push(
                        grandparent.join("virtual-camera").join("phonecam-vcam-register.exe"),
                    );
                }
            }
        }
        candidates.into_iter().find(|c| c.is_file()).map(|path| Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Asks whether the camera is registered **and enumerable**.
    pub fn status(&self) -> RegistrationState {
        match self.run(&["--status"]) {
            Ok((success, output)) => {
                if success {
                    RegistrationState::Registered
                } else {
                    RegistrationState::NotRegistered { detail: output.trim().to_string() }
                }
            }
            Err(detail) => RegistrationState::Unavailable { detail },
        }
    }

    /// Registers the camera. Needs elevation.
    pub fn register(&self) -> Result<(), String> {
        let (success, output) = self.run(&["--register"])?;
        if success {
            Ok(())
        } else {
            Err(output.trim().to_string())
        }
    }

    /// Unregisters the camera.
    ///
    /// Deliberately does not delete the machine-wide CLSID key: disabling the
    /// camera is a routine, non-elevated action, and removing HKLM means the next
    /// enable cannot put it back without elevation.
    pub fn unregister(&self) -> Result<(), String> {
        let (success, output) = self.run(&["--unregister"])?;
        if success {
            Ok(())
        } else {
            Err(output.trim().to_string())
        }
    }

    fn run(&self, args: &[&str]) -> Result<(bool, String), String> {
        let mut command = Command::new(&self.path);
        command
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }

        let child = command
            .spawn()
            .map_err(|e| format!("could not run the registrar: {e}"))?;

        // The tool can block behind a UAC prompt, which no amount of waiting will
        // resolve; a bounded wait keeps the UI responsive.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(child.wait_with_output());
        });

        match rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(output)) => {
                let mut text = String::from_utf8_lossy(&output.stdout).to_string();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                Ok((output.status.success(), text))
            }
            Ok(Err(e)) => Err(format!("could not read the registrar's output: {e}")),
            Err(_) => Err("the registrar did not answer in 15 s; it may be waiting for elevation".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_tool_is_none_rather_than_a_fake_path() {
        let found = Registrar::locate(Some(Path::new(r"Z:\definitely\not\here")));
        assert!(found.is_none(), "a missing registrar must not be invented");
    }

    #[test]
    fn registration_state_reports_what_it_knows() {
        assert!(RegistrationState::Registered.is_registered());
        assert!(!RegistrationState::NotRegistered { detail: "x".into() }.is_registered());
        assert!(!RegistrationState::Unavailable { detail: "x".into() }.is_registered());
    }

    #[test]
    fn a_status_failure_names_its_reason() {
        // The detail is what the UI shows, so it must not be dropped.
        let state = RegistrationState::NotRegistered { detail: "COM registration: ABSENT".into() };
        match state {
            RegistrationState::NotRegistered { detail } => assert!(detail.contains("ABSENT")),
            other => panic!("expected NotRegistered, got {other:?}"),
        }
    }
}
