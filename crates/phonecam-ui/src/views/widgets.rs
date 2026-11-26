//! The shared controls: panel, section, field, button, chip, toggle, slider,
//! key/value row, status dot.
//!
//! These exist so the design rules are applied once. Two of them are load-bearing
//! rather than stylistic:
//!
//! * **An unavailable control is replaced by a sentence saying why**, never
//!   greyed out in place. A dead grey control tells the user nothing; a sentence
//!   tells them what their hardware does not do.
//! * **A chip is the only uppercase-with-tracking in the system.** Everywhere
//!   else is sentence case, because when every group label shouts the hierarchy
//!   collapses into noise.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Hsla, IntoElement, SharedString};

use crate::theme::{Accent, Fonts, Radius, Space, Status, Text, Tone, Type, SHELL};

thread_local! {
    /// Tracks the currently dragged slider ID across frames on the UI thread.
    static ACTIVE_SLIDER_DRAG: RefCell<Option<SharedString>> = const { RefCell::new(None) };
}

/// Focus handle + sequential tab-index helper passed during view rendering.
pub struct FocusContext<'a> {
    handles: &'a mut std::collections::HashMap<String, gpui::FocusHandle>,
    next_tab_index: isize,
    app_cx: &'a mut gpui::App,
}

impl<'a> FocusContext<'a> {
    pub fn new(
        handles: &'a mut std::collections::HashMap<String, gpui::FocusHandle>,
        app_cx: &'a mut gpui::App,
    ) -> Self {
        Self {
            handles,
            next_tab_index: 1,
            app_cx,
        }
    }

    /// Returns a persistent `FocusHandle` and the next tab stop index for `key`.
    pub fn acquire(&mut self, key: &str) -> (gpui::FocusHandle, isize) {
        let handle = self
            .handles
            .entry(key.to_string())
            .or_insert_with(|| self.app_cx.focus_handle())
            .clone();
        let idx = self.next_tab_index;
        self.next_tab_index += 1;
        (handle, idx)
    }
}

/// A panel: a rule, a label, and its content.
///
/// A panel header is a rule and a label, not a second surface.
pub struct Panel {
    title: SharedString,
    actions: Vec<AnyElement>,
    children: Vec<AnyElement>,
}

impl Panel {
    pub fn new(title: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            actions: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn action(mut self, element: impl IntoElement) -> Self {
        self.actions.push(element.into_any_element());
        self
    }

    pub fn child(mut self, element: impl IntoElement) -> Self {
        self.children.push(element.into_any_element());
        self
    }

    pub fn children(mut self, elements: impl IntoIterator<Item = AnyElement>) -> Self {
        self.children.extend(elements);
        self
    }
}

impl IntoElement for Panel {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(Space::LG))
            .border_b_1()
            .border_color(SHELL.border.alpha(0.35))
            .px(px(Space::PAD))
            .py(px(Space::LG))
            .child(
                div()
                    .font(Fonts::display())
                    .text_size(px(Type::TITLE))
                    .text_color(Text::HEADING)
                    .child(self.title),
            )
            .children((!self.actions.is_empty()).then(|| {
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(Space::MD))
                    .children(self.actions)
            }));

        // No border and no shadow: a panel is a *grouping surface*, and giving it
        // a border, a fill and a shadow at once is the generic "card" look. The
        // fill alone separates it from the shell, and the header rule does the
        // rest. Depth in this window comes from the stage being recessed, not
        // from every panel claiming to float.
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w_full()
            .rounded(px(Radius::LG))
            .bg(SHELL.panel)
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .px(px(Space::PAD))
                    .py(px(Space::PAD))
                    .children(self.children),
            )
            .into_any_element()
    }
}

/// A labelled group inside a panel.
///
/// A word at body weight, not a tracked capital with a rule running off to the
/// right: the rule made every group heading look like a stamped divider.
pub fn section(label: impl Into<SharedString>, children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .mt(px(Space::SECTION))
        .child(
            div()
                .mb(px(Space::LG))
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(Text::BODY)
                .child(label.into()),
        )
        .children(children)
        .into_any_element()
}

/// A field: label, optional hint, the control, then a description **below** it.
///
/// The description goes underneath so it never pushes the control out of
/// alignment with the one beside it.
pub fn field(
    label: impl Into<SharedString>,
    hint: Option<SharedString>,
    control: AnyElement,
    description: Option<SharedString>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .mb(px(Space::FIELD_GAP))
        .child(
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .justify_between()
                .gap(px(Space::LG))
                .mb(px(Space::SM))
                .child(
                    div()
                        .min_w(px(0.0))
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_SM))
                        .text_color(Text::BODY)
                        .child(label.into()),
                )
                .children(hint.map(|hint| {
                    div()
                        .flex_shrink_0()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .child(hint)
                })),
        )
        .child(control)
        .children(description.map(|text| {
            div()
                .mt(px(Space::SM))
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .line_height(px(Type::LINE_XS))
                .text_color(Text::MUTED)
                .child(text)
        }))
        .into_any_element()
}

/// A field whose control cannot exist, replaced by a sentence saying why.
pub fn unavailable_field(
    label: impl Into<SharedString>,
    reason: impl Into<SharedString>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .mb(px(Space::FIELD_GAP))
        .child(
            div()
                .flex()
                .flex_row()
                .items_baseline()
                .justify_between()
                .gap(px(Space::LG))
                .mb(px(Space::SM))
                .child(
                    div()
                        .font(Fonts::sans())
                        .text_size(px(Type::TWO_SM))
                        .text_color(Text::MUTED)
                        .child(label.into()),
                )
                .child(chip("Not available", Tone::Neutral)),
        )
        .child(
            div()
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .line_height(px(Type::LINE_XS))
                .text_color(Text::MUTED)
                .child(reason.into()),
        )
        .into_any_element()
}

/// The standard focus ring style: accent border marking what is active/focused.
pub fn focus_ring(style: gpui::StyleRefinement) -> gpui::StyleRefinement {
    style.border_color(Accent::DEFAULT)
}

/// The variants a button can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Default,
    /// The one primary action on a surface.
    Primary,
    Danger,
    Ghost,
}

/// A button.
pub struct Button {
    id: Option<SharedString>,
    label: SharedString,
    variant: ButtonVariant,
    disabled: bool,
    selected: bool,
    full_width: bool,
    on_click: Option<Rc<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    focus_handle: Option<gpui::FocusHandle>,
    tab_index: Option<isize>,
}

impl Button {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            id: None,
            label: label.into(),
            variant: ButtonVariant::Default,
            disabled: false,
            selected: false,
            full_width: false,
            on_click: None,
            focus_handle: None,
            tab_index: None,
        }
    }

    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn primary(label: impl Into<SharedString>) -> Self {
        Self {
            variant: ButtonVariant::Primary,
            ..Self::new(label)
        }
    }

    pub fn danger(label: impl Into<SharedString>) -> Self {
        Self {
            variant: ButtonVariant::Danger,
            ..Self::new(label)
        }
    }

    pub fn ghost(label: impl Into<SharedString>) -> Self {
        Self {
            variant: ButtonVariant::Ghost,
            ..Self::new(label)
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Draws the button in its "selected" state, which the accent marks.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }

    pub fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.focus_handle = Some(handle.clone());
        self
    }

    pub fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = Some(index);
        self
    }

    pub fn focus_with(mut self, focus: Option<&mut FocusContext<'_>>, key: &str) -> Self {
        if let Some(ctx) = focus {
            let (handle, idx) = ctx.acquire(key);
            self.focus_handle = Some(handle);
            self.tab_index = Some(idx);
        }
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }
}

impl IntoElement for Button {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let (border, background, text) = match (self.variant, self.disabled) {
            (ButtonVariant::Primary, true) => (Accent::DEFAULT, Accent::DEFAULT, SHELL.recessed),
            (ButtonVariant::Primary, false) => (Accent::DEFAULT, Accent::DEFAULT, SHELL.recessed),
            (ButtonVariant::Danger, true) => (
                Status::DANGER.alpha(0.8),
                Status::DANGER.alpha(0.1),
                Status::DANGER_TEXT,
            ),
            (ButtonVariant::Danger, false) => (
                Status::DANGER.alpha(0.8),
                Status::DANGER.alpha(0.1),
                Status::DANGER_TEXT,
            ),
            (ButtonVariant::Ghost, true) => (
                gpui::transparent_black(),
                gpui::transparent_black(),
                Text::MUTED,
            ),
            (ButtonVariant::Ghost, false) => (
                gpui::transparent_black(),
                gpui::transparent_black(),
                Text::MUTED,
            ),
            (ButtonVariant::Default, _) if self.selected => {
                (Accent::DEFAULT, SHELL.control, Accent::DEFAULT)
            }
            (ButtonVariant::Default, true) => (SHELL.border, SHELL.control, Text::PRIMARY),
            (ButtonVariant::Default, false) => (SHELL.border, SHELL.control, Text::PRIMARY),
        };

        // The hover style is what separates a control from a label.
        let (hover_border, hover_bg) = match (self.variant, self.disabled) {
            (ButtonVariant::Primary, false) => (Accent::HOVER, Accent::HOVER),
            (ButtonVariant::Danger, false) => (Status::DANGER, Status::DANGER.alpha(0.2)),
            (ButtonVariant::Ghost, false) => (gpui::transparent_black(), SHELL.hover),
            (ButtonVariant::Default, false) if self.selected => (Accent::DEFAULT, SHELL.control),
            (ButtonVariant::Default, false) => (Text::MUTED, SHELL.hover),
            (_, true) => (border, background),
        };

        let elem_id = self.id.unwrap_or_else(|| self.label.clone());
        let mut element = div()
            .id(gpui::ElementId::Name(elem_id))
            .flex()
            .flex_row()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .gap(px(Space::MD))
            .rounded(px(Radius::MD))
            .border_1()
            .border_color(border)
            .bg(background)
            .px(px(Space::XL))
            .py(px(Space::MD))
            .font(Fonts::sans())
            .text_size(px(Type::TWO_SM))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(text)
            .child(self.label);

        if self.full_width {
            element = element.w_full();
        }

        if self.disabled {
            element = element.opacity(0.4).cursor_default();
        } else {
            element = element
                .key_context("Control")
                .cursor_pointer()
                .hover(move |style| style.border_color(hover_border).bg(hover_bg))
                .active(|style| style.opacity(0.85))
                .focus(|style| style.border_2().border_color(Accent::DEFAULT));

            if let Some(ref handle) = self.focus_handle {
                element = element.track_focus(handle);
            }

            if let Some(index) = self.tab_index {
                element = element.tab_index(index);
            }

            if let Some(handler) = self.on_click {
                let key_handler = Rc::clone(&handler);
                element = element
                    .on_click(move |_event, window, cx| {
                        handler(window, cx);
                    })
                    .on_key_down(move |event, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            key_handler(window, cx);
                            cx.stop_propagation();
                        }
                    });
            }
        }

        element.into_any_element()
    }
}

/// Tabular figures with cross-platform fallbacks, so a number does not change width as it counts.
pub fn numeric_font(family: &'static str) -> gpui::Font {
    gpui::Font {
        family: family.into(),
        features: gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".to_string(), 1)])),
        fallbacks: Some(Fonts::fallbacks_for(family)),
        weight: gpui::FontWeight::NORMAL,
        style: gpui::FontStyle::Normal,
    }
}

/// A status chip.
///
/// The only uppercase-with-tracking in the system: a three-word status token
/// genuinely benefits from being read as a stamp.
pub fn chip(label: impl Into<SharedString>, tone: Tone) -> AnyElement {
    div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(Space::SM))
        .rounded(px(Radius::SM))
        .border_1()
        .border_color(tone.border())
        .bg(tone.fill())
        .px(px(Space::MD))
        .py(px(Space::XS))
        .font(Fonts::display())
        .text_size(px(Type::TWO_XS))
        .text_color(tone.fg())
        .child(label.into().to_uppercase())
        .into_any_element()
}

/// A status dot. `rounded-full` territory.
pub fn status_dot(tone: Tone) -> AnyElement {
    div()
        .flex_shrink_0()
        .w(px(8.0))
        .h(px(8.0))
        .rounded(px(Radius::FULL))
        .bg(tone.dot())
        .into_any_element()
}

/// A key/value row.
pub fn key_value(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    mono: bool,
) -> AnyElement {
    let value: SharedString = value.into();
    div()
        .flex()
        .flex_row()
        .items_baseline()
        .justify_between()
        .gap(px(Space::PAD))
        .py(px(Space::XS))
        .child(
            div()
                .flex_shrink()
                .min_w(px(0.0))
                .truncate()
                .font(Fonts::sans())
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child(label.into()),
        )
        .child(
            div()
                .flex_shrink()
                .min_w(px(0.0))
                .truncate()
                .text_right()
                .font(numeric_font(if mono { Fonts::MONO } else { Fonts::SANS }))
                .text_size(px(if mono { Type::XS } else { Type::TWO_SM }))
                .text_color(Text::HEADING)
                .child(value),
        )
        .into_any_element()
}

/// A recessed well for data that is not a control.
pub fn well(children: Vec<AnyElement>) -> AnyElement {
    div()
        .w_full()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border.alpha(0.35))
        .bg(SHELL.recessed)
        .p(px(Space::MD))
        .children(children)
        .into_any_element()
}

/// A notice or informational box, tinted with its tone.
pub fn tinted_box(tone: Tone, children: Vec<AnyElement>) -> AnyElement {
    div()
        .w_full()
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(tone.border())
        .bg(tone.fill())
        .px(px(Space::XL))
        .py(px(Space::LG))
        .children(children)
        .into_any_element()
}

/// A toggle: a track and a knob, `rounded-full`.
pub struct Toggle {
    id: Option<SharedString>,
    label: SharedString,
    checked: bool,
    detail: Option<SharedString>,
    disabled: bool,
    on_change: Option<Rc<dyn Fn(bool, &mut gpui::Window, &mut gpui::App) + 'static>>,
    focus_handle: Option<gpui::FocusHandle>,
    tab_index: Option<isize>,
}

impl Toggle {
    pub fn new(label: impl Into<SharedString>, checked: bool) -> Self {
        Self {
            id: None,
            label: label.into(),
            checked,
            detail: None,
            disabled: false,
            on_change: None,
            focus_handle: None,
            tab_index: None,
        }
    }

    pub fn id(mut self, id: impl Into<SharedString>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn detail(mut self, detail: Option<impl Into<SharedString>>) -> Self {
        self.detail = detail.map(Into::into);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.focus_handle = Some(handle.clone());
        self
    }

    pub fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = Some(index);
        self
    }

    pub fn focus_with(mut self, focus: Option<&mut FocusContext<'_>>, key: &str) -> Self {
        if let Some(ctx) = focus {
            let (handle, idx) = ctx.acquire(key);
            self.focus_handle = Some(handle);
            self.tab_index = Some(idx);
        }
        self
    }

    pub fn on_change(
        mut self,
        handler: impl Fn(bool, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl IntoElement for Toggle {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let checked = self.checked;
        let track_bg = if checked {
            Accent::DEFAULT
        } else {
            SHELL.control
        };
        let track_border = if checked {
            Accent::DEFAULT
        } else {
            SHELL.border
        };

        let elem_id = self.id.unwrap_or_else(|| self.label.clone());
        let mut row = div()
            .id(gpui::ElementId::Name(elem_id))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(Space::PAD))
            .mb(px(Space::SM))
            .rounded(px(Radius::MD))
            .border_1()
            .border_color(gpui::transparent_black())
            .px(px(Space::SM))
            .py(px(Space::XS))
            .child(
                div()
                    .flex_shrink()
                    .min_w(px(0.0))
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .items_baseline()
                    .gap(px(Space::MD))
                    .child(
                        div()
                            .font(Fonts::sans())
                            .text_size(px(Type::TWO_SM))
                            .text_color(Text::BODY)
                            .child(self.label),
                    )
                    .children(self.detail.map(|detail| {
                        div()
                            .font(Fonts::sans())
                            .text_size(px(Type::TWO_XS))
                            .text_color(Text::MUTED)
                            .child(detail)
                    })),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .w(px(28.0))
                    .h(px(16.0))
                    .rounded(px(Radius::FULL))
                    .border_1()
                    .border_color(track_border)
                    .bg(track_bg)
                    .child(
                        div()
                            .w(px(12.0))
                            .h(px(12.0))
                            .rounded(px(Radius::FULL))
                            .bg(Text::PRIMARY)
                            .ml(px(if checked { 12.0 } else { 2.0 })),
                    ),
            );

        if self.disabled {
            row = row.opacity(0.4);
        } else {
            row = row
                .key_context("Control")
                .cursor_pointer()
                .hover(|style| style.bg(SHELL.hover).border_color(SHELL.border.alpha(0.5)))
                .focus(|style| style.border_2().border_color(Accent::DEFAULT));

            if let Some(ref handle) = self.focus_handle {
                row = row.track_focus(handle);
            }

            if let Some(index) = self.tab_index {
                row = row.tab_index(index);
            }

            if let Some(handler) = self.on_change {
                let key_handler = Rc::clone(&handler);
                row = row
                    .on_click(move |_event, window, cx| {
                        handler(!checked, window, cx);
                    })
                    .on_key_down(move |event, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            key_handler(!checked, window, cx);
                            cx.stop_propagation();
                        }
                    });
            }
        }
        row.into_any_element()
    }
}

/// Computes normalized `[0.0, 1.0]` position for a slider value.
pub fn slider_ratio(value: f32, min: f32, max: f32) -> f32 {
    let span = max - min;
    if !span.is_finite() || span <= 0.0 {
        return 0.0;
    }
    ((value - min) / span).clamp(0.0, 1.0)
}

/// Snaps a slider value to `step` within `[min, max]`.
pub fn slider_snap(raw: f32, min: f32, max: f32, step: f32) -> f32 {
    let clamped = raw.clamp(min, max);
    if !step.is_finite() || step <= 0.0 {
        return clamped;
    }
    let steps = ((clamped - min) / step).round();
    let snapped = (min + steps * step).clamp(min, max);
    (snapped * 1000.0).round() / 1000.0
}

/// Converts a pointer x-coordinate inside `origin_x..origin_x+width` to a snapped slider value.
pub fn slider_value_from_x(
    x: f32,
    origin_x: f32,
    width: f32,
    min: f32,
    max: f32,
    step: f32,
) -> f32 {
    let ratio = ((x - origin_x) / width.max(1.0)).clamp(0.0, 1.0);
    slider_snap(min + ratio * (max - min), min, max, step)
}

/// Interactive horizontal slider supporting click, drag, scroll-wheel, and arrow keys.
pub struct Slider {
    id: SharedString,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    disabled: bool,
    on_change: Option<Rc<dyn Fn(f32, &mut gpui::Window, &mut gpui::App) + 'static>>,
    focus_handle: Option<gpui::FocusHandle>,
    tab_index: Option<isize>,
}

impl Slider {
    pub fn new(id: impl Into<SharedString>, value: f32, min: f32, max: f32) -> Self {
        Self {
            id: id.into(),
            value,
            min,
            max,
            step: 0.1,
            disabled: false,
            on_change: None,
            focus_handle: None,
            tab_index: None,
        }
    }

    pub fn step(mut self, step: f32) -> Self {
        self.step = step;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn track_focus(mut self, handle: &gpui::FocusHandle) -> Self {
        self.focus_handle = Some(handle.clone());
        self
    }

    pub fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = Some(index);
        self
    }

    pub fn focus_with(mut self, focus: Option<&mut FocusContext<'_>>, key: &str) -> Self {
        if let Some(ctx) = focus {
            let (handle, idx) = ctx.acquire(key);
            self.focus_handle = Some(handle);
            self.tab_index = Some(idx);
        }
        self
    }

    pub fn on_change(
        mut self,
        handler: impl Fn(f32, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_change = Some(Rc::new(handler));
        self
    }
}

impl IntoElement for Slider {
    type Element = AnyElement;

    fn into_element(self) -> Self::Element {
        let min = self.min;
        let max = self.max.max(self.min);
        let step = self.step;
        let value = self.value.clamp(min, max);
        let ratio = slider_ratio(value, min, max);
        let slider_id = self.id.clone();

        let track_bounds = Rc::new(Cell::new(gpui::Bounds::<gpui::Pixels>::default()));
        let prepaint_bounds = Rc::clone(&track_bounds);
        let drag_id = slider_id.clone();
        let drag_handler = self.on_change.clone();

        let tracker_canvas = gpui::canvas(
            move |bounds, _window, _cx| {
                prepaint_bounds.set(bounds);
                bounds
            },
            move |bounds, _prepaint_state, window, _cx| {
                let is_dragging =
                    ACTIVE_SLIDER_DRAG.with(|active| active.borrow().as_ref() == Some(&drag_id));
                if !is_dragging {
                    return;
                }
                let move_id = drag_id.clone();
                if let Some(handler) = drag_handler.clone() {
                    window.on_mouse_event(move |ev: &gpui::MouseMoveEvent, phase, window, cx| {
                        if phase != gpui::DispatchPhase::Capture {
                            return;
                        }
                        let still_dragging = ACTIVE_SLIDER_DRAG
                            .with(|active| active.borrow().as_ref() == Some(&move_id));
                        if !still_dragging {
                            return;
                        }
                        if ev.pressed_button != Some(gpui::MouseButton::Left) {
                            ACTIVE_SLIDER_DRAG.with(|active| {
                                *active.borrow_mut() = None;
                            });
                            return;
                        }
                        let next = slider_value_from_x(
                            f32::from(ev.position.x),
                            f32::from(bounds.origin.x),
                            f32::from(bounds.size.width),
                            min,
                            max,
                            step,
                        );
                        if (next - value).abs() > 1e-4 {
                            handler(next, window, cx);
                        }
                    });
                }
                window.on_mouse_event(move |_ev: &gpui::MouseUpEvent, phase, _window, _cx| {
                    if phase == gpui::DispatchPhase::Capture {
                        ACTIVE_SLIDER_DRAG.with(|active| {
                            *active.borrow_mut() = None;
                        });
                    }
                });
            },
        )
        .absolute()
        .inset_0();

        let track = div()
            .relative()
            .w_full()
            .h(px(6.0))
            .rounded(px(Radius::FULL))
            .border_1()
            .border_color(SHELL.border)
            .bg(SHELL.recessed)
            .child(
                div()
                    .h_full()
                    .w(gpui::relative(ratio))
                    .rounded(px(Radius::FULL))
                    .bg(Accent::DEFAULT),
            )
            .child(
                div()
                    .absolute()
                    .top(px(-5.0))
                    .left(gpui::relative(ratio))
                    .ml(px(-7.0))
                    .w(px(14.0))
                    .h(px(14.0))
                    .rounded(px(Radius::FULL))
                    .border_2()
                    .border_color(Accent::DEFAULT)
                    .bg(Text::PRIMARY),
            )
            .child(tracker_canvas);

        let mut container = div()
            .id(gpui::ElementId::Name(slider_id.clone()))
            .w_full()
            .h(px(24.0))
            .flex()
            .items_center()
            .rounded(px(Radius::MD))
            .border_1()
            .border_color(gpui::transparent_black())
            .px(px(Space::SM))
            .child(track);

        if self.disabled {
            container = container.opacity(0.4).cursor_default();
        } else {
            container = container
                .key_context("Control")
                .cursor_pointer()
                .hover(|s| s.border_color(SHELL.border.alpha(0.5)))
                .focus(|s| s.border_2().border_color(Accent::DEFAULT));

            if let Some(ref handle) = self.focus_handle {
                container = container.track_focus(handle);
            }

            if let Some(index) = self.tab_index {
                container = container.tab_index(index);
            }

            if let Some(handler) = self.on_change {
                let down_handler = Rc::clone(&handler);
                let down_id = slider_id;
                let down_bounds = Rc::clone(&track_bounds);
                let scroll_handler = Rc::clone(&handler);
                let key_handler = Rc::clone(&handler);

                container = container
                    .on_mouse_down(gpui::MouseButton::Left, move |ev, window, cx| {
                        ACTIVE_SLIDER_DRAG.with(|active| {
                            *active.borrow_mut() = Some(down_id.clone());
                        });
                        let b = down_bounds.get();
                        let next = slider_value_from_x(
                            f32::from(ev.position.x),
                            f32::from(b.origin.x),
                            f32::from(b.size.width),
                            min,
                            max,
                            step,
                        );
                        down_handler(next, window, cx);
                    })
                    .on_scroll_wheel(move |ev, window, cx| {
                        let dy = match ev.delta {
                            gpui::ScrollDelta::Pixels(p) => f32::from(p.y),
                            gpui::ScrollDelta::Lines(l) => l.y,
                        };
                        if dy.abs() > 0.0 {
                            let dir = if dy > 0.0 { 1.0 } else { -1.0 };
                            let next = slider_snap(value + dir * step, min, max, step);
                            if (next - value).abs() > 1e-4 {
                                scroll_handler(next, window, cx);
                            }
                            cx.stop_propagation();
                        }
                    })
                    .on_key_down(move |ev, window, cx| {
                        let next = match ev.keystroke.key.as_str() {
                            "left" | "down" => Some(slider_snap(value - step, min, max, step)),
                            "right" | "up" => Some(slider_snap(value + step, min, max, step)),
                            "home" => Some(min),
                            "end" => Some(max),
                            _ => None,
                        };
                        if let Some(next_val) = next {
                            if (next_val - value).abs() > 1e-4 {
                                key_handler(next_val, window, cx);
                            }
                            cx.stop_propagation();
                        }
                    });
            }
        }

        container.into_any_element()
    }
}

/// A single-line value with a label above it, for a large readout.
pub fn readout(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    tone: Option<Tone>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .font(Fonts::sans())
                .text_size(px(Type::TWO_XS))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(Text::BODY)
                .child(label.into()),
        )
        .child(
            div()
                .mt(px(Space::XS))
                .font(numeric_font(Fonts::DISPLAY))
                .text_size(px(Type::DISPLAY))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(tone.map(|t| t.fg()).unwrap_or(Text::PRIMARY))
                .child(value.into()),
        )
        .into_any_element()
}

/// A small metric: label above, value below.
pub fn metric(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    tone: Option<Tone>,
) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .min_w(px(0.0))
        .child(
            div()
                .font(Fonts::sans())
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child(label.into()),
        )
        .child(
            div()
                .mt(px(Space::XS))
                .font(numeric_font(Fonts::SANS))
                .text_size(px(Type::TWO_SM))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(tone.map(|t| t.fg()).unwrap_or(Text::HEADING))
                .child(value.into()),
        )
        .into_any_element()
}

/// Body prose, the most common thing on a panel.
pub fn prose(text: impl Into<SharedString>) -> AnyElement {
    div()
        .font(Fonts::sans())
        .text_size(px(Type::XS))
        .line_height(px(Type::LINE_XS))
        .text_color(Text::MUTED)
        .child(text.into())
        .into_any_element()
}

/// A hairline rule.
pub fn rule() -> AnyElement {
    div()
        .w_full()
        .h(px(1.0))
        .bg(SHELL.border.alpha(0.35))
        .into_any_element()
}

/// An empty state: a sentence, and optionally the action that resolves it.
pub fn empty_state(title: impl Into<SharedString>, body: impl Into<SharedString>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .py(px(Space::SM))
        .child(
            div()
                .font(Fonts::sans())
                .text_size(px(Type::TWO_SM))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(Text::HEADING)
                .child(title.into()),
        )
        .child(
            div()
                .mt(px(Space::SM))
                .font(Fonts::sans())
                .text_size(px(Type::XS))
                .line_height(px(Type::LINE_XS))
                .text_color(Text::MUTED)
                .child(body.into()),
        )
        .into_any_element()
}

/// A coloured value for a `key_value` row.
pub fn toned_text(text: impl Into<SharedString>, colour: Hsla) -> AnyElement {
    div()
        .text_color(colour)
        .child(text.into())
        .into_any_element()
}

#[cfg(test)]
mod interaction_tests {
    use super::*;

    /// The hover colours a variant resolves to, mirroring the builder.
    fn hover_target(variant: ButtonVariant, disabled: bool, selected: bool) -> (Hsla, Hsla) {
        match (variant, disabled) {
            (ButtonVariant::Primary, false) => (Accent::HOVER, Accent::HOVER),
            (ButtonVariant::Danger, false) => (Status::DANGER, Status::DANGER.alpha(0.2)),
            (ButtonVariant::Ghost, false) => (gpui::transparent_black(), SHELL.hover),
            (ButtonVariant::Default, false) if selected => (Accent::DEFAULT, SHELL.control),
            (ButtonVariant::Default, false) => (Text::MUTED, SHELL.hover),
            (_, true) => (SHELL.border, SHELL.control),
        }
    }

    #[test]
    fn every_button_variant_has_a_distinct_hover_background() {
        let variants = [
            ButtonVariant::Default,
            ButtonVariant::Primary,
            ButtonVariant::Danger,
            ButtonVariant::Ghost,
        ];
        let mut backgrounds = Vec::new();
        for variant in variants {
            let (_, background) = hover_target(variant, false, false);
            backgrounds.push(format!("{background:?}"));
        }
        let distinct: std::collections::HashSet<_> = backgrounds.iter().collect();
        assert!(
            distinct.len() >= 3,
            "variants share hover backgrounds, so hover cannot distinguish them: {backgrounds:?}"
        );
    }

    #[test]
    fn a_disabled_button_has_no_hover_change() {
        let (border, background) = hover_target(ButtonVariant::Default, true, false);
        assert_eq!(format!("{border:?}"), format!("{:?}", SHELL.border));
        assert_eq!(format!("{background:?}"), format!("{:?}", SHELL.control));
    }

    #[test]
    fn numeric_text_requests_tabular_figures_and_fallbacks() {
        let font = numeric_font(Fonts::MONO);
        let features = font.features.tag_value_list();
        assert!(
            features
                .iter()
                .any(|(tag, value)| tag == "tnum" && *value == 1),
            "readouts must use tabular figures, or every number shifts the row as it updates"
        );
        assert!(
            font.fallbacks.is_some(),
            "numeric_font must wire cross-platform font fallbacks"
        );
    }

    #[test]
    fn the_accent_hover_step_is_lighter_than_the_resting_accent() {
        assert!(
            Accent::HOVER.l > Accent::DEFAULT.l,
            "the hover step must be lighter than the resting accent"
        );
    }

    #[test]
    fn a_panel_groups_and_a_control_acts() {
        assert!(Radius::LG > Radius::MD, "a panel groups, a control acts");
    }

    #[test]
    fn button_focus_ring_uses_accent_color() {
        let style = focus_ring(gpui::StyleRefinement::default());
        assert_eq!(style.border_color, Some(Accent::DEFAULT));
    }

    #[test]
    fn button_and_toggle_support_focus_and_explicit_ids() {
        let btn = Button::new("x").id("notice-dismiss-7").tab_index(1);
        assert_eq!(
            btn.id.as_ref().map(|s| s.as_ref()),
            Some("notice-dismiss-7")
        );
        assert_eq!(btn.tab_index, Some(1));

        let tgl = Toggle::new("Test", false).id("tgl-test").tab_index(2);
        assert_eq!(tgl.id.as_ref().map(|s| s.as_ref()), Some("tgl-test"));
        assert_eq!(tgl.tab_index, Some(2));
    }

    #[test]
    fn button_supports_full_width() {
        let btn = Button::new("Test").full_width();
        assert!(btn.full_width);
    }

    #[test]
    fn slider_ratio_snap_and_pointer_math_are_accurate() {
        assert!((slider_ratio(5.0, 0.0, 10.0) - 0.5).abs() < 1e-5);
        assert_eq!(slider_ratio(-1.0, 0.0, 10.0), 0.0);
        assert_eq!(slider_ratio(20.0, 0.0, 10.0), 1.0);
        assert_eq!(slider_ratio(5.0, 10.0, 10.0), 0.0);

        assert!((slider_snap(1.44, 1.0, 8.0, 0.1) - 1.4).abs() < 1e-4);
        assert!((slider_snap(1.46, 1.0, 8.0, 0.1) - 1.5).abs() < 1e-4);
        assert_eq!(slider_snap(12.0, 1.0, 8.0, 0.5), 8.0);
        assert_eq!(slider_snap(0.2, 1.0, 8.0, 0.5), 1.0);

        let mid = slider_value_from_x(150.0, 100.0, 100.0, 1.0, 5.0, 0.5);
        assert!((mid - 3.0).abs() < 1e-4);
        let left_edge = slider_value_from_x(50.0, 100.0, 100.0, 1.0, 5.0, 0.5);
        assert!((left_edge - 1.0).abs() < 1e-4);
        let right_edge = slider_value_from_x(250.0, 100.0, 100.0, 1.0, 5.0, 0.5);
        assert!((right_edge - 5.0).abs() < 1e-4);
    }
}
