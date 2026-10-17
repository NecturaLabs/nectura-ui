//! Small labels: [`Badge`], [`CountBadge`], [`ChangeBadge`], [`RefPill`] and [`RefOverflow`],
//! [`Keycap`] and [`KeyChord`], [`StatusDot`], [`DiffStat`] and the interactive status [`Chip`]
//! (design spec 4.17, 4.18).
//!
//! Everything here but [`Chip`] (and a ref pill given `.clickable()`) is read-only: it senses
//! hover for its tooltip and reports itself to screen readers as a label.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Galley, Id, Painter, Pos2, Rect, Response, Sense,
    Shape, Ui, Vec2, Widget, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::Shortcut;
use super::overlay::TooltipExt;
use super::{ForceState, Interact, Tone, paint};
use crate::motion;
use crate::theme::{self, FULL, Fill, Palette};

/// A radius that `FULL` turns into half the height.
pub fn radius_for(radius: u8, height: f32) -> CornerRadius {
    if radius == FULL {
        CornerRadius::same(crate::saturate_u8(
            (height / 2.0).round().min(f32::from(u8::MAX)),
        ))
    } else {
        CornerRadius::same(radius)
    }
}

// Component geometry from spec 4.18 that no theme token matches (the spacing scale steps by 4).
/// Badge and keycap text inset either side ("padding 5").
const CAP_PAD: f32 = 5.0;
/// Space between the parts of a `KeyChord`'s alternatives.
const GAP: f32 = theme::XS;
/// `KeyChord`: caps in a chord 3 apart, chords and alternatives 6 apart.
const CHORD_GAP: f32 = 3.0;
const SEQUENCE_GAP: f32 = 6.0;
/// `StatusDot`: a 2 pt ring of the surface behind (its diameter is `Metrics::status_dot`).
const DOT_RING: f32 = 2.0;
/// `DiffStat`: `+n` and `−n` 6 apart, then 6 before five 6 pt blocks 2 apart.
const STAT_GAP: f32 = 6.0;
const BLOCK: f32 = 6.0;
const BLOCK_GAP: f32 = 2.0;
const BLOCK_RADIUS: u8 = 2;

pub fn layout(ui: &Ui, text: String, font: FontId) -> Arc<Galley> {
    ui.painter()
        .layout_no_wrap(text, font, Color32::PLACEHOLDER)
}

/// A read-only label's response: its tooltip and its AccessKit label.
pub fn label_response(response: Response, label: &str, tooltip: Option<&str>) -> Response {
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, label));
    match tooltip {
        Some(t) => response.kit_tooltip(t).show(),
        None => response,
    }
}

// -------------------------------------------------------------------------------------- Badge

/// A small rounded count or status label. Neutral by default; a [`Tone`] tints it.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Badge {
    text: String,
    tone: Tone,
    hue: Option<Color32>,
    mono: bool,
    strong: bool,
    muted: bool,
    tooltip: Option<String>,
}

impl Badge {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            tone: Tone::Neutral,
            hue: None,
            mono: false,
            strong: false,
            muted: false,
            tooltip: None,
        }
    }

    /// A neutral badge in secondary text (`text_dim`), for a note that should sit behind the
    /// words beside it. Ignored by a tinted or strong badge.
    pub fn muted(mut self) -> Self {
        self.muted = true;
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// A tint of `color` (a palette colour that is not a tone's, such as a field's) with text
    /// worked out to read on it, like a tone's badge. Ignored by a strong badge.
    pub fn hue(mut self, color: Color32) -> Self {
        self.hue = Some(color);
        self
    }

    /// Monospaced digits (counts, SHAs).
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }

    /// The `primary` fill with `on_accent` text (the view switcher's counts); a danger badge
    /// keeps its tone as `danger_solid` with `on_danger` text.
    pub fn strong(mut self) -> Self {
        self.strong = true;
        self
    }

    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }

    fn font(&self) -> FontId {
        let mono = self.mono || self.tone == Tone::Accent;
        let family = if mono {
            FontFamily::Monospace
        } else {
            theme::semibold()
        };
        FontId::new(theme::metrics().micro_size, family)
    }

    /// The badge's fill and text colours.
    pub fn colors(&self, p: &Palette) -> (Fill, Color32) {
        let s = p.s();
        if self.strong {
            return match self.tone {
                Tone::Danger => (s.danger_solid, p.on_danger),
                _ => (s.primary, p.on_accent),
            };
        }
        if let Some(c) = self.hue {
            let fill = Palette::badge_fill(c);
            return (Fill::flat(fill), p.ink(c, fill));
        }
        match self.tone {
            Tone::Neutral => (
                Fill::flat(s.badge),
                if self.muted { p.text_dim } else { p.text },
            ),
            Tone::Danger => {
                let fill = p.danger_badge_fill();
                (Fill::flat(fill), p.ink(p.danger, fill))
            }
            tone => {
                let c = tone.color(p);
                let fill = Palette::badge_fill(c);
                (Fill::flat(fill), p.ink(c, fill))
            }
        }
    }

    /// Its size: `badge_height` tall, at least as wide, 5 pt either side of the text.
    pub fn measure(&self, ui: &Ui) -> Vec2 {
        let m = theme::metrics();
        let g = layout(ui, self.text.clone(), self.font());
        vec2(
            (g.size().x + 2.0 * CAP_PAD).max(m.badge_height),
            m.badge_height,
        )
    }

    /// Paints the badge in `rect` (sized by [`Badge::measure`]).
    pub fn paint(&self, ui: &Ui, rect: Rect) {
        let p = theme::palette(ui.ctx());
        let (fill, ink) = self.colors(p);
        let g = layout(ui, self.text.clone(), self.font());
        let painter = ui.painter();
        let radius = radius_for(FULL, theme::metrics().badge_height);
        paint::fill(painter, rect, radius, fill);
        painter.galley(rect.center() - g.size() / 2.0, g, ink);
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let size = self.measure(ui);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui, rect);
        }
        label_response(response, &self.text, self.tooltip.as_deref())
    }
}

impl Widget for Badge {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// A [`Badge`] with a count: mono digits, capped at `99+`.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct CountBadge {
    badge: Badge,
}

impl CountBadge {
    pub fn new(count: usize) -> Self {
        Self {
            badge: Badge::new(count_text(count)).mono(),
        }
    }
    pub fn tone(mut self, tone: Tone) -> Self {
        self.badge = self.badge.tone(tone);
        self
    }
    pub fn strong(mut self) -> Self {
        self.badge = self.badge.strong();
        self
    }
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.badge = self.badge.tooltip(text);
        self
    }
    pub fn show(self, ui: &mut Ui) -> Response {
        self.badge.show(ui)
    }
}

impl Widget for CountBadge {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// A count as a badge shows it.
pub fn count_text(count: usize) -> String {
    if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    }
}

// ---------------------------------------------------------------------------- Keycap, KeyChord

/// One key as a cap: `Ctrl`, `P`, `↵`.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Keycap {
    label: String,
}

impl Keycap {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }

    fn galley(&self, painter: &Painter) -> Arc<Galley> {
        let m = theme::metrics();
        painter.layout_no_wrap(
            self.label.clone(),
            FontId::new(m.chord_size, FontFamily::Monospace),
            Color32::PLACEHOLDER,
        )
    }

    pub fn measure(&self, ui: &Ui) -> Vec2 {
        self.measure_on(ui.painter())
    }

    /// [`Keycap::measure`] for code that holds a painter rather than a `Ui`.
    pub fn measure_on(&self, painter: &Painter) -> Vec2 {
        let m = theme::metrics();
        let g = self.galley(painter);
        vec2(
            (g.size().x + 2.0 * CAP_PAD).max(m.keycap_height),
            m.keycap_height,
        )
    }

    /// Paints the cap in `rect` (sized by [`Keycap::measure`]).
    pub fn paint(&self, ui: &Ui, rect: Rect) {
        self.paint_on(ui.painter(), rect);
    }

    /// [`Keycap::paint`] with a painter: the search field's hint, the palette's rows, anything
    /// painted inside another component.
    pub fn paint_on(&self, painter: &Painter, rect: Rect) {
        let p = theme::palette(painter.ctx());
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let radius = CornerRadius::same(m.keycap_radius);
        paint::fill(painter, rect, radius, s.keycap);
        // The inner bottom line sits inside the 1 pt ring.
        let inset = f32::from(m.keycap_radius);
        paint::seam_h(
            painter,
            egui::Rangef::new(rect.left() + inset, rect.right() - inset),
            rect.bottom() - 1.5 * paint::SEAM,
            s.keycap_edge,
        );
        paint::rim(painter, rect, radius, s.edge);
        let g = self.galley(painter);
        painter.galley(rect.center() - g.size() / 2.0, g, p.text_dim);
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let size = self.measure(ui);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui, rect);
        }
        label_response(response, &self.label, None)
    }
}

impl Widget for Keycap {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// Splits a chord label (`Ctrl+Shift+P`, `Ctrl++`) into its keys.
pub fn chord_keys(chord: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut current = String::new();
    for c in chord.chars() {
        if c == '+' && !current.is_empty() {
            keys.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        keys.push(current);
    }
    keys
}

/// Splits a keys label (`Ctrl+K  ·  Ctrl+Shift+P`, `g g`) into alternatives, each a sequence of
/// chords, each a list of keys.
pub fn parse_keys(text: &str) -> Vec<Vec<Vec<String>>> {
    text.split('·')
        .map(str::trim)
        .filter(|a| !a.is_empty())
        .map(|alt| alt.split_whitespace().map(chord_keys).collect())
        .collect()
}

/// A shortcut as keycaps: keys of a chord 3 apart, chords of a sequence 6 apart, alternatives
/// separated by a faint `·`. Labels come from `keymap.rs`.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct KeyChord {
    keys: Vec<Vec<Vec<String>>>,
    spoken: String,
}

impl KeyChord {
    /// The keys of `command` as the keymap lists them.
    pub fn command(command: impl Into<Shortcut>) -> Self {
        Self::new(command.into().hint())
    }

    /// Keys written as the keymap writes them (`Ctrl+Shift+P · f`).
    pub fn new(text: impl Into<String>) -> Self {
        let spoken = text.into();
        Self {
            keys: parse_keys(&spoken),
            spoken,
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let p = theme::palette(ui.ctx());
        let m = theme::metrics();
        let dot = layout(ui, "·".to_owned(), FontId::proportional(m.small_size));
        let mut items: Vec<(Option<Keycap>, f32)> = Vec::new();
        for (a, alt) in self.keys.iter().enumerate() {
            if a > 0 {
                items.push((None, SEQUENCE_GAP));
            }
            for (c, chord) in alt.iter().enumerate() {
                for (k, key) in chord.iter().enumerate() {
                    let gap = if k > 0 {
                        CHORD_GAP
                    } else if c > 0 {
                        SEQUENCE_GAP
                    } else {
                        0.0
                    };
                    items.push((Some(Keycap::new(key.clone())), gap));
                }
            }
        }
        let widths: Vec<f32> = items
            .iter()
            .map(|(cap, gap)| {
                gap + cap
                    .as_ref()
                    .map_or(dot.size().x + SEQUENCE_GAP, |k| k.measure(ui).x)
            })
            .collect();
        let size = vec2(widths.iter().sum(), m.keycap_height);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            let mut x = rect.left();
            for ((cap, gap), w) in items.iter().zip(&widths) {
                x += gap;
                match cap {
                    Some(k) => {
                        let at = Rect::from_min_size(pos2(x, rect.top()), k.measure(ui));
                        k.paint(ui, at);
                    }
                    None => {
                        ui.painter().galley(
                            pos2(x, rect.center().y - dot.size().y / 2.0),
                            Arc::clone(&dot),
                            p.text_faint,
                        );
                    }
                }
                x += w - gap;
            }
        }
        label_response(response, &self.spoken, None)
    }
}

impl Widget for KeyChord {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ---------------------------------------------------------------------------------- StatusDot

/// A small status circle with a ring of the surface behind it; `.pulse()` breathes while
/// something runs (still with reduced motion).
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct StatusDot {
    tone: Tone,
    pulse: bool,
    ground: Option<Color32>,
    label: String,
}

impl StatusDot {
    pub fn new(tone: Tone, label: impl Into<String>) -> Self {
        Self {
            tone,
            pulse: false,
            ground: None,
            label: label.into(),
        }
    }

    /// Breathes (alpha 0.5 to 1 over 1.6 s) while shown.
    pub fn pulse(mut self, on: bool) -> Self {
        self.pulse = on;
        self
    }

    /// The surface it sits on (default `pane`).
    pub fn ground(mut self, ground: Color32) -> Self {
        self.ground = Some(ground);
        self
    }

    /// The dot's diameter: 8, 6 in Dense.
    pub fn diameter() -> f32 {
        theme::metrics().status_dot
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let p = theme::palette(ui.ctx());
        let d = Self::diameter();
        let (rect, response) =
            ui.allocate_exact_size(Vec2::splat(d + 2.0 * DOT_RING), Sense::hover());
        if ui.is_rect_visible(rect) {
            let ground = self.ground.unwrap_or(theme::surfaces(ui.ctx()).pane);
            let alpha = if self.pulse {
                pulse_alpha(ui.ctx())
            } else {
                1.0
            };
            let c = rect.center();
            ui.painter().circle_filled(c, d / 2.0 + DOT_RING, ground);
            ui.painter()
                .circle_filled(c, d / 2.0, self.tone.color(p).gamma_multiply(alpha));
        }
        label_response(response, &self.label, Some(&self.label))
    }
}

impl Widget for StatusDot {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The breathing alpha of a pulsing dot now (0.5 to 1 over 1.6 s), repainting while shown; 1 and
/// still with reduced motion.
fn pulse_alpha(ctx: &egui::Context) -> f32 {
    if motion::reduced() {
        return 1.0;
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    let t = crate::to_f32(ctx.input(|i| i.time) % 1.6) / 1.6;
    0.75 + 0.25 * (t * std::f32::consts::TAU).cos()
}

// ----------------------------------------------------------------------------------- DiffStat

/// Lines added and removed: `+12 −3` in the diff inks (`diff_added`, `diff_removed`), with an
/// optional five-block bar.
#[derive(Debug, Clone, Copy)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct DiffStat {
    added: usize,
    removed: usize,
    bar: bool,
}

impl DiffStat {
    pub fn new(added: usize, removed: usize) -> Self {
        Self {
            added,
            removed,
            bar: false,
        }
    }

    /// Adds the five-block bar after the counts.
    pub fn bar(mut self, on: bool) -> Self {
        self.bar = on;
        self
    }

    fn counts(self, painter: &Painter) -> Arc<Galley> {
        let p = theme::palette(painter.ctx());
        let font = FontId::new(theme::metrics().sha_size, FontFamily::Monospace);
        let mut job = LayoutJob::default();
        let fmt = |c: Color32| TextFormat::simple(font.clone(), c);
        job.append(&format!("+{}", self.added), 0.0, fmt(p.diff_added));
        job.append(&format!("−{}", self.removed), STAT_GAP, fmt(p.diff_removed));
        painter.layout_job(job)
    }

    /// The size the counts (and bar) take.
    pub fn size(self, painter: &Painter) -> Vec2 {
        let g = self.counts(painter);
        let bar_w = if self.bar {
            STAT_GAP + 5.0 * BLOCK + 4.0 * BLOCK_GAP
        } else {
            0.0
        };
        vec2(g.size().x + bar_w, g.size().y)
    }

    /// Paints the counts (and bar) with their top left at `at`, with any painter (a list row's
    /// own painter); returns the rect painted.
    pub fn paint(self, painter: &Painter, at: Pos2) -> Rect {
        let p = theme::palette(painter.ctx());
        let s = theme::surfaces(painter.ctx());
        let g = self.counts(painter);
        let rect = Rect::from_min_size(at, self.size(painter));
        painter.galley(rect.left_top(), Arc::clone(&g), p.text);
        if self.bar {
            let (green, red, _) = blocks(self.added, self.removed);
            let mut x = rect.left() + g.size().x + STAT_GAP;
            for i in 0..5 {
                let c = if i < green {
                    p.diff_added
                } else if i < green + red {
                    p.diff_removed
                } else {
                    s.switch_off
                };
                let b =
                    Rect::from_min_size(pos2(x, rect.center().y - BLOCK / 2.0), Vec2::splat(BLOCK));
                painter.rect_filled(b, CornerRadius::same(BLOCK_RADIUS), c);
                x += BLOCK + BLOCK_GAP;
            }
        }
        rect
    }

    /// What a screen reader says for it.
    pub fn spoken(self) -> String {
        format!(
            "{} addition{}, {} deletion{}",
            self.added,
            if self.added == 1 { "" } else { "s" },
            self.removed,
            if self.removed == 1 { "" } else { "s" }
        )
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let size = self.size(ui.painter());
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            self.paint(ui.painter(), rect.left_top());
        }
        label_response(response, &self.spoken(), None)
    }
}

impl Widget for DiffStat {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The five blocks of a diffstat bar: (added, removed, neutral). Up to five lines fill one block
/// each; more fill all five in proportion, and a side with any change keeps at least one block.
pub fn blocks(added: usize, removed: usize) -> (usize, usize, usize) {
    let total = added + removed;
    if total == 0 {
        return (0, 0, 5);
    }
    let filled = total.min(5);
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "line counts are exact in f64, and the rounded share is between 0 and 5"
    )]
    let mut green = ((added * filled) as f64 / total as f64).round() as usize;
    if added > 0 {
        green = green.max(1);
    }
    if removed > 0 {
        green = green.min(filled - 1);
    }
    let red = filled - green;
    (green, red, 5 - filled)
}

// --------------------------------------------------------------------------------------- Chip

/// An interactive status chip (the status bar's trust, restart and update chips): a tinted pill
/// with an icon and a label, the ghost hover and press, and a focus ring.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Chip {
    text: String,
    spoken: Option<String>,
    icon: Option<String>,
    tone: Tone,
    enabled: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl Chip {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            spoken: None,
            icon: None,
            tone: Tone::Neutral,
            enabled: true,
            tooltip: None,
            disabled_reason: None,
            id_salt: None,
            force: None,
        }
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }
    /// What a screen reader says when the painted text is terse ("3 behind" for "↓3").
    pub fn spoken(mut self, name: impl Into<String>) -> Self {
        self.spoken = Some(name.into());
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
        self.disabled_reason = Some(text.into());
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    /// The chip's fill (the tone's tint; none when neutral) and its label colour.
    pub fn colors(&self, p: &Palette) -> (Color32, Color32) {
        match self.tone {
            Tone::Neutral => (Color32::TRANSPARENT, p.text_dim),
            tone => {
                let fill = tone.tint(p);
                (fill, p.ink(tone.color(p), fill))
            }
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let font = FontId::new(m.small_size, theme::medium());
        let text = layout(ui, self.text.clone(), font);
        let glyph = self
            .icon
            .as_ref()
            .map(|g| layout(ui, g.clone(), FontId::proportional(m.icon_xs)));
        let height = m.keycap_height;
        let gap = GAP;
        let content = text.size().x + glyph.as_ref().map_or(0.0, |g| g.size().x + gap);
        let size = vec2(content + 2.0 * theme::SM, height);
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = match self.id_salt {
            Some(id) => {
                let rect = ui.allocate_space(size).1;
                (rect, ui.interact(rect, ui.id().with(id), sense))
            }
            None => ui.allocate_exact_size(size, sense),
        };
        let st = Interact::of(&ctx, &response, enabled, self.force);
        if ui.is_rect_visible(rect) {
            // A disabled chip paints in the disabled tokens at full strength: a toned chip's tint
            // gives way to `disabled_fill`, and the label reads in `disabled_text`.
            let painter = super::content_painter(ui);
            let radius = radius_for(FULL, height);
            let (fill, ink) = match self.colors(p) {
                (fill, _) if !enabled && fill.a() > 0 => {
                    (p.surfaces(theme::style()).disabled_fill, p.disabled_text)
                }
                (fill, _) if !enabled => (fill, p.disabled_text),
                colors => colors,
            };
            if fill.a() > 0 {
                painter.rect_filled(rect, radius, fill);
            }
            let over = Color32::TRANSPARENT
                .lerp_to_gamma(p.hover_strong(), st.hover)
                .lerp_to_gamma(p.pressed(), st.press);
            if over.a() > 0 {
                painter.rect_filled(rect, radius, over);
            }
            if st.focus > 0.0 {
                paint::focus_ring(
                    &painter,
                    rect,
                    radius,
                    p.focus.gamma_multiply(st.focus),
                    Color32::TRANSPARENT,
                );
            }
            let ink = if enabled && self.tone == Tone::Neutral {
                ink.lerp_to_gamma(p.text, st.hover.max(st.press))
            } else {
                ink
            };
            let mut x = rect.left() + theme::SM;
            let mut shapes: Vec<Shape> = Vec::new();
            if let Some(g) = glyph {
                shapes.push(Shape::galley(
                    pos2(x, rect.center().y - g.size().y / 2.0),
                    Arc::clone(&g),
                    ink,
                ));
                x += g.size().x + gap;
            }
            shapes.push(Shape::galley(
                pos2(x, rect.center().y - text.size().y / 2.0),
                text,
                ink,
            ));
            painter.extend(shapes);
        }
        let name = self.spoken.as_deref().unwrap_or(&self.text);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, name));
        let tip = if enabled {
            self.tooltip
        } else {
            self.disabled_reason.or(self.tooltip)
        };
        match tip {
            Some(t) => response.kit_tooltip(t).show(),
            None => response,
        }
    }
}

impl Widget for Chip {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------ gallery

/// The gallery's Chips section: ref pills, badges per tone, counts, change letters, keycaps and
/// chords, status dots, diffstats and status chips.
pub(crate) fn gallery(ui: &mut Ui) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let small = |ui: &mut Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .size(m.small_size)
                .color(p.text_faint),
        );
    };
    let row = |ui: &mut Ui, f: &mut dyn FnMut(&mut Ui)| {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, theme::SM);
            f(ui);
        });
    };
    small(ui, "Badges per tone, strong, counts (1, 42, 99+)");
    row(ui, &mut |ui| {
        for tone in Tone::ALL {
            Badge::new(tone.label()).tone(tone).show(ui);
        }
        Badge::new("12").strong().mono().show(ui);
        for n in [1, 42, 140] {
            CountBadge::new(n).show(ui);
        }
        CountBadge::new(4).tone(Tone::Accent).show(ui);
        CountBadge::new(3).tone(Tone::Danger).show(ui);
        CountBadge::new(7)
            .strong()
            .tooltip("7 changed files")
            .show(ui);
    });
    small(ui, "Keycaps and chords");
    row(ui, &mut |ui| {
        Keycap::new("Esc").show(ui);
        Keycap::new("↵").show(ui);
        KeyChord::new("Ctrl+Shift+P").show(ui);
        KeyChord::command("Ctrl+K").show(ui);
        KeyChord::new("g g").show(ui);
        KeyChord::new("Ctrl++").show(ui);
    });
    small(ui, "Status dots (static, pulsing), diffstats");
    row(ui, &mut |ui| {
        for tone in Tone::ALL {
            StatusDot::new(tone, tone.label()).show(ui);
        }
        StatusDot::new(Tone::Accent, "Running").pulse(true).show(ui);
        StatusDot::new(Tone::Success, "On the chrome")
            .ground(theme::surfaces(ui.ctx()).panel)
            .show(ui);
        ui.add_space(theme::MD);
        DiffStat::new(12, 3).show(ui);
        DiffStat::new(12, 3).bar(true).show(ui);
        DiffStat::new(2, 0).bar(true).show(ui);
        DiffStat::new(0, 40).bar(true).show(ui);
    });
    small(
        ui,
        "Status chips: neutral, warning, accent, hover, focus, disabled",
    );
    row(ui, &mut |ui| {
        Chip::new("4 changed")
            .icon(icon::PENCIL_SIMPLE)
            .spoken("4 files changed")
            .id_salt("gc0")
            .show(ui);
        Chip::new("Downloads off · Trust…")
            .icon(icon::SHIELD_WARNING)
            .tone(Tone::Warning)
            .tooltip("Trust this folder")
            .id_salt("gc1")
            .show(ui);
        Chip::new("Restart to update")
            .icon(icon::ARROW_CLOCKWISE)
            .tone(Tone::Accent)
            .id_salt("gc2")
            .show(ui);
        Chip::new("Hover")
            .icon(icon::GIT_BRANCH)
            .id_salt("gc3")
            .show_forced(ui, ForceState::HOVER);
        Chip::new("Focus")
            .tone(Tone::Success)
            .id_salt("gc4")
            .show_forced(ui, ForceState::FOCUS);
        Chip::new("Disabled")
            .enabled(false)
            .disabled_reason("Nothing to do")
            .id_salt("gc5")
            .show(ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Style;

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, mut f: impl FnMut(&mut Ui)) {
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
    }

    /// Texts a frame painted.
    fn texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn painter_level_paints_match_what_the_widgets_draw() {
        let ctx = ctx();
        let at = pos2(20.0, 20.0);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter().clone();
            let cap = Keycap::new("Ctrl");
            let size = cap.measure_on(&painter);
            assert_eq!(size, cap.measure(ui));
            cap.paint_on(&painter, Rect::from_min_size(at, size));
            let stat = DiffStat::new(12, 3).bar(true);
            let rect = stat.paint(&painter, at);
            assert_eq!(rect.size(), stat.size(&painter));
            assert_eq!(stat.spoken(), "12 additions, 3 deletions");
        });
        out.textures_delta.clear();
        let painted = texts(&out.shapes);
        for t in ["Ctrl", "+12−3"] {
            assert!(painted.iter().any(|p| p == t), "{t} in {painted:?}");
        }
    }

    #[test]
    fn a_chip_can_speak_a_fuller_name_than_it_shows() {
        let ctx = ctx();
        let nodes = super::super::testkit::spoken(&ctx, |ui| {
            Chip::new("↓3").spoken("3 commits behind").show(ui);
        });
        assert!(super::super::testkit::find(&nodes, "3 commits behind").is_some());
        assert!(super::super::testkit::find(&nodes, "↓3").is_none());
    }

    #[test]
    fn counts_cap_at_99() {
        assert_eq!(count_text(0), "0");
        assert_eq!(count_text(99), "99");
        assert_eq!(count_text(100), "99+");
    }

    #[test]
    fn chords_split_into_keys() {
        assert_eq!(chord_keys("Ctrl+Shift+P"), ["Ctrl", "Shift", "P"]);
        assert_eq!(chord_keys("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(chord_keys("+"), ["+"]);
        assert_eq!(
            parse_keys("Ctrl+K  ·  Ctrl+Shift+P"),
            vec![
                vec![vec!["Ctrl".to_owned(), "K".to_owned()]],
                vec![vec!["Ctrl".to_owned(), "Shift".to_owned(), "P".to_owned()]],
            ]
        );
        assert_eq!(parse_keys("g g").first().map(Vec::len), Some(2));
    }

    #[test]
    fn diffstat_blocks_fill_in_proportion() {
        assert_eq!(blocks(0, 0), (0, 0, 5));
        assert_eq!(blocks(2, 0), (2, 0, 3));
        assert_eq!(blocks(12, 3), (4, 1, 0));
        assert_eq!(blocks(100, 1), (4, 1, 0));
        assert_eq!(blocks(1, 100), (1, 4, 0));
        assert_eq!(blocks(0, 40), (0, 5, 0));
        for a in 0..30 {
            for r in 0..30 {
                let (g, d, n) = blocks(a, r);
                assert_eq!(g + d + n, 5);
                assert_eq!(g > 0, a > 0, "{a} {r}");
                assert_eq!(d > 0, r > 0, "{a} {r}");
            }
        }
    }

    #[test]
    fn coloured_labels_read_on_their_fills() {
        for id in theme::ThemeId::ALL {
            let p = id.palette();
            for style in Style::ALL {
                theme::with_look(style, theme::Density::Comfortable, || {
                    let s = p.surfaces(style);
                    let grounds = [s.pane, s.panel];
                    for tone in Tone::ALL {
                        let (fill, ink) = Badge::new("1").tone(tone).colors(p);
                        for g in grounds {
                            let behind = theme::over(fill.top, g);
                            assert!(
                                theme::contrast(ink, behind) >= 4.5,
                                "{} {style:?} badge {tone:?}",
                                p.name
                            );
                        }
                        let (fill, ink) = Chip::new("").tone(tone).colors(p);
                        for g in grounds {
                            assert!(
                                theme::contrast(ink, theme::over(fill, g)) >= 4.5,
                                "{} {style:?} chip {tone:?}",
                                p.name
                            );
                        }
                    }
                });
            }
        }
    }

    /// Spec 4.18: a status dot is 8 pt, 6 in Dense, from the density's metrics.
    #[test]
    fn the_status_dot_follows_the_density() {
        let ctx = ctx();
        for (density, d) in [
            (theme::Density::Comfortable, 8.0),
            (theme::Density::Dense, 6.0),
        ] {
            theme::apply(
                &ctx,
                theme::ThemeId::Midnight,
                theme::Appearance {
                    density,
                    ..theme::Appearance::STILL
                },
            );
            assert!(
                (StatusDot::diameter() - d).abs() < f32::EPSILON,
                "{density:?}"
            );
        }
    }

    #[test]
    fn a_chip_clicks_from_the_keyboard_and_not_when_disabled() {
        let ctx = ctx();
        for enabled in [true, false] {
            let mut clicked = false;
            for i in 0..3 {
                let events = if i == 1 {
                    vec![egui::Event::Key {
                        key: egui::Key::Enter,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::default(),
                    }]
                } else {
                    Vec::new()
                };
                frame(&ctx, events, |ui| {
                    let r = Chip::new("Trust")
                        .enabled(enabled)
                        .id_salt("chip-test")
                        .show(ui);
                    if i == 0 {
                        ui.memory_mut(|m| m.request_focus(r.id));
                    }
                    clicked |= r.clicked();
                });
            }
            assert_eq!(clicked, enabled, "enabled {enabled}");
        }
    }

    #[test]
    fn labels_reach_screen_readers() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            DiffStat::new(1, 2).show(ui);
            KeyChord::new("Ctrl+K").show(ui);
            Chip::new("Restart to update").show(ui);
        });
        out.textures_delta.clear();
        let labels: Vec<String> = out
            .platform_output
            .accesskit_update
            .map(|u| {
                u.nodes
                    .iter()
                    .filter_map(|(_, n)| n.label().or(n.value()).map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        for want in ["1 addition, 2 deletions", "Ctrl+K", "Restart to update"] {
            assert!(labels.iter().any(|l| l == want), "{want}: {labels:?}");
        }
    }

    #[test]
    fn pulsing_dots_hold_still_with_reduced_motion() {
        let ctx = ctx();
        assert_eq!(pulse_alpha(&ctx), 1.0);
    }

    #[test]
    fn keycaps_are_at_least_square() {
        let ctx = ctx();
        frame(&ctx, Vec::new(), |ui| {
            let m = theme::metrics();
            assert_eq!(Keycap::new("K").measure(ui).x, m.keycap_height);
            assert!(Keycap::new("Shift").measure(ui).x > m.keycap_height);
        });
    }
}
