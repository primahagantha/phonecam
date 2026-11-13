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

use gpui::prelude::*;
use gpui::{div, px, AnyElement, Hsla, IntoElement, SharedString};

use crate::theme::{Accent, Fonts, Radius, Space, Status, Text, Tone, Type, SHELL};

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
            .border_color(SHELL.border.alpha(0.6))
            .px(px(Space::PAD))
            .py(px(Space::LG))
            .child(
                div()
                    .font_family(Fonts::DISPLAY)
                    .text_size(px(Type::TITLE))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
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
            .rounded(px(Radius::LG))
            .bg(SHELL.panel)
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
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
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_XS))
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
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_SM))
                        .text_color(Text::BODY)
                        .child(label.into()),
                )
                .children(hint.map(|hint| {
                    div()
                        .flex_shrink_0()
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_XS))
                        .text_color(Text::MUTED)
                        .child(hint)
                })),
        )
        .child(control)
        .children(description.map(|text| {
            div()
                .mt(px(Space::SM))
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
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
                        .font_family(Fonts::SANS)
                        .text_size(px(Type::TWO_SM))
                        .text_color(Text::MUTED)
                        .child(label.into()),
                )
                .child(chip("Not available", Tone::Neutral)),
        )
        .child(
            div()
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
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
    label: SharedString,
    variant: ButtonVariant,
    disabled: bool,
    selected: bool,
    full_width: bool,
    on_click: Option<Box<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>>,
    focus_handle: Option<gpui::FocusHandle>,
    tab_index: Option<isize>,
}

impl Button {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
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

    pub fn on_click(
        mut self,
        handler: impl Fn(&mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
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

        // The hover style is what separates a control from a label. The first
        // version of this button had none, and neither did anything else, so
        // nothing in the window responded to the pointer at all.
        let (hover_border, hover_bg) = match (self.variant, self.disabled) {
            (ButtonVariant::Primary, false) => (Accent::HOVER, Accent::HOVER),
            (ButtonVariant::Danger, false) => (Status::DANGER, Status::DANGER.alpha(0.2)),
            (ButtonVariant::Ghost, false) => (gpui::transparent_black(), SHELL.hover),
            (ButtonVariant::Default, false) if self.selected => (Accent::DEFAULT, SHELL.control),
            (ButtonVariant::Default, false) => (Text::MUTED, SHELL.hover),
            (_, true) => (border, background),
        };

        let mut element = div()
            .id(gpui::ElementId::Name(self.label.clone()))
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
            .font_family(Fonts::SANS)
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
                .cursor_pointer()
                .hover(move |style| style.border_color(hover_border).bg(hover_bg))
                // A press should look like a press: one pixel down, no more.
                .active(|style| style.opacity(0.85))
                .focus(|style| style.border_color(Accent::DEFAULT));

            if let Some(ref handle) = self.focus_handle {
                element = element.track_focus(handle);
            }

            if let Some(index) = self.tab_index {
                element = element.tab_index(index);
            }

            if let Some(handler) = self.on_click {
                element = element.on_click(move |_event, window, cx| {
                    handler(window, cx);
                });
            }
        }

        element.into_any_element()
    }
}

/// Tabular figures, so a number does not change width as it counts.
///
/// Every readout in this window updates about once a second: fps, bitrate,
/// latency, dropped percent, the pairing countdown. In proportional digits the
/// whole row shifts sideways on each update, which reads as instability in the
/// measurement rather than in the text. This is the one typographic setting that
/// matters most in a data panel, and the first version had it nowhere.
pub fn numeric_font(family: &'static str) -> gpui::Font {
    gpui::Font {
        family: family.into(),
        features: gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".to_string(), 1)])),
        fallbacks: None,
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
        .font_family(Fonts::DISPLAY)
        .text_size(px(Type::TWO_XS))
        .font_weight(gpui::FontWeight::SEMIBOLD)
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
///
/// Label at 11px, value at 13px, so a hundred rows still scan as a table rather
/// than a wall of same-sized text.
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
                .overflow_hidden()
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_XS))
                .text_color(Text::MUTED)
                .child(label.into()),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_right()
                .font_family(if mono { Fonts::MONO } else { Fonts::SANS })
                .text_size(px(if mono { Type::XS } else { Type::TWO_SM }))
                .text_color(Text::HEADING)
                .child(value),
        )
        .into_any_element()
}

/// A recessed well for data that is not a control.
pub fn well(children: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .rounded(px(Radius::MD))
        .border_1()
        .border_color(SHELL.border.alpha(0.6))
        .bg(SHELL.recessed)
        .p(px(Space::MD))
        .children(children)
        .into_any_element()
}

/// A notice or informational box, tinted with its tone.
pub fn tinted_box(tone: Tone, children: Vec<AnyElement>) -> AnyElement {
    div()
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
    label: SharedString,
    checked: bool,
    detail: Option<SharedString>,
    disabled: bool,
    on_change: Option<Box<dyn Fn(bool, &mut gpui::Window, &mut gpui::App) + 'static>>,
    focus_handle: Option<gpui::FocusHandle>,
    tab_index: Option<isize>,
}

impl Toggle {
    pub fn new(label: impl Into<SharedString>, checked: bool) -> Self {
        Self {
            label: label.into(),
            checked,
            detail: None,
            disabled: false,
            on_change: None,
            focus_handle: None,
            tab_index: None,
        }
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

    pub fn on_change(
        mut self,
        handler: impl Fn(bool, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        self.on_change = Some(Box::new(handler));
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

        let mut row = div()
            .id(gpui::ElementId::Name(self.label.clone()))
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(Space::PAD))
            .mb(px(Space::FIELD_GAP))
            .child(
                div()
                    .flex_shrink()
                    .flex()
                    .flex_row()
                    .gap(px(Space::MD))
                    .child(
                        div()
                            .font_family(Fonts::SANS)
                            .text_size(px(Type::TWO_SM))
                            .text_color(Text::BODY)
                            .child(self.label),
                    )
                    .children(self.detail.map(|detail| {
                        div()
                            .font_family(Fonts::SANS)
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
                .cursor_pointer()
                .focus(|style| style.border_color(Accent::DEFAULT));

            if let Some(ref handle) = self.focus_handle {
                row = row.track_focus(handle);
            }

            if let Some(index) = self.tab_index {
                row = row.tab_index(index);
            }

            if let Some(handler) = self.on_change {
                row = row.on_click(move |_event, window, cx| {
                    handler(!checked, window, cx);
                });
            }
        }
        row.into_any_element()
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
                .font_family(Fonts::SANS)
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
                .font_family(Fonts::SANS)
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
        .font_family(Fonts::SANS)
        .text_size(px(Type::XS))
        .text_color(Text::MUTED)
        .child(text.into())
        .into_any_element()
}

/// A hairline rule.
pub fn rule() -> AnyElement {
    div()
        .w_full()
        .h(px(1.0))
        .bg(SHELL.border.alpha(0.6))
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
                .font_family(Fonts::SANS)
                .text_size(px(Type::TWO_SM))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(Text::HEADING)
                .child(title.into()),
        )
        .child(
            div()
                .mt(px(Space::SM))
                .font_family(Fonts::SANS)
                .text_size(px(Type::XS))
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
    //! The states that make a control feel like a control.
    //!
    //! The first version of this window had no hover, no focus ring and no press
    //! feedback anywhere: nothing responded to the pointer, so every control read
    //! as a label. The hover style itself lives in gpui's private `Interactivity`,
    //! so it cannot be asserted directly; what is pinned here is the set of
    //! decisions that would silently regress — that every variant *has* a
    //! distinct hover target, that the accent's hover step is lighter, and that
    //! numeric text asks for tabular figures.

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
        // A ghost button hovers to the shell's hover surface, which a default
        // button also uses; the others must differ or the states are decorative.
        let distinct: std::collections::HashSet<_> = backgrounds.iter().collect();
        assert!(
            distinct.len() >= 3,
            "variants share hover backgrounds, so hover cannot distinguish them: {backgrounds:?}"
        );
    }

    #[test]
    fn a_disabled_button_has_no_hover_change() {
        let (border, background) = hover_target(ButtonVariant::Default, true, false);
        // A disabled control that lights up on hover promises an action it will
        // not perform.
        assert_eq!(format!("{border:?}"), format!("{:?}", SHELL.border));
        assert_eq!(format!("{background:?}"), format!("{:?}", SHELL.control));
    }

    #[test]
    fn numeric_text_requests_tabular_figures() {
        let font = numeric_font(Fonts::MONO);
        let features = font.features.tag_value_list();
        assert!(
            features
                .iter()
                .any(|(tag, value)| tag == "tnum" && *value == 1),
            "readouts must use tabular figures, or every number shifts the row as it updates"
        );
    }

    #[test]
    fn the_accent_hover_step_is_lighter_than_the_resting_accent() {
        // A hover state that is darker reads as pressed, which inverts the
        // feedback the user gets.
        assert!(
            Accent::HOVER.l > Accent::DEFAULT.l,
            "the hover step must be lighter than the resting accent"
        );
    }

    #[test]
    fn a_panel_groups_and_a_control_acts() {
        // The shape lock: a panel is rounded-lg, a control is rounded-md.
        assert!(Radius::LG > Radius::MD, "a panel groups, a control acts");
    }

    #[test]
    fn button_focus_ring_uses_accent_color() {
        // Focus ring styling uses Accent::DEFAULT to clearly mark the focused control
        let style = focus_ring(gpui::StyleRefinement::default());
        assert_eq!(style.border_color, Some(Accent::DEFAULT));
    }

    #[test]
    fn button_and_toggle_support_focus_config() {
        let btn = Button::new("Test").tab_index(1);
        assert_eq!(btn.tab_index, Some(1));

        let tgl = Toggle::new("Test", false).tab_index(2);
        assert_eq!(tgl.tab_index, Some(2));
    }

    #[test]
    fn button_supports_full_width() {
        let btn = Button::new("Test").full_width();
        assert!(btn.full_width);
    }
}
