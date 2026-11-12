//! The system tray icon.
//!
//! GPUI 0.2.2 ships a status item on macOS only, so on Windows this is built
//! directly on `Shell_NotifyIcon`. The shipped Electron build used a tray for two
//! reasons that still hold: closing the window must keep the virtual camera
//! publishing, because other apps hold the device; and the camera toggle has to
//! be reachable without restoring the window.
//!
//! The menu is rebuilt on demand from the current state rather than mutated,
//! because `TrackPopupMenu` returns once and the labels change with the phase.

use parking_lot::Mutex;
use std::sync::Arc;

use phonecam_ui::views::state::AppState;

/// What the tray asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    ShowWindow,
    ToggleVcam,
    ShowPairing,
    Quit,
    /// The right button was released: the UI thread should open the menu.
    OpenMenu,
}

/// The menu's labels, built from the current state.
///
/// Pure data, so the wording is testable without a message loop.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayMenu {
    /// A disabled status row, e.g. "● Streaming".
    pub status: String,
    /// A disabled row naming the phone, or "No phone connected".
    pub device: String,
    /// A disabled row naming the virtual camera's state.
    pub vcam: String,
    /// Whether the camera toggle is ticked.
    pub vcam_checked: bool,
}

impl Default for TrayMenu {
    fn default() -> Self {
        Self {
            status: "Idle".into(),
            device: "No phone connected".into(),
            vcam: "Virtual camera: off".into(),
            vcam_checked: false,
        }
    }
}

/// The phase glyph, so the tray can be read at a glance.
pub fn phase_dot(phase: &str) -> &'static str {
    match phase {
        "idle" => "○",
        "connecting" => "◌",
        "connected" => "◉",
        "streaming" => "●",
        "reconnecting" => "◍",
        "error" => "⊗",
        _ => "•",
    }
}

/// Builds the menu rows from the current state.
pub fn build_menu(state: &AppState) -> TrayMenu {
    let phase = state.display_phase();
    let status = phonecam_ui::views::state::stream_phase_label(
        state.core.as_ref().map(|c| c.phase).unwrap_or_default(),
    );

    let device = match (state.device_name(), state.core.as_ref().map(|c| c.transport.as_str())) {
        (Some(name), Some(transport)) => {
            let transport_label = match transport {
                "lan" => "Wi-Fi",
                "usb" => "USB",
                _ => "No connection",
            };
            format!("{name} ({transport_label})")
        }
        (Some(name), None) => name,
        _ => "No phone connected".to_string(),
    };

    let vcam_state = state.vcam_state().unwrap_or("stopped");
    let vcam =
        match state.core.as_ref().and_then(|c| c.vcam.as_ref()).and_then(|v| v.error.clone()) {
            Some(error) => format!("Virtual camera error: {error}"),
            None => match vcam_state {
                "running" => format!(
                    "Virtual camera: {}",
                    state
                        .core
                        .as_ref()
                        .and_then(|c| c.vcam.as_ref())
                        .map(|v| v.name.clone())
                        .unwrap_or_else(|| "PhoneCam Camera".into())
                ),
                _ => "Virtual camera: off".to_string(),
            },
        };

    TrayMenu {
        status: format!("{} {status}", phase_dot(&phase)),
        device,
        vcam,
        vcam_checked: vcam_state == "running" || vcam_state == "starting",
    }
}

/// The tray's tooltip.
pub fn tooltip(state: &AppState) -> String {
    let status = phonecam_ui::views::state::stream_phase_label(
        state.core.as_ref().map(|c| c.phase).unwrap_or_default(),
    );
    match state.device_name() {
        Some(name) => format!("PhoneCam: {name} ({status})"),
        None => format!("PhoneCam: {status}"),
    }
}

/// The window handle of a GPUI window, as the tray needs it.
///
/// GPUI exposes the native handle through `raw-window-handle`, so this is the one
/// place the platform-specific unwrapping lives.
#[cfg(windows)]
pub fn hwnd_of(window: &gpui::Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // `Window` has an inherent `window_handle()` returning an `AnyWindowHandle`,
    // so the trait method has to be named explicitly.
    let handle = HasWindowHandle::window_handle(window).ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get() as isize),
        _ => None,
    }
}

/// Hides a window without destroying it.
///
/// GPUI exposes `hide` on its platform window but not on `Window`, and the tray's
/// whole purpose is that closing hides rather than quits: a close that merely
/// refuses to happen leaves an X button that does nothing, which is worse than
/// having no close-to-tray at all.
#[cfg(windows)]
pub fn hide_window(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
    if hwnd != 0 {
        unsafe {
            let _ = ShowWindow(HWND(hwnd as *mut _), SW_HIDE);
        }
    }
}

/// Sets or clears the always-on-top (`HWND_TOPMOST`) window z-order flag.
#[cfg(windows)]
pub fn set_window_always_on_top(hwnd: isize, enabled: bool) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };
    if hwnd != 0 {
        let insert_after = if enabled { HWND_TOPMOST } else { HWND_NOTOPMOST };
        unsafe {
            let _ = SetWindowPos(
                HWND(hwnd as *mut _),
                Some(insert_after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }
}

#[cfg(not(windows))]
pub fn hide_window(_hwnd: isize) {}
#[cfg(not(windows))]
pub fn set_window_always_on_top(_hwnd: isize, _enabled: bool) {}
#[cfg(not(windows))]
pub fn hwnd_of(_window: &gpui::Window) -> Option<isize> {
    None
}

/* ------------------------------------------------------------------ */
/* Windows                                                             */
/* ------------------------------------------------------------------ */

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
        NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
        GetCursorPos, LoadIconW, RegisterClassW, SetForegroundWindow, ShowWindow,
        TrackPopupMenu, IDI_APPLICATION, MF_CHECKED, MF_GRAYED, MF_SEPARATOR,
        MF_STRING, SW_SHOW, TPM_RETURNCMD, TPM_RIGHTBUTTON, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP,
        WM_CONTEXTMENU, WM_LBUTTONUP, WM_RBUTTONUP, WNDCLASSW,
    };

    /// The message the tray posts when its icon is clicked.
    pub const WM_PHONECAM_TRAY: u32 = WM_APP + 1;

    const TRAY_ID: u32 = 1;
    const CMD_SHOW: usize = 1001;
    const CMD_VCAM: usize = 1002;
    const CMD_QR: usize = 1003;
    const CMD_QUIT: usize = 1004;

    /// A tray icon registered with the shell, and the window that receives its
    /// messages.
    ///
    /// The icon needs a window to post its clicks to, and that window has to be
    /// one this module owns: the GPUI window is created and destroyed by the
    /// framework, and hiding it on close is exactly the case where a click still
    /// has to arrive. A tiny message-only window outlives the visible one.
    pub struct Tray {
        /// The message-only window, created here.
        hwnd: HWND,
        /// The GPUI window, so a click can bring it back.
        target: HWND,
        added: bool,
        /// Where a click's intent is recorded, for the UI thread to drain.
        commands: Arc<Mutex<Vec<TrayCommand>>>,
    }

    // The HWNDs are plain handles and every call below is a Win32 message that is
    // safe from any thread.
    unsafe impl Send for Tray {}
    unsafe impl Sync for Tray {}

    /// The commands a click has recorded, shared with the UI thread.
    pub type SharedCommands = Arc<Mutex<Vec<TrayCommand>>>;

    static COMMANDS: std::sync::OnceLock<SharedCommands> = std::sync::OnceLock::new();

    /// The window procedure for the tray's message-only window.
    unsafe extern "system" fn tray_wndproc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_PHONECAM_TRAY => {
                let event = lparam.0 as u32;
                // WM_LBUTTONUP shows the window; WM_RBUTTONUP opens the menu.
                if event == WM_LBUTTONUP {
                    if let Some(commands) = COMMANDS.get() {
                        commands.lock().push(TrayCommand::ShowWindow);
                    }
                } else if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                    if let Some(commands) = COMMANDS.get() {
                        commands.lock().push(TrayCommand::OpenMenu);
                    }
                }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
        }
    }

    fn register_window_class() -> bool {
        use std::sync::OnceLock;
        static REGISTERED: OnceLock<bool> = OnceLock::new();
        *REGISTERED.get_or_init(|| {
            let class = wide("PhoneCamTrayWindow");
            let wc = WNDCLASSW {
                lpfnWndProc: Some(tray_wndproc),
                hInstance: unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap_or_default().into(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
(unsafe { RegisterClassW(&wc) }) != 0
        })
    }

    fn create_message_window() -> Option<HWND> {
        if !register_window_class() {
            return None;
        }
        let class = wide("PhoneCamTrayWindow");
        let title = wide("PhoneCam tray");
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                None,
                None,
                GetModuleHandleW(PCWSTR::null()).ok().map(Into::into),
                None,
            )
        }
        .ok()?;
        Some(hwnd)
    }


    impl Tray {
        /// Registers the icon, posting its clicks to a window this module owns.
        ///
        /// `target` is the GPUI window, used only to bring it back when the user
        /// clicks the icon.
        pub fn install(target: isize, tooltip: &str) -> Option<Self> {
            if target == 0 {
                return None;
            }
            let target = HWND(target as *mut _);
            let hwnd = create_message_window()?;

            let commands: SharedCommands = Arc::new(Mutex::new(Vec::new()));
            let _ = COMMANDS.set(commands.clone());

            let mut data = NOTIFYICONDATAW::default();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = hwnd;
            data.uID = TRAY_ID;
            data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            data.uCallbackMessage = WM_PHONECAM_TRAY;

            // The stock application icon: an installed tray icon beats a missing
            // asset, and this build ships no .ico yet.
            if let Ok(icon) = unsafe { LoadIconW(None, IDI_APPLICATION) } {
                data.hIcon = icon;
            }
            copy_tip(&mut data, tooltip);

            if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
                tracing::warn!("could not add the tray icon");
                return None;
            }

            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };

            tracing::info!("tray icon installed");
            Some(Self { hwnd, target, added: true, commands })
        }

        /// Takes whatever the user asked for since the last call.
        pub fn drain(&self) -> Vec<TrayCommand> {
            let mut commands = self.commands.lock();
            std::mem::take(&mut *commands)
        }

        /// Brings the GPUI window back after a close hid it.
        pub fn show_target(&self) {
            unsafe {
                let _ = ShowWindow(self.target, SW_SHOW);
                let _ = SetForegroundWindow(self.target);
            }
        }

        /// Shows the menu and returns what the user chose.
        pub fn show_menu(&self, menu: &TrayMenu) -> Option<TrayCommand> {
            unsafe {
                let popup = CreatePopupMenu().ok()?;

                let status = wide(&menu.status);
                let device = wide(&menu.device);
                let vcam = wide(&menu.vcam);
                let show = wide("Show PhoneCam");
                let toggle = wide("Virtual camera");
                let qr = wide("Show QR code");
                let quit = wide("Quit PhoneCam");

                // The first three are readouts, not actions.
                let _ = AppendMenuW(popup, MF_STRING | MF_GRAYED, 0, PCWSTR(status.as_ptr()));
                let _ = AppendMenuW(popup, MF_STRING | MF_GRAYED, 0, PCWSTR(device.as_ptr()));
                let _ = AppendMenuW(popup, MF_STRING | MF_GRAYED, 0, PCWSTR(vcam.as_ptr()));
                let _ = AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null());
                let _ = AppendMenuW(popup, MF_STRING, CMD_SHOW, PCWSTR(show.as_ptr()));
                let vcam_flags = if menu.vcam_checked { MF_STRING | MF_CHECKED } else { MF_STRING };
                let _ = AppendMenuW(popup, vcam_flags, CMD_VCAM, PCWSTR(toggle.as_ptr()));
                let _ = AppendMenuW(popup, MF_STRING, CMD_QR, PCWSTR(qr.as_ptr()));
                let _ = AppendMenuW(popup, MF_SEPARATOR, 0, PCWSTR::null());
                let _ = AppendMenuW(popup, MF_STRING, CMD_QUIT, PCWSTR(quit.as_ptr()));

                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);

                // Required: without setting the foreground window first, the menu
                // does not dismiss when the user clicks elsewhere.
                let _ = SetForegroundWindow(self.target);

                let chosen = TrackPopupMenu(
                    popup,
                    TPM_RETURNCMD | TPM_RIGHTBUTTON,
                    point.x,
                    point.y,
                    None,
                    self.target,
                    None,
                );

                let _ = DestroyMenu(popup);

                match chosen.0 as usize {
                    CMD_SHOW => Some(TrayCommand::ShowWindow),
                    CMD_VCAM => Some(TrayCommand::ToggleVcam),
                    CMD_QR => Some(TrayCommand::ShowPairing),
                    CMD_QUIT => Some(TrayCommand::Quit),
                    _ => None,
                }
            }
        }

        /// Updates the hover tooltip.
        pub fn set_tooltip(&self, tooltip: &str) {
            let mut data = NOTIFYICONDATAW::default();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = self.hwnd;
            data.uID = TRAY_ID;
            data.uFlags = NIF_TIP;
            copy_tip(&mut data, tooltip);
            let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
        }
    }

    impl Drop for Tray {
        fn drop(&mut self) {
            if self.added {
                let mut data = NOTIFYICONDATAW::default();
                data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                data.hWnd = self.hwnd;
                data.uID = TRAY_ID;
                let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
                self.added = false;
            }
        }
    }

    fn copy_tip(data: &mut NOTIFYICONDATAW, text: &str) {
        let encoded: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let len = encoded.len().min(data.szTip.len());
        data.szTip[..len].copy_from_slice(&encoded[..len]);
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub const WM_PHONECAM_TRAY: u32 = 0;

    pub struct Tray;

    impl Tray {
        pub fn install(_target: isize, _tooltip: &str) -> Option<Self> {
            None
        }
        pub fn show_menu(&self, _menu: &TrayMenu) -> Option<TrayCommand> {
            None
        }
        pub fn set_tooltip(&self, _tooltip: &str) {}
        pub fn drain(&self) -> Vec<TrayCommand> {
            Vec::new()
        }
        pub fn show_target(&self) {}
    }
}

pub use imp::{Tray, WM_PHONECAM_TRAY};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_phase_has_a_distinct_glyph() {
        let phases = ["idle", "connecting", "connected", "streaming", "reconnecting", "error"];
        let mut seen = std::collections::HashSet::new();
        for phase in phases {
            assert!(seen.insert(phase_dot(phase)), "{phase} reuses a glyph");
        }
        // An unknown phase still renders something rather than nothing.
        assert_eq!(phase_dot("who-knows"), "•");
    }

    #[test]
    fn the_default_menu_is_honest_about_having_no_phone() {
        let menu = TrayMenu::default();
        assert_eq!(menu.device, "No phone connected");
        assert!(!menu.vcam_checked);
    }

    #[test]
    fn an_empty_state_names_no_phone_and_an_off_camera() {
        let state = AppState::default();
        let menu = build_menu(&state);
        assert_eq!(menu.device, "No phone connected");
        assert_eq!(menu.vcam, "Virtual camera: off");
        assert!(!menu.vcam_checked);
        assert_eq!(tooltip(&state), "PhoneCam: Idle");
    }

    #[test]
    fn a_running_camera_is_ticked_and_named() {
        use phonecam_protocol::{CoreState, VcamState, VcamStatus};
        let mut state = AppState::default();
        state.core = Some(CoreState {
            vcam: Some(VcamStatus {
                state: VcamState::Running,
                name: "PhoneCam Camera".into(),
                registered: true,
                consumers: 0,
                error: None,
            }),
            ..Default::default()
        });
        let menu = build_menu(&state);
        assert!(menu.vcam_checked);
        assert!(menu.vcam.contains("PhoneCam Camera"));
    }

    #[test]
    fn a_connected_phone_names_its_transport() {
        use phonecam_protocol::{CoreState, DeviceIdentity, StreamPhase, TransportKind};
        let mut state = AppState::default();
        state.status.connected = true;
        state.core = Some(CoreState {
            transport: TransportKind::Lan,
            phase: StreamPhase::Streaming,
            device: Some(DeviceIdentity {
                device_id: "android-a".into(),
                name: "Redmi Note 11".into(),
                model: "2201117TG".into(),
                android_version: 33,
            }),
            ..Default::default()
        });

        let menu = build_menu(&state);
        assert_eq!(menu.device, "Redmi Note 11 (Wi-Fi)");
        assert!(menu.status.contains("Streaming"), "status was {}", menu.status);
        assert_eq!(tooltip(&state), "PhoneCam: Redmi Note 11 (Streaming)");
    }

    #[test]
    fn a_camera_error_is_named_rather_than_shown_as_off() {
        use phonecam_protocol::{CoreState, VcamState, VcamStatus};
        let mut state = AppState::default();
        state.core = Some(CoreState {
            vcam: Some(VcamStatus {
                state: VcamState::Error,
                name: "PhoneCam Camera".into(),
                registered: false,
                consumers: 0,
                error: Some("not registered".into()),
            }),
            ..Default::default()
        });
        let menu = build_menu(&state);
        assert!(menu.vcam.contains("not registered"), "vcam was {}", menu.vcam);
        assert!(!menu.vcam_checked);
    }
}










