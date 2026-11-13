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

/// Sets the window titlebar icon and Windows taskbar icon via WM_SETICON.
#[cfg(windows)]
pub fn set_window_icon(hwnd: isize) {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, LoadIconW, LoadImageW, SendMessageW, HICON, ICON_BIG, ICON_SMALL,
        IDI_APPLICATION, IMAGE_ICON, LR_LOADFROMFILE, SM_CXICON, SM_CXSMICON, SM_CYICON,
        SM_CYSMICON, WM_SETICON,
    };

    if hwnd == 0 {
        return;
    }
    let hwnd = HWND(hwnd as *mut _);

    let icon_path = find_icon_path();
    let (hicon_big, hicon_small) = if let Some(ref path) = icon_path {
        let wide_path: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let cx_big = unsafe { GetSystemMetrics(SM_CXICON) };
        let cy_big = unsafe { GetSystemMetrics(SM_CYICON) };
        let cx_sm = unsafe { GetSystemMetrics(SM_CXSMICON) };
        let cy_sm = unsafe { GetSystemMetrics(SM_CYSMICON) };

        let big = unsafe {
            LoadImageW(
                None,
                PCWSTR(wide_path.as_ptr()),
                IMAGE_ICON,
                cx_big,
                cy_big,
                LR_LOADFROMFILE,
            )
        }
        .ok()
        .map(|h| HICON(h.0))
        .filter(|h| !h.is_invalid());

        let small = unsafe {
            LoadImageW(
                None,
                PCWSTR(wide_path.as_ptr()),
                IMAGE_ICON,
                cx_sm,
                cy_sm,
                LR_LOADFROMFILE,
            )
        }
        .ok()
        .map(|h| HICON(h.0))
        .filter(|h| !h.is_invalid());

        (big, small)
    } else {
        (None, None)
    };

    let fallback = if hicon_big.is_none() && hicon_small.is_none() {
        unsafe { LoadIconW(None, IDI_APPLICATION).ok().filter(|h| !h.is_invalid()) }
    } else {
        None
    };

    let final_big = hicon_big.or(fallback);
    let final_small = hicon_small.or(final_big);

    if let Some(hicon) = final_big {
        unsafe {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_BIG as usize)),
                Some(LPARAM(hicon.0 as isize)),
            );
        }
    }
    if let Some(hicon) = final_small {
        unsafe {
            let _ = SendMessageW(
                hwnd,
                WM_SETICON,
                Some(WPARAM(ICON_SMALL as usize)),
                Some(LPARAM(hicon.0 as isize)),
            );
        }
    }
    if icon_path.is_some() {
        tracing::info!("Applied PhoneCam window & taskbar icon from {:?}", icon_path);
    }
}

/// Keeps the process-wide single-instance mutex alive for the lifetime of the app.
#[cfg(windows)]
pub struct SingleInstanceGuard(isize);

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        use windows::Win32::Foundation::{CloseHandle, HANDLE};
        if self.0 != 0 {
            unsafe {
                let _ = CloseHandle(HANDLE(self.0 as *mut _));
            }
            self.0 = 0;
        }
    }
}

#[cfg(not(windows))]
pub struct SingleInstanceGuard;

/// Acquires the single-instance lock, or focuses the already-running PhoneCam
/// window and returns `None` if another instance is active.
#[cfg(windows)]
pub fn acquire_single_instance_or_focus() -> Option<SingleInstanceGuard> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;

    let name: Vec<u16> = "Local\\PhoneCamDesktopSingleInstance\0".encode_utf16().collect();
    let handle = match unsafe { CreateMutexW(None, false, PCWSTR(name.as_ptr())) } {
        Ok(h) => h,
        Err(err) => {
            if err.code() == ERROR_ALREADY_EXISTS.into() {
                focus_existing_window();
                return None;
            }
            return Some(SingleInstanceGuard(0));
        }
    };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(handle);
        }
        focus_existing_window();
        return None;
    }
    Some(SingleInstanceGuard(handle.0 as isize))
}

#[cfg(not(windows))]
pub fn acquire_single_instance_or_focus() -> Option<SingleInstanceGuard> {
    Some(SingleInstanceGuard)
}

#[cfg(windows)]
fn focus_existing_window() {
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, FindWindowW, GetClassNameW, GetWindowTextW, IsIconic, SetForegroundWindow,
        ShowWindow, SW_RESTORE, SW_SHOW,
    };

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let mut class_buf = [0u16; 64];
        let class_len = unsafe { GetClassNameW(hwnd, &mut class_buf) };
        if class_len > 0 {
            let class_name = String::from_utf16_lossy(&class_buf[..class_len as usize]);
            if class_name == "PhoneCamTrayWindow" {
                return BOOL(1);
            }
        }

        let mut title_buf = [0u16; 256];
        let len = unsafe { GetWindowTextW(hwnd, &mut title_buf) };
        if len > 0 {
            let title = String::from_utf16_lossy(&title_buf[..len as usize]);
            if title == "PhoneCam" || title.starts_with("PhoneCam - ") {
                let target = unsafe { &mut *(lparam.0 as *mut HWND) };
                *target = hwnd;
                return BOOL(0);
            }
        }
        BOOL(1)
    }

    let exact_title: Vec<u16> = "PhoneCam\0".encode_utf16().collect();
    let mut found = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(exact_title.as_ptr())) }
        .unwrap_or(HWND::default());
    if found.is_invalid() || found.0.is_null() {
        let mut enumerated = HWND::default();
        let _ = unsafe { EnumWindows(Some(enum_cb), LPARAM(&mut enumerated as *mut HWND as isize)) };
        found = enumerated;
    }

    if !found.is_invalid() && !found.0.is_null() {
        unsafe {
            if IsIconic(found).as_bool() {
                let _ = ShowWindow(found, SW_RESTORE);
            }
            let _ = ShowWindow(found, SW_SHOW);
            let _ = SetForegroundWindow(found);
        }
    }
}

/// Returns `(x, y, width, height, maximized)` for the window if available.
#[cfg(windows)]
pub fn window_bounds(hwnd: isize) -> Option<(f32, f32, f32, f32, bool)> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowPlacement, GetWindowRect, IsWindow, IsZoomed, WINDOWPLACEMENT,
    };

    if hwnd == 0 {
        return None;
    }
    let hwnd = HWND(hwnd as *mut _);
    if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
        return None;
    }

    let maximized = unsafe { IsZoomed(hwnd) }.as_bool();
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_ok() {
        let r = placement.rcNormalPosition;
        let width = (r.right - r.left) as f32;
        let height = (r.bottom - r.top) as f32;
        if width > 0.0 && height > 0.0 {
            return Some((r.left as f32, r.top as f32, width, height, maximized));
        }
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut rect) }.is_ok() {
        let width = (rect.right - rect.left) as f32;
        let height = (rect.bottom - rect.top) as f32;
        if width > 0.0 && height > 0.0 {
            return Some((rect.left as f32, rect.top as f32, width, height, maximized));
        }
    }

    None
}

/// Returns `true` when the desktop window is currently visible (not hidden in tray
/// and not minimized).
#[cfg(windows)]
pub fn is_window_visible(hwnd: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow, IsWindowVisible};

    if hwnd == 0 {
        return true;
    }
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return true;
        }
        IsWindowVisible(hwnd).as_bool() && !IsIconic(hwnd).as_bool()
    }
}

#[cfg(not(windows))]
pub fn is_window_visible(_hwnd: isize) -> bool {
    true
}

/// Result of prompting the user for a diagnostics export destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveDialogResult {
    Selected(std::path::PathBuf),
    Cancelled,
    Unavailable,
}

/// Opens a native Windows "Save As" dialog for diagnostics export.
#[cfg(windows)]
pub fn save_diagnostics_dialog(hwnd: Option<isize>) -> SaveDialogResult {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Controls::Dialogs::{
        CommDlgExtendedError, GetSaveFileNameW, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT,
        OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    let mut file_buf = [0u16; 1024];
    let default_name: Vec<u16> = "phonecam-diagnostics.txt\0".encode_utf16().collect();
    file_buf[..default_name.len()].copy_from_slice(&default_name);

    let filter: Vec<u16> = "Text files (*.txt)\0*.txt\0All files (*.*)\0*.*\0\0"
        .encode_utf16()
        .collect();
    let title: Vec<u16> = "Export PhoneCam diagnostics\0".encode_utf16().collect();
    let def_ext: Vec<u16> = "txt\0".encode_utf16().collect();

    let owner = hwnd
        .filter(|h| *h != 0)
        .map(|h| HWND(h as *mut _))
        .unwrap_or_default();

    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        nFilterIndex: 1,
        lpstrFile: PWSTR(file_buf.as_mut_ptr()),
        nMaxFile: file_buf.len() as u32,
        lpstrTitle: PCWSTR(title.as_ptr()),
        lpstrDefExt: PCWSTR(def_ext.as_ptr()),
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
        ..Default::default()
    };

    if unsafe { GetSaveFileNameW(&mut ofn) }.as_bool() {
        let len = file_buf
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(file_buf.len());
        let os = OsString::from_wide(&file_buf[..len]);
        SaveDialogResult::Selected(std::path::PathBuf::from(os))
    } else if unsafe { CommDlgExtendedError().0 } == 0 {
        SaveDialogResult::Cancelled
    } else {
        SaveDialogResult::Unavailable
    }
}

#[cfg(not(windows))]
pub fn save_diagnostics_dialog(_hwnd: Option<isize>) -> SaveDialogResult {
    SaveDialogResult::Unavailable
}

fn find_icon_path_in(candidates: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

/// Searches standard app and repository locations for `icon.ico`.
pub fn find_icon_path() -> Option<std::path::PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("icon.ico"));
            candidates.push(dir.join("resources").join("icon.ico"));
            candidates.push(dir.join("assets").join("icon.ico"));
            let mut curr = dir.parent();
            while let Some(parent) = curr {
                candidates.push(parent.join("assets").join("icon.ico"));
                candidates.push(parent.join("resources").join("icon.ico"));
                candidates.push(parent.join("dist").join("unpacked").join("icon.ico"));
                candidates.push(parent.join("icon.ico"));
                curr = parent.parent();
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("assets").join("icon.ico"));
        candidates.push(cwd.join("resources").join("icon.ico"));
        candidates.push(cwd.join("dist").join("unpacked").join("icon.ico"));
        candidates.push(cwd.join("icon.ico"));
        let mut curr = cwd.parent();
        while let Some(parent) = curr {
            candidates.push(parent.join("assets").join("icon.ico"));
            candidates.push(parent.join("resources").join("icon.ico"));
            candidates.push(parent.join("dist").join("unpacked").join("icon.ico"));
            candidates.push(parent.join("icon.ico"));
            curr = parent.parent();
        }
    }
    candidates.push(std::path::PathBuf::from("assets").join("icon.ico"));
    candidates.push(std::path::PathBuf::from("resources").join("icon.ico"));
    candidates.push(std::path::PathBuf::from("dist").join("unpacked").join("icon.ico"));
    candidates.push(std::path::PathBuf::from("icon.ico"));
    find_icon_path_in(&candidates)
}

#[cfg(not(windows))]
pub fn window_bounds(_hwnd: isize) -> Option<(f32, f32, f32, f32, bool)> {
    None
}

#[cfg(not(windows))]
pub fn hide_window(_hwnd: isize) {}
#[cfg(not(windows))]
pub fn set_window_always_on_top(_hwnd: isize, _enabled: bool) {}
#[cfg(not(windows))]
pub fn set_window_icon(_hwnd: isize) {}
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
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
        GetCursorPos, LoadIconW, LoadImageW, RegisterClassW, SetForegroundWindow, ShowWindow,
        TrackPopupMenu, HICON, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE,
        MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, SW_SHOW, TPM_RETURNCMD, TPM_RIGHTBUTTON,
        WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CONTEXTMENU, WM_LBUTTONUP, WM_RBUTTONUP,
        WNDCLASSW,
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
        owned_icon: Option<HICON>,
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

            let mut owned_icon = None;
            if let Some(icon_path) = find_icon_path() {
                let wide_path = wide(&icon_path.to_string_lossy());
                if let Ok(handle) = unsafe {
                    LoadImageW(
                        None,
                        PCWSTR(wide_path.as_ptr()),
                        IMAGE_ICON,
                        0,
                        0,
                        LR_LOADFROMFILE | LR_DEFAULTSIZE,
                    )
                } {
                    let hicon = HICON(handle.0);
                    if !hicon.is_invalid() {
                        data.hIcon = hicon;
                        owned_icon = Some(hicon);
                    }
                }
            }
            if data.hIcon.is_invalid() {
                if let Ok(icon) = unsafe { LoadIconW(None, IDI_APPLICATION) } {
                    data.hIcon = icon;
                }
            }
            copy_tip(&mut data, tooltip);

            if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
                if let Some(icon) = owned_icon {
                    let _ = unsafe { DestroyIcon(icon) };
                }
                tracing::warn!("could not add the tray icon");
                return None;
            }

            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };

            tracing::info!("tray icon installed");
            Some(Self { hwnd, target, added: true, owned_icon, commands })
        }

        /// Takes whatever the user asked for since the last call.
        pub fn drain(&self) -> Vec<TrayCommand> {
            let mut commands = self.commands.lock();
            std::mem::take(&mut *commands)
        }

        /// Brings the GPUI window back after a close hid it.
        pub fn show_target(&self) {
            set_window_icon(self.target.0 as isize);
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
            if let Some(icon) = self.owned_icon.take() {
                let _ = unsafe { DestroyIcon(icon) };
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

    #[test]
    fn window_bounds_returns_none_for_null_hwnd() {
        assert_eq!(window_bounds(0), None);
    }

    #[test]
    fn find_icon_path_in_returns_first_existing_file() {
        let temp = std::env::temp_dir().join(format!("phonecam-tray-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&temp);
        std::fs::create_dir_all(temp.join("resources")).unwrap();
        let missing = temp.join("missing.ico");
        let present = temp.join("resources").join("icon.ico");
        std::fs::write(&present, b"ico").unwrap();

        assert_eq!(find_icon_path_in(&[missing.clone(), present.clone()]), Some(present.clone()));
        assert_eq!(find_icon_path_in(&[missing]), None);
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn set_window_icon_handles_null_hwnd_safely() {
        set_window_icon(0);
    }

    #[test]
    fn find_icon_path_finds_workspace_icon() {
        let path = find_icon_path();
        assert!(path.is_some(), "expected to find icon.ico in repository");
        let p = path.unwrap();
        assert!(p.exists(), "found path {:?} should exist", p);
    }
}










