//! The four panels, the status strip, the notice bar and the approval prompt.
//!
//! The layout follows the job, not the data model. An operator sits beside this
//! window with a phone on a tripod, and at any moment they are doing exactly one
//! of four things: get a phone connected, choose what it captures, choose what
//! this PC publishes, or read the numbers when something is wrong. Those are the
//! four tabs, in the order the job happens.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Hsla, ScrollHandle, SharedString};

use crate::theme::rules;
use crate::theme::{hsla_from_hex, Accent, Fonts, Radius, Space, Status, Text, Tone, Type, SHELL};
use crate::views::effects::{push_effect, SideEffect, Tab};
use crate::views::state::{
    camera_facing_label, format_battery, format_log_time, resolution_label_of, thermal_label,
    AppState, FailureCode, LogSource, NoticeAction, UpdateStatus,
};
use crate::views::widgets::{
    chip, key_value, numeric_font, prose, section, status_dot, tinted_box, well, Button,
    FocusContext, Panel, Slider, Toggle,
};

/// UI-only flags owned by `AppView` and passed into the pure panel builders.
#[derive(Debug, Clone, Copy, Default)]
pub struct RailUiState {
    pub show_preferences: bool,
    pub show_updates: bool,
    pub usb_help_expanded: bool,
    pub labs_expanded: bool,
    pub gesture_ref_expanded: bool,
}

/// Optional view callbacks wired by `AppView::render` for local UI state transitions.
#[derive(Default)]
pub struct PanelCallbacks {
    pub on_toggle_preferences: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    pub on_toggle_updates: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    pub on_open_updates: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    pub on_toggle_usb_help: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    pub on_toggle_labs: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    pub on_toggle_gesture_ref: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
}

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

/// Renders a crisp vector battery gauge pill without Unicode emojis.
fn battery_indicator(battery: i32, charging: bool, tone: Tone, label: &str) -> AnyElement {
    let pct = (battery.clamp(0, 100) as f32) / 100.0;
    let fill_color = if charging {
        Status::OK
    } else if battery <= 15 {
        Status::WARN
    } else {
        tone.fg()
    };

    div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(px(Space::SM))
        .rounded(px(Radius::PILL))
        .border_1()
        .border_color(tone.border())
        .bg(tone.fill())
        .px(px(Space::LG))
        .py(px(4.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .child(
                    div()
                        .w(px(14.0))
                        .h(px(8.0))
                        .p(px(1.0))
                        .rounded(px(1.5))
                        .border_1()
                        .border_color(tone.fg())
                        .child(
                            div()
                                .h_full()
                                .w(gpui::relative(pct))
                                .rounded(px(1.0))
                                .bg(fill_color),
                        ),
                )
                .child(div().w(px(2.0)).h(px(4.0)).bg(tone.fg()).rounded_r(px(1.0))),
        )
        .child(
            div()
                .font(numeric_font(Fonts::MONO))
                .text_size(px(Type::TWO_XS))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(tone.fg())
                .child(label.to_string()),
        )
        .into_any_element()
}

/// The top studio header strip.
pub fn status_strip(state: &AppState) -> AnyElement {
    status_strip_with_ui(
        state,
        RailUiState::default(),
        &PanelCallbacks::default(),
        None,
    )
}

/// Interactive fixed-height (`54px`) top studio header with zero vertical clipping
/// and zero label collisions.
pub fn status_strip_with_ui(
    state: &AppState,
    ui: RailUiState,
    callbacks: &PanelCallbacks,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let summary = state.status_summary();
    let tone = tone_of(summary.tone);
    let show_real_state_dot = !matches!(tone, Tone::Neutral);

    let vcam_running = state.vcam_state() == Some("running");
    let streaming = state.is_streaming();
    let vcam_label = rules::vcam_chip_label(
        state.vcam_state(),
        state.vcam_registered(),
        state
            .core
            .as_ref()
            .and_then(|c| c.vcam.as_ref())
            .map(|v| v.name.as_str()),
    );
    let vcam_consumers =
        rules::vcam_consumer_summary(state.vcam_state(), streaming, state.vcam_consumers());
    let vcam_pill_text = match vcam_consumers {
        Some(c) => format!("{vcam_label} · {c}"),
        None => vcam_label,
    };
    let vcam_dot_tone = if state.vcam_state() == Some("error") {
        Some(Tone::Danger)
    } else if vcam_running && streaming {
        Some(Tone::Ok)
    } else if vcam_running {
        Some(Tone::Accent)
    } else {
        None
    };

    let phone_state = state.core.as_ref().and_then(|c| c.phone.as_ref());
    let battery_info = phone_state.and_then(|p| {
        format_battery(p.battery, p.charging).map(|batt| {
            let tone = if p.battery <= 15 && !p.charging {
                Tone::Warn
            } else {
                Tone::Neutral
            };
            (p.battery, p.charging, batt, tone)
        })
    });
    let thermal_info = phone_state.and_then(|p| thermal_label(p.thermal));

    let has_update_available = matches!(state.update_status, UpdateStatus::Available(_));

    // Left cluster: status pill + single-line truncated muted detail.
    let mut status_pill = div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(px(Space::MD))
        .rounded(px(Radius::PILL))
        .border_1()
        .border_color(tone.border())
        .bg(tone.fill())
        .px(px(10.0))
        .py(px(4.0));

    if show_real_state_dot {
        status_pill = status_pill.child(status_dot(tone));
    }
    status_pill = status_pill.child(
        div()
            .font(Fonts::sans())
            .text_size(px(Type::TWO_SM))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .text_color(if matches!(tone, Tone::Neutral) {
                Text::PRIMARY
            } else {
                tone.fg()
            })
            .child(summary.headline.clone()),
    );

    let mut left_cluster = div()
        .flex()
        .flex_row()
        .flex_1()
        .min_w(px(0.0))
        .items_center()
        .gap(px(12.0))
        .child(status_pill);

    if let Some(detail) = summary.detail {
        left_cluster = left_cluster.child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .truncate()
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .text_color(Text::MUTED)
                .child(detail),
        );
    }

    // Right cluster: vcam pill, optional battery/thermal, Updates button, Preferences button.
    let mut right_cluster = div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(px(Space::LG));

    // Sleek PhoneCam Camera pill
    let mut vcam_pill = div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(px(Space::MD))
        .rounded(px(Radius::PILL))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.control)
        .px(px(10.0))
        .py(px(4.0));
    if let Some(dot_tone) = vcam_dot_tone {
        vcam_pill = vcam_pill.child(status_dot(dot_tone));
    }
    vcam_pill = vcam_pill.child(
        div()
            .font(Fonts::sans())
            .text_size(px(Type::XS))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(Text::HEADING)
            .child(vcam_pill_text),
    );
    right_cluster = right_cluster.child(vcam_pill);

    if let Some((pct, charging, batt, b_tone)) = battery_info {
        right_cluster = right_cluster.child(battery_indicator(pct, charging, b_tone, &batt));
    }
    if let Some((label, critical)) = thermal_info {
        right_cluster = right_cluster.child(chip(
            label,
            if critical { Tone::Danger } else { Tone::Warn },
        ));
    }

    // Updates button (small accent dot ONLY when an update is available)
    let toggle_updates_cb = callbacks.on_toggle_updates.clone();
    let mut updates_btn = div()
        .id("top-updates-btn")
        .key_context("Control")
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .gap(px(Space::MD))
        .rounded(px(Radius::SM))
        .border_1()
        .border_color(if ui.show_updates {
            Accent::DEFAULT.alpha(0.75)
        } else {
            SHELL.control_border
        })
        .bg(if ui.show_updates {
            Accent::DEFAULT.alpha(0.14)
        } else {
            SHELL.control
        })
        .px(px(10.0))
        .py(px(5.0))
        .cursor_pointer()
        .hover(|s| s.border_color(SHELL.control_border_hover).bg(SHELL.hover))
        .focus(|s| s.border_2().border_color(Accent::DEFAULT))
        .font(Fonts::sans())
        .text_size(px(Type::XS))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(Text::HEADING)
        .child("Updates")
        .on_click(move |_ev, window, cx| {
            if let Some(ref cb) = toggle_updates_cb {
                cb(window, cx);
            } else {
                push_effect(SideEffect::CheckForUpdates);
            }
        });
    if has_update_available {
        updates_btn = updates_btn.child(
            div()
                .flex_shrink_0()
                .w(px(6.0))
                .h(px(6.0))
                .rounded(px(Radius::PILL))
                .bg(Accent::DEFAULT),
        );
    }
    if let Some(ctx) = focus.as_deref_mut() {
        let (handle, idx) = ctx.acquire("top-updates-btn");
        updates_btn = updates_btn.track_focus(&handle).tab_index(idx);
    }
    right_cluster = right_cluster.child(updates_btn);

    // Preferences button
    let toggle_prefs_cb = callbacks.on_toggle_preferences.clone();
    right_cluster = right_cluster.child(
        Button::new("Preferences")
            .id("top-preferences-gear")
            .selected(ui.show_preferences)
            .focus_with(focus, "top-preferences-gear")
            .on_click(move |window, cx| {
                if let Some(ref cb) = toggle_prefs_cb {
                    cb(window, cx);
                }
            }),
    );

    div()
        .flex()
        .flex_row()
        .flex_shrink_0()
        .h(px(54.0))
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .border_b_1()
        .border_color(SHELL.border)
        .bg(SHELL.panel)
        .px(px(16.0))
        .child(left_cluster)
        .child(right_cluster)
        .into_any_element()
}

/* ------------------------------------------------------------------ */
/* Notice bar                                                          */
/* ------------------------------------------------------------------ */

/// Compact slim alert bar. Automatically suppresses stale `PairOk` notices so
/// they can never contradict "No phone connected" or "Reconnecting".
pub fn notice_bar(state: &AppState) -> AnyElement {
    if state.notices.is_empty() {
        return div().into_any_element();
    }

    let pair_ok_stale = rules::is_pair_ok_stale(state.phase(), state.has_device());
    let mut seen = std::collections::HashSet::new();
    let mut unique_notices = Vec::new();
    for notice in state.notices.iter().rev() {
        if notice.code == FailureCode::PairOk && pair_ok_stale {
            continue;
        }
        let body = notice.code.body(&notice.values);
        let key = (notice.code, body);
        if seen.insert(key) {
            unique_notices.push(notice);
            if unique_notices.len() >= 2 {
                break;
            }
        }
    }
    if unique_notices.is_empty() {
        return div().into_any_element();
    }
    unique_notices.reverse();

    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .gap(px(Space::MD))
        .px(px(Space::PAD))
        .pt(px(Space::LG))
        .children(unique_notices.into_iter().map(|notice| {
            let tone = notice.code.tone();
            let id = notice.id;
            let primary = notice.code.action_is_primary();
            let action = notice.code.action();

            let mut banner = div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(Space::PAD))
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(tone.border())
                .bg(tone.fill())
                .px(px(12.0))
                .py(px(8.0))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_1()
                        .min_w(px(0.0))
                        .items_center()
                        .gap(px(Space::LG))
                        .child(status_dot(tone))
                        .child(
                            div()
                                .flex_shrink_0()
                                .font(Fonts::sans())
                                .text_size(px(Type::XS))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(tone.fg())
                                .child(notice.code.title()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .truncate()
                                .font(Fonts::sans())
                                .text_size(px(Type::XS))
                                .text_color(Text::BODY)
                                .child(notice.code.body(&notice.values)),
                        ),
                );

            let mut actions = div()
                .flex()
                .flex_shrink_0()
                .items_center()
                .gap(px(Space::SM));
            if let Some(action) = action {
                actions = actions.child(action_button(action, primary, id));
            }
            actions = actions.child(
                Button::ghost("×")
                    .id(format!("notice-dismiss-{id}"))
                    .on_click(move |_window, _cx| push_effect(SideEffect::DismissNotice(id))),
            );
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
    let button = if primary {
        Button::primary(label)
    } else {
        Button::new(label)
    };
    button
        .id(format!("notice-action-{notice_id}"))
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

    let title = if expired {
        "This request has expired"
    } else {
        "Allow this phone to connect?"
    };
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
                .max_w(px(420.0))
                .rounded(px(Radius::LG))
                .border_1()
                .border_color(SHELL.control_border)
                .bg(SHELL.panel)
                .p(px(20.0))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TITLE))
                        .line_height(px(Type::LINE_TITLE))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::PRIMARY)
                        .child(title),
                )
                .child(
                    div()
                        .mt(px(Space::MD))
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .line_height(px(Type::LINE_XS))
                        .text_color(Text::MUTED)
                        .child(body),
                )
                .child(
                    div()
                        .mt(px(Space::PAD))
                        .rounded(px(Radius::MD))
                        .border_1()
                        .border_color(SHELL.border.alpha(0.45))
                        .bg(SHELL.recessed)
                        .px(px(Space::PAD))
                        .py(px(Space::XL))
                        .child(
                            div()
                                .min_w(px(0.0))
                                .truncate()
                                .font(Fonts::sans())
                                .text_size(px(Type::TWO_SM))
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .text_color(Text::PRIMARY)
                                .child(prompt.name.clone()),
                        )
                        .child(
                            div()
                                .mt(px(Space::XS))
                                .min_w(px(0.0))
                                .truncate()
                                .font(Fonts::sans())
                                .text_size(px(Type::XS))
                                .text_color(Text::MUTED)
                                .child(prompt.model.clone()),
                        )
                        .child(
                            div()
                                .mt(px(Space::SM))
                                .min_w(px(0.0))
                                .truncate()
                                .font(Fonts::mono())
                                .text_size(px(Type::TWO_XS))
                                .text_color(Text::MUTED)
                                .child(prompt.device_id.clone()),
                        ),
                )
                .child(
                    div()
                        .mt(px(Space::XL))
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .line_height(px(Type::LINE_XS))
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
                                .font(numeric_font(Fonts::MONO))
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
                                .child(
                                    Button::danger("Don't allow")
                                        .id("pair-deny")
                                        .on_click(|_window, _cx| {
                                            push_effect(SideEffect::ResolvePair(false))
                                        }),
                                )
                                .child(
                                    Button::primary("Allow")
                                        .id("pair-allow")
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
pub fn rail(state: &AppState, tab: Tab, narrow: bool, scroll_handle: &ScrollHandle) -> AnyElement {
    rail_with_ui(
        state,
        tab,
        narrow,
        scroll_handle,
        RailUiState::default(),
        &PanelCallbacks::default(),
        None,
    )
}

/// Interactive `rail` wired with view UI state, collapsible Labs/Gesture refs, and keyboard focus.
pub fn rail_with_ui(
    state: &AppState,
    tab: Tab,
    narrow: bool,
    scroll_handle: &ScrollHandle,
    ui: RailUiState,
    callbacks: &PanelCallbacks,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let width = if narrow {
        crate::theme::RAIL_WIDTH_NARROW
    } else {
        crate::theme::RAIL_WIDTH
    };

    let nav_track = div()
        .flex_shrink_0()
        .border_b_1()
        .border_color(SHELL.border)
        .px(px(14.0))
        .py(px(12.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(4.0))
                .rounded(px(Radius::PILL))
                .border_1()
                .border_color(SHELL.border)
                .bg(SHELL.recessed)
                .p(px(3.0))
                .children(Tab::ALL.iter().map(|entry| {
                    let selected = *entry == tab;
                    let target = *entry;
                    let hint_text = entry.hint();
                    let active_bg = hsla_from_hex(0x1a2330);
                    let mut tab_el = div()
                        .id(gpui::ElementId::Name(SharedString::from(format!(
                            "tab-{}",
                            entry.label()
                        ))))
                        .key_context("Control")
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(Radius::PILL))
                        .px(px(8.0))
                        .py(px(7.0))
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .font_weight(if selected {
                            gpui::FontWeight::SEMIBOLD
                        } else {
                            gpui::FontWeight::MEDIUM
                        })
                        .text_color(if selected { Text::PRIMARY } else { Text::MUTED })
                        .bg(if selected {
                            active_bg
                        } else {
                            gpui::transparent_black()
                        })
                        .border_1()
                        .border_color(if selected {
                            SHELL.control_border
                        } else {
                            gpui::transparent_black()
                        })
                        .cursor_pointer()
                        .hover(move |style| {
                            if selected {
                                style
                            } else {
                                style.text_color(Text::HEADING).bg(SHELL.hover.alpha(0.55))
                            }
                        })
                        .focus(|style| style.border_2().border_color(Accent::DEFAULT))
                        .tooltip(move |_window, cx| {
                            let tip = hint_text;
                            cx.new(|_| TooltipText(tip)).into()
                        })
                        .child(entry.label())
                        .on_click(move |_event, _window, _cx| {
                            push_effect(SideEffect::SetTab(target))
                        })
                        .on_key_down(move |ev, _window, cx| {
                            if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                                push_effect(SideEffect::SetTab(target));
                                cx.stop_propagation();
                            }
                        });

                    if let Some(ctx) = focus.as_deref_mut() {
                        let (handle, idx) = ctx.acquire(&format!("tab-{}", entry.index()));
                        tab_el = tab_el.track_focus(&handle).tab_index(idx);
                    }

                    tab_el.into_any_element()
                })),
        );

    let body: Vec<AnyElement> = match tab {
        Tab::Setup => vec![
            connect_panel_with_ui(state, ui, callbacks, focus.as_deref_mut()),
            paired_phone_panel(state),
        ],
        Tab::Capture => vec![capture_panel_with_ui(state, ui, callbacks, focus)],
        Tab::Publish => vec![publish_panel(state), connection_panel(state)],
        Tab::Diagnostics => vec![diagnostics_panel_with_ui(state, callbacks, focus)],
    };

    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w(px(width))
        .h_full()
        .max_h_full()
        .overflow_hidden()
        .rounded(px(Radius::LG))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.panel)
        .child(nav_track)
        .child(
            div()
                .id(gpui::ElementId::Name(SharedString::from(format!(
                    "rail-scroll-{}",
                    tab.index()
                ))))
                .flex()
                .flex_col()
                .flex_1()
                .min_h(px(0.0))
                .overflow_y_scroll()
                .track_scroll(scroll_handle)
                .gap(px(14.0))
                .p(px(14.0))
                // The bottom padding is what stops the last panel being clipped
                // by the viewport edge.
                .pb(px(Space::RAIL_BOTTOM))
                .children(body),
        )
        .into_any_element()
}

/// Simple GPUI tooltip view for tab hover hints.
struct TooltipText(&'static str);

impl gpui::Render for TooltipText {
    fn render(
        &mut self,
        _window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        div()
            .rounded(px(Radius::SM))
            .border_1()
            .border_color(SHELL.border)
            .bg(SHELL.raised)
            .px(px(Space::MD))
            .py(px(Space::SM))
            .font(Fonts::sans())
            .text_size(px(Type::XS))
            .text_color(Text::PRIMARY)
            .child(self.0)
    }
}

/* ------------------------------------------------------------------ */
/* Setup                                                               */
/* ------------------------------------------------------------------ */

/// Pairing. Describes where the button is; it does not provide one, because the
/// stage owns that action and offering the same job twice is what made the
/// first-run screen feel like a coin toss.
pub fn connect_panel(state: &AppState) -> AnyElement {
    connect_panel_with_ui(
        state,
        RailUiState::default(),
        &PanelCallbacks::default(),
        None,
    )
}

/// Pairing and connection setup panel with keyboard focus support.
pub fn connect_panel_with_focus(
    state: &AppState,
    focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    connect_panel_with_ui(
        state,
        RailUiState::default(),
        &PanelCallbacks::default(),
        focus,
    )
}

/// Clean studio Setup tab: compact Wi-Fi card (IP:Port badge + 2 concise steps),
/// compact USB Cable card with collapsible "USB Debugging Help" accordion, and
/// cleanly stacked USB auto-connect toggle.
pub fn connect_panel_with_ui(
    state: &AppState,
    ui: RailUiState,
    callbacks: &PanelCallbacks,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let pairing_open = state.core.as_ref().map(|c| c.pairing.open).unwrap_or(false);
    let connected = state.has_device();
    let streaming = state.is_streaming();

    let mut panel = Panel::new("Connection");
    if pairing_open {
        panel = panel.action(
            Button::new("Cancel")
                .id("setup-pairing-cancel")
                .focus_with(focus.as_deref_mut(), "setup-pairing-cancel")
                .on_click(|_window, _cx| push_effect(SideEffect::DisarmPairing)),
        );
    }

    let mut children: Vec<AnyElement> = Vec::new();

    if connected {
        children.push(tinted_box(
            if streaming { Tone::Ok } else { Tone::Accent },
            vec![div()
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .line_height(px(Type::LINE_XS))
                .text_color(Text::HEADING)
                .child("A phone is connected. Pairing another device will not disturb the current session.")
                .into_any_element()],
        ));
    }

    if pairing_open {
        children.push(
            div()
                .w_full()
                .min_w(px(0.0))
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(Accent::DEFAULT.alpha(0.45))
                .bg(Accent::DEFAULT.alpha(0.08))
                .p(px(14.0))
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Accent::DEFAULT)
                        .child("QR code ready on stage"),
                )
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .line_height(px(Type::LINE_XS))
                        .text_color(Text::BODY)
                        .child("Point PhoneCam on your phone at the QR code on the left."),
                )
                .into_any_element(),
        );
    }

    let has_lan_ip = state
        .local_ip
        .as_deref()
        .map(|ip| !ip.is_empty())
        .unwrap_or(false);
    let wifi_endpoint = match state.local_ip.as_deref().filter(|ip| !ip.is_empty()) {
        Some(ip) => format!("{ip}:{}", phonecam_protocol::SERVICE_PORT),
        None => "Unavailable".to_string(),
    };

    // Compact Wi-Fi card (IP:Port monospace badge + 2 concise steps)
    let wifi_card = div()
        .w_full()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.recessed)
        .p(px(16.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_SM))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::PRIMARY)
                        .child("Wi-Fi"),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .rounded(px(Radius::SM))
                        .border_1()
                        .border_color(SHELL.control_border)
                        .bg(SHELL.control)
                        .px(px(8.0))
                        .py(px(3.0))
                        .font(numeric_font(Fonts::MONO))
                        .text_size(px(Type::TWO_XS))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(if has_lan_ip {
                            Text::PRIMARY
                        } else {
                            Text::MUTED
                        })
                        .child(wifi_endpoint),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(wifi_step_row(
                    "1.",
                    "Keep your phone and PC on the same Wi-Fi network.",
                ))
                .child(wifi_step_row(
                    "2.",
                    "Scan the QR code on the left, or enter the address above in PhoneCam Android.",
                )),
        )
        .into_any_element();

    children.push(wifi_card);

    // Compact USB Cable card with collapsible "USB Debugging Help" accordion
    children.push(usb_list_with_ui(
        state,
        ui.usb_help_expanded,
        callbacks.on_toggle_usb_help.clone(),
        focus.as_deref_mut(),
    ));

    // Cleanly stacked USB automation toggle
    children.push(
        div()
            .w_full()
            .rounded(px(Radius::MD))
            .border_1()
            .border_color(SHELL.border)
            .bg(SHELL.recessed)
            .px(px(10.0))
            .py(px(6.0))
            .child(
                Toggle::new("Auto-connect on USB plug", state.settings.auto_connect_usb)
                    .id("setup-auto-connect-usb")
                    .focus_with(focus, "setup-auto-connect-usb")
                    .detail(Some(if state.settings.auto_connect_usb {
                        "Automatically bridges a plugged-in USB phone"
                    } else {
                        "Connect manually via USB button"
                    }))
                    .on_change(|val, _window, _cx| push_effect(SideEffect::SetAutoConnectUsb(val)))
                    .into_element(),
            )
            .into_any_element(),
    );

    panel.children(children).into_element()
}

fn wifi_step_row(num: &'static str, text: &'static str) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(Space::SM))
        .child(
            div()
                .flex_shrink_0()
                .w(px(16.0))
                .font(numeric_font(Fonts::SANS))
                .text_size(px(Type::XS))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(Accent::DEFAULT)
                .child(num),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .line_height(px(Type::LINE_XS))
                .text_color(Text::BODY)
                .child(text),
        )
        .into_any_element()
}

/// What action (or status hint) an ADB-listed USB row can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbDeviceAction {
    UsePhone,
    AuthorizeHint,
    OfflineHint,
}

/// Formats a device model string into a clean, human-readable name.
pub fn clean_device_model(raw_model: &str) -> String {
    let trimmed = raw_model.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed.replace('_', " ")
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
pub fn usb_list_summary(
    devices: &[phonecam_protocol::AdbDevice],
    forwarding: bool,
) -> &'static str {
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
#[allow(dead_code)]
fn usb_list(state: &AppState) -> AnyElement {
    usb_list_with_ui(state, false, None, None)
}

fn usb_list_with_ui(
    state: &AppState,
    help_expanded: bool,
    on_toggle_help: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let usb = state.core.as_ref().map(|c| &c.usb);
    let adb = usb.map(|u| u.adb_available);
    let device_count = usb.map(|u| u.devices.len()).unwrap_or(0);
    let readiness = rules::usb_readiness(adb, device_count);

    if let rules::UsbReadiness::Device = readiness {
        let forwarding = usb.map(|u| u.forwarding).unwrap_or(false);
        let devices = usb.map(|u| u.devices.clone()).unwrap_or_default();
        let mut column = div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap(px(10.0))
            .rounded(px(Radius::MD))
            .border_1()
            .border_color(SHELL.border)
            .bg(SHELL.recessed)
            .p(px(16.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(10.0))
                    .child(
                        div()
                            .font(Fonts::sans())
                            .text_size(px(Type::TWO_SM))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(Text::PRIMARY)
                            .child("USB Cable"),
                    )
                    .child(chip(
                        if forwarding {
                            "Tunnel open"
                        } else {
                            "Detected"
                        },
                        if forwarding { Tone::Ok } else { Tone::Accent },
                    )),
            )
            .child(prose(usb_list_summary(&devices, forwarding)));

        for device in devices {
            let model_clean = clean_device_model(&device.model);
            let model_label = if model_clean.is_empty() {
                format!("Android Device ({})", device.serial)
            } else {
                model_clean
            };
            let serial_for_connect = device.serial.clone();
            let serial_id = device.serial.clone();
            let action = usb_device_action(&device.state);
            let mode_label = if forwarding {
                "USB link active"
            } else {
                match action {
                    UsbDeviceAction::UsePhone => "Cable ready",
                    UsbDeviceAction::AuthorizeHint => "Authorize on phone",
                    UsbDeviceAction::OfflineHint => "Offline",
                }
            };
            let serial_detail = format!("{} · {}", device.serial, mode_label);

            column = column.child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(10.0))
                    .rounded(px(Radius::SM))
                    .border_1()
                    .border_color(SHELL.border)
                    .bg(SHELL.panel)
                    .px(px(12.0))
                    .py(px(10.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .font(Fonts::sans())
                                    .text_size(px(Type::TWO_SM))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(Text::PRIMARY)
                                    .child(model_label),
                            )
                            .child(
                                div()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .font(Fonts::mono())
                                    .text_size(px(Type::TWO_XS))
                                    .text_color(Text::MUTED)
                                    .child(serial_detail),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_end()
                            .child(match (forwarding, action) {
                                (true, _) => {
                                    let btn_id = format!("usb-close-{serial_id}");
                                    Button::danger("Close tunnel")
                                        .id(btn_id.clone())
                                        .focus_with(focus.as_deref_mut(), &btn_id)
                                        .on_click(|_window, _cx| {
                                            push_effect(SideEffect::DisconnectUsb)
                                        })
                                        .into_any_element()
                                }
                                (false, UsbDeviceAction::UsePhone) => {
                                    let btn_id = format!("usb-use-{serial_id}");
                                    Button::primary("Connect")
                                        .id(btn_id.clone())
                                        .focus_with(focus.as_deref_mut(), &btn_id)
                                        .on_click(move |_window, _cx| {
                                            push_effect(SideEffect::ConnectUsb(Some(
                                                serial_for_connect.clone(),
                                            )))
                                        })
                                        .into_any_element()
                                }
                                (false, UsbDeviceAction::AuthorizeHint) => {
                                    chip("Approve on phone", Tone::Warn)
                                }
                                (false, UsbDeviceAction::OfflineHint) => {
                                    chip("Offline", Tone::Neutral)
                                }
                            }),
                    ),
            );
        }

        column = column.child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(10.0))
                .child(
                    Button::new("Check again")
                        .id("usb-check-again-list")
                        .focus_with(focus.as_deref_mut(), "usb-check-again-list")
                        .on_click(|_window, _cx| push_effect(SideEffect::RefreshUsb)),
                )
                .child(
                    Button::new("Restart ADB")
                        .id("usb-restart-adb-list")
                        .focus_with(focus.as_deref_mut(), "usb-restart-adb-list")
                        .on_click(|_window, _cx| push_effect(SideEffect::RestartAdb)),
                ),
        );

        return column.into_any_element();
    }

    let (chip_label, summary_copy) = match readiness {
        rules::UsbReadiness::ToolsMissing => (
            "Tools missing",
            "Android platform-tools are missing from this copy. Reinstall PhoneCam or download platform-tools.",
        ),
        rules::UsbReadiness::Unknown => (
            "Not checked",
            "Plug in a USB data cable with USB debugging enabled, then click Check again.",
        ),
        _ => (
            "Not connected",
            "Plug in a USB data cable with USB debugging enabled for zero-jitter tethered video.",
        ),
    };

    let mut actions = div().flex().flex_row().flex_wrap().gap(px(10.0));

    if matches!(readiness, rules::UsbReadiness::ToolsMissing) {
        actions = actions.child(
            Button::new("Download tools")
                .id("usb-download-tools")
                .focus_with(focus.as_deref_mut(), "usb-download-tools")
                .on_click(|_window, _cx| push_effect(SideEffect::OpenPlatformToolsUrl)),
        );
    }

    actions = actions.child(
        Button::new("Check again")
            .id("usb-check-again-empty")
            .focus_with(focus.as_deref_mut(), "usb-check-again-empty")
            .on_click(|_window, _cx| push_effect(SideEffect::RefreshUsb)),
    );

    if !matches!(readiness, rules::UsbReadiness::ToolsMissing) {
        actions = actions.child(
            Button::new("Restart ADB")
                .id("usb-restart-adb-empty")
                .focus_with(focus.as_deref_mut(), "usb-restart-adb-empty")
                .on_click(|_window, _cx| push_effect(SideEffect::RestartAdb)),
        );
    }

    let mut card = div()
        .w_full()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.recessed)
        .p(px(16.0))
        .child(
            div()
                .w_full()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_SM))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::PRIMARY)
                        .child("USB Cable"),
                )
                .child(chip(chip_label, Tone::Neutral)),
        )
        .child(prose(summary_copy))
        .child(actions);

    if !matches!(readiness, rules::UsbReadiness::ToolsMissing) {
        let help_toggle = div()
            .pt(px(6.0))
            .border_t_1()
            .border_color(SHELL.border)
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(10.0))
            .child(
                div()
                    .font(Fonts::sans())
                    .text_size(px(Type::XS))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(Text::BODY)
                    .child("USB Debugging Help"),
            )
            .child(
                Button::ghost(if help_expanded { "Hide" } else { "Show steps" })
                    .id("setup-usb-help-toggle")
                    .focus_with(focus, "setup-usb-help-toggle")
                    .on_click(move |window, cx| {
                        if let Some(ref cb) = on_toggle_help {
                            cb(window, cx);
                        }
                    }),
            );
        card = card.child(help_toggle);
        if help_expanded {
            card = card.child(usb_debugging_guide());
        }
    }

    card.into_any_element()
}

/// The step-by-step instructions for turning on USB debugging.
pub const USB_DEBUGGING_STEPS: [&str; 4] = [
    "Settings \u{203a} About phone \u{203a} tap OS version 7 times to unlock Developer options.",
    "Settings \u{203a} Additional settings \u{203a} Developer options \u{203a} turn on USB debugging.",
    "On MIUI/HyperOS, also enable Install via USB and USB debugging (Security settings); both must be on for the cable to work.",
    "Plug in the cable, unlock the phone, and tap Allow on the debugging prompt.",
];

/// Step-by-step USB debugging guide.
pub fn usb_debugging_guide() -> AnyElement {
    div()
        .mt(px(Space::MD))
        .flex()
        .flex_col()
        .gap(px(Space::SM))
        .font(Fonts::sans())
        .text_size(px(Type::XS))
        .line_height(px(Type::LINE_XS))
        .text_color(Text::MUTED)
        .children(USB_DEBUGGING_STEPS.iter().enumerate().map(|(i, step)| {
            div()
                .flex()
                .flex_row()
                .items_start()
                .gap(px(Space::SM))
                .child(
                    div()
                        .flex_shrink_0()
                        .w(px(16.0))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(Accent::DEFAULT)
                        .child(format!("{}.", i + 1)),
                )
                .child(div().flex_1().min_w(px(0.0)).child(*step))
        }))
        .into_any_element()
}

/// The paired phone, and the way to forget it.
pub fn paired_phone_panel(state: &AppState) -> AnyElement {
    let device = state.core.as_ref().and_then(|c| c.device.as_ref());
    let stored_id = state.settings.last_device_id.clone();

    if device.is_none() && stored_id.is_none() {
        return div().into_any_element();
    }

    let name = device
        .map(|d| d.name.clone())
        .unwrap_or_else(|| "this phone".to_string());
    let facts = state.device_facts();
    let connected = device.is_some();
    let streaming = state.is_streaming();

    let panel = Panel::new(name.clone()).action(chip(
        if connected {
            "Connected"
        } else {
            "Not connected"
        },
        if connected && streaming {
            Tone::Ok
        } else if connected {
            Tone::Accent
        } else {
            Tone::Neutral
        },
    ));

    let mut children: Vec<AnyElement> = Vec::new();
    let mut kv_items: Vec<AnyElement> = Vec::new();

    if let Some(device) = device {
        if let Some(model) = facts.model {
            kv_items.push(key_value("Model", clean_device_model(&model), false));
        }
        if let Some(version) = facts.android_version {
            kv_items.push(key_value("Android", version.to_string(), false));
        }
        if let Some(batt) = state
            .core
            .as_ref()
            .and_then(|c| c.phone.as_ref())
            .and_then(|p| format_battery(p.battery, p.charging))
        {
            kv_items.push(key_value("Battery", batt, false));
        }
        kv_items.push(key_value("Device id", device.device_id.clone(), true));
    } else if let Some(id) = &stored_id {
        kv_items.push(key_value("Device id", id.clone(), true));
    }

    if !kv_items.is_empty() {
        children.push(well(kv_items));
    }

    let target_id = device
        .map(|d| d.device_id.clone())
        .or_else(|| stored_id.clone());

    children.push(
        div()
            .w_full()
            .mt(px(Space::PAD))
            .rounded(px(Radius::MD))
            .border_1()
            .border_color(Status::DANGER.alpha(0.4))
            .bg(Status::DANGER.alpha(0.05))
            .px(px(Space::LG))
            .py(px(Space::MD))
            .child(
                div()
                    .font(Fonts::sans())
                    .text_size(px(Type::XS))
                    .line_height(px(Type::LINE_XS))
                    .text_color(Text::MUTED)
                    .child(format!(
                        "Forgetting {name} deletes its credentials from this PC. It will need a new pairing code."
                    )),
            )
            .child(
                div()
                    .mt(px(Space::LG))
                    .child(Button::danger(format!("Forget {name}")).on_click(move |_window, _cx| {
                        push_effect(SideEffect::ForgetDevice(target_id.clone()))
                    })),
            )
            .into_any_element(),
    );

    panel.children(children).into_element()
}

/* ------------------------------------------------------------------ */
/* Capture                                                             */
/* ------------------------------------------------------------------ */

/// Converts normalized `[0.0, 1.0]` audio level to logarithmic dBFS (`[-60.0, 0.0]`).
pub fn audio_level_dbfs(audio_level: f32) -> f32 {
    let level = audio_level.clamp(0.0, 1.0);
    if level <= 1e-3 {
        -60.0
    } else {
        (20.0 * level.log10()).clamp(-60.0, 0.0)
    }
}

/// Logarithmic dBFS threshold for each of the 16 VU segments:
/// 10 green (-60..-12 dBFS), 4 amber (-12..-3 dBFS), 2 red (> -3 dBFS).
pub fn vu_segment_threshold_db(index: usize) -> f32 {
    if index < 10 {
        -60.0 + (index as f32 + 1.0) * (48.0 / 10.0)
    } else if index < 14 {
        -12.0 + ((index - 10) as f32 + 1.0) * (9.0 / 4.0)
    } else if index == 14 {
        -3.0
    } else {
        -1.0
    }
}

/// Real-time 16-segment logarithmic dBFS audio level meter (10 green / 4 amber / 2 red).
pub fn audio_vu_meter(audio_level: f32, audio_enabled: bool) -> AnyElement {
    let num_segments = 16;
    let db = audio_level_dbfs(audio_level);
    let mut meter = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(2.0))
        .w_full()
        .h(px(12.0))
        .p(px(1.0))
        .rounded(px(Radius::SM))
        .bg(SHELL.recessed)
        .border_1()
        .border_color(SHELL.border.alpha(0.5));

    for i in 0..num_segments {
        let threshold_db = vu_segment_threshold_db(i);
        let active = audio_enabled && audio_level > 1e-3 && db >= threshold_db;
        let (lit_color, dim_color) = if i >= 14 {
            (Tone::Danger.dot(), Tone::Danger.dot().alpha(0.18))
        } else if i >= 10 {
            (Tone::Warn.dot(), Tone::Warn.dot().alpha(0.18))
        } else {
            (Tone::Ok.dot(), Tone::Ok.dot().alpha(0.18))
        };
        let seg_color = if active { lit_color } else { dim_color };
        meter = meter.child(div().flex_1().h_full().rounded(px(1.0)).bg(seg_color));
    }
    meter.into_any_element()
}

fn audio_vu_row(audio_level: f32, audio_enabled: bool, label: &'static str) -> AnyElement {
    let db = audio_level_dbfs(audio_level);
    let db_label = if !audio_enabled || audio_level <= 1e-3 {
        "-60 dBFS".to_string()
    } else {
        format!("{db:.0} dBFS")
    };

    div()
        .flex()
        .flex_col()
        .gap(px(3.0))
        .w_full()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .child(label),
                )
                .child(
                    div()
                        .font(numeric_font(Fonts::MONO))
                        .text_size(px(Type::TWO_XS))
                        .text_color(if audio_enabled && audio_level > 0.02 {
                            Text::PRIMARY
                        } else {
                            Text::MUTED
                        })
                        .child(db_label),
                ),
        )
        .child(audio_vu_meter(audio_level, audio_enabled))
        .into_any_element()
}

/// Hand gesture toggle and collapsible 6-gesture reference inside Labs.
pub fn hand_gesture_guide_card(state: &AppState) -> AnyElement {
    hand_gesture_guide_card_with_ui(state, true, None, None)
}

fn hand_gesture_guide_card_with_ui(
    state: &AppState,
    ref_expanded: bool,
    on_toggle_ref: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let gestures_enabled = state.settings.hand_gestures;

    let active_status_chip = if gestures_enabled {
        if let Some(gesture) = state.active_gesture {
            match gesture {
                phonecam_media::HandGesture::OpenPalm => chip("Open Palm detected", Tone::Ok),
                phonecam_media::HandGesture::PeaceV => chip("V-Sign detected", Tone::Ok),
                phonecam_media::HandGesture::IndexPoint => chip("Index Up detected", Tone::Ok),
                phonecam_media::HandGesture::ShakaCall => chip("Shaka detected", Tone::Ok),
                phonecam_media::HandGesture::PinchOk => chip("Pinch/OK detected", Tone::Ok),
                phonecam_media::HandGesture::ThumbsUp => chip("Thumbs Up detected", Tone::Ok),
                phonecam_media::HandGesture::None => chip("Active (6 gestures)", Tone::Ok),
            }
        } else {
            chip("Active (6 gestures)", Tone::Ok)
        }
    } else {
        chip("Disabled", Tone::Neutral)
    };

    let master_toggle = Toggle::new("Hand Gestures", gestures_enabled)
        .id("labs-hand-gestures")
        .focus_with(focus.as_deref_mut(), "labs-hand-gestures")
        .detail(Some(if gestures_enabled {
            "Palm Lock | V-Sign Zoom | Shaka Reset"
        } else {
            "Off"
        }))
        .on_change(|enabled, _window, _cx| push_effect(SideEffect::SetHandGestures(enabled)))
        .into_element();

    struct GestureInfo {
        badge: &'static str,
        name: &'static str,
        action: &'static str,
        how_to: &'static str,
        camera_effect: &'static str,
        gesture: phonecam_media::HandGesture,
    }

    let gesture_catalog = [
        GestureInfo {
            badge: "5F",
            name: "Open Palm",
            action: "Lock / Unlock Tracking",
            how_to: "Hold an open palm facing camera with all 5 fingers spread.",
            camera_effect: "Pauses or resumes face tracking. When locked, framing stays fixed even if you move.",
            gesture: phonecam_media::HandGesture::OpenPalm,
        },
        GestureInfo {
            badge: "V",
            name: "Peace / V-Sign",
            action: "Toggle Zoom (1.0x to 2.0x)",
            how_to: "Extend index and middle fingers in a V-sign with other fingers folded in.",
            camera_effect: "Smoothly switches digital zoom between 1.0x wide-angle and 2.0x zoomed punch-in.",
            gesture: phonecam_media::HandGesture::PeaceV,
        },
        GestureInfo {
            badge: "1F",
            name: "Pointing Up",
            action: "1.5x Medium Zoom",
            how_to: "Point index finger straight up with thumb and other fingers folded.",
            camera_effect: "Punches in to 1.5x medium focal framing, centered on the subject.",
            gesture: phonecam_media::HandGesture::IndexPoint,
        },
        GestureInfo {
            badge: "Y",
            name: "Shaka / Call",
            action: "Reset Framing & Zoom (1.0x Center)",
            how_to: "Extend thumb and pinky outward with middle 3 fingers curled in.",
            camera_effect: "Resets digital zoom to 1.0x and recenters framing to the default center.",
            gesture: phonecam_media::HandGesture::ShakaCall,
        },
        GestureInfo {
            badge: "OK",
            name: "Pinch / OK",
            action: "2.2x Close-Up Macro Focus",
            how_to: "Touch thumb and index fingertips together forming a circular ring.",
            camera_effect: "Enters 2.2x close-up macro framing for showing notes, cards, or fine objects.",
            gesture: phonecam_media::HandGesture::PinchOk,
        },
        GestureInfo {
            badge: "UP",
            name: "Thumbs Up",
            action: "Confirm / Toggle Zoom",
            how_to: "Point thumb upward with closed fist facing camera.",
            camera_effect: "Quick affirmative trigger that toggles zoom between 1.0x and 2.0x.",
            gesture: phonecam_media::HandGesture::ThumbsUp,
        },
    ];

    let mut container = div()
        .w_full()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap(px(Space::SM))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(Space::SM))
                .child(master_toggle)
                .child(active_status_chip),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(Space::SM))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::BODY)
                        .child("Supported gestures (6)"),
                )
                .child(
                    Button::ghost(if ref_expanded {
                        "Hide guide"
                    } else {
                        "Show guide"
                    })
                    .id("labs-gesture-guide-toggle")
                    .focus_with(focus, "labs-gesture-guide-toggle")
                    .on_click(move |window, cx| {
                        if let Some(ref cb) = on_toggle_ref {
                            cb(window, cx);
                        }
                    }),
                ),
        );

    if ref_expanded {
        let gesture_cards = gesture_catalog.iter().map(|item| {
            let is_detected = gestures_enabled && state.active_gesture == Some(item.gesture);
            let border_color = if is_detected {
                Tone::Ok.border()
            } else {
                SHELL.border.alpha(0.35)
            };
            let bg_color = if is_detected {
                Tone::Ok.fill()
            } else {
                SHELL.recessed
            };

            div()
                .w_full()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(border_color)
                .bg(bg_color)
                .p(px(Space::MD))
                .gap(px(Space::XS))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .w_full()
                        .min_w(px(0.0))
                        .gap(px(Space::SM))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(Space::SM))
                                .min_w(px(0.0))
                                .flex_1()
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .w(px(28.0))
                                        .h(px(24.0))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(Radius::SM))
                                        .border_1()
                                        .border_color(SHELL.border.alpha(0.6))
                                        .bg(SHELL.control)
                                        .font(numeric_font(Fonts::MONO))
                                        .text_size(px(Type::TWO_XS))
                                        .font_weight(gpui::FontWeight::BOLD)
                                        .text_color(Accent::DEFAULT)
                                        .child(item.badge),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .min_w(px(0.0))
                                        .flex_1()
                                        .child(
                                            div()
                                                .truncate()
                                                .font(Fonts::sans())
                                                .text_size(px(Type::TWO_SM))
                                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                                .text_color(Text::HEADING)
                                                .child(item.name),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .font(Fonts::sans())
                                                .text_size(px(Type::TWO_XS))
                                                .font_weight(gpui::FontWeight::MEDIUM)
                                                .text_color(Accent::DEFAULT)
                                                .child(item.action),
                                        ),
                                ),
                        )
                        .child(div().flex_shrink_0().child(if is_detected {
                            chip("Detected", Tone::Ok)
                        } else if gestures_enabled {
                            chip("Ready", Tone::Neutral)
                        } else {
                            chip("Off", Tone::Neutral)
                        })),
                )
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .line_height(px(Type::LINE_XS))
                        .text_color(Text::BODY)
                        .child(format!("How: {}", item.how_to)),
                )
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .line_height(px(Type::LINE_XS))
                        .text_color(Text::MUTED)
                        .child(format!("Effect: {}", item.camera_effect)),
                )
                .into_any_element()
        });

        container = container
            .child(prose(
                "Hold a gesture clearly in front of the camera for ~0.3s to trigger.",
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(Space::SM))
                    .w_full()
                    .min_w(px(0.0))
                    .children(gesture_cards),
            );
    }

    container.into_any_element()
}

/// What the phone captures.
pub fn capture_panel(state: &AppState) -> AnyElement {
    capture_panel_with_ui(
        state,
        RailUiState::default(),
        &PanelCallbacks::default(),
        None,
    )
}

/// Capture panel with Quick Framing, Sliders, and collapsible Labs drawer.
pub fn capture_panel_with_ui(
    state: &AppState,
    ui: RailUiState,
    callbacks: &PanelCallbacks,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let streaming = state.is_streaming();
    let has_device = state.has_device();
    let action_control = if streaming {
        Button::danger("Stop Camera")
            .id("capture-stop-camera")
            .focus_with(focus.as_deref_mut(), "capture-stop-camera")
            .on_click(|_window, _cx| push_effect(SideEffect::StopStream))
            .into_any_element()
    } else if has_device {
        Button::primary("Start Camera")
            .id("capture-start-camera")
            .focus_with(focus.as_deref_mut(), "capture-start-camera")
            .on_click(|_window, _cx| push_effect(SideEffect::StartStream))
            .into_any_element()
    } else {
        chip("No phone", Tone::Neutral)
    };
    let panel = Panel::new("Phone camera").action(action_control);

    let not_connected = !state.status.connected || !state.has_device();
    let probed_cameras = state
        .capabilities
        .as_ref()
        .map(|c| c.cameras.clone())
        .unwrap_or_default();
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
            vec![div()
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .line_height(px(Type::LINE_XS))
                .text_color(Text::MUTED)
                .child(
                    "No phone connected. Settings below will apply automatically once connected.",
                )
                .into_any_element()],
        ));
    }

    // --- Framing (Mirror, Rotation, Auto-Rotate) ---
    let next_mirror = !state.settings.output_mirror;
    let quick_framing = section(
        "Framing",
        vec![
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .gap(px(10.0))
                .mb(px(Space::LG))
                .child(
                    Button::new(if state.settings.output_mirror {
                        "Mirrored"
                    } else {
                        "Mirror"
                    })
                    .id("quick-mirror")
                    .selected(state.settings.output_mirror)
                    .focus_with(focus.as_deref_mut(), "quick-mirror")
                    .on_click(move |_window, _cx| push_effect(SideEffect::SetMirror(next_mirror))),
                )
                .children([0u32, 90, 180, 270].into_iter().map(|deg| {
                    let active = state.settings.output_rotation == deg;
                    pill_button(
                        format!("quick-rot-{deg}"),
                        format!("{deg}°"),
                        active,
                        false,
                        move |_window, _cx| push_effect(SideEffect::SetRotation(deg)),
                    )
                }))
                .into_any_element(),
            Toggle::new("Auto-rotate with phone", state.settings.auto_rotate)
                .id("quick-auto-rotate")
                .focus_with(focus.as_deref_mut(), "quick-auto-rotate")
                .detail(Some(if state.settings.auto_rotate {
                    "Follows phone orientation"
                } else {
                    "Orientation locked"
                }))
                .on_change(|enabled, _window, _cx| push_effect(SideEffect::SetAutoRotate(enabled)))
                .into_element(),
        ],
    );

    let resolved_id = cameras
        .iter()
        .find(|c| c.id == selected)
        .or_else(|| cameras.iter().find(|c| c.facing.as_str() == selected))
        .map(|c| c.id.as_str())
        .unwrap_or(selected.as_str())
        .to_string();

    let res_options = resolution_presets(
        selected_cam
            .map(|c| c.resolutions.as_slice())
            .unwrap_or(&[]),
        &state.settings.resolution,
    );
    let res_detail: Option<SharedString> = if let Some(cam) = selected_cam {
        let max_fps = cam.fps.iter().copied().max().unwrap_or(60);
        let mut caps_str = Vec::new();
        for r in &cam.resolutions {
            let lbl = resolution_label_of(r);
            let fps_cap = if lbl == "4K" || lbl == "2160p" {
                30
            } else {
                max_fps
            };
            let entry = format!("{lbl}@{fps_cap}");
            if !caps_str.contains(&entry) {
                caps_str.push(entry);
            }
        }
        if !caps_str.is_empty() {
            Some(format!("Supported on selected lens: {}", caps_str.join(" · ")).into())
        } else {
            None
        }
    } else {
        None
    };

    let res_hint: SharedString = phone
        .map(|p| p.actual_resolution.clone())
        .filter(|s| !s.is_empty())
        .map(|actual| {
            let actual_fps = phone.map(|p| p.actual_fps).unwrap_or(0);
            if actual_fps > 0 {
                SharedString::from(format!(
                    "{}@{actual_fps} ({actual})",
                    resolution_label_of(&actual)
                ))
            } else {
                SharedString::from(format!("{} ({actual})", resolution_label_of(&actual)))
            }
        })
        .unwrap_or_else(|| {
            if let Some(cam) = selected_cam {
                let base = resolution_label_of(&state.settings.resolution);
                if base == "Auto" {
                    SharedString::from("Auto (Dynamic)")
                } else {
                    let max_fps = if base == "4K" || base == "2160p" {
                        30
                    } else {
                        cam.fps.iter().copied().max().unwrap_or(60)
                    };
                    SharedString::from(format!("{base}@{max_fps}"))
                }
            } else {
                SharedString::from(resolution_label_of(&state.settings.resolution))
            }
        });

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

    let bitrate_mbps = (current_bitrate as f32 / 1_000_000.0).clamp(1.0, 20.0);
    let bitrate_control = div()
        .flex()
        .flex_col()
        .gap(px(Space::LG))
        .child(
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .justify_between()
                .child(
                    div()
                        .font(numeric_font(Fonts::MONO))
                        .text_size(px(Type::TITLE))
                        .font_weight(gpui::FontWeight::BOLD)
                        .text_color(Accent::DEFAULT)
                        .child(format!("{bitrate_mbps:.1} Mbps")),
                )
                .child(
                    div()
                        .font(numeric_font(Fonts::SANS))
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .child("1.0 to 20.0 Mbps"),
                ),
        )
        .child(
            Slider::new("slider-bitrate", bitrate_mbps, 1.0, 20.0)
                .step(0.5)
                .focus_with(focus.as_deref_mut(), "slider-bitrate")
                .on_change(move |mbps, _window, _cx| {
                    let bps = ((mbps * 2.0).round() as u64) * 500_000;
                    push_effect(SideEffect::SetVideo {
                        fps: current_fps_opt,
                        bitrate: bps.clamp(1_000_000, 20_000_000),
                    });
                }),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(Space::LG))
                .children(
                    [
                        (2_000_000, "2M Eco", "bitrate-2M"),
                        (4_000_000, "4M Normal", "bitrate-4M"),
                        (8_000_000, "8M HD", "bitrate-8M"),
                        (16_000_000, "16M Ultra", "bitrate-16M"),
                    ]
                    .into_iter()
                    .map(|(bps, label, id)| {
                        let active = current_bitrate == bps;
                        pill_button_compact(id, label, active, false, move |_window, _cx| {
                            push_effect(SideEffect::SetVideo {
                                fps: current_fps_opt,
                                bitrate: bps,
                            })
                        })
                    }),
                ),
        )
        .into_any_element();

    children.push(section(
        "Source & Quality",
        vec![
            camera_tiles(&cameras, &resolved_id),
            div().h(px(Space::SM)).into_any_element(),
            crate::views::widgets::field(
                "Resolution",
                Some(res_hint),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::LG))
                    .children(res_options.into_iter().map(|res| {
                        let active = state.settings.resolution == res;
                        let label = if res == "auto" {
                            "Auto".to_string()
                        } else {
                            let base_label = resolution_label_of(&res);
                            if let Some(cam) = selected_cam {
                                let max_fps = if base_label == "4K" || base_label == "2160p" {
                                    30
                                } else {
                                    cam.fps.iter().copied().max().unwrap_or(60)
                                };
                                format!("{base_label}@{max_fps}")
                            } else {
                                base_label
                            }
                        };
                        let id = format!("res-{res}");
                        pill_button(id, label, active, false, move |_window, _cx| {
                            push_effect(SideEffect::SetResolution(res.clone()))
                        })
                    }))
                    .into_any_element(),
                res_detail,
            ),
            crate::views::widgets::field(
                "Frame rate",
                fps_hint,
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::LG))
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
                None,
                bitrate_control,
                Some("Higher bitrate improves detail and needs stronger Wi-Fi. 8 Mbps works well for 1080p.".into()),
            ),
        ],
    ));
    children.push(quick_framing);

    // Link & smoothness (plain language; technical RTT/cadence stays in Diagnostics)
    let latency_ms = state
        .core
        .as_ref()
        .map(|c| c.stats.latency_ms)
        .unwrap_or(-1.0);
    let active_fps = phone
        .map(|p| p.actual_fps)
        .filter(|&f| f > 0)
        .or_else(|| phone.map(|p| p.fps).filter(|&f| f > 0))
        .unwrap_or(if state.settings.fps == 0 {
            30
        } else {
            state.settings.fps
        });
    let frame_interval_ms = if active_fps > 0 {
        1000.0 / active_fps as f32
    } else {
        33.3
    };

    let (latency_val, status_badge, status_color) = if not_connected || latency_ms < 0.0 {
        ("-- ms".to_string(), "Idle", Text::MUTED)
    } else if latency_ms < 25.0 {
        (format!("{latency_ms:.1} ms"), "Instant", Status::OK)
    } else if latency_ms < 50.0 {
        (format!("{latency_ms:.1} ms"), "Smooth", Accent::DEFAULT)
    } else if latency_ms < 100.0 {
        (format!("{latency_ms:.1} ms"), "Normal", Status::WARN)
    } else {
        (format!("{latency_ms:.1} ms"), "Queued", Status::DANGER_TEXT)
    };

    let sensor_cadence_str = format!("{frame_interval_ms:.1} ms ({active_fps} fps)");

    let responsiveness_well = well(vec![
        div()
            .flex()
            .flex_row()
            .justify_between()
            .items_center()
            .mb(px(Space::SM))
            .child(
                div()
                    .font(Fonts::sans())
                    .text_size(px(Type::XS))
                    .text_color(Text::MUTED)
                    .child("Live stream responsiveness"),
            )
            .child(
                div()
                    .font(Fonts::sans())
                    .text_size(px(Type::TWO_XS))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(status_color)
                    .bg(SHELL.control)
                    .px(px(Space::SM))
                    .py(px(Space::XS))
                    .rounded(px(Radius::SM))
                    .child(status_badge),
            )
            .into_any_element(),
        div()
            .flex()
            .flex_row()
            .gap(px(Space::MD))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .bg(SHELL.panel)
                    .rounded(px(Radius::SM))
                    .p(px(Space::SM))
                    .child(
                        div()
                            .font(Fonts::sans())
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child("Link delay"),
                    )
                    .child(
                        div()
                            .font(numeric_font(Fonts::MONO))
                            .text_size(px(Type::TWO_SM))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(Accent::DEFAULT)
                            .child(latency_val),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .bg(SHELL.panel)
                    .rounded(px(Radius::SM))
                    .p(px(Space::SM))
                    .child(
                        div()
                            .font(Fonts::sans())
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child("Frame interval"),
                    )
                    .child(
                        div()
                            .font(numeric_font(Fonts::MONO))
                            .text_size(px(Type::TWO_SM))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(Text::PRIMARY)
                            .child(sensor_cadence_str),
                    ),
            )
            .into_any_element(),
    ]);

    children.push(section("Link & smoothness", vec![responsiveness_well]));

    let zoom_element = if zoom_supported {
        let min_zoom = selected_cam
            .map(|c| if c.min_zoom > 0.0 { c.min_zoom } else { 1.0 })
            .unwrap_or(1.0);
        let max_zoom = selected_cam
            .map(|c| {
                if c.max_zoom > min_zoom {
                    c.max_zoom
                } else {
                    min_zoom + 1.0
                }
            })
            .unwrap_or(8.0);
        let live_zoom = phone
            .map(|p| p.zoom)
            .filter(|z| *z > 0.0)
            .unwrap_or(1.0)
            .clamp(min_zoom, max_zoom);
        let zoom_presets = zoom_presets_for(min_zoom, max_zoom);

        let zoom_control = div()
            .flex()
            .flex_col()
            .gap(px(Space::SM))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_baseline()
                    .justify_between()
                    .child(
                        div()
                            .font(numeric_font(Fonts::MONO))
                            .text_size(px(Type::TITLE))
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(Accent::DEFAULT)
                            .child(format!("{live_zoom:.1}x")),
                    )
                    .child(
                        div()
                            .font(numeric_font(Fonts::SANS))
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child(format!("{min_zoom:.1}x to {max_zoom:.1}x")),
                    ),
            )
            .child(
                Slider::new("slider-zoom", live_zoom, min_zoom, max_zoom)
                    .step(0.1)
                    .focus_with(focus.as_deref_mut(), "slider-zoom")
                    .on_change(|z, _window, _cx| push_effect(SideEffect::SetZoom(z))),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::XS))
                    .children(zoom_presets.into_iter().map(|z| {
                        let active = (live_zoom - z).abs() < 0.05;
                        let label = format!("{z:.1}x");
                        pill_button_compact(
                            format!("zoom-{z:.1}"),
                            label,
                            active,
                            false,
                            move |_window, _cx| push_effect(SideEffect::SetZoom(z)),
                        )
                    })),
            )
            .into_any_element();

        crate::views::widgets::field(
            "Zoom",
            None,
            zoom_control,
            Some("Optical or digital zoom within the selected lens's supported range.".into()),
        )
    } else {
        crate::views::widgets::unavailable_field("Zoom", "This camera does not support zoom.")
    };

    let torch_element = if torch_supported {
        Toggle::new("Flashlight / Torch", torch_on)
            .id("capture-torch")
            .focus_with(focus.as_deref_mut(), "capture-torch")
            .detail(Some(if torch_on { "Rear LED on" } else { "Off" }))
            .on_change(move |enabled, _window, _cx| push_effect(SideEffect::SetTorch(enabled)))
            .into_element()
    } else {
        crate::views::widgets::unavailable_field(
            "Torch",
            "This camera has no torch. Front cameras and many external cameras do not.",
        )
    };

    let current_flicker = state.settings.anti_flicker;
    let anti_flicker_hint = match current_flicker {
        phonecam_protocol::AntiFlickerMode::Auto => "Auto (Dynamic detection)",
        phonecam_protocol::AntiFlickerMode::Hz50 => "50 Hz (EU, Asia, UK, AU)",
        phonecam_protocol::AntiFlickerMode::Hz60 => "60 Hz (US, Americas, JP)",
        phonecam_protocol::AntiFlickerMode::Off => "Off (No filter)",
    };

    let anti_flicker_element = crate::views::widgets::field(
        "Anti-flicker",
        Some(anti_flicker_hint.into()),
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(Space::SM))
            .children(
                [
                    (
                        phonecam_protocol::AntiFlickerMode::Auto,
                        "Auto",
                        "flicker-auto",
                    ),
                    (
                        phonecam_protocol::AntiFlickerMode::Hz50,
                        "50 Hz",
                        "flicker-50hz",
                    ),
                    (
                        phonecam_protocol::AntiFlickerMode::Hz60,
                        "60 Hz",
                        "flicker-60hz",
                    ),
                    (
                        phonecam_protocol::AntiFlickerMode::Off,
                        "Off",
                        "flicker-off",
                    ),
                ]
                .into_iter()
                .map(|(mode, label, id)| {
                    let active = current_flicker == mode;
                    pill_button(id, label, active, false, move |_window, _cx| {
                        push_effect(SideEffect::SetAntiFlicker(mode))
                    })
                }),
            )
            .into_any_element(),
        Some("Reduces rolling bands from indoor AC lights and monitors.".into()),
    );

    let exp_val = state.settings.exposure;
    let exposure_element = crate::views::widgets::field(
        "Exposure compensation",
        Some(format!("{exp_val:+} EV").into()),
        div()
            .flex()
            .flex_col()
            .gap(px(Space::SM))
            .child(
                Slider::new("slider-exposure", exp_val as f32, -2.0, 2.0)
                    .step(1.0)
                    .focus_with(focus.as_deref_mut(), "slider-exposure")
                    .on_change(|v, _window, _cx| {
                        push_effect(SideEffect::SetExposure(v.round() as i32))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::XS))
                    .children([-2, -1, 0, 1, 2].into_iter().map(|step| {
                        let label = if step == 0 {
                            "0 EV (Auto)".to_string()
                        } else {
                            format!("{step:+} EV")
                        };
                        pill_button_compact(
                            format!("exp-{step}"),
                            label,
                            exp_val == step,
                            false,
                            move |_window, _cx| push_effect(SideEffect::SetExposure(step)),
                        )
                    })),
            )
            .into_any_element(),
        Some("Brighten dark rooms or prevent blown-out window highlights.".into()),
    );

    let current_wb = state.settings.white_balance_kelvin;
    let wb_slider_val = current_wb.unwrap_or(5500) as f32;
    let wb_element = crate::views::widgets::field(
        "White balance",
        Some(
            current_wb
                .map(|k| format!("{k}K"))
                .unwrap_or_else(|| "Auto".to_string())
                .into(),
        ),
        div()
            .flex()
            .flex_col()
            .gap(px(Space::SM))
            .child(
                Slider::new("slider-wb", wb_slider_val, 2800.0, 7500.0)
                    .step(100.0)
                    .focus_with(focus.as_deref_mut(), "slider-wb")
                    .on_change(|k, _window, _cx| {
                        let kelvin = ((k / 100.0).round() as u32) * 100;
                        push_effect(SideEffect::SetWhiteBalance(Some(kelvin.clamp(2800, 7500))))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::XS))
                    .child(pill_button_compact(
                        "wb-auto",
                        "Auto AWB",
                        current_wb.is_none(),
                        false,
                        |_window, _cx| push_effect(SideEffect::SetWhiteBalance(None)),
                    ))
                    .child(pill_button_compact(
                        "wb-3200",
                        "3200K",
                        current_wb == Some(3200),
                        false,
                        |_window, _cx| push_effect(SideEffect::SetWhiteBalance(Some(3200))),
                    ))
                    .child(pill_button_compact(
                        "wb-4000",
                        "4000K",
                        current_wb == Some(4000),
                        false,
                        |_window, _cx| push_effect(SideEffect::SetWhiteBalance(Some(4000))),
                    ))
                    .child(pill_button_compact(
                        "wb-5500",
                        "5500K",
                        current_wb == Some(5500),
                        false,
                        |_window, _cx| push_effect(SideEffect::SetWhiteBalance(Some(5500))),
                    ))
                    .child(pill_button_compact(
                        "wb-6500",
                        "6500K",
                        current_wb == Some(6500),
                        false,
                        |_window, _cx| push_effect(SideEffect::SetWhiteBalance(Some(6500))),
                    )),
            )
            .into_any_element(),
        Some(
            "Lock color temperature to match room lighting and keep skin tones consistent.".into(),
        ),
    );

    let current_focus = state.settings.focus_distance;
    let focus_slider_val = current_focus.unwrap_or(0.0);
    let focus_element = crate::views::widgets::field(
        "Focus distance",
        Some(
            match current_focus {
                None => "Auto AF".to_string(),
                Some(d) => format!("{d:.1} D"),
            }
            .into(),
        ),
        div()
            .flex()
            .flex_col()
            .gap(px(Space::SM))
            .child(
                Slider::new("slider-focus", focus_slider_val, 0.0, 10.0)
                    .step(0.5)
                    .focus_with(focus.as_deref_mut(), "slider-focus")
                    .on_change(|d, _window, _cx| {
                        push_effect(SideEffect::SetFocus(Some(d.clamp(0.0, 10.0))))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::XS))
                    .child(pill_button_compact(
                        "focus-auto",
                        "Continuous AF",
                        current_focus.is_none(),
                        false,
                        |_window, _cx| push_effect(SideEffect::SetFocus(None)),
                    ))
                    .child(pill_button_compact(
                        "focus-inf",
                        "Infinity",
                        current_focus.map(|d| d.abs() < 0.1).unwrap_or(false),
                        false,
                        |_window, _cx| push_effect(SideEffect::SetFocus(Some(0.0))),
                    ))
                    .child(pill_button_compact(
                        "focus-mid",
                        "Desk (0.5m)",
                        current_focus
                            .map(|d| (d - 2.0).abs() < 0.5)
                            .unwrap_or(false),
                        false,
                        |_window, _cx| push_effect(SideEffect::SetFocus(Some(2.0))),
                    ))
                    .child(pill_button_compact(
                        "focus-macro",
                        "Macro (0.1m)",
                        current_focus
                            .map(|d| (d - 10.0).abs() < 0.5)
                            .unwrap_or(false),
                        false,
                        |_window, _cx| push_effect(SideEffect::SetFocus(Some(10.0))),
                    )),
            )
            .into_any_element(),
        Some("Lock focal distance to stop autofocus hunting and breathing.".into()),
    );

    children.push(section(
        "Exposure & Focus",
        vec![
            zoom_element,
            torch_element,
            anti_flicker_element,
            exposure_element,
            wb_element,
            focus_element,
        ],
    ));

    let auto = state.settings.quality_mode == phonecam_protocol::QualityMode::Auto;
    let ambient_hint = match state.settings.screen_mode {
        phonecam_protocol::ScreenMode::KeepOn => "Keep On (Normal display)",
        phonecam_protocol::ScreenMode::Dim => "Dim (Reduced brightness)",
        phonecam_protocol::ScreenMode::Black => "Pure Black (100% OLED off)",
    };

    children.push(section(
        "While streaming",
        vec![
            Toggle::new("Adjust quality automatically", auto)
                .id("capture-auto-quality")
                .focus_with(focus.as_deref_mut(), "capture-auto-quality")
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
                "OLED Ambient Display",
                Some(ambient_hint.into()),
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(Space::SM))
                    .children(
                        [
                            (phonecam_protocol::ScreenMode::KeepOn, "Keep On", "screen-keep-on"),
                            (phonecam_protocol::ScreenMode::Dim, "Dim", "screen-dim"),
                            (phonecam_protocol::ScreenMode::Black, "Pure Black", "screen-black"),
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
                Some("OLED Pure-Black ambient mode turns off display pixels to save battery and prevent burn-in while streaming.".into()),
            ),
        ],
    ));

    // --- Single Collapsible Experimental (Labs) Card (collapsed by default) ---
    let active_labs_count = [
        state.settings.auto_framing,
        state.settings.hand_gestures,
        state.settings.portrait_blur,
        state.settings.lut_preset != phonecam_media::LutPreset::None,
        state.settings.stream_audio,
    ]
    .into_iter()
    .filter(|&x| x)
    .count();

    let labs_badge = if active_labs_count == 0 {
        chip("Off", Tone::Neutral)
    } else {
        chip(format!("{active_labs_count} active"), Tone::Accent)
    };

    let toggle_labs_cb = callbacks.on_toggle_labs.clone();
    let labs_header = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(Space::SM))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(Space::SM))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_SM))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::HEADING)
                        .child("Experimental (Labs)"),
                )
                .child(labs_badge),
        )
        .child(
            Button::ghost(if ui.labs_expanded {
                "Collapse"
            } else {
                "Expand"
            })
            .id("labs-expand-toggle")
            .focus_with(focus.as_deref_mut(), "labs-expand-toggle")
            .on_click(move |window, cx| {
                if let Some(ref cb) = toggle_labs_cb {
                    cb(window, cx);
                }
            }),
        );

    let mut labs_children: Vec<AnyElement> = vec![labs_header.into_any_element()];

    if ui.labs_expanded {
        let current_lut = state.settings.lut_preset;
        let lut_selector = crate::views::widgets::field(
            "Color LUT Filter (Preview Only)",
            Some(current_lut.display_name().into()),
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(Space::SM))
                .children(phonecam_media::LutPreset::all().iter().map(|&preset| {
                    let active = current_lut == preset;
                    pill_button(
                        format!("lut-{}", preset.display_name()),
                        preset.display_name(),
                        active,
                        false,
                        move |_window, _cx| push_effect(SideEffect::SetLutFilter(preset)),
                    )
                }))
                .into_any_element(),
            Some(
                "Local preview color grading presets. Does not alter the Virtual Camera stream."
                    .into(),
            ),
        );

        let blur_enabled = state.settings.portrait_blur;
        let blur_intensity = state.settings.portrait_blur_intensity.clamp(0.1, 1.0);
        let blur_pct = (blur_intensity * 100.0).round() as u32;
        let blur_element = crate::views::widgets::field(
            "Portrait Bokeh (Preview Only)",
            Some(
                if blur_enabled {
                    format!("{blur_pct}%")
                } else {
                    "Off".to_string()
                }
                .into(),
            ),
            div()
                .flex()
                .flex_col()
                .gap(px(Space::SM))
                .child(
                    Toggle::new("Enable Portrait Bokeh Blur", blur_enabled)
                        .id("labs-portrait-blur")
                        .focus_with(focus.as_deref_mut(), "labs-portrait-blur")
                        .detail(Some(if blur_enabled { "Active" } else { "Off" }))
                        .on_change(move |enabled, _window, _cx| {
                            push_effect(SideEffect::SetPortraitBlur(enabled, blur_intensity))
                        })
                        .into_element(),
                )
                .child(
                    Slider::new("slider-bokeh", blur_intensity, 0.10, 1.00)
                        .step(0.05)
                        .disabled(!blur_enabled)
                        .focus_with(focus.as_deref_mut(), "slider-bokeh")
                        .on_change(|amt, _window, _cx| {
                            push_effect(SideEffect::SetPortraitBlur(true, amt))
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap(px(Space::XS))
                        .child(pill_button_compact(
                            "blur-soft",
                            "Soft (30%)",
                            blur_enabled && (blur_intensity - 0.30).abs() < 0.12,
                            false,
                            move |_window, _cx| {
                                push_effect(SideEffect::SetPortraitBlur(true, 0.30))
                            },
                        ))
                        .child(pill_button_compact(
                            "blur-studio",
                            "Studio (70%)",
                            blur_enabled && (blur_intensity - 0.70).abs() < 0.12,
                            false,
                            move |_window, _cx| {
                                push_effect(SideEffect::SetPortraitBlur(true, 0.70))
                            },
                        ))
                        .child(pill_button_compact(
                            "blur-dreamy",
                            "Dreamy (100%)",
                            blur_enabled && (blur_intensity - 1.00).abs() < 0.12,
                            false,
                            move |_window, _cx| {
                                push_effect(SideEffect::SetPortraitBlur(true, 1.00))
                            },
                        )),
                )
                .into_any_element(),
            Some(
                "Applies background blur to the preview surface only (not sent to Virtual Camera)."
                    .into(),
            ),
        );

        let auto_framing_enabled = state.settings.auto_framing;
        let auto_framing_element = crate::views::widgets::field(
            "Auto-Framing (Preview Only)",
            Some(
                if auto_framing_enabled {
                    if state.tracking_locked {
                        "Tracking locked"
                    } else {
                        "Active"
                    }
                } else {
                    "Off"
                }
                .into(),
            ),
            Toggle::new("Face Tracking Auto-Framing", auto_framing_enabled)
                .id("labs-auto-framing")
                .focus_with(focus.as_deref_mut(), "labs-auto-framing")
                .detail(Some(if auto_framing_enabled {
                    if state.tracking_locked {
                        "Locked (Open Palm to unlock)"
                    } else {
                        "PTZ face tracking active"
                    }
                } else {
                    "Off"
                }))
                .on_change(|enabled, _window, _cx| {
                    push_effect(SideEffect::SetAutoFraming(enabled))
                })
                .into_element(),
            Some("Smooth pan and zoom tracking in the preview window. Does not crop the Virtual Camera output.".into()),
        );

        let audio_stream_enabled = state.settings.stream_audio;
        let audio_level = state.audio_level.clamp(0.0, 1.0);
        let selected_audio_out = state
            .settings
            .audio_output_device
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "monitor_only");
        let audio_out_label: SharedString = match selected_audio_out {
            None => "Monitor only".into(),
            Some("default") => "Windows default".into(),
            Some(dev) => dev.to_string().into(),
        };

        let mut audio_content = div().flex().flex_col().gap(px(Space::SM)).child(
            Toggle::new("Audio Monitor & Output", audio_stream_enabled)
                .id("labs-audio-monitor")
                .focus_with(focus.as_deref_mut(), "labs-audio-monitor")
                .detail(Some(if audio_stream_enabled {
                    if selected_audio_out.is_some() {
                        "Streaming & Playing"
                    } else {
                        "Monitoring (VU only)"
                    }
                } else {
                    "Off"
                }))
                .on_change(|enabled, _window, _cx| push_effect(SideEffect::SetAudioStream(enabled)))
                .into_element(),
        );
        if audio_stream_enabled {
            audio_content = audio_content.child(audio_vu_row(audio_level, true, "Input level"));
        }

        let mut device_rows = div().flex().flex_col().gap(px(Space::XS)).child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(Space::XS))
                .child(pill_button_compact(
                    "audio-out-monitor-only",
                    "Monitor only (VU meter)",
                    selected_audio_out.is_none(),
                    false,
                    move |_window, _cx| push_effect(SideEffect::SetAudioOutputDevice(None)),
                ))
                .child(pill_button_compact(
                    "audio-out-default",
                    "Windows default",
                    selected_audio_out == Some("default"),
                    false,
                    move |_window, _cx| {
                        push_effect(SideEffect::SetAudioOutputDevice(Some("default".into())))
                    },
                )),
        );

        for (idx, dev_name) in state.available_audio_devices.iter().enumerate() {
            let is_selected = selected_audio_out == Some(dev_name.as_str());
            let lower = dev_name.to_ascii_lowercase();
            let is_vbcable = lower.contains("cable input")
                || lower.contains("vb-audio")
                || lower.contains("virtual audio")
                || lower.contains("voicemeeter");
            let dev_clone = dev_name.clone();
            let label = if is_vbcable {
                format!("{dev_name} [Virtual Cable]")
            } else {
                dev_name.clone()
            };
            device_rows = device_rows.child(
                div()
                    .id(gpui::ElementId::Name(SharedString::from(format!(
                        "audio-out-dev-{idx}"
                    ))))
                    .key_context("Control")
                    .w_full()
                    .min_w(px(0.0))
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .gap(px(Space::SM))
                    .rounded(px(Radius::MD))
                    .border_1()
                    .border_color(if is_selected {
                        Accent::DEFAULT
                    } else {
                        SHELL.border
                    })
                    .bg(SHELL.control)
                    .px(px(Space::MD))
                    .py(px(Space::XS))
                    .cursor_pointer()
                    .hover(|s| s.border_color(Accent::HOVER).bg(SHELL.hover))
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .font(Fonts::sans())
                            .text_size(px(Type::XS))
                            .font_weight(if is_selected {
                                gpui::FontWeight::SEMIBOLD
                            } else {
                                gpui::FontWeight::MEDIUM
                            })
                            .text_color(if is_selected {
                                Accent::DEFAULT
                            } else {
                                Text::PRIMARY
                            })
                            .child(label),
                    )
                    .on_click(move |_event, _window, _cx| {
                        push_effect(SideEffect::SetAudioOutputDevice(Some(dev_clone.clone())))
                    }),
            );
        }

        audio_content = audio_content.child(device_rows);

        let audio_element = crate::views::widgets::field(
            "Audio Monitor & Output",
            Some(if audio_stream_enabled {
                audio_out_label
            } else {
                "Off".into()
            }),
            audio_content.into_any_element(),
            Some("Routes phone audio to VB-Audio Virtual Cable or a Windows playback device, or stays Monitor only to prevent speaker feedback. Native kernel virtual mic driver is still Roadmap.".into()),
        );

        let hand_gestures_element = hand_gesture_guide_card_with_ui(
            state,
            ui.gesture_ref_expanded,
            callbacks.on_toggle_gesture_ref.clone(),
            focus,
        );

        labs_children.push(prose(
            "Experimental features: visual effects run locally on the preview stage; Audio Monitor & Output can optionally play to a Windows playback device or VB-Audio Virtual Cable.",
        ));
        labs_children.push(auto_framing_element);
        labs_children.push(hand_gestures_element);
        labs_children.push(lut_selector);
        labs_children.push(blur_element);
        labs_children.push(audio_element);
    } else {
        labs_children.push(prose(
            "Auto-Framing, Hand Gestures, Portrait Bokeh, Color LUTs, and Audio Monitor & Output.",
        ));
    }

    children.push(section("Labs", vec![well(labs_children)]));

    div()
        .flex()
        .flex_col()
        .pb(px(Space::RAIL_BOTTOM))
        .child(panel.children(children))
        .into_any_element()
}

fn camera_tiles(cameras: &[phonecam_protocol::CameraCapability], selected_id: &str) -> AnyElement {
    let fallback_cams = [
        phonecam_protocol::CameraCapability {
            id: "0".to_string(),
            facing: phonecam_protocol::CameraFacing::Back,
            resolutions: vec!["1920x1080".into(), "1280x720".into()],
            fps: vec![30, 24],
            torch: true,
            focus: true,
            zoom: true,
            exposure: true,
            white_balance: true,
            min_zoom: 1.0,
            max_zoom: 8.0,
            orientation_degrees: 90,
        },
        phonecam_protocol::CameraCapability {
            id: "1".to_string(),
            facing: phonecam_protocol::CameraFacing::Front,
            resolutions: vec!["1920x1080".into(), "1280x720".into()],
            fps: vec![30],
            torch: false,
            focus: true,
            zoom: true,
            exposure: true,
            white_balance: true,
            min_zoom: 1.0,
            max_zoom: 4.0,
            orientation_degrees: 270,
        },
    ];
    let list: &[phonecam_protocol::CameraCapability] = if cameras.is_empty() {
        &fallback_cams
    } else {
        cameras
    };

    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap(px(Space::SM))
        .children(list.iter().map(|camera| {
            let is_selected = camera.id == selected_id;
            let id = camera.id.clone();
            let label = match camera.facing {
                phonecam_protocol::CameraFacing::Back => "Back camera",
                phonecam_protocol::CameraFacing::Front => "Front camera",
                phonecam_protocol::CameraFacing::External => "External camera",
                phonecam_protocol::CameraFacing::Unknown => camera_facing_label(camera.facing),
            };
            let sub = if camera.zoom && camera.max_zoom > camera.min_zoom {
                format!(
                    "Lens {} · {:.1}x-{:.1}x",
                    camera.id, camera.min_zoom, camera.max_zoom
                )
            } else {
                format!("Lens ID {}", camera.id)
            };

            div()
                .id(gpui::ElementId::Name(SharedString::from(format!(
                    "camera-{id}"
                ))))
                .key_context("Control")
                .flex_1()
                .min_w(px(120.0))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(Space::SM))
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(if is_selected {
                    Accent::DEFAULT
                } else {
                    SHELL.control_border
                })
                .bg(if is_selected {
                    Accent::DEFAULT.alpha(0.14)
                } else {
                    SHELL.control
                })
                .px(px(12.0))
                .py(px(10.0))
                .cursor_pointer()
                .hover(|s| s.border_color(SHELL.control_border_hover).bg(SHELL.hover))
                .focus(|s| s.border_2().border_color(Accent::DEFAULT))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .truncate()
                                .font(Fonts::sans())
                                .text_size(px(Type::TWO_SM))
                                .font_weight(if is_selected {
                                    gpui::FontWeight::SEMIBOLD
                                } else {
                                    gpui::FontWeight::MEDIUM
                                })
                                .text_color(if is_selected {
                                    Text::PRIMARY
                                } else {
                                    Text::BODY
                                })
                                .child(label),
                        )
                        .child(
                            div()
                                .truncate()
                                .font(numeric_font(Fonts::MONO))
                                .text_size(px(Type::TWO_XS))
                                .text_color(Text::MUTED)
                                .child(sub),
                        ),
                )
                .child(if is_selected {
                    div()
                        .flex_shrink_0()
                        .rounded(px(Radius::FULL))
                        .bg(Accent::DEFAULT)
                        .px(px(6.0))
                        .py(px(2.0))
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_XS))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(SHELL.base)
                        .child("Active")
                } else {
                    div()
                        .flex_shrink_0()
                        .rounded(px(Radius::FULL))
                        .border_1()
                        .border_color(SHELL.border)
                        .bg(SHELL.base)
                        .px(px(6.0))
                        .py(px(2.0))
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .child("Select")
                })
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
    let vcam_error = state
        .core
        .as_ref()
        .and_then(|c| c.vcam.as_ref())
        .and_then(|v| v.error.clone());
    let state_label = state.vcam_state().unwrap_or("stopped");
    let running = state_label == "running";
    let starting = state_label == "starting";
    let streaming = state.is_streaming();

    let panel = Panel::new("Virtual Camera").action(chip(
        match state_label {
            "starting" => "Starting",
            "running" if streaming => "Live",
            "running" => "Registered",
            "error" => "Failed",
            _ => "Off",
        },
        if running && streaming {
            Tone::Ok
        } else if running {
            Tone::Accent
        } else if state_label == "error" {
            Tone::Danger
        } else {
            Tone::Neutral
        },
    ));

    let mut children: Vec<AnyElement> = Vec::new();

    children.push(
        Toggle::new("PhoneCam Camera", running || starting)
            .id("publish-vcam-toggle")
            .detail(rules::vcam_usage_detail(
                state.vcam_state(),
                streaming,
                state.vcam_consumers(),
            ))
            .on_change(|enabled, _window, _cx| push_effect(SideEffect::SetVcam(enabled)))
            .into_element(),
    );

    children.push(match vcam_error {
        Some(error) => tinted_box(
            Tone::Danger,
            vec![div()
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .line_height(px(Type::LINE_XS))
                .text_color(Status::DANGER_TEXT)
                .child(error)
                .into_any_element()],
        ),
        None => prose(if running && streaming {
            "Select 'PhoneCam Camera' in OBS, Zoom, Meet, or Discord. It is carrying your live phone video."
        } else if running {
            "Registered and ready. Apps see a stand-in slate until you start streaming."
        } else {
            "Enable this to expose your phone as a system webcam in other apps."
        }),
    });

    // 1-click registration / troubleshoot bar
    children.push(
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .gap(px(10.0))
            .child(
                Button::new(if state.vcam_registered() {
                    "Restart Virtual Camera"
                } else {
                    "Register Virtual Camera"
                })
                .id("publish-restart-vcam")
                .on_click(|_window, _cx| push_effect(SideEffect::RestartVcam)),
            )
            .into_any_element(),
    );

    let sideways = state.settings.output_rotation == 90 || state.settings.output_rotation == 270;

    let output_summary: SharedString = if state.settings.output_follows_stream {
        match state.frame_size {
            Some((w, h)) => format!("Follows phone ({w} x {h})").into(),
            None => "Follows phone".into(),
        }
    } else {
        format!(
            "{} x {}",
            state.settings.output_width, state.settings.output_height
        )
        .into()
    };

    let framing_summary = format!(
        "{} ({}){}",
        rotation_label(state.settings.output_rotation),
        if state.settings.output_mirror {
            "Mirrored"
        } else {
            "Normal"
        },
        if state.settings.auto_rotate {
            ", Auto-rotate on"
        } else {
            ""
        }
    );

    children.push(section(
        "Output format",
        vec![
            Toggle::new("Follow phone stream", state.settings.output_follows_stream)
                .id("publish-follow-stream")
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
            key_value("Orientation & mirror", framing_summary, false),
            prose("Adjust Mirror, Rotation, and Auto-rotate from Quick framing in the Capture tab."),
            sideways
                .then(|| {
                    div()
                        .mt(px(Space::SM))
                        .mb(px(Space::FIELD_GAP))
                        .rounded(px(Radius::MD))
                        .border_1()
                        .border_color(SHELL.border.alpha(0.45))
                        .bg(SHELL.raised.alpha(0.6))
                        .px(px(Space::LG))
                        .py(px(Space::MD))
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .line_height(px(Type::LINE_XS))
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
        90 => "90 deg clockwise",
        180 => "180 deg",
        270 => "270 deg clockwise",
        _ => "Upright",
    }
}

/// The connection's own numbers. Informational, never a control.
pub fn connection_panel(state: &AppState) -> AnyElement {
    let transport = state
        .core
        .as_ref()
        .map(|c| c.transport.as_str())
        .unwrap_or("none");
    let forwarding = state
        .core
        .as_ref()
        .map(|c| c.usb.forwarding)
        .unwrap_or(false);
    let streaming = state.is_streaming();

    let link_label = match transport {
        "usb" => "USB cable",
        "lan" => "Wi-Fi",
        _ if forwarding => "Cable ready, waiting for the phone",
        _ => "Not connected",
    };
    let link_tone = if transport != "none" && streaming {
        Some(Tone::Ok)
    } else if transport != "none" {
        Some(Tone::Accent)
    } else if forwarding {
        Some(Tone::Warn)
    } else {
        None
    };

    let usb_chip = if forwarding {
        chip(
            "Forwarding",
            if transport == "usb" && streaming {
                Tone::Ok
            } else {
                Tone::Accent
            },
        )
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
        usb_row = usb_row.child(
            Button::danger("Stop forwarding")
                .id("conn-stop-forwarding")
                .on_click(|_window, _cx| push_effect(SideEffect::DisconnectUsb)),
        );
    }

    Panel::new("Connection")
        .child(key_value_toned("Phone link", link_label, link_tone))
        .child(key_value_toned(
            "Service socket",
            if state.status.connected {
                "Connected"
            } else {
                "Retrying"
            },
            Some(if state.status.connected {
                Tone::Accent
            } else {
                Tone::Warn
            }),
        ))
        .child(key_value(
            "Received",
            crate::views::state::format_bytes(state.status.bytes_in),
            true,
        ))
        .child(key_value(
            "Reconnects",
            state.status.connects.to_string(),
            true,
        ))
        .child(section("USB", vec![usb_row.into_any_element()]))
        .into_element()
}

/* ------------------------------------------------------------------ */
/* Diagnostics                                                         */
/* ------------------------------------------------------------------ */

/// The System / Diagnostics panel: 2x2 KPI tiles, Software Updates card, device/output
/// info cards, and filterable log viewer.
pub fn diagnostics_panel(state: &AppState) -> AnyElement {
    diagnostics_panel_with_ui(state, &PanelCallbacks::default(), None)
}

/// Interactive System panel wired with the Software Updates modal trigger and focus tracking.
pub fn diagnostics_panel_with_ui(
    state: &AppState,
    callbacks: &PanelCallbacks,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let (status_chip_label, status_tone) = if !state.status.connected {
        ("Error", Tone::Danger)
    } else if state.is_streaming() {
        ("Live", Tone::Ok)
    } else {
        ("Idle", Tone::Neutral)
    };

    let panel = Panel::new("System").action(chip(status_chip_label, status_tone));

    let mut children: Vec<AnyElement> = Vec::new();

    // 1. Live Throughput 2x2 KPI Tiles
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
        .gap(px(10.0))
        .child(metric_card_tile("FPS", fps_display))
        .child(metric_card_tile("Bitrate", readout.bitrate))
        .child(metric_card_tile("Latency (RTT)", readout.latency))
        .child(metric_card_tile("Dropped", readout.dropped))
        .into_any_element();
    children.push(throughput_grid);

    // 2. Software Updates Card
    children.push(software_updates_card(
        state,
        callbacks.on_open_updates.clone(),
        focus.as_deref_mut(),
    ));

    // 3. Action Toolbar: 2x2 grid with generous 10px spacing
    let toolbar = div()
        .grid()
        .grid_cols(2)
        .gap(px(10.0))
        .child(
            Button::new("Refresh")
                .id("diag-refresh")
                .focus_with(focus.as_deref_mut(), "diag-refresh")
                .full_width()
                .on_click(|_window, _cx| push_effect(SideEffect::RefreshState)),
        )
        .child(
            Button::new("Copy Report")
                .id("diag-copy-report")
                .focus_with(focus.as_deref_mut(), "diag-copy-report")
                .full_width()
                .on_click(|_window, _cx| push_effect(SideEffect::CopyDiagnosticsReport)),
        )
        .child(
            Button::new("Export")
                .id("diag-export")
                .focus_with(focus.as_deref_mut(), "diag-export")
                .full_width()
                .on_click(|_window, _cx| push_effect(SideEffect::ExportDiagnostics)),
        )
        .child(
            Button::new("Open Log File")
                .id("diag-open-log")
                .focus_with(focus, "diag-open-log")
                .full_width()
                .on_click(|_window, _cx| push_effect(SideEffect::OpenLogFile)),
        )
        .into_any_element();
    children.push(section("Diagnostics tools", vec![toolbar]));

    // 3. Structured System & Device Info Cards
    let device = state.core.as_ref().and_then(|c| c.device.as_ref());
    let facts = state.device_facts();
    let phone_card: AnyElement = match device {
        Some(device) => {
            let mut rows: Vec<(SharedString, AnyElement)> = Vec::new();
            rows.push(("Name".into(), card_text_val(device.name.clone(), false)));
            if let Some(model) = facts.model {
                rows.push((
                    "Model".into(),
                    chip(clean_device_model(&model), Tone::Neutral),
                ));
            }
            rows.push((
                "Device ID".into(),
                card_text_val(device.device_id.clone(), true),
            ));
            if let Some(version) = facts.android_version {
                rows.push(("Android".into(), card_text_val(version.to_string(), false)));
            }

            let phone_info = state.core.as_ref().and_then(|c| c.phone.as_ref());
            let (batt_label, batt_tone) =
                match phone_info.and_then(|p| format_battery(p.battery, p.charging)) {
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
        None => info_card(vec![div()
            .py(px(Space::SM))
            .child(prose(
                "Nothing connected. Pair a phone from Setup and its details appear here.",
            ))
            .into_any_element()]),
    };
    children.push(section("Connected phone", vec![phone_card]));

    // Card B: "Output Stream"
    let published_output = if state.settings.output_follows_stream {
        match state.frame_size {
            Some((w, h)) => format!("{w}x{h}"),
            None => format!(
                "{}x{}",
                state.settings.output_width, state.settings.output_height
            ),
        }
    } else {
        format!(
            "{}x{}",
            state.settings.output_width, state.settings.output_height
        )
    };
    let decoded_str = state
        .frame_size
        .map(|(w, h)| format!("{w}x{h}"))
        .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string());
    let phone_out_str = state
        .core
        .as_ref()
        .and_then(|c| c.phone.as_ref())
        .map(|p| p.actual_resolution.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::theme::EMPTY_VALUE.to_string());

    let output_rows = vec![
        (
            "Mirror".into(),
            card_text_val(
                if state.settings.output_mirror {
                    "On"
                } else {
                    "Off"
                },
                false,
            ),
        ),
        (
            "Rotation".into(),
            card_text_val(format!("{} deg", state.settings.output_rotation), false),
        ),
        (
            "Output resolution".into(),
            card_text_val(published_output, true),
        ),
        ("Decoded stream".into(), card_text_val(decoded_str, true)),
        (
            "Phone stream resolution".into(),
            card_text_val(phone_out_str, true),
        ),
    ];
    children.push(section(
        "Output stream",
        vec![info_card_from_rows(output_rows)],
    ));

    // Card: "Audio & Microphone Stream" (Technical details live here in Diagnostics)
    let diag_audio_level = state.audio_level.clamp(0.0, 1.0);
    let diag_audio_enabled = state.settings.stream_audio;
    let diag_audio_out = state
        .settings
        .audio_output_device
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "monitor_only");
    let (audio_status_text, audio_tone) = if !state.status.connected {
        ("Offline", Tone::Neutral)
    } else if diag_audio_enabled && diag_audio_out.is_some() {
        ("Output Active", Tone::Ok)
    } else if diag_audio_enabled {
        ("Monitor Active (VU Only)", Tone::Accent)
    } else {
        ("Off", Tone::Neutral)
    };

    let audio_route_text = match diag_audio_out {
        None => "Monitor only (VU meter)".to_string(),
        Some("default") => "Windows default playback".to_string(),
        Some(dev) => dev.to_string(),
    };
    let sample_rate_khz = state
        .core
        .as_ref()
        .and_then(|c| c.phone.as_ref())
        .and_then(|p| p.audio_sample_rate)
        .unwrap_or(48_000)
        / 1000;
    let audio_codec_text = format!("PCM 16-bit {sample_rate_khz} kHz Mono");

    let audio_level_widget = audio_vu_row(diag_audio_level, diag_audio_enabled, "Input dBFS");

    let audio_diag_rows = vec![
        ("Audio monitor".into(), chip(audio_status_text, audio_tone)),
        (
            "Audio output".into(),
            card_text_val(audio_route_text, false),
        ),
        ("Input level".into(), audio_level_widget),
        ("Audio codec".into(), card_text_val(audio_codec_text, true)),
        (
            "Transport channel".into(),
            card_text_val("TCP Role 3 (Low Latency)", true),
        ),
    ];
    children.push(section(
        "Audio & microphone stream",
        vec![info_card_from_rows(audio_diag_rows)],
    ));

    // Card C: "Network & Endpoints"
    let service_endpoint = format!("127.0.0.1:{}", phonecam_protocol::SERVICE_PORT);
    let wifi_endpoint = state
        .local_ip
        .as_deref()
        .map(|ip| format!("{ip}:{}", phonecam_protocol::SERVICE_PORT))
        .unwrap_or_else(|| "Unavailable".to_string());

    let socket_dot_el = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(Space::SM))
        .child(status_dot(if state.status.connected {
            Tone::Accent
        } else {
            Tone::Danger
        }))
        .child(
            div()
                .font(Fonts::sans())
                .text_size(px(Type::TWO_SM))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(if state.status.connected {
                    Accent::DEFAULT
                } else {
                    Status::DANGER_TEXT
                })
                .child(if state.status.connected {
                    "Connected"
                } else {
                    "Disconnected"
                }),
        )
        .into_any_element();

    let network_rows = vec![
        (
            "Service endpoint".into(),
            card_text_val(service_endpoint, true),
        ),
        ("Wi-Fi endpoint".into(), card_text_val(wifi_endpoint, true)),
        ("Socket status".into(), socket_dot_el),
        (
            "Log file".into(),
            card_text_val("%APPDATA%\\PhoneCam\\desktop.log", true),
        ),
        (
            "Platform".into(),
            card_text_val(
                if cfg!(windows) {
                    "win32 x64"
                } else {
                    "linux x64"
                },
                false,
            ),
        ),
    ];
    children.push(section(
        "Network & endpoints",
        vec![info_card_from_rows(network_rows)],
    ));

    // 4. Redesigned Log Viewer with real segmented pill selectors
    let matching_logs = filtered_logs(state);
    let log_body: Vec<AnyElement> = if state.logs.is_empty() {
        vec![div()
            .py(px(Space::MD))
            .font(Fonts::mono())
            .text_size(px(Type::XS))
            .line_height(px(Type::LINE_XS))
            .text_color(Text::MUTED)
            .child("Nothing logged yet. Lines appear as the service and the phone report in.")
            .into_any_element()]
    } else if matching_logs.is_empty() {
        vec![div()
            .py(px(Space::MD))
            .font(Fonts::mono())
            .text_size(px(Type::XS))
            .line_height(px(Type::LINE_XS))
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
                            .w(px(76.0))
                            .font(numeric_font(Fonts::MONO))
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child(format_log_time(line.ts_ms)),
                    )
                    .child(log_level_badge(line.level))
                    .child(
                        div()
                            .flex_shrink_0()
                            .w(px(56.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(Radius::SM))
                            .bg(Accent::DEFAULT.alpha(0.10))
                            .border_1()
                            .border_color(Accent::DEFAULT.alpha(0.20))
                            .py(px(1.0))
                            .font(Fonts::mono())
                            .text_size(px(Type::TWO_XS))
                            .text_color(Accent::DEFAULT)
                            .child(line.source.label()),
                    )
                    .child(
                        div()
                            .flex_grow()
                            .min_w(px(0.0))
                            .font(Fonts::mono())
                            .text_size(px(Type::XS))
                            .line_height(px(Type::LINE_XS))
                            .text_color(Text::PRIMARY)
                            .child(line.message.clone()),
                    )
                    .into_any_element()
            })
            .collect()
    };

    let log_line_count = if matching_logs.len() == state.logs.len() {
        format!("{} lines", matching_logs.len())
    } else {
        format!("{}/{} lines", matching_logs.len(), state.logs.len())
    };

    let level_pills = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(Space::XS))
        .children(
            [
                (None, "All", "log-lvl-all"),
                (
                    Some(phonecam_protocol::LogLevel::Error),
                    "Error",
                    "log-lvl-err",
                ),
                (
                    Some(phonecam_protocol::LogLevel::Warn),
                    "Warn",
                    "log-lvl-warn",
                ),
                (
                    Some(phonecam_protocol::LogLevel::Info),
                    "Info",
                    "log-lvl-info",
                ),
                (
                    Some(phonecam_protocol::LogLevel::Debug),
                    "Debug",
                    "log-lvl-dbg",
                ),
            ]
            .into_iter()
            .map(|(lvl, label, id)| {
                pill_button_compact(
                    id,
                    label,
                    state.log_filter_level == lvl,
                    false,
                    move |_window, _cx| push_effect(SideEffect::SetLogFilterLevel(lvl)),
                )
            }),
        );

    let source_pills = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(Space::XS))
        .children(
            [
                (None, "All", "log-src-all"),
                (Some(LogSource::Core), "Service", "log-src-core"),
                (Some(LogSource::App), "PhoneCam", "log-src-app"),
                (Some(LogSource::Renderer), "App", "log-src-ui"),
                (Some(LogSource::Media), "Media", "log-src-media"),
                (Some(LogSource::Phone), "Phone", "log-src-phone"),
            ]
            .into_iter()
            .map(|(src, label, id)| {
                pill_button_compact(
                    id,
                    label,
                    state.log_filter_source == src,
                    false,
                    move |_window, _cx| push_effect(SideEffect::SetLogFilterSource(src)),
                )
            }),
        );

    let filter_bar = div()
        .mb(px(Space::MD))
        .flex()
        .flex_col()
        .gap(px(Space::SM))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(Space::SM))
                .child(level_pills)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(Space::MD))
                        .child(
                            div()
                                .font(numeric_font(Fonts::MONO))
                                .text_size(px(Type::TWO_XS))
                                .text_color(Text::MUTED)
                                .child(log_line_count),
                        )
                        .child(
                            Button::new("Clear")
                                .id("diag-clear-logs")
                                .disabled(state.logs.is_empty())
                                .on_click(|_window, _cx| push_effect(SideEffect::ClearLogs)),
                        ),
                ),
        )
        .child(source_pills)
        .into_any_element();

    let terminal_well = div()
        .id(gpui::ElementId::Name(SharedString::from("log-well")))
        .flex()
        .flex_col()
        .gap(px(Space::XS))
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border.alpha(0.45))
        .bg(SHELL.recessed)
        .min_h(px(160.0))
        .max_h(px(320.0))
        .overflow_y_scroll()
        .p(px(Space::MD))
        .children(log_body)
        .into_any_element();

    let runtime_lvl_pills = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(Space::XS))
        .children(
            [
                (phonecam_protocol::LogLevel::Error, "Error", "rt-lvl-err"),
                (phonecam_protocol::LogLevel::Warn, "Warn", "rt-lvl-warn"),
                (phonecam_protocol::LogLevel::Info, "Info", "rt-lvl-info"),
                (phonecam_protocol::LogLevel::Debug, "Debug", "rt-lvl-dbg"),
                (phonecam_protocol::LogLevel::Trace, "Trace", "rt-lvl-trace"),
            ]
            .into_iter()
            .map(|(lvl, label, id)| {
                pill_button_compact(
                    id,
                    label,
                    state.settings.log_level == lvl,
                    false,
                    move |_window, _cx| push_effect(SideEffect::SetLogLevel(lvl)),
                )
            }),
        );

    let bottom_bar = div()
        .mt(px(Space::MD))
        .flex()
        .flex_col()
        .gap(px(Space::SM))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(Space::MD))
                .child(runtime_lvl_pills)
                .child(
                    Button::new("Open in Notepad")
                        .id("diag-open-notepad")
                        .on_click(|_window, _cx| push_effect(SideEffect::OpenLogFile)),
                ),
        )
        .child(
            div()
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .line_height(px(Type::LINE_XS))
                .text_color(Text::MUTED)
                .child("Pairing tokens, device secrets and video frames are never logged. Open desktop.log directly in Notepad."),
        )
        .into_any_element();

    children.push(section("Log", vec![filter_bar, terminal_well, bottom_bar]));

    panel.children(children).into_element()
}

/// Software Updates summary card displayed inside the System tab.
pub fn software_updates_card(
    state: &AppState,
    on_open_updates: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let (badge_label, badge_tone, detail_text, cta_label, primary_cta) = match &state.update_status
    {
        UpdateStatus::Idle => (
            format!("v{}", state.current_version),
            Tone::Neutral,
            "Checks GitHub Releases only when you click below.".to_string(),
            "Check for Updates",
            false,
        ),
        UpdateStatus::Checking => (
            "Checking...".to_string(),
            Tone::Accent,
            "Querying GitHub Releases for the latest version...".to_string(),
            "View Status",
            false,
        ),
        UpdateStatus::UpToDate { current } => (
            "Up to date".to_string(),
            Tone::Ok,
            format!("PhoneCam v{current} is the latest release."),
            "Check Again",
            false,
        ),
        UpdateStatus::Available(info) => (
            format!("{} available", info.tag),
            Tone::Accent,
            format!(
                "Newer release available (current v{}).",
                state.current_version
            ),
            "View Update & Changelog",
            true,
        ),
        UpdateStatus::Failed(err) => (
            "Check failed".to_string(),
            Tone::Danger,
            err.clone(),
            "Try Again",
            false,
        ),
    };

    let action_btn = if primary_cta {
        Button::primary(cta_label)
    } else {
        Button::new(cta_label)
    }
    .id("system-check-updates")
    .focus_with(focus, "system-check-updates")
    .on_click(move |window, cx| {
        if let Some(ref cb) = on_open_updates {
            cb(window, cx);
        } else {
            push_effect(SideEffect::CheckForUpdates);
        }
    });

    div()
        .w_full()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .gap(px(10.0))
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.recessed)
        .p(px(16.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(10.0))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_SM))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::PRIMARY)
                        .child("Software Updates"),
                )
                .child(chip(badge_label, badge_tone)),
        )
        .child(prose(detail_text))
        .child(
            div()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(10.0))
                .child(action_btn),
        )
        .into_any_element()
}

/// Sleek centered Software Update modal showing current vs latest GitHub version,
/// scrollable changelog notes, "Check Again", and "Download Update on GitHub".
pub fn updates_modal(
    state: &AppState,
    on_close: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let close_btn = Button::ghost("Close")
        .id("updates-modal-close")
        .focus_with(focus.as_deref_mut(), "updates-modal-close")
        .on_click(move |window, cx| {
            if let Some(ref cb) = on_close {
                cb(window, cx);
            }
        });

    let current_ver_str = format!("v{}", state.current_version);
    let (latest_ver_str, latest_tone, status_banner) = match &state.update_status {
        UpdateStatus::Idle => (
            crate::theme::EMPTY_VALUE.to_string(),
            Tone::Neutral,
            prose("Click Check for Updates to query GitHub Releases. PhoneCam never checks automatically in the background."),
        ),
        UpdateStatus::Checking => (
            "Checking...".to_string(),
            Tone::Accent,
            tinted_box(
                Tone::Accent,
                vec![div()
                    .font(Fonts::sans())
                    .text_size(px(Type::XS))
                    .line_height(px(Type::LINE_XS))
                    .text_color(Text::PRIMARY)
                    .child("Contacting GitHub Releases...")
                    .into_any_element()],
            ),
        ),
        UpdateStatus::UpToDate { current } => (
            format!("v{current}"),
            Tone::Ok,
            tinted_box(
                Tone::Ok,
                vec![div()
                    .font(Fonts::sans())
                    .text_size(px(Type::XS))
                    .line_height(px(Type::LINE_XS))
                    .text_color(Status::OK)
                    .child(format!("You are running the latest version (v{current})."))
                    .into_any_element()],
            ),
        ),
        UpdateStatus::Available(info) => (
            info.tag.clone(),
            Tone::Accent,
            tinted_box(
                Tone::Accent,
                vec![div()
                    .font(Fonts::sans())
                    .text_size(px(Type::XS))
                    .line_height(px(Type::LINE_XS))
                    .text_color(Text::PRIMARY)
                    .child(match info.published_at.as_deref() {
                        Some(date) if !date.is_empty() => {
                            format!("Release {} is ready to download (published {}).", info.tag, date)
                        }
                        _ => format!("Release {} is ready to download.", info.tag),
                    })
                    .into_any_element()],
            ),
        ),
        UpdateStatus::Failed(err) => (
            "Unavailable".to_string(),
            Tone::Danger,
            tinted_box(
                Tone::Danger,
                vec![div()
                    .font(Fonts::sans())
                    .text_size(px(Type::XS))
                    .line_height(px(Type::LINE_XS))
                    .text_color(Status::DANGER_TEXT)
                    .child(err.clone())
                    .into_any_element()],
            ),
        ),
    };

    let version_row = div()
        .grid()
        .grid_cols(2)
        .gap(px(10.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(SHELL.border)
                .bg(SHELL.recessed)
                .p(px(14.0))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .child("Installed Version"),
                )
                .child(
                    div()
                        .font(numeric_font(Fonts::MONO))
                        .text_size(px(Type::TITLE))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::PRIMARY)
                        .child(current_ver_str),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .rounded(px(Radius::MD))
                .border_1()
                .border_color(SHELL.border)
                .bg(SHELL.recessed)
                .p(px(14.0))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .child("Latest on GitHub"),
                )
                .child(
                    div()
                        .font(numeric_font(Fonts::MONO))
                        .text_size(px(Type::TITLE))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(if matches!(latest_tone, Tone::Neutral) {
                            Text::MUTED
                        } else {
                            latest_tone.fg()
                        })
                        .child(latest_ver_str),
                ),
        );

    let mut card = div()
        .w_full()
        .max_w(px(500.0))
        .flex()
        .flex_col()
        .gap(px(14.0))
        .rounded(px(Radius::LG))
        .border_1()
        .border_color(SHELL.control_border)
        .bg(SHELL.panel)
        .p(px(20.0))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(12.0))
                .child(
                    div()
                        .font(Fonts::display())
                        .text_size(px(Type::STATUS))
                        .text_color(Text::PRIMARY)
                        .child("Software Updates"),
                )
                .child(close_btn),
        )
        .child(version_row)
        .child(status_banner);

    if let UpdateStatus::Available(info) = &state.update_status {
        let changelog_lines: Vec<AnyElement> = if info.changelog.trim().is_empty() {
            vec![prose("No release notes provided for this tag.")]
        } else {
            info.changelog
                .lines()
                .map(|line| {
                    let trimmed = line.trim_end();
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .line_height(px(Type::LINE_XS))
                        .text_color(if trimmed.starts_with('#') {
                            Text::PRIMARY
                        } else {
                            Text::BODY
                        })
                        .font_weight(if trimmed.starts_with('#') {
                            gpui::FontWeight::SEMIBOLD
                        } else {
                            gpui::FontWeight::NORMAL
                        })
                        .child(if trimmed.is_empty() {
                            " ".to_string()
                        } else {
                            trimmed.to_string()
                        })
                        .into_any_element()
                })
                .collect()
        };

        card = card.child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.0))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::XS))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(Text::HEADING)
                        .child("Changelog"),
                )
                .child(
                    div()
                        .id("updates-changelog-scroll")
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .max_h(px(220.0))
                        .overflow_y_scroll()
                        .rounded(px(Radius::MD))
                        .border_1()
                        .border_color(SHELL.border)
                        .bg(SHELL.recessed)
                        .p(px(14.0))
                        .children(changelog_lines),
                ),
        );
    }

    let is_checking = matches!(state.update_status, UpdateStatus::Checking);
    let mut footer = div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .justify_end()
        .gap(px(10.0))
        .pt(px(4.0))
        .child(
            Button::new(if matches!(state.update_status, UpdateStatus::Idle) {
                "Check for Updates"
            } else {
                "Check Again"
            })
            .id("updates-check-again")
            .disabled(is_checking)
            .focus_with(focus.as_deref_mut(), "updates-check-again")
            .on_click(|_window, _cx| push_effect(SideEffect::CheckForUpdates)),
        );

    if let UpdateStatus::Available(info) = &state.update_status {
        let download_url = info.url.clone();
        footer = footer.child(
            Button::primary("Download Update on GitHub")
                .id("updates-download-github")
                .focus_with(focus, "updates-download-github")
                .on_click(move |_window, _cx| {
                    push_effect(SideEffect::OpenUpdateUrl(download_url.clone()))
                }),
        );
    }

    card = card.child(footer);

    div()
        .id("updates-modal-overlay")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.68))
        .p(px(24.0))
        .child(card)
        .into_any_element()
}

/// A modern metric card tile: rounded corners, dark recessed background, subtle border,
/// small label, and large prominent bold numeric value with unit.
fn metric_card_tile(label: &'static str, value: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .justify_between()
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border)
        .bg(SHELL.recessed)
        .p(px(14.0))
        .gap(px(4.0))
        .child(
            div()
                .font(Fonts::sans())
                .text_size(px(Type::TWO_XS))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(Text::MUTED)
                .child(label),
        )
        .child(
            div()
                .truncate()
                .font(numeric_font(Fonts::DISPLAY))
                .text_size(px(Type::STATUS))
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
        .border_color(SHELL.border)
        .bg(SHELL.recessed)
        .px(px(16.0))
        .py(px(6.0))
        .children(rows)
        .into_any_element()
}

/// A key-value row inside a structured card, with subtle row divider if not last.
fn card_kv_row(label: impl Into<SharedString>, value_el: AnyElement, is_last: bool) -> AnyElement {
    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(Space::PAD))
        .py(px(Space::SM))
        .child(
            div()
                .flex_shrink_0()
                .font(Fonts::sans())
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
        .flex_shrink()
        .min_w(px(0.0))
        .truncate()
        .text_right()
        .font(numeric_font(if mono { Fonts::MONO } else { Fonts::SANS }))
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
        phonecam_protocol::LogLevel::Error => {
            (Status::DANGER.alpha(0.15), Status::DANGER.alpha(0.4))
        }
        phonecam_protocol::LogLevel::Warn => (Status::WARN.alpha(0.15), Status::WARN.alpha(0.4)),
        phonecam_protocol::LogLevel::Info => {
            (Accent::DEFAULT.alpha(0.12), Accent::DEFAULT.alpha(0.3))
        }
        phonecam_protocol::LogLevel::Debug => (SHELL.border.alpha(0.2), SHELL.border.alpha(0.4)),
        phonecam_protocol::LogLevel::Trace => (SHELL.border.alpha(0.1), SHELL.border.alpha(0.3)),
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
        .font(Fonts::mono())
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

/// Preferences panel (clean desktop preferences without duplicated Labs toggles).
pub fn preferences_panel(state: &AppState) -> AnyElement {
    preferences_panel_with_focus(state, None, None)
}

/// Preferences overlay drawer triggered from the top-bar gear button.
pub fn preferences_drawer(
    state: &AppState,
    on_close: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    div()
        .id("preferences-overlay")
        .absolute()
        .inset_0()
        .flex()
        .flex_row()
        .justify_end()
        .bg(gpui::hsla(0.0, 0.0, 0.0, 0.55))
        .p(px(Space::XL))
        .child(
            div()
                .id("preferences-drawer-scroll")
                .w(px(396.0))
                .max_w_full()
                .h_full()
                .overflow_y_scroll()
                .rounded(px(Radius::LG))
                .border_1()
                .border_color(SHELL.control_border)
                .bg(SHELL.panel)
                .p(px(16.0))
                .child(preferences_panel_with_focus(state, on_close, focus)),
        )
        .into_any_element()
}

fn preferences_panel_with_focus(
    state: &AppState,
    on_close: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    mut focus: Option<&mut FocusContext<'_>>,
) -> AnyElement {
    let mut panel = Panel::new("Preferences");
    if let Some(close_cb) = on_close {
        panel = panel.action(
            Button::new("Done")
                .id("prefs-close")
                .focus_with(focus.as_deref_mut(), "prefs-close")
                .on_click(move |window, cx| close_cb(window, cx)),
        );
    }

    panel
        .child(
            Toggle::new("Keep window on top", state.settings.always_on_top)
                .id("prefs-always-on-top")
                .focus_with(focus.as_deref_mut(), "prefs-always-on-top")
                .detail(Some(if state.settings.always_on_top {
                    "Pinned"
                } else {
                    "Normal"
                }))
                .on_change(|_enabled, _window, _cx| push_effect(SideEffect::ToggleAlwaysOnTop))
                .into_element(),
        )
        .child(
            Toggle::new(
                "Show preview in this window",
                state.settings.preview_enabled,
            )
            .id("prefs-preview-enabled")
            .focus_with(focus.as_deref_mut(), "prefs-preview-enabled")
            .detail(Some(if state.settings.preview_enabled {
                "Preview On"
            } else {
                "Off (Virtual Camera Only)"
            }))
            .on_change(|enabled, _window, _cx| push_effect(SideEffect::SetPreviewEnabled(enabled)))
            .into_element(),
        )
        .child(
            Toggle::new(
                "Auto-start stream on connect",
                state.settings.auto_start_stream,
            )
            .id("prefs-auto-start-stream")
            .focus_with(focus.as_deref_mut(), "prefs-auto-start-stream")
            .detail(Some(if state.settings.auto_start_stream {
                "Starts camera automatically"
            } else {
                "Manual"
            }))
            .on_change(|val, _window, _cx| push_effect(SideEffect::SetAutoStartStream(val)))
            .into_element(),
        )
        .child(
            Toggle::new("Auto-connect on USB plug", state.settings.auto_connect_usb)
                .id("prefs-auto-connect-usb")
                .focus_with(focus.as_deref_mut(), "prefs-auto-connect-usb")
                .detail(Some(if state.settings.auto_connect_usb {
                    "Automatic USB setup"
                } else {
                    "Manual"
                }))
                .on_change(|val, _window, _cx| push_effect(SideEffect::SetAutoConnectUsb(val)))
                .into_element(),
        )
        .child(
            Toggle::new(
                "Minimize to tray on window close",
                state.settings.minimize_to_tray,
            )
            .id("prefs-minimize-to-tray")
            .focus_with(focus, "prefs-minimize-to-tray")
            .detail(Some(if state.settings.minimize_to_tray {
                "Keeps Virtual Camera in tray"
            } else {
                "Tray background preference off"
            }))
            .on_change(|val, _window, _cx| push_effect(SideEffect::SetMinimizeToTray(val)))
            .into_element(),
        )
        .child(
            div()
                .mt(px(Space::MD))
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .gap(px(Space::LG))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_SM))
                        .text_color(Text::BODY)
                        .child("Service"),
                )
                .child(chip(
                    if state.status.connected {
                        "Connected"
                    } else {
                        "Offline"
                    },
                    if state.status.connected {
                        Tone::Accent
                    } else {
                        Tone::Warn
                    },
                ))
                .into_any_element(),
        )
        .child(section(
            "Shortcuts",
            vec![
                key_value("Space (canvas)", "Start or stop streaming", false),
                key_value("F11", "Full-window preview", false),
                key_value("Esc", "Leave full-window preview / close drawer", false),
                key_value("Ctrl+1", "Setup tab", false),
                key_value("Ctrl+2", "Camera tab", false),
                key_value("Ctrl+3", "Output tab", false),
                key_value("Ctrl+4", "System tab", false),
                key_value("Tab / Shift+Tab", "Navigate controls", false),
                key_value("Enter / Space", "Activate focused control", false),
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
                .font(Fonts::sans())
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child(label.to_string()),
        )
        .child(
            div()
                .flex_shrink()
                .min_w(px(0.0))
                .truncate()
                .text_right()
                .font(Fonts::sans())
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
        (
            Accent::DEFAULT.alpha(0.75),
            Accent::DEFAULT.alpha(0.14),
            Text::PRIMARY,
        )
    } else {
        (SHELL.control_border, SHELL.control, Text::HEADING)
    };

    let mut el = div()
        .id(gpui::ElementId::Name(id.into()))
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .min_h(px(32.0))
        .rounded(px(Radius::SM))
        .border_1()
        .border_color(border)
        .bg(background)
        .px(px(12.0))
        .py(px(6.0))
        .font(Fonts::sans())
        .text_size(px(Type::XS))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(text)
        .child(label.into());

    if disabled {
        el = el.opacity(0.4).cursor_default();
    } else {
        let hover_border = if selected {
            Accent::HOVER
        } else {
            SHELL.control_border_hover
        };
        let hover_bg = if selected {
            Accent::DEFAULT.alpha(0.22)
        } else {
            SHELL.hover
        };
        let cb = Rc::new(on_click);
        let key_cb = Rc::clone(&cb);
        el = el
            .key_context("Control")
            .cursor_pointer()
            .hover(move |style| style.border_color(hover_border).bg(hover_bg))
            .active(|style| style.opacity(0.85))
            .focus(|style| style.border_2().border_color(Accent::DEFAULT))
            .on_click(move |_event, window, cx| cb(window, cx))
            .on_key_down(move |ev, window, cx| {
                if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                    key_cb(window, cx);
                    cx.stop_propagation();
                }
            });
    }

    el.into_any_element()
}

fn pill_button_compact(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    disabled: bool,
    on_click: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let (border, background, text) = if selected {
        (
            Accent::DEFAULT.alpha(0.75),
            Accent::DEFAULT.alpha(0.14),
            Text::PRIMARY,
        )
    } else {
        (SHELL.control_border, SHELL.control, Text::BODY)
    };

    let mut el = div()
        .id(gpui::ElementId::Name(id.into()))
        .flex()
        .flex_row()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .min_h(px(28.0))
        .rounded(px(Radius::SM))
        .border_1()
        .border_color(border)
        .bg(background)
        .px(px(10.0))
        .py(px(5.0))
        .font(Fonts::sans())
        .text_size(px(Type::XS))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(text)
        .child(label.into());

    if disabled {
        el = el.opacity(0.4).cursor_default();
    } else {
        let hover_border = if selected {
            Accent::HOVER
        } else {
            SHELL.control_border_hover
        };
        let hover_bg = if selected {
            Accent::DEFAULT.alpha(0.22)
        } else {
            SHELL.hover
        };
        let cb = Rc::new(on_click);
        let key_cb = Rc::clone(&cb);
        el = el
            .key_context("Control")
            .cursor_pointer()
            .hover(move |style| style.border_color(hover_border).bg(hover_bg))
            .active(|style| style.opacity(0.85))
            .focus(|style| style.border_2().border_color(Accent::DEFAULT))
            .on_click(move |_event, window, cx| cb(window, cx))
            .on_key_down(move |ev, window, cx| {
                if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                    key_cb(window, cx);
                    cx.stop_propagation();
                }
            });
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
        if !presets
            .iter()
            .any(|existing: &f32| (*existing - v).abs() < 0.05)
        {
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
    let mut seen_labels = std::collections::HashSet::new();
    seen_labels.insert("auto".to_string());

    let current_label = if !current.is_empty() && current != "auto" {
        Some(resolution_label_of(current))
    } else {
        None
    };

    let supports = |res: &str| -> bool { camera_resolutions.iter().any(|r| r == res) };

    let supports_label = |target_label: &str| -> bool {
        camera_resolutions
            .iter()
            .any(|r| resolution_label_of(r) == target_label)
    };

    // 1. 4K (3840x2160) - included if supported by camera or active
    let has_4k = supports("3840x2160")
        || supports_label("4K")
        || supports_label("2160p")
        || current_label.as_deref() == Some("4K")
        || current_label.as_deref() == Some("2160p");
    if has_4k {
        let res_4k = if current_label.as_deref() == Some("4K")
            || current_label.as_deref() == Some("2160p")
        {
            current.to_string()
        } else if supports("3840x2160") {
            "3840x2160".to_string()
        } else {
            camera_resolutions
                .iter()
                .find(|r| {
                    let l = resolution_label_of(r);
                    l == "4K" || l == "2160p"
                })
                .cloned()
                .unwrap_or_else(|| "3840x2160".to_string())
        };
        let lbl = resolution_label_of(&res_4k);
        if seen_labels.insert(lbl) {
            out.push(res_4k);
        }
    }

    // 2. 1440p (2560x1440) - included if supported by camera or active
    let has_1440p = supports("2560x1440")
        || supports_label("1440p")
        || current_label.as_deref() == Some("1440p");
    if has_1440p {
        let res_1440p = if current_label.as_deref() == Some("1440p") {
            current.to_string()
        } else if supports("2560x1440") {
            "2560x1440".to_string()
        } else {
            camera_resolutions
                .iter()
                .find(|r| resolution_label_of(r) == "1440p")
                .cloned()
                .unwrap_or_else(|| "2560x1440".to_string())
        };
        let lbl = resolution_label_of(&res_1440p);
        if seen_labels.insert(lbl) {
            out.push(res_1440p);
        }
    }

    // 3. 1080p (1920x1080)
    let res_1080p = if current_label.as_deref() == Some("1080p") {
        current.to_string()
    } else if supports("1920x1080") || camera_resolutions.is_empty() {
        "1920x1080".to_string()
    } else if let Some(r) = camera_resolutions
        .iter()
        .find(|r| resolution_label_of(r) == "1080p")
    {
        r.clone()
    } else {
        "1920x1080".to_string()
    };
    let lbl = resolution_label_of(&res_1080p);
    if seen_labels.insert(lbl) {
        out.push(res_1080p);
    }

    // 4. 720p (1280x720)
    let res_720p = if current_label.as_deref() == Some("720p") {
        current.to_string()
    } else if supports("1280x720") || camera_resolutions.is_empty() {
        "1280x720".to_string()
    } else if let Some(r) = camera_resolutions
        .iter()
        .find(|r| resolution_label_of(r) == "720p")
    {
        r.clone()
    } else {
        "1280x720".to_string()
    };
    let lbl = resolution_label_of(&res_720p);
    if seen_labels.insert(lbl) {
        out.push(res_720p);
    }

    // 5. 480p (854x480 or 640x480)
    let res_480p = if current_label.as_deref() == Some("480p") {
        current.to_string()
    } else if supports("854x480") || camera_resolutions.is_empty() {
        "854x480".to_string()
    } else if supports("640x480") {
        "640x480".to_string()
    } else if let Some(r) = camera_resolutions
        .iter()
        .find(|r| resolution_label_of(r) == "480p")
    {
        r.clone()
    } else {
        "854x480".to_string()
    };
    let lbl = resolution_label_of(&res_480p);
    if seen_labels.insert(lbl) {
        out.push(res_480p);
    }

    // 6. 360p (640x360)
    let res_360p = if current_label.as_deref() == Some("360p") {
        current.to_string()
    } else if supports("640x360") || camera_resolutions.is_empty() {
        "640x360".to_string()
    } else if let Some(r) = camera_resolutions
        .iter()
        .find(|r| resolution_label_of(r) == "360p")
    {
        r.clone()
    } else {
        "640x360".to_string()
    };
    let lbl = resolution_label_of(&res_360p);
    if seen_labels.insert(lbl) {
        out.push(res_360p);
    }

    // 7. Active resolution if custom (non-standard label)
    if let Some(lbl) = current_label {
        if seen_labels.insert(lbl) {
            out.push(current.to_string());
        }
    }

    out
}

fn fps_presets(camera_fps: &[u32], current: Option<u32>) -> Vec<Option<u32>> {
    let mut fixed: Vec<u32> = if camera_fps.is_empty() {
        vec![15, 24, 30, 60]
    } else {
        camera_fps.to_vec()
    };
    // Always unlock 60 fps so the user can stream at high refresh rates
    if !fixed.contains(&60) {
        fixed.push(60);
    }
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
        // Deduplication: 640x480 (active) replaces 854x480 as both share label '480p'
        assert!(!presets.contains(&"854x480".to_string()));
    }

    #[test]
    fn fps_presets_include_auto_and_sort_unique() {
        let presets = fps_presets(&[60, 30, 15, 30], Some(24));
        assert_eq!(presets, vec![None, Some(15), Some(24), Some(30), Some(60)]);
    }

    #[test]
    fn fps_presets_always_unlocks_60_fps() {
        let presets = fps_presets(&[30, 24], None);
        assert_eq!(presets, vec![None, Some(24), Some(30), Some(60)]);
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
        assert_eq!(
            usb_device_action("unauthorized"),
            UsbDeviceAction::AuthorizeHint
        );
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
    fn resolution_presets_deduplicates_by_label_and_keeps_clean_row() {
        let caps: Vec<String> = vec![
            "3840x2160",
            "2560x1440",
            "1920x1080",
            "1440x1080",
            "1280x720",
            "960x720",
            "854x480",
            "640x480",
            "640x360",
            "320x240",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let presets = resolution_presets(&caps, "auto");
        // Deduplicated: auto, 4K, 1440p, 1080p, 720p, 480p, 360p (7 total, 1 clean row)
        assert_eq!(presets.len(), 7);
        assert_eq!(presets[0], "auto");
        assert!(presets.contains(&"3840x2160".to_string()));
        assert!(presets.contains(&"2560x1440".to_string()));
        assert!(presets.contains(&"1920x1080".to_string()));
        assert!(!presets.contains(&"1440x1080".to_string())); // deduplicated duplicate 1080p
        assert!(presets.contains(&"1280x720".to_string()));
        assert!(!presets.contains(&"960x720".to_string())); // deduplicated duplicate 720p
        assert!(presets.contains(&"854x480".to_string()));
        assert!(!presets.contains(&"640x480".to_string())); // deduplicated duplicate 480p
        assert!(presets.contains(&"640x360".to_string()));

        // Verify every resolution in presets maps to a distinct label (no duplicate pills)
        let labels: Vec<String> = presets
            .iter()
            .map(|r| {
                if r == "auto" {
                    "Auto".to_string()
                } else {
                    resolution_label_of(r)
                }
            })
            .collect();
        let mut unique_labels = labels.clone();
        unique_labels.sort();
        unique_labels.dedup();
        assert_eq!(labels.len(), unique_labels.len());

        // And if a custom resolution is selected, it is preserved
        let custom_presets = resolution_presets(&caps, "320x240");
        assert!(custom_presets.contains(&"320x240".to_string()));
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

    #[test]
    fn capture_panel_renders_all_pro_controls_and_toggles() {
        let mut state = AppState::default();
        state.settings.auto_framing = true;
        state.settings.portrait_blur = true;
        state.settings.portrait_blur_intensity = 0.70;
        state.settings.stream_audio = true;
        state.audio_level = 0.65;
        state.settings.lut_preset = phonecam_media::LutPreset::WarmStudio;
        state.settings.screen_mode = phonecam_protocol::ScreenMode::Black;
        state.settings.auto_connect_usb = true;
        state.settings.minimize_to_tray = true;

        let _el = capture_panel(&state);
        let _pref = preferences_panel(&state);
        let _setup = connect_panel(&state);
        let _pub = publish_panel(&state);
    }

    #[test]
    fn lut_preset_all_includes_none_off_and_natural() {
        let presets = phonecam_media::LutPreset::all();
        assert!(presets.contains(&phonecam_media::LutPreset::None));
        assert!(presets.contains(&phonecam_media::LutPreset::Natural));
        assert!(presets.contains(&phonecam_media::LutPreset::WarmStudio));
        assert!(presets.contains(&phonecam_media::LutPreset::CoolCorporate));
        assert!(presets.contains(&phonecam_media::LutPreset::VibrantStreamer));
        assert!(presets.contains(&phonecam_media::LutPreset::Monochrome));

        assert_eq!(phonecam_media::LutPreset::None.display_name(), "None / Off");
        assert_eq!(phonecam_media::LutPreset::Natural.display_name(), "Natural");
    }

    #[test]
    fn rail_scroll_containers_render_with_distinct_ids_across_all_tabs() {
        let state = AppState::default();
        let scroll_handle = ScrollHandle::new();
        for tab in Tab::ALL {
            let _rail = rail(&state, tab, false, &scroll_handle);
            let _narrow = rail(&state, tab, true, &scroll_handle);
        }
    }

    #[test]
    fn clean_device_model_preserves_codes_and_cleans_underscores() {
        assert_eq!(clean_device_model(""), "");
        assert_eq!(clean_device_model("   "), "");
        assert_eq!(clean_device_model("2201117TG"), "2201117TG");
        assert_eq!(clean_device_model("Pixel_8_Pro"), "Pixel 8 Pro");
        assert_eq!(clean_device_model("Galaxy S24"), "Galaxy S24");
    }

    #[test]
    fn usb_list_renders_device_row_with_tunnel_active_and_use_phone() {
        let mut state = AppState::default();
        let mut usb = phonecam_protocol::UsbStatus {
            adb_available: true,
            available: true,
            authorized: true,
            forwarding: false,
            error: None,
            devices: vec![phonecam_protocol::AdbDevice {
                serial: "s123".into(),
                state: "device".into(),
                model: "2201117TG".into(),
            }],
        };
        state.core = Some(phonecam_protocol::CoreState {
            usb: usb.clone(),
            ..Default::default()
        });
        let _unforwarded = usb_list(&state);

        usb.forwarding = true;
        state.core = Some(phonecam_protocol::CoreState {
            usb,
            ..Default::default()
        });
        let _forwarded = usb_list(&state);
    }

    #[test]
    fn capture_panel_renders_active_controls_for_streaming_and_idle() {
        let mut state = AppState::default();
        // Disconnected: chip("No phone", Tone::Neutral)
        let _disconnected = capture_panel(&state);

        // Connected, idle: Button::primary("Start Camera")
        state.status.connected = true;
        state.core = Some(phonecam_protocol::CoreState {
            phase: phonecam_protocol::StreamPhase::Connected,
            device: Some(phonecam_protocol::DeviceIdentity {
                device_id: "dev-1".into(),
                name: "Test Phone".into(),
                model: "2201117TG".into(),
                android_version: 33,
            }),
            ..Default::default()
        });
        let _idle = capture_panel(&state);

        // Streaming: Button::danger("Stop Camera")
        if let Some(core) = &mut state.core {
            core.phase = phonecam_protocol::StreamPhase::Streaming;
        }
        let _streaming = capture_panel(&state);
    }

    #[test]
    fn paired_phone_panel_renders_with_well_and_clean_model() {
        let mut state = AppState::default();
        state.status.connected = true;
        state.core = Some(phonecam_protocol::CoreState {
            phase: phonecam_protocol::StreamPhase::Connected,
            device: Some(phonecam_protocol::DeviceIdentity {
                device_id: "dev-1".into(),
                name: "Test Phone".into(),
                model: "2201117TG".into(),
                android_version: 33,
            }),
            ..Default::default()
        });
        let _panel = paired_phone_panel(&state);
    }

    #[test]
    fn paired_phone_panel_with_stored_id_renders() {
        let mut state = AppState::default();
        state.settings.last_device_id = Some("stored-dev-42".into());
        let _panel = paired_phone_panel(&state);
    }

    #[test]
    fn paired_phone_panel_empty_when_no_device() {
        let state = AppState::default();
        let _panel = paired_phone_panel(&state);
    }

    #[test]
    fn audio_vu_meter_renders_active_and_muted_states() {
        let _active = audio_vu_meter(0.75, true);
        let _muted = audio_vu_meter(0.0, false);
    }

    #[test]
    fn capture_panel_renders_dynamic_capabilities_and_all_pro_toggles() {
        let mut state = AppState::default();
        state.status.connected = true;
        state.settings.auto_framing = true;
        state.settings.hand_gestures = true;
        state.settings.stream_audio = true;
        state.settings.audio_output_device = Some("CABLE Input (VB-Audio Virtual Cable)".into());
        state.available_audio_devices = vec![
            "CABLE Input (VB-Audio Virtual Cable)".into(),
            "Speakers (Realtek(R) Audio)".into(),
        ];
        state.audio_level = 0.50;
        state.tracking_locked = true;
        state.active_gesture = Some(phonecam_media::HandGesture::OpenPalm);

        state.capabilities = Some(phonecam_protocol::CapabilitiesPayload {
            device: phonecam_protocol::DeviceCapabilities {
                model: "Pixel 8 Pro".into(),
                android_version: 34,
                app_version: "1.2.1".into(),
            },
            cameras: vec![phonecam_protocol::CameraCapability {
                id: "0".into(),
                facing: phonecam_protocol::CameraFacing::Back,
                resolutions: vec![
                    "3840x2160".into(),
                    "1920x1080".into(),
                    "1280x720".into(),
                    "854x480".into(),
                ],
                fps: vec![24, 30, 60],
                torch: true,
                focus: true,
                zoom: true,
                exposure: true,
                white_balance: true,
                min_zoom: 1.0,
                max_zoom: 10.0,
                orientation_degrees: 90,
            }],
        });

        let _el = capture_panel(&state);
        let _diag = diagnostics_panel(&state);
        let _pref = preferences_panel(&state);
    }

    #[test]
    fn hand_gesture_guide_card_renders_all_states_and_gestures() {
        let mut state = AppState::default();
        state.settings.hand_gestures = true;
        state.settings.auto_framing = true;
        state.tracking_locked = true;

        // Test with no active gesture
        state.active_gesture = None;
        let _guide_none = hand_gesture_guide_card(&state);

        // Test with all 6 active gestures
        let gestures = [
            phonecam_media::HandGesture::OpenPalm,
            phonecam_media::HandGesture::PeaceV,
            phonecam_media::HandGesture::IndexPoint,
            phonecam_media::HandGesture::ShakaCall,
            phonecam_media::HandGesture::PinchOk,
            phonecam_media::HandGesture::ThumbsUp,
        ];
        for g in gestures {
            state.active_gesture = Some(g);
            let _guide_active = hand_gesture_guide_card(&state);
        }

        // Test with gestures disabled
        state.settings.hand_gestures = false;
        let _guide_disabled = hand_gesture_guide_card(&state);
    }

    #[test]
    fn status_strip_renders_correctly_across_all_connection_states() {
        // 1. Disconnected / Idle state
        let mut state = AppState::default();
        let _strip_idle = status_strip(&state);

        // 2. Connected phone with battery and device name
        state.status.connected = true;
        state.core = Some(phonecam_protocol::CoreState {
            phase: phonecam_protocol::StreamPhase::Connected,
            device: Some(phonecam_protocol::DeviceIdentity {
                device_id: "dev-pixel".into(),
                name: "Pixel 8 Pro".into(),
                model: "Pixel_8_Pro".into(),
                android_version: 34,
            }),
            phone: Some(phonecam_protocol::PhoneState {
                battery: 84,
                charging: false,
                thermal: phonecam_protocol::ThermalState::None,
                ..Default::default()
            }),
            vcam: Some(phonecam_protocol::VcamStatus {
                state: phonecam_protocol::VcamState::Running,
                name: "PhoneCam Camera".into(),
                registered: true,
                consumers: 1,
                error: None,
            }),
            ..Default::default()
        });
        let _strip_connected = status_strip(&state);

        // 3. Streaming with thermal warning, low battery, and stop button
        if let Some(core) = &mut state.core {
            core.phase = phonecam_protocol::StreamPhase::Streaming;
            if let Some(phone) = &mut core.phone {
                phone.battery = 12;
                phone.charging = false;
                phone.thermal = phonecam_protocol::ThermalState::Moderate;
            }
        }
        let _strip_streaming = status_strip(&state);

        // 4. Critical thermal state and charging
        if let Some(core) = &mut state.core {
            if let Some(phone) = &mut core.phone {
                phone.battery = 15;
                phone.charging = true;
                phone.thermal = phonecam_protocol::ThermalState::Critical;
            }
        }
        let _strip_critical = status_strip(&state);
    }

    #[test]
    fn audio_vu_meter_uses_logarithmic_dbfs_thresholds() {
        assert!(audio_level_dbfs(0.0) <= -60.0);
        assert!((audio_level_dbfs(1.0) - 0.0).abs() < 0.01);
        assert!((audio_level_dbfs(0.5) - (-6.02)).abs() < 0.2);

        // 10 green (-60..-12), 4 amber (-12..-3), 2 red (> -3)
        assert!(vu_segment_threshold_db(0) <= -55.0);
        assert!((vu_segment_threshold_db(9) - (-12.0)).abs() < 0.01);
        assert!(vu_segment_threshold_db(10) > -12.0 && vu_segment_threshold_db(10) < -3.0);
        assert!(vu_segment_threshold_db(14) >= -3.0);
        assert!(vu_segment_threshold_db(15) > -3.0);
    }

    #[test]
    fn capture_panel_with_ui_and_preferences_drawer_render() {
        let mut state = AppState::default();
        state.status.connected = true;
        state.core = Some(phonecam_protocol::CoreState {
            phase: phonecam_protocol::StreamPhase::Streaming,
            ..Default::default()
        });
        let callbacks = PanelCallbacks::default();
        let _collapsed = capture_panel_with_ui(
            &state,
            RailUiState {
                show_preferences: false,
                show_updates: false,
                usb_help_expanded: false,
                labs_expanded: false,
                gesture_ref_expanded: false,
            },
            &callbacks,
            None,
        );
        let _expanded = capture_panel_with_ui(
            &state,
            RailUiState {
                show_preferences: true,
                show_updates: true,
                usb_help_expanded: true,
                labs_expanded: true,
                gesture_ref_expanded: true,
            },
            &callbacks,
            None,
        );
        let _drawer = preferences_drawer(&state, None, None);
    }

    #[test]
    fn updates_modal_and_software_updates_card_render_all_states() {
        let mut state = AppState::default();
        let _idle_card = software_updates_card(&state, None, None);
        let _idle_modal = updates_modal(&state, None, None);

        state.update_status = UpdateStatus::Checking;
        let _checking_modal = updates_modal(&state, None, None);

        state.update_status = UpdateStatus::UpToDate {
            current: state.current_version.clone(),
        };
        let _up_to_date_modal = updates_modal(&state, None, None);

        state.update_status = UpdateStatus::Available(crate::views::state::ReleaseInfo {
            version: "2.0.0".into(),
            tag: "v2.0.0".into(),
            url: "https://github.com/primahagantha/phonecam/releases/tag/v2.0.0".into(),
            changelog: "## What's New\n- Studio UI overhaul\n- Bundled Inter fonts".into(),
            published_at: Some("2026-11-26".into()),
        });
        let _avail_card = software_updates_card(&state, None, None);
        let _avail_modal = updates_modal(&state, None, None);
        let _strip_with_dot = status_strip(&state);

        state.update_status = UpdateStatus::Failed("Network unreachable".into());
        let _failed_modal = updates_modal(&state, None, None);
    }

    #[test]
    fn setup_panel_collapses_usb_debugging_help_by_default() {
        let mut state = AppState::default();
        state.status.connected = true;
        state.core = Some(phonecam_protocol::CoreState {
            usb: phonecam_protocol::UsbStatus {
                adb_available: true,
                ..Default::default()
            },
            ..Default::default()
        });
        let callbacks = PanelCallbacks::default();
        let _collapsed = connect_panel_with_ui(&state, RailUiState::default(), &callbacks, None);
        let _expanded = connect_panel_with_ui(
            &state,
            RailUiState {
                usb_help_expanded: true,
                ..RailUiState::default()
            },
            &callbacks,
            None,
        );
    }

    #[test]
    fn live_studio_hud_renders_with_telemetry_and_controls() {
        let mut state = AppState::default();
        state.status.connected = true;
        state.frame_size = Some((1920, 1080));
        state.core = Some(phonecam_protocol::CoreState {
            phase: phonecam_protocol::StreamPhase::Streaming,
            stats: phonecam_protocol::ConnectionStats {
                fps: 59.8,
                latency_ms: 14.0,
                ..Default::default()
            },
            ..Default::default()
        });
        let _hud = crate::views::preview::live_studio_hud(&state, false, None);
        let _hud_focus = crate::views::preview::live_studio_hud(&state, true, None);
    }

    #[test]
    fn panel_copy_avoids_en_dashes_and_raw_gesture_emojis() {
        let _guide = usb_debugging_guide();
        let cleaned = crate::views::preview::clean_gesture_text("✋ Tracking Locked");
        assert!(!cleaned.contains('\u{270b}'));
        assert_eq!(cleaned, "Tracking Locked");
        let _strip = status_strip(&AppState::default());
    }
}
