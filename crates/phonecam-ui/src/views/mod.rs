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
    CoreStatus, FailureCode, LogLine, LogSource, Notice, NoticeAction, PairingPrompt, Settings,
};

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
    /// Which network's code the user picked, when the PC is on several.
    pub pairing_network: usize,
    /// The one image handle for the picture.
    ///
    /// Reused across frames: a fresh texture 30 times a second is wasted
    /// allocation, and the shipped app's equivalent rule was "exactly one
    /// canvas, never remounted".
    pub preview_image: Option<std::sync::Arc<gpui::RenderImage>>,
    preview_capacity: usize,
    /// The pairing code, regenerated only when its payload changes.
    pub qr_image: Option<std::sync::Arc<gpui::RenderImage>>,
    qr_payload_cached: Option<String>,
    pub focus_handle: Option<gpui::FocusHandle>,
}

impl AppView {
    pub fn new() -> Self {
        Self {
            state: AppState::default(),
            tab: Tab::Setup,
            focus_preview: false,
            pairing_network: 0,
            preview_image: None,
            preview_capacity: 0,
            qr_image: None,
            qr_payload_cached: None,
            focus_handle: None,
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
        self.state = state;
    }

    /// Installs a decoded picture. The pixels are BGRA.
    ///
    /// Returns the previously consumed preview buffer if it can be recycled.
    pub fn set_preview(&mut self, pixels: Vec<u8>, width: u32, height: u32) -> Option<Vec<u8>> {
        self.set_preview_in(pixels, width, height, None)
    }

    /// Installs a decoded picture into the preview slot. Reclaims the previous
    /// frame's buffer for recycling without forcefully dropping the GPU texture
    /// mid-stream (which causes D3D11 texture atlas eviction flickering).
    pub fn set_preview_in(
        &mut self,
        pixels: Vec<u8>,
        width: u32,
        height: u32,
        _window: Option<&mut Window>,
    ) -> Option<Vec<u8>> {
        // Pass None for window drop during per-frame stream updates so GPUI replaces
        // textures smoothly without blanking or atlas churn.
        let recycled =
            recycle_preview_image(&mut self.preview_image, &mut self.preview_capacity, None);

        let cap = pixels.capacity();
        if let Some(buffer) = image::RgbaImage::from_raw(width, height, pixels) {
            self.preview_capacity = cap;
            self.preview_image = Some(std::sync::Arc::new(gpui::RenderImage::new(vec![
                image::Frame::new(buffer),
            ])));
        }

        recycled
    }

    pub fn clear_preview(&mut self) -> Option<Vec<u8>> {
        self.clear_preview_in(None)
    }

    /// Clears the active preview image and drops its GPU texture from `window`.
    pub fn clear_preview_in(&mut self, window: Option<&mut Window>) -> Option<Vec<u8>> {
        recycle_preview_image(&mut self.preview_image, &mut self.preview_capacity, window)
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

        let narrow = f32::from(window.viewport_size().width) < crate::theme::NARROW_WINDOW;

        let stage: gpui::AnyElement = match &content {
            StageContent::Picture => {
                let picture: gpui::AnyElement = if !self.state.settings.preview_enabled {
                    preview::preview_off_card()
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
                        None => div().flex_1().into_any_element(),
                    }
                };
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .rounded(px(Radius::LG))
                    .border_1()
                    .border_color(SHELL.border.alpha(0.6))
                    .bg(gpui::rgb(0x000000))
                    .overflow_hidden()
                    .child(picture)
                    .child(preview::telemetry(
                        self.state.device_name(),
                        self.state.has_notice(FailureCode::DecoderFailed)
                            || self.state.has_notice(FailureCode::DecoderUnsupported),
                        self.state.frame_size,
                        self.state.core.as_ref().map(|c| c.stats.fps).unwrap_or(0.0),
                        self.state
                            .core
                            .as_ref()
                            .map(|c| c.stats.latency_ms)
                            .filter(|ms| *ms >= 0.0),
                        self.state.settings.output_rotation,
                    ))
                    .into_any_element()
            }
            StageContent::Pairing { .. } => {
                preview::pairing_stage(&content, self.qr_image.as_ref())
            }
            StageContent::Empty { .. } => preview::stage_frame(&content),
        };

        let mut content_row = div()
            .flex()
            .flex_row()
            .flex_1()
            .min_h(px(0.0))
            .gap(px(Space::XL))
            .p(px(Space::XL))
            .child(stage);

        if !self.focus_preview {
            content_row = content_row.child(panels::rail(&self.state, self.tab, narrow));
        }

        let mut root = div()
            .id("phonecam-root")
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
                if this.focus_preview {
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
            .bg(SHELL.base)
            .font_family(Fonts::SANS)
            .text_size(px(Type::TWO_SM))
            .text_color(Text::PRIMARY)
            .child(panels::status_strip(&self.state))
            .child(panels::notice_bar(&self.state));

        if let Some(toast) = self.state.toast.clone() {
            root = root.child(panels::toast(&toast));
        }

        root = root.child(content_row);

        if self.state.pairing_prompt.is_some() {
            root = root.child(panels::pair_request_prompt(&self.state));
        }

        root
    }
}

/// Drops the GPU texture and reclaims the backing BGRA allocation with its original capacity.
fn recycle_preview_image(
    slot: &mut Option<std::sync::Arc<gpui::RenderImage>>,
    stored_cap: &mut usize,
    window: Option<&mut Window>,
) -> Option<Vec<u8>> {
    let old_cap = std::mem::take(stored_cap);
    let arc_img = slot.take()?;
    if let Some(w) = window {
        let _ = w.drop_image(arc_img.clone());
    }
    std::sync::Arc::try_unwrap(arc_img)
        .ok()
        .and_then(|render_img| {
            let (ptr, len) = if let Some(bytes) = render_img.as_bytes(0) {
                (bytes.as_ptr() as *mut u8, bytes.len())
            } else {
                return None;
            };
            std::mem::forget(render_img);
            let cap = old_cap.max(len);
            Some(unsafe { Vec::from_raw_parts(ptr, len, cap) })
        })
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
    fn set_preview_recycles_previous_buffer() {
        let mut view = AppView::new();
        let buf1 = vec![42u8; 100];
        let r1 = view.set_preview(buf1, 5, 5);
        assert!(r1.is_none());

        let buf2 = vec![99u8; 100];
        let r2 = view.set_preview(buf2, 5, 5);
        assert!(r2.is_some());
        let recycled = r2.unwrap();
        assert_eq!(recycled.len(), 100);
        assert_eq!(recycled[0], 42);

        let r3 = view.clear_preview();
        assert!(r3.is_some());
        let recycled3 = r3.unwrap();
        assert_eq!(recycled3.len(), 100);
        assert_eq!(recycled3[0], 99);
    }

    #[test]
    fn set_preview_preserves_original_vec_capacity() {
        let mut view = AppView::new();
        let mut buf1 = Vec::with_capacity(256);
        buf1.resize(100, 7);
        assert!(view.set_preview(buf1, 5, 5).is_none());

        let buf2 = vec![1u8; 100];
        let recycled = view.set_preview(buf2, 5, 5).expect("buffer should recycle");
        assert_eq!(recycled.len(), 100);
        assert!(
            recycled.capacity() >= 256,
            "capacity should be preserved, got {}",
            recycled.capacity()
        );
    }
}
