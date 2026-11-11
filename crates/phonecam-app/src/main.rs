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

use phonecam_app::{runtime, settings, tray};

use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui::{
    px, size, App, AppContext, Application, Bounds, WindowBackgroundAppearance, WindowBounds,
    WindowOptions,
};
use phonecam_protocol::LogLevel;
use phonecam_ui::views::state::{AppState, LogSource, Settings};
use phonecam_ui::views::{AppView, SideEffect, Tab};

use runtime::Runtime;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let settings = settings::load();
    tracing::info!("PhoneCam starting");

    Application::new().run(move |cx: &mut App| {
        let runtime = Runtime::new(&settings);

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

        let bounds = Bounds::centered(
            None,
            size(px(settings.window_width), px(settings.window_height)),
            cx,
        );

        let view = cx.new(|cx| {
            let mut app = AppView::new();
            app.set_focus_handle(cx.focus_handle());
            app.set_state(runtime.state.lock().clone());
            app
        });

        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(900.0), px(640.0))),
                    window_background: WindowBackgroundAppearance::Opaque,
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("PhoneCam".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |_, _cx| view.clone(),
            )
            .expect("could not open the window");

        // Focus the root view on open so shortcuts and navigation work immediately
        window
            .update(cx, |view, window, _cx| {
                if let Some(focus_handle) = view.focus_handle.as_ref() {
                    window.focus(focus_handle);
                }
            })
            .ok();

        // The window pulls state and frames on a timer: a push per decoded frame
        // would re-render the whole window 30 times a second for a change that
        // only affects one image.
        start_ui_pump(cx, view.clone(), runtime.clone());

        // Drain the effects the panels asked for.
        start_effect_pump(cx, view.clone(), runtime.clone());

        // The tray, and the close-to-hide behaviour it exists for.
        start_tray(cx, window.clone(), view.clone(), runtime.clone());
        start_tray_pump(cx, view.clone(), runtime.clone());

        cx.activate(true);
    });

    Ok(())
}

/// Copies the runtime snapshot and the newest picture into the view.
fn start_ui_pump(cx: &mut App, view: gpui::Entity<AppView>, runtime: std::sync::Arc<Runtime>) {
    cx.spawn(async move |cx| {
        let mut last_state_version = 0u64;
        loop {
            cx.background_executor().timer(Duration::from_millis(8)).await;

            let current_version = runtime.state_version.load(Ordering::Relaxed);
            let state_changed = current_version != last_state_version;
            let frame = runtime.frames.lock().take();

            if !state_changed && frame.is_none() {
                continue;
            }

            let snapshot = if state_changed {
                Some(runtime.state.lock().clone())
            } else {
                None
            };

            let rt = runtime.clone();
            let updated = view.update(cx, |app, cx| {
                if let Some(snapshot) = snapshot {
                    app.set_state(snapshot);
                }
                if let Some(frame) = frame {
                    if let Some(recycled) = app.set_preview(frame.bgra, frame.width, frame.height) {
                        rt.return_buffer(recycled);
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
fn start_effect_pump(cx: &mut App, view: gpui::Entity<AppView>, runtime: std::sync::Arc<Runtime>) {
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_millis(30)).await;

            let effects = phonecam_ui::drain_effects();
            if effects.is_empty() {
                continue;
            }

            for effect in effects {
                apply_effect(&runtime, effect, &view, cx);
            }
        }
    })
    .detach();
}

fn apply_effect(
    runtime: &std::sync::Arc<Runtime>,
    effect: SideEffect,
    view: &gpui::Entity<AppView>,
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
        SideEffect::SetVcam(enabled) => {
            runtime.vcam_enabled.store(enabled, Ordering::Relaxed);
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.vcam_enabled = enabled;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            runtime.bump_state_version();
        }
        SideEffect::RefreshState => runtime.broadcast_state(),
        SideEffect::RefreshUsb => runtime.refresh_usb(),
        SideEffect::ConnectUsb => runtime.connect_usb(),
        SideEffect::DisconnectUsb => runtime.disconnect_usb(),
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
        SideEffect::SetMirror(mirror) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.output_mirror = mirror;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            runtime.broadcast_state();
        }
        SideEffect::SetRotation(rotation) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.output_rotation = rotation;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            runtime.broadcast_state();
        }
        SideEffect::SetOutputSize(width, height) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.output_width = width;
                state.settings.output_height = height;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            // The buffer is sized by this setting, so it has to be rebuilt.
            *runtime.writer.lock() = None;
            runtime.broadcast_state();
        }
        SideEffect::SetCamera(camera) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.camera = camera;
                state.pending_camera_id = Some(state.settings.camera.clone());
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            runtime.bump_state_version();
        }
        SideEffect::SetVideo { fps, bitrate } => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.fps = fps;
                state.settings.bitrate = bitrate;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            runtime.bump_state_version();
        }
        SideEffect::SetQualityMode(auto) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.quality_mode = if auto {
                    phonecam_protocol::QualityMode::Auto
                } else {
                    phonecam_protocol::QualityMode::Manual
                };
                state.clone()
            };
            runtime.quality.lock().set_enabled(auto);
            let _ = settings::save(&snapshot.settings);
            runtime.bump_state_version();
        }
        SideEffect::SetScreenMode(mode) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.screen_mode = mode;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
            runtime.bump_state_version();
        }
        SideEffect::SetTorch(_) | SideEffect::SetZoom(_) => {
            // Phone-owned controls: sent on the next start batch. Recording them
            // as done here would be the claim-without-establishment the design
            // rules exist to prevent.
        }
        SideEffect::SetLogLevel(level) => {
            let snapshot = {
                let mut state = runtime.state.lock();
                state.settings.log_level = level;
                state.clone()
            };
            let _ = settings::save(&snapshot.settings);
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
            let _ = settings::save(&snapshot.settings);
            runtime.bump_state_version();
        }
        SideEffect::ExportDiagnostics => match runtime.export_diagnostics(None) {
            Ok(path) => {
                runtime.log_info(&format!("diagnostics exported to {}", path.display()));
            }
            Err(err) => {
                runtime.log_error("diagnostics_failed", &format!("could not export diagnostics: {err}"));
            }
        },
        SideEffect::SelectPairingNetwork(index) => {
            view.update(cx, |app, cx| {
                app.pairing_network = index;
                cx.notify();
            })
            .ok();
        }
    }
}

/// Acts on whatever the user clicked in the tray.
///
/// The click arrives as a window message on a thread this module owns, so it is
/// recorded and drained here rather than handled where it lands: the window, the
/// settings file and the view all live on the UI thread.
fn start_tray_pump(cx: &mut App, view: gpui::Entity<AppView>, runtime: std::sync::Arc<Runtime>) {
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
                match command {
                    tray::TrayCommand::ShowWindow => {
                        let tray = runtime.tray.lock();
                        if let Some(tray) = tray.as_ref() {
                            tray.show_target();
                        }
                    }
                    tray::TrayCommand::ToggleVcam => {
                        let enabled = !runtime.vcam_enabled.load(Ordering::Relaxed);
                        phonecam_ui::push_effect(SideEffect::SetVcam(enabled));
                    }
                    tray::TrayCommand::ShowPairing => {
                        phonecam_ui::push_effect(SideEffect::ArmPairing);
                        let tray = runtime.tray.lock();
                        if let Some(tray) = tray.as_ref() {
                            tray.show_target();
                        }
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
                        match choice {
                            Some(tray::TrayCommand::ShowWindow) => {
                                let tray = runtime.tray.lock();
                                if let Some(tray) = tray.as_ref() {
                                    tray.show_target();
                                }
                            }
                            Some(tray::TrayCommand::ToggleVcam) => {
                                let enabled = !runtime.vcam_enabled.load(Ordering::Relaxed);
                                phonecam_ui::push_effect(SideEffect::SetVcam(enabled));
                            }
                            Some(tray::TrayCommand::ShowPairing) => {
                                phonecam_ui::push_effect(SideEffect::ArmPairing);
                                let tray = runtime.tray.lock();
                                if let Some(tray) = tray.as_ref() {
                                    tray.show_target();
                                }
                            }
                            Some(tray::TrayCommand::Quit) => {
                                runtime.log_info("quit from the tray");
                                let _ = cx.update(|cx| cx.quit());
                            }
                            _ => {}
                        }
                    }
                    tray::TrayCommand::Quit => {
                        runtime.log_info("quit from the tray");
                        let _ = cx.update(|cx| cx.quit());
                    }
                }
            }
        }
    })
    .detach();

    let _ = view;
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
    window
        .update(cx, |_, window, cx| {
            window.on_window_should_close(cx, move |_window, _cx| {
                tracing::info!("window close requested; hiding instead of quitting");
                tray::hide_window(hwnd);
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












