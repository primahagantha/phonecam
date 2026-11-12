//! The four panels, the status strip, the notice bar and the approval prompt.
//!
//! The layout follows the job, not the data model. An operator sits beside this
//! window with a phone on a tripod, and at any moment they are doing exactly one
//! of four things: get a phone connected, choose what it captures, choose what
//! this PC publishes, or read the numbers when something is wrong. Those are the
//! four tabs, in the order the job happens.

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Hsla, SharedString};

use crate::theme::rules;
use crate::theme::{Accent, Fonts, Radius, SHELL, Space, Status, Text, Tone, Type};
use crate::views::effects::{push_effect, SideEffect, Tab};
use crate::views::state::{
    format_log_time, resolution_label_of, thermal_label, AppState, LogSource, NoticeAction,
};
use crate::views::widgets::{
    chip, empty_state, key_value, metric, prose, section, status_dot, tinted_box, Button, Panel,
    Toggle,
};

/// Maps a status tone onto a widget tone.
pub fn tone_of(tone: crate::theme::rules::StatusTone) -> Tone {
    use crate::theme::rules::StatusTone;
    match tone {
        StatusTone::Ok => Tone::Ok,
        StatusTone::Warn => Tone::Warn,
        StatusTone::Danger => Tone::Danger,
        StatusTone::Neutral => Tone::Neutral,
        StatusTone::Accent => Tone::Accent,
    }
}

/* ------------------------------------------------------------------ */
/* Status strip                                                        */
/* ------------------------------------------------------------------ */

/// The one sentence the app says about itself, plus the transport control.
///
/// The header carries exactly one control, and only while streaming, because
/// stopping is the only thing that cannot be done anywhere else.
pub fn status_strip(state: &AppState) -> AnyElement {
    let summary = state.status_summary();
    let tone = tone_of(summary.tone);

    let vcam_running = state.vcam_state() == Some("running");
    let streaming = state.is_streaming();
    let vcam_label = rules::vcam_chip_label(
        state.vcam_state(),
        state.vcam_registered(),
        state.core.as_ref().and_then(|c| c.vcam.as_ref()).map(|v| v.name.as_str()),
    );
    let vcam_tone = if state.vcam_state() == Some("error") {
        Tone::Danger
    } else if vcam_running && streaming {
        Tone::Ok
    } else {
        Tone::Neutral
    };

    let mut row = div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(px(Space::PAD))
        .border_b_1()
        .border_color(SHELL.border.alpha(0.6))
        .bg(SHELL.raised)
        .px(px(Space::PAD))
        .py(px(Space::LG))
        .child(
            div()
                .flex()
                .flex_row()
                .min_w(px(0.0))
                .items_center()
                .gap(px(Space::XL))
                .child(status_dot(tone))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .font_family(Fonts::DISPLAY)
                                .text_size(px(Type::STATUS))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(Text::PRIMARY)
                                .child(summary.headline.clone()),
                        )
                        .children(summary.detail.clone().map(|detail| {
                            div()
                                .font_family(Fonts::MONO)
                                .text_size(px(Type::TWO_XS))
                                .text_color(Text::MUTED)
                                .child(detail)
                        })),
                ),
        )
        .child(div().flex_shrink_0().w(px(1.0)).h(px(32.0)).bg(SHELL.border.alpha(0.7)))
        .child(
            div()
                .flex()
                .flex_row()
                .min_w(px(0.0))
                .items_center()
                .gap(px(Space::MD))
                .child(
                    div()
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .child(if vcam_running && streaming {
                            "Publishing to"
                        } else {
                            "Virtual camera"
                        }),
                )
                .child(chip(vcam_label, vcam_tone))
                .children(
                    rules::vcam_consumer_summary(
                        state.vcam_state(),
                        streaming,
                        state.vcam_consumers(),
                    )
                    .map(|text| {
                        div()
                            .font_family(Fonts::SANS)
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child(text)
                    }),
                ),
        );

    if let Some((label, critical)) = state
        .core
        .as_ref()
        .and_then(|c| c.phone.as_ref())
        .and_then(|p| thermal_label(p.thermal))
    {
        row = row.child(chip(label, if critical { Tone::Danger } else { Tone::Warn }));
    }

    if streaming {
        row = row.child(
            div()
                .ml_auto()
                .flex()
                .flex_row()
                .flex_shrink_0()
                .items_center()
                .child(Button::new("Stop").on_click(|_window, _cx| {
                    push_effect(SideEffect::StopStream)
                })),
        );
    }

    row.into_any_element()
}

/* ------------------------------------------------------------------ */
/* Notice bar                                                          */
/* ------------------------------------------------------------------ */

/// The failure banners. Collapses to nothing when there are none.
pub fn notice_bar(state: &AppState) -> AnyElement {
    if state.notices.is_empty() {
        return div().into_any_element();
    }

    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(px(Space::MD))
        .px(px(Space::XL))
        .pt(px(Space::XL))
        .children(state.notices.iter().map(|notice| {
            let tone = notice.code.tone();
            let id = notice.id;
            let primary = notice.code.action_is_primary();
            let action = notice.code.action();

            let mut banner = div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(Space::PAD))
                .rounded(px(Radius::LG))
                .border_1()
                .border_color(tone.border())
                .bg(tone.fill())
                .px(px(Space::PAD))
                .py(px(Space::LG))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .font_family(Fonts::SANS)
                                .text_size(px(Type::TWO_SM))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tone.fg())
                                .child(notice.code.title()),
                        )
                        .child(
                            div()
                                .mt(px(Space::XS))
                                .font_family(Fonts::SANS)
                                .text_size(px(Type::XS))
                                .text_color(Text::BODY)
                                .child(notice.code.body(&notice.values)),
                        ),
                );

            if let Some(action) = action {
                banner = banner.child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(Space::MD))
                        .pt(px(Space::XS))
                        .child(action_button(action, primary))
                        .child(Button::ghost("x").on_click(move |_window, _cx| {
                            push_effect(SideEffect::DismissNotice(id))
                        })),
                );
            }

            banner.into_any_element()
        }))
        .into_any_element()
}

fn action_button(action: NoticeAction, primary: bool) -> AnyElement {
    let label = match action {
        NoticeAction::RefreshState => "Check again",
        NoticeAction::StopStream => "Stop streaming",
        NoticeAction::ArmPairing => "Show a pairing code",
        NoticeAction::StartStream => "Start streaming",
        NoticeAction::ConnectUsb => "Use the cable",
        NoticeAction::RestartVcam => "Turn the camera off and on",
        NoticeAction::Dismiss => "Dismiss",
    };
    let button = if primary { Button::primary(label) } else { Button::new(label) };
    button
        .on_click(move |_window, _cx| {
            let effect = match action {
                NoticeAction::RefreshState => SideEffect::RefreshState,
                NoticeAction::StopStream => SideEffect::StopStream,
                NoticeAction::ArmPairing => SideEffect::ArmPairing,
                NoticeAction::StartStream => SideEffect::StartStream,
                NoticeAction::ConnectUsb => SideEffect::ConnectUsb(None),
                NoticeAction::RestartVcam => SideEffect::SetVcam(true),
                // The prompt itself is the affordance, so dismissing is enough.
                NoticeAction::Dismiss => return,
            };
            push_effect(effect);
        })
        .into_any_element()
}

/// The toast. Feedback only, never a substitute for a state.
pub fn toast(message: &str) -> AnyElement {
    div()
        .absolute()
        .top(px(80.0))
        .left(px(0.0))
        .right(px(0.0))
        .flex()
        .justify_center()
        .child(
            div()
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(Accent::DEFAULT.alpha(0.6))
                .bg(SHELL.hover)
                .px(px(Space::PAD))
                .py(px(Space::LG))
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(Text::PRIMARY)
                .child(message.to_string()),
        )
        .into_any_element()
}

/* ------------------------------------------------------------------ */
/* The approval prompt                                                 */
/* ------------------------------------------------------------------ */

/// The "allow this phone?" dialog.
///
/// Decline is the safe side and is drawn first; Allow is disabled, not hidden,
/// after the request expires, so the user sees the control and learns why it
/// will not work.
pub fn pair_request_prompt(state: &AppState) -> AnyElement {
    let Some(prompt) = &state.pairing_prompt else {
        return div().into_any_element();
    };
    let elapsed = crate::views::state::now_ms() - prompt.raised_at_ms;
    let expired = elapsed >= crate::theme::Timing::APPROVAL_WAIT_MS as i64;

    let title = if expired { "This request has expired" } else { "Allow this phone to connect?" };
    let body = if expired {
        "The phone stopped waiting for an answer. Ask it to try again to get a new request."
    } else {
        "A phone on your network wants to use this PC as a webcam."
    };

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.7))
        .p(px(24.0))
        .child(
            div()
                .w_full()
                .max_w(px(384.0))
                .rounded(px(Radius::LG))
                .border_1()
                .border_color(Text::MUTED.alpha(0.6))
                .bg(SHELL.panel)
                .p(px(16.0))
                .child(
                    div()
                        .font_family(Fonts::SANS)
                        .text_size(px(14.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::PRIMARY)
                        .child(title),
                )
                .child(
                    div()
                        .mt(px(Space::MD))
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::XS))
                        .text_color(Text::MUTED)
                        .child(body),
                )
                .child(
                    div()
                        .mt(px(Space::PAD))
                        .rounded(px(Radius::MD))
                        .border_1()
                        .border_color(SHELL.border.alpha(0.6))
                        .bg(SHELL.recessed)
                        .px(px(Space::PAD))
                        .py(px(Space::XL))
                        .child(
                            div()
                                .font_family(Fonts::SANS)
                                .text_size(px(Type::TWO_SM))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(Text::PRIMARY)
                                .child(prompt.name.clone()),
                        )
                        .child(
                            div()
                                .mt(px(Space::XS))
                                .font_family(Fonts::SANS)
                                .text_size(px(Type::XS))
                                .text_color(Text::MUTED)
                                .child(prompt.model.clone()),
                        )
                        .child(
                            div()
                                .mt(px(Space::SM))
                                .font_family(Fonts::MONO)
                                .text_size(px(Type::TWO_XS))
                                .text_color(Text::MUTED)
                                .child(prompt.device_id.clone()),
                        ),
                )
                .child(
                    div()
                        .mt(px(Space::XL))
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::XS))
                        .text_color(Text::MUTED)
                        .child("Only approve if this is your phone. If you are not expecting a connection right now, choose Don't allow."),
                )
                .child(
                    div()
                        .mt(px(Space::LG))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_family(Fonts::MONO)
                                .text_size(px(Type::TWO_XS))
                                .text_color(Text::MUTED)
                                .child(if expired {
                                    "Expired".to_string()
                                } else {
                                    format!("Waiting {}s", elapsed / 1000)
                                }),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(Space::MD))
                                .child(Button::danger("Don't allow").on_click(|_window, _cx| {
                                    push_effect(SideEffect::ResolvePair(false))
                                }))
                                .child(
                                    Button::primary("Allow")
                                        .disabled(expired)
                                        .on_click(|_window, _cx| {
                                            push_effect(SideEffect::ResolvePair(true))
                                        }),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

/* ------------------------------------------------------------------ */
/* The rail                                                            */
/* ------------------------------------------------------------------ */

/// The four tabs and their panel.
///
/// The rail scrolls as one column so a long panel never pushes the picture out
/// of shape.
pub fn rail(state: &AppState, tab: Tab, narrow: bool) -> AnyElement {
    let width = if narrow {
        crate::theme::RAIL_WIDTH_NARROW
    } else {
        crate::theme::RAIL_WIDTH
    };

    let nav = div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .border_b_1()
        .border_color(SHELL.border.alpha(0.6))
        .children(Tab::ALL.iter().map(|entry| {
            let selected = *entry == tab;
            let target = *entry;
            div()
                .id(gpui::ElementId::Name(SharedString::from(entry.label())))
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .px(px(Space::PAD))
                .py(px(Space::XL))
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_SM))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if selected { Text::PRIMARY } else { Text::MUTED })
                .border_b_2()
                .border_color(if selected { Accent::DEFAULT } else { gpui::transparent_black() })
                .cursor_pointer()
                .hover(move |style| {
                    if selected {
                        style
                    } else {
                        style.text_color(Text::HEADING).border_color(SHELL.border)
                    }
                })
                .focus(|style| style.border_color(Accent::DEFAULT))
                .child(entry.label())
                .on_click(move |_event, _window, _cx| push_effect(SideEffect::SetTab(target)))
                .into_any_element()
        }));

    let body: Vec<AnyElement> = match tab {
        Tab::Setup => vec![connect_panel(state), paired_phone_panel(state)],
        Tab::Capture => vec![capture_panel(state)],
        Tab::Publish => vec![publish_panel(state), connection_panel(state)],
        Tab::Diagnostics => vec![diagnostics_panel(state), preferences_panel(state)],
    };

    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w(px(width))
        .overflow_hidden()
        .rounded(px(Radius::LG))
        .border_1()
        .border_color(SHELL.border.alpha(0.6))
        .bg(SHELL.panel)
        .child(nav)
        .child(
            div()
                .id(gpui::ElementId::Name(SharedString::from("rail-scroll")))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .gap(px(Space::XL))
                .p(px(Space::XL))
                // The bottom padding is what stops the last panel being clipped
                // by the viewport edge.
                .pb(px(Space::RAIL_BOTTOM))
                .children(body),
        )
        .into_any_element()
}

/* ------------------------------------------------------------------ */
/* Setup                                                               */
/* ------------------------------------------------------------------ */

/// Pairing. Describes where the button is; it does not provide one, because the
/// stage owns that action and offering the same job twice is what made the
/// first-run screen feel like a coin toss.
pub fn connect_panel(state: &AppState) -> AnyElement {
    let pairing_open = state.core.as_ref().map(|c| c.pairing.open).unwrap_or(false);
    let connected = state.has_device();

    let mut panel = Panel::new("Pairing");
    if pairing_open {
        panel = panel.action(Button::new("Cancel").on_click(|_window, _cx| {
            push_effect(SideEffect::DisarmPairing)
        }));
    }

    let mut children: Vec<AnyElement> = Vec::new();

    if connected {
        children.push(tinted_box(
            Tone::Ok,
            vec![div()
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
                .text_color(Text::HEADING)
                .child("A phone is connected. Pairing another one here will not disturb it.")
                .into_any_element()],
        ));
    }

    if pairing_open {
        children.push(
            div()
                .mt(px(Space::LG))
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(Accent::DEFAULT.alpha(0.5))
                .bg(Accent::DEFAULT.alpha(0.05))
                .px(px(Space::XL))
                .py(px(Space::LG))
                .child(
                    div()
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_XS))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Accent::DEFAULT)
                        .child("PAIRING OPEN"),
                )
                .child(
                    div()
                        .mt(px(Space::SM))
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::XS))
                        .text_color(Text::BODY)
                        .child("The code is in the preview area, on the left. Point your phone at it."),
                )
                .into_any_element(),
        );
    } else {
        children.push(prose(if connected {
            "Press Pair another phone in the preview area to show a code."
        } else {
            "Press Show a pairing code in the preview area. Your phone scans it once, and this PC remembers it from then on."
        }));
    }

    children.push(section(
        "Or use a cable",
        vec![
            usb_list(state),
            div()
                .mt(px(Space::MD))
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
                .text_color(Text::MUTED)
                .child("A cable is worth trying when Wi-Fi is unreliable. It needs USB debugging switched on in the phone's developer options.")
                .into_any_element(),
        ],
    ));

    panel.children(children).into_element()
}

/// The cable list, which has four disjoint states.
fn usb_list(state: &AppState) -> AnyElement {
    let usb = state.core.as_ref().map(|c| &c.usb);
    let adb = usb.map(|u| u.adb_available);
    let device_count = usb.map(|u| u.devices.len()).unwrap_or(0);
    let readiness = rules::usb_readiness(adb, device_count);

    if let rules::UsbReadiness::Device = readiness {
        let forwarding = usb.map(|u| u.forwarding).unwrap_or(false);
        let devices = usb.map(|u| u.devices.clone()).unwrap_or_default();
        let mut column = div()
            .flex()
            .flex_col()
            .mb(px(Space::MD))
            .font_family(Fonts::SANS)
            .text_size(px(Type::XS))
            .text_color(Text::MUTED)
            .child(if forwarding {
                "The cable tunnel is open. Open PhoneCam on the phone to connect."
            } else if device_count == 1 {
                "A phone is on the cable, not connected yet."
            } else {
                "Phones on the cable, none connected yet."
            });

        for device in devices {
            let label = if device.model.is_empty() { device.serial.clone() } else { device.model.clone() };
            let serial_for_connect = device.serial.clone();
            let id_for_btn = device.serial.clone();
            column = column.child(
                div()
                    .mt(px(Space::MD))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(Space::LG))
                    .rounded(px(Radius::MD))
                    .border_1()
                    .border_color(SHELL.border.alpha(0.6))
                    .bg(SHELL.raised.alpha(0.6))
                    .px(px(Space::LG))
                    .py(px(Space::MD))
                    .child(
                        div()
                            .min_w(px(0.0))
                            .font_family(Fonts::SANS)
                            .text_size(px(Type::TWO_SM))
                            .text_color(Text::HEADING)
                            .child(label),
                    )
                    .child(
                        div()
                            .font_family(Fonts::MONO)
                            .text_size(px(Type::XS))
                            .text_color(Text::MUTED)
                            .child(format!("serial {}", device.serial)),
                    )
                    .child(if forwarding {
                        Button::danger("Close tunnel").on_click(|_window, _cx| {
                            push_effect(SideEffect::DisconnectUsb)
                        })
                        .into_any_element()
                    } else {
                        // Not primary: the cable is the fallback path.
                        pill_button(
                            format!("usb-use-{id_for_btn}"),
                            "Use this phone",
                            false,
                            false,
                            move |_window, _cx| {
                                push_effect(SideEffect::ConnectUsb(Some(serial_for_connect.clone())))
                            },
                        )
                    }),
            );
        }

        column = column.child(
            div()
                .mt(px(Space::MD))
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(Space::MD))
                .child(Button::new("Check again").on_click(|_window, _cx| {
                    push_effect(SideEffect::RefreshUsb)
                }))
                .child(Button::new("Restart ADB").on_click(|_window, _cx| {
                    push_effect(SideEffect::RestartAdb)
                })),
        );

        return column.into_any_element();
    }

    let (chip_label, body): (&'static str, AnyElement) = match readiness {
        rules::UsbReadiness::ToolsMissing => (
            "Cable tools missing",
            div()
                .mt(px(Space::MD))
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
                .text_color(Text::MUTED)
                .child("PhoneCam ships Google's Android platform tools, but they are missing from this copy. Reinstall PhoneCam, or install them yourself and check again.")
                .into_any_element(),
        ),
        rules::UsbReadiness::Unknown => (
            "Not checked yet",
            div()
                .mt(px(Space::MD))
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
                .text_color(Text::MUTED)
                .child("Connect the phone with a data cable and turn on USB debugging, then check again.")
                .into_any_element(),
        ),
        _ => (
            "No phone detected",
            usb_debugging_guide(),
        ),
    };

    let mut actions = div()
        .mt(px(Space::MD))
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(Space::MD));

    if matches!(readiness, rules::UsbReadiness::ToolsMissing) {
        actions = actions.child(Button::new("Download platform-tools").on_click(|_window, _cx| {
            push_effect(SideEffect::OpenPlatformToolsUrl)
        }));
    }

    actions = actions.child(Button::new("Check again").on_click(|_window, _cx| {
        push_effect(SideEffect::RefreshUsb)
    }));

    if !matches!(readiness, rules::UsbReadiness::ToolsMissing) {
        actions = actions.child(Button::new("Restart ADB").on_click(|_window, _cx| {
            push_effect(SideEffect::RestartAdb)
        }));
    }

    div()
        .flex()
        .flex_col()
        .rounded(px(Radius::MD))
        .bg(SHELL.raised)
        .px(px(Space::LG))
        .py(px(Space::MD))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(Space::LG))
                .child(
                    div()
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_SM))
                        .text_color(Text::MUTED)
                        .child("Cable"),
                )
                .child(chip(chip_label, Tone::Neutral)),
        )
        .child(body)
        .child(actions)
        .into_any_element()
}

/// The step-by-step instructions for turning on USB debugging.
/// Matching upstream commit 8194295.
pub const USB_DEBUGGING_STEPS: [&str; 4] = [
    "Settings \u{203a} About phone \u{203a} tap OS version 7 times to unlock Developer options.",
    "Settings \u{203a} Additional settings \u{203a} Developer options \u{203a} turn on USB debugging.",
    "On MIUI/HyperOS, also enable Install via USB and USB debugging (Security settings); both must be on for the cable to work.",
    "Plug in the cable, unlock the phone, and tap Allow on the debugging prompt.",
];

/// The full path, not just the name of the setting: "turn on USB
/// debugging" assumes the reader knows where developer options live,
/// and on MIUI/HyperOS two extra switches also gate adb. Numbered so
/// the order is explicit; step 3 is the one users skip on Xiaomi.
pub fn usb_debugging_guide() -> AnyElement {
    div()
        .mt(px(Space::MD))
        .flex()
        .flex_col()
        .gap(px(Space::XS))
        .font_family(Fonts::SANS)
        .text_size(px(Type::XS))
        .text_color(Text::MUTED)
        .children(USB_DEBUGGING_STEPS.iter().enumerate().map(|(i, step)| {
            div()
                .flex()
                .flex_row()
                .gap(px(Space::SM))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(Text::BODY)
                        .child(format!("{}.", i + 1)),
                )
                .child(
                    div()
                        .flex_1()
                        .child(*step),
                )
        }))
        .into_any_element()
}

/// The paired phone, and the way to forget it.
///
/// Returns nothing at all when there is no phone and none has ever been paired:
/// a card explaining a phone that would be remembered, on a screen where none
/// has been, is filler.
pub fn paired_phone_panel(state: &AppState) -> AnyElement {
    let device = state.core.as_ref().and_then(|c| c.device.as_ref());
    let stored_id = state.settings.last_device_id.clone();

    if device.is_none() && stored_id.is_none() {
        return div().into_any_element();
    }

    let name = device.map(|d| d.name.clone()).unwrap_or_else(|| "this phone".to_string());
    let facts = state.device_facts();
    let connected = device.is_some();

    let panel = Panel::new(name.clone()).action(chip(
        if connected { "Connected" } else { "Not connected" },
        if connected { Tone::Ok } else { Tone::Neutral },
    ));

    let mut children: Vec<AnyElement> = Vec::new();
    if let Some(device) = device {
        if let Some(model) = facts.model {
            children.push(key_value("Model", model, false));
        }
        if let Some(version) = facts.android_version {
            children.push(key_value("Android", version.to_string(), false));
        }
        children.push(key_value("Device id", device.device_id.clone(), true));
    } else if let Some(id) = stored_id {
        children.push(key_value("Device id", id, true));
    }

    children.push(
        div()
            .mt(px(Space::PAD))
            .rounded(px(Radius::MD))
            .border_1()
            .border_color(Status::DANGER.alpha(0.4))
            .bg(Status::DANGER.alpha(0.05))
            .px(px(Space::LG))
            .py(px(Space::MD))
            .child(
                div()
                    .font_family(Fonts::SANS)
                    .text_size(px(Type::XS))
                    .text_color(Text::MUTED)
                    .child(format!(
                        "Forgetting {name} deletes its credentials from this PC. It will need a new pairing code."
                    )),
            )
            .child(
                div()
                    .mt(px(Space::LG))
                    .child(Button::danger(format!("Forget {name}")).on_click(|_window, _cx| {
                        push_effect(SideEffect::ForgetDevice)
                    })),
            )
            .into_any_element(),
    );

    panel.children(children).into_element()
}

/* ------------------------------------------------------------------ */
/* Capture                                                             */
/* ------------------------------------------------------------------ */

/// What the phone captures.
pub fn capture_panel(state: &AppState) -> AnyElement {
    let streaming = state.is_streaming();
    let panel = Panel::new("Phone camera").action(chip(
        if streaming { "Streaming" } else { "Idle" },
        if streaming { Tone::Ok } else { Tone::Neutral },
    ));

    if !state.status.connected || !state.has_device() {
        return panel
            .child(empty_state(
                "No phone is connected.",
                "Connect one from the Setup tab to see its camera controls.",
            ))
            .into_element();
    }

    let cameras = state.capabilities.as_ref().map(|c| c.cameras.clone()).unwrap_or_default();

    if cameras.is_empty() {
        return panel
            .child(empty_state(
                "Waiting for this phone's camera list.",
                "The phone has not reported its capabilities yet. It retries on its own.",
            ))
            .into_element();
    }

    let selected = state
        .pending_camera_id
        .clone()
        .or_else(|| {
            state
                .core
                .as_ref()
                .and_then(|c| c.phone.as_ref())
                .map(|p| p.camera.clone())
                .filter(|c| !c.is_empty())
        })
        .unwrap_or_else(|| state.settings.camera.clone());

    let selected_cam = cameras
        .iter()
        .find(|c| c.id == selected || c.facing.as_str() == selected)
        .or_else(|| cameras.first());

    let zoom_supported = selected_cam.map(|c| c.zoom).unwrap_or(false);
    let torch_supported = selected_cam.map(|c| c.torch).unwrap_or(false);
    let phone = state.core.as_ref().and_then(|c| c.phone.as_ref());
    let torch_on = phone.map(|p| p.torch).unwrap_or(false);
    let current_fps_opt = if state.settings.fps == 0 {
        None
    } else {
        Some(state.settings.fps)
    };
    let current_bitrate = state.settings.bitrate;

    let mut children: Vec<AnyElement> = Vec::new();

    // Quick camera ID cycle list: known camera IDs plus fallback 0..=4.
    let mut cycle_ids: Vec<String> = cameras.iter().map(|c| c.id.clone()).collect();
    for fallback in ["0", "1", "2", "3", "4"] {
        if !cycle_ids.iter().any(|id| id == fallback) {
            cycle_ids.push(fallback.to_string());
        }
    }
    let resolved_id = selected_cam
        .map(|c| c.id.as_str())
        .unwrap_or(selected.as_str())
        .to_string();
    let current_idx = cycle_ids.iter().position(|id| id == &resolved_id).unwrap_or(0);
    let prev_id = cycle_ids[(current_idx + cycle_ids.len() - 1) % cycle_ids.len()].clone();
    let next_id = cycle_ids[(current_idx + 1) % cycle_ids.len()].clone();

    children.push(section(
        "Which camera",
        vec![
            camera_tiles(&cameras, &selected),
            crate::views::widgets::field(
                "Camera id",
                Some("If the list missed one".into()),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(Space::SM))
                    .child(pill_button(
                        "cam-id-prev",
                        "<",
                        false,
                        false,
                        move |_window, _cx| push_effect(SideEffect::SetCamera(prev_id.clone())),
                    ))
                    .child(
                        div()
                            .rounded(px(Radius::MD))
                            .border_1()
                            .border_color(SHELL.border)
                            .bg(SHELL.base)
                            .px(px(Space::LG))
                            .py(px(Space::SM))
                            .font_family(Fonts::MONO)
                            .text_size(px(Type::XS))
                            .text_color(Text::HEADING)
                            .child(format!("id {resolved_id}")),
                    )
                    .child(pill_button(
                        "cam-id-next",
                        ">",
                        false,
                        false,
                        move |_window, _cx| push_effect(SideEffect::SetCamera(next_id.clone())),
                    ))
                    .children(["0", "1", "2", "3"].into_iter().map(|id| {
                        let active = resolved_id == id;
                        pill_button(
                            format!("cam-id-{id}"),
                            id,
                            active,
                            false,
                            move |_window, _cx| {
                                push_effect(SideEffect::SetCamera(id.to_string()))
                            },
                        )
                    }))
                    .into_any_element(),
                None,
            ),
        ],
    ));

    let res_options = resolution_presets(
        selected_cam.map(|c| c.resolutions.as_slice()).unwrap_or(&[]),
        &state.settings.resolution,
    );
    let res_hint: SharedString = phone
        .map(|p| p.actual_resolution.clone())
        .filter(|s| !s.is_empty())
        .map(|actual| SharedString::from(format!("{} ({actual})", resolution_label_of(&actual))))
        .unwrap_or_else(|| SharedString::from(resolution_label_of(&state.settings.resolution)));

    let fps_options = fps_presets(
        selected_cam.map(|c| c.fps.as_slice()).unwrap_or(&[]),
        current_fps_opt,
    );
    let fps_hint: Option<SharedString> = if current_fps_opt.is_none() {
        phone
            .map(|p| p.actual_fps)
            .filter(|&f| f > 0)
            .map(|f| SharedString::from(format!("{f} fps")))
            .or_else(|| Some("automatic".into()))
    } else {
        None
    };

    let bitrate_dec = current_bitrate.saturating_sub(1_000_000).max(1_000_000);
    let bitrate_inc = (current_bitrate + 1_000_000).min(20_000_000);

    children.push(section(
        "What it captures",
        vec![
            crate::views::widgets::field(
                "Resolution",
                Some(res_hint),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::SM))
                    .children(res_options.into_iter().map(|res| {
                        let active = state.settings.resolution == res;
                        let label = if res == "auto" {
                            "Auto".to_string()
                        } else {
                            format!("{} ({})", resolution_label_of(&res), res)
                        };
                        let id = format!("res-{res}");
                        pill_button(id, label, active, false, move |_window, _cx| {
                            push_effect(SideEffect::SetResolution(res.clone()))
                        })
                    }))
                    .into_any_element(),
                Some("The phone picks the nearest it supports and tells you what it used.".into()),
            ),
            crate::views::widgets::field(
                "Frame rate",
                fps_hint,
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::SM))
                    .children(fps_options.into_iter().map(|fps_opt| {
                        let active = current_fps_opt == fps_opt;
                        let label = match fps_opt {
                            None => "Auto".to_string(),
                            Some(f) => format!("{f} fps"),
                        };
                        let id = match fps_opt {
                            None => "fps-auto".to_string(),
                            Some(f) => format!("fps-{f}"),
                        };
                        pill_button(id, label, active, false, move |_window, _cx| {
                            push_effect(SideEffect::SetVideo {
                                fps: fps_opt,
                                bitrate: current_bitrate,
                            })
                        })
                    }))
                    .into_any_element(),
                None,
            ),
            crate::views::widgets::field(
                "Bitrate",
                Some(format!("{:.1} Mbps", current_bitrate as f64 / 1e6).into()),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_center()
                    .gap(px(Space::SM))
                    .child(pill_button(
                        "bitrate-dec",
                        "-",
                        false,
                        current_bitrate <= 1_000_000,
                        move |_window, _cx| {
                            push_effect(SideEffect::SetVideo {
                                fps: current_fps_opt,
                                bitrate: bitrate_dec,
                            })
                        },
                    ))
                    .children([2u64, 4, 8, 12, 16, 20].into_iter().map(|mbps| {
                        let bps = mbps * 1_000_000;
                        let active = current_bitrate == bps;
                        pill_button(
                            format!("bitrate-{mbps}m"),
                            format!("{mbps}M"),
                            active,
                            false,
                            move |_window, _cx| {
                                push_effect(SideEffect::SetVideo {
                                    fps: current_fps_opt,
                                    bitrate: bps,
                                })
                            },
                        )
                    }))
                    .child(pill_button(
                        "bitrate-inc",
                        "+",
                        false,
                        current_bitrate >= 20_000_000,
                        move |_window, _cx| {
                            push_effect(SideEffect::SetVideo {
                                fps: current_fps_opt,
                                bitrate: bitrate_inc,
                            })
                        },
                    ))
                    .into_any_element(),
                Some("Higher is sharper and needs more Wi-Fi. 8 Mbps is a good 1080p default.".into()),
            ),
        ],
    ));

    let zoom_element = if zoom_supported {
        let min_zoom = selected_cam
            .map(|c| if c.min_zoom > 0.0 { c.min_zoom } else { 1.0 })
            .unwrap_or(1.0);
        let max_zoom = selected_cam
            .map(|c| if c.max_zoom > min_zoom { c.max_zoom } else { min_zoom + 1.0 })
            .unwrap_or(8.0);
        let live_zoom = phone
            .map(|p| p.zoom)
            .filter(|z| *z > 0.0)
            .unwrap_or(1.0)
            .clamp(min_zoom, max_zoom);
        let zoom_dec = ((live_zoom - 0.5).max(min_zoom) * 10.0).round() / 10.0;
        let zoom_inc = ((live_zoom + 0.5).min(max_zoom) * 10.0).round() / 10.0;
        let zoom_presets: Vec<f32> = [1.0f32, 1.5, 2.0, 3.0, 5.0]
            .into_iter()
            .filter(|&z| z >= min_zoom - 0.01 && z <= max_zoom + 0.01)
            .map(|z| z.clamp(min_zoom, max_zoom))
            .collect();

        crate::views::widgets::field(
            "Zoom",
            Some(format!("{live_zoom:.1}x of {max_zoom:.1}x").into()),
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(Space::SM))
                .child(pill_button(
                    "zoom-dec",
                    "-",
                    false,
                    live_zoom <= min_zoom + 0.01,
                    move |_window, _cx| push_effect(SideEffect::SetZoom(zoom_dec)),
                ))
                .children(zoom_presets.into_iter().map(|z| {
                    let active = (live_zoom - z).abs() < 0.05;
                    let label = format!("{z:.1}x");
                    pill_button(
                        format!("zoom-{z:.1}"),
                        label,
                        active,
                        false,
                        move |_window, _cx| push_effect(SideEffect::SetZoom(z)),
                    )
                }))
                .child(pill_button(
                    "zoom-inc",
                    "+",
                    false,
                    live_zoom >= max_zoom - 0.01,
                    move |_window, _cx| push_effect(SideEffect::SetZoom(zoom_inc)),
                ))
                .into_any_element(),
            None,
        )
    } else {
        crate::views::widgets::unavailable_field("Zoom", "This camera does not support zoom.")
    };

    children.push(section(
        "How it behaves",
        vec![
            zoom_element,
            if torch_supported {
                Toggle::new("Torch", torch_on)
                    .on_change(|enabled, _window, _cx| push_effect(SideEffect::SetTorch(enabled)))
                    .into_element()
            } else {
                crate::views::widgets::unavailable_field(
                    "Torch",
                    "This camera has no torch. Front cameras and many external cameras do not.",
                )
            },
        ],
    ));

    let auto = state.settings.quality_mode == phonecam_protocol::QualityMode::Auto;
    children.push(section(
        "While streaming",
        vec![
            Toggle::new("Adjust quality automatically", auto)
                .on_change(|enabled, _window, _cx| {
                    push_effect(SideEffect::SetQualityMode(enabled))
                })
                .into_element(),
            prose(if auto {
                "The phone steps the resolution down on its own if the link struggles, and back up when it recovers."
            } else {
                "Your resolution, frame rate and bitrate above are used exactly as set."
            }),
            Toggle::new(
                "Turn the phone screen off",
                state.settings.screen_mode == phonecam_protocol::ScreenMode::Black,
            )
            .on_change(|off, _window, _cx| {
                push_effect(SideEffect::SetScreenMode(if off {
                    phonecam_protocol::ScreenMode::Black
                } else {
                    phonecam_protocol::ScreenMode::KeepOn
                }))
            })
            .into_element(),
            prose("The phone screen is not part of the stream. Turning it off saves battery on a long session."),
        ],
    ));

    panel.children(children).into_element()
}

fn camera_tiles(
    cameras: &[phonecam_protocol::CameraCapability],
    selected: &str,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(Space::LG))
        .mb(px(Space::FIELD_GAP))
        .children(cameras.iter().map(|camera| {
            let is_selected = camera.id == selected || camera.facing.as_str() == selected;
            let id = camera.id.clone();
            div()
                .id(gpui::ElementId::Name(SharedString::from(format!("camera-{id}"))))
                .flex()
                .flex_col()
                .min_w(px(120.0))
                .items_center()
                .justify_center()
                .gap(px(Space::SM))
                .rounded(px(Radius::LG))
                .border_1()
                .border_color(if is_selected { Accent::DEFAULT } else { SHELL.control })
                .bg(if is_selected { Accent::DEFAULT.alpha(0.10) } else { SHELL.raised })
                .px(px(16.0))
                .py(px(Space::PAD))
                .text_color(if is_selected { Text::PRIMARY } else { Text::BODY })
                .cursor_pointer()
                .child(
                    div()
                        .font_family(Fonts::SANS)
                        .text_size(px(14.0))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .child(camera.facing.as_str().to_string()),
                )
                .child(
                    div()
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::XS))
                        .opacity(0.7)
                        .child(if is_selected { "In use" } else { "Tap to use" }),
                )
                .on_click(move |_event, _window, _cx| {
                    push_effect(SideEffect::SetCamera(id.clone()))
                })
                .into_any_element()
        }))
        .into_any_element()
}

/* ------------------------------------------------------------------ */
/* Publish                                                             */
/* ------------------------------------------------------------------ */

/// The camera other apps open.
pub fn publish_panel(state: &AppState) -> AnyElement {
    let vcam_error = state.core.as_ref().and_then(|c| c.vcam.as_ref()).and_then(|v| v.error.clone());
    let state_label = state.vcam_state().unwrap_or("stopped");
    let running = state_label == "running";
    let starting = state_label == "starting";
    let streaming = state.is_streaming();

    let panel = Panel::new("Publish to other apps").action(chip(
        match state_label {
            "starting" => "Starting",
            // Running is deliberately NOT "Live": it means registered with
            // Windows and having a frame source, which is true with or without a
            // picture.
            "running" => "Registered",
            "error" => "Failed",
            _ => "Off",
        },
        if running {
            Tone::Ok
        } else if state_label == "error" {
            Tone::Danger
        } else {
            Tone::Neutral
        },
    ));

    let mut children: Vec<AnyElement> = Vec::new();

    children.push(
        Toggle::new("PhoneCam Camera", running || starting)
            .detail(rules::vcam_usage_detail(
                state.vcam_state(),
                streaming,
                state.vcam_consumers(),
            ))
            .on_change(|enabled, _window, _cx| push_effect(SideEffect::SetVcam(enabled)))
            .into_element(),
    );

    children.push(match vcam_error {
        Some(error) => div()
            .font_family(Fonts::SANS)
            .text_size(px(Type::XS))
            .text_color(Status::DANGER_TEXT)
            .child(error)
            .into_any_element(),
        None => prose(if running && streaming {
            "Pick PhoneCam Camera in Meet, OBS or any other app. It is carrying your phone right now."
        } else if running {
            "Registered and waiting. It shows a stand-in until the phone stream starts."
        } else {
            "Turn this on to make the phone selectable in other apps."
        }),
    });

    let sideways = state.settings.output_rotation == 90 || state.settings.output_rotation == 270;

    let next_mirror = !state.settings.output_mirror;
    let output_sizes: [(u32, u32); 5] = [
        (1920, 1080),
        (1280, 720),
        (2560, 1440),
        (3840, 2160),
        (854, 480),
    ];

    children.push(section(
        "Framing",
        vec![
            crate::views::widgets::field(
                "Output size",
                Some(format!("{} × {}", state.settings.output_width, state.settings.output_height).into()),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::SM))
                    .children(output_sizes.into_iter().map(|(w, h)| {
                        let active =
                            state.settings.output_width == w && state.settings.output_height == h;
                        pill_button(
                            format!("out-size-{w}x{h}"),
                            format!("{w}x{h}"),
                            active,
                            false,
                            move |_window, _cx| push_effect(SideEffect::SetOutputSize(w, h)),
                        )
                    }))
                    .into_any_element(),
                Some(
                    "What apps receive. A sideways phone is fitted inside this frame, not stretched."
                        .into(),
                ),
            ),
            div()
                .flex()
                .flex_row()
                .items_end()
                .gap(px(Space::LG))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(crate::views::widgets::field(
                            "Rotation",
                            Some(rotation_label(state.settings.output_rotation).into()),
                            div()
                                .flex()
                                .flex_row()
                                .flex_wrap()
                                .gap(px(Space::SM))
                                .children([0u32, 90, 180, 270].into_iter().map(|deg| {
                                    let active = state.settings.output_rotation == deg;
                                    pill_button(
                                        format!("rot-{deg}"),
                                        format!("{deg}°"),
                                        active,
                                        false,
                                        move |_window, _cx| {
                                            push_effect(SideEffect::SetRotation(deg))
                                        },
                                    )
                                }))
                                .into_any_element(),
                            Some("Rotate until the preview looks right.".into()),
                        )),
                )
                .child(
                    div()
                        .mb(px(Space::FIELD_GAP))
                        .flex_shrink_0()
                        .child(
                            Button::new(if state.settings.output_mirror {
                                "Mirrored"
                            } else {
                                "Mirror"
                            })
                            .selected(state.settings.output_mirror)
                            .on_click(move |_window, _cx| {
                                push_effect(SideEffect::SetMirror(next_mirror))
                            }),
                        ),
                )
                .into_any_element(),
            // A quarter turn is fitted, not stretched, so bars appear. Saying so
            // stops it reading as a crop.
            sideways
                .then(|| {
                    div()
                        .mb(px(Space::FIELD_GAP))
                        .rounded(px(Radius::MD))
                        .border_1()
                        .border_color(SHELL.border.alpha(0.6))
                        .bg(SHELL.raised.alpha(0.6))
                        .px(px(Space::LG))
                        .py(px(Space::MD))
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::XS))
                        .text_color(Text::MUTED)
                        .child(format!(
                            "A sideways picture is fitted inside the {} x {} frame at its own shape, so black bars appear on the sides. Nothing is cropped.",
                            state.settings.output_width, state.settings.output_height
                        ))
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
        ],
    ));

    children.push(section(
        "Right now",
        vec![key_value(
            "What apps receive",
            if streaming {
                "Your phone, live"
            } else if running {
                "A stand-in, until you start streaming"
            } else {
                "Nothing"
            },
            false,
        )],
    ));

    panel.children(children).into_element()
}

fn rotation_label(rotation: u32) -> &'static str {
    match rotation {
        90 => "90 degrees clockwise",
        180 => "180 degrees",
        270 => "270 degrees clockwise",
        _ => "Upright",
    }
}

/// The connection's own numbers. Informational, never a control.
pub fn connection_panel(state: &AppState) -> AnyElement {
    let transport = state.core.as_ref().map(|c| c.transport.as_str()).unwrap_or("none");
    let forwarding = state.core.as_ref().map(|c| c.usb.forwarding).unwrap_or(false);

    let link_label = match transport {
        "usb" => "USB cable",
        "lan" => "Wi-Fi",
        _ if forwarding => "Cable ready, waiting for the phone",
        _ => "Not connected",
    };
    let link_tone = if transport != "none" {
        Some(Tone::Ok)
    } else if forwarding {
        Some(Tone::Warn)
    } else {
        None
    };

    let usb_chip = if forwarding {
        chip("Forwarding", Tone::Ok)
    } else if transport != "none" {
        chip("Ready", Tone::Neutral)
    } else {
        chip("No phone", Tone::Neutral)
    };

    let mut usb_row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(Space::MD))
        .child(usb_chip);

    if forwarding {
        usb_row = usb_row.child(Button::danger("Stop forwarding").on_click(|_window, _cx| {
            push_effect(SideEffect::DisconnectUsb)
        }));
    }

    Panel::new("Connection")
        .child(key_value_toned("Phone link", link_label, link_tone))
        .child(key_value_toned(
            "Service socket",
            if state.status.connected { "Connected" } else { "Retrying" },
            Some(if state.status.connected { Tone::Ok } else { Tone::Warn }),
        ))
        .child(key_value(
            "Received",
            crate::views::state::format_bytes(state.status.bytes_in),
            false,
        ))
        .child(key_value("Handshakes", state.status.connects.to_string(), false))
        .child(section("USB", vec![usb_row.into_any_element()]))
        .into_element()
}

/* ------------------------------------------------------------------ */
/* Diagnostics                                                         */
/* ------------------------------------------------------------------ */

/// The numbers, and the log.
pub fn diagnostics_panel(state: &AppState) -> AnyElement {
    let panel = Panel::new("Diagnostics")
        .action(Button::new("Refresh").on_click(|_window, _cx| {
            push_effect(SideEffect::RefreshState)
        }))
        .action(Button::new("Export").on_click(|_window, _cx| {
            push_effect(SideEffect::ExportDiagnostics)
        }))
        .action(Button::new("Copy report").on_click(|_window, _cx| {
            push_effect(SideEffect::CopyDiagnosticsReport)
        }));

    let stats = state.core.as_ref().map(|c| &c.stats);
    let readout = rules::throughput_readout(stats, state.phase(), crate::theme::EMPTY_VALUE);
    let phase_label = state
        .core
        .as_ref()
        .map(|c| crate::views::state::stream_phase_label(c.phase))
        .unwrap_or("Idle");

    let mut children: Vec<AnyElement> = Vec::new();

    children.push(section(
        "Live",
        vec![
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .justify_between()
                .gap(px(Space::LG))
                .child(
                    div()
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_XS))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::BODY)
                        .child("Throughput"),
                )
                .child(chip(
                    phase_label,
                    if state.is_streaming() { Tone::Ok } else { Tone::Neutral },
                ))
                .into_any_element(),
            div()
                .grid()
                .grid_cols(2)
                .gap_x(px(16.0))
                .gap_y(px(Space::XL))
                .child(metric("Frames per second", readout.fps, None))
                .child(metric("Bitrate", readout.bitrate, None))
                .child(metric("Latency", readout.latency, None))
                .child(metric("Dropped", readout.dropped, None))
                .into_any_element(),
        ],
    ));

    let device = state.core.as_ref().and_then(|c| c.device.as_ref());
    let facts = state.device_facts();
    let phone_rows: Vec<AnyElement> = match device {
        Some(device) => {
            let mut rows = vec![key_value("Name", device.name.clone(), false)];
            if let Some(model) = facts.model {
                rows.push(key_value("Model", model, false));
            }
            rows.push(key_value("Device id", device.device_id.clone(), true));
            if let Some(version) = facts.android_version {
                rows.push(key_value("Android", version.to_string(), false));
            }
            rows.push(key_value(
                "Cameras",
                state
                    .capabilities
                    .as_ref()
                    .map(|c| c.cameras.len().to_string())
                    .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string()),
                false,
            ));
            rows.push(key_value(
                "App version",
                state
                    .capabilities
                    .as_ref()
                    .map(|c| c.device.app_version.clone())
                    .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string()),
                false,
            ));
            rows
        }
        None => vec![prose(
            "Nothing connected. Pair a phone from Setup and its details appear here.",
        )],
    };
    children.push(section("Connected phone", phone_rows));

    children.push(section(
        "Being published",
        vec![
            key_value("Mirror", if state.settings.output_mirror { "On" } else { "Off" }, false),
            key_value("Rotation", format!("{}°", state.settings.output_rotation), false),
            key_value(
                "Output",
                format!("{}x{}", state.settings.output_width, state.settings.output_height),
                false,
            ),
            key_value(
                "Decoded",
                state
                    .frame_size
                    .map(|(w, h)| format!("{w}x{h}"))
                    .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string()),
                false,
            ),
            key_value(
                "Phone output",
                state
                    .core
                    .as_ref()
                    .and_then(|c| c.phone.as_ref())
                    .map(|p| p.actual_resolution.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string()),
                false,
            ),
        ],
    ));

    let matching_logs = filtered_logs(state);
    let log_body: Vec<AnyElement> = if state.logs.is_empty() {
        vec![div()
            .font_family(Fonts::SANS)
            .text_size(px(Type::XS))
            .text_color(Text::MUTED)
            .child("Nothing logged yet. Lines appear as the service and the phone report in.")
            .into_any_element()]
    } else if matching_logs.is_empty() {
        vec![div()
            .font_family(Fonts::SANS)
            .text_size(px(Type::XS))
            .text_color(Text::MUTED)
            .child("No lines match these filters.")
            .into_any_element()]
    } else {
        matching_logs
            .into_iter()
            .map(|line| {
                div()
                    .flex()
                    .flex_row()
                    .gap(px(Space::MD))
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(Fonts::MONO)
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child(format_log_time(line.ts_ms)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(Fonts::MONO)
                            .text_size(px(Type::TWO_XS))
                            .text_color(log_level_colour(line.level))
                            .child(level_label(line.level)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(Fonts::MONO)
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child(line.source.label()),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .font_family(Fonts::MONO)
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::BODY)
                            .child(line.message.clone()),
                    )
                    .into_any_element()
            })
            .collect()
    };

    let level_filters: [(Option<phonecam_protocol::LogLevel>, &str, &str); 5] = [
        (None, "All", "log-filter-lvl-all"),
        (Some(phonecam_protocol::LogLevel::Error), "Error", "log-filter-lvl-error"),
        (Some(phonecam_protocol::LogLevel::Warn), "Warn", "log-filter-lvl-warn"),
        (Some(phonecam_protocol::LogLevel::Info), "Info", "log-filter-lvl-info"),
        (Some(phonecam_protocol::LogLevel::Debug), "Debug", "log-filter-lvl-debug"),
    ];
    let source_filters: [(Option<LogSource>, &str, &str); 5] = [
        (None, "All", "log-filter-src-all"),
        (Some(LogSource::Core), "Core", "log-filter-src-core"),
        (Some(LogSource::Media), "Media", "log-filter-src-media"),
        (Some(LogSource::App), "App", "log-filter-src-app"),
        (Some(LogSource::Phone), "Phone", "log-filter-src-phone"),
    ];
    let runtime_levels: [(phonecam_protocol::LogLevel, &str, &str); 5] = [
        (phonecam_protocol::LogLevel::Error, "Error", "log-lvl-error"),
        (phonecam_protocol::LogLevel::Warn, "Warn", "log-lvl-warn"),
        (phonecam_protocol::LogLevel::Info, "Info", "log-lvl-info"),
        (phonecam_protocol::LogLevel::Debug, "Debug", "log-lvl-debug"),
        (phonecam_protocol::LogLevel::Trace, "Trace", "log-lvl-trace"),
    ];

    children.push(section(
        "Log",
        vec![
            div()
                .mb(px(Space::SM))
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(Space::XS))
                .children(level_filters.into_iter().map(|(lvl, label, id)| {
                    pill_button(
                        id,
                        label,
                        state.log_filter_level == lvl,
                        false,
                        move |_window, _cx| push_effect(SideEffect::SetLogFilterLevel(lvl)),
                    )
                }))
                .into_any_element(),
            div()
                .mb(px(Space::MD))
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(Space::XS))
                .children(source_filters.into_iter().map(|(src, label, id)| {
                    pill_button(
                        id,
                        label,
                        state.log_filter_source == src,
                        false,
                        move |_window, _cx| push_effect(SideEffect::SetLogFilterSource(src)),
                    )
                }))
                .into_any_element(),
            div()
                .id(gpui::ElementId::Name(SharedString::from("log-well")))
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(SHELL.border.alpha(0.6))
                .bg(SHELL.recessed)
                .max_h(px(288.0))
                .overflow_y_scroll()
                .p(px(Space::MD))
                .children(log_body)
                .into_any_element(),
            div()
                .mt(px(Space::LG))
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(Space::SM))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .gap(px(Space::XS))
                        .children(runtime_levels.into_iter().map(|(lvl, label, id)| {
                            pill_button(
                                id,
                                label,
                                state.settings.log_level == lvl,
                                false,
                                move |_window, _cx| push_effect(SideEffect::SetLogLevel(lvl)),
                            )
                        })),
                )
                .child(
                    Button::new("Clear")
                        .disabled(state.logs.is_empty())
                        .on_click(|_window, _cx| push_effect(SideEffect::ClearLogs)),
                )
                .into_any_element(),
            div()
                .mt(px(Space::MD))
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
                .text_color(Text::MUTED)
                .child("Pairing tokens, device secrets and video data never appear here.")
                .into_any_element(),
        ],
    ));

    panel.children(children).into_element()
}

fn log_level_rank(level: phonecam_protocol::LogLevel) -> u8 {
    match level {
        phonecam_protocol::LogLevel::Error => 0,
        phonecam_protocol::LogLevel::Warn => 1,
        phonecam_protocol::LogLevel::Info => 2,
        phonecam_protocol::LogLevel::Debug => 3,
        phonecam_protocol::LogLevel::Trace => 4,
    }
}

fn filtered_logs(state: &AppState) -> Vec<&crate::views::state::LogLine> {
    state
        .logs
        .iter()
        .rev()
        .filter(|line| {
            state
                .log_filter_source
                .map(|src| line.source == src)
                .unwrap_or(true)
        })
        .filter(|line| {
            state
                .log_filter_level
                .map(|max| log_level_rank(line.level) <= log_level_rank(max))
                .unwrap_or(true)
        })
        .take(200)
        .collect()
}

fn level_label(level: phonecam_protocol::LogLevel) -> &'static str {
    match level {
        phonecam_protocol::LogLevel::Error => "ERROR",
        phonecam_protocol::LogLevel::Warn => "WARN",
        phonecam_protocol::LogLevel::Info => "INFO",
        phonecam_protocol::LogLevel::Debug => "DEBUG",
        phonecam_protocol::LogLevel::Trace => "TRACE",
    }
}

fn log_level_colour(level: phonecam_protocol::LogLevel) -> Hsla {
    match level {
        phonecam_protocol::LogLevel::Error => Status::DANGER_TEXT,
        phonecam_protocol::LogLevel::Warn => Status::WARN,
        phonecam_protocol::LogLevel::Info => Accent::DEFAULT,
        _ => Text::MUTED,
    }
}

/// Preferences.
pub fn preferences_panel(state: &AppState) -> AnyElement {
    Panel::new("Preferences")
        .child(
            Toggle::new("Keep window on top", state.settings.always_on_top)
                .on_change(|_enabled, _window, _cx| push_effect(SideEffect::ToggleAlwaysOnTop))
                .into_element(),
        )
        .child(
            Toggle::new("Show preview in this window", state.settings.preview_enabled)
                .detail(Some("Off keeps virtual camera running"))
                .on_change(|enabled, _window, _cx| {
                    push_effect(SideEffect::SetPreviewEnabled(enabled))
                })
                .into_element(),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(Space::LG))
                .child(
                    div()
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_SM))
                        .text_color(Text::BODY)
                        .child("Service"),
                )
                .child(chip(
                    if state.status.connected { "Connected" } else { "Offline" },
                    if state.status.connected { Tone::Ok } else { Tone::Warn },
                ))
                .into_any_element(),
        )
        .child(section(
            "Shortcuts",
            vec![
                key_value("Space", "Start or stop streaming", false),
                key_value("F11", "Full-window preview", false),
                key_value("Esc", "Leave full-window preview", false),
                key_value("Ctrl+1", "Setup tab", false),
                key_value("Ctrl+2", "Capture tab", false),
                key_value("Ctrl+3", "Publish tab", false),
                key_value("Ctrl+4", "Diagnostics tab", false),
                key_value("Tab / Shift+Tab", "Navigate controls", false),
            ],
        ))
        .into_element()
}

fn key_value_toned(label: &str, value: &str, tone: Option<Tone>) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_baseline()
        .justify_between()
        .gap(px(Space::PAD))
        .py(px(Space::XS))
        .child(
            div()
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_right()
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_SM))
                .text_color(tone.map(|t| t.fg()).unwrap_or(Text::HEADING))
                .child(value.to_string()),
        )
        .into_any_element()
}

fn pill_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    disabled: bool,
    on_click: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let (border, background, text) = if selected {
        (Accent::DEFAULT, SHELL.control, Accent::DEFAULT)
    } else {
        (SHELL.border, SHELL.control, Text::PRIMARY)
    };

    let mut el = div()
        .id(gpui::ElementId::Name(id.into()))
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(border)
        .bg(background)
        .px(px(Space::LG))
        .py(px(Space::SM))
        .font_family(Fonts::SANS)
        .text_size(px(Type::XS))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(text)
        .child(label.into());

    if disabled {
        el = el.opacity(0.4).cursor_default();
    } else {
        let hover_border = if selected { Accent::DEFAULT } else { Text::MUTED };
        let hover_bg = if selected { SHELL.control } else { SHELL.hover };
        el = el
            .cursor_pointer()
            .hover(move |style| style.border_color(hover_border).bg(hover_bg))
            .active(|style| style.opacity(0.85))
            .focus(|style| style.border_color(Accent::DEFAULT))
            .on_click(move |_event, window, cx| on_click(window, cx));
    }

    el.into_any_element()
}

fn resolution_presets(camera_resolutions: &[String], current: &str) -> Vec<String> {
    let mut out = vec!["auto".to_string()];
    let preferred = [
        "3840x2160",
        "2560x1440",
        "1920x1080",
        "1280x720",
        "854x480",
    ];
    if camera_resolutions.is_empty() {
        for res in ["3840x2160", "1920x1080", "1280x720", "854x480"] {
            out.push(res.to_string());
        }
    } else {
        for res in preferred {
            if camera_resolutions.iter().any(|r| r == res) && out.len() < 7 {
                out.push(res.to_string());
            }
        }
        for res in camera_resolutions {
            if out.len() >= 7 {
                break;
            }
            if !out.iter().any(|existing| existing == res) {
                out.push(res.clone());
            }
        }
    }
    if !current.is_empty() && !out.iter().any(|existing| existing == current) {
        out.push(current.to_string());
    }
    out
}

fn fps_presets(camera_fps: &[u32], current: Option<u32>) -> Vec<Option<u32>> {
    let mut fixed: Vec<u32> = if camera_fps.is_empty() {
        vec![15, 24, 30, 60]
    } else {
        camera_fps.to_vec()
    };
    if let Some(f) = current {
        if !fixed.contains(&f) {
            fixed.push(f);
        }
    }
    fixed.sort_unstable();
    fixed.dedup();
    let mut out = vec![None];
    out.extend(fixed.into_iter().map(Some));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_debugging_guide_has_four_steps_matching_upstream() {
        assert_eq!(USB_DEBUGGING_STEPS.len(), 4);
        assert!(USB_DEBUGGING_STEPS[0].contains("About phone"));
        assert!(USB_DEBUGGING_STEPS[0].contains("7 times"));
        assert!(USB_DEBUGGING_STEPS[1].contains("Developer options"));
        assert!(USB_DEBUGGING_STEPS[1].contains("USB debugging"));
        assert!(USB_DEBUGGING_STEPS[2].contains("MIUI/HyperOS"));
        assert!(USB_DEBUGGING_STEPS[2].contains("Install via USB"));
        assert!(USB_DEBUGGING_STEPS[2].contains("USB debugging (Security settings)"));
        assert!(USB_DEBUGGING_STEPS[3].contains("debugging prompt"));
    }

    #[test]
    fn resolution_presets_include_auto_and_keep_selected() {
        let cam = vec![
            "3840x2160".to_string(),
            "1920x1080".to_string(),
            "1280x720".to_string(),
            "854x480".to_string(),
        ];
        let presets = resolution_presets(&cam, "640x480");
        assert_eq!(presets[0], "auto");
        assert!(presets.contains(&"1920x1080".to_string()));
        assert!(presets.contains(&"640x480".to_string()));
    }

    #[test]
    fn fps_presets_include_auto_and_sort_unique() {
        let presets = fps_presets(&[60, 30, 15, 30], Some(24));
        assert_eq!(presets, vec![None, Some(15), Some(24), Some(30), Some(60)]);
    }

    #[test]
    fn filtered_logs_respect_level_and_source_and_cap_at_200() {
        use phonecam_protocol::LogLevel;
        let mut state = AppState::default();
        for i in 0..250 {
            state.push_log(LogLevel::Info, LogSource::Core, format!("core {i}"));
        }
        state.push_log(LogLevel::Debug, LogSource::Media, "media debug".to_string());
        state.push_log(LogLevel::Error, LogSource::Phone, "phone err".to_string());

        // Default: all filters None -> capped at 200
        assert_eq!(filtered_logs(&state).len(), 200);

        // Filter Warn: Error and Warn only
        state.log_filter_level = Some(LogLevel::Warn);
        let warn_or_higher = filtered_logs(&state);
        assert_eq!(warn_or_higher.len(), 1);
        assert_eq!(warn_or_higher[0].source, LogSource::Phone);

        // Filter Source Media
        state.log_filter_level = None;
        state.log_filter_source = Some(LogSource::Media);
        let media_only = filtered_logs(&state);
        assert_eq!(media_only.len(), 1);
        assert_eq!(media_only[0].message, "media debug");
    }
}







