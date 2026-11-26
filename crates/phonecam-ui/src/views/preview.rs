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

use crate::theme::{Fonts, Radius, Space, Status, Text, Tone, Type, EMPTY_VALUE, SHELL};
use crate::views::effects::{push_effect, SideEffect};
use crate::views::state::AppState;
use crate::views::widgets::{chip, numeric_font, Button, FocusContext};

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
    StartStreaming {
        enabled: bool,
        title: Option<String>,
    },
    ShowPairingCode {
        remembered: bool,
    },
}

/// Decides what the stage draws.
///
/// Pairing yields to any problem state: while a code is on screen there is
/// nothing else the user can usefully do, but it must not cover a reconnecting
/// session — the old build showed a scannable code over a dropping phone and
/// hid the sentence explaining it.
pub fn stage_content(state: &AppState, pairing_selected: usize) -> StageContent {
    let streaming = state.is_streaming();
    if streaming {
        return StageContent::Picture;
    }

    let has_device = state.has_device();
    let summary = state.status_summary();
    let pairing = state.core.as_ref().map(|c| &c.pairing);
    let pairing_open =
        pairing.map(|p| p.open).unwrap_or(false) && !summary.attention && !has_device;

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
        let mut note = format!(
            "{} x {}",
            state.settings.output_width, state.settings.output_height
        );
        if state.settings.output_mirror {
            note.push_str(" · mirrored");
        }
        if state.settings.output_rotation != 0 {
            note.push_str(&format!(" · {}°", state.settings.output_rotation));
        }
        note
    });

    StageContent::Empty {
        title: title.into(),
        body: body.into(),
        action,
        output_note,
    }
}

/// Pulls `host` out of a QR payload without trusting its shape.
fn network_host(payload: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    value.get("host")?.as_str().map(|s| s.to_string())
}

/// Renders the stage chrome around the empty state.
pub fn stage_frame(content: &StageContent) -> AnyElement {
    stage_frame_with_focus(content, None)
}

/// Renders the stage chrome around the empty state with optional keyboard focus tracking.
pub fn stage_frame_with_focus(
    content: &StageContent,
    focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let inner: AnyElement = match content {
        StageContent::Pairing { .. } => div().into_any_element(),
        StageContent::Empty { .. } => empty_body(content, focus),
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
        .border_color(SHELL.border)
        .bg(SHELL.stage)
        .p(px(20.0))
        .child(inner)
        .into_any_element()
}

/// The pairing stage, with the code already rendered.
pub fn pairing_stage(
    content: &StageContent,
    qr: Option<&std::sync::Arc<gpui::RenderImage>>,
) -> AnyElement {
    pairing_stage_with_context(content, qr, 840.0, None)
}

/// Responsive, scrollable pairing stage with viewport height awareness and focus tracking.
pub fn pairing_stage_with_context(
    content: &StageContent,
    qr: Option<&std::sync::Arc<gpui::RenderImage>>,
    viewport_height: f32,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let StageContent::Pairing {
        payload,
        expires_in_ms,
        expired,
        networks,
        selected,
        no_phone_broadcasting,
    } = content
    else {
        return div().into_any_element();
    };

    let title = if *expired {
        "Pairing code expired"
    } else {
        "Scan QR code with PhoneCam"
    };
    let sub = if *expired {
        "Codes work once and last five minutes.".to_string()
    } else {
        match expires_in_ms {
            Some(ms) => {
                format!(
                    "Single-use code · expires in {}",
                    crate::views::state::format_duration(*ms)
                )
            }
            None => "Single-use code".to_string(),
        }
    };

    let qr_display_size = if viewport_height < 740.0 {
        200.0
    } else {
        248.0
    };

    let code: AnyElement = match qr {
        Some(image) => div()
            .mt(px(Space::PAD))
            .rounded(px(Radius::MD))
            .bg(gpui::rgb(0xf8fafc))
            .p(px(Space::XL))
            .child(
                gpui::img(image.clone())
                    .w(px(qr_display_size))
                    .h(px(qr_display_size)),
            )
            .into_any_element(),
        None => div()
            .mt(px(Space::PAD))
            .w(px(qr_display_size))
            .h(px(qr_display_size))
            .rounded(px(Radius::MD))
            .bg(SHELL.recessed)
            .into_any_element(),
    };

    let mut card = div()
        .w_full()
        .max_w(px(420.0))
        .my_auto()
        .flex()
        .flex_col()
        .items_center()
        .rounded(px(Radius::LG))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.panel)
        .p(px(24.0))
        .child(
            div()
                .font(Fonts::display())
                .text_size(px(Type::STATUS))
                .line_height(px(26.0))
                .text_center()
                .text_color(Text::PRIMARY)
                .child(title),
        )
        .child(
            div()
                .mt(px(Space::MD))
                .font(numeric_font(Fonts::MONO))
                .text_size(px(Type::XS))
                .text_color(Text::MUTED)
                .child(sub),
        )
        .child(code);

    if networks.len() > 1 {
        card = card
            .child(
                div()
                    .mt(px(Space::PAD))
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(Space::MD))
                    .children(networks.iter().enumerate().map(|(index, host)| {
                        div()
                            .id(gpui::ElementId::Name(gpui::SharedString::from(format!(
                                "pairing-net-{index}"
                            ))))
                            .cursor_pointer()
                            .on_click(move |_event, _window, _cx| {
                                crate::views::effects::push_effect(
                                    crate::views::effects::SideEffect::SelectPairingNetwork(index),
                                );
                            })
                            .child(chip(
                                host.clone(),
                                if index == *selected {
                                    Tone::Accent
                                } else {
                                    Tone::Neutral
                                },
                            ))
                            .into_any_element()
                    })),
            )
            .child(
                div()
                    .mt(px(Space::LG))
                    .font(Fonts::sans())
                    .text_size(px(Type::XS))
                    .line_height(px(Type::LINE_XS))
                    .text_center()
                    .text_color(Text::MUTED)
                    .child("Multiple networks detected. Select the Wi-Fi your phone is on."),
            );
    }

    if !expired && *no_phone_broadcasting {
        card = card.child(
            div()
                .mt(px(Space::PAD))
                .w_full()
                .child(crate::views::widgets::tinted_box(
                    Tone::Warn,
                    vec![div()
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .line_height(px(Type::LINE_XS))
                        .text_color(Status::WARN)
                        .child("No phone is broadcasting on this network yet. Open PhoneCam on your phone and confirm both devices share the same Wi-Fi.")
                        .into_any_element()],
                )),
        );
    }

    let copy_btn = (!*expired && !payload.is_empty()).then(|| {
        Button::new("Copy Pairing Code")
            .id("pairing-copy-code")
            .focus_with(focus.as_deref_mut(), "pairing-copy-code")
            .on_click(|_window, _cx| {
                crate::views::effects::push_effect(
                    crate::views::effects::SideEffect::CopyPairingPayload,
                )
            })
            .into_any_element()
    });

    let primary_btn = if *expired {
        Button::primary("Show New Code")
            .id("pairing-new-code")
            .focus_with(focus.as_deref_mut(), "pairing-new-code")
            .on_click(|_window, _cx| {
                crate::views::effects::push_effect(crate::views::effects::SideEffect::ArmPairing)
            })
            .into_any_element()
    } else {
        Button::new("Cancel")
            .id("pairing-cancel")
            .focus_with(focus, "pairing-cancel")
            .on_click(|_window, _cx| {
                crate::views::effects::push_effect(crate::views::effects::SideEffect::DisarmPairing)
            })
            .into_any_element()
    };

    card = card.child(
        div()
            .mt(px(Space::SECTION))
            .flex()
            .flex_row()
            .flex_wrap()
            .justify_center()
            .gap(px(Space::LG))
            .children(copy_btn)
            .child(primary_btn),
    );

    div()
        .id("pairing-stage-scroll")
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .min_w(px(0.0))
        .overflow_y_scroll()
        .items_center()
        .rounded(px(Radius::LG))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.stage)
        .p(px(20.0))
        .child(card)
        .into_any_element()
}

fn empty_body(content: &StageContent, mut focus: Option<&mut FocusContext<'_>>) -> AnyElement {
    let StageContent::Empty {
        title,
        body,
        action,
        output_note,
    } = content
    else {
        return div().into_any_element();
    };

    let action_element: AnyElement = match action {
        EmptyAction::WaitingForService => div()
            .mt(px(Space::SECTION))
            .child(chip("Waiting for local service", Tone::Warn))
            .into_any_element(),
        EmptyAction::StartStreaming { enabled, .. } => div()
            .mt(px(Space::SECTION))
            .flex()
            .flex_row()
            .flex_wrap()
            .justify_center()
            .gap(px(Space::LG))
            .child(
                Button::primary("Start Streaming")
                    .id("stage-start-stream")
                    .focus_with(focus.as_deref_mut(), "stage-start-stream")
                    .disabled(!enabled)
                    .on_click(|_window, _cx| push_effect(SideEffect::StartStream))
                    .into_any_element(),
            )
            .child(
                Button::new("Pair Another Phone")
                    .id("stage-pair-another")
                    .focus_with(focus.as_deref_mut(), "stage-pair-another")
                    .on_click(|_window, _cx| push_effect(SideEffect::ArmPairing))
                    .into_any_element(),
            )
            .into_any_element(),
        EmptyAction::ShowPairingCode { remembered } => div()
            .mt(px(Space::SECTION))
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(Space::PAD))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_center()
                    .gap(px(Space::LG))
                    .child(
                        Button::primary("Show QR Pairing Code")
                            .id("stage-show-pairing")
                            .focus_with(focus.as_deref_mut(), "stage-show-pairing")
                            .on_click(|_window, _cx| push_effect(SideEffect::ArmPairing))
                            .into_any_element(),
                    )
                    .child(
                        Button::new("Connect via USB")
                            .id("stage-connect-usb")
                            .focus_with(focus, "stage-connect-usb")
                            .on_click(|_window, _cx| push_effect(SideEffect::ConnectUsb(None)))
                            .into_any_element(),
                    ),
            )
            .child(
                div()
                    .font(Fonts::sans())
                    .text_size(px(Type::XS))
                    .line_height(px(Type::LINE_XS))
                    .text_center()
                    .text_color(Text::MUTED)
                    .child(if *remembered {
                        "This PC remembers your phone. Open PhoneCam on it to reconnect automatically."
                    } else {
                        "Once connected, pick 'PhoneCam Camera' in OBS, Zoom, Meet, or Discord."
                    }),
            )
            .into_any_element(),
    };

    div()
        .w_full()
        .max_w(px(420.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .rounded(px(Radius::LG))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.panel)
        .p(px(24.0))
        .child(
            div()
                .font(Fonts::display())
                .text_size(px(Type::STATUS))
                .line_height(px(26.0))
                .text_center()
                .text_color(Text::PRIMARY)
                .child(title.clone()),
        )
        .child(
            div()
                .mt(px(Space::LG))
                .font(Fonts::sans())
                .text_size(px(Type::TWO_SM))
                .line_height(px(Type::LINE_TWO_SM))
                .text_center()
                .text_color(Text::MUTED)
                .child(body.clone()),
        )
        .child(action_element)
        .children(output_note.as_ref().map(|note| {
            div()
                .mt(px(Space::PAD))
                .rounded(px(Radius::PILL))
                .border_1()
                .border_color(SHELL.border)
                .bg(SHELL.recessed)
                .px(px(10.0))
                .py(px(3.0))
                .font(numeric_font(Fonts::MONO))
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child(note.clone())
        }))
        .into_any_element()
}

pub(crate) fn clean_gesture_text(raw: &str) -> String {
    raw.trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
        .trim()
        .to_string()
}

/// Renders visual feedback toast/chip on the stage when gestures are detected
/// or face tracking is locked.
pub fn stage_gesture_feedback(state: &AppState) -> AnyElement {
    let mut chips = Vec::new();

    if state.settings.auto_framing && state.tracking_locked {
        chips.push(chip("Tracking locked", Tone::Warn));
    }

    if let Some(action) = &state.gesture_action {
        if state.toast.as_ref() == Some(action) || state.active_gesture.is_some() {
            let cleaned = clean_gesture_text(action);
            if !cleaned.is_empty() {
                chips.push(chip(cleaned, Tone::Ok));
            }
        }
    } else if let Some(gesture) = state.active_gesture {
        match gesture {
            phonecam_media::HandGesture::OpenPalm => {
                chips.push(chip("Palm: Lock tracking", Tone::Ok));
            }
            phonecam_media::HandGesture::PeaceV => {
                chips.push(chip("V-Sign: Zoom 2.0x", Tone::Ok));
            }
            phonecam_media::HandGesture::IndexPoint => {
                chips.push(chip("Index: 1.5x zoom", Tone::Ok));
            }
            phonecam_media::HandGesture::ShakaCall => {
                chips.push(chip("Shaka: Reset framing", Tone::Ok));
            }
            phonecam_media::HandGesture::PinchOk => {
                chips.push(chip("Pinch: 2.2x macro", Tone::Ok));
            }
            phonecam_media::HandGesture::ThumbsUp => {
                chips.push(chip("Thumbs up: Confirmed", Tone::Ok));
            }
            phonecam_media::HandGesture::None => {}
        }
    }

    if chips.is_empty() {
        return div().into_any_element();
    }

    div()
        .absolute()
        .top(px(Space::MD))
        .left(px(0.0))
        .right(px(0.0))
        .flex()
        .flex_row()
        .justify_center()
        .gap(px(Space::SM))
        .children(chips)
        .into_any_element()
}

/// Sleek floating translucent bottom studio HUD pill bar rendered over the
/// full-bleed live preview stage.
pub fn live_studio_hud(
    state: &AppState,
    focus_preview: bool,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let preview_failed = state.has_notice(crate::views::state::FailureCode::DecoderFailed)
        || state.has_notice(crate::views::state::FailureCode::DecoderUnsupported);
    let mirror = state.settings.output_mirror;
    let next_rot = (state.settings.output_rotation + 90) % 360;
    let fps = state.core.as_ref().map(|c| c.stats.fps).unwrap_or(0.0);
    let rtt = state
        .core
        .as_ref()
        .map(|c| c.stats.latency_ms)
        .filter(|ms| *ms > 0.0);

    let mut metrics_parts: Vec<String> = Vec::new();
    if fps > 0.0 {
        metrics_parts.push(format!("{fps:.1} fps"));
    }
    if let Some((w, h)) = state.frame_size {
        metrics_parts.push(format!("{w}x{h}"));
    }
    if let Some(ms) = rtt {
        metrics_parts.push(format!("{ms:.0} ms"));
    }
    if state.settings.output_rotation != 0 {
        metrics_parts.push(format!("{}°", state.settings.output_rotation));
    }
    let metrics_text = if metrics_parts.is_empty() {
        "Live".to_string()
    } else {
        metrics_parts.join(" · ")
    };

    let status_tone = if preview_failed { Tone::Warn } else { Tone::Ok };
    let status_label = if preview_failed { "No preview" } else { "Live" };

    let hud_btn = |id: &'static str,
                   label: &'static str,
                   selected: bool,
                   danger: bool,
                   focus: Option<&mut FocusContext<'_>>,
                   on_click: Box<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>|
     -> AnyElement {
        let mut btn = if danger {
            Button::danger(label)
        } else {
            Button::new(label).selected(selected)
        };
        btn = btn
            .id(id)
            .focus_with(focus, id)
            .on_click(move |w, cx| on_click(w, cx));
        btn.into_any_element()
    };

    let bar = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(Space::MD))
        .rounded(px(Radius::PILL))
        .border_1()
        .border_color(SHELL.control_border.alpha(0.85))
        .bg(SHELL.stage.alpha(0.86))
        .px(px(12.0))
        .py(px(6.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(Space::MD))
                .pl(px(Space::SM))
                .pr(px(Space::MD))
                .child(crate::views::widgets::status_dot(status_tone))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(status_tone.fg())
                        .child(status_label),
                ),
        )
        .child(hud_btn(
            "hud-stop-stream",
            "Stop",
            false,
            true,
            focus.as_deref_mut(),
            Box::new(|_w, _cx| push_effect(SideEffect::StopStream)),
        ))
        .child(hud_btn(
            "hud-mirror",
            if mirror { "Mirrored" } else { "Mirror" },
            mirror,
            false,
            focus.as_deref_mut(),
            Box::new(move |_w, _cx| push_effect(SideEffect::SetMirror(!mirror))),
        ))
        .child(hud_btn(
            "hud-rotate",
            "Rotate 90°",
            state.settings.output_rotation != 0,
            false,
            focus.as_deref_mut(),
            Box::new(move |_w, _cx| push_effect(SideEffect::SetRotation(next_rot))),
        ))
        .child(hud_btn(
            "hud-focus",
            if focus_preview { "Exit Focus" } else { "Focus" },
            focus_preview,
            false,
            focus,
            Box::new(|_w, _cx| push_effect(SideEffect::ToggleFocusPreview)),
        ))
        .child(
            div()
                .flex_shrink_0()
                .w(px(1.0))
                .h(px(18.0))
                .bg(SHELL.control_border),
        )
        .child(
            div()
                .px(px(Space::MD))
                .font(numeric_font(Fonts::MONO))
                .text_size(px(Type::TWO_XS))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(Text::BODY)
                .child(metrics_text),
        );

    div()
        .absolute()
        .bottom(px(14.0))
        .left(px(0.0))
        .right(px(0.0))
        .flex()
        .flex_row()
        .justify_center()
        .px(px(16.0))
        .child(bar)
        .into_any_element()
}

/// The telemetry strip, inside the frame's bottom edge.
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
        .min_w(px(0.0))
        .items_center()
        .gap(px(Space::MD))
        .rounded(px(Radius::PILL))
        .border_1()
        .border_color(SHELL.control_border.alpha(0.7))
        .bg(SHELL.stage.alpha(0.85))
        .px(px(Space::XL))
        .py(px(Space::SM));

    left = left.child(if preview_failed {
        chip("No preview", Tone::Warn)
    } else {
        chip("Live", Tone::Ok)
    });
    if let Some(name) = device_name {
        left = left.child(
            div()
                .min_w(px(0.0))
                .truncate()
                .font(Fonts::sans())
                .text_size(px(Type::TWO_XS))
                .text_color(Text::BODY)
                .child(name),
        );
    }

    let mut right = div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(px(Space::XL))
        .rounded(px(Radius::PILL))
        .border_1()
        .border_color(SHELL.control_border.alpha(0.7))
        .bg(SHELL.stage.alpha(0.85))
        .px(px(Space::XL))
        .py(px(Space::SM))
        .font(numeric_font(Fonts::MONO))
        .text_size(px(Type::TWO_XS))
        .text_color(Text::BODY);

    if let Some((w, h)) = frame_size {
        right = right.child(format!("{w}x{h}"));
    }
    if core_fps > 0.0 {
        right = right.child(format!("{core_fps:.1} fps"));
    }
    if let Some(ms) = local_latency_ms.filter(|ms| *ms > 0.0) {
        right = right.child(format!("{ms:.0} ms"));
    }
    if rotation != 0 {
        right = right.child(format!("{rotation}°"));
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
/// The token never becomes a string, a data URL or a log line — only pixels.
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
            let dark = if module_x == 0 || module_y == 0 || module_x > width || module_y > width {
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
                && value
                    .get("host")
                    .and_then(|v| v.as_str())
                    .map(|h| !h.is_empty())
                    .unwrap_or(false)
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

/// Card shown on the stage when streaming is active but window preview is disabled.
pub fn preview_off_card() -> AnyElement {
    preview_off_card_with_action(None, None)
}

/// Interactive `preview_off_card` with an `"Open Preferences"` action button.
pub fn preview_off_card_with_action(
    on_open_preferences: Option<Box<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let pref_button = Button::new("Open Preferences")
        .id("preview-off-open-prefs")
        .focus_with(focus, "preview-off-open-prefs")
        .on_click(move |window, cx| {
            if let Some(ref handler) = on_open_preferences {
                handler(window, cx);
            } else {
                push_effect(SideEffect::SetPreviewEnabled(true));
            }
        });

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .items_center()
        .justify_center()
        .px(px(32.0))
        .child(
            div()
                .w_full()
                .max_w(px(420.0))
                .flex()
                .flex_col()
                .items_center()
                .rounded(px(Radius::LG))
                .border_1()
                .border_color(SHELL.border)
                .bg(SHELL.panel)
                .p(px(24.0))
                .child(
                    div()
                        .font(Fonts::display())
                        .text_size(px(Type::STATUS))
                        .text_color(Text::PRIMARY)
                        .child("Preview is off"),
                )
                .child(
                    div()
                        .mt(px(Space::LG))
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_SM))
                        .line_height(px(Type::LINE_TWO_SM))
                        .text_color(Text::MUTED)
                        .text_center()
                        .child("The phone is still streaming and the virtual camera is still publishing. Turn the preview back on in Preferences."),
                )
                .child(div().mt(px(Space::SECTION)).child(pref_button)),
        )
        .into_any_element()
}

/// Shown when the stream is active or starting but the first decoded GPU frame has not yet arrived.
/// This prevents a jarring pitch-black void on the stage.
pub fn preview_waiting_card(device_name: Option<&str>) -> AnyElement {
    preview_waiting_card_with_focus(device_name, None)
}

/// `preview_waiting_card` with optional focus tracking on the Cancel / Stop button.
pub fn preview_waiting_card_with_focus(
    device_name: Option<&str>,
    focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.0))
        .items_center()
        .justify_center()
        .px(px(24.0))
        .child(
            div()
                .w_full()
                .max_w(px(420.0))
                .flex()
                .flex_col()
                .items_center()
                .rounded(px(Radius::LG))
                .border_1()
                .border_color(SHELL.border)
                .bg(SHELL.panel)
                .p(px(24.0))
                .child(
                    div()
                        .font(Fonts::display())
                        .text_size(px(Type::STATUS))
                        .text_color(Text::PRIMARY)
                        .child("Waiting for video feed..."),
                )
                .child(
                    div()
                        .mt(px(Space::LG))
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_SM))
                        .line_height(px(Type::LINE_TWO_SM))
                        .text_color(Text::MUTED)
                        .text_center()
                        .child(format!(
                            "Connected to {}. Receiving initial camera frames...",
                            device_name.unwrap_or("phone camera")
                        )),
                )
                .child(
                    div().mt(px(Space::SECTION)).child(
                        Button::danger("Cancel / Stop Camera")
                            .id("waiting-stop-camera")
                            .focus_with(focus, "waiting-stop-camera")
                            .on_click(|_window, _cx| push_effect(SideEffect::StopStream)),
                    ),
                ),
        )
        .into_any_element()
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
    fn connected_device_does_not_show_pairing_code() {
        let mut state = state_with_device();
        // Even if pairing.open is still true, a connected device must never show the QR pairing stage.
        if let Some(core) = &mut state.core {
            core.pairing.open = true;
            core.pairing.qr_payloads = vec!["some_payload".into()];
        }
        match stage_content(&state, 0) {
            StageContent::Empty { title, action, .. } => {
                assert_eq!(title, "Ready to stream");
                assert!(matches!(
                    action,
                    EmptyAction::StartStreaming { enabled: true, .. }
                ));
            }
            other => panic!("expected Empty ready to stream, got {other:?}"),
        }

        // And when streaming, it shows Picture
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
            StageContent::Empty {
                title,
                action,
                output_note,
                ..
            } => {
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
            StageContent::Empty {
                output_note,
                action,
                ..
            } => {
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
                assert!(note.contains("90°"));
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
        assert!(
            buffer[0] > 200,
            "the quiet zone should be light, got {}",
            buffer[0]
        );
    }

    #[test]
    fn empty_and_pairing_stages_render_without_panicking() {
        let state = state_with_device();
        let empty = stage_content(&state, 0);
        assert!(matches!(empty, StageContent::Empty { .. }));
        let _el = empty_body(&empty, None);

        let mut pairing_state = AppState::default();
        pairing_state.status.connected = true;
        if let Some(core) = &mut pairing_state.core {
            core.pairing.open = true;
            core.pairing.qr_payloads = vec![r#"{"protocol":"phonecam","version":1,"host":"10.0.0.2","port":48721,"token":"0123456789abcdef0123456789abcdef","name":"PC","serverId":"pc-2"}"#.into()];
        } else {
            let mut core = phonecam_protocol::CoreState::default();
            core.pairing.open = true;
            core.pairing.qr_payloads = vec![r#"{"protocol":"phonecam","version":1,"host":"10.0.0.2","port":48721,"token":"0123456789abcdef0123456789abcdef","name":"PC","serverId":"pc-2"}"#.into()];
            pairing_state.core = Some(core);
        }
        let pairing = stage_content(&pairing_state, 0);
        assert!(matches!(pairing, StageContent::Pairing { .. }));
        let _pel = pairing_stage(&pairing, None);
    }

    #[test]
    fn preview_waiting_card_renders_with_device_and_cancel_button() {
        let _card_with_name = preview_waiting_card(Some("Pixel 8"));
        let _card_fallback = preview_waiting_card(None);
    }

    #[test]
    fn stage_gesture_feedback_renders_chips() {
        let mut state = AppState::default();
        let _empty = stage_gesture_feedback(&state);

        state.settings.auto_framing = true;
        state.tracking_locked = true;
        let _locked = stage_gesture_feedback(&state);

        state.active_gesture = Some(phonecam_media::HandGesture::PeaceV);
        state.gesture_action = Some("✌ V-Sign: 2.0x Zoom".into());
        state.toast = Some("✌ V-Sign: 2.0x Zoom".into());
        let _feedback = stage_gesture_feedback(&state);

        // Test all expanded hand gestures
        state.active_gesture = Some(phonecam_media::HandGesture::OpenPalm);
        state.gesture_action = None;
        let _feedback_palm = stage_gesture_feedback(&state);

        state.active_gesture = Some(phonecam_media::HandGesture::IndexPoint);
        state.gesture_action = Some("☝ Index: 1.5x Zoom".into());
        state.toast = Some("☝ Index: 1.5x Zoom".into());
        let _feedback_index = stage_gesture_feedback(&state);

        state.active_gesture = Some(phonecam_media::HandGesture::ShakaCall);
        state.gesture_action = Some("🤙 Shaka: Framing & Zoom Reset".into());
        state.toast = Some("🤙 Shaka: Framing & Zoom Reset".into());
        let _feedback_shaka = stage_gesture_feedback(&state);

        state.active_gesture = Some(phonecam_media::HandGesture::PinchOk);
        state.gesture_action = Some("🤏 Pinch: 2.2x Macro".into());
        state.toast = Some("🤏 Pinch: 2.2x Macro".into());
        let _feedback_pinch = stage_gesture_feedback(&state);

        state.active_gesture = Some(phonecam_media::HandGesture::ThumbsUp);
        state.gesture_action = Some("👍 Thumbs Up: Confirmed".into());
        state.toast = Some("👍 Thumbs Up: Confirmed".into());
        let _feedback_thumbs = stage_gesture_feedback(&state);

        assert_eq!(
            clean_gesture_text("✋ Open Palm: Tracking Locked"),
            "Open Palm: Tracking Locked"
        );
        assert_eq!(
            clean_gesture_text("✌ V-Sign: 2.0x Zoom"),
            "V-Sign: 2.0x Zoom"
        );
    }

    #[test]
    fn preview_off_card_and_responsive_pairing_render() {
        let _off = preview_off_card();
        let mut pairing_state = AppState::default();
        pairing_state.status.connected = true;
        pairing_state.core = Some(phonecam_protocol::CoreState {
            pairing: phonecam_protocol::PairingStatus {
                open: true,
                qr_payloads: vec![r#"{"protocol":"phonecam","version":1,"host":"10.0.0.2","port":48721,"token":"0123456789abcdef0123456789abcdef","name":"PC","serverId":"pc-2"}"#.into()],
                ..Default::default()
            },
            ..Default::default()
        });
        let pairing = stage_content(&pairing_state, 0);
        let _compact = pairing_stage_with_context(&pairing, None, 680.0, None);
    }
}
