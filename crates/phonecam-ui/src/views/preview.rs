//! The preview stage: the picture, the pairing code, and the telemetry.
//!
//! Three things in here are deliberate and look like omissions:
//!
//! * **The code renders at 264px in the stage, not in the rail.** It is the only
//!   asset a second device has to *read*, and at sidebar size it was a
//!   business-card thumbnail while 65% of the window showed nothing.
//! * **The empty aperture is the true output aspect**, not a fixed 16:9, so the
//!   frame the user sees is the frame other apps receive.
//! * **The output size is stated only once a phone is present**, because on an
//!   unpaired screen it asserted a stream that did not exist.
//!
//! The picture itself is drawn by the window's own image view, which owns one
//! image handle for the lifetime of the window. That is the GPUI equivalent of
//! the shipped "exactly one canvas, never remounted" rule: a preview that swaps
//! its target paints into a detached surface and shows nothing while the core
//! decodes happily.

use gpui::prelude::*;
use gpui::{div, px, AnyElement, IntoElement};

use crate::theme::{Fonts, Radius, SHELL, Space, Status, Text, Tone, Type, EMPTY_VALUE};
use crate::views::state::AppState;
use crate::views::widgets::chip;

/// What the stage should draw.
#[derive(Debug, Clone, PartialEq)]
pub enum StageContent {
    /// The live picture.
    Picture,
    /// The pairing code, at hero size.
    Pairing {
        payload: String,
        expires_in_ms: Option<i64>,
        expired: bool,
        /// One entry per reachable network, so the user can pick the one their
        /// phone is on. Picking wrong is the most common reason a scan appears
        /// to do nothing.
        networks: Vec<String>,
        selected: usize,
        no_phone_broadcasting: bool,
    },
    /// Nothing yet: the aperture and the next step.
    Empty {
        title: String,
        body: String,
        action: EmptyAction,
        /// Shown only when a phone is present.
        output_note: Option<String>,
    },
}

/// The one primary control of the empty state.
#[derive(Debug, Clone, PartialEq)]
pub enum EmptyAction {
    WaitingForService,
    StartStreaming { enabled: bool, title: Option<String> },
    ShowPairingCode { remembered: bool },
}

/// Decides what the stage draws.
///
/// Pairing yields to any problem state: while a code is on screen there is
/// nothing else the user can usefully do, but it must not cover a reconnecting
/// session â€” the old build showed a scannable code over a dropping phone and
/// hid the sentence explaining it.
pub fn stage_content(state: &AppState, pairing_selected: usize) -> StageContent {
    let streaming = state.is_streaming();
    if streaming {
        return StageContent::Picture;
    }

    let summary = state.status_summary();
    let pairing = state.core.as_ref().map(|c| &c.pairing);
    let pairing_open = pairing.map(|p| p.open).unwrap_or(false) && !summary.attention;

    if pairing_open {
        let expires_at = pairing.and_then(|p| p.expires_at);
        let now = crate::views::state::now_ms();
        let expires_in_ms = expires_at.map(|at| (at - now).max(0));
        let expired = expires_in_ms.map(|ms| ms <= 0).unwrap_or(false);
        let payloads = pairing.map(|p| p.qr_payloads.clone()).unwrap_or_default();
        let hosts: Vec<String> = payloads
            .iter()
            .map(|p| network_host(p).unwrap_or_else(|| "Network".to_string()))
            .collect();
        let selected = if payloads.is_empty() {
            0
        } else {
            pairing_selected.min(payloads.len() - 1)
        };
        let payload = payloads.get(selected).cloned().unwrap_or_default();
        let nearby_empty = state
            .core
            .as_ref()
            .map(|c| c.nearby_phones.is_empty())
            .unwrap_or(true);

        return StageContent::Pairing {
            payload,
            expires_in_ms,
            expired,
            networks: hosts,
            selected,
            no_phone_broadcasting: nearby_empty,
        };
    }

    let connected = state.status.connected;
    let has_device = state.has_device();

    let title = if !connected {
        "Starting PhoneCam"
    } else if state.phase() == "reconnecting" {
        "Reconnecting to your phone"
    } else if has_device {
        "Ready to stream"
    } else {
        "Pair a phone to begin"
    };

    let body = if !connected {
        "The service starts with the app. This usually takes a moment."
    } else if state.phase() == "reconnecting" {
        "The last frame stays on the virtual camera while PhoneCam retries."
    } else if has_device {
        "Your phone is connected and its camera is idle."
    } else {
        "Your phone scans one code, and this PC remembers it from then on."
    };

    let action = if !connected {
        EmptyAction::WaitingForService
    } else if has_device {
        EmptyAction::StartStreaming {
            enabled: connected,
            title: (!connected)
                .then(|| "The PhoneCam service is not connected".to_string())
                .or_else(|| (!has_device).then(|| "No phone is connected".to_string())),
        }
    } else {
        EmptyAction::ShowPairingCode {
            remembered: state.settings.last_device_id.is_some(),
        }
    };

    let output_note = has_device.then(|| {
        let mut note = format!("{} x {}", state.settings.output_width, state.settings.output_height);
        if state.settings.output_mirror {
            note.push_str(" Â· mirrored");
        }
        if state.settings.output_rotation != 0 {
            note.push_str(&format!(" Â· {}Â°", state.settings.output_rotation));
        }
        note
    });

    StageContent::Empty { title: title.into(), body: body.into(), action, output_note }
}

/// Pulls `host` out of a QR payload without trusting its shape.
fn network_host(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    value.get("host")?.as_str().map(|s| s.to_string())
}

/// Renders the stage chrome around the empty state.
pub fn stage_frame(content: &StageContent) -> AnyElement {
    let inner: AnyElement = match content {
        StageContent::Pairing { .. } => div().into_any_element(),
        StageContent::Empty { .. } => empty_body(content),
        StageContent::Picture => div().into_any_element(),
    };

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .min_h(px(0.0))
        .items_center()
        .justify_center()
        .rounded(px(Radius::LG))
        .border_1()
        .border_color(SHELL.border.alpha(0.6))
        // The stage is recessed, not lifted: the one place a pure-black
        // background is correct, because it is the letterbox around the picture.
        .bg(gpui::rgb(0x000000))
        .p(px(Space::PAD))
        .child(inner)
        .into_any_element()
}

/// The pairing stage, with the code already rendered.
///
/// The image is passed in rather than drawn here so the view owns exactly one
/// QR texture and regenerates it only when the payload changes.
pub fn pairing_stage(content: &StageContent, qr: Option<&std::sync::Arc<gpui::RenderImage>>) -> AnyElement {
    let StageContent::Pairing {
        expires_in_ms,
        expired,
        networks,
        selected,
        no_phone_broadcasting,
        ..
    } = content
    else {
        return div().into_any_element();
    };

    let title = if *expired { "That code expired" } else { "Scan this with PhoneCam on your phone" };
    let sub = if *expired {
        "Codes work once and last five minutes.".to_string()
    } else {
        match expires_in_ms {
            Some(ms) => {
                format!("Works once Â· expires in {}", crate::views::state::format_duration(*ms))
            }
            None => "Works once".to_string(),
        }
    };

    let code: AnyElement = match qr {
        Some(image) => div()
            .mt(px(Space::LG))
            .rounded(px(Radius::LG))
            .bg(gpui::rgb(0xf1f5f9))
            .p(px(Space::XL))
            .child(
                gpui::img(image.clone())
                    .w(px(crate::theme::QR_SIZE as f32))
                    .h(px(crate::theme::QR_SIZE as f32)),
            )
            .into_any_element(),
        None => div()
            .mt(px(Space::LG))
            .w(px(crate::theme::QR_SIZE as f32))
            .h(px(crate::theme::QR_SIZE as f32))
            .into_any_element(),
    };

    let mut column = div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px(px(24.0))
        .py(px(16.0))
        .child(
            div()
                .font_family(Fonts::DISPLAY)
                .text_size(px(Type::TITLE))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(Text::PRIMARY)
                .child(title),
        )
        .child(
            div()
                .mt(px(Space::SM))
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child(sub),
        )
        .child(code);

    if networks.len() > 1 {
        column = column
            .child(
                div()
                    .mt(px(Space::LG))
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(Space::MD))
                    .children(networks.iter().enumerate().map(|(index, host)| {
                        chip(
                            host.clone(),
                            if index == *selected { Tone::Accent } else { Tone::Neutral },
                        )
                    })),
            )
            .child(
                div()
                    .mt(px(Space::LG))
                    .font_family(Fonts::SANS)
                    .text_size(px(Type::XS))
                    .text_color(Text::MUTED)
                    .child("This PC is on more than one network. Pick the one your phone is on."),
            );
    }

    if !expired && *no_phone_broadcasting {
        column = column.child(
            div()
                .mt(px(Space::LG))
                .max_w(px(384.0))
                .child(crate::views::widgets::tinted_box(
                    Tone::Warn,
                    vec![div()
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::XS))
                        .text_color(Status::WARN)
                        .child("No phone is broadcasting on this network yet. Open PhoneCam on your phone and check it is on the same Wi-Fi as this PC.")
                        .into_any_element()],
                )),
        );
    }

    column = column.child(
        div()
            .mt(px(Space::SECTION))
            .flex()
            .flex_row()
            .gap(px(Space::LG))
            .child(if *expired {
                crate::views::widgets::Button::primary("Show a new code")
                    .on_click(|_window, _cx| crate::views::effects::push_effect(crate::views::effects::SideEffect::ArmPairing))
                    .into_any_element()
            } else {
                crate::views::widgets::Button::new("Cancel")
                    .on_click(|_window, _cx| crate::views::effects::push_effect(crate::views::effects::SideEffect::DisarmPairing))
                    .into_any_element()
            }),
    );

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .items_center()
        .justify_center()
        .rounded(px(Radius::LG))
        .border_1()
        .border_color(SHELL.border.alpha(0.6))
        .bg(gpui::rgb(0x000000))
        .child(column)
        .into_any_element()
}

fn empty_body(content: &StageContent) -> AnyElement {
    let StageContent::Empty { title, body, action, output_note } = content else {
        return div().into_any_element();
    };

    let action_element: AnyElement = match action {
        EmptyAction::WaitingForService => div()
            .mt(px(Space::SECTION))
            .child(chip("Waiting for the service", Tone::Warn))
            .into_any_element(),
        EmptyAction::StartStreaming { enabled, .. } => div()
            .mt(px(Space::SECTION))
            .flex()
            .flex_row()
            .gap(px(Space::LG))
            .child(
                crate::views::widgets::Button::primary("Start streaming")
                    .disabled(!enabled)
                    .on_click(|_window, _cx| crate::views::effects::push_effect(crate::views::effects::SideEffect::StartStream))
                    .into_any_element(),
            )
            .into_any_element(),
        EmptyAction::ShowPairingCode { remembered } => div()
            .mt(px(Space::SECTION))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(Space::LG))
            .child(
                crate::views::widgets::Button::primary("Show a pairing code")
                    .on_click(|_window, _cx| crate::views::effects::push_effect(crate::views::effects::SideEffect::ArmPairing))
                    .into_any_element(),
            )
            .children(remembered.then(|| {
                div()
                    .font_family(Fonts::SANS)
                    .text_size(px(Type::XS))
                    .text_color(Text::MUTED)
                    .child("This PC already knows a phone. Open PhoneCam on it and it reconnects by itself.")
            }))
            .into_any_element(),
    };

    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .max_w(px(416.0))
        .px(px(32.0))
        .child(
            div()
                .font_family(Fonts::DISPLAY)
                .text_size(px(Type::DISPLAY))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(Text::PRIMARY)
                .child(title.clone()),
        )
        .child(
            div()
                .mt(px(Space::LG))
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_SM))
                .text_color(Text::MUTED)
                .child(body.clone()),
        )
        .child(action_element)
        .children(output_note.as_ref().map(|note| {
            div()
                .mt(px(Space::SECTION))
                .font_family(Fonts::MONO)
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child(note.clone())
        }))
        .into_any_element()
}

/// The telemetry strip, inside the frame's bottom edge.
///
/// Deliberately low contrast: it is a confidence check, not the thing being
/// watched. The badge reports what **this window** is showing, not what the
/// phone is sending â€” when the decoder has stopped, the stream is genuinely
/// still running and the virtual camera is still being fed, but this surface has
/// no picture, and a green "Live" over a black rectangle is the app contradicting
/// itself on its most important surface.
pub fn telemetry(
    device_name: Option<String>,
    preview_failed: bool,
    frame_size: Option<(u32, u32)>,
    core_fps: f64,
    local_latency_ms: Option<f64>,
    rotation: u32,
) -> AnyElement {
    let mut left = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(Space::MD))
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border.alpha(0.6))
        .bg(SHELL.recessed.alpha(0.85))
        .px(px(Space::LG))
        .py(px(Space::SM));

    left = left.child(if preview_failed {
        chip("No preview", Tone::Warn)
    } else {
        chip("Live", Tone::Ok)
    });
    if let Some(name) = device_name {
        left = left.child(
            div()
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_XS))
                .text_color(Text::BODY)
                .child(name),
        );
    }

    let mut right = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(Space::XL))
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border.alpha(0.6))
        .bg(SHELL.recessed.alpha(0.85))
        .px(px(Space::LG))
        .py(px(Space::SM))
        .font_family(Fonts::MONO)
        .text_size(px(Type::TWO_XS))
        .text_color(Text::BODY);

    if let Some((w, h)) = frame_size {
        right = right.child(format!("{w}x{h}"));
    }
    // The core's figure, not the preview's: it counts decoded frames from the
    // phone, which is the number the virtual camera and any other app receive.
    if core_fps > 0.0 {
        right = right.child(format!("{core_fps:.1} fps"));
    }
    if let Some(ms) = local_latency_ms.filter(|ms| *ms > 0.0) {
        right = right.child(format!("{ms:.0} ms"));
    }
    if rotation != 0 {
        right = right.child(format!("{rotation}Â°"));
    }

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(Space::LG))
        .px(px(Space::PAD))
        .pb(px(Space::PAD))
        .child(left)
        .child(right)
        .into_any_element()
}

/// Renders a QR payload into a BGRA buffer for display.
///
/// The token never becomes a string, a data URL or a log line â€” only pixels.
pub fn qr_pixels(payload: &str, size: usize) -> Option<(Vec<u8>, usize, usize)> {
    use qrcode::QrCode;

    // Guard the payload first: a malformed one means the core has a bug, and a
    // QR of garbage would waste the user's time.
    if !is_well_formed_payload(payload) {
        return None;
    }

    let code = QrCode::new(payload.as_bytes()).ok()?;
    let modules = code.to_colors();
    let width = code.width();
    if width == 0 {
        return None;
    }

    // Integer module size keeps the code crisp at any dimension.
    let scale = (size / (width + 2)).max(1);
    let dimension = (width + 2) * scale;
    let mut buffer = vec![0u8; dimension * dimension * 4];

    // Quiet zone and light modules are the near-white the shipped code uses.
    let (lr, lg, lb) = (0xf1u8, 0xf5u8, 0xf9u8);
    let (dr, dg, db) = (0x08u8, 0x0bu8, 0x0fu8);

    for y in 0..dimension {
        for x in 0..dimension {
            let module_x = x / scale;
            let module_y = y / scale;
            let dark = if module_x == 0
                || module_y == 0
                || module_x > width
                || module_y > width
            {
                false
            } else {
                modules[(module_y - 1) * width + (module_x - 1)] == qrcode::Color::Dark
            };
            let (r, g, b) = if dark { (dr, dg, db) } else { (lr, lg, lb) };
            let offset = (y * dimension + x) * 4;
            // BGRA, because that is what the renderer expects.
            buffer[offset] = b;
            buffer[offset + 1] = g;
            buffer[offset + 2] = r;
            buffer[offset + 3] = 255;
        }
    }

    Some((buffer, dimension, dimension))
}

/// Whether a payload looks like something a phone can scan.
fn is_well_formed_payload(payload: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(payload) {
        Ok(value) => {
            value.get("protocol").and_then(|v| v.as_str()) == Some("phonecam")
                && value.get("host").and_then(|v| v.as_str()).map(|h| !h.is_empty()).unwrap_or(false)
                && value
                    .get("token")
                    .and_then(|v| v.as_str())
                    .map(|t| t.len() == 32)
                    .unwrap_or(false)
        }
        Err(_) => false,
    }
}

/// A placeholder used where a value has not been established.
pub fn empty_value() -> &'static str {
    EMPTY_VALUE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::state::Settings;

    fn state_with_device() -> AppState {
        let mut state = AppState::default();
        state.status.connected = true;
        state.core = Some(phonecam_protocol::CoreState {
            phase: phonecam_protocol::StreamPhase::Connected,
            device: Some(phonecam_protocol::DeviceIdentity {
                device_id: "android-a".into(),
                name: "Redmi Note 11".into(),
                model: "2201117TG".into(),
                android_version: 33,
            }),
            ..Default::default()
        });
        state
    }

    #[test]
    fn streaming_shows_the_picture() {
        let mut state = state_with_device();
        if let Some(core) = &mut state.core {
            core.phase = phonecam_protocol::StreamPhase::Streaming;
        }
        assert_eq!(stage_content(&state, 0), StageContent::Picture);
    }

    #[test]
    fn an_unpaired_screen_offers_the_code() {
        let mut state = AppState::default();
        state.status.connected = true;
        match stage_content(&state, 0) {
            StageContent::Empty { title, action, output_note, .. } => {
                assert_eq!(title, "Pair a phone to begin");
                assert_eq!(action, EmptyAction::ShowPairingCode { remembered: false });
                // The output size is not stated with no phone: it would assert a
                // stream that does not exist.
                assert!(output_note.is_none());
            }
            other => panic!("expected the empty stage, got {other:?}"),
        }
    }

    #[test]
    fn a_present_phone_states_the_output_size() {
        let state = state_with_device();
        match stage_content(&state, 0) {
            StageContent::Empty { output_note, action, .. } => {
                assert!(output_note.is_some());
                assert!(matches!(action, EmptyAction::StartStreaming { .. }));
            }
            other => panic!("expected the empty stage, got {other:?}"),
        }
    }

    #[test]
    fn the_output_note_reports_mirror_and_rotation() {
        let mut state = state_with_device();
        state.settings = Settings {
            output_mirror: true,
            output_rotation: 90,
            output_width: 1280,
            output_height: 720,
            ..Settings::default()
        };
        match stage_content(&state, 0) {
            StageContent::Empty { output_note, .. } => {
                let note = output_note.unwrap();
                assert!(note.contains("1280 x 720"));
                assert!(note.contains("mirrored"));
                assert!(note.contains("90Â°"));
            }
            other => panic!("expected the empty stage, got {other:?}"),
        }
    }

    #[test]
    fn an_unreachable_service_waits_rather_than_offering_a_code() {
        let state = AppState::default();
        match stage_content(&state, 0) {
            StageContent::Empty { title, action, .. } => {
                assert_eq!(title, "Starting PhoneCam");
                assert_eq!(action, EmptyAction::WaitingForService);
            }
            other => panic!("expected the empty stage, got {other:?}"),
        }
    }

    #[test]
    fn pairing_yields_to_a_problem_state() {
        let mut state = AppState::default();
        state.status.connected = true;
        // Pairing open, but the phone is reconnecting: the code must not cover
        // the sentence explaining that.
        if let Some(core) = &mut state.core {
            core.pairing.open = true;
        } else {
            let mut core = phonecam_protocol::CoreState::default();
            core.pairing.open = true;
            core.phase = phonecam_protocol::StreamPhase::Reconnecting;
            core.device = Some(phonecam_protocol::DeviceIdentity {
                device_id: "d".into(),
                name: "Phone".into(),
                model: "m".into(),
                android_version: 30,
            });
            state.core = Some(core);
        }
        match stage_content(&state, 0) {
            StageContent::Pairing { .. } => panic!("pairing must yield to a problem"),
            StageContent::Empty { .. } => {}
            StageContent::Picture => panic!("not streaming"),
        }
    }

    #[test]
    fn a_malformed_payload_produces_no_qr() {
        assert!(qr_pixels("not json", 264).is_none());
        assert!(qr_pixels(r#"{"protocol":"phonecam"}"#, 264).is_none());
        // A short token is not a pairing code.
        assert!(qr_pixels(
            r#"{"protocol":"phonecam","host":"1.2.3.4","token":"short"}"#,
            264
        )
        .is_none());
    }

    #[test]
    fn a_valid_payload_renders_square_bgra() {
        let payload = r#"{"protocol":"phonecam","version":1,"host":"192.168.1.24","port":48721,"token":"9f2ab4c71de8036f5a1c9e2d4b7f0a63","name":"DESKTOP","serverId":"pc-1"}"#;
        let (buffer, w, h) = qr_pixels(payload, 264).expect("a valid payload must render");
        assert_eq!(w, h, "a QR is square");
        assert_eq!(buffer.len(), w * h * 4);
        // Opaque everywhere, so the code reads against the panel.
        assert!(buffer.chunks_exact(4).all(|px| px[3] == 255));
        // And it is not a single flat colour.
        let first = &buffer[0..4];
        assert!(buffer.chunks_exact(4).any(|px| px != first));
    }

    #[test]
    fn the_qr_has_a_quiet_zone() {
        let payload = r#"{"protocol":"phonecam","version":1,"host":"10.0.0.2","port":48721,"token":"0123456789abcdef0123456789abcdef","name":"PC","serverId":"pc-2"}"#;
        let (buffer, _w, _h) = qr_pixels(payload, 264).unwrap();
        // The top-left corner is quiet zone, so it must be light (near-white).
        assert!(buffer[0] > 200, "the quiet zone should be light, got {}", buffer[0]);
    }
}






