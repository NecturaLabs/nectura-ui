//! [`Checkbox`] (with indeterminate), [`Radio`], [`RadioGroup`] and [`Switch`]. Design spec 4.4
//! to 4.6.
//!
//! Each is a glyph (box, dot or track) and an optional label, one hit area for both. Checked
//! fills crossfade and the tick, dot and switch thumb move on the spec's springs; reduced motion
//! makes every change instant. Disabled controls paint with the disabled tokens at full strength
//! (the glyph on `disabled_fill` outlined in the disabled mark, no accent, the label in
//! `disabled_text`) and stay hoverable so their reason can show.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Galley, Id, Key, Modifiers, Painter, Pos2, Rect,
    Response, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, Widget, WidgetInfo,
    WidgetType, pos2, vec2,
};

use super::button::tip;
use super::{ForceState, Interact, anim, paint};
use crate::motion;
use crate::theme::{self, Elevation, Fill, Palette};

/// Gap between a glyph and its label (concept `.k-choice`).
const LABEL_GAP: f32 = 9.0;
/// Width of the checkbox and radio rim.
const RIM: f32 = 1.5;
/// The tick's drawing box and stroke, in the concept's 24-unit view box drawn at 12 pt.
const TICK_BOX: f32 = 12.0;
const TICK_STROKE: f32 = 2.6;
/// The radio's checked dot.
const DOT: f32 = 6.0;
/// The switch thumb's inset from the track and how much it widens while pressed.
const KNOB_INSET: f32 = 2.0;
const KNOB_PRESS_GROW: f32 = 3.0;
/// Tick and dot: in on `SPRING_CURVE` over 0.26 s (fading in over 0.12 s), out in 0.08 s with no
/// overshoot; checked fill crossfade 0.14 s; switch track crossfade 0.2 s.
const MARK_IN: f32 = 0.26;
const MARK_FADE_IN: f32 = 0.12;
const MARK_OUT: f32 = 0.08;
const FILL_FADE: f32 = 0.14;
const TRACK_FADE: f32 = 0.20;

/// Where a glyph and label sit: the whole row is the hit area.
fn allocate_row(
    ui: &mut Ui,
    glyph: Vec2,
    text: Option<&Arc<Galley>>,
    id_salt: Option<Id>,
    sense: Sense,
) -> (Rect, Response) {
    let m = theme::metrics();
    let width = glyph.x + text.map_or(0.0, |t| LABEL_GAP + t.size().x);
    let height = glyph
        .y
        .max(text.map_or(0.0, |t| t.size().y))
        .max(m.control_sm);
    let size = vec2(width, height);
    match id_salt {
        Some(id) => {
            let rect = ui.allocate_space(size).1;
            (rect, ui.interact(rect, ui.id().with(id), sense))
        }
        None => ui.allocate_exact_size(size, sense),
    }
}

fn label_galley(ui: &Ui, label: &str) -> Option<Arc<Galley>> {
    let m = theme::metrics();
    (!label.is_empty()).then(|| {
        ui.painter().layout_no_wrap(
            label.to_owned(),
            FontId::new(m.body_size, FontFamily::Proportional),
            Color32::PLACEHOLDER,
        )
    })
}

/// A disabled control's label stays readable: opaque, in `disabled_text`.
fn paint_label(
    ui: &Ui,
    painter: &Painter,
    enabled: bool,
    rect: Rect,
    glyph_w: f32,
    text: Option<&Arc<Galley>>,
    p: &Palette,
) {
    if let Some(t) = text {
        let pos = pos2(
            rect.left() + glyph_w + LABEL_GAP,
            rect.center().y - t.size().y / 2.0,
        );
        if enabled {
            painter.galley(pos, Arc::clone(t), p.text);
        } else {
            let color = p.disabled_text;
            super::content_painter(ui).galley(pos, Arc::clone(t), color);
        }
    }
}

fn sense_for(enabled: bool) -> Sense {
    if enabled {
        Sense::click()
    } else {
        Sense::hover()
    }
}

/// How far a mark (tick, dash or dot) has come in, as (eased scale, alpha), both 0 to 1. Turning
/// on it springs in over `MARK_IN` and fades in over `MARK_FADE_IN`; turning off it shrinks back
/// over `MARK_OUT` with no overshoot. A forced progress shows it turning on part-way.
fn mark(ctx: &egui::Context, id: Id, on: bool, force: Option<ForceState>) -> (f32, f32) {
    let lin = match force.and_then(|f| f.progress) {
        Some(k) => {
            if on {
                k
            } else {
                0.0
            }
        }
        None if force.is_some() => f32::from(u8::from(on)),
        None => {
            ctx.animate_bool_with_time(id, on, motion::time(if on { MARK_IN } else { MARK_OUT }))
        }
    };
    if on {
        let alpha = (lin * MARK_IN / MARK_FADE_IN).min(1.0);
        (motion::SPRING_CURVE.at(lin), alpha)
    } else {
        (lin, lin)
    }
}

/// A crossfade of `seconds` towards `on` (or the forced progress).
fn crossfade(
    ctx: &egui::Context,
    id: Id,
    on: bool,
    seconds: f32,
    force: Option<ForceState>,
) -> f32 {
    match force {
        Some(f) => {
            let k = f.progress.unwrap_or(1.0);
            if on { k } else { 0.0 }
        }
        None => anim::ease(ctx, id, on, seconds),
    }
}

/// `fill` painted at `opacity` (gradients included, through the painter's opacity).
fn fill_at(painter: &Painter, rect: Rect, radius: CornerRadius, fill: Fill, opacity: f32) {
    if opacity <= 0.0 {
        return;
    }
    let mut faded = painter.clone();
    faded.multiply_opacity(opacity.min(1.0));
    paint::fill(&faded, rect, radius, fill);
}

/// The unchecked box or circle: `field` fill and a 1.5 pt rim, `control_border` at rest and
/// `text_dim` under the pointer; disabled, `disabled_fill` in the disabled mark.
fn paint_unchecked(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    p: &Palette,
    hover: f32,
    enabled: bool,
) {
    let s = theme::surfaces(painter.ctx());
    let (fill, rim) = if enabled {
        (s.field, p.control_border.lerp_to_gamma(p.text_dim, hover))
    } else {
        (s.disabled_fill, super::disabled_mark(p, s))
    };
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(rect, radius, Stroke::new(RIM, rim), StrokeKind::Inside);
}

/// The tick, dash or dot over a checked glyph: `on_accent` on the accent fill, `disabled_text`
/// on a disabled glyph (which has no accent fill), faded in by `alpha`.
fn mark_color(p: &Palette, enabled: bool, alpha: f32) -> Color32 {
    let c = if enabled {
        p.on_accent
    } else {
        p.disabled_text
    };
    c.gamma_multiply(alpha.min(1.0))
}

/// A round-capped polyline (the tick and the dash).
fn polyline(painter: &Painter, points: &[Pos2], width: f32, color: Color32) {
    painter.add(Shape::line(points.to_vec(), Stroke::new(width, color)));
    for &pt in points {
        painter.circle_filled(pt, width / 2.0, color);
    }
}

// ------------------------------------------------------------------------------------ Checkbox

/// A checkbox with a label; `indeterminate` shows a dash (some, not all). Clicking an
/// indeterminate checkbox checks it.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Checkbox<'a> {
    value: &'a mut bool,
    label: String,
    indeterminate: bool,
    enabled: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl<'a> Checkbox<'a> {
    pub fn new(value: &'a mut bool, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            indeterminate: false,
            enabled: true,
            tooltip: None,
            disabled_reason: None,
            id_salt: None,
            force: None,
        }
    }
    pub fn indeterminate(mut self, indeterminate: bool) -> Self {
        self.indeterminate = indeterminate;
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

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let text = label_galley(ui, &self.label);
        let side = m.checkbox;
        let (rect, mut response) = allocate_row(
            ui,
            Vec2::splat(side),
            text.as_ref(),
            self.id_salt,
            sense_for(enabled),
        );
        if enabled && response.clicked() {
            *self.value = self.indeterminate || !*self.value;
            response.mark_changed();
        }
        let checked = *self.value && !self.indeterminate;
        let on = checked || self.indeterminate;
        let st = Interact::of(&ctx, &response, enabled, self.force);
        if ui.is_rect_visible(rect) {
            let painter = super::content_painter(ui);
            let id = response.id;
            let boxed = Rect::from_center_size(
                pos2(rect.left() + side / 2.0, rect.center().y),
                Vec2::splat(side),
            );
            let radius = CornerRadius::same(m.radius_sm);
            paint_unchecked(&painter, boxed, radius, p, st.hover, enabled);
            let fill = crossfade(&ctx, id.with("kit-check-fill"), on, FILL_FADE, self.force);
            if enabled {
                fill_at(&painter, boxed, radius, s.primary, fill);
            }
            let unit = TICK_BOX / 24.0;
            let at = |x: f32, y: f32, scale: f32| {
                boxed.center() + vec2(x - 12.0, y - 12.0) * unit * scale
            };
            let (scale, alpha) = mark(&ctx, id.with("kit-check-tick"), checked, self.force);
            if alpha > 0.0 {
                let k = 0.4 + 0.6 * scale;
                polyline(
                    &painter,
                    &[at(5.0, 12.5, k), at(9.5, 17.0, k), at(19.0, 7.5, k)],
                    TICK_STROKE * unit * k,
                    mark_color(p, enabled, alpha),
                );
            }
            let (scale, alpha) = mark(
                &ctx,
                id.with("kit-check-dash"),
                self.indeterminate,
                self.force,
            );
            if alpha > 0.0 {
                let k = 0.4 + 0.6 * scale;
                polyline(
                    &painter,
                    &[at(6.0, 12.0, k), at(18.0, 12.0, k)],
                    TICK_STROKE * unit * k,
                    mark_color(p, enabled, alpha),
                );
            }
            if st.focus > 0.0 {
                paint::focus_ring(
                    &painter,
                    boxed,
                    radius,
                    p.focus.gamma_multiply(st.focus),
                    Color32::TRANSPARENT,
                );
            }
            paint_label(ui, &painter, enabled, rect, side, text.as_ref(), p);
        }
        let (label, indeterminate) = (self.label, self.indeterminate);
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(WidgetType::Checkbox, enabled, &label);
            // No `selected` is AccessKit's mixed state.
            info.selected = (!indeterminate).then_some(checked);
            info
        });
        tip(
            response,
            enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
            None,
            true,
        )
    }
}

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// --------------------------------------------------------------------------------------- Radio

/// One radio button. `clicked()` means the user chose it; the caller sets the value. Use
/// [`RadioGroup`] for a set of options, which adds arrow keys and the group role.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Radio {
    selected: bool,
    label: String,
    enabled: bool,
    tab_stop: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl Radio {
    pub fn new(selected: bool, label: impl Into<String>) -> Self {
        Self {
            selected,
            label: label.into(),
            enabled: true,
            tab_stop: true,
            tooltip: None,
            disabled_reason: None,
            id_salt: None,
            force: None,
        }
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
    /// Whether Tab stops here (a group keeps one stop, on its chosen option).
    fn tab_stop(mut self, stop: bool) -> Self {
        self.tab_stop = stop;
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let text = label_galley(ui, &self.label);
        let side = m.radio;
        let sense = match (enabled, self.tab_stop) {
            (false, _) => Sense::hover(),
            (true, true) => Sense::click(),
            (true, false) => Sense::CLICK,
        };
        let (rect, response) =
            allocate_row(ui, Vec2::splat(side), text.as_ref(), self.id_salt, sense);
        let st = Interact::of(&ctx, &response, enabled, self.force);
        if ui.is_rect_visible(rect) {
            let painter = super::content_painter(ui);
            let id = response.id;
            let circle = Rect::from_center_size(
                pos2(rect.left() + side / 2.0, rect.center().y),
                Vec2::splat(side),
            );
            let radius = CornerRadius::same(theme::FULL);
            paint_unchecked(&painter, circle, radius, p, st.hover, enabled);
            let fill = crossfade(
                &ctx,
                id.with("kit-radio-fill"),
                self.selected,
                FILL_FADE,
                self.force,
            );
            if enabled {
                fill_at(&painter, circle, radius, s.primary, fill);
            }
            let (scale, alpha) = mark(&ctx, id.with("kit-radio-dot"), self.selected, self.force);
            if alpha > 0.0 && scale > 0.0 {
                painter.circle_filled(
                    circle.center(),
                    DOT / 2.0 * scale,
                    mark_color(p, enabled, alpha),
                );
            }
            if st.focus > 0.0 {
                paint::focus_ring(
                    &painter,
                    circle,
                    radius,
                    p.focus.gamma_multiply(st.focus),
                    Color32::TRANSPARENT,
                );
            }
            paint_label(ui, &painter, enabled, rect, side, text.as_ref(), p);
        }
        let (label, selected) = (self.label, self.selected);
        response.widget_info(|| {
            WidgetInfo::selected(WidgetType::RadioButton, enabled, selected, &label)
        });
        tip(
            response,
            enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
            None,
            true,
        )
    }
}

impl Widget for Radio {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The index `delta` steps from `from` among `len` options, wrapping.
pub(super) fn step(from: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let (from, shift) = (from % len, delta.unsigned_abs() % len);
    if delta < 0 {
        (from + len - shift) % len
    } else {
        (from + shift) % len
    }
}

/// The arrow key pressed this frame, as a step: Down and Right forward, Up and Left back.
pub(super) fn arrow_step(ui: &Ui, vertical: bool, horizontal: bool) -> Option<isize> {
    ui.input_mut(|i| {
        if (horizontal && i.consume_key(Modifiers::NONE, Key::ArrowRight))
            || (vertical && i.consume_key(Modifiers::NONE, Key::ArrowDown))
        {
            Some(1)
        } else if (horizontal && i.consume_key(Modifiers::NONE, Key::ArrowLeft))
            || (vertical && i.consume_key(Modifiers::NONE, Key::ArrowUp))
        {
            Some(-1)
        } else {
            None
        }
    })
}

/// Keeps the arrow keys with the focused control `id` (egui would otherwise move focus to the
/// neighbouring widget), so its own arrow handling decides what they do.
pub(super) fn claim_arrows(ui: &Ui, id: Id, horizontal: bool, vertical: bool) {
    ui.memory_mut(|m| {
        m.set_focus_lock_filter(
            id,
            egui::EventFilter {
                horizontal_arrows: horizontal,
                vertical_arrows: vertical,
                ..Default::default()
            },
        );
    });
}

/// The id a component keeps its state under: from its salt when it has one, else from its place
/// in `ui` (stable while the layout is), so two unsalted components never share state.
pub(super) fn component_id(ui: &Ui, kind: &str, salt: Option<Id>) -> Id {
    match salt {
        Some(salt) => ui.id().with((kind, salt)),
        None => ui.next_auto_id().with(kind),
    }
}

/// Marks `options`, the `Ui` that holds a group's options, as an AccessKit group of `role` named
/// `label`, so the options are its parts (a node made beside them would be an empty group).
pub(super) fn group_node(options: &Ui, role: Role, label: &str, enabled: bool) {
    super::a11y::group(options, role, label);
    if !enabled {
        options
            .ctx()
            .accesskit_node_builder(options.unique_id(), egui::accesskit::Node::set_disabled);
    }
}

/// Mutually exclusive options with optional descriptions; arrow keys move the choice.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct RadioGroup<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    options: &'a [(T, &'a str, Option<&'a str>)],
    label: String,
    horizontal: bool,
    enabled: bool,
    id_salt: Option<Id>,
    force: Option<(usize, ForceState)>,
}

impl<'a, T: PartialEq + Clone> RadioGroup<'a, T> {
    /// Options as (value, label, description).
    pub fn new(value: &'a mut T, options: &'a [(T, &'a str, Option<&'a str>)]) -> Self {
        Self {
            value,
            options,
            label: String::new(),
            horizontal: false,
            enabled: true,
            id_salt: None,
            force: None,
        }
    }
    /// The group's accessible name (what the options choose).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
    /// Options side by side instead of stacked.
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    /// Forces a state on option `index` (the gallery and tests only).
    pub(crate) fn show_forced(mut self, ui: &mut Ui, index: usize, force: ForceState) -> Response {
        self.force = Some((index, force));
        self.show(ui)
    }

    /// Shows the options; the response is the group's and reports `changed` when the choice moved.
    pub fn show(self, ui: &mut Ui) -> Response {
        let m = theme::metrics();
        let p = theme::palette(ui.ctx());
        let enabled = self.enabled && ui.is_enabled();
        let group_id = component_id(ui, "kit-radio-group", self.id_salt);
        let chosen = self.options.iter().position(|(v, _, _)| v == self.value);
        let mut responses = Vec::with_capacity(self.options.len());
        let layout = if self.horizontal {
            egui::Layout::left_to_right(egui::Align::Min)
        } else {
            egui::Layout::top_down(egui::Align::Min)
        };
        let inner = ui.scope_builder(UiBuilder::new().layout(layout), |ui| {
            group_node(ui, Role::RadioGroup, &self.label, enabled);
            ui.spacing_mut().item_spacing = if self.horizontal {
                vec2(theme::LG, theme::SM)
            } else {
                vec2(theme::SM, theme::SM)
            };
            for (i, (_, label, description)) in self.options.iter().enumerate() {
                let radio = Radio::new(chosen == Some(i), *label)
                    .enabled(enabled)
                    .tab_stop(chosen.unwrap_or(0) == i)
                    .id_salt((group_id, i));
                let force = self.force.filter(|(at, _)| *at == i).map(|(_, f)| f);
                let r = match description {
                    Some(d) => {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let r = match force {
                                Some(f) => radio.show_forced(ui, f),
                                None => radio.show(ui),
                            };
                            ui.horizontal(|ui| {
                                ui.add_space(m.radio + LABEL_GAP);
                                let (painter, color) = if enabled {
                                    (ui.painter().clone(), p.text_dim)
                                } else {
                                    (super::content_painter(ui), p.disabled_text)
                                };
                                let g = painter.layout(
                                    (*d).to_owned(),
                                    FontId::new(m.meta_size, FontFamily::Proportional),
                                    color,
                                    ui.available_width().max(1.0),
                                );
                                let (rect, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                                painter.galley(rect.min, g, color);
                            });
                            r
                        })
                        .inner
                    }
                    None => match force {
                        Some(f) => radio.show_forced(ui, f),
                        None => radio.show(ui),
                    },
                };
                responses.push(r);
            }
        });
        let mut changed = false;
        for (i, r) in responses.iter().enumerate() {
            if r.clicked() && chosen != Some(i) {
                *self.value = self.options[i].0.clone();
                changed = true;
            }
        }
        let focused = responses.iter().position(Response::has_focus);
        if let Some(at) = focused {
            claim_arrows(ui, responses[at].id, true, true);
        }
        if let (Some(at), true) = (focused, enabled)
            && let Some(delta) = arrow_step(ui, true, true)
        {
            let next = step(at, delta, self.options.len());
            *self.value = self.options[next].0.clone();
            responses[next].request_focus();
            changed = true;
        }
        let mut group = ui.interact(inner.response.rect, group_id, Sense::hover());
        if changed {
            group.mark_changed();
        }
        group
    }
}

// -------------------------------------------------------------------------------------- Switch

/// An on/off switch that takes effect at once (settings that wait for Save use a [`Checkbox`]).
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Switch<'a> {
    value: &'a mut bool,
    label: String,
    show_label: bool,
    enabled: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl<'a> Switch<'a> {
    pub fn new(value: &'a mut bool, label: impl Into<String>) -> Self {
        Self {
            value,
            label: label.into(),
            show_label: true,
            enabled: true,
            tooltip: None,
            disabled_reason: None,
            id_salt: None,
            force: None,
        }
    }
    /// `false`: the label is the switch's accessible name only, not painted beside it (a
    /// settings row that lays its own title out). The switch reports itself once either way.
    pub fn show_label(mut self, show: bool) -> Self {
        self.show_label = show;
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

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let text = if self.show_label {
            label_galley(ui, &self.label)
        } else {
            None
        };
        let (rect, mut response) = allocate_row(
            ui,
            m.switch,
            text.as_ref(),
            self.id_salt,
            sense_for(enabled),
        );
        if enabled && response.clicked() {
            *self.value = !*self.value;
            response.mark_changed();
        }
        let on = *self.value;
        let st = Interact::of(&ctx, &response, enabled, self.force);
        if ui.is_rect_visible(rect) {
            let painter = super::content_painter(ui);
            let id = response.id;
            let track = Rect::from_min_size(
                pos2(rect.left(), rect.center().y - m.switch.y / 2.0),
                m.switch,
            );
            let radius = CornerRadius::same(theme::FULL);
            let fill = crossfade(
                &ctx,
                id.with("kit-switch-track"),
                on,
                TRACK_FADE,
                self.force,
            );
            if enabled {
                painter.rect_filled(track, radius, s.switch_off);
                if p.strict && fill < 1.0 {
                    paint::rim(
                        &painter,
                        track,
                        radius,
                        p.control_border.gamma_multiply(1.0 - fill),
                    );
                }
                fill_at(&painter, track, radius, s.primary, fill);
            } else {
                // Disabled: the neutral track in the disabled mark; the knob's side keeps the
                // value.
                painter.rect_filled(track, radius, s.disabled_fill);
                paint::rim(&painter, track, radius, super::disabled_mark(p, s));
            }
            let target = if on { m.switch_travel } else { 0.0 };
            let travel = match self.force.map(|f| f.progress) {
                Some(Some(k)) => target * k,
                Some(None) => target,
                None => anim::spring(&ctx, id.with("kit-switch-knob"), target, motion::SWITCH),
            };
            let grow = KNOB_PRESS_GROW * st.press;
            let left = track.left() + KNOB_INSET + travel - if on { grow } else { 0.0 };
            let knob = Rect::from_min_size(
                pos2(left, track.center().y - m.switch_knob / 2.0),
                vec2(m.switch_knob + grow, m.switch_knob),
            );
            if enabled {
                paint::surface(
                    &painter,
                    knob,
                    radius,
                    p,
                    style,
                    Elevation::Knob,
                    s.knob,
                    s.knob_edge,
                );
            } else {
                painter.rect_filled(knob, radius, super::disabled_mark(p, s));
            }
            if st.focus > 0.0 {
                paint::focus_ring(
                    &painter,
                    track,
                    radius,
                    p.focus.gamma_multiply(st.focus),
                    Color32::TRANSPARENT,
                );
            }
            paint_label(ui, &painter, enabled, rect, m.switch.x, text.as_ref(), p);
        }
        let label = self.label;
        response.widget_info(|| WidgetInfo::selected(WidgetType::Checkbox, enabled, on, &label));
        ctx.accesskit_node_builder(response.id, |n| n.set_role(Role::Switch));
        tip(
            response,
            enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
            None,
            true,
        )
    }
}

impl Widget for Switch<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------- gallery

/// Runs `f` on a value the gallery keeps between frames under `key`, so its live controls respond.
pub(super) fn remembered<T: Clone + Send + Sync + 'static>(
    ui: &mut Ui,
    key: &str,
    default: T,
    f: impl FnOnce(&mut Ui, &mut T),
) {
    let id = ui.id().with(("kit-gallery-value", key));
    let mut value = ui.data(|d| d.get_temp::<T>(id)).unwrap_or(default);
    f(ui, &mut value);
    ui.data_mut(|d| d.insert_temp(id, value));
}

/// A small caption over a gallery row.
pub(super) fn gallery_caption(ui: &mut Ui, text: &str) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    ui.label(
        egui::RichText::new(text)
            .size(m.small_size)
            .color(p.text_faint),
    );
}

/// A wrapped gallery row with room between the controls.
pub(super) fn gallery_row(ui: &mut Ui, f: impl FnOnce(&mut Ui)) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(theme::LG, theme::SM);
        f(ui);
    });
}

/// The gallery's toggles: every checkbox, radio and switch state, then their transitions.
pub(crate) fn gallery(ui: &mut Ui) {
    gallery_caption(
        ui,
        "Checkbox: off, on, indeterminate, hover, focus, disabled, live",
    );
    gallery_row(ui, |ui| {
        Checkbox::new(&mut false, "Prune on fetch")
            .id_salt("g-c0")
            .show(ui);
        Checkbox::new(&mut true, "Sign commits")
            .id_salt("g-c1")
            .show(ui);
        Checkbox::new(&mut false, "Stage all 4 files")
            .indeterminate(true)
            .id_salt("g-c2")
            .show(ui);
        Checkbox::new(&mut false, "Hover")
            .tooltip("Removes branches deleted on the remote")
            .id_salt("g-c3")
            .show_forced(ui, ForceState::HOVER);
        Checkbox::new(&mut true, "Focus")
            .id_salt("g-c4")
            .show_forced(ui, ForceState::FOCUS);
        Checkbox::new(&mut true, "Disabled")
            .enabled(false)
            .disabled_reason("Nothing to amend yet")
            .id_salt("g-c5")
            .show(ui);
        remembered(ui, "check", false, |ui, v| {
            Checkbox::new(v, "Click me").id_salt("g-c6").show(ui);
        });
    });
    ui.add_space(theme::XS);
    gallery_caption(
        ui,
        "Radio group, stacked with descriptions; side by side with focus; hover; disabled",
    );
    remembered(ui, "radio", 0_u8, |ui, v| {
        let options = [
            (
                0_u8,
                "Merge commit",
                Some("Keeps every commit and adds a merge commit."),
            ),
            (1, "Squash", Some("Combines the branch into one commit.")),
            (2, "Rebase", None),
        ];
        RadioGroup::new(v, &options)
            .label("Merge style")
            .id_salt("g-r0")
            .show(ui);
    });
    ui.add_space(theme::XS);
    let options = [(0_u8, "Merge commit", None), (1, "Squash", None)];
    gallery_row(ui, |ui| {
        RadioGroup::new(&mut 0_u8, &options)
            .label("Merge style")
            .horizontal()
            .id_salt("g-r1")
            .show_forced(ui, 0, ForceState::FOCUS);
        Radio::new(false, "Hover")
            .tooltip("Replays your commits on top")
            .id_salt("g-r2")
            .show_forced(ui, ForceState::HOVER);
        Radio::new(true, "Disabled")
            .enabled(false)
            .disabled_reason("The remote allows merge commits only")
            .id_salt("g-r3")
            .show(ui);
        RadioGroup::new(&mut 1_u8, &options)
            .label("Disabled group")
            .horizontal()
            .enabled(false)
            .id_salt("g-r4")
            .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(
        ui,
        "Switch: off, on, hover, pressed off, pressed on, focus, disabled, live",
    );
    gallery_row(ui, |ui| {
        Switch::new(&mut false, "Off").id_salt("g-s0").show(ui);
        Switch::new(&mut true, "On").id_salt("g-s1").show(ui);
        Switch::new(&mut false, "Hover")
            .tooltip("Takes effect at once")
            .id_salt("g-s2")
            .show_forced(ui, ForceState::HOVER);
        Switch::new(&mut false, "Pressed")
            .id_salt("g-s3")
            .show_forced(ui, ForceState::PRESSED);
        Switch::new(&mut true, "Pressed")
            .id_salt("g-s4")
            .show_forced(ui, ForceState::PRESSED);
        Switch::new(&mut true, "Focus")
            .id_salt("g-s5")
            .show_forced(ui, ForceState::FOCUS);
        Switch::new(&mut true, "Disabled")
            .enabled(false)
            .disabled_reason("Not available with High contrast")
            .id_salt("g-s6")
            .show(ui);
        remembered(ui, "switch", true, |ui, v| {
            Switch::new(v, "Click me").id_salt("g-s7").show(ui);
        });
        Switch::new(&mut true, "Named, label not drawn")
            .show_label(false)
            .id_salt("g-s8")
            .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(ui, "Turning on at t = 0, 0.5, 1: tick, dash, dot, thumb");
    gallery_row(ui, |ui| {
        for (i, t) in [0.0_f32, 0.5, 1.0].into_iter().enumerate() {
            let force = ForceState {
                progress: Some(t),
                ..ForceState::default()
            };
            Checkbox::new(&mut true, "")
                .id_salt(("g-mt", i))
                .show_forced(ui, force);
            Checkbox::new(&mut false, "")
                .indeterminate(true)
                .id_salt(("g-md", i))
                .show_forced(ui, force);
            Radio::new(true, "")
                .id_salt(("g-mr", i))
                .show_forced(ui, force);
            Switch::new(&mut true, format!("{t:.1}"))
                .id_salt(("g-ms", i))
                .show_forced(ui, force);
        }
    });
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Toggled;

    use super::super::testkit::{self, find, frame, key, spoken};
    use super::*;

    #[test]
    fn checkbox_toggles_by_click_and_space_and_reports_its_state() {
        let ctx = testkit::ctx();
        let mut on = false;
        let (mut rect, mut id) = (Rect::NOTHING, Id::NULL);
        frame(&ctx, Vec::new(), |ui| {
            let r = Checkbox::new(&mut on, "Amend").show(ui);
            (rect, id) = (r.rect, r.id);
        });
        testkit::click(&ctx, rect.center(), |ui| {
            Checkbox::new(&mut on, "Amend").show(ui);
        });
        assert!(on, "a click checks it");
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, key(Key::Space), |ui| {
            Checkbox::new(&mut on, "Amend").show(ui);
        });
        assert!(!on, "Space unchecks the focused checkbox");
        let nodes = spoken(&ctx, |ui| {
            Checkbox::new(&mut on, "Amend").show(ui);
        });
        let n = find(&nodes, "Amend").expect("spoken");
        assert_eq!(n.role, Role::CheckBox);
        assert_eq!(n.toggled, Some(Toggled::False));
    }

    #[test]
    fn indeterminate_reads_as_mixed_and_a_click_checks_it() {
        let ctx = testkit::ctx();
        let mut on = false;
        let nodes = spoken(&ctx, |ui| {
            Checkbox::new(&mut on, "Stage all")
                .indeterminate(true)
                .show(ui);
        });
        assert_eq!(
            find(&nodes, "Stage all").and_then(|n| n.toggled),
            Some(Toggled::Mixed)
        );
        let mut rect = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            rect = Checkbox::new(&mut on, "Stage all")
                .indeterminate(true)
                .show(ui)
                .rect;
        });
        testkit::click(&ctx, rect.center(), |ui| {
            Checkbox::new(&mut on, "Stage all")
                .indeterminate(true)
                .show(ui);
        });
        assert!(on, "clicking a partial selection selects everything");
    }

    #[test]
    fn a_switch_can_be_named_without_drawing_its_label() {
        let ctx = testkit::ctx();
        let shapes = frame(&ctx, Vec::new(), |ui| {
            Switch::new(&mut true, "Prune on fetch")
                .show_label(false)
                .show(ui);
        });
        assert!(
            !shapes.iter().any(|c| matches!(&c.shape, Shape::Text(_))),
            "no label is painted"
        );
        let nodes = spoken(&ctx, |ui| {
            Switch::new(&mut true, "Prune on fetch")
                .show_label(false)
                .show(ui);
        });
        let named: Vec<_> = nodes
            .iter()
            .filter(|n| n.label.as_deref() == Some("Prune on fetch"))
            .collect();
        assert_eq!(named.len(), 1, "named once: {nodes:?}");
        assert_eq!(named[0].role, Role::Switch);
    }

    #[test]
    fn disabled_toggles_ignore_clicks_and_keys() {
        let ctx = testkit::ctx();
        let (mut c, mut s) = (false, false);
        let show = |ui: &mut Ui, c: &mut bool, s: &mut bool| {
            let a = Checkbox::new(c, "Box").enabled(false).show(ui);
            let b = Switch::new(s, "Toggle").enabled(false).show(ui);
            ([a.rect, b.rect], a.id)
        };
        let mut rects = [Rect::NOTHING; 2];
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| {
            (rects, id) = show(ui, &mut c, &mut s);
        });
        for rect in rects {
            testkit::click(&ctx, rect.center(), |ui| {
                show(ui, &mut c, &mut s);
            });
        }
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, key(Key::Space), |ui| {
            show(ui, &mut c, &mut s);
        });
        assert!(!c && !s);
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut c, &mut s);
        });
        assert!(find(&nodes, "Box").is_some_and(|n| n.disabled));
        assert!(find(&nodes, "Toggle").is_some_and(|n| n.disabled));
    }

    #[test]
    fn switch_flips_and_speaks_as_a_switch() {
        let ctx = testkit::ctx();
        let mut on = false;
        let mut rect = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            rect = Switch::new(&mut on, "Prune").show(ui).rect;
        });
        testkit::click(&ctx, rect.center(), |ui| {
            Switch::new(&mut on, "Prune").show(ui);
        });
        assert!(on);
        let nodes = spoken(&ctx, |ui| {
            Switch::new(&mut on, "Prune").show(ui);
        });
        let n = find(&nodes, "Prune").expect("spoken");
        assert_eq!(n.role, Role::Switch);
        assert_eq!(n.toggled, Some(Toggled::True));
    }

    /// A11Y-4: a radio group holds its options (it was an empty group beside them).
    #[test]
    fn a_radio_group_holds_its_options() {
        let ctx = testkit::ctx();
        let options = [(1_u8, "One", None), (2, "Two", Some("The second"))];
        let mut v = 1_u8;
        let parts = testkit::parts(&ctx, "Count", |ui| {
            RadioGroup::new(&mut v, &options).label("Count").show(ui);
        });
        assert_eq!(parts, ["One", "Two"]);
    }

    #[test]
    fn radio_group_moves_with_arrows_and_wraps() {
        let ctx = testkit::ctx();
        let options = [
            (1_u8, "One", None),
            (2, "Two", Some("The second")),
            (3, "Three", None),
        ];
        let mut v = 1_u8;
        let show = |ui: &mut Ui, v: &mut u8| RadioGroup::new(v, &options).label("Count").show(ui);
        frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        // Tab lands on the chosen option, the group's only stop. Each key gets its own frame
        // and a quiet frame after it, as a press and its release do.
        let press = |k: Key, v: &mut u8| {
            let mut changed = false;
            frame(&ctx, key(k), |ui| changed = show(ui, v).changed());
            frame(&ctx, Vec::new(), |ui| {
                show(ui, v);
            });
            changed
        };
        press(Key::Tab, &mut v);
        assert!(press(Key::ArrowDown, &mut v));
        assert_eq!(v, 2);
        press(Key::ArrowDown, &mut v);
        assert_eq!(v, 3, "focus followed the choice");
        press(Key::ArrowRight, &mut v);
        assert_eq!(
            v, 1,
            "Down or Right from the last option wraps to the first"
        );
        press(Key::ArrowUp, &mut v);
        assert_eq!(v, 3, "Up from the first option wraps to the last");
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut v);
        });
        assert_eq!(
            find(&nodes, "Count").map(|n| n.role),
            Some(Role::RadioGroup)
        );
        let three = find(&nodes, "Three").expect("spoken");
        assert_eq!(three.role, Role::RadioButton);
        assert_eq!(three.toggled, Some(Toggled::True));
    }

    #[test]
    fn steps_wrap_both_ways() {
        assert_eq!(step(0, -1, 3), 2);
        assert_eq!(step(2, 1, 3), 0);
        assert_eq!(step(1, 1, 3), 2);
        assert_eq!(step(0, 1, 0), 0);
    }

    #[test]
    fn reduced_motion_lands_marks_and_thumb_at_once() {
        let ctx = testkit::ctx();
        let id = Id::new("mark");
        frame(&ctx, Vec::new(), |ui| {
            let ctx = ui.ctx();
            assert_eq!(mark(ctx, id, false, None), (0.0, 0.0));
            assert_eq!(crossfade(ctx, id.with("f"), false, FILL_FADE, None), 0.0);
        });
        frame(&ctx, Vec::new(), |ui| {
            let ctx = ui.ctx();
            assert_eq!(mark(ctx, id, true, None), (1.0, 1.0));
            assert_eq!(crossfade(ctx, id.with("f"), true, FILL_FADE, None), 1.0);
        });
        let mut on = false;
        let knob = |on: &mut bool| {
            let shapes = frame(&ctx, Vec::new(), |ui| {
                Switch::new(on, "").id_salt("k").show(ui);
            });
            let knob = theme::surfaces(&ctx).knob.top;
            shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Rect(r) if r.fill == knob && r.blur_width == 0.0 => Some(r.rect.left()),
                    _ => None,
                })
                .next_back()
        };
        let off = knob(&mut on).expect("knob painted");
        on = true;
        let at = knob(&mut on).expect("knob painted");
        assert_eq!(at - off, theme::metrics().switch_travel, "no frame between");
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the box is painted at exactly the token's side length"
    )]
    fn checked_fill_follows_the_style() {
        for style in theme::Style::ALL {
            let ctx = testkit::ctx_with(theme::ThemeId::Daylight, style, theme::Density::Dense);
            let s = theme::surfaces(&ctx);
            let side = theme::metrics().checkbox;
            let shapes = frame(&ctx, Vec::new(), |ui| {
                Checkbox::new(&mut true, "On").show(ui);
            });
            let painted = shapes.iter().any(|c| match &c.shape {
                Shape::Rect(r) => {
                    r.rect.width() == side
                        && (r.fill == s.primary.top || (!s.primary.is_flat() && r.brush.is_some()))
                }
                _ => false,
            });
            assert!(painted, "{style:?}");
        }
    }
}
