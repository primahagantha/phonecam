//! PhoneCam: one process.
//!
//! The Electron build was three processes and two decodes: a renderer decoding
//! H.264 with WebCodecs for the preview, a C++ service decoding the same stream
//! again for the virtual camera, and a main process shuttling frames between them
//! over a loopback socket. Here the decode happens once, the UI draws the same
//! pixels the frame bus publishes, and there is no IPC hop in the media path.
//!
//! What stays where it was:
//!
//! * The **phone is still the client** and the desktop is the server, so the wire
//!   protocol is untouched and the existing Android app works.
//! * The **virtual camera is still a Media Foundation DLL** loaded by the Windows
//!   Frame Server, fed through the same `framebuffer-v2.bin` with the same
//!   68-byte header.

use phonecam_app::{logging, runtime, settings, tray};

use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui::{
    px, size, App, AppContext, Application, Bounds, ClipboardItem, WindowBackgroundAppearance,
    WindowBounds, WindowHandle, WindowOptions,
};
use phonecam_protocol::LogLevel;
use phonecam_ui::theme::Timing;
use phonecam_ui::views::state::{now_ms, AppState, LogSource, Settings};
use phonecam_ui::views::{AppView, SideEffect, Tab};

use runtime::Runtime;

fn main() -> anyhow::Result<()> {
    let _single_instance = match tray::acquire_single_instance_or_focus() {
        Some(guard) => guard,
        None => return Ok(()),
    };

    let settings = settings::load();
    logging::init(settings.log_level);
    tracing::info!("PhoneCam starting");

    Application::new().run(move |cx: &mut App| {
        let runtime = Runtime::new(&settings);
        logging::bind_runtime(runtime.clone());
        runtime.set_preview_enabled(settings.preview_enabled);
        runtime.set_quality_mode(settings.quality_mode == phonecam_protocol::QualityMode::Auto);

        // Start the listeners before the window so the first frame of UI already
        // knows whether the service is reachable.
        match runtime.start() {
            Some(port) => tracing::info!("listening for a phone on {port}"),
            None => runtime.set_notice(
                phonecam_ui::views::FailureCode::Generic,
                vec![(
                    "message".into(),
                    "Port 48721 is in use. Another PhoneCam may be running.".into(),
                )],
                "core-error:port_unavailable",
            ),
        }
        runtime.refresh_usb();
        runtime.check_vcam();
        runtime.broadcast_state();

        // Register GPUI keyboard shortcuts (Space, F11, Esc, Ctrl+1..4, Tab/Shift-Tab)
        phonecam_ui::bind_keys(cx);

        let bounds = match (settings.window_x, settings.window_y) {
            (Some(x), Some(y)) => Bounds {
                origin: gpui::point(px(x), px(y)),
                size: size(px(settings.window_width), px(settings.window_height)),
            },
            _ => Bounds::centered(
                None,
                size(px(settings.window_width), px(settings.window_height)),
                cx,
            ),
        };
        let window_bounds = if settings.window_maximized {
            WindowBounds::Maximized(bounds)
        } else {
            WindowBounds::Windowed(bounds)
        };

        let view = cx.new(|cx| {
            let mut app = AppView::new();
            app.set_focus_handle(cx.focus_handle());
            app.set_state(runtime.state.lock().clone());
            app
        });

        let initial_title = title_for(&runtime.state.lock());
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(window_bounds),
                    window_min_size: Some(size(px(900.0), px(640.0))),
                    window_background: WindowBackgroundAppearance::Opaque,
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some(initial_title.into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, _cx| view.clone(),
            )
            .expect("could not open the window");

        // Focus the root view on open so shortcuts and navigation work immediately
        let hwnd = window
            .update(cx, |view, window, _cx| {
                if let Some(focus_handle) = view.focus_handle.as_ref() {
                    window.focus(focus_handle);
                }
                tray::hwnd_of(window)
            })
            .unwrap_or(None);

        if let Some(hwnd) = hwnd {
            tray::set_window_always_on_top(hwnd, settings.always_on_top);
        }

        // The window pulls state and frames on a timer: a push per decoded frame
        // would re-render the whole window 30 times a second for a change that
        // only affects one image.
        start_ui_pump(cx, window, view.clone(), runtime.clone(), hwnd);

        // Drain the effects the panels asked for.
        start_effect_pump(cx, window, view.clone(), runtime.clone(), hwnd);

        // The tray, and the close-to-hide behaviour it exists for.
        start_tray(cx, window, view.clone(), runtime.clone());
        start_tray_pump(cx, window, view.clone(), runtime.clone());

        cx.activate(true);
    });

    Ok(())
}

/// Copies the runtime snapshot and the newest picture into the view.
fn start_ui_pump(
    cx: &mut App,
    window: WindowHandle<AppView>,
    _view: gpui::Entity<AppView>,
    runtime: std::sync::Arc<Runtime>,
    hwnd: Option<isize>,
) {
    cx.spawn(async move |cx| {
        let mut last_state_version = 0u64;
        let (mut last_title, mut last_tooltip) = {
            let state = runtime.state.lock();
            (title_for(&state), tray::tooltip(&state))
        };
        let mut last_visible = true;
        let mut geometry_elapsed_ms = 0u64;
        loop {
            let visible = hwnd.map(tray::is_window_visible).unwrap_or(true);
            if visible != last_visible {
                last_visible = visible;
                runtime.set_window_visible(visible);
            }
            let sleep_ms = if visible { 8 } else { 100 };
            cx.background_executor()
                .timer(Duration::from_millis(sleep_ms))
                .await;

            if visible {
                geometry_elapsed_ms = geometry_elapsed_ms.saturating_add(sleep_ms);
                if geometry_elapsed_ms >= 1000 {
                    geometry_elapsed_ms = 0;
                    let _ = window.update(cx, |_, window, _| {
                        persist_window_geometry(window, &runtime);
                    });
                }
            }

            let toast_expired = if runtime.has_active_toast() {
                let mut state = runtime.state.lock();
                state.toast_expires_at_ms.is_some() && state.expire_toast_if_due(now_ms())
            } else {
                false
            };
            if toast_expired {
                runtime.bump_state_version();
            }

            let current_version = runtime.state_version.load(Ordering::Relaxed);
            let state_changed = current_version != last_state_version;
            let frame = if visible {
                runtime.frames.lock().take()
            } else {
                if let Some(discarded) = runtime.frames.lock().take() {
                    runtime.return_buffer(discarded.bgra);
                }
                None
            };

            if !state_changed && frame.is_none() {
                continue;
            }

            let snapshot = if state_changed {
                Some(runtime.state.lock().clone())
            } else {
                None
            };

            if let Some(snapshot) = snapshot.as_ref() {
                let next_title = title_for(snapshot);
                if next_title != last_title {
                    last_title = next_title.clone();
                    let _ = window.update(cx, |_, window, _| {
                        window.set_window_title(&next_title);
                    });
                }

                let next_tooltip = tray::tooltip(snapshot);
                if next_tooltip != last_tooltip {
                    last_tooltip = next_tooltip.clone();
                    let tray = runtime.tray.lock();
                    if let Some(tray) = tray.as_ref() {
                        tray.set_tooltip(&next_tooltip);
                    }
                }
            }

            let rt = runtime.clone();
            let updated = window.update(cx, |app, window, cx| {
                if let Some(snapshot) = snapshot {
                    if !snapshot.settings.preview_enabled {
                        if let Some(recycled) = app.clear_preview_in(Some(window)) {
                            rt.return_buffer(recycled);
                        }
                    }
                    app.set_state(snapshot);
                }
                if let Some(frame) = frame {
                    if app.state.settings.preview_enabled {
                        if let Some(recycled) =
                            app.set_preview_in(frame.bgra, frame.width, frame.height, Some(window))
                        {
                            rt.return_buffer(recycled);
                        }
                    } else {
                        rt.return_buffer(frame.bgra);
                    }
                }
                cx.notify();
            });
            if updated.is_err() {
                break;
            }

            if state_changed {
                last_state_version = current_version;
            }
        }
    })
    .detach();
}

/// Performs whatever the user clicked.
fn start_effect_pump(
    cx: &mut App,
    window: WindowHandle<AppView>,
    view: gpui::Entity<AppView>,
    runtime: std::sync::Arc<Runtime>,
    hwnd: Option<isize>,
) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(30)).await;

            let effects = phonecam_ui::drain_effects();
            if effects.is_empty() {
                continue;
            }

            for effect in effects {
                apply_effect(&runtime, effect, &window, &view, hwnd, cx);
            }
        }
    })
    .detach();
}

fn open_platform_tools_url(runtime: &Runtime) {
    const URL: &str = "https://developer.android.com/tools/releases/platform-tools";

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if let Err(err) = std::process::Command::new("cmd")
            .args(["/C", "start", "", URL])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
        {
            runtime.log_error(
                "open_url_failed",
                &format!("could not open platform-tools download page: {err}"),
            );
        }
    }

    #[cfg(not(windows))]
    {
        runtime.log_info(&format!("Open {URL} to install Android platform-tools"));
    }
}

fn apply_effect(
    runtime: &std::sync::Arc<Runtime>,
    effect: SideEffect,
    window: &WindowHandle<AppView>,
    view: &gpui::Entity<AppView>,
    hwnd: Option<isize>,
    cx: &mut gpui::AsyncApp,
) {
    match effect {
        SideEffect::ArmPairing => {
            runtime.arm_pairing();
        }
        SideEffect::DisarmPairing => runtime.disarm_pairing(),
        SideEffect::ResolvePair(approved) => runtime.resolve_pair_request(approved),
        SideEffect::StartStream => runtime.start_stream(),
        SideEffect::StopStream => runtime.stop_stream(),
        SideEffect::SetVcam(enabled) => runtime.set_vcam(enabled),
        SideEffect::RestartVcam => runtime.restart_vcam(),
        SideEffect::RefreshState => runtime.broadcast_state(),
        SideEffect::RefreshUsb => runtime.refresh_usb(),
        SideEffect::ConnectUsb(serial) => runtime.connect_usb_device(serial),
        SideEffect::DisconnectUsb => runtime.disconnect_usb(),
        SideEffect::RestartAdb => runtime.restart_adb(),
        SideEffect::OpenPlatformToolsUrl => open_platform_tools_url(runtime),
        SideEffect::ForgetDevice => {
            let id = runtime.state.lock().settings.last_device_id.clone();
            if let Some(id) = id {
                runtime.pairing.lock().forget_device(&id);
                runtime.registry.lock().retain(|r| r.device_id != id);
                let snapshot = {
                    let mut state = runtime.state.lock();
                    state.settings.last_device_id = None;
                    state.clone()
                };
                let _ = settings::save(&snapshot.settings);
                runtime.bump_state_version();
            }
        }
        SideEffect::SetTab(tab) => {
            view.update(cx, |app, cx| {
                app.tab = tab;
                cx.notify();
            })
            .ok();
        }
        SideEffect::ToggleFocusPreview => {
            view.update(cx, |app, cx| {
                app.focus_preview = !app.focus_preview;
                cx.notify();
            })
            .ok();
        }
        SideEffect::SetPreviewEnabled(enabled) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.preview_enabled = enabled;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            runtime.set_preview_enabled(enabled);
            if !enabled {
                window
                    .update(cx, |app, window, cx| {
                        if let Some(recycled) = app.clear_preview_in(Some(window)) {
                            runtime.return_buffer(recycled);
                        }
                        cx.notify();
                    })
                    .ok();
            }
        }
        SideEffect::SetMirror(mirror) => {
            let rotation = runtime.state.lock().settings.output_rotation;
            runtime.sync_output_transform(mirror, rotation);
        }
        SideEffect::SetRotation(rotation) => {
            let mirror = runtime.state.lock().settings.output_mirror;
            runtime.sync_output_transform(mirror, rotation);
        }
        SideEffect::SetOutputSize(width, height) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.output_width = width;
                state.settings.output_height = height;
                state.settings.output_follows_stream = false;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            // The buffer is sized by this setting, so it has to be rebuilt.
            *runtime.writer.lock() = None;
            runtime.broadcast_state();
        }
        SideEffect::SetOutputFollowsStream(enabled) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.output_follows_stream = enabled;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            *runtime.writer.lock() = None;
            runtime.broadcast_state();
        }
        SideEffect::SetCamera(camera) => runtime.set_camera(camera),
        SideEffect::SetResolution(resolution) => {
            let (fps, bitrate) = {
                let state = runtime.state.lock();
                let fps_opt = (state.settings.fps > 0).then_some(state.settings.fps);
                let prev_default = phonecam_ui::views::bitrate_for(
                    &state.settings.resolution,
                    state.settings.fps,
                    state.settings.bitrate,
                );
                let next_bitrate = if state.settings.bitrate == prev_default {
                    phonecam_ui::views::bitrate_for(
                        &resolution,
                        state.settings.fps,
                        state.settings.bitrate,
                    )
                } else {
                    state.settings.bitrate
                };
                (fps_opt, next_bitrate)
            };
            runtime.set_video(resolution, fps, bitrate);
        }
        SideEffect::SetVideo { fps, bitrate } => {
            let resolution = runtime.state.lock().settings.resolution.clone();
            runtime.set_video(resolution, fps, bitrate);
        }
        SideEffect::SetQualityMode(auto) => runtime.set_quality_mode(auto),
        SideEffect::SetScreenMode(mode) => runtime.set_screen_mode(mode),
        SideEffect::SetTorch(enabled) => runtime.set_torch(enabled),
        SideEffect::SetZoom(zoom) => runtime.set_zoom(zoom),
        SideEffect::SetLogLevel(level) => {
            logging::set_level(level);
            runtime.set_log_level(level);
        }
        SideEffect::SetLogFilterLevel(level) => {
            runtime.state.lock().log_filter_level = level;
            runtime.bump_state_version();
        }
        SideEffect::SetLogFilterSource(source) => {
            runtime.state.lock().log_filter_source = source;
            runtime.bump_state_version();
        }
        SideEffect::DismissNotice(id) => {
            runtime.state.lock().dismiss_notice_by_id(id);
            runtime.bump_state_version();
        }
        SideEffect::ClearLogs => {
            runtime.state.lock().logs.clear();
            runtime.bump_state_version();
        }
        SideEffect::ToggleAlwaysOnTop => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.always_on_top = !state.settings.always_on_top;
                state.clone()
            };
            if let Some(hwnd) = hwnd {
                tray::set_window_always_on_top(hwnd, snapshot.settings.always_on_top);
            }
            let _ = settings::save(&snapshot.settings);
            runtime.bump_state_version();
        }
        SideEffect::ExportDiagnostics => {
            let custom_path = match tray::save_diagnostics_dialog(hwnd) {
                tray::SaveDialogResult::Selected(path) => Some(path),
                tray::SaveDialogResult::Cancelled => {
                    runtime
                        .state
                        .lock()
                        .set_toast("Diagnostics export cancelled", Timing::TOAST_MS as i64);
                    runtime.bump_state_version();
                    return;
                }
                tray::SaveDialogResult::Unavailable => None,
            };
            match runtime.export_diagnostics(custom_path.as_deref()) {
                Ok(path) => {
                    runtime.log_info(&format!("diagnostics exported to {}", path.display()));
                    let path_str = path.to_string_lossy().to_string();
                    let _ = cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(path_str)));
                    runtime.state.lock().set_toast(
                        format!("Diagnostics exported to {}", path.display()),
                        Timing::TOAST_MS as i64,
                    );
                    runtime.bump_state_version();
                }
                Err(err) => {
                    runtime.log_error(
                        "diagnostics_failed",
                        &format!("could not export diagnostics: {err}"),
                    );
                }
            }
        }
        SideEffect::CopyDiagnosticsReport => {
            let report = runtime.generate_diagnostics_report();
            let copied = cx
                .update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(report)))
                .is_ok();
            {
                let mut state = runtime.state.lock();
                if copied {
                    state.set_toast(
                        "Diagnostics report copied to clipboard",
                        Timing::TOAST_MS as i64,
                    );
                } else {
                    state.set_toast(
                        "Could not copy diagnostics report",
                        Timing::TOAST_MS as i64,
                    );
                }
            }
            runtime.bump_state_version();
        }
        SideEffect::SelectPairingNetwork(index) => {
            view.update(cx, |app, cx| {
                app.pairing_network = index;
                cx.notify();
            })
            .ok();
        }
        SideEffect::CopyPairingPayload => {
            let selected_network = view.update(cx, |app, _| app.pairing_network).unwrap_or(0);
            let payload = {
                let state = runtime.state.lock();
                let payloads = state
                    .core
                    .as_ref()
                    .map(|c| c.pairing.qr_payloads.as_slice())
                    .unwrap_or(&[]);
                if payloads.is_empty() {
                    None
                } else {
                    Some(payloads[selected_network.min(payloads.len() - 1)].clone())
                }
            };
            let copied = payload
                .filter(|p| !p.is_empty())
                .map(|p| {
                    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(p)))
                        .is_ok()
                })
                .unwrap_or(false);
            {
                let mut state = runtime.state.lock();
                if copied {
                    state.set_toast("Pairing code copied to clipboard", Timing::TOAST_MS as i64);
                } else {
                    state.set_toast("No active pairing code to copy", Timing::TOAST_MS as i64);
                }
            }
            runtime.bump_state_version();
        }
    }
}

fn persist_window_geometry(window: &gpui::Window, runtime: &Runtime) {
    let (x, y, width, height, maximized) = match window.window_bounds() {
        WindowBounds::Windowed(b) => (
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
            false,
        ),
        WindowBounds::Maximized(b) => (
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
            true,
        ),
        WindowBounds::Fullscreen(b) => (
            f32::from(b.origin.x),
            f32::from(b.origin.y),
            f32::from(b.size.width),
            f32::from(b.size.height),
            false,
        ),
    };
    if width < 100.0
        || height < 100.0
        || !(-10_000.0..=20_000.0).contains(&x)
        || !(-10_000.0..=20_000.0).contains(&y)
    {
        return;
    }

    let updated = {
        let mut state = runtime.state.lock();
        let s = &mut state.settings;
        let changed = s.window_x != Some(x)
            || s.window_y != Some(y)
            || (s.window_width - width).abs() > 0.5
            || (s.window_height - height).abs() > 0.5
            || s.window_maximized != maximized;
        if !changed {
            None
        } else {
            s.window_x = Some(x);
            s.window_y = Some(y);
            s.window_width = width;
            s.window_height = height;
            s.window_maximized = maximized;
            Some(s.clone())
        }
    };
    if let Some(settings) = updated {
        let _ = settings::save(&settings);
    }
}

fn handle_tray_command(
    command: tray::TrayCommand,
    window: &WindowHandle<AppView>,
    view: &gpui::Entity<AppView>,
    runtime: &Runtime,
    cx: &mut gpui::AsyncApp,
) {
    match command {
        tray::TrayCommand::ShowWindow => {
            let tray = runtime.tray.lock();
            if let Some(tray) = tray.as_ref() {
                tray.show_target();
            }
            runtime.set_window_visible(true);
        }
        tray::TrayCommand::ToggleVcam => {
            let enabled = !runtime.vcam_enabled.load(Ordering::Relaxed);
            phonecam_ui::push_effect(SideEffect::SetVcam(enabled));
        }
        tray::TrayCommand::ShowPairing => {
            let _ = view.update(cx, |app, cx| {
                app.tab = Tab::Setup;
                cx.notify();
            });
            phonecam_ui::push_effect(SideEffect::ArmPairing);
            let tray = runtime.tray.lock();
            if let Some(tray) = tray.as_ref() {
                tray.show_target();
            }
            runtime.set_window_visible(true);
        }
        tray::TrayCommand::Quit => {
            let _ = window.update(cx, |_, window, _| {
                persist_window_geometry(window, runtime);
            });
            runtime.log_info("quit from the tray");
            let _ = cx.update(|cx| cx.quit());
        }
        tray::TrayCommand::OpenMenu => {
            // The menu is built from the state as it is now, so a
            // tick in it cannot disagree with the window.
            let state = runtime.state.lock().clone();
            let menu = tray::build_menu(&state);
            let choice = {
                let tray = runtime.tray.lock();
                tray.as_ref().and_then(|tray| tray.show_menu(&menu))
            };
            if let Some(choice) = choice {
                if choice != tray::TrayCommand::OpenMenu {
                    handle_tray_command(choice, window, view, runtime, cx);
                }
            }
        }
    }
}

/// Acts on whatever the user clicked in the tray.
///
/// The click arrives as a window message on a thread this module owns, so it is
/// recorded and drained here rather than handled where it lands: the window, the
/// settings file and the view all live on the UI thread.
fn start_tray_pump(
    cx: &mut App,
    window: WindowHandle<AppView>,
    view: gpui::Entity<AppView>,
    runtime: std::sync::Arc<Runtime>,
) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(100)).await;

            let commands: Vec<tray::TrayCommand> = {
                let tray = runtime.tray.lock();
                match tray.as_ref() {
                    Some(tray) => tray.drain(),
                    None => Vec::new(),
                }
            };
            if commands.is_empty() {
                continue;
            }

            for command in commands {
                handle_tray_command(command, &window, &view, &runtime, cx);
            }
        }
    })
    .detach();
}
/// Installs the tray icon and makes closing the window hide it.
///
/// Closing must not quit: the virtual camera keeps publishing and other apps hold
/// the device, so a closed window would silently kill every consumer's camera.
/// Quit lives in the tray, which is the only place it can be deliberate.
fn start_tray(
    cx: &mut App,
    window: gpui::WindowHandle<AppView>,
    view: gpui::Entity<AppView>,
    runtime: std::sync::Arc<Runtime>,
) {
    // The HWND and the close hook both need a `Window`, which only an App-level
    // window update can supply.
    let hwnd = window
        .update(cx, |_, window, _| tray::hwnd_of(window))
        .unwrap_or(None);

    let Some(hwnd) = hwnd else {
        runtime.log_info("no window handle; the tray is unavailable");
        return;
    };

    let state = runtime.state.lock().clone();
    let installed = tray::Tray::install(hwnd, &tray::tooltip(&state));
    match &installed {
        Some(_) => runtime.log_info("tray icon installed"),
        None => runtime.log_info("the tray icon could not be installed"),
    }
    *runtime.tray.lock() = installed;

    // Closing hides the window instead of quitting, so the camera keeps running.
    // The tray's Quit is the only deliberate exit.
    let close_runtime = runtime.clone();
    window
        .update(cx, |_, window, cx| {
            window.on_window_should_close(cx, move |window, _cx| {
                tracing::info!("window close requested; hiding instead of quitting");
                persist_window_geometry(window, &close_runtime);
                tray::hide_window(hwnd);
                close_runtime.set_window_visible(false);
                // Refuse the close: the window is hidden, not destroyed, so the
                // camera and the listener keep running.
                false
            });
        })
        .ok();

    let _ = view;
}
/// The window title, which names the phone so a screenshot is self-describing.
pub fn title_for(state: &AppState) -> String {
    phonecam_ui::views::window_title(state)
}

/// The settings the app starts with.
pub fn initial_settings() -> Settings {
    settings::load()
}

/// Logs one line at startup, so a fresh install's diagnostics tab is not empty.
pub fn greet(state: &mut AppState) {
    state.push_log(LogLevel::Info, LogSource::App, "PhoneCam ready");
}

/// The tab a fresh window opens on.
pub fn default_tab() -> Tab {
    Tab::Setup
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_window_opens_on_setup() {
        assert_eq!(default_tab(), Tab::Setup);
    }

    #[test]
    fn the_startup_greeting_is_in_the_log() {
        let mut state = AppState::default();
        greet(&mut state);
        assert!(state.logs.iter().any(|l| l.message.contains("ready")));
    }

    #[test]
    fn the_title_names_the_phone_when_there_is_one() {
        assert_eq!(title_for(&AppState::default()), "PhoneCam");
    }

    #[test]
    fn default_keybindings_are_defined() {
        let bindings = phonecam_ui::default_keybindings();
        assert!(!bindings.is_empty());
    }
}












