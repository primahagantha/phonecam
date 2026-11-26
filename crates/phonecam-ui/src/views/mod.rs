//! The window: shell, panels, and the pure rules they read.

pub mod actions;
pub mod effects;
pub mod panels;
pub mod preview;
pub mod state;
pub mod widgets;

pub use actions::*;
pub use effects::{drain_effects, push_effect, SideEffect, Tab};
pub use state::{
    bitrate_for, camera_facing_label, format_battery, format_bytes, format_duration,
    format_log_time, now_ms, screen_mode_label, stream_phase_label, thermal_label, AppState,
    CoreStatus, FailureCode, LogLine, LogSource, Notice, NoticeAction, PairingPrompt, ReleaseInfo,
    Settings, UpdateStatus,
};

use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, Context, IntoElement, Render, Window};

use crate::theme::{Fonts, Radius, Space, Text, Type, SHELL};
use preview::StageContent;

/// The root view.
pub struct AppView {
    pub state: AppState,
    pub tab: Tab,
    /// F11 gives the picture the whole window, because that is what an operator
    /// framing a shot actually wants.
    pub focus_preview: bool,
    /// Top-bar Preferences drawer open/closed state (UI-local).
    pub show_preferences: bool,
    /// Software Updates modal open/closed state (UI-local).
    pub show_updates: bool,
    /// Setup -> USB Debugging Help accordion expanded/collapsed state (default collapsed).
    pub usb_help_expanded: bool,
    /// Camera -> Experimental (Labs) card expanded/collapsed state (default collapsed).
    pub labs_expanded: bool,
    /// Labs -> Supported gestures reference collapsed/expanded state.
    pub gesture_ref_expanded: bool,
    /// Auto-arm QR pairing once on first run when no device has paired yet.
    auto_armed_first_run: bool,
    /// Which network's code the user picked, when the PC is on several.
    pub pairing_network: usize,
    /// The one image handle for the picture.
    ///
    /// Reused across frames: a fresh texture 30 times a second is wasted
    /// allocation, and the shipped app's equivalent rule was "exactly one
    /// canvas, never remounted".
    pub preview_image: Option<std::sync::Arc<gpui::RenderImage>>,
    /// Double-buffered preview handle held for one extra frame so the GPU texture
    /// atlas eviction can safely drop the frame from 2 frames ago without flickering.
    pub previous_preview_image: Option<std::sync::Arc<gpui::RenderImage>>,
    /// The pairing code, regenerated only when its payload changes.
    pub qr_image: Option<std::sync::Arc<gpui::RenderImage>>,
    qr_payload_cached: Option<String>,
    pub focus_handle: Option<gpui::FocusHandle>,
    focus_handles: HashMap<String, gpui::FocusHandle>,
    pub scroll_handle: gpui::ScrollHandle,
    pub scroll_handles: [gpui::ScrollHandle; 4],
}

impl AppView {
    pub fn new() -> Self {
        Self {
            state: AppState::default(),
            tab: Tab::Setup,
            focus_preview: false,
            show_preferences: false,
            show_updates: false,
            usb_help_expanded: false,
            labs_expanded: false,
            gesture_ref_expanded: false,
            auto_armed_first_run: false,
            pairing_network: 0,
            preview_image: None,
            previous_preview_image: None,
            qr_image: None,
            qr_payload_cached: None,
            focus_handle: None,
            focus_handles: HashMap::new(),
            scroll_handle: gpui::ScrollHandle::new(),
            scroll_handles: [
                gpui::ScrollHandle::new(),
                gpui::ScrollHandle::new(),
                gpui::ScrollHandle::new(),
                gpui::ScrollHandle::new(),
            ],
        }
    }

    pub fn set_focus_handle(&mut self, handle: gpui::FocusHandle) {
        self.focus_handle = Some(handle);
    }

    /// Space shortcut: starts/stops streaming or shows toast when no device connected.
    pub fn toggle_stream(&mut self) {
        if self.state.is_streaming() {
            push_effect(SideEffect::StopStream);
        } else if self.state.has_device() {
            push_effect(SideEffect::StartStream);
        } else {
            self.state.toast = Some("Connect a phone before starting the camera".into());
        }
    }

    /// F11 shortcut: full-window preview toggle.
    pub fn toggle_focus_preview(&mut self) {
        self.focus_preview = !self.focus_preview;
    }

    /// Ctrl+1..4 shortcuts: switch tab.
    pub fn set_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// Replaces the state and re-derives anything that depends on it.
    pub fn set_state(&mut self, state: AppState) {
        // A fresh notice that needs action brings Setup forward, so the way out
        // is on screen rather than described. Only ever moves to Setup, which is
        // the one tab that resolves an empty state, and never back.
        if state
            .notices
            .iter()
            .any(|n| n.code == FailureCode::NoDevice)
            && !state.has_device()
        {
            self.tab = Tab::Setup;
        }
        // First-run onboarding: auto-arm QR pairing once when connected to core
        // with no previously paired phone so users get an immediate QR code.
        let pairing_open = state.core.as_ref().is_some_and(|c| c.pairing.open);
        if state.status.connected
            && !state.has_device()
            && state.settings.last_device_id.is_none()
            && !pairing_open
            && !self.auto_armed_first_run
        {
            self.auto_armed_first_run = true;
            push_effect(SideEffect::ArmPairing);
        }
        self.state = state;
    }

    /// Installs a decoded picture. The pixels are BGRA.
    ///
    /// Returns `None` because `gpui::RenderImage` owns its pixel frames safely.
    pub fn set_preview(&mut self, pixels: Vec<u8>, width: u32, height: u32) -> Option<Vec<u8>> {
        self.set_preview_in(pixels, width, height, None)
    }

    /// Installs a decoded picture into the preview slot and drops the GPU atlas
    /// texture from two frames ago so VRAM remains bounded without unsafe aliasing.
    pub fn set_preview_in(
        &mut self,
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        mut window: Option<&mut Window>,
    ) -> Option<Vec<u8>> {
        if let Some(frame_2_ago) = self.previous_preview_image.take() {
            if let Some(w) = window.as_mut() {
                let _ = w.drop_image(frame_2_ago.clone());
            }
            drop(frame_2_ago);
        }

        self.previous_preview_image = self.preview_image.take();

        if let Some(buffer) = image::RgbaImage::from_raw(width, height, pixels) {
            self.preview_image = Some(std::sync::Arc::new(gpui::RenderImage::new(vec![
                image::Frame::new(buffer),
            ])));
        }

        None
    }

    pub fn clear_preview(&mut self) -> Option<Vec<u8>> {
        self.clear_preview_in(None)
    }

    /// Clears both active and previous preview images, dropping GPU textures from `window`.
    pub fn clear_preview_in(&mut self, mut window: Option<&mut Window>) -> Option<Vec<u8>> {
        if let Some(prev) = self.previous_preview_image.take() {
            if let Some(w) = window.as_mut() {
                let _ = w.drop_image(prev.clone());
            }
            drop(prev);
        }

        if let Some(curr) = self.preview_image.take() {
            if let Some(w) = window.as_mut() {
                let _ = w.drop_image(curr.clone());
            }
            drop(curr);
        }

        None
    }

    /// Renders the pairing code once per payload, not once per frame.
    fn ensure_qr(&mut self, payload: &str, window: Option<&mut Window>) {
        if self.qr_payload_cached.as_deref() == Some(payload) && self.qr_image.is_some() {
            return;
        }
        if let Some(old_qr) = self.qr_image.take() {
            if let Some(w) = window {
                let _ = w.drop_image(old_qr);
            }
        }
        self.qr_image =
            preview::qr_pixels(payload, crate::theme::QR_SIZE).and_then(|(buffer, w, h)| {
                image::RgbaImage::from_raw(w as u32, h as u32, buffer).map(|image| {
                    std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(image)]))
                })
            });
        self.qr_payload_cached = Some(payload.to_string());
    }
}

impl Default for AppView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // GPUI re-rendering is driven on demand via cx.notify() when new frames or state updates arrive,
        // avoiding continuous spinning at 144Hz.
        let focus_handle = self
            .focus_handle
            .get_or_insert_with(|| cx.focus_handle())
            .clone();

        let content = preview::stage_content(&self.state, self.pairing_network);
        if let StageContent::Pairing { payload, .. } = &content {
            if !payload.is_empty() {
                self.ensure_qr(payload, Some(window));
            }
        }

        let viewport = window.viewport_size();
        let narrow = f32::from(viewport.width) < crate::theme::NARROW_WINDOW;
        let viewport_height = f32::from(viewport.height);

        let view = cx.entity();
        let on_toggle_preferences: Rc<dyn Fn(&mut Window, &mut gpui::App) + 'static> = {
            let view = view.clone();
            Rc::new(move |_window, app| {
                view.update(app, |this, cx| {
                    this.show_preferences = !this.show_preferences;
                    if this.show_preferences {
                        this.show_updates = false;
                    }
                    cx.notify();
                });
            })
        };
        let on_open_preferences: Box<dyn Fn(&mut Window, &mut gpui::App) + 'static> = {
            let view = view.clone();
            Box::new(move |_window, app| {
                view.update(app, |this, cx| {
                    this.show_preferences = true;
                    this.show_updates = false;
                    cx.notify();
                });
            })
        };
        let on_close_preferences: Rc<dyn Fn(&mut Window, &mut gpui::App) + 'static> = {
            let view = view.clone();
            Rc::new(move |_window, app| {
                view.update(app, |this, cx| {
                    this.show_preferences = false;
                    cx.notify();
                });
            })
        };
        let on_toggle_updates: Rc<dyn Fn(&mut Window, &mut gpui::App) + 'static> = {
            let view = view.clone();
            Rc::new(move |_window, app| {
                view.update(app, |this, cx| {
                    this.show_updates = !this.show_updates;
                    if this.show_updates {
                        this.show_preferences = false;
                        if matches!(this.state.update_status, UpdateStatus::Idle) {
                            push_effect(SideEffect::CheckForUpdates);
                        }
                    }
                    cx.notify();
                });
            })
        };
        let on_open_updates: Rc<dyn Fn(&mut Window, &mut gpui::App) + 'static> = {
            let view = view.clone();
            Rc::new(move |_window, app| {
                view.update(app, |this, cx| {
                    this.show_updates = true;
                    this.show_preferences = false;
                    push_effect(SideEffect::CheckForUpdates);
                    cx.notify();
                });
            })
        };
        let on_close_updates: Rc<dyn Fn(&mut Window, &mut gpui::App) + 'static> = {
            let view = view.clone();
            Rc::new(move |_window, app| {
                view.update(app, |this, cx| {
                    this.show_updates = false;
                    cx.notify();
                });
            })
        };
        let on_toggle_usb_help: Rc<dyn Fn(&mut Window, &mut gpui::App) + 'static> = {
            let view = view.clone();
            Rc::new(move |_window, app| {
                view.update(app, |this, cx| {
                    this.usb_help_expanded = !this.usb_help_expanded;
                    cx.notify();
                });
            })
        };
        let on_toggle_labs: Rc<dyn Fn(&mut Window, &mut gpui::App) + 'static> = {
            let view = view.clone();
            Rc::new(move |_window, app| {
                view.update(app, |this, cx| {
                    this.labs_expanded = !this.labs_expanded;
                    cx.notify();
                });
            })
        };
        let on_toggle_gesture_ref: Rc<dyn Fn(&mut Window, &mut gpui::App) + 'static> = {
            let view = view.clone();
            Rc::new(move |_window, app| {
                view.update(app, |this, cx| {
                    this.gesture_ref_expanded = !this.gesture_ref_expanded;
                    cx.notify();
                });
            })
        };

        let ui_state = panels::RailUiState {
            show_preferences: self.show_preferences,
            show_updates: self.show_updates,
            usb_help_expanded: self.usb_help_expanded,
            labs_expanded: self.labs_expanded,
            gesture_ref_expanded: self.gesture_ref_expanded,
        };
        let callbacks = panels::PanelCallbacks {
            on_toggle_preferences: Some(on_toggle_preferences),
            on_toggle_updates: Some(on_toggle_updates),
            on_open_updates: Some(on_open_updates),
            on_toggle_usb_help: Some(on_toggle_usb_help),
            on_toggle_labs: Some(on_toggle_labs),
            on_toggle_gesture_ref: Some(on_toggle_gesture_ref),
        };

        let mut focus_handles = std::mem::take(&mut self.focus_handles);
        let (status_strip_el, stage, rail_el, prefs_overlay_el, updates_overlay_el) = {
            let mut focus_ctx = widgets::FocusContext::new(&mut focus_handles, cx);

            let status_strip_el = panels::status_strip_with_ui(
                &self.state,
                ui_state,
                &callbacks,
                Some(&mut focus_ctx),
            );

            let stage: gpui::AnyElement = match &content {
                StageContent::Picture => {
                    let picture: gpui::AnyElement = if !self.state.settings.preview_enabled {
                        preview::preview_off_card_with_action(
                            Some(on_open_preferences),
                            Some(&mut focus_ctx),
                        )
                    } else {
                        match &self.preview_image {
                            Some(image) => div()
                                .flex_1()
                                .min_h(px(0.0))
                                .child(
                                    gpui::img(image.clone())
                                        .w_full()
                                        .h_full()
                                        .object_fit(gpui::ObjectFit::Contain),
                                )
                                .into_any_element(),
                            None => preview::preview_waiting_card_with_focus(
                                self.state.device_name().as_deref(),
                                Some(&mut focus_ctx),
                            ),
                        }
                    };
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h(px(0.0))
                        .min_w(px(0.0))
                        .rounded(px(Radius::LG))
                        .border_1()
                        .border_color(SHELL.border)
                        .bg(SHELL.stage)
                        .overflow_hidden()
                        .child(picture)
                        .child(preview::stage_gesture_feedback(&self.state))
                        .child(preview::live_studio_hud(
                            &self.state,
                            self.focus_preview,
                            Some(&mut focus_ctx),
                        ))
                        .into_any_element()
                }
                StageContent::Pairing { .. } => preview::pairing_stage_with_context(
                    &content,
                    self.qr_image.as_ref(),
                    viewport_height,
                    Some(&mut focus_ctx),
                ),
                StageContent::Empty { .. } => {
                    preview::stage_frame_with_focus(&content, Some(&mut focus_ctx))
                }
            };

            let rail_el = if !self.focus_preview {
                let handle = &self.scroll_handles[self.tab.index()];
                Some(panels::rail_with_ui(
                    &self.state,
                    self.tab,
                    narrow,
                    handle,
                    ui_state,
                    &callbacks,
                    Some(&mut focus_ctx),
                ))
            } else {
                None
            };

            let prefs_overlay_el = if self.show_preferences {
                Some(panels::preferences_drawer(
                    &self.state,
                    Some(on_close_preferences),
                    Some(&mut focus_ctx),
                ))
            } else {
                None
            };

            let updates_overlay_el = if self.show_updates {
                Some(panels::updates_modal(
                    &self.state,
                    Some(on_close_updates),
                    Some(&mut focus_ctx),
                ))
            } else {
                None
            };

            (
                status_strip_el,
                stage,
                rail_el,
                prefs_overlay_el,
                updates_overlay_el,
            )
        };
        self.focus_handles = focus_handles;

        let mut content_row = div()
            .flex()
            .flex_row()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .gap(px(Space::PAD))
            .p(px(Space::PAD))
            .child(stage);

        if let Some(rail_el) = rail_el {
            content_row = content_row.child(rail_el);
        }

        let mut root = div()
            .id("phonecam-root")
            .relative()
            .track_focus(&focus_handle)
            .on_action(cx.listener(|this, _: &ToggleStream, _window, cx| {
                this.toggle_stream();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleFocusPreview, _window, cx| {
                this.toggle_focus_preview();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ExitFocusPreview, _window, cx| {
                if this.show_updates {
                    this.show_updates = false;
                    cx.notify();
                } else if this.show_preferences {
                    this.show_preferences = false;
                    cx.notify();
                } else if this.focus_preview {
                    this.focus_preview = false;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &SelectTabSetup, _window, cx| {
                this.set_tab(Tab::Setup);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectTabCapture, _window, cx| {
                this.set_tab(Tab::Capture);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectTabStream, _window, cx| {
                this.set_tab(Tab::Capture);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectTabPublish, _window, cx| {
                this.set_tab(Tab::Publish);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectTabVideo, _window, cx| {
                this.set_tab(Tab::Publish);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SelectTabDiagnostics, _window, cx| {
                this.set_tab(Tab::Diagnostics);
                cx.notify();
            }))
            .on_action(cx.listener(|_this, _: &TabNext, window, _cx| {
                window.focus_next();
            }))
            .on_action(cx.listener(|_this, _: &TabPrev, window, _cx| {
                window.focus_prev();
            }))
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(SHELL.base)
            .font(Fonts::sans())
            .text_size(px(Type::TWO_SM))
            .text_color(Text::PRIMARY)
            .child(status_strip_el)
            .child(panels::notice_bar(&self.state));

        if let Some(toast) = self.state.toast.clone() {
            root = root.child(panels::toast(&toast));
        }

        root = root.child(content_row);

        if let Some(prefs) = prefs_overlay_el {
            root = root.child(prefs);
        }

        if let Some(updates) = updates_overlay_el {
            root = root.child(updates);
        }

        if self.state.pairing_prompt.is_some() {
            root = root.child(panels::pair_request_prompt(&self.state));
        }

        root
    }
}

/// The window title, which names the phone so a screenshot is self-describing.
pub fn window_title(state: &AppState) -> String {
    match state.device_name() {
        Some(name) => format!("PhoneCam - {name}"),
        None => "PhoneCam".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_names_the_phone_when_there_is_one() {
        assert_eq!(window_title(&AppState::default()), "PhoneCam");
    }

    #[test]
    fn a_new_view_starts_on_setup_with_the_rail_showing() {
        let view = AppView::new();
        assert_eq!(view.tab, Tab::Setup);
        assert!(!view.focus_preview);
    }

    #[test]
    fn a_no_device_notice_brings_setup_forward() {
        let mut view = AppView::new();
        view.tab = Tab::Diagnostics;
        let mut state = AppState::default();
        state.add_notice(FailureCode::NoDevice, vec![], "no-device".into());
        view.set_state(state);
        assert_eq!(view.tab, Tab::Setup);
    }

    #[test]
    fn other_notices_do_not_move_the_user() {
        let mut view = AppView::new();
        view.tab = Tab::Capture;
        let mut state = AppState::default();
        state.add_notice(FailureCode::UsbMissing, vec![], "usb".into());
        view.set_state(state);
        assert_eq!(view.tab, Tab::Capture);
    }

    #[test]
    fn f11_toggles_focus_preview() {
        let mut view = AppView::new();
        assert!(!view.focus_preview);
        view.toggle_focus_preview();
        assert!(view.focus_preview);
        view.toggle_focus_preview();
        assert!(!view.focus_preview);
    }

    #[test]
    fn set_tab_switches_active_tab() {
        let mut view = AppView::new();
        view.set_tab(Tab::Diagnostics);
        assert_eq!(view.tab, Tab::Diagnostics);
        view.set_tab(Tab::Capture);
        assert_eq!(view.tab, Tab::Capture);
        view.set_tab(Tab::Publish);
        assert_eq!(view.tab, Tab::Publish);
        view.set_tab(Tab::Setup);
        assert_eq!(view.tab, Tab::Setup);
    }

    #[test]
    fn space_shortcut_behavior() {
        drain_effects();
        let mut view = AppView::new();

        // No device connected -> shows toast
        view.toggle_stream();
        assert_eq!(
            view.state.toast.as_deref(),
            Some("Connect a phone before starting the camera")
        );
        assert!(drain_effects().is_empty());

        // Device present but not streaming -> pushes StartStream effect
        let mut state = AppState::default();
        state.status.connected = true;
        state.core = Some(phonecam_protocol::CoreState {
            phase: phonecam_protocol::StreamPhase::Connected,
            device: Some(phonecam_protocol::DeviceIdentity {
                device_id: "phone-1".into(),
                name: "Pixel 8".into(),
                model: "Pixel 8".into(),
                android_version: 34,
            }),
            ..Default::default()
        });
        view.set_state(state);
        view.toggle_stream();
        assert_eq!(drain_effects(), vec![SideEffect::StartStream]);

        // Streaming -> pushes StopStream effect
        if let Some(core) = view.state.core.as_mut() {
            core.phase = phonecam_protocol::StreamPhase::Streaming;
        }
        view.toggle_stream();
        assert_eq!(drain_effects(), vec![SideEffect::StopStream]);
    }

    #[test]
    fn set_preview_drops_previous_frame_safely() {
        let mut view = AppView::new();
        let buf1 = vec![42u8; 100];
        assert!(view.set_preview(buf1, 5, 5).is_none());
        assert!(view.preview_image.is_some());
        assert!(view.previous_preview_image.is_none());

        let buf2 = vec![99u8; 100];
        assert!(view.set_preview(buf2, 5, 5).is_none());
        assert!(view.preview_image.is_some());
        assert!(view.previous_preview_image.is_some());

        let buf3 = vec![77u8; 100];
        assert!(view.set_preview(buf3, 5, 5).is_none());
        assert!(view.preview_image.is_some());
        assert!(view.previous_preview_image.is_some());

        assert!(view.clear_preview().is_none());
        assert!(view.preview_image.is_none());
        assert!(view.previous_preview_image.is_none());
    }

    #[test]
    fn first_run_auto_arms_pairing_once_and_ui_drawers_default_closed() {
        drain_effects();
        let mut view = AppView::new();
        assert!(!view.show_preferences);
        assert!(!view.labs_expanded);
        assert!(!view.gesture_ref_expanded);

        let mut state = AppState::default();
        state.status.connected = true;
        view.set_state(state.clone());
        assert_eq!(drain_effects(), vec![SideEffect::ArmPairing]);

        // Subsequent state snapshots do not re-arm automatically.
        view.set_state(state);
        assert!(drain_effects().is_empty());
    }

    #[test]
    fn app_view_has_valid_scroll_handle_and_rail_renders_across_all_tabs() {
        let view = AppView::new();
        let offset = view.scroll_handle.offset();
        assert_eq!(offset.x, px(0.0));
        assert_eq!(offset.y, px(0.0));

        let state = AppState::default();
        for tab in Tab::ALL {
            let _rail = panels::rail(&state, tab, false, &view.scroll_handle);
            let _narrow_rail = panels::rail(&state, tab, true, &view.scroll_handle);
        }
    }
}
