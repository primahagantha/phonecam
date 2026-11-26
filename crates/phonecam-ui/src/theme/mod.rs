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

    let max = if r > g {
        if r > b {
            r
        } else {
            b
        }
    } else if g > b {
        g
    } else {
        b
    };
    let min = if r < g {
        if r < b {
            r
        } else {
            b
        }
    } else if g < b {
        g
    } else {
        b
    };
    let l = (max + min) / 2.0;
    let delta = max - min;

    if delta == 0.0 {
        return Hsla {
            h: 0.0,
            s: 0.0,
            l,
            a: 1.0,
        };
    }

    let s = if l > 0.5 {
        delta / (2.0 - max - min)
    } else {
        delta / (max + min)
    };

    let h = if max == r {
        ((g - b) / delta + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };

    Hsla { h, s, l, a: 1.0 }
}

/// Surface steps. The studio shell is one refined dark slate hue lifted toward
/// the operator in deliberate layers, with separate subtle structural borders
/// and crisp interactive control borders.
#[derive(Debug, Clone, Copy)]
pub struct ShellColors {
    /// Deepest obsidian backdrop for the video stage, recessed tracks, and log wells.
    pub stage: Hsla,
    /// Recessed wells, segmented tab tracks, and log containers.
    pub recessed: Hsla,
    /// App canvas background.
    pub base: Hsla,
    /// Primary panel / inspector / studio card surface.
    pub panel: Hsla,
    /// Elevated panels, active tab pills, and floating surfaces.
    pub panel_elevated: Hsla,
    /// Raised surface alias matching `panel_elevated` for subtle card headers and rows.
    pub raised: Hsla,
    /// Default interactive control fill and toggle track.
    pub control: Hsla,
    /// Hover surface for interactive buttons, rows, and toasts.
    pub hover: Hsla,
    /// Subtle structural dividers and card outlines (never bright wireframe).
    pub border: Hsla,
    /// Crisp borders for interactive controls (buttons, inputs, sliders).
    pub control_border: Hsla,
    /// Hover border for interactive controls.
    pub control_border_hover: Hsla,
}

pub const SHELL: ShellColors = ShellColors {
    stage: hsla_from_hex(0x06080c),
    recessed: hsla_from_hex(0x0a0e14),
    base: hsla_from_hex(0x090c10),
    panel: hsla_from_hex(0x10151c),
    panel_elevated: hsla_from_hex(0x151c26),
    raised: hsla_from_hex(0x151c26),
    control: hsla_from_hex(0x18212c),
    hover: hsla_from_hex(0x212d3c),
    border: hsla_from_hex(0x1d2633),
    control_border: hsla_from_hex(0x2d3a4c),
    control_border_hover: hsla_from_hex(0x44566e),
};

/// The one accent. Means "active, selected, or actionable". Never "healthy".
pub struct Accent;
impl Accent {
    /// Focus rings, selected outlines, slider fills, and accent text on dark surfaces.
    pub const DEFAULT: Hsla = hsla_from_hex(0x3b82f6);
    /// Lighter interactive hover highlight for accent glyphs and outlines.
    pub const HOVER: Hsla = hsla_from_hex(0x60a5fa);
    /// Solid primary button fill engineered for >= 4.5:1 AA contrast with white text.
    pub const SOLID: Hsla = hsla_from_hex(0x2563eb);
    /// Primary button hover fill (>= 4.5:1 AA contrast with white text).
    pub const SOLID_HOVER: Hsla = hsla_from_hex(0x2c6bed);
}

/// Health colours. These are the only ones allowed to mean "good".
pub struct Status;
impl Status {
    pub const OK: Hsla = hsla_from_hex(0x37c07a);
    pub const WARN: Hsla = hsla_from_hex(0xe0a02a);
    /// Borders, dots and fills only.
    pub const DANGER: Hsla = hsla_from_hex(0xe05a4f);
    /// The lighter step for glyphs: `DANGER` drops below AA on a danger-tinted
    /// background, while this stays comfortably above 4.5:1.
    pub const DANGER_TEXT: Hsla = hsla_from_hex(0xf0786d);
}

/// Neutral text. slate-500 is deliberately absent: it fails AA on every surface.
pub struct Text;
impl Text {
    pub const PRIMARY: Hsla = hsla_from_hex(0xf8fafc); // slate-50
    pub const HEADING: Hsla = hsla_from_hex(0xe2e8f0); // slate-200
    pub const BODY: Hsla = hsla_from_hex(0xcbd5e1); // slate-300
    /// The dimmest text that still passes. Used for hints and metadata.
    pub const MUTED: Hsla = hsla_from_hex(0x94a3b8); // slate-400
}

/// Bundled Inter and JetBrains Mono faces with zero missing-font fallbacks.
pub struct Fonts;
impl Fonts {
    /// Primary studio sans-serif face bundled via `bundled_fonts()`.
    pub const SANS: &str = "Inter";
    /// Display headings and numeric readouts (same clean Inter family, semibold/bold).
    pub const DISPLAY: &str = "Inter";
    /// Monospace face bundled via `bundled_fonts()` for telemetry, endpoints, and logs.
    pub const MONO: &str = "JetBrains Mono";

    #[cfg(windows)]
    pub const SANS_FALLBACKS: [&str; 1] = ["Segoe UI"];
    #[cfg(not(windows))]
    pub const SANS_FALLBACKS: [&str; 1] = ["sans-serif"];

    #[cfg(windows)]
    pub const MONO_FALLBACKS: [&str; 1] = ["Consolas"];
    #[cfg(not(windows))]
    pub const MONO_FALLBACKS: [&str; 1] = ["monospace"];

    /// Platform-native fallback stack so DirectWrite never logs missing-font errors.
    pub fn fallbacks_for(family: &str) -> gpui::FontFallbacks {
        let list = if family == Self::MONO {
            Self::MONO_FALLBACKS
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        } else {
            Self::SANS_FALLBACKS
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        };
        gpui::FontFallbacks::from_fonts(list)
    }

    /// Standard UI sans font with platform-native fallback.
    pub fn sans() -> gpui::Font {
        gpui::Font {
            family: Self::SANS.into(),
            features: gpui::FontFeatures::default(),
            fallbacks: Some(Self::fallbacks_for(Self::SANS)),
            weight: gpui::FontWeight::NORMAL,
            style: gpui::FontStyle::Normal,
        }
    }

    /// Display heading font with semibold weight and platform-native fallback.
    pub fn display() -> gpui::Font {
        gpui::Font {
            family: Self::DISPLAY.into(),
            features: gpui::FontFeatures::default(),
            fallbacks: Some(Self::fallbacks_for(Self::DISPLAY)),
            weight: gpui::FontWeight::SEMIBOLD,
            style: gpui::FontStyle::Normal,
        }
    }

    /// Monospace font with platform-native fallback.
    pub fn mono() -> gpui::Font {
        gpui::Font {
            family: Self::MONO.into(),
            features: gpui::FontFeatures::default(),
            fallbacks: Some(Self::fallbacks_for(Self::MONO)),
            weight: gpui::FontWeight::NORMAL,
            style: gpui::FontStyle::Normal,
        }
    }
}

/// Six sizes, each with one job.
pub struct Type;
impl Type {
    /// 11px — chips and metadata only, never body copy.
    pub const TWO_XS: f32 = 11.0;
    /// 13px — control labels, list rows, button labels, values.
    pub const TWO_SM: f32 = 13.0;
    /// 12px — hints, explanations, secondary values.
    pub const XS: f32 = 12.0;
    /// 15px — panel and dialog headings.
    pub const TITLE: f32 = 15.0;
    /// 20px — prominent KPI tiles and studio card headlines.
    pub const STATUS: f32 = 20.0;
    /// 24px — hero empty-state heading.
    pub const DISPLAY: f32 = 24.0;

    /// Line heights matching the type scale for multi-line prose readability.
    pub const LINE_XS: f32 = 18.0;
    pub const LINE_TWO_SM: f32 = 20.0;
    pub const LINE_TITLE: f32 = 22.0;
}

/// Deliberate studio radius scale. Four radii, no arbitrary intermediates.
pub struct Radius;
impl Radius {
    /// Small controls, badges, and chips (6px).
    pub const SM: f32 = 6.0;
    /// Cards, interactive buttons, inputs, and recessed wells (10px).
    pub const MD: f32 = 10.0;
    /// Panels, stage container, studio cards, modals, and drawers (14px).
    pub const LG: f32 = 14.0;
    /// Status pills, segmented switches, and dots only.
    pub const PILL: f32 = 9999.0;
    /// Alias for `PILL`.
    pub const FULL: f32 = 9999.0;
}

/// Spacing scale in pixels.
pub struct Space;
impl Space {
    pub const XS: f32 = 2.0;
    pub const SM: f32 = 4.0;
    pub const MD: f32 = 6.0;
    pub const LG: f32 = 8.0;
    pub const XL: f32 = 10.0;
    /// Panel body padding, header spacing.
    pub const PAD: f32 = 14.0;
    pub const SECTION: f32 = 18.0;
    pub const FIELD_GAP: f32 = 14.0;
    /// The rail's bottom padding so the final control never clips at the viewport edge.
    pub const RAIL_BOTTOM: f32 = 36.0;
}

/// The modern right inspector rail's fixed width.
pub const RAIL_WIDTH: f32 = 368.0;
pub const RAIL_WIDTH_NARROW: f32 = 336.0;
/// Below this window width the rail narrows so the preview stage keeps priority.
pub const NARROW_WINDOW: f32 = 1080.0;

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

    /// Subtle semantic border for chips, status pills, and slim alerts.
    pub fn border(self) -> Hsla {
        match self {
            Self::Neutral => SHELL.control_border,
            Self::Ok => Status::OK.alpha(0.45),
            Self::Warn => Status::WARN.alpha(0.45),
            Self::Danger => Status::DANGER.alpha(0.55),
            Self::Accent => Accent::DEFAULT.alpha(0.45),
        }
    }

    /// Subtle wash so a chip or pill reads as a refined stamp rather than a heavy box.
    pub fn fill(self) -> Hsla {
        match self {
            Self::Neutral => Text::MUTED.alpha(0.08),
            Self::Ok => Status::OK.alpha(0.11),
            Self::Warn => Status::WARN.alpha(0.11),
            Self::Danger => Status::DANGER.alpha(0.11),
            Self::Accent => Accent::DEFAULT.alpha(0.12),
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
            let q = if c.l < 0.5 {
                c.l * (1.0 + c.s)
            } else {
                c.l + c.s - c.l * c.s
            };
            let p = 2.0 * c.l - q;
            (
                hue(p, q, c.h + 1.0 / 3.0),
                hue(p, q, c.h),
                hue(p, q, c.h - 1.0 / 3.0),
            )
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
        let q = if c.l < 0.5 {
            c.l * (1.0 + c.s)
        } else {
            c.l + c.s - c.l * c.s
        };
        let p = 2.0 * c.l - q;
        (
            hue(p, q, c.h + 1.0 / 3.0),
            hue(p, q, c.h),
            hue(p, q, c.h - 1.0 / 3.0),
        )
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
        // Solid studio blue with crisp white text passes AA (>= 4.5:1),
        // and dark text on Accent::DEFAULT also passes AA.
        assert!(contrast(Text::PRIMARY, Accent::SOLID) >= 4.5);
        assert!(contrast(Text::PRIMARY, Accent::SOLID_HOVER) >= 4.5);
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
        assert_eq!(Radius::SM, 6.0);
        assert_eq!(Radius::MD, 10.0);
        assert_eq!(Radius::LG, 14.0);
        assert_eq!(Radius::PILL, Radius::FULL);
        assert!(Radius::SM < Radius::MD);
        assert!(Radius::MD < Radius::LG);
        assert!(Radius::LG < Radius::FULL);
    }

    #[test]
    fn the_type_scale_is_ordered_and_bounded() {
        // The hierarchy lives between 11px and 24px, keeping a compact studio console.
        assert!(Type::TWO_XS < Type::XS);
        assert!(Type::XS < Type::TWO_SM);
        assert!(Type::TWO_SM < Type::TITLE);
        assert!(Type::TITLE < Type::STATUS);
        assert!(Type::STATUS < Type::DISPLAY);
    }

    #[test]
    fn studio_surface_and_border_hierarchy_is_layered_and_focus_passes_contrast() {
        // Layered dark surfaces step gently upward from stage -> base -> panel -> elevated -> control -> hover.
        assert!(SHELL.stage.l < SHELL.base.l);
        assert!(SHELL.base.l < SHELL.panel.l);
        assert!(SHELL.panel.l < SHELL.panel_elevated.l);
        assert!(SHELL.panel_elevated.l < SHELL.control.l);
        assert!(SHELL.control.l < SHELL.hover.l);
        // Subtle structural dividers stay darker than interactive control borders.
        assert!(SHELL.border.l < SHELL.control_border.l);
        assert!(SHELL.control_border.l < SHELL.control_border_hover.l);
        // Focus ring (`Accent::DEFAULT`) exceeds 3.0:1 non-text contrast on every surface.
        assert!(contrast(Accent::DEFAULT, SHELL.panel) >= 3.0);
        assert!(contrast(Accent::DEFAULT, SHELL.control) >= 3.0);
    }

    #[test]
    fn fonts_wire_cross_platform_fallbacks() {
        let sans = Fonts::sans();
        let display = Fonts::display();
        let mono = Fonts::mono();

        assert_eq!(Fonts::SANS, "Inter");
        assert_eq!(Fonts::DISPLAY, "Inter");
        assert_eq!(Fonts::MONO, "JetBrains Mono");

        let sans_list = sans.fallbacks.expect("sans fallbacks wired");
        let display_list = display.fallbacks.expect("display fallbacks wired");
        let mono_list = mono.fallbacks.expect("mono fallbacks wired");

        #[cfg(windows)]
        {
            assert_eq!(sans_list.fallback_list(), &["Segoe UI"]);
            assert_eq!(display_list.fallback_list(), &["Segoe UI"]);
            assert_eq!(mono_list.fallback_list(), &["Consolas"]);
            for forbidden in ["Liberation Sans", "Noto Sans", "DejaVu Sans Mono"] {
                assert!(!sans_list.fallback_list().contains(&forbidden.to_string()));
                assert!(!mono_list.fallback_list().contains(&forbidden.to_string()));
            }
        }
        #[cfg(not(windows))]
        {
            assert_eq!(sans_list.fallback_list(), &["sans-serif"]);
            assert_eq!(display_list.fallback_list(), &["sans-serif"]);
            assert_eq!(mono_list.fallback_list(), &["monospace"]);
        }
    }
}
