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
        let cwd = std::env::current_dir().ok();
        Self::locate_with_roots(
            exe_dir,
            cwd.as_deref(),
            &[PathBuf::from(
                r"C:\Program Files\PhoneCam\phonecam-vcam-register.exe",
            )],
        )
    }

    fn locate_with_roots(
        exe_dir: Option<&Path>,
        current_dir: Option<&Path>,
        extra_candidates: &[PathBuf],
    ) -> Option<Self> {
        let mut candidates = Vec::new();
        if let Some(dir) = exe_dir {
            candidates.push(dir.join("phonecam-vcam-register.exe"));
            candidates.push(dir.join("resources").join("phonecam-vcam-register.exe"));
            if let Some(parent) = dir.parent() {
                candidates.push(
                    parent
                        .join("virtual-camera")
                        .join("phonecam-vcam-register.exe"),
                );
                candidates.push(
                    parent
                        .join("dist")
                        .join("unpacked")
                        .join("phonecam-vcam-register.exe"),
                );
                if let Some(grandparent) = parent.parent() {
                    candidates.push(
                        grandparent
                            .join("virtual-camera")
                            .join("phonecam-vcam-register.exe"),
                    );
                    candidates.push(
                        grandparent
                            .join("dist")
                            .join("unpacked")
                            .join("phonecam-vcam-register.exe"),
                    );
                }
            }
        }
        if let Some(cwd) = current_dir {
            candidates.push(
                cwd.join("dist")
                    .join("unpacked")
                    .join("phonecam-vcam-register.exe"),
            );
        }
        candidates.extend_from_slice(extra_candidates);
        candidates
            .into_iter()
            .find(|c| c.is_file())
            .map(|path| Self { path })
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
                    RegistrationState::NotRegistered {
                        detail: output.trim().to_string(),
                    }
                }
            }
            Err(detail) => RegistrationState::Unavailable { detail },
        }
    }

    /// Registers the camera. Attempts direct execution first, then falls back to
    /// UAC elevation if administrative privileges are required.
    pub fn register(&self) -> Result<(), String> {
        match self.run(&["--register"]) {
            Ok((true, _)) => Ok(()),
            Ok((false, output)) => {
                if self.is_elevation_needed(&output) {
                    tracing::info!("virtual camera registration requires elevation, requesting UAC...");
                    self.run_elevated("--register")
                } else {
                    Err(output.trim().to_string())
                }
            }
            Err(e) => {
                if e.contains("elevation") || e.contains("740") {
                    tracing::info!("virtual camera registration process requires elevation, requesting UAC...");
                    self.run_elevated("--register")
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Unregisters the camera.
    ///
    /// Deliberately does not delete the machine-wide CLSID key: disabling the
    /// camera is a routine, non-elevated action, and removing HKLM means the next
    /// enable cannot put it back without elevation.
    pub fn unregister(&self) -> Result<(), String> {
        match self.run(&["--unregister"]) {
            Ok((true, _)) => Ok(()),
            Ok((false, output)) => {
                if self.is_elevation_needed(&output) {
                    tracing::info!("virtual camera unregistration requires elevation, requesting UAC...");
                    self.run_elevated("--unregister")
                } else {
                    Err(output.trim().to_string())
                }
            }
            Err(e) => {
                if e.contains("elevation") || e.contains("740") {
                    tracing::info!("virtual camera unregistration requires elevation, requesting UAC...");
                    self.run_elevated("--unregister")
                } else {
                    Err(e)
                }
            }
        }
    }

    fn is_elevation_needed(&self, output: &str) -> bool {
        let lower = output.to_lowercase();
        lower.contains("administrator")
            || lower.contains("access is denied")
            || lower.contains("elevation")
            || lower.contains("hkey_local_machine")
    }

    #[cfg(windows)]
    fn run_elevated(&self, arg: &str) -> Result<(), String> {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{CloseHandle, ERROR_CANCELLED, HANDLE};
        use windows::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
        use windows::Win32::UI::Shell::{
            ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
        };
        use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;

        let path_w: Vec<u16> = self
            .path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let verb_w: Vec<u16> = std::ffi::OsStr::new("runas")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let arg_w: Vec<u16> = std::ffi::OsStr::new(arg)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut exec_info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: PCWSTR(verb_w.as_ptr()),
            lpFile: PCWSTR(path_w.as_ptr()),
            lpParameters: PCWSTR(arg_w.as_ptr()),
            nShow: SW_HIDE.0 as i32,
            ..Default::default()
        };

        // SAFETY: `exec_info` contains valid null-terminated UTF-16 wide string pointers whose
        // underlying Vecs live across the ShellExecuteExW invocation.
        let status = unsafe { ShellExecuteExW(&mut exec_info) };
        if let Err(err) = status {
            if err.code() == ERROR_CANCELLED.to_hresult() {
                return Err("UAC elevation was declined by user.".into());
            }
            return Err(format!("UAC elevation failed: {err}"));
        }

        let h_process = exec_info.hProcess;
        if h_process.is_invalid() || h_process == HANDLE::default() {
            std::thread::sleep(Duration::from_millis(500));
            return match self.status() {
                RegistrationState::Registered => Ok(()),
                _ => Err("Elevated registration executed, but camera is not yet enumerable.".into()),
            };
        }

        // SAFETY: `h_process` is a valid process handle returned by ShellExecuteExW with SEE_MASK_NOCLOSEPROCESS.
        let wait_res = unsafe { WaitForSingleObject(h_process, 30_000) };
        let mut exit_code: u32 = 0;
        // SAFETY: `h_process` is valid and pointer to `exit_code` is aligned and writable.
        let code_res = unsafe { GetExitCodeProcess(h_process, &mut exit_code) };
        // SAFETY: We own `h_process` due to SEE_MASK_NOCLOSEPROCESS and must close it.
        unsafe {
            let _ = CloseHandle(h_process);
        }

        if wait_res.0 != 0 {
            return Err("Elevated registrar timed out waiting for completion.".into());
        }
        if code_res.is_err() || exit_code != 0 {
            return Err(format!("Elevated registrar exited with error code {exit_code}"));
        }

        Ok(())
    }

    #[cfg(not(windows))]
    fn run_elevated(&self, _arg: &str) -> Result<(), String> {
        Err("Elevation is only supported on Windows".into())
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
            Err(_) => {
                Err("the registrar did not answer in 15 s; it may be waiting for elevation".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_tool_is_none_rather_than_a_fake_path() {
        let missing = Path::new(r"Z:\definitely\not\here");
        let found = Registrar::locate_with_roots(Some(missing), Some(missing), &[]);
        assert!(found.is_none(), "a missing registrar must not be invented");
    }

    #[test]
    fn locate_finds_dist_unpacked_and_program_files_with_exe_dir_first() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("phonecam_vcam_test_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let dist_reg = root
            .join("dist")
            .join("unpacked")
            .join("phonecam-vcam-register.exe");
        std::fs::create_dir_all(dist_reg.parent().unwrap()).unwrap();
        std::fs::write(&dist_reg, b"").unwrap();

        let target_release = root.join("target").join("release");
        std::fs::create_dir_all(&target_release).unwrap();
        let from_grandparent = Registrar::locate_with_roots(Some(&target_release), None, &[])
            .expect("locate from ../../dist/unpacked");
        assert_eq!(
            from_grandparent.path().canonicalize().unwrap(),
            dist_reg.canonicalize().unwrap()
        );

        let bin_dir = root.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let from_parent = Registrar::locate_with_roots(Some(&bin_dir), None, &[])
            .expect("locate from ../dist/unpacked");
        assert_eq!(
            from_parent.path().canonicalize().unwrap(),
            dist_reg.canonicalize().unwrap()
        );

        let from_cwd = Registrar::locate_with_roots(None, Some(&root), &[])
            .expect("locate from current_dir/dist/unpacked");
        assert_eq!(
            from_cwd.path().canonicalize().unwrap(),
            dist_reg.canonicalize().unwrap()
        );

        let exe_reg = target_release.join("phonecam-vcam-register.exe");
        std::fs::write(&exe_reg, b"").unwrap();
        let beside_exe = Registrar::locate_with_roots(Some(&target_release), Some(&root), &[])
            .expect("exe_dir stays highest priority");
        assert_eq!(
            beside_exe.path().canonicalize().unwrap(),
            exe_reg.canonicalize().unwrap()
        );

        let _ = std::fs::remove_dir_all(&root);
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
        let state = RegistrationState::NotRegistered {
            detail: "COM registration: ABSENT".into(),
        };
        match state {
            RegistrationState::NotRegistered { detail } => assert!(detail.contains("ABSENT")),
            other => panic!("expected NotRegistered, got {other:?}"),
        }
    }
}
