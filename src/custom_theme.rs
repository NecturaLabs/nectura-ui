//! The custom theme: Midnight, or Daylight for a light one, recoloured from a few choices. The
//! accent hue and intensity recolour the theme's indigo family (accent, focus, selection, the
//! primary button and its tones); the tint gives every neutral (grounds, lines and text) a hue.
//! Each colour keeps its relative luminance, so the base theme's contrast mostly holds as it is;
//! a tint can round two near-black grounds closer, and a recoloured wash blends differently, so
//! the grounds are then kept as far apart as the base keeps them, every colour drawn on a ground
//! is moved to MARGIN beyond the ratio the base keeps ([`super::contrast::lift`]), and every
//! colour drawn as text is moved until it reads on each wash it is drawn over. Status colours,
//! graph lanes and syntax keep the base's hues; only their lightness may move.

use serde::{Deserialize, Serialize};

use super::contrast::{self, at_luminance, oklab};
use super::theme::{
    DAYLIGHT, Fill, MIDNIGHT, Palette, Surfaces, TONE_ALPHA, Tones, contrast as ratio, luminance,
    over,
};

/// Hue of the base themes' indigo family, in degrees: the default accent.
pub const BASE_ACCENT_HUE: u16 = 275;
/// Chroma below which a colour counts as a neutral the tint recolours.
const NEUTRAL: f32 = 0.04;
/// How far from the base accent's hue a colour still belongs to its family, in degrees.
const FAMILY: f32 = 30.0;
/// Chroma of a ground at full tint (Observatory's navy is about 0.025).
const FULL_TINT: f32 = 0.05;
/// How far beyond the base theme's ratios a recoloured theme's foregrounds are moved on its
/// grounds; the washes over them are then checked one by one ([`clear_text`]).
const MARGIN: f64 = 1.04;
/// How far beyond its target text is moved on a wash, so 8-bit rounding cannot take it below.
const ROUNDING: f64 = 1.01;

/// The hues the editor offers as swatches, in degrees, around the colour wheel.
pub const HUES: [(u16, &str); 9] = [
    (28, "Red"),
    (60, "Orange"),
    (95, "Yellow"),
    (150, "Green"),
    (205, "Teal"),
    (250, "Blue"),
    (BASE_ACCENT_HUE, "Indigo"),
    (310, "Violet"),
    (350, "Pink"),
];

/// The user's choices, saved in settings. The default is Midnight's own colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct CustomTheme {
    /// Built on Daylight instead of Midnight.
    pub light: bool,
    /// Hue of the accent, in degrees (0..360).
    pub accent_hue: u16,
    /// The accent's chroma, in percent of the base accent's (20..=140).
    pub accent_intensity: u8,
    /// Hue of the tint on grounds, lines and text, in degrees (0..360).
    pub tint_hue: u16,
    /// How strongly the neutrals take the tint, in percent (0 keeps them neutral).
    pub tint: u8,
}

impl Default for CustomTheme {
    fn default() -> Self {
        Self {
            light: false,
            accent_hue: BASE_ACCENT_HUE,
            accent_intensity: 100,
            tint_hue: BASE_ACCENT_HUE,
            tint: 0,
        }
    }
}

impl CustomTheme {
    pub const ACCENT_INTENSITY: std::ops::RangeInclusive<u8> = 20..=140;
    pub const TINT: std::ops::RangeInclusive<u8> = 0..=100;

    /// The choices with every value inside its range, as read from a settings file.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            accent_hue: self.accent_hue % 360,
            accent_intensity: self.accent_intensity.clamp(
                *Self::ACCENT_INTENSITY.start(),
                *Self::ACCENT_INTENSITY.end(),
            ),
            tint_hue: self.tint_hue % 360,
            tint: self.tint.min(*Self::TINT.end()),
            ..self
        }
    }

    /// The primary button's colour these choices would give at `hue`: an accent swatch.
    pub fn accent_sample(self, hue: u16) -> egui::Color32 {
        let primary = self.base().standard.primary.top;
        let [_, a, b] = oklab(primary);
        let chroma = a.hypot(b) * f32::from(self.accent_intensity) / 100.0;
        at_luminance(chroma, f32::from(hue).to_radians(), luminance(primary))
    }

    /// A tint swatch for `hue`: stronger than any ground, so its hue reads.
    pub fn tint_sample(hue: u16) -> egui::Color32 {
        at_luminance(0.09, f32::from(hue).to_radians(), 0.12)
    }

    /// The theme it starts from.
    pub fn base(self) -> &'static Palette {
        if self.light { &DAYLIGHT } else { &MIDNIGHT }
    }

    /// The palette these choices make.
    pub fn palette(self) -> Palette {
        let base = self.base();
        let named = Palette {
            name: "Custom",
            concept: if self.light {
                "Your own colours on Daylight."
            } else {
                "Your own colours on Midnight."
            },
            ..*base
        };
        let recolour = Recolour::new(self, base);
        if recolour.accent.is_none() && recolour.tint.is_none() {
            return named;
        }
        let recoloured = keep_steps(&Palette {
            standard: recolour.surfaces(&base.standard),
            deeper: recolour.surfaces(&base.deeper),
            text: recolour.color(base.text),
            text_dim: recolour.color(base.text_dim),
            text_faint: recolour.color(base.text_faint),
            disabled_text: recolour.color(base.disabled_text),
            accent: recolour.color(base.accent),
            accent2: recolour.color(base.accent2),
            focus: recolour.color(base.focus),
            selection: recolour.color(base.selection),
            on_accent: recolour.color(base.on_accent),
            tones: Tones {
                accent: recolour.color(base.tones.accent),
                ..base.tones
            },
            control_border: recolour.color(base.control_border),
            on_tooltip: recolour.color(base.on_tooltip),
            on_tooltip_dim: recolour.color(base.on_tooltip_dim),
            hover_ink: recolour.color(base.hover_ink),
            primary_shadow: recolour.color(base.primary_shadow),
            ..named
        });
        let lifted = contrast::lift(&recoloured, base, MARGIN);
        let ground = lifted.standard.bg;
        clear_text(&Palette {
            lanes: std::array::from_fn(|i| {
                contrast::fit(
                    lifted.lanes[i],
                    ground,
                    base.lanes[i],
                    base.standard.bg,
                    MARGIN,
                )
            }),
            ..lifted
        })
    }
}

/// CIE L* of `c`, the lightness the surface steps are measured in.
fn lightness(c: egui::Color32) -> f64 {
    let y = luminance(c);
    if y > 216.0 / 24_389.0 {
        116.0 * y.cbrt() - 16.0
    } else {
        y * 24_389.0 / 27.0
    }
}

/// `c` moved, on its own side of `from`, until it is `min` L* apart from it.
fn step_from(c: egui::Color32, from: egui::Color32, min: f64) -> egui::Color32 {
    let lighter = lightness(c) >= lightness(from);
    let [_, a, b] = oklab(c);
    let mut out = c;
    let mut gap = min;
    for _ in 0..8 {
        if (lightness(out) - lightness(from)).abs() >= min {
            break;
        }
        let l = lightness(from) + if lighter { gap } else { -gap };
        let y = ((l.clamp(0.0, 100.0) + 16.0) / 116.0).powi(3);
        out = at_luminance(a.hypot(b), b.atan2(a), y);
        gap += 0.2;
    }
    out
}

/// The grounds as far apart as the base keeps them: Standard's panel 2.5 L* from its panes and
/// Deeper's cards 4 L* from the window. A tint can round two near-black grounds closer.
fn keep_steps(p: &Palette) -> Palette {
    let panel = step_from(p.standard.panel, p.standard.pane, 2.6);
    Palette {
        standard: Surfaces {
            panel,
            // Standard's deck is the panel itself.
            deck: if p.standard.deck == Fill::flat(p.standard.panel) {
                Fill::flat(panel)
            } else {
                p.standard.deck
            },
            ..p.standard
        },
        deeper: Surfaces {
            pane: step_from(p.deeper.pane, p.deeper.bg, 4.1),
            ..p.deeper
        },
        ..*p
    }
}

/// Every ground and wash text is drawn on in both styles: the grounds, their hover, pressed and
/// selected states, fields, tracks, Dense's zebra rows, the diff rows and the warning wash.
fn washes(p: &Palette) -> Vec<egui::Color32> {
    let mut out = Vec::new();
    for s in [&p.standard, &p.deeper] {
        out.extend([s.field, s.track, over(p.zebra(), s.pane)]);
        for g in [s.bg, s.pane, s.panel, s.elevated, s.overlay] {
            let selected = over(p.selected(), g);
            out.extend([
                g,
                over(p.hover(), g),
                over(p.hover_strong(), g),
                over(p.pressed(), g),
                selected,
                over(p.hover(), selected),
            ]);
        }
        for d in [s.added_bg, s.removed_bg] {
            out.extend([d, over(p.selected(), d), over(p.hover(), d)]);
        }
        out.extend([
            s.added_gutter,
            s.removed_gutter,
            over(p.warning_tint(), s.pane),
        ]);
    }
    out
}

/// The status tints over the grounds they are drawn on: `own`'s alone, or with `None` all four.
fn tints(p: &Palette, own: Option<egui::Color32>) -> Vec<egui::Color32> {
    let colours = own.map_or_else(
        || vec![p.accent, p.success, p.warning, p.danger],
        |c| vec![c],
    );
    let mut out = Vec::new();
    for s in [&p.standard, &p.deeper] {
        for g in [s.pane, s.panel, s.elevated, s.overlay] {
            out.extend(
                colours
                    .iter()
                    .map(|c| over(Palette::tint(*c, TONE_ALPHA), g)),
            );
        }
    }
    out
}

/// `c` moved until it reads at `want` on every colour in `grounds`.
fn clear_over(c: egui::Color32, grounds: &[egui::Color32], want: f64) -> egui::Color32 {
    let mut c = c;
    for _ in 0..8 {
        let Some(worst) = grounds
            .iter()
            .copied()
            .min_by(|x, y| ratio(c, *x).total_cmp(&ratio(c, *y)))
        else {
            break;
        };
        if ratio(c, worst) >= want {
            break;
        }
        c = contrast::clear(c, worst, want);
    }
    c
}

/// Every colour drawn as text moved until it reads at its target (7:1 for text, 4.5:1 for the
/// rest) on every wash it is drawn over. The lift measures against the window ground alone, and a
/// recoloured wash, a vivid selection most of all, blends a few percent differently.
fn clear_text(p: &Palette) -> Palette {
    let washes = washes(p);
    let with_tints = [washes.clone(), tints(p, None)].concat();
    let text = |c, want: f64| clear_over(c, &with_tints, want * ROUNDING);
    let colour = |c| {
        let grounds = [washes.clone(), tints(p, Some(c))].concat();
        clear_over(c, &grounds, 4.5 * ROUNDING)
    };
    Palette {
        text: text(p.text, 7.0),
        text_dim: text(p.text_dim, 4.5),
        text_faint: text(p.text_faint, 4.5),
        accent: colour(p.accent),
        accent2: colour(p.accent2),
        success: colour(p.success),
        warning: colour(p.warning),
        danger: colour(p.danger),
        diff_added: colour(p.diff_added),
        diff_removed: colour(p.diff_removed),
        ..*p
    }
}

/// One set of choices applied to one base theme's colours.
struct Recolour {
    /// The accent's hue in radians and its chroma scale, or `None` when the base accent stays.
    accent: Option<(f32, f32)>,
    /// The tint's a and b at full strength, and its strength (0..=1), or `None` for no tint.
    tint: Option<([f32; 2], f32)>,
    /// The base accent's hue in radians.
    family_hue: f32,
    dark: bool,
}

impl Recolour {
    fn new(choice: CustomTheme, base: &Palette) -> Self {
        let [_, a, b] = oklab(base.standard.primary.top);
        let family_hue = b.atan2(a);
        let accent_hue = f32::from(choice.accent_hue).to_radians();
        let scale = f32::from(choice.accent_intensity) / 100.0;
        let accent = (choice.accent_hue != BASE_ACCENT_HUE || choice.accent_intensity != 100)
            .then_some((accent_hue, scale));
        let tint_hue = f32::from(choice.tint_hue).to_radians();
        let tint = (choice.tint > 0).then(|| {
            (
                [FULL_TINT * tint_hue.cos(), FULL_TINT * tint_hue.sin()],
                f32::from(choice.tint) / 100.0,
            )
        });
        Self {
            accent,
            tint,
            family_hue,
            dark: base.dark,
        }
    }

    /// `c` recoloured at its own luminance. Translucent colours (shadows, washes) stay.
    fn color(&self, c: egui::Color32) -> egui::Color32 {
        if c.a() < 255 {
            return c;
        }
        let [l, a, b] = oklab(c);
        let chroma = a.hypot(b);
        let (na, nb) = if chroma < NEUTRAL {
            let Some(([ta, tb], strength)) = self.tint else {
                return c;
            };
            // Grounds take the tint fully; text and lines on the far side of the ladder less.
            let weight = if self.dark { 1.0 - l } else { l }.clamp(0.0, 1.0) * strength;
            (a + (ta - a) * weight, b + (tb - b) * weight)
        } else {
            let Some((hue, scale)) = self.accent else {
                return c;
            };
            let offset = signed_angle(b.atan2(a), self.family_hue);
            if offset.abs() > FAMILY.to_radians() {
                return c;
            }
            // Keeps the colour's offset from the family's centre, so focus and accent stay apart.
            let h = hue + offset;
            let chroma = chroma * scale;
            (chroma * h.cos(), chroma * h.sin())
        };
        at_luminance(na.hypot(nb), nb.atan2(na), luminance(c))
    }

    fn fill(&self, f: Fill) -> Fill {
        Fill {
            top: self.color(f.top),
            bottom: self.color(f.bottom),
        }
    }

    fn option(&self, c: Option<egui::Color32>) -> Option<egui::Color32> {
        c.map(|c| self.color(c))
    }

    fn surfaces(&self, s: &Surfaces) -> Surfaces {
        Surfaces {
            bg: self.color(s.bg),
            panel: self.color(s.panel),
            pane: self.color(s.pane),
            elevated: self.color(s.elevated),
            overlay: self.color(s.overlay),
            field: self.color(s.field),
            track: self.color(s.track),
            switch_off: self.color(s.switch_off),
            badge: self.color(s.badge),
            border: self.color(s.border),
            edge: self.color(s.edge),
            raised_edge: self.color(s.raised_edge),
            overlay_edge: self.color(s.overlay_edge),
            tab_edge: self.color(s.tab_edge),
            track_edge: self.color(s.track_edge),
            pane_edge: self.option(s.pane_edge),
            keycap_edge: self.color(s.keycap_edge),
            raised: self.fill(s.raised),
            raised_hover: self.fill(s.raised_hover),
            raised_pressed: self.fill(s.raised_pressed),
            thumb: self.fill(s.thumb),
            keycap: self.fill(s.keycap),
            tab: self.fill(s.tab),
            tooltip: self.fill(s.tooltip),
            tooltip_key: self.color(s.tooltip_key),
            tooltip_key_edge: self.color(s.tooltip_key_edge),
            tooltip_key_base: self.option(s.tooltip_key_base),
            on_tooltip_key: self.color(s.on_tooltip_key),
            knob: self.fill(s.knob),
            primary: self.fill(s.primary),
            primary_hover: self.fill(s.primary_hover),
            primary_pressed: self.fill(s.primary_pressed),
            deck: self.fill(s.deck),
            deck_edge: self.color(s.deck_edge),
            pane_header: self.color(s.pane_header),
            pane_header_seam: self.color(s.pane_header_seam),
            knob_edge: self.option(s.knob_edge),
            disabled_fill: self.color(s.disabled_fill),
            disabled_edge: self.color(s.disabled_edge),
            // Danger fills, diff backgrounds and status colours keep their meaning.
            ..*s
        }
    }
}

/// `a - b` in radians, wrapped to -π..π.
fn signed_angle(a: f32, b: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (a - b + PI).rem_euclid(TAU) - PI
}
