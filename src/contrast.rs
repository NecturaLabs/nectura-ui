//! High contrast over any theme. [`derive()`] keeps a theme's grounds and its hues, and moves every
//! colour drawn on them to the contrast ratio the reference ([`crate::theme::CONTRAST_REFERENCE`],
//! Midnight at high contrast) keeps against its own ground: lighter on a dark theme, darker on a
//! light one. A colour that already contrasts more stays as it is. Graph lanes keep the theme's
//! own colours: they already meet the 3:1 non-text floor on these grounds and must stay apart.

#![expect(
    clippy::disallowed_methods,
    reason = "high contrast makes the colours of every theme, like the palette module"
)]

use egui::Color32;

use super::theme::{Fields, Fill, Palette, Surfaces, Syntax, Tones, contrast as ratio, luminance};

fn to_linear(v: u8) -> f32 {
    let v = f32::from(v) / 255.0;
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(x: f32) -> u8 {
    let x = x.clamp(0.0, 1.0);
    let v = if x <= 0.003_130_8 {
        12.92 * x
    } else {
        1.055 * x.powf(1.0 / 2.4) - 0.055
    };
    crate::saturate_u8((v * 255.0).round())
}

/// `OKLab` lightness, a and b of `c`.
pub fn oklab(c: Color32) -> [f32; 3] {
    let (r, g, b) = (to_linear(c.r()), to_linear(c.g()), to_linear(c.b()));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Linear `sRGB` of an `OKLab` colour (may fall outside 0..1).
fn linear_rgb(l: f32, a: f32, b: f32) -> [f32; 3] {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
}

/// The colour of `OKLab` lightness `l` at hue `hue`, its chroma `chroma` reduced until it fits
/// `sRGB`.
fn at_lightness(l: f32, chroma: f32, hue: f32) -> Color32 {
    let rgb = |c: f32| linear_rgb(l, c * hue.cos(), c * hue.sin());
    let fits = |c: f32| rgb(c).iter().all(|v| (-1e-4..=1.0 + 1e-4).contains(v));
    let fit = if fits(chroma) {
        chroma
    } else {
        let (mut lo, mut hi) = (0.0, chroma);
        for _ in 0..24 {
            let mid = f32::midpoint(lo, hi);
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    let [r, g, b] = rgb(fit);
    Color32::from_rgb(from_linear(r), from_linear(g), from_linear(b))
}

/// The colour of `OKLab` chroma `chroma` and hue `hue` (radians) at relative luminance `target`.
pub fn at_luminance(chroma: f32, hue: f32, target: f64) -> Color32 {
    at_lightness(lightness_at(chroma, hue, target), chroma, hue)
}

/// The `OKLab` lightness at which `hue_of`'s hue and chroma reach relative luminance `target`.
fn lightness_for(hue_of: Color32, target: f64) -> f32 {
    let [_, a, b] = oklab(hue_of);
    lightness_at(a.hypot(b), b.atan2(a), target)
}

/// The `OKLab` lightness at which chroma `chroma` and hue `hue` reach relative luminance `target`.
fn lightness_at(chroma: f32, hue: f32, target: f64) -> f32 {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..32 {
        let mid = f32::midpoint(lo, hi);
        if luminance(at_lightness(mid, chroma, hue)) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    hi
}

/// `c` with its `OKLab` lightness set to `l`, hue and chroma kept (chroma as far as `sRGB` allows).
fn relit(c: Color32, l: f32) -> Color32 {
    let [_, a, b] = oklab(c);
    at_lightness(l.clamp(0.0, 1.0), a.hypot(b), b.atan2(a))
}

/// `hue_of`'s hue and chroma at relative luminance `target`.
fn with_luminance(hue_of: Color32, target: f64) -> Color32 {
    relit(hue_of, lightness_for(hue_of, target))
}

/// Moves colours drawn on one ground to the ratios the reference keeps on its ground, times a
/// margin.
#[derive(Clone, Copy)]
struct Lift {
    ground: Color32,
    reference_ground: Color32,
    margin: f64,
}

impl Lift {
    fn new(ground: Color32, reference_ground: Color32, margin: f64) -> Self {
        Self {
            ground,
            reference_ground,
            margin,
        }
    }

    /// `base` moved to the ratio `reference` keeps on its ground, away from this ground's side.
    fn color(self, base: Color32, reference: Color32) -> Color32 {
        if ratio(base, self.ground) >= self.want(reference) {
            return base;
        }
        with_luminance(base, self.target(reference))
    }

    fn want(self, reference: Color32) -> f64 {
        ratio(reference, self.reference_ground) * self.margin
    }

    fn fill(self, base: Fill, reference: Fill) -> Fill {
        Fill {
            top: self.color(base.top, reference.top),
            bottom: self.color(base.bottom, reference.bottom),
        }
    }

    /// The relative luminance `reference` asks for on this ground.
    fn target(self, reference: Color32) -> f64 {
        let want = self.want(reference);
        let g = luminance(self.ground);
        let t = if g < 0.18 {
            (g + 0.05) * want - 0.05
        } else {
            (g + 0.05) / want - 0.05
        };
        t.clamp(0.0, 1.0)
    }

    fn option(self, base: Option<Color32>, reference: Option<Color32>) -> Option<Color32> {
        match (base, reference) {
            (Some(b), Some(r)) => Some(self.color(b, r)),
            (b, _) => b,
        }
    }
}

/// `c` moved to the ratio `reference` keeps on `reference_ground`, times `margin`, on `ground`;
/// unchanged when it already reaches it.
pub fn fit(
    c: Color32,
    ground: Color32,
    reference: Color32,
    reference_ground: Color32,
    margin: f64,
) -> Color32 {
    Lift::new(ground, reference_ground, margin).color(c, reference)
}

/// `c` moved, away from `ground`'s side, until it reads on `ground` at `want`; unchanged when it
/// already does. Rounding to 8 bits can land just short, so the target creeps up until it holds.
pub fn clear(c: Color32, ground: Color32, want: f64) -> Color32 {
    let g = luminance(ground);
    let lighter = luminance(c) > g;
    let mut out = c;
    let mut target = want;
    for _ in 0..8 {
        if ratio(out, ground) >= want {
            break;
        }
        let t = if lighter {
            (g + 0.05) * target - 0.05
        } else {
            (g + 0.05) / target - 0.05
        };
        out = with_luminance(c, t.clamp(0.0, 1.0));
        target *= 1.005;
    }
    out
}

/// `s` with its primary fills moved until `on_accent` reads on them at `want`, and its danger
/// fills until `on_danger` does.
fn clear_fills(s: Surfaces, on_accent: Color32, on_danger: Color32, want: f64) -> Surfaces {
    let fill = |f: Fill, ink: Color32| Fill {
        top: clear(f.top, ink, want),
        bottom: clear(f.bottom, ink, want),
    };
    Surfaces {
        primary: fill(s.primary, on_accent),
        primary_hover: fill(s.primary_hover, on_accent),
        primary_pressed: fill(s.primary_pressed, on_accent),
        danger_solid: fill(s.danger_solid, on_danger),
        danger_solid_hover: fill(s.danger_solid_hover, on_danger),
        danger_solid_pressed: fill(s.danger_solid_pressed, on_danger),
        ..s
    }
}

/// Text on a solid fill in every state (`fills`): `own` when it reads at `want` on all of them,
/// else white or black, whichever reads better on the worst.
fn ink_over(own: Color32, fills: &[Color32], want: f64) -> Color32 {
    let worst = |ink: Color32| {
        fills
            .iter()
            .map(|f| ratio(ink, *f))
            .fold(f64::MAX, f64::min)
    };
    if worst(own) >= want {
        return own;
    }
    if worst(Color32::WHITE) >= worst(Color32::BLACK) {
        Color32::WHITE
    } else {
        Color32::BLACK
    }
}

/// The solid fills `pick` names in both styles, top and bottom: the states text on them meets.
fn states(p: &Palette, pick: impl Fn(&Surfaces) -> [Fill; 3]) -> Vec<Color32> {
    [&p.standard, &p.deeper]
        .into_iter()
        .flat_map(pick)
        .flat_map(|f| [f.top, f.bottom])
        .collect()
}

fn primaries(s: &Surfaces) -> [Fill; 3] {
    [s.primary, s.primary_hover, s.primary_pressed]
}

fn dangers(s: &Surfaces) -> [Fill; 3] {
    [s.danger_solid, s.danger_solid_hover, s.danger_solid_pressed]
}

fn surfaces(b: &Surfaces, r: &Surfaces, margin: f64) -> Surfaces {
    let on = Lift::new(b.bg, r.bg, margin);
    let tip = Lift::new(b.tooltip.top, r.tooltip.top, margin);
    let primary = on.fill(b.primary, r.primary);
    Surfaces {
        switch_off: on.color(b.switch_off, r.switch_off),
        border: on.color(b.border, r.border),
        edge: on.color(b.edge, r.edge),
        raised_edge: on.color(b.raised_edge, r.raised_edge),
        overlay_edge: on.color(b.overlay_edge, r.overlay_edge),
        tab_edge: on.color(b.tab_edge, r.tab_edge),
        track_edge: on.color(b.track_edge, r.track_edge),
        pane_edge: on.option(b.pane_edge, r.pane_edge),
        keycap_edge: on.color(b.keycap_edge, r.keycap_edge),
        tooltip_key_edge: tip.color(b.tooltip_key_edge, r.tooltip_key_edge),
        tooltip_key_base: tip.option(b.tooltip_key_base, r.tooltip_key_base),
        on_tooltip_key: tip.color(b.on_tooltip_key, r.on_tooltip_key),
        primary,
        primary_hover: on.fill(b.primary_hover, r.primary_hover),
        primary_pressed: on.fill(b.primary_pressed, r.primary_pressed),
        danger_solid: on.fill(b.danger_solid, r.danger_solid),
        danger_solid_hover: on.fill(b.danger_solid_hover, r.danger_solid_hover),
        danger_solid_pressed: on.fill(b.danger_solid_pressed, r.danger_solid_pressed),
        deck_edge: on.color(b.deck_edge, r.deck_edge),
        pane_header_seam: on.color(b.pane_header_seam, r.pane_header_seam),
        knob_edge: on.option(b.knob_edge, r.knob_edge),
        disabled_edge: on.color(b.disabled_edge, r.disabled_edge),
        ..*b
    }
}

/// `base` at high contrast, measured against `reference`.
pub fn derive(base: &Palette, reference: &Palette) -> Palette {
    let lifted = lift(base, reference, 1.0);
    Palette {
        strict: true,
        // High contrast outlines the selection, so the wash keeps the theme's strength (a stronger
        // one sinks graph lanes under it below 3:1 on a light theme).
        selection_alpha: base.selection_alpha,
        on_accent: ink_over(base.on_accent, &states(&lifted, primaries), 7.0),
        on_danger: ink_over(base.on_danger, &states(&lifted, dangers), 7.0),
        glass_alpha: 0,
        ..lifted
    }
}

/// `base` with every colour drawn on a ground moved to at least the ratio `reference` keeps on
/// its own, times `margin`; white or black text on the primary and danger fills wherever its own
/// falls short of 4.5:1 in any state, and those fills kept where that text reads at 4.5:1 times
/// `margin`.
pub fn lift(base: &Palette, reference: &Palette, margin: f64) -> Palette {
    let (b, r) = (base, reference);
    let on = Lift::new(b.standard.bg, r.standard.bg, margin);
    let tip = Lift::new(b.standard.tooltip.top, r.standard.tooltip.top, margin);
    let standard = surfaces(&b.standard, &r.standard, margin);
    let deeper = surfaces(&b.deeper, &r.deeper, margin);
    let shown = Palette {
        standard,
        deeper,
        ..*b
    };
    // Lifting the fills away from the ground can take them past the point where either ink reads
    // on them; those move back toward the ground until the ink chosen for them does.
    let on_accent = ink_over(b.on_accent, &states(&shown, primaries), 4.5);
    let on_danger = ink_over(b.on_danger, &states(&shown, dangers), 4.5);
    let want = 4.5 * margin;
    let standard = clear_fills(standard, on_accent, on_danger, want);
    let deeper = clear_fills(deeper, on_accent, on_danger, want);
    Palette {
        text: on.color(b.text, r.text),
        text_dim: on.color(b.text_dim, r.text_dim),
        text_faint: on.color(b.text_faint, r.text_faint),
        disabled_text: on.color(b.disabled_text, r.disabled_text),
        accent: on.color(b.accent, r.accent),
        accent2: on.color(b.accent2, r.accent2),
        focus: on.color(b.focus, r.focus),
        selection: on.color(b.selection, r.selection),
        on_accent,
        success: on.color(b.success, r.success),
        warning: on.color(b.warning, r.warning),
        danger: on.color(b.danger, r.danger),
        danger_base: on.color(b.danger_base, r.danger_base),
        on_danger,
        diff_added: on.color(b.diff_added, r.diff_added),
        diff_removed: on.color(b.diff_removed, r.diff_removed),
        tones: Tones {
            accent: on.color(b.tones.accent, r.tones.accent),
            success: on.color(b.tones.success, r.tones.success),
            warning: on.color(b.tones.warning, r.tones.warning),
            danger: on.color(b.tones.danger, r.tones.danger),
        },
        control_border: on.color(b.control_border, r.control_border),
        on_tooltip: tip.color(b.on_tooltip, r.on_tooltip),
        on_tooltip_dim: tip.color(b.on_tooltip_dim, r.on_tooltip_dim),
        syntax: Syntax {
            keyword: on.color(b.syntax.keyword, r.syntax.keyword),
            string: on.color(b.syntax.string, r.syntax.string),
            comment: on.color(b.syntax.comment, r.syntax.comment),
            number: on.color(b.syntax.number, r.syntax.number),
            function: on.color(b.syntax.function, r.syntax.function),
            type_: on.color(b.syntax.type_, r.syntax.type_),
            constant: on.color(b.syntax.constant, r.syntax.constant),
        },
        fields: Fields {
            number: on.color(b.fields.number, r.fields.number),
            author: on.color(b.fields.author, r.fields.author),
            time: on.color(b.fields.time, r.fields.time),
            label: on.color(b.fields.label, r.fields.label),
            comments: on.color(b.fields.comments, r.fields.comments),
        },
        standard,
        deeper,
        ..*b
    }
}
