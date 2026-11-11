//! The window: shell, panels, and the pure rules they read.

pub mod effects;
pub mod panels;
pub mod preview;
pub mod state;
pub mod widgets;

pub use effects::{drain_effects, push_effect, SideEffect, Tab};
pub use state::{
    format_bytes, format_duration, format_log_time, now_ms, stream_phase_label, thermal_label,
    AppState, CoreStatus, FailureCode, LogLine, LogSource, Notice, NoticeAction, PairingPrompt,
    Settings,
};

use gpui::prelude::*;
use gpui::{div, px, Context, IntoElement, Render, Window};

use crate::theme::{Fonts, Radius, SHELL, Space, Text, Type};
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
    /// The pairing code, regenerated only when its payload changes.
    pub qr_image: Option<std::sync::Arc<gpui::RenderImage>>,
    qr_payload_cached: Option<String>,
    
}

impl AppView {
    pub fn new() -> Self {
        Self {
            state: AppState::default(),
            tab: Tab::Setup,
            focus_preview: false,
            pairing_network: 0,
            preview_image: None,
            qr_image: None,
            qr_payload_cached: None,
        }
    }

    /// Replaces the state and re-derives anything that depends on it.
    pub fn set_state(&mut self, state: AppState) {
        // A fresh notice that needs action brings Setup forward, so the way out
        // is on screen rather than described. Only ever moves to Setup, which is
        // the one tab that resolves an empty state, and never back.
        if state.notices.iter().any(|n| n.code == FailureCode::NoDevice) && !state.has_device() {
            self.tab = Tab::Setup;
        }
        self.state = state;
    }

    /// Installs a decoded picture. The pixels are BGRA.
    pub fn set_preview(&mut self, pixels: Vec<u8>, width: u32, height: u32) {
        if let Some(buffer) = image::RgbaImage::from_raw(width, height, pixels) {
            self.preview_image =
                Some(std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(buffer)])));
        }
    }

    pub fn clear_preview(&mut self) {
        self.preview_image = None;
    }

    /// Renders the pairing code once per payload, not once per frame.
    fn ensure_qr(&mut self, payload: &str) {
        if self.qr_payload_cached.as_deref() == Some(payload) && self.qr_image.is_some() {
            return;
        }
        self.qr_image = preview::qr_pixels(payload, crate::theme::QR_SIZE).and_then(
            |(buffer, w, h)| {
                image::RgbaImage::from_raw(w as u32, h as u32, buffer).map(|image| {
                    std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(image)]))
                })
            },
        );
        self.qr_payload_cached = Some(payload.to_string());
    }

}

impl Default for AppView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // While a picture is on screen the window pulls frames rather than being
        // pushed them, so a stalled decode cannot queue work in the UI.
        if self.state.is_streaming() {
            window.request_animation_frame();
        }

        let content = preview::stage_content(&self.state, self.pairing_network);
        if let StageContent::Pairing { payload, .. } = &content {
            if !payload.is_empty() {
                self.ensure_qr(payload);
            }
        }

        let narrow = f32::from(window.viewport_size().width) < crate::theme::NARROW_WINDOW;

        let stage: gpui::AnyElement = match &content {
            StageContent::Picture => {
                let picture: gpui::AnyElement = match &self.preview_image {
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
                        None,
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
}





