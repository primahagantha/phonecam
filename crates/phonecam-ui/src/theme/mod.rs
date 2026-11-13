//! The design system, as tokens.
//!
//! These are locked, not preferences â€” they come from the shipped app's
//! `tailwind.config.ts` and `styles.css`, and the reasoning behind each is
//! recorded here because a rewrite that only copies the values will "fix" them
//! into something worse.
//!
//! Three rules carry most of it:
//!
//! **One accent, and it means "act", not "healthy".** `#3ba3ff` marks what is
//! active, selected or actionable. Green is reserved for health and appears only
//! when a live picture is actually flowing: a registered virtual camera with
//! nothing behind it is not a success, and colouring it green once made the
//! loudest element on screen mean the least.
//!
//! **Four radii, no others.** A control is never `rounded-lg` and a panel is
//! never `rounded-md`.
//!
//! **Muted text is slate-400 or lighter.** slate-500 is 3.9:1 on a panel and
//! fails the 4.5:1 AA floor, so it is reserved for borders and rules.

pub mod rules;

use gpui::Hsla;

/// Converts `0xRRGGBB` to `Hsla` at compile time.
///
/// `rgb()` returns `Rgba` and `Hsla::from` is not `const`, so the palette could
/// not stay a set of constants with the obvious approach. A `const fn` keeps
/// every token a compile-time constant, which is what lets the contrast tests
/// read the same values the UI draws with.
pub const fn hsla_from_hex(hex: u32) -> Hsla {
    let r = ((hex >> 16) & 0xFF) as f32 / 255.0;
    let g = ((hex >> 8) & 0xFF) as f32 / 255.0;
    let b = (hex & 0xFF) as f32 / 255.0;

    let max = if r > g { if r > b { r } else { b } } else if g > b { g } else { b };
    let min = if r < g { if r < b { r } else { b } } else if g < b { g } else { b };
    let l = (max + min) / 2.0;
    let delta = max - min;

    if delta == 0.0 {
        return Hsla { h: 0.0, s: 0.0, l, a: 1.0 };
    }

    let s = if l > 0.5 { delta / (2.0 - max - min) } else { delta / (max + min) };

    let h = if max == r {
        ((g - b) / delta + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };

    Hsla { h, s, l, a: 1.0 }
}

/// Surface steps. The shell is one hue, lifted toward the user in a fixed order.
#[derive(Debug, Clone, Copy)]
pub struct ShellColors {
    /// Recessed: wells, the log, the stage's letterbox.
    pub recessed: Hsla,
    /// App background, field fill.
    pub base: Hsla,
    /// Panels.
    pub panel: Hsla,
    /// The status strip, list rows.
    pub raised: Hsla,
    /// Button hover surface, toast, kbd.
    pub hover: Hsla,
    /// Default button fill, toggle track.
    pub control: Hsla,
    /// Borders and rules â€” the only border colour for panels and controls.
    pub border: Hsla,
}

pub const SHELL: ShellColors = ShellColors {
    recessed: hsla_from_hex(0x080b0f),
    base: hsla_from_hex(0x0b0f14),
    panel: hsla_from_hex(0x0e131a),
    raised: hsla_from_hex(0x11161d),
    hover: hsla_from_hex(0x171e27),
    control: hsla_from_hex(0x1f2733),
    border: hsla_from_hex(0x2b3543),
};

/// The one accent. Means "active, selected, or actionable". Never "healthy".
pub struct Accent;
impl Accent {
    pub const DEFAULT: Hsla = hsla_from_hex(0x3ba3ff);
    pub const HOVER: Hsla = hsla_from_hex(0x5db4ff);
}

/// Health colours. These are the only ones allowed to mean "good".
pub struct Status;
impl Status {
    pub const OK: Hsla = hsla_from_hex(0x37c07a);
    pub const WARN: Hsla = hsla_from_hex(0xe0a02a);
    /// Borders, dots and fills only.
    pub const DANGER: Hsla = hsla_from_hex(0xe05a4f);
    /// The lighter step for glyphs: `DANGER` drops to 4.63:1 on a danger-tinted
    /// background and fails AA, while this measures 6.14:1 there.
    pub const DANGER_TEXT: Hsla = hsla_from_hex(0xf0786d);
}

/// Neutral text. slate-500 is deliberately absent: it fails AA on every surface.
pub struct Text;
impl Text {
    pub const PRIMARY: Hsla = hsla_from_hex(0xf1f5f9); // slate-100
    pub const HEADING: Hsla = hsla_from_hex(0xe2e8f0); // slate-200
    pub const BODY: Hsla = hsla_from_hex(0xcbd5e1); // slate-300
    /// The dimmest text that still passes. Used for hints and metadata.
    pub const MUTED: Hsla = hsla_from_hex(0x94a3b8); // slate-400
}

/// Three faces, three jobs. All three ship with Windows 11, so there is no
/// webfont to download and no fallback flash.
pub struct Fonts;
impl Fonts {
    /// Labels, body copy, buttons. Body copy stays here because condensed text
    /// is measurably worse to read in paragraphs.
    pub const SANS: &str = "Segoe UI Variable Text";
    /// A DIN derivative: headings and large numeric readouts, where its
    /// condensed width lets a number be big without the layout paying for it.
    ///
    /// The shipped app asked for `Bahnschrift SemiCondensed`, which is a *face*
    /// inside the Bahnschrift family rather than a family name. DirectWrite
    /// resolves families, so asking for it by that name falls back to Segoe UI
    /// and every heading silently loses its width. GPUI exposes weight and
    /// style but not stretch, so the family is named and the condensed face is
    /// reached through the semibold weight, which is what the headings use.
    pub const DISPLAY: &str = "Bahnschrift";
    /// Anything compared character by character: addresses, ids, log lines.
    pub const MONO: &str = "Cascadia Mono";

    pub const SANS_FALLBACK: &str = "Segoe UI";
    pub const DISPLAY_FALLBACK: &str = "Bahnschrift";
    pub const MONO_FALLBACK: &str = "Cascadia Code";
}

/// Six sizes, each with one job. Tailwind is 16px-rooted, so these are the
/// rem values from the config converted to pixels.
pub struct Type;
impl Type {
    /// 11px â€” chips and metadata only, never body copy.
    pub const TWO_XS: f32 = 11.0;
    /// 13px â€” control labels, list rows, button labels, values.
    pub const TWO_SM: f32 = 13.0;
    /// 12px â€” hints, explanations, secondary values.
    pub const XS: f32 = 12.0;
    /// 15px â€” panel and dialog headings.
    pub const TITLE: f32 = 15.0;
    /// 22px â€” the status headline: the loudest type in the window, because it is
    /// the one thing a person glances at from across a desk.
    pub const STATUS: f32 = 22.0;
    /// 26px â€” the stage's empty state only, which appears when the window is
    /// otherwise blank.
    pub const DISPLAY: f32 = 26.0;
}

/// The shape lock. Four radii, no others.
pub struct Radius;
impl Radius {
    /// Chips and small tags.
    pub const SM: f32 = 2.0;
    /// Interactive controls: buttons, inputs, selects, rows, wells.
    pub const MD: f32 = 6.0;
    /// Surfaces that group things: panels, notices, dialogs, the stage.
    pub const LG: f32 = 8.0;
    /// Pills and dots only.
    pub const FULL: f32 = 9999.0;
}

/// Spacing. Tailwind's scale, in pixels.
pub struct Space;
impl Space {
    pub const XS: f32 = 2.0;
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 6.0;
    pub const LG: f32 = 8.0;
    pub const XL: f32 = 10.0;
    /// Panel body padding, status-strip gap.
    pub const PAD: f32 = 12.0;
    pub const SECTION: f32 = 20.0;
    pub const FIELD_GAP: f32 = 14.0;
    /// The rail's bottom padding, which is what stops the last panel being
    /// clipped by the viewport edge.
    pub const RAIL_BOTTOM: f32 = 40.0;
}

/// The rail's fixed width. It narrows on a small window so the picture keeps
/// its share.
pub const RAIL_WIDTH: f32 = 416.0;
pub const RAIL_WIDTH_NARROW: f32 = 352.0;
/// Below this window width the rail narrows.
pub const NARROW_WINDOW: f32 = 1100.0;

/// The pairing code, at hero size.
///
/// It is the only asset in the product that a second device has to *read*, so it
/// is the largest object in the window rather than a sidebar thumbnail â€” the old
/// build rendered it at business-card size in the rail while 65% of the window
/// showed nothing.
pub const QR_SIZE: usize = 264;

/// Timing that is UI state, not animation.
pub struct Timing;
impl Timing {
    pub const TOAST_MS: u64 = 2200;
    pub const COPIED_LABEL_MS: u64 = 2500;
    pub const SETTINGS_DEBOUNCE_MS: u64 = 250;
    /// Must stay in step with the core's approval wait.
    pub const APPROVAL_WAIT_MS: u64 = 120_000;
    /// The start cooldown: a second `start_stream` inside this window is a
    /// duplicate of one still in flight.
    pub const START_COOLDOWN_MS: u64 = 2500;
    /// How long a camera selection may go unconfirmed before the phone's own
    /// state is trusted again.
    pub const PENDING_CAMERA_GRACE_MS: u64 = 5000;
}

/// The placeholder shown where a value has not been established.
///
/// Two hyphens, not an em-dash: the shipped app bans em/en-dashes in copy, and
/// more importantly this is a claim that *nothing has been measured*, which is
/// different from a measured zero.
pub const EMPTY_VALUE: &str = "--";

/// Chip tones, mapped to colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The absence of a problem: idle, off, not installed.
    Neutral,
    /// Something is genuinely flowing, or succeeded.
    Ok,
    /// Needs attention but is not a failure.
    Warn,
    /// A hard failure.
    Danger,
    /// In-flight and actionable.
    Accent,
}

impl Tone {
    pub fn fg(self) -> Hsla {
        match self {
            Self::Neutral => Text::MUTED,
            Self::Ok => Status::OK,
            Self::Warn => Status::WARN,
            Self::Danger => Status::DANGER_TEXT,
            Self::Accent => Accent::DEFAULT,
        }
    }

    /// The border, at the alpha the shipped notices use.
    pub fn border(self) -> Hsla {
        match self {
            Self::Neutral => Text::MUTED.alpha(0.6),
            Self::Ok => Status::OK.alpha(0.7),
            Self::Warn => Status::WARN.alpha(0.7),
            Self::Danger => Status::DANGER.alpha(0.8),
            Self::Accent => Accent::DEFAULT.alpha(0.7),
        }
    }

    /// The fill, a 10% wash so a chip reads as a stamp rather than a button.
    pub fn fill(self) -> Hsla {
        match self {
            Self::Neutral => Text::MUTED.alpha(0.06),
            Self::Ok => Status::OK.alpha(0.10),
            Self::Warn => Status::WARN.alpha(0.10),
            Self::Danger => Status::DANGER.alpha(0.10),
            Self::Accent => Accent::DEFAULT.alpha(0.10),
        }
    }

    pub fn dot(self) -> Hsla {
        match self {
            Self::Neutral => hsla_from_hex(0x64748b),
            Self::Ok => Status::OK,
            Self::Warn => Status::WARN,
            Self::Danger => Status::DANGER,
            Self::Accent => Accent::DEFAULT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped app pins every text pair it uses against a 4.5:1 floor.
    /// Re-checking the tokens here means a palette edit cannot silently fail AA.
    /// Relative luminance of an `Hsla` colour, composited over the app background.
    fn luminance(c: Hsla) -> f32 {
        let to_linear = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };

        // HSL to RGB, then composite the alpha over the base surface.
        let hue = |p: f32, q: f32, mut t: f32| {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };

        let (r, g, b) = if c.s == 0.0 {
            (c.l, c.l, c.l)
        } else {
            let q = if c.l < 0.5 { c.l * (1.0 + c.s) } else { c.l + c.s - c.l * c.s };
            let p = 2.0 * c.l - q;
            (hue(p, q, c.h + 1.0 / 3.0), hue(p, q, c.h), hue(p, q, c.h - 1.0 / 3.0))
        };

        let base = SHELL.base;
        let base_rgb = hsl_to_rgb(base);
        let blend = |fg: f32, bg: f32| fg * c.a + bg * (1.0 - c.a);
        let r = to_linear(blend(r, base_rgb.0));
        let g = to_linear(blend(g, base_rgb.1));
        let b = to_linear(blend(b, base_rgb.2));
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    fn hsl_to_rgb(c: Hsla) -> (f32, f32, f32) {
        if c.s == 0.0 {
            return (c.l, c.l, c.l);
        }
        let hue = |p: f32, q: f32, mut t: f32| {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            if t < 1.0 / 6.0 {
                p + (q - p) * 6.0 * t
            } else if t < 0.5 {
                q
            } else if t < 2.0 / 3.0 {
                p + (q - p) * (2.0 / 3.0 - t) * 6.0
            } else {
                p
            }
        };
        let q = if c.l < 0.5 { c.l * (1.0 + c.s) } else { c.l + c.s - c.l * c.s };
        let p = 2.0 * c.l - q;
        (hue(p, q, c.h + 1.0 / 3.0), hue(p, q, c.h), hue(p, q, c.h - 1.0 / 3.0))
    }

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let (l1, l2) = (luminance(a), luminance(b));
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }
    /// The const conversion must agree with gpui's own `Rgba -> Hsla`, or every
    /// colour in the app is subtly (or grossly) wrong in a way no contrast test
    /// would catch.
    #[test]
    fn the_const_conversion_matches_gpui() {
        for hex in [
            0x080b0f, 0x0b0f14, 0x0e131a, 0x11161d, 0x171e27, 0x1f2733, 0x2b3543, 0x3ba3ff,
            0x5db4ff, 0x37c07a, 0xe0a02a, 0xe05a4f, 0xf0786d, 0xf1f5f9, 0xe2e8f0, 0xcbd5e1,
            0x94a3b8, 0x64748b, 0xffffff, 0x000000,
        ] {
            let mine = hsla_from_hex(hex);
            let theirs: Hsla = gpui::rgb(hex).into();
            assert!(
                (mine.h - theirs.h).abs() < 0.002,
                "hue for {hex:#08x}: mine {} vs gpui {}",
                mine.h,
                theirs.h
            );
            assert!(
                (mine.s - theirs.s).abs() < 0.002,
                "saturation for {hex:#08x}: mine {} vs gpui {}",
                mine.s,
                theirs.s
            );
            assert!(
                (mine.l - theirs.l).abs() < 0.002,
                "lightness for {hex:#08x}: mine {} vs gpui {}",
                mine.l,
                theirs.l
            );
            assert_eq!(mine.a, theirs.a);
        }
    }
    #[test]
    fn the_accent_passes_aa_on_a_panel() {
        assert!(contrast(Accent::DEFAULT, SHELL.panel) >= 4.5);
    }

    #[test]
    fn danger_text_passes_aa_on_its_own_tint() {
        // `DANGER` itself is 4.63:1 here and fails; the lighter step is what
        // glyphs must use.
        assert!(contrast(Status::DANGER_TEXT, Status::DANGER.alpha(0.10)) >= 4.5);
    }

    #[test]
    fn every_muted_text_colour_passes_on_every_surface() {
        for surface in [SHELL.panel, SHELL.base, SHELL.recessed, SHELL.raised] {
            assert!(
                contrast(Text::MUTED, surface) >= 4.5,
                "muted text fails on a surface"
            );
            assert!(contrast(Text::BODY, surface) >= 4.5);
            assert!(contrast(Text::HEADING, surface) >= 4.5);
        }
    }

    #[test]
    fn status_colours_pass_on_a_panel() {
        for colour in [Status::OK, Status::WARN, Status::DANGER_TEXT] {
            assert!(contrast(colour, SHELL.panel) >= 4.5);
        }
    }

    #[test]
    fn the_primary_button_label_passes_on_the_accent() {
        // A solid accent with near-black text, measured at 7.4:1.
        assert!(contrast(SHELL.recessed, Accent::DEFAULT) >= 4.5);
        assert!(contrast(SHELL.recessed, Accent::HOVER) >= 4.5);
    }

    #[test]
    fn the_default_button_label_passes_on_its_fill() {
        assert!(contrast(Text::PRIMARY, SHELL.control) >= 4.5);
    }

    /// Em-dashes and en-dashes are banned in the shipped copy, a rule that is
    /// enforced by a test there. Pinned here so the placeholder cannot drift.
    #[test]
    fn the_empty_placeholder_is_two_hyphens() {
        assert_eq!(EMPTY_VALUE, "--");
        assert!(!EMPTY_VALUE.contains('\u{2014}'));
        assert!(!EMPTY_VALUE.contains('\u{2013}'));
    }

    #[test]
    fn the_shape_lock_has_four_radii() {
        assert!(Radius::SM < Radius::MD);
        assert!(Radius::MD < Radius::LG);
        assert!(Radius::LG < Radius::FULL);
    }

    #[test]
    fn the_type_scale_is_ordered_and_bounded() {
        // The hierarchy lives between 11px and 26px, which is what a dense
        // console allows.
        assert!(Type::TWO_XS < Type::XS);
        assert!(Type::XS < Type::TWO_SM);
        assert!(Type::TWO_SM < Type::TITLE);
        assert!(Type::TITLE < Type::STATUS);
        assert!(Type::STATUS < Type::DISPLAY);
    }
}












