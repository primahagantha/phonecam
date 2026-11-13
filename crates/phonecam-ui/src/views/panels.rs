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
    camera_facing_label, format_battery, format_log_time, resolution_label_of, screen_mode_label,
    thermal_label, AppState, LogSource, NoticeAction,
};
use crate::views::widgets::{
    chip, key_value, numeric_font, prose, section, status_dot, tinted_box, Button, Panel,
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

    if let Some(phone) = state.core.as_ref().and_then(|c| c.phone.as_ref()) {
        if let Some(batt) = format_battery(phone.battery, phone.charging) {
            let tone = if phone.battery <= 15 && !phone.charging {
                Tone::Warn
            } else {
                Tone::Neutral
            };
            row = row.child(chip(batt, tone));
        }
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

    let mut seen = std::collections::HashSet::new();
    let mut unique_notices = Vec::new();
    for notice in state.notices.iter().rev() {
        let body = notice.code.body(&notice.values);
        let key = (notice.code, body);
        if seen.insert(key) {
            unique_notices.push(notice);
            if unique_notices.len() >= 2 {
                break;
            }
        }
    }
    unique_notices.reverse();

    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(px(Space::MD))
        .px(px(Space::XL))
        .pt(px(Space::XL))
        .children(unique_notices.into_iter().map(|notice| {
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

            let mut actions = div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap(px(Space::MD))
                .pt(px(Space::XS));
            if let Some(action) = action {
                actions = actions.child(action_button(action, primary, id));
            }
            actions = actions.child(Button::ghost("x").on_click(move |_window, _cx| {
                push_effect(SideEffect::DismissNotice(id))
            }));
            banner = banner.child(actions);

            banner.into_any_element()
        }))
        .into_any_element()
}

fn action_button(action: NoticeAction, primary: bool, notice_id: u64) -> AnyElement {
    let label = match action {
        NoticeAction::RefreshState => "Check again",
        NoticeAction::StopStream => "Stop streaming",
        NoticeAction::ArmPairing => "Show a pairing code",
        NoticeAction::StartStream => "Start streaming",
        NoticeAction::ConnectUsb => "Use the cable",
        NoticeAction::RestartVcam => "Turn the camera off and on",
        NoticeAction::Dismiss => "Dismiss",
        NoticeAction::InstallPhoneApp => "Install app on phone",
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
                NoticeAction::RestartVcam => SideEffect::RestartVcam,
                NoticeAction::Dismiss => SideEffect::DismissNotice(notice_id),
                NoticeAction::InstallPhoneApp => SideEffect::InstallPhoneApp,
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

    let local_ip_str = state.local_ip.as_deref().unwrap_or("127.0.0.1");
    let wifi_endpoint = format!("{}:{}", local_ip_str, phonecam_protocol::SERVICE_PORT);

    children.push(section(
        "Connect via Wi-Fi",
        vec![
            div()
                .flex()
                .flex_col()
                .gap(px(Space::SM))
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(Accent::DEFAULT.alpha(0.35))
                .bg(Accent::DEFAULT.alpha(0.06))
                .p(px(Space::PAD))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(Space::SM))
                                .child(status_dot(Tone::Ok))
                                .child(
                                    div()
                                        .font_family(Fonts::SANS)
                                        .text_size(px(Type::TWO_SM))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(Text::PRIMARY)
                                        .child("Wi-Fi / LAN Network"),
                                ),
                        )
                        .child(
                            div()
                                .rounded(px(Radius::FULL))
                                .border_1()
                                .border_color(SHELL.border)
                                .bg(SHELL.base)
                                .px(px(Space::SM))
                                .py(px(2.0))
                                .font_family(Fonts::MONO)
                                .text_size(px(Type::TWO_XS))
                                .text_color(Accent::DEFAULT)
                                .child(format!("Port {}", phonecam_protocol::SERVICE_PORT)),
                        ),
                )
                .child(
                    div()
                        .mt(px(Space::XS))
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_family(Fonts::SANS)
                                .text_size(px(Type::XS))
                                .text_color(Text::MUTED)
                                .child("PC IP Address:"),
                        )
                        .child(
                            div()
                                .font_family(Fonts::MONO)
                                .text_size(px(Type::TWO_SM))
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(Text::PRIMARY)
                                .child(wifi_endpoint),
                        ),
                )
                .child(
                    div()
                        .mt(px(Space::XS))
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::BODY)
                        .child("1. Connect phone and PC to the same Wi-Fi.\n2. In PhoneCam Android, scan the QR code on the left or tap Search.\n3. Or enter the PC IP address above manually in the Android app."),
                )
                .into_any_element(),
        ],
    ));

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

/// What action (or status hint) an ADB-listed USB row can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDeviceAction {
    UsePhone,
    AuthorizeHint,
    OfflineHint,
}

/// Maps an ADB device `state` string to whether it is ready to connect.
pub fn usb_device_action(state: &str) -> UsbDeviceAction {
    match state {
        "device" => UsbDeviceAction::UsePhone,
        "unauthorized" => UsbDeviceAction::AuthorizeHint,
        _ => UsbDeviceAction::OfflineHint,
    }
}

/// Summary sentence above the USB device list.
pub fn usb_list_summary(devices: &[phonecam_protocol::AdbDevice], forwarding: bool) -> &'static str {
    if forwarding {
        return "The cable tunnel is open. Open PhoneCam on the phone to connect.";
    }
    let ready_count = devices
        .iter()
        .filter(|d| usb_device_action(&d.state) == UsbDeviceAction::UsePhone)
        .count();
    if ready_count == 1 {
        "A phone is on the cable, not connected yet."
    } else if ready_count > 1 {
        "Phones on the cable, none connected yet."
    } else if devices
        .iter()
        .any(|d| usb_device_action(&d.state) == UsbDeviceAction::AuthorizeHint)
    {
        "Unlock your phone and tap Allow on the USB debugging prompt."
    } else {
        "Phone on the cable is offline. Reconnect the cable or restart ADB."
    }
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
            .child(usb_list_summary(&devices, forwarding));

        for device in devices {
            let label = if device.model.is_empty() { device.serial.clone() } else { device.model.clone() };
            let serial_for_connect = device.serial.clone();
            let id_for_btn = device.serial.clone();
            let action = usb_device_action(&device.state);
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
                    .child(match (forwarding, action) {
                        (true, _) => Button::danger("Close tunnel")
                            .on_click(|_window, _cx| push_effect(SideEffect::DisconnectUsb))
                            .into_any_element(),
                        (false, UsbDeviceAction::UsePhone) => {
                            // Not primary: the cable is the fallback path.
                            pill_button(
                                format!("usb-use-{id_for_btn}"),
                                "Use this phone",
                                false,
                               false,
                                move |_window, _cx| {
                                    push_effect(SideEffect::ConnectUsb(Some(
                                        serial_for_connect.clone(),
                                    )))
                                },
                            )
                        }
                        (false, UsbDeviceAction::AuthorizeHint) => {
                            chip("Approve on phone", Tone::Warn)
                        }
                        (false, UsbDeviceAction::OfflineHint) => {
                            chip("Offline", Tone::Neutral)
                        }
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
        if let Some(batt) = state
            .core
            .as_ref()
            .and_then(|c| c.phone.as_ref())
            .and_then(|p| format_battery(p.battery, p.charging))
        {
            children.push(key_value("Battery", batt, false));
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

    let not_connected = !state.status.connected || !state.has_device();
    let probed_cameras = state.capabilities.as_ref().map(|c| c.cameras.clone()).unwrap_or_default();
    let cameras = if probed_cameras.is_empty() {
        vec![
            phonecam_protocol::CameraCapability {
                id: "0".to_string(),
                facing: phonecam_protocol::CameraFacing::Back,
                resolutions: vec!["1920x1080".into(), "1280x720".into(), "640x480".into()],
                fps: vec![24, 25, 30, 60],
                torch: true,
                focus: true,
                zoom: true,
                exposure: true,
                white_balance: true,
                min_zoom: 1.0,
                max_zoom: 10.0,
                orientation_degrees: 90,
            },
            phonecam_protocol::CameraCapability {
                id: "1".to_string(),
                facing: phonecam_protocol::CameraFacing::Front,
                resolutions: vec!["1920x1080".into(), "1280x720".into(), "640x480".into()],
                fps: vec![24, 25, 30],
                torch: false,
                focus: true,
                zoom: true,
                exposure: true,
                white_balance: true,
                min_zoom: 1.0,
                max_zoom: 10.0,
                orientation_degrees: 270,
            },
        ]
    } else {
        probed_cameras
    };

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

    if not_connected {
        children.push(tinted_box(
            Tone::Neutral,
            vec![
                div()
                    .font_family(Fonts::SANS)
                    .text_size(px(Type::XS))
                    .text_color(Text::MUTED)
                    .child("No phone connected. Settings below will apply automatically once connected.")
                    .into_any_element(),
            ],
        ));
    }

    // Quick camera ID cycle list: known camera IDs plus fallback 0..=4.
    let mut cycle_ids: Vec<String> = cameras.iter().map(|c| c.id.clone()).collect();
    for fallback in ["0", "1", "2", "3", "4"] {
        if !cycle_ids.iter().any(|id| id == fallback) {
            cycle_ids.push(fallback.to_string());
        }
    }
    let resolved_id = cameras
        .iter()
        .find(|c| c.id == selected)
        .or_else(|| cameras.iter().find(|c| c.facing.as_str() == selected))
        .map(|c| c.id.as_str())
        .unwrap_or(selected.as_str())
        .to_string();
    let current_idx = cycle_ids.iter().position(|id| id == &resolved_id).unwrap_or(0);
    let prev_id = cycle_ids[(current_idx + cycle_ids.len() - 1) % cycle_ids.len()].clone();
    let next_id = cycle_ids[(current_idx + 1) % cycle_ids.len()].clone();

    children.push(section(
        "Which camera",
        vec![
            camera_tiles(&cameras, &resolved_id),
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(Space::XS))
                .child(
                    div()
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .mr(px(Space::XS))
                        .child("Manual ID:"),
                )
                .child(pill_button(
                    "cam-id-prev",
                    "<",
                    false,
                    false,
                    move |_window, _cx| push_effect(SideEffect::SetCamera(prev_id.clone())),
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
                .child(pill_button(
                    "cam-id-next",
                    ">",
                    false,
                    false,
                    move |_window, _cx| push_effect(SideEffect::SetCamera(next_id.clone())),
                ))
                .into_any_element(),
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

    let bitrate_dec = current_bitrate.saturating_sub(500_000).max(1_000_000);
    let bitrate_inc = (current_bitrate + 500_000).min(20_000_000);
    let bitrate_presets: [(u64, &str); 7] = [
        (1_500_000, "1.5M"),
        (2_000_000, "2M"),
        (4_000_000, "4M"),
        (8_000_000, "8M"),
        (12_000_000, "12M"),
        (16_000_000, "16M"),
        (20_000_000, "20M"),
    ];

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
                            resolution_label_of(&res)
                        };
                        let id = format!("res-{res}");
                        pill_button(id, label, active, false, move |_window, _cx| {
                            push_effect(SideEffect::SetResolution(res.clone()))
                        })
                    }))
                    .into_any_element(),
                None,
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
                    .children(bitrate_presets.into_iter().map(|(bps, label)| {
                        let active = current_bitrate == bps;
                        pill_button(
                            format!("bitrate-{label}"),
                            label,
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
        let zoom_dec = ((live_zoom - 0.1).max(min_zoom) * 10.0).round() / 10.0;
        let zoom_inc = ((live_zoom + 0.1).min(max_zoom) * 10.0).round() / 10.0;
        let zoom_presets = zoom_presets_for(min_zoom, max_zoom);

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
            crate::views::widgets::field(
                "Phone screen",
                Some(screen_mode_label(state.settings.screen_mode).into()),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::SM))
                    .children(
                        [
                            (phonecam_protocol::ScreenMode::KeepOn, "Keep on", "screen-keep-on"),
                            (phonecam_protocol::ScreenMode::Dim, "Dim", "screen-dim"),
                            (phonecam_protocol::ScreenMode::Black, "Off", "screen-black"),
                        ]
                        .into_iter()
                        .map(|(mode, label, id)| {
                            let active = state.settings.screen_mode == mode;
                            pill_button(id, label, active, false, move |_window, _cx| {
                                push_effect(SideEffect::SetScreenMode(mode))
                            })
                        }),
                    )
                    .into_any_element(),
                Some("The phone screen is not part of the stream. Turning it off saves battery on a long session.".into()),
            ),
        ],
    ));

    panel.children(children).into_element()
}

fn camera_tiles(
    cameras: &[phonecam_protocol::CameraCapability],
    selected_id: &str,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(Space::MD))
        .children(cameras.iter().map(|camera| {
            let is_selected = camera.id == selected_id;
            let id = camera.id.clone();
            div()
                .id(gpui::ElementId::Name(SharedString::from(format!("camera-{id}"))))
                .flex_1()
                .min_w(px(130.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(Space::SM))
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(if is_selected { Accent::DEFAULT } else { SHELL.control })
                .bg(if is_selected { Accent::DEFAULT.alpha(0.12) } else { SHELL.base })
                .px(px(Space::PAD))
                .py(px(Space::MD))
                .cursor_pointer()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .font_family(Fonts::SANS)
                                .text_size(px(Type::TWO_SM))
                                .font_weight(if is_selected { gpui::FontWeight::SEMIBOLD } else { gpui::FontWeight::MEDIUM })
                                .text_color(if is_selected { Text::PRIMARY } else { Text::BODY })
                                .child(camera_facing_label(camera.facing)),
                        )
                        .child(
                            div()
                                .font_family(Fonts::MONO)
                                .text_size(px(Type::TWO_XS))
                                .text_color(Text::MUTED)
                                .child(format!("Lens ID {}", camera.id)),
                        ),
                )
                .child(
                    if is_selected {
                        div()
                            .rounded(px(Radius::FULL))
                            .bg(Accent::DEFAULT)
                            .px(px(6.0))
                            .py(px(2.0))
                            .font_family(Fonts::SANS)
                            .text_size(px(Type::TWO_XS))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(SHELL.base)
                            .child("Active")
                    } else {
                        div()
                            .font_family(Fonts::SANS)
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child("Select")
                    }
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
    let output_summary: SharedString = if state.settings.output_follows_stream {
        match state.frame_size {
            Some((w, h)) => format!("Follows phone ({w} × {h})").into(),
            None => "Follows phone".into(),
        }
    } else {
        format!("{} × {}", state.settings.output_width, state.settings.output_height).into()
    };

    children.push(section(
        "Framing",
        vec![
            Toggle::new("Follow phone stream", state.settings.output_follows_stream)
                .on_change(|enabled, _window, _cx| {
                    push_effect(SideEffect::SetOutputFollowsStream(enabled))
                })
                .into_element(),
            crate::views::widgets::field(
                "Output size",
                Some(output_summary),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::SM))
                    .children(OUTPUT_SIZE_PRESETS.into_iter().map(|(w, h)| {
                        let active = !state.settings.output_follows_stream
                            && state.settings.output_width == w
                            && state.settings.output_height == h;
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
                    "What apps receive. Pick a size to pin it, or follow the phone stream; sideways phones fit inside without stretching."
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
    let (status_chip_label, status_tone) = if !state.status.connected {
        ("ERROR", Tone::Danger)
    } else if state.is_streaming() {
        ("OK", Tone::Ok)
    } else {
        ("IDLE", Tone::Neutral)
    };

    let panel = Panel::new("Diagnostics")
        .action(chip(status_chip_label, status_tone));

    let mut children: Vec<AnyElement> = Vec::new();

    // 1. Action Toolbar: dedicated, beautifully aligned 2x2 grid
    let toolbar = div()
        .grid()
        .grid_cols(2)
        .gap(px(Space::MD))
        .child(
            Button::new("Refresh")
                .full_width()
                .on_click(|_window, _cx| push_effect(SideEffect::RefreshState)),
        )
        .child(
            Button::new("Copy Report")
                .full_width()
                .on_click(|_window, _cx| push_effect(SideEffect::CopyDiagnosticsReport)),
        )
        .child(
            Button::new("Export")
                .full_width()
                .on_click(|_window, _cx| push_effect(SideEffect::ExportDiagnostics)),
        )
        .child(
            Button::new("Open Log File")
                .full_width()
                .on_click(|_window, _cx| push_effect(SideEffect::OpenLogFile)),
        )
        .into_any_element();
    children.push(toolbar);

    // 2. Live Throughput Metric Cards: modern 2x2 card tiles
    let stats = state.core.as_ref().map(|c| &c.stats);
    let readout = rules::throughput_readout(stats, state.phase(), crate::theme::EMPTY_VALUE);
    let fps_display = if readout.fps == crate::theme::EMPTY_VALUE {
        crate::theme::EMPTY_VALUE.to_string()
    } else {
        format!("{} fps", readout.fps)
    };

    let throughput_grid = div()
        .grid()
        .grid_cols(2)
        .gap(px(Space::MD))
        .child(metric_card_tile("FPS", fps_display))
        .child(metric_card_tile("BITRATE", readout.bitrate))
        .child(metric_card_tile("LATENCY", readout.latency))
        .child(metric_card_tile("DROPPED", readout.dropped))
        .into_any_element();
    children.push(section("Throughput", vec![throughput_grid]));

    // 3. Structured System & Device Info Cards
    // Card A: "Connected Phone"
    let device = state.core.as_ref().and_then(|c| c.device.as_ref());
    let facts = state.device_facts();
    let phone_card: AnyElement = match device {
        Some(device) => {
            let mut rows: Vec<(SharedString, AnyElement)> = Vec::new();
            rows.push(("Name".into(), card_text_val(device.name.clone(), false)));
            if let Some(model) = facts.model {
                rows.push(("Model".into(), chip(model, Tone::Neutral)));
            }
            rows.push(("Device ID".into(), card_text_val(device.device_id.clone(), true)));
            if let Some(version) = facts.android_version {
                rows.push(("Android".into(), card_text_val(version.to_string(), false)));
            }

            let phone_info = state.core.as_ref().and_then(|c| c.phone.as_ref());
            let (batt_label, batt_tone) = match phone_info.and_then(|p| format_battery(p.battery, p.charging)) {
                Some(label) => {
                    let b = phone_info.map(|p| p.battery).unwrap_or(0);
                    let is_charging = phone_info.map(|p| p.charging).unwrap_or(false);
                    let tone = if is_charging {
                        Tone::Ok
                    } else if b <= 20 {
                        Tone::Danger
                    } else {
                        Tone::Neutral
                    };
                    (label, tone)
                }
                None => (crate::theme::EMPTY_VALUE.to_string(), Tone::Neutral),
            };
            rows.push(("Battery".into(), chip(batt_label, batt_tone)));

            let cam_count = state.capabilities.as_ref().map(|c| c.cameras.len());
            let cam_label = cam_count
                .map(|count| format!("{count} cameras"))
                .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string());
            rows.push(("Cameras".into(), chip(cam_label, Tone::Neutral)));

            let app_ver = state
                .capabilities
                .as_ref()
                .map(|c| format!("v{}", c.device.app_version))
                .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string());
            rows.push(("App version".into(), chip(app_ver, Tone::Neutral)));

            info_card_from_rows(rows)
        }
        None => info_card(vec![
            div()
                .py(px(Space::SM))
                .child(prose(
                    "Nothing connected. Pair a phone from Setup and its details appear here.",
                ))
                .into_any_element(),
        ]),
    };
    children.push(section("Connected phone", vec![phone_card]));

    // Card B: "Output Stream"
    let published_output = if state.settings.output_follows_stream {
        match state.frame_size {
            Some((w, h)) => format!("{w}×{h}"),
            None => format!("{}×{}", state.settings.output_width, state.settings.output_height),
        }
    } else {
        format!("{}×{}", state.settings.output_width, state.settings.output_height)
    };
    let decoded_str = state
        .frame_size
        .map(|(w, h)| format!("{w}×{h}"))
        .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string());
    let phone_out_str = state
        .core
        .as_ref()
        .and_then(|c| c.phone.as_ref())
        .map(|p| p.actual_resolution.replace('x', "×"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string());

    let output_rows = vec![
        ("Mirror".into(), card_text_val(if state.settings.output_mirror { "On" } else { "Off" }, false)),
        ("Rotation".into(), card_text_val(format!("{}°", state.settings.output_rotation), false)),
        ("Output resolution".into(), card_text_val(published_output, false)),
        ("Decoded stream".into(), card_text_val(decoded_str, false)),
        ("Phone stream resolution".into(), card_text_val(phone_out_str, false)),
    ];
    children.push(section("Output stream", vec![info_card_from_rows(output_rows)]));

    // Card C: "Network & Endpoints"
    let service_endpoint = format!("127.0.0.1:{}", phonecam_protocol::SERVICE_PORT);
    let wifi_endpoint = state
        .local_ip
        .as_deref()
        .map(|ip| format!("{}:{}", ip, phonecam_protocol::SERVICE_PORT))
        .unwrap_or_else(|| "Unavailable".to_string());

    let socket_dot_el = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(Space::SM))
        .child(status_dot(if state.status.connected { Tone::Ok } else { Tone::Danger }))
        .child(
            div()
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_SM))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if state.status.connected { Status::OK } else { Status::DANGER_TEXT })
                .child(if state.status.connected { "Connected" } else { "Disconnected" }),
        )
        .into_any_element();

    let network_rows = vec![
        ("Service endpoint".into(), card_text_val(service_endpoint, true)),
        ("Wi-Fi endpoint".into(), card_text_val(wifi_endpoint, true)),
        ("Socket status".into(), socket_dot_el),
        ("Log file".into(), card_text_val("%APPDATA%\\PhoneCam\\desktop.log", true)),
        ("Platform".into(), card_text_val(if cfg!(windows) { "win32 x64" } else { "linux x64" }, false)),
    ];
    children.push(section("Network & endpoints", vec![info_card_from_rows(network_rows)]));

    // 4. Redesigned Log Viewer
    let matching_logs = filtered_logs(state);
    let log_body: Vec<AnyElement> = if state.logs.is_empty() {
        vec![div()
            .py(px(Space::MD))
            .font_family(Fonts::MONO)
            .text_size(px(Type::XS))
            .text_color(Text::MUTED)
            .child("Nothing logged yet. Lines appear as the service and the phone report in.")
            .into_any_element()]
    } else if matching_logs.is_empty() {
        vec![div()
            .py(px(Space::MD))
            .font_family(Fonts::MONO)
            .text_size(px(Type::XS))
            .text_color(Text::MUTED)
            .child("No lines match these filters.")
            .into_any_element()]
    } else {
        matching_logs
            .iter()
            .map(|line| {
                div()
                    .flex()
                    .flex_row()
                    .items_start()
                    .gap(px(Space::SM))
                    .py(px(2.0))
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(72.0))
                            .font_family(Fonts::MONO)
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child(format_log_time(line.ts_ms)),
                    )
                    .child(log_level_badge(line.level))
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(52.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(Radius::SM))
                            .bg(Accent::DEFAULT.alpha(0.10))
                            .border_1()
                            .border_color(Accent::DEFAULT.alpha(0.20))
                            .py(px(1.0))
                            .font_family(Fonts::MONO)
                            .text_size(px(Type::TWO_XS))
                            .text_color(Accent::DEFAULT)
                            .child(line.source.label()),
                    )
                    .child(
                        div()
                            .flex_grow()
                            .min_w(px(0.0))
                            .font_family(Fonts::MONO)
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::PRIMARY)
                            .child(line.message.clone()),
                    )
                    .into_any_element()
            })
            .collect()
    };

    let next_filter_lvl = match state.log_filter_level {
        None => Some(phonecam_protocol::LogLevel::Error),
        Some(phonecam_protocol::LogLevel::Error) => Some(phonecam_protocol::LogLevel::Warn),
        Some(phonecam_protocol::LogLevel::Warn) => Some(phonecam_protocol::LogLevel::Info),
        Some(phonecam_protocol::LogLevel::Info) => Some(phonecam_protocol::LogLevel::Debug),
        Some(phonecam_protocol::LogLevel::Debug) => None,
        Some(phonecam_protocol::LogLevel::Trace) => None,
    };
    let filter_lvl_label = match state.log_filter_level {
        None => "All levels",
        Some(phonecam_protocol::LogLevel::Error) => "Error",
        Some(phonecam_protocol::LogLevel::Warn) => "Warn",
        Some(phonecam_protocol::LogLevel::Info) => "Info",
        Some(phonecam_protocol::LogLevel::Debug) => "Debug",
        Some(phonecam_protocol::LogLevel::Trace) => "Trace",
    };

    let next_filter_src = match state.log_filter_source {
        None => Some(LogSource::Core),
        Some(LogSource::Core) => Some(LogSource::App),
        Some(LogSource::App) => Some(LogSource::Renderer),
        Some(LogSource::Renderer) => Some(LogSource::Media),
        Some(LogSource::Media) => Some(LogSource::Phone),
        Some(LogSource::Phone) => None,
    };
    let filter_src_label = match state.log_filter_source {
        None => "All sources",
        Some(LogSource::Core) => "Service",
        Some(LogSource::App) => "PhoneCam",
        Some(LogSource::Renderer) => "App",
        Some(LogSource::Media) => "Media",
        Some(LogSource::Phone) => "Phone",
    };

    let next_runtime_lvl = match state.settings.log_level {
        phonecam_protocol::LogLevel::Info => phonecam_protocol::LogLevel::Debug,
        phonecam_protocol::LogLevel::Debug => phonecam_protocol::LogLevel::Trace,
        phonecam_protocol::LogLevel::Trace => phonecam_protocol::LogLevel::Error,
        phonecam_protocol::LogLevel::Error => phonecam_protocol::LogLevel::Warn,
        phonecam_protocol::LogLevel::Warn => phonecam_protocol::LogLevel::Info,
    };
    let runtime_lvl_label = format!("Log level: {}", state.settings.log_level.as_str());

    let log_line_count = if matching_logs.len() == state.logs.len() {
        format!("{} lines", matching_logs.len())
    } else {
        format!("{}/{} lines", matching_logs.len(), state.logs.len())
    };

    let filter_bar = div()
        .mb(px(Space::MD))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(Space::MD))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(Space::SM))
                .child(select_dropdown(
                    "log-filter-level-select",
                    filter_lvl_label,
                    move |_window, _cx| push_effect(SideEffect::SetLogFilterLevel(next_filter_lvl)),
                ))
                .child(select_dropdown(
                    "log-filter-source-select",
                    filter_src_label,
                    move |_window, _cx| push_effect(SideEffect::SetLogFilterSource(next_filter_src)),
                )),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(Space::MD))
                .child(
                    div()
                        .font_family(Fonts::MONO)
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .child(log_line_count),
                )
                .child(
                    Button::new("Clear")
                        .disabled(state.logs.is_empty())
                        .on_click(|_window, _cx| push_effect(SideEffect::ClearLogs)),
                ),
        )
        .into_any_element();

    let terminal_well = div()
        .id(gpui::ElementId::Name(SharedString::from("log-well")))
        .flex()
        .flex_col()
        .gap(px(Space::XS))
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border.alpha(0.6))
        .bg(SHELL.recessed)
        .min_h(px(160.0))
        .max_h(px(320.0))
        .overflow_y_scroll()
        .p(px(Space::MD))
        .children(log_body)
        .into_any_element();

    let bottom_bar = div()
        .mt(px(Space::MD))
        .flex()
        .flex_col()
        .gap(px(Space::SM))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(Space::MD))
                .child(
                    div()
                        .w(px(200.0))
                        .child(select_dropdown(
                            "log-runtime-level-select",
                            runtime_lvl_label,
                            move |_window, _cx| push_effect(SideEffect::SetLogLevel(next_runtime_lvl)),
                        )),
                )
                .child(
                    Button::new("Open in Notepad")
                        .on_click(|_window, _cx| push_effect(SideEffect::OpenLogFile)),
                ),
        )
        .child(
            div()
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
                .text_color(Text::MUTED)
                .child("Pairing tokens, device secrets and video frames are never logged. Open desktop.log directly in Notepad."),
        )
        .into_any_element();

    children.push(section("Log", vec![filter_bar, terminal_well, bottom_bar]));

    panel.children(children).into_element()
}

/// A modern metric card tile: rounded corners, dark recessed background, subtle border,
/// small uppercase muted label, and large prominent bold numeric value with unit.
fn metric_card_tile(label: &'static str, value: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .justify_between()
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border.alpha(0.6))
        .bg(SHELL.recessed)
        .p(px(Space::PAD))
        .gap(px(Space::XS))
        .child(
            div()
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_XS))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(Text::MUTED)
                .child(label),
        )
        .child(
            div()
                .font(numeric_font(Fonts::DISPLAY))
                .text_size(px(18.0))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(Text::PRIMARY)
                .child(value.into()),
        )
        .into_any_element()
}

/// A structured info card container: rounded corners, subtle border, recessed background.
fn info_card(rows: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border.alpha(0.6))
        .bg(SHELL.recessed)
        .px(px(Space::PAD))
        .py(px(Space::XS))
        .children(rows)
        .into_any_element()
}

/// A key-value row inside a structured card, with subtle row divider if not last.
fn card_kv_row(
    label: impl Into<SharedString>,
    value_el: AnyElement,
    is_last: bool,
) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(Space::PAD))
        .py(px(Space::SM))
        .child(
            div()
                .flex_shrink()
                .overflow_hidden()
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child(label.into()),
        )
        .child(value_el);

    if !is_last {
        row = row.border_b_1().border_color(SHELL.border.alpha(0.25));
    }

    row.into_any_element()
}

/// Formats a value as text for a card row.
fn card_text_val(text: impl Into<SharedString>, mono: bool) -> AnyElement {
    div()
        .flex_shrink_0()
        .text_right()
        .font_family(if mono { Fonts::MONO } else { Fonts::SANS })
        .text_size(px(if mono { Type::XS } else { Type::TWO_SM }))
        .text_color(Text::HEADING)
        .child(text.into())
        .into_any_element()
}

/// Builds an info card from a list of (label, value_element) pairs.
fn info_card_from_rows(rows: Vec<(SharedString, AnyElement)>) -> AnyElement {
    let len = rows.len();
    let row_elements = rows
        .into_iter()
        .enumerate()
        .map(|(idx, (label, val))| card_kv_row(label, val, idx + 1 == len))
        .collect();
    info_card(row_elements)
}

/// A fixed-width level badge with distinct background, border, and text styling.
fn log_level_badge(level: phonecam_protocol::LogLevel) -> AnyElement {
    let (bg, border) = match level {
        phonecam_protocol::LogLevel::Error => (
            Status::DANGER.alpha(0.15),
            Status::DANGER.alpha(0.4),
        ),
        phonecam_protocol::LogLevel::Warn => (
            Status::WARN.alpha(0.15),
            Status::WARN.alpha(0.4),
        ),
        phonecam_protocol::LogLevel::Info => (
            Accent::DEFAULT.alpha(0.12),
            Accent::DEFAULT.alpha(0.3),
        ),
        phonecam_protocol::LogLevel::Debug => (
            SHELL.border.alpha(0.2),
            SHELL.border.alpha(0.4),
        ),
        phonecam_protocol::LogLevel::Trace => (
            SHELL.border.alpha(0.1),
            SHELL.border.alpha(0.3),
        ),
    };
    let text = log_level_colour(level);

    div()
        .flex_shrink_0()
        .w(px(46.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(Radius::SM))
        .border_1()
        .border_color(border)
        .bg(bg)
        .py(px(1.0))
        .font_family(Fonts::MONO)
        .text_size(px(Type::TWO_XS))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(text)
        .child(level_label(level))
        .into_any_element()
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

fn select_dropdown(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    on_click: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    div()
        .id(gpui::ElementId::Name(id.into()))
        .flex_1()
        .min_w(px(120.0))
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(Space::MD))
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.control)
        .hover(|s| s.bg(SHELL.raised).border_color(Accent::DEFAULT.alpha(0.6)))
        .cursor_pointer()
        .px(px(Space::LG))
        .py(px(Space::SM))
        .on_click(move |_ev, window, cx| {
            on_click(window, cx);
        })
        .child(
            div()
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(Text::PRIMARY)
                .child(label.into()),
        )
        .child(
            div()
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child("▼"),
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

const OUTPUT_SIZE_PRESETS: [(u32, u32); 6] = [
    (1920, 1080),
    (1280, 720),
    (2560, 1440),
    (3840, 2160),
    (854, 480),
    (640, 360),
];

#[allow(dead_code)]
fn app_metadata_summary() -> String {
    let os = if cfg!(target_os = "windows") {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else {
        "Linux"
    };
    format!("PhoneCam v{} · {os} · Rust/GPUI", env!("CARGO_PKG_VERSION"))
}

fn zoom_presets_for(min_zoom: f32, max_zoom: f32) -> Vec<f32> {
    let round1 = |v: f32| (v * 10.0).round() / 10.0;
    let min_z = round1(min_zoom);
    let max_z = round1(max_zoom.max(min_z));
    let mut presets = Vec::new();
    let mut push_unique = |val: f32| {
        let v = round1(val.clamp(min_z, max_z));
        if !presets.iter().any(|existing: &f32| (*existing - v).abs() < 0.05) {
            presets.push(v);
        }
    };

    if min_z < 0.99 {
        push_unique(min_z);
    }
    for candidate in [1.0_f32, 1.5, 2.0, 3.0, 5.0, 8.0, 10.0] {
        if candidate >= min_z - 0.01 && candidate <= max_z + 0.01 {
            push_unique(candidate);
        }
    }
    push_unique(max_z);
    presets.sort_by(|a, b| a.total_cmp(b));
    presets
}

fn resolution_presets(camera_resolutions: &[String], current: &str) -> Vec<String> {
    let mut out = vec!["auto".to_string()];
    let preferred = [
        "3840x2160",
        "2560x1440",
        "1920x1080",
        "1280x720",
        "854x480",
        "640x480",
        "640x360",
    ];
    if camera_resolutions.is_empty() {
        for res in [
            "3840x2160",
            "1920x1080",
            "1280x720",
            "854x480",
            "640x480",
            "640x360",
        ] {
            out.push(res.to_string());
        }
    } else {
        for res in preferred {
            if camera_resolutions.iter().any(|r| r == res) {
                out.push(res.to_string());
            }
        }
        for res in camera_resolutions {
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

    #[test]
    fn usb_device_action_gates_unauthorized_and_offline_devices() {
        assert_eq!(usb_device_action("device"), UsbDeviceAction::UsePhone);
        assert_eq!(usb_device_action("unauthorized"), UsbDeviceAction::AuthorizeHint);
        assert_eq!(usb_device_action("offline"), UsbDeviceAction::OfflineHint);
    }

    #[test]
    fn usb_list_summary_counts_ready_and_pending_devices() {
        let ready = vec![phonecam_protocol::AdbDevice {
            serial: "s1".into(),
            state: "device".into(),
            model: "Pixel 8".into(),
        }];
        assert_eq!(
            usb_list_summary(&ready, false),
            "A phone is on the cable, not connected yet."
        );
        assert_eq!(
            usb_list_summary(&ready, true),
            "The cable tunnel is open. Open PhoneCam on the phone to connect."
        );

        let unauth = vec![phonecam_protocol::AdbDevice {
            serial: "s1".into(),
            state: "unauthorized".into(),
            model: "Pixel 8".into(),
        }];
        assert_eq!(
            usb_list_summary(&unauth, false),
            "Unlock your phone and tap Allow on the USB debugging prompt."
        );

        let offline = vec![phonecam_protocol::AdbDevice {
            serial: "s2".into(),
            state: "offline".into(),
            model: "Pixel 7".into(),
        }];
        assert_eq!(
            usb_list_summary(&offline, false),
            "Phone on the cable is offline. Reconnect the cable or restart ADB."
        );
    }

    #[test]
    fn resolution_presets_uncapped_and_includes_640_presets() {
        let caps: Vec<String> = vec![
            "3840x2160",
            "2560x1440",
            "1920x1080",
            "1280x720",
            "854x480",
            "640x480",
            "640x360",
            "320x240",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let presets = resolution_presets(&caps, "auto");
        assert_eq!(presets.len(), 9);
        assert!(presets.contains(&"640x360".to_string()));
        assert!(presets.contains(&"320x240".to_string()));
    }

    #[test]
    fn output_size_presets_include_640x360() {
        assert!(OUTPUT_SIZE_PRESETS.contains(&(640, 360)));
    }

    #[test]
    fn zoom_presets_cover_min_one_and_max() {
        let presets = zoom_presets_for(0.6, 8.0);
        assert_eq!(presets.first().copied(), Some(0.6));
        assert!(presets.contains(&1.0));
        assert_eq!(presets.last().copied(), Some(8.0));
    }

    #[test]
    fn app_metadata_summary_mentions_version_and_gpui() {
        let summary = app_metadata_summary();
        assert!(summary.contains(env!("CARGO_PKG_VERSION")));
        assert!(summary.contains("Rust/GPUI"));
    }

    #[test]
    fn diagnostics_panel_renders_different_states_without_panicking() {
        // 1. Idle disconnected
        let state = AppState::default();
        let _el = diagnostics_panel(&state);

        // 2. Connected with device and facts
        let mut connected_state = AppState::default();
        connected_state.status.connected = true;
        let core = phonecam_protocol::CoreState {
            phase: phonecam_protocol::StreamPhase::Connected,
            device: Some(phonecam_protocol::DeviceIdentity {
                device_id: "dev-123".into(),
                name: "Pixel 9".into(),
                model: "Pixel 9 Pro".into(),
                android_version: 34,
            }),
            phone: Some(phonecam_protocol::PhoneState {
                battery: 75,
                charging: true,
                actual_resolution: "1920x1080".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        connected_state.core = Some(core);
        let _el2 = diagnostics_panel(&connected_state);

        // 3. Streaming with throughput stats
        let mut streaming_state = connected_state.clone();
        if let Some(core) = &mut streaming_state.core {
            core.phase = phonecam_protocol::StreamPhase::Streaming;
            core.stats = phonecam_protocol::ConnectionStats {
                fps: 60.0,
                bitrate: 15_000_000,
                latency_ms: 12.0,
                dropped_percent: 0.05,
                ..Default::default()
            };
        }
        let _el3 = diagnostics_panel(&streaming_state);
    }

    #[test]
    fn log_level_badge_covers_all_variants() {
        use phonecam_protocol::LogLevel;
        for level in [
            LogLevel::Error,
            LogLevel::Warn,
            LogLevel::Info,
            LogLevel::Debug,
            LogLevel::Trace,
        ] {
            let _el = log_level_badge(level);
        }
    }
}







