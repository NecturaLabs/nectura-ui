//! Buttons: [`Button`] (primary, secondary, ghost, danger), [`IconButton`], [`ToolButton`],
//! [`LinkButton`] and [`SplitButton`]. Design spec 4.1.
//!
//! This is the reference component: the others follow its shape. A builder holds the inputs,
//! `show` allocates the exact size, reads hover, press and focus through [`Interact`], paints the
//! surface with [`paint::surface`] (shadow, fill, highlight, ring) and the content (optionally
//! scaled while pressed), draws the focus ring and reports the AccessKit role and label.

use std::sync::Arc;

use egui::emath::TSTransform;
use egui::epaint::TextShape;
use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, FontSelection, Galley, Id, Painter, Pos2,
    Rect, Response, Sense, Shape, Stroke, TextWrapMode, Ui, Vec2, Widget, WidgetInfo, WidgetText,
    WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::Shortcut;
use super::overlay::TooltipExt;
use super::{ForceState, Interact, Size, Tone, anim, paint};
use crate::motion::{self, OUTCOME_HOLD, SHAKE, SHAKE_REACH, SHAKE_SWINGS};
use crate::theme::{self, Elevation, Fill, Palette, Style, ToolLayout};

/// What a [`Button`] is for. One primary per surface; danger confirms destruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ButtonKind {
    Primary,
    #[default]
    Secondary,
    Ghost,
    Danger,
}

impl ButtonKind {
    pub const ALL: [Self; 4] = [Self::Primary, Self::Secondary, Self::Ghost, Self::Danger];

    pub fn label(self) -> &'static str {
        match self {
            Self::Primary => "Primary",
            Self::Secondary => "Secondary",
            Self::Ghost => "Ghost",
            Self::Danger => "Danger",
        }
    }
}

/// `a` moved towards `b` by `t`, both ends.
pub(super) fn blend(a: Fill, b: Fill, t: f32) -> Fill {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    Fill {
        top: a.top.lerp_to_gamma(b.top, t),
        bottom: a.bottom.lerp_to_gamma(b.bottom, t),
    }
}

/// A control's kit tooltip: why it is disabled, else `tooltip` with the shortcut's keys as
/// keycaps; nothing when there is no text. A disabled control also tells screen readers it is
/// unavailable and why ([`super::a11y::unavailable`]), so call this after its `widget_info`.
/// On a `labelled` control (its name is on screen) a tooltip without a shortcut only describes
/// it, so it is [`super::overlay::Tooltip::extra`].
pub(super) fn tip(
    response: Response,
    enabled: bool,
    tooltip: Option<&str>,
    reason: Option<&str>,
    shortcut: Option<Shortcut>,
    labelled: bool,
) -> Response {
    if !enabled {
        super::a11y::unavailable(&response, reason);
    }
    let text = if enabled { tooltip } else { reason.or(tooltip) };
    match (text, shortcut) {
        (None, _) => response,
        (Some(t), Some(c)) if enabled => response.kit_tooltip(t).shortcut(c).show(),
        (Some(t), _) => {
            let why = !enabled && reason.is_some();
            response
                .kit_tooltip(t)
                .extra(labelled && !why)
                .unavailable(why)
                .show()
        }
    }
}

/// Splits a leading Phosphor glyph (Private Use Area) and its spacing off `text`, so labels written
/// as "<glyph>  Label" before the kit draw their glyph as the button's icon.
pub(crate) fn split_glyph(text: &str) -> (Option<String>, &str) {
    let mut chars = text.chars();
    match chars.next() {
        Some(c) if ('\u{E000}'..='\u{F8FF}').contains(&c) => {
            let rest = chars.as_str().trim_start();
            (Some(c.to_string()), rest)
        }
        _ => (None, text),
    }
}

/// Text in `font` laid out on one line.
pub(super) fn galley(ui: &Ui, text: WidgetText, font: FontId) -> Arc<Galley> {
    text.into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        FontSelection::FontId(font),
    )
}

/// Content shapes, scaled about `pivot` by `scale` (the press scale) before they are painted.
pub(super) fn paint_scaled(painter: &Painter, shapes: Vec<Shape>, pivot: Pos2, scale: f32) {
    if (scale - 1.0).abs() < 1e-3 {
        painter.extend(shapes);
        return;
    }
    let p = pivot.to_vec2();
    let tf = TSTransform::from_translation(p)
        * TSTransform::from_scaling(scale)
        * TSTransform::from_translation(-p);
    painter.extend(shapes.into_iter().map(|mut s| {
        s.transform(tf);
        s
    }));
}

/// How far a spinner has faded in over the icon (`None` while nothing runs). An action the user
/// just took shows its work at once: the spinner starts with the click and crossfades in over
/// `BASE` (full at once with reduced motion), never after a wait.
pub(super) fn spinner_fade(ctx: &egui::Context, id: Id, loading: bool) -> Option<f32> {
    let key = id.with("kit-loading-since");
    if !loading {
        ctx.data_mut(|d| d.remove::<f64>(key));
        return None;
    }
    let now = ctx.input(|i| i.time);
    let since = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
    let fade = motion::time(motion::BASE);
    if fade <= 0.0 {
        return Some(1.0);
    }
    let t = (crate::to_f32(now - since) / fade).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    Some(motion::EASE_OUT.at(t))
}

/// When a passive spinner (a field checking its value as the user types) may show: after
/// `LOADING_DELAY` of loading, so a quick check never flashes it (instantly with reduced motion).
pub(super) fn spinner_due(ctx: &egui::Context, id: Id, loading: bool) -> bool {
    let key = id.with("kit-loading-since");
    if !loading {
        ctx.data_mut(|d| d.remove::<f64>(key));
        return false;
    }
    let now = ctx.input(|i| i.time);
    let since = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(key, || now));
    let delay = motion::time(motion::LOADING_DELAY);
    if crate::to_f32(now - since) < delay {
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(delay));
        false
    } else {
        true
    }
}

// ---------------------------------------------------------------------------------- Operation

/// The outcome mark's size as it pops in, against its full size.
const POP_FROM: f32 = 0.6;

/// What the operation a button starts is doing, shown in the button's icon place:
/// `ToolButton::new(icon::ARROW_DOWN, "Fetch").operation(op)`. Pass it every frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Operation {
    /// At rest: the button's own icon.
    #[default]
    Idle,
    /// Running: a spinner from the click; clicks are ignored, focus stays.
    Running,
    /// Finished at `at` (`ctx.input(|i| i.time)` when the result arrived): a check in `success`
    /// for [`OUTCOME_HOLD`], then the icon again.
    Succeeded { at: f64 },
    /// Failed at `at`: a warning mark in `danger` that shakes once, for [`OUTCOME_HOLD`]. The
    /// error itself goes to a toast as usual.
    Failed { at: f64 },
}

impl Operation {
    /// The outcome of an operation that finished at `at`.
    pub fn finished(ok: bool, at: f64) -> Self {
        if ok {
            Self::Succeeded { at }
        } else {
            Self::Failed { at }
        }
    }

    fn running(self) -> bool {
        self == Self::Running
    }
}

/// What a button shows in its icon's place this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Mark {
    Icon,
    /// The spinner at `show` strength (0..=1), crossfading with the icon.
    Spinner {
        show: f32,
    },
    /// An outcome's mark at `show` strength (0..=1: popping in, then fading back to the icon),
    /// shifted `dx` sideways by the failure shake.
    Outcome {
        ok: bool,
        show: f32,
        dx: f32,
    },
}

/// What `operation` shows on the button `id` now, asking for the repaints that move it on.
fn mark(ctx: &egui::Context, id: Id, operation: Operation) -> Mark {
    let (ok, at) = match operation {
        Operation::Idle => {
            spinner_fade(ctx, id, false);
            return Mark::Icon;
        }
        Operation::Running => {
            return spinner_fade(ctx, id, true).map_or(Mark::Icon, |show| Mark::Spinner { show });
        }
        Operation::Succeeded { at } => (true, at),
        Operation::Failed { at } => (false, at),
    };
    spinner_fade(ctx, id, false);
    let age = crate::to_f32(ctx.input(|i| i.time) - at).max(0.0);
    if age >= OUTCOME_HOLD {
        return Mark::Icon;
    }
    if motion::reduced() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(OUTCOME_HOLD - age));
        return Mark::Outcome {
            ok,
            show: 1.0,
            dx: 0.0,
        };
    }
    let fade = motion::BASE;
    let show = (age / fade)
        .min((OUTCOME_HOLD - age) / fade)
        .clamp(0.0, 1.0);
    let dx = if ok || age >= SHAKE {
        0.0
    } else {
        let t = age / SHAKE;
        (t * SHAKE_SWINGS * std::f32::consts::TAU).sin() * SHAKE_REACH * (1.0 - t)
    };
    let still_until = OUTCOME_HOLD - fade;
    if show < 1.0 || dx != 0.0 || age < SHAKE.max(fade) || age >= still_until {
        ctx.request_repaint();
    } else {
        ctx.request_repaint_after(std::time::Duration::from_secs_f32(still_until - age));
    }
    Mark::Outcome {
        ok,
        show: motion::EASE_OUT.at(show),
        dx,
    }
}

/// Paints `mark` centred on `center`: the `icon` (if any) in `color`, the spinner of `spin`
/// radius, or the outcome glyph at `size` in `outcome` (success, danger), crossfading with the
/// icon.
#[expect(
    clippy::too_many_arguments,
    reason = "one paint step shared by three buttons"
)]
fn paint_mark(
    painter: &Painter,
    shapes: &mut Vec<Shape>,
    mark: Mark,
    icon: Option<&Arc<Galley>>,
    center: Pos2,
    size: f32,
    spin: f32,
    color: Color32,
    outcome: (Color32, Color32),
) {
    let icon_at = |shapes: &mut Vec<Shape>, alpha: f32| {
        if let Some(g) = icon {
            let text = TextShape::new(center - g.size() / 2.0, Arc::clone(g), color);
            shapes.push(text.with_opacity_factor(alpha).into());
        }
    };
    match mark {
        Mark::Icon => icon_at(shapes, 1.0),
        Mark::Spinner { show } => {
            if show < 1.0 {
                icon_at(shapes, 1.0 - show);
            }
            paint::spinner(
                painter,
                center,
                spin,
                paint::SPINNER_WIDTH,
                color.gamma_multiply(show),
            );
        }
        Mark::Outcome { ok, show, dx } => {
            if show < 1.0 {
                icon_at(shapes, 1.0 - show);
            }
            let (glyph, tint) = if ok {
                (icon::CHECK, outcome.0)
            } else {
                (icon::WARNING_CIRCLE, outcome.1)
            };
            let g = painter.layout_no_wrap(glyph.to_owned(), FontId::proportional(size), tint);
            let at = center + vec2(dx, 0.0);
            let mut s: Shape = TextShape::new(at - g.size() / 2.0, g, tint)
                .with_opacity_factor(show)
                .into();
            let scale = POP_FROM + (1.0 - POP_FROM) * show;
            if scale < 1.0 {
                let p = at.to_vec2();
                s.transform(
                    TSTransform::from_translation(p)
                        * TSTransform::from_scaling(scale)
                        * TSTransform::from_translation(-p),
                );
            }
            shapes.push(s);
        }
    }
}

/// The outcome marks' colours on a button whose label is `label`: `success` and `danger`, or
/// the label's own colour on a disabled button.
fn outcome_colors(p: &Palette, label: Color32, on_fill: bool) -> (Color32, Color32) {
    if on_fill {
        (label, label)
    } else {
        (p.success, p.danger)
    }
}

/// Reports a running operation to AccessKit as busy.
fn report_busy(ctx: &egui::Context, id: Id, operation: Operation) {
    if operation.running() {
        ctx.accesskit_node_builder(id, egui::accesskit::Node::set_busy);
    }
}

// ------------------------------------------------------------------------------------- Button

/// A labelled button. `Button::new("Fetch").primary().icon(icon::ARROW_DOWN).show(ui)`.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Button {
    text: WidgetText,
    kind: ButtonKind,
    size: Size,
    icon: Option<String>,
    icon_tone: Option<Tone>,
    trailing: Option<&'static str>,
    enabled: bool,
    loading: bool,
    operation: Option<Operation>,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    shortcut: Option<Shortcut>,
    show_shortcut: bool,
    min_width: f32,
    full_width: bool,
    plain_text: bool,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl Button {
    /// A secondary button labelled `text`. A leading Phosphor glyph in `text` becomes its icon.
    pub fn new(text: impl Into<WidgetText>) -> Self {
        let text = text.into();
        let (icon, text) = match &text {
            WidgetText::Text(s) => {
                let (glyph, rest) = split_glyph(s);
                let rest = WidgetText::from(rest.to_owned());
                (glyph, if s.is_empty() { text } else { rest })
            }
            _ => (None, text),
        };
        Self {
            text,
            kind: ButtonKind::Secondary,
            size: Size::Md,
            icon,
            icon_tone: None,
            trailing: None,
            enabled: true,
            loading: false,
            operation: None,
            tooltip: None,
            disabled_reason: None,
            shortcut: None,
            show_shortcut: false,
            min_width: 0.0,
            full_width: false,
            plain_text: false,
            id_salt: None,
            force: None,
        }
    }

    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn primary(self) -> Self {
        self.kind(ButtonKind::Primary)
    }
    pub fn secondary(self) -> Self {
        self.kind(ButtonKind::Secondary)
    }
    pub fn ghost(self) -> Self {
        self.kind(ButtonKind::Ghost)
    }
    pub fn danger(self) -> Self {
        self.kind(ButtonKind::Danger)
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    /// A leading Phosphor glyph, in the label's colour.
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    /// Colours the leading icon by `tone` (a destructive Secondary button's `danger` icon).
    pub fn icon_tone(mut self, tone: Tone) -> Self {
        self.icon_tone = Some(tone);
        self
    }
    /// A trailing caret: the button opens a menu.
    pub fn caret(mut self) -> Self {
        self.trailing = Some(icon::CARET_DOWN);
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// Busy: the spinner fades in over the icon from the click; clicks are ignored, focus and
    /// width are kept.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
    /// The operation this button runs: a spinner while it runs, then its outcome, in the icon's
    /// place (an icon slot is kept even without an icon, so the width never changes).
    pub fn operation(mut self, operation: Operation) -> Self {
        self.operation = Some(operation);
        self
    }
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    /// Why the button is disabled, shown as its tooltip while it is.
    pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
        self.disabled_reason = Some(text.into());
        self
    }
    /// The command this button runs: its chord joins the tooltip.
    pub fn shortcut(mut self, command: impl Into<Shortcut>) -> Self {
        self.shortcut = Some(command.into());
        self
    }
    /// Also show the shortcut's chord right of the label.
    pub fn show_shortcut(mut self, show: bool) -> Self {
        self.show_shortcut = show;
        self
    }
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
    pub fn full_width(mut self) -> Self {
        self.full_width = true;
        self
    }
    /// A ghost button's label in the body text colour, not the dimmed one: a value that can be
    /// clicked (a commit id) beside a muted label.
    pub fn plain_text(mut self) -> Self {
        self.plain_text = true;
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }

    /// Shows the button with `force`d states (the gallery and tests only).
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
        let height = self.size.height();
        let pad = if matches!(self.size, Size::Xs | Size::Sm) {
            m.button_pad_x_sm
        } else {
            m.button_pad_x
        };
        // The tonal primary and danger buttons hold their own with a semibold label.
        let font = if matches!(self.kind, ButtonKind::Primary | ButtonKind::Danger) {
            FontId::new(m.body_size, theme::semibold())
        } else {
            FontId::new(m.body_size, theme::medium())
        };
        let accessible = self.text.text().to_owned();
        let text = galley(ui, self.text, font.clone());
        let icon_font = FontId::proportional(m.icon_md);
        let operation = if self.loading {
            Operation::Running
        } else {
            self.operation.unwrap_or_default()
        };
        let spinner_slot = self.icon.is_none() && (self.loading || self.operation.is_some());
        let icon = self.icon.map(|g| {
            ui.painter()
                .layout_no_wrap(g, icon_font.clone(), Color32::PLACEHOLDER)
        });
        let chord = self
            .shortcut
            .as_ref()
            .filter(|_| self.show_shortcut)
            .map(|c| {
                ui.painter()
                    .layout_no_wrap(c.hint().to_owned(), font.clone(), Color32::PLACEHOLDER)
            });
        let trailing = self.trailing.map(|g| {
            ui.painter().layout_no_wrap(
                g.to_owned(),
                FontId::proportional(m.icon_xs),
                Color32::PLACEHOLDER,
            )
        });
        let icon_w = icon
            .as_ref()
            .map(|g| g.size().x)
            .or(spinner_slot.then_some(m.icon_md));
        let mut content = text.size().x;
        if let Some(w) = icon_w {
            content += w + if text.is_empty() { 0.0 } else { m.button_gap };
        }
        if let Some(c) = &chord {
            content += theme::SM + c.size().x;
        }
        if let Some(t) = &trailing {
            content += m.button_gap + t.size().x;
        }
        let mut width = (content + 2.0 * pad).max(self.min_width).max(height);
        if self.full_width {
            width = width.max(ui.available_width());
        }
        let sense = if operation.running() && enabled {
            Sense::focusable_noninteractive()
        } else if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, mut response) = match self.id_salt {
            Some(id) => {
                let rect = ui.allocate_space(vec2(width, height)).1;
                let response = ui.interact(rect, ui.id().with(id), sense);
                (rect, response)
            }
            None => ui.allocate_exact_size(vec2(width, height), sense),
        };
        let st = Interact::of(&ctx, &response, enabled, self.force);
        if ui.is_rect_visible(rect) {
            // A disabled button paints its own dimmed tokens at full strength, also in a disabled
            // `Ui` (whose fade `content_painter` takes back off).
            let painter = super::content_painter(ui);
            let ink_painter = painter.clone();
            let radius = CornerRadius::same(m.radius);
            let (fill, ring, level, label, focus) = match self.kind {
                // Disabled: no accent or danger fill (a dimmed one reads as a muddy, half-lit
                // button) but the flat, neutral `disabled_fill` in its `disabled_edge` ring, and
                // a label in `disabled_text`.
                ButtonKind::Primary | ButtonKind::Danger | ButtonKind::Secondary if !enabled => (
                    Some(Fill::flat(s.disabled_fill)),
                    Some(s.disabled_edge),
                    None,
                    p.disabled_text,
                    p.focus,
                ),
                ButtonKind::Ghost if !enabled => (None, None, None, p.disabled_text, p.focus),
                // Tonal, not solid: the control ground tinted with the accent (or danger), its
                // hairline and its label in that colour.
                ButtonKind::Primary => {
                    let (fill, ring, label) = p.tonal(p.accent, st.hover, st.press);
                    (
                        Some(fill),
                        Some(ring),
                        Some(Elevation::Control),
                        label,
                        p.focus,
                    )
                }
                ButtonKind::Danger => {
                    let (fill, ring, label) = p.tonal(p.danger, st.hover, st.press);
                    (
                        Some(fill),
                        Some(ring),
                        Some(Elevation::Control),
                        label,
                        p.danger_base,
                    )
                }
                ButtonKind::Secondary => (
                    Some(blend(
                        blend(s.raised, s.raised_hover, st.hover),
                        s.raised_pressed,
                        st.press,
                    )),
                    Some(s.raised_edge),
                    Some(Elevation::Control),
                    p.text,
                    p.focus,
                ),
                ButtonKind::Ghost => {
                    let ground = Color32::TRANSPARENT
                        .lerp_to_gamma(p.hover_strong(), st.hover)
                        .lerp_to_gamma(p.pressed(), st.press);
                    (
                        (ground.a() > 0).then_some(Fill::flat(ground)),
                        None,
                        None,
                        if self.plain_text {
                            p.text
                        } else {
                            p.text_dim.lerp_to_gamma(p.text, st.hover.max(st.press))
                        },
                        p.focus,
                    )
                }
            };
            match (fill, level) {
                (Some(f), Some(level)) => {
                    paint::surface(&painter, rect, radius, p, style, level, f, ring);
                }
                (Some(f), None) => {
                    paint::fill(&painter, rect, radius, f);
                    if let Some(ring) = ring {
                        paint::rim(&painter, rect, radius, ring);
                    }
                }
                _ => {}
            }
            if enabled && self.kind == ButtonKind::Ghost {
                paint::hover_mark(&painter, rect, radius, p, s, st.hover);
            }
            if st.focus > 0.0 {
                paint::focus_ring(
                    &painter,
                    rect,
                    radius,
                    focus.gamma_multiply(st.focus),
                    Color32::TRANSPARENT,
                );
            }
            let painter = ink_painter;
            let scale = anim::press_scale(&ctx, response.id, st.held, 0.97);
            let mut shapes = Vec::new();
            let mut x = rect.center().x - content / 2.0;
            let cy = rect.center().y;
            if let Some(w) = icon_w {
                let icon_color = match self.icon_tone {
                    Some(t) if enabled => t.color(p),
                    _ => label,
                };
                let on_fill = !enabled;
                paint_mark(
                    &painter,
                    &mut shapes,
                    mark(&ctx, response.id, operation),
                    icon.as_ref(),
                    pos2(x + w / 2.0, cy),
                    m.icon_md,
                    m.icon_md * 0.38,
                    icon_color,
                    outcome_colors(p, label, on_fill),
                );
                x += w + if text.is_empty() { 0.0 } else { m.button_gap };
            }
            shapes.push(Shape::galley(
                pos2(x, cy - text.size().y / 2.0),
                Arc::clone(&text),
                label,
            ));
            x += text.size().x;
            if let Some(c) = &chord {
                x += theme::SM;
                // A disabled label is already as faint as it may get.
                let hint = if enabled {
                    label.gamma_multiply(0.4)
                } else {
                    label
                };
                shapes.push(Shape::galley(
                    pos2(x, cy - c.size().y / 2.0),
                    Arc::clone(c),
                    hint,
                ));
                x += c.size().x;
            }
            if let Some(t) = &trailing {
                x += m.button_gap;
                shapes.push(Shape::galley(
                    pos2(x, cy - t.size().y / 2.0),
                    Arc::clone(t),
                    label,
                ));
            }
            paint_scaled(&painter, shapes, rect.center(), scale);
        }
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &accessible));
        report_busy(&ctx, response.id, operation);
        response = tip(
            response,
            enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
            self.shortcut,
            !accessible.is_empty(),
        );
        response
    }
}

impl Widget for Button {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// --------------------------------------------------------------------------------- IconButton

/// A square icon-only button. `label` is its tooltip and AccessKit name, in imperative form.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct IconButton {
    glyph: String,
    label: String,
    size: Size,
    tone: Option<Tone>,
    active: bool,
    enabled: bool,
    operation: Operation,
    disabled_reason: Option<String>,
    shortcut: Option<Shortcut>,
    min_target: f32,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

/// The side of a square [`IconButton`] of `size`, for layouts that make room for one.
pub fn icon_button_side(size: Size) -> f32 {
    let m = theme::metrics();
    match size {
        Size::Xs => 20.0,
        Size::Sm => m.control_sm + 2.0,
        Size::Md => m.control,
        Size::Lg => m.control_lg,
    }
}

impl IconButton {
    pub fn new(glyph: &str, label: impl Into<String>) -> Self {
        Self {
            glyph: glyph.to_owned(),
            label: label.into(),
            size: Size::Md,
            tone: None,
            active: false,
            enabled: true,
            operation: Operation::Idle,
            disabled_reason: None,
            shortcut: None,
            min_target: 0.0,
            id_salt: None,
            force: None,
        }
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    /// The smallest side of the area that answers the pointer and takes focus: a button drawn
    /// smaller than `side` keeps its look and gets this much hit area around it, centred
    /// (WCAG 2.5.8: [`theme::MIN_TARGET`]).
    pub fn min_target(mut self, side: f32) -> Self {
        self.min_target = side;
        self
    }
    /// Colours the icon by `tone` at rest (a destructive action's `danger`).
    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = Some(tone);
        self
    }
    /// Toggled on: the icon in `accent`.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }
    /// The operation this button runs: a spinner while it runs, then its outcome, in place of
    /// the icon.
    pub fn operation(mut self, operation: Operation) -> Self {
        self.operation = operation;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
        self.disabled_reason = Some(text.into());
        self
    }
    pub fn shortcut(mut self, command: impl Into<Shortcut>) -> Self {
        self.shortcut = Some(command.into());
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
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let side = icon_button_side(self.size);
        let (radius, glyph_size) = match self.size {
            Size::Xs => (m.radius_sm, 13.0),
            Size::Sm | Size::Md => (m.radius, m.icon_md),
            Size::Lg => (m.radius, m.icon_lg),
        };
        let sense = if !enabled {
            Sense::hover()
        } else if self.operation.running() {
            Sense::focusable_noninteractive()
        } else {
            Sense::click()
        };
        let target = side.max(self.min_target);
        let (hit, response) = match self.id_salt {
            Some(id) => {
                let rect = ui.allocate_space(Vec2::splat(target)).1;
                (rect, ui.interact(rect, ui.id().with(id), sense))
            }
            None => ui.allocate_exact_size(Vec2::splat(target), sense),
        };
        // What is drawn is `side` across, in the middle of the hit area.
        let rect = Rect::from_center_size(hit.center(), Vec2::splat(side));
        let st = Interact::of(&ctx, &response, enabled, self.force);
        if ui.is_rect_visible(rect) {
            let painter = super::content_painter(ui);
            let radius = CornerRadius::same(radius);
            let ground = Color32::TRANSPARENT
                .lerp_to_gamma(p.hover_strong(), st.hover)
                .lerp_to_gamma(p.pressed(), st.press);
            if !enabled {
                // Like every disabled control it keeps its shape: the flat `disabled_fill` in its
                // `disabled_edge` ring, so it does not read as an enabled icon a shade dimmer.
                let s = p.surfaces(theme::style());
                paint::fill(&painter, rect, radius, Fill::flat(s.disabled_fill));
                paint::rim(&painter, rect, radius, s.disabled_edge);
            } else if ground.a() > 0 {
                painter.rect_filled(rect, radius, ground);
            }
            if enabled {
                paint::hover_mark(&painter, rect, radius, p, p.s(), st.hover);
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
            let rest = if !enabled {
                p.disabled_text
            } else if self.active {
                p.accent
            } else {
                self.tone.map_or(p.text_dim, |t| t.color(p))
            };
            let color = if !enabled || self.active || self.tone.is_some() {
                rest
            } else {
                rest.lerp_to_gamma(p.text, st.hover.max(st.press))
            };
            let scale = anim::press_scale(&ctx, response.id, st.held, 0.94);
            let g = painter.layout_no_wrap(
                self.glyph,
                theme::icon_font(glyph_size, self.active && enabled),
                color,
            );
            let mut shapes = Vec::new();
            paint_mark(
                &painter,
                &mut shapes,
                mark(&ctx, response.id, self.operation),
                Some(&g),
                rect.center(),
                glyph_size,
                glyph_size * 0.38,
                color,
                outcome_colors(p, color, !enabled),
            );
            paint_scaled(&painter, shapes, rect.center(), scale);
        }
        let label = self.label;
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, &label));
        report_busy(&ctx, response.id, self.operation);
        tip(
            response,
            enabled,
            Some(&label),
            self.disabled_reason.as_deref(),
            self.shortcut,
            false,
        )
    }
}

impl Widget for IconButton {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// --------------------------------------------------------------------------------- HelpMark

/// A "?" after a label that explains it at length: hovering or focusing it shows `text` as a
/// tooltip at every Tooltips level, since that is all it does, and screen readers hear `text` as
/// its description.
#[derive(Debug)]
#[must_use = "call .show(ui)"]
pub struct HelpMark {
    about: String,
    text: String,
    notice: Option<bool>,
    apart: bool,
}

impl HelpMark {
    /// `about` names what it explains: the mark is announced as "About {about}".
    pub fn new(about: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            about: about.into(),
            text: text.into(),
            notice: None,
            apart: false,
        }
    }

    /// For an info mark shown beside an explanation mark: draws it in the accent colour, so the
    /// two cannot be mistaken for each other. A warning mark is already told apart by its colour.
    pub fn apart(mut self) -> Self {
        self.apart = true;
        self
    }

    /// A remark about the current state instead of an explanation: an info mark, or a warning
    /// mark in the warning colour, announced as "Note on {about}" or "Warning on {about}".
    pub fn notice(mut self, warn: bool) -> Self {
        self.notice = Some(warn);
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let side = theme::metrics().icon_md;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::FOCUSABLE);
        let st = Interact::of(&ctx, &response, true, None);
        if ui.is_rect_visible(rect) {
            let painter = super::content_painter(ui);
            if st.focus > 0.0 {
                paint::focus_ring(
                    &painter,
                    rect,
                    CornerRadius::from(side / 2.0),
                    p.focus.gamma_multiply(st.focus),
                    Color32::TRANSPARENT,
                );
            }
            let (glyph, color) = match self.notice {
                None => (icon::QUESTION, p.text_faint),
                Some(false) if self.apart => (icon::INFO, p.accent),
                Some(false) => (icon::INFO, p.text_faint),
                Some(true) => (icon::WARNING, p.warning),
            };
            let hover = if self.notice == Some(true) {
                color
            } else {
                p.text
            };
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                glyph,
                theme::icon_font(side, false),
                color.lerp_to_gamma(hover, st.hover.max(st.focus)),
            );
        }
        let name = match self.notice {
            None => format!("About {}", self.about),
            Some(false) => format!("Note on {}", self.about),
            Some(true) => format!("Warning on {}", self.about),
        };
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, &name));
        let description = self.text.clone();
        ctx.accesskit_node_builder(response.id, |n| n.set_description(description));
        response.kit_tooltip(self.text).always().show()
    }
}

// --------------------------------------------------------------------------------- ToolButton

/// An action-bar tool: icon over caption (Comfortable) or beside its label (Dense).
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct ToolButton {
    glyph: String,
    label: String,
    tooltip: Option<String>,
    badge: Option<String>,
    toggled: Option<bool>,
    operation: Operation,
    enabled: bool,
    disabled_reason: Option<String>,
    shortcut: Option<Shortcut>,
    layout: Option<ToolLayout>,
    min_width: f32,
    force: Option<ForceState>,
}

impl ToolButton {
    pub fn new(glyph: &str, label: impl Into<String>) -> Self {
        Self {
            glyph: glyph.to_owned(),
            label: label.into(),
            tooltip: None,
            badge: None,
            toggled: None,
            operation: Operation::Idle,
            enabled: true,
            disabled_reason: None,
            shortcut: None,
            layout: None,
            min_width: 0.0,
            force: None,
        }
    }
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    /// A short count beside the icon (`↓3`).
    pub fn badge(mut self, text: impl Into<String>) -> Self {
        self.badge = Some(text.into());
        self
    }
    /// The tool switches something on and off (a panel), and it is `on` now: it is announced
    /// as a toggle button, pressed or not. Without it the tool is a plain button.
    pub fn toggled(mut self, on: bool) -> Self {
        self.toggled = Some(on);
        self
    }
    /// The tool's operation is running: a spinner replaces the icon (`operation(Running)`).
    pub fn running(mut self, running: bool) -> Self {
        self.operation = if running {
            Operation::Running
        } else {
            Operation::Idle
        };
        self
    }
    /// The tool's operation: a spinner while it runs, then its outcome, in place of the icon.
    pub fn operation(mut self, operation: Operation) -> Self {
        self.operation = operation;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
        self.disabled_reason = Some(text.into());
        self
    }
    pub fn shortcut(mut self, command: impl Into<Shortcut>) -> Self {
        self.shortcut = Some(command.into());
        self
    }
    /// Overrides the density's layout (the gallery shows both).
    pub fn layout(mut self, layout: ToolLayout) -> Self {
        self.layout = Some(layout);
        self
    }
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
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
        let layout = self.layout.unwrap_or(m.tool_layout);
        let stacked = layout == ToolLayout::Stacked;
        // Tool anatomy follows the layout: Comfortable's for stacked tools, Dense's for inline
        // ones, whichever density is active (the gallery shows both).
        let t = if stacked {
            &theme::COMFORTABLE
        } else {
            &theme::DENSE
        };
        let (icon_size, caption_size, gap) = (t.icon_tool, t.tool_caption, t.tool_gap);
        let caption_font = FontId::new(caption_size, theme::medium());
        let caption =
            ui.painter()
                .layout_no_wrap(self.label.clone(), caption_font, Color32::PLACEHOLDER);
        let glyph = ui.painter().layout_no_wrap(
            self.glyph.clone(),
            theme::icon_font(icon_size, self.toggled == Some(true) && enabled),
            Color32::PLACEHOLDER,
        );
        let badge = self.badge.as_ref().map(|b| {
            ui.painter().layout_no_wrap(
                b.clone(),
                FontId::new(11.5, FontFamily::Monospace),
                Color32::PLACEHOLDER,
            )
        });
        let badge_w = badge.as_ref().map_or(0.0, |b| b.size().x + 3.0);
        let width = if stacked {
            (caption.size().x.max(glyph.size().x + badge_w) + 2.0 * t.tool_pad)
                .max(t.tool_min_width)
        } else {
            glyph.size().x + badge_w + gap + caption.size().x + 2.0 * t.tool_pad
        };
        let size = vec2(width.max(self.min_width), t.tool_height);
        let sense = if !enabled {
            Sense::hover()
        } else if self.operation.running() {
            Sense::focusable_noninteractive()
        } else {
            Sense::click()
        };
        let (rect, response) = ui.allocate_exact_size(size, sense);
        let st = Interact::of(&ctx, &response, enabled, self.force);
        if ui.is_rect_visible(rect) {
            let painter = super::content_painter(ui);
            let radius = CornerRadius::same(m.radius);
            if self.toggled == Some(true) {
                // A disabled toggled tool keeps its state on the neutral disabled fill.
                let ground = if enabled {
                    p.accent_tint()
                } else {
                    s.disabled_fill
                };
                painter.rect_filled(rect, radius, ground);
            }
            let lit = st.hover.max(st.press);
            if lit > 0.0 && self.toggled != Some(true) {
                if style == Style::Deeper {
                    let body = blend(s.raised_hover, s.raised_pressed, st.press);
                    let mut faded = painter.clone();
                    faded.multiply_opacity(lit);
                    paint::surface(
                        &faded,
                        rect,
                        radius,
                        p,
                        style,
                        Elevation::Control,
                        body,
                        Some(s.raised_edge),
                    );
                } else {
                    let ground = Color32::TRANSPARENT
                        .lerp_to_gamma(p.hover_strong(), st.hover)
                        .lerp_to_gamma(p.pressed(), st.press);
                    painter.rect_filled(rect, radius, ground);
                    if enabled {
                        paint::hover_mark(&painter, rect, radius, p, s, st.hover);
                    }
                }
            }
            if st.focus > 0.0 {
                paint::inset_ring(&painter, rect, radius, p.focus.gamma_multiply(st.focus));
            }
            let text_color = if enabled {
                p.text_dim.lerp_to_gamma(p.text, lit)
            } else {
                p.disabled_text
            };
            let icon_color = if self.toggled == Some(true) && enabled {
                p.accent
            } else {
                text_color
            };
            let scale = anim::press_scale(&ctx, response.id, st.held, 0.96);
            let mut shapes = Vec::new();
            let (icon_pos, caption_pos) = if stacked {
                let block = glyph.size().y + gap + caption.size().y;
                let top = rect.center().y - block / 2.0;
                let icon_x = rect.center().x - f32::midpoint(glyph.size().x, badge_w);
                (
                    pos2(icon_x, top),
                    pos2(
                        rect.center().x - caption.size().x / 2.0,
                        top + glyph.size().y + gap,
                    ),
                )
            } else {
                let x = rect.left() + t.tool_pad;
                (
                    pos2(x, rect.center().y - glyph.size().y / 2.0),
                    pos2(
                        x + glyph.size().x + badge_w + gap,
                        rect.center().y - caption.size().y / 2.0,
                    ),
                )
            };
            paint_mark(
                &painter,
                &mut shapes,
                mark(&ctx, response.id, self.operation),
                Some(&glyph),
                icon_pos + glyph.size() / 2.0,
                icon_size,
                icon_size * 0.36,
                icon_color,
                outcome_colors(p, icon_color, !enabled),
            );
            if let Some(b) = &badge {
                let at = pos2(
                    icon_pos.x + glyph.size().x + 3.0,
                    icon_pos.y + glyph.size().y / 2.0 - b.size().y / 2.0,
                );
                let badge_color = if enabled { p.accent } else { text_color };
                shapes.push(Shape::galley(at, Arc::clone(b), badge_color));
            }
            shapes.push(Shape::galley(caption_pos, caption, text_color));
            paint_scaled(&painter, shapes, rect.center(), scale);
        }
        let name = self.label;
        response.widget_info(|| match self.toggled {
            Some(on) => WidgetInfo::selected(WidgetType::Button, enabled, on, &name),
            None => WidgetInfo::labeled(WidgetType::Button, enabled, &name),
        });
        report_busy(&ctx, response.id, self.operation);
        tip(
            response,
            enabled,
            Some(self.tooltip.as_deref().unwrap_or(&name)),
            self.disabled_reason.as_deref(),
            self.shortcut,
            true,
        )
    }
}

impl Widget for ToolButton {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The 1 pt divider between tool button groups (`tool_divider`).
pub fn tool_divider(ui: &mut Ui) {
    let m = theme::metrics();
    let s = theme::surfaces(ui.ctx());
    let width = 2.0 * m.tool_divider_margin + paint::SEAM;
    let (rect, _) = ui.allocate_exact_size(vec2(width, m.tool_divider_height), Sense::hover());
    paint::seam_v(ui.painter(), rect.center().x, rect.y_range(), s.border);
}

// --------------------------------------------------------------------------------- LinkButton

/// Text that opens something: `accent2`, brightening and underlined on hover (at rest too where
/// the theme's `Design::link_underline` asks). A link to a web address ([`LinkButton::url`]) ends
/// in an arrow icon, shows the address in a tooltip and offers "Copy link" in its context menu
/// (open web addresses through `platform.rs`). Every link is at least [`theme::MIN_TARGET`]
/// tall, and Enter on a focused one clicks it.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct LinkButton {
    text: String,
    url: Option<String>,
    size: Option<f32>,
    max_text_width: Option<f32>,
    force: Option<ForceState>,
}

impl LinkButton {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            url: None,
            size: None,
            max_text_width: None,
            force: None,
        }
    }
    /// Leaves Canopy for the web page at `url`: appends "↗", names the address in a tooltip and
    /// offers to copy it.
    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }
    /// Text size (defaults to `body_size`).
    pub fn text_size(mut self, size: f32) -> Self {
        self.size = Some(size);
        self
    }
    /// Limits the visible text to this width, with an ellipsis; its accessible name stays whole.
    pub fn max_text_width(mut self, width: f32) -> Self {
        self.max_text_width = Some(width.max(0.0));
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let m = theme::metrics();
        let enabled = ui.is_enabled();
        let size = self.size.unwrap_or(m.body_size);
        let font = FontId::new(size, theme::medium());
        let mut text = egui::text::LayoutJob::simple_singleline(self.text.clone(), font, p.accent2);
        text.wrap.max_width = self.max_text_width.unwrap_or(f32::INFINITY);
        text.wrap.max_rows = 1;
        text.wrap.break_anywhere = true;
        let g = ui.painter().layout_job(text);
        // An external link's arrow is the icon font's, a little smaller than the text.
        let arrow = self.url.is_some().then(|| {
            ui.painter().layout_no_wrap(
                icon::ARROW_UP_RIGHT.to_owned(),
                FontId::proportional(size * 0.85),
                p.accent2,
            )
        });
        let gap = 3.0;
        let width = g.size().x + arrow.as_ref().map_or(0.0, |a| gap + a.size().x);
        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        // The target is as tall as WCAG 2.5.8 asks; the text sits in the middle of it.
        let (hit, response) =
            ui.allocate_exact_size(vec2(width, g.size().y.max(theme::MIN_TARGET)), sense);
        let rect = Rect::from_center_size(hit.center(), vec2(width, g.size().y));
        let st = Interact::of(&ctx, &response, enabled, self.force);
        if ui.is_rect_visible(rect) {
            let painter = &super::content_painter(ui);
            if st.focus > 0.0 {
                paint::focus_ring(
                    painter,
                    rect.expand2(vec2(3.0, 1.0)),
                    CornerRadius::same(m.radius_sm),
                    p.focus.gamma_multiply(st.focus),
                    Color32::TRANSPARENT,
                );
            }
            // Brighter under the pointer, a touch dimmer while pressed.
            let ink = if enabled {
                p.accent2
                    .lerp_to_gamma(p.text, 0.3 * st.hover)
                    .gamma_multiply(1.0 - 0.2 * st.press)
            } else {
                p.disabled_text
            };
            let baseline = rect.top() + g.rows.first().map_or(rect.height(), |r| r.rect().bottom());
            let text_w = g.size().x;
            painter.galley(rect.min, g, ink);
            if let Some(a) = arrow {
                let at = pos2(
                    rect.left() + text_w + gap,
                    rect.center().y - a.size().y / 2.0,
                );
                painter.galley(at, a, ink);
            }
            // The underline fades in with the hover (always shown where the theme asks).
            let underline = if theme::design(&ctx).link_underline {
                1.0
            } else {
                st.hover
            };
            if underline > 0.0 {
                paint::seam_h(
                    painter,
                    egui::Rangef::new(rect.left(), rect.left() + text_w),
                    baseline + 1.5,
                    ink.gamma_multiply(underline),
                );
            }
        }
        let mut response = if enabled {
            response.on_hover_cursor(egui::CursorIcon::PointingHand)
        } else {
            response
        };
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Link, enabled, &self.text));
        if let (Some(url), true) = (&self.url, enabled) {
            response = response.kit_tooltip(url.clone()).show();
            super::menu::link_menu(&response, url, "Copy link");
        }
        response
    }
}

impl Widget for LinkButton {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// -------------------------------------------------------------------------------- SplitButton

/// An action and a choice of what it does: the main half runs the chosen option, the caret half
/// opens a menu of options. The returned response is the main half's (`clicked` = run it).
#[derive(Debug)]
#[must_use = "call .show(ui)"]
pub struct SplitButton<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    options: &'a [(T, &'a str)],
    kind: ButtonKind,
    enabled: bool,
    loading: bool,
    min_width: f32,
    force: Option<ForceState>,
}

impl<'a, T: PartialEq + Clone> SplitButton<'a, T> {
    pub fn new(value: &'a mut T, options: &'a [(T, &'a str)]) -> Self {
        Self {
            value,
            options,
            kind: ButtonKind::Primary,
            enabled: true,
            loading: false,
            min_width: 0.0,
            force: None,
        }
    }
    pub fn primary(mut self) -> Self {
        self.kind = ButtonKind::Primary;
        self
    }
    pub fn secondary(mut self) -> Self {
        self.kind = ButtonKind::Secondary;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
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
        let label = self
            .options
            .iter()
            .find(|(v, _)| v == self.value)
            .or(self.options.first())
            .map_or("", |(_, l)| *l);
        let font = FontId::new(m.body_size, theme::medium());
        let text = ui
            .painter()
            .layout_no_wrap(label.to_owned(), font, Color32::PLACEHOLDER);
        let caret_w = 22.0;
        let height = m.control;
        let main_w = (text.size().x + 2.0 * m.button_pad_x).max(self.min_width - caret_w);
        let (rect, _) = ui.allocate_exact_size(vec2(main_w + caret_w, height), Sense::hover());
        let main_rect = Rect::from_min_max(rect.min, pos2(rect.left() + main_w, rect.bottom()));
        let caret_rect = Rect::from_min_max(pos2(main_rect.right(), rect.top()), rect.max);
        let id = ui.id().with(("kit-split", label));
        let sense = if enabled && !self.loading {
            Sense::click()
        } else if enabled {
            Sense::focusable_noninteractive()
        } else {
            Sense::hover()
        };
        let main = ui.interact(main_rect, id.with("main"), sense);
        let caret = ui.interact(
            caret_rect,
            id.with("caret"),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        let force = self.force;
        let a = Interact::of(&ctx, &main, enabled, force.filter(|f| !f.open));
        let b = Interact::of(
            &ctx,
            &caret,
            enabled,
            force.filter(|f| f.open).map(|_| ForceState::HOVER),
        );
        let (rest, hover, pressed, on, level) = match self.kind {
            ButtonKind::Secondary | ButtonKind::Ghost => (
                s.raised,
                s.raised_hover,
                s.raised_pressed,
                p.text,
                Elevation::Control,
            ),
            _ => {
                let (rest, _, on) = p.tonal(p.accent, 0.0, 0.0);
                (
                    rest,
                    p.tonal(p.accent, 1.0, 0.0).0,
                    p.tonal(p.accent, 1.0, 1.0).0,
                    on,
                    Elevation::Control,
                )
            }
        };
        let (rest, hover, pressed, on) = if enabled {
            (rest, hover, pressed, on)
        } else {
            // Disabled like a Button: the flat, neutral disabled fill.
            let off = Fill::flat(s.disabled_fill);
            (off, off, off, p.disabled_text)
        };
        if ui.is_rect_visible(rect) {
            let painter = super::content_painter(ui);
            let radius = CornerRadius::same(m.radius);
            let ring = Some(if enabled && self.kind == ButtonKind::Primary {
                p.tonal(p.accent, 0.0, 0.0).1
            } else {
                s.raised_edge
            });
            if enabled {
                paint::surface(&painter, rect, radius, p, style, level, rest, ring);
            } else {
                paint::fill(&painter, rect, radius, rest);
                paint::rim(&painter, rect, radius, s.disabled_edge);
            }
            for (half, st, corners) in [
                (
                    main_rect,
                    a,
                    CornerRadius {
                        ne: 0,
                        se: 0,
                        ..radius
                    },
                ),
                (
                    caret_rect,
                    b,
                    CornerRadius {
                        nw: 0,
                        sw: 0,
                        ..radius
                    },
                ),
            ] {
                let f = blend(blend(rest, hover, st.hover), pressed, st.press);
                if st.hover > 0.0 || st.press > 0.0 {
                    paint::fill(
                        &painter,
                        half.shrink(if ring.is_some() { 1.0 } else { 0.0 }),
                        corners,
                        f,
                    );
                }
                if st.focus > 0.0 {
                    paint::focus_ring(
                        &painter,
                        half,
                        corners,
                        p.focus.gamma_multiply(st.focus),
                        Color32::TRANSPARENT,
                    );
                }
            }
            let seam = if enabled {
                Palette::tint(on, 64)
            } else {
                s.disabled_edge
            };
            painter.vline(
                caret_rect.left(),
                egui::Rangef::new(rect.top() + 5.0, rect.bottom() - 5.0),
                Stroke::new(1.0, seam),
            );
            if let Some(show) = spinner_fade(&ctx, main.id, self.loading) {
                paint::spinner(
                    &painter,
                    pos2(
                        main_rect.left() + m.button_pad_x / 2.0 + 4.0,
                        main_rect.center().y,
                    ),
                    m.icon_md * 0.33,
                    paint::SPINNER_WIDTH,
                    on.gamma_multiply(show),
                );
            }
            painter.galley(
                main_rect.center() - text.size() / 2.0,
                Arc::clone(&text),
                on,
            );
            painter.text(
                caret_rect.center(),
                Align2::CENTER_CENTER,
                icon::CARET_DOWN,
                FontId::proportional(m.icon_xs),
                on,
            );
        }
        // The options as a kit menu under the whole button, at least as wide, the chosen one
        // checked. Forced open (the gallery) it opens to the right, clear of what follows.
        let forced_open = force.is_some_and(|f| f.open) && ui.is_rect_visible(rect);
        let popup = egui::Popup::menu(&caret)
            .id(id.with("menu"))
            .width(rect.width())
            .anchor(rect)
            .align(if forced_open {
                egui::RectAlign::RIGHT_START
            } else {
                egui::RectAlign::BOTTOM_START
            })
            .align_alternatives(&[egui::RectAlign::TOP_START]);
        let popup = if forced_open { popup.open(true) } else { popup };
        let value = self.value;
        let options = self.options;
        super::menu::menu_in(&caret, popup, |m| {
            for (v, l) in options {
                let chosen = *v == *value;
                if m.item("", *l).checked(chosen).show(m.ui).clicked() {
                    *value = v.clone();
                    m.ui.close();
                }
            }
        });
        main.widget_info(|| WidgetInfo::labeled(WidgetType::Button, enabled, label));
        caret.widget_info(|| WidgetInfo::labeled(WidgetType::ComboBox, enabled, "More options"));
        main
    }
}

// ------------------------------------------------------------------------------------ gallery

/// The gallery's Buttons section: every variant in every state and size, then the icon, tool,
/// link and split buttons.
pub(crate) fn gallery(ui: &mut Ui) {
    let states: [(&str, Option<ForceState>, bool, bool); 6] = [
        ("Rest", None, true, false),
        ("Hover", Some(ForceState::HOVER), true, false),
        ("Pressed", Some(ForceState::PRESSED), true, false),
        ("Focus", Some(ForceState::FOCUS), true, false),
        ("Disabled", None, false, false),
        ("Loading", None, true, true),
    ];
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let small = |ui: &mut Ui, text: &str| {
        ui.label(
            egui::RichText::new(text)
                .size(m.small_size)
                .color(p.text_faint),
        );
    };
    for kind in ButtonKind::ALL {
        small(ui, kind.label());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(theme::SM, theme::SM);
            for (i, (name, force, enabled, loading)) in states.iter().enumerate() {
                let b = Button::new(*name)
                    .kind(kind)
                    .enabled(*enabled)
                    .loading(*loading)
                    .icon(icon::CHECK)
                    .tooltip("Run it")
                    .disabled_reason("Nothing to run")
                    .id_salt(("gallery-button", kind, i));
                match force {
                    Some(f) => b.show_forced(ui, *f),
                    None => b.show(ui),
                };
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(theme::SM, theme::SM);
            for (i, size) in [Size::Sm, Size::Md, Size::Lg].into_iter().enumerate() {
                Button::new(format!("{size:?}"))
                    .kind(kind)
                    .size(size)
                    .id_salt(("gallery-size", kind, i))
                    .show(ui);
            }
            Button::new("Shortcut")
                .kind(kind)
                .shortcut("Ctrl+Enter")
                .show_shortcut(true)
                .id_salt(("gallery-chord", kind))
                .show(ui);
            Button::new("Menu")
                .kind(kind)
                .caret()
                .id_salt(("gallery-caret", kind))
                .show(ui);
        });
        ui.add_space(theme::XS);
    }
    small(ui, "Secondary with a danger icon");
    Button::new("Delete branch…")
        .secondary()
        .icon(icon::TRASH)
        .icon_tone(Tone::Danger)
        .id_salt("gallery-destructive")
        .show(ui);
    ui.add_space(theme::XS);
    small(
        ui,
        "Icon buttons: Xs, Sm, Md, Lg, hover, pressed, focus, active, danger, disabled",
    );
    ui.horizontal_wrapped(|ui| {
        for (i, size) in Size::ALL.into_iter().enumerate() {
            IconButton::new(icon::X, "Close")
                .size(size)
                .id_salt(("gi", i))
                .show(ui);
        }
        let gear = |salt: &str| IconButton::new(icon::GEAR, "Settings").id_salt(salt.to_owned());
        gear("gi-h").show_forced(ui, ForceState::HOVER);
        gear("gi-p").show_forced(ui, ForceState::PRESSED);
        gear("gi-f").show_forced(ui, ForceState::FOCUS);
        IconButton::new(icon::FUNNEL, "Filter")
            .active(true)
            .id_salt("gi-a")
            .show(ui);
        IconButton::new(icon::TRASH, "Delete")
            .tone(Tone::Danger)
            .id_salt("gi-d")
            .show(ui);
        gear("gi-x")
            .enabled(false)
            .disabled_reason("Settings are open")
            .shortcut("Ctrl+,")
            .show(ui);
    });
    ui.add_space(theme::XS);
    for layout in [ToolLayout::Stacked, ToolLayout::Inline] {
        small(ui, &format!("Tool buttons, {layout:?}"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = m.toolbar_gap;
            let tool = |glyph: &str, label: &str| ToolButton::new(glyph, label).layout(layout);
            tool(icon::ARROW_DOWN, "Pull").badge("↓3").show(ui);
            tool(icon::ARROW_UP, "Push").show_forced(ui, ForceState::HOVER);
            tool(icon::ARROWS_CLOCKWISE, "Fetch").show_forced(ui, ForceState::PRESSED);
            tool_divider(ui);
            tool(icon::GIT_BRANCH, "Branch").show_forced(ui, ForceState::FOCUS);
            tool(icon::STACK, "Stash").toggled(true).show(ui);
            tool(icon::ARROWS_CLOCKWISE, "Sync").running(true).show(ui);
            tool(icon::TAG, "Tag")
                .enabled(false)
                .disabled_reason("Select a commit first")
                .shortcut("Ctrl+Enter")
                .min_width(m.tool_min_width)
                .show(ui);
        });
    }
    ui.add_space(theme::XS);
    gallery_operations(ui, &small);
    ui.add_space(theme::XS);
    small(ui, "Links");
    ui.horizontal_wrapped(|ui| {
        LinkButton::new("Release notes").show(ui);
        LinkButton::new("Open on GitHub")
            .url("https://example.com/releases")
            .show_forced(ui, ForceState::HOVER);
        LinkButton::new("Focused link").show_forced(ui, ForceState::FOCUS);
        LinkButton::new("Small link")
            .text_size(m.small_size)
            .show(ui);
    });
    ui.add_space(theme::XS);
    small(ui, "Split buttons: closed, open");
    let id = ui.id().with("gallery-split");
    let mut choice = ui.ctx().data(|d| d.get_temp::<u8>(id)).unwrap_or(0);
    let options = [(0_u8, "Commit"), (1, "Commit and push")];
    ui.horizontal(|ui| {
        SplitButton::new(&mut choice, &options)
            .primary()
            .enabled(true)
            .loading(false)
            .min_width(150.0)
            .show(ui);
        ui.add_space(theme::LG);
        let mut open = 1_u8;
        SplitButton::new(&mut open, &options)
            .secondary()
            .show_forced(ui, ForceState::OPEN);
    });
    // Room for the open split button's menu.
    ui.add_space(64.0);
    ui.ctx().data_mut(|d| d.insert_temp(id, choice));
}

/// How long the gallery's Fetch runs before it finishes.
const GALLERY_RUN: f64 = 1.2;

/// The gallery's operation states, held mid-outcome, and a Fetch that runs one when clicked
/// (every other run fails).
fn gallery_operations(ui: &mut Ui, small: &impl Fn(&mut Ui, &str)) {
    small(
        ui,
        "Operations: running, succeeded, failed; Fetch runs one (every other run fails)",
    );
    let now = ui.input(|i| i.time);
    // Half way through the hold: past the pop-in and the shake, before the fade back.
    let shown = now - f64::from(OUTCOME_HOLD / 2.0);
    let id = ui.id().with("gallery-operation");
    let (started, runs) = ui
        .ctx()
        .data(|d| d.get_temp::<(Option<f64>, u32)>(id))
        .unwrap_or((None, 0));
    let live = match started {
        None => Operation::Idle,
        Some(t) if now - t < GALLERY_RUN => {
            ui.ctx().request_repaint();
            Operation::Running
        }
        Some(t) => Operation::finished(runs % 2 == 1, t + GALLERY_RUN),
    };
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(theme::SM, theme::SM);
        for (i, op) in [
            Operation::Running,
            Operation::Succeeded { at: shown },
            Operation::Failed { at: shown },
        ]
        .into_iter()
        .enumerate()
        {
            Button::new("Push")
                .primary()
                .icon(icon::ARROW_UP)
                .operation(op)
                .id_salt(("gallery-op", i))
                .show(ui);
            Button::new("Pull")
                .operation(op)
                .id_salt(("gallery-op-plain", i))
                .show(ui);
            IconButton::new(icon::ARROWS_CLOCKWISE, "Refresh")
                .operation(op)
                .id_salt(("gallery-op-icon", i))
                .show(ui);
        }
        clicked = ToolButton::new(icon::ARROWS_CLOCKWISE, "Fetch")
            .layout(ToolLayout::Inline)
            .operation(live)
            .show(ui)
            .clicked();
    });
    if clicked {
        ui.ctx()
            .data_mut(|d| d.insert_temp(id, (Some(now), runs + 1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::testkit;

    /// One frame; returns what it painted.
    fn frame(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out.shapes
    }

    #[test]
    fn leading_glyphs_become_icons() {
        let label = format!("{}  Stage all", icon::PLUS);
        let (g, rest) = split_glyph(&label);
        assert_eq!(g.as_deref(), Some(icon::PLUS));
        assert_eq!(rest, "Stage all");
        assert_eq!(split_glyph("Fetch"), (None, "Fetch"));
    }

    #[test]
    fn buttons_follow_the_metrics_and_report_their_role() {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        for density in theme::Density::ALL {
            for style in Style::ALL {
                theme::apply(
                    &ctx,
                    theme::ThemeId::Midnight,
                    theme::Appearance {
                        style,
                        density,
                        ..theme::Appearance::STILL
                    },
                );
                let m = theme::metrics();
                let mut heights = Vec::new();
                for _ in 0..2 {
                    heights.clear();
                    let _ = frame(&ctx, Vec::new(), |ui| {
                        for kind in ButtonKind::ALL {
                            heights.push(Button::new("Fetch").kind(kind).show(ui).rect.height());
                        }
                        heights.push(Button::new("Fetch").size(Size::Sm).show(ui).rect.height());
                        heights.push(IconButton::new(icon::X, "Close").show(ui).rect.height());
                    });
                }
                assert_eq!(&heights[..4], &[m.control; 4], "{density:?} {style:?}");
                assert_eq!(heights[4], m.control_sm);
                assert_eq!(heights[5], m.control);
            }
        }
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
    }

    #[test]
    fn disabled_and_loading_buttons_ignore_clicks() {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        for (enabled, loading, expect) in [
            (true, false, true),
            (false, false, false),
            (true, true, false),
        ] {
            let mut clicked = false;
            let mut rect = Rect::NOTHING;
            for i in 0..4 {
                let events = match i {
                    2 => vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: egui::Modifiers::default(),
                        },
                    ],
                    3 => vec![egui::Event::PointerButton {
                        pos: rect.center(),
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::default(),
                    }],
                    _ => Vec::new(),
                };
                let _ = frame(&ctx, events, |ui| {
                    let r = Button::new("Commit")
                        .primary()
                        .enabled(enabled)
                        .loading(loading)
                        .show(ui);
                    rect = r.rect;
                    clicked |= r.clicked();
                });
            }
            assert_eq!(clicked, expect, "enabled {enabled}, loading {loading}");
        }
    }

    #[test]
    fn disabled_buttons_keep_a_readable_label_in_every_theme() {
        for id in theme::ThemeId::ALL {
            for style in Style::ALL {
                let ctx = testkit::ctx_with(id, style, theme::Density::Comfortable);
                let s = id.palette().surfaces(style);
                let grounds = [s.pane, s.panel, s.overlay];
                let off = |w: f64, what: &str| {
                    assert!(w >= 3.0, "{id:?} {style:?} disabled {what}: {w:.2}:1");
                };
                for kind in ButtonKind::ALL {
                    let w = testkit::weakest_label(&ctx, &grounds, move |ui: &mut Ui| {
                        Button::new("Commit")
                            .kind(kind)
                            .icon(icon::CHECK)
                            .enabled(false)
                            .show(ui);
                    });
                    off(w, kind.label());
                }
                let w = testkit::weakest_label(&ctx, &grounds, |ui: &mut Ui| {
                    IconButton::new(icon::GEAR, "Settings")
                        .enabled(false)
                        .show(ui);
                });
                off(w, "icon button");
                let w = testkit::weakest_label(&ctx, &grounds, |ui: &mut Ui| {
                    ToolButton::new(icon::TAG, "Tag").enabled(false).show(ui);
                });
                off(w, "tool button");
                let w = testkit::weakest_label(&ctx, &grounds, |ui: &mut Ui| {
                    let mut v = 0_u8;
                    SplitButton::new(&mut v, &[(0, "Commit"), (1, "Commit and push")])
                        .enabled(false)
                        .show(ui);
                });
                off(w, "split button");
            }
        }
    }

    /// A disabled filled button sits on the flat, neutral `disabled_fill` inside its
    /// `disabled_edge` ring (theme-design "Disabled tokens"), with no shadow and no dimmed accent,
    /// in every theme and style, disabled itself or inside a disabled `Ui`.
    #[test]
    fn disabled_filled_buttons_use_the_neutral_disabled_fill() {
        for id in theme::ThemeId::ALL {
            for style in Style::ALL {
                let ctx = testkit::ctx_with(id, style, theme::Density::Comfortable);
                let p = id.palette();
                let s = p.surfaces(style);
                let paint = |kind: ButtonKind, enabled: bool, in_disabled_ui: bool| {
                    let shapes = frame(&ctx, Vec::new(), |ui| {
                        ui.add_enabled_ui(!in_disabled_ui, |ui| {
                            Button::new("Commit").kind(kind).enabled(enabled).show(ui);
                        });
                    });
                    let mut fills = Vec::new();
                    let mut rings = Vec::new();
                    let mut shadow = false;
                    for c in &shapes {
                        if let Shape::Rect(r) = &c.shape {
                            if r.blur_width > 0.0 {
                                shadow = true;
                            } else {
                                fills.push(r.fill);
                                rings.push(r.stroke.color);
                            }
                        }
                    }
                    (fills, rings, shadow)
                };
                let tonal = |c: Color32| {
                    let (fill, ring, _) = p.tonal(c, 0.0, 0.0);
                    (fill.top, ring)
                };
                if style == Style::Standard {
                    // The tonal look: the accent-tinted ground in its accent hairline.
                    let (fills, rings, _) = paint(ButtonKind::Primary, true, false);
                    let (fill, ring) = tonal(p.accent);
                    assert!(fills.contains(&fill), "{id:?}: {fills:?}");
                    assert!(rings.contains(&ring), "{id:?}: {rings:?}");
                }
                for kind in [
                    ButtonKind::Primary,
                    ButtonKind::Danger,
                    ButtonKind::Secondary,
                ] {
                    for (enabled, in_disabled_ui) in [(false, false), (true, true)] {
                        let (fills, rings, shadow) = paint(kind, enabled, in_disabled_ui);
                        let at =
                            format!("{id:?} {style:?} {kind:?} (disabled Ui: {in_disabled_ui})");
                        assert!(fills.contains(&s.disabled_fill), "{at}: {fills:?}");
                        assert!(rings.contains(&s.disabled_edge), "{at}: {rings:?}");
                        assert!(!shadow, "{at}: a disabled button casts a shadow");
                        for accent in [tonal(p.accent).0, tonal(p.danger).0] {
                            let muddy = accent.gamma_multiply(theme::DISABLED_ALPHA);
                            assert!(
                                !fills.contains(&muddy) && !fills.contains(&accent),
                                "{at} keeps an accent: {fills:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// A disabled icon-only button keeps its shape, the flat `disabled_fill` in its
    /// `disabled_edge` ring like every other disabled control, so it does not read as an enabled
    /// one in a slightly dimmer colour; an enabled one at rest is a bare icon.
    #[test]
    fn a_disabled_icon_button_sits_on_the_disabled_fill_in_its_ring() {
        for id in theme::ThemeId::ALL {
            for style in Style::ALL {
                let ctx = testkit::ctx_with(id, style, theme::Density::Comfortable);
                let p = id.palette();
                let s = p.surfaces(style);
                let paint = |enabled: bool, in_disabled_ui: bool| {
                    let shapes = frame(&ctx, Vec::new(), |ui| {
                        ui.add_enabled_ui(!in_disabled_ui, |ui| {
                            IconButton::new(icon::CARET_LEFT, "Scroll left")
                                .size(Size::Sm)
                                .enabled(enabled)
                                .show(ui);
                        });
                    });
                    let mut rects = Vec::new();
                    for c in &shapes {
                        if let Shape::Rect(r) = &c.shape {
                            rects.push((r.fill, r.stroke.color));
                        }
                    }
                    rects
                };
                for (enabled, in_disabled_ui) in [(false, false), (true, true)] {
                    let rects = paint(enabled, in_disabled_ui);
                    let at = format!("{id:?} {style:?} (disabled Ui: {in_disabled_ui})");
                    assert!(
                        rects.iter().any(|(fill, _)| *fill == s.disabled_fill),
                        "{at}: {rects:?}"
                    );
                    assert!(
                        rects.iter().any(|(_, ring)| *ring == s.disabled_edge),
                        "{at}: {rects:?}"
                    );
                }
                let rects = paint(true, false);
                assert!(
                    rects
                        .iter()
                        .all(|(fill, ring)| *fill != s.disabled_fill && *ring != s.disabled_edge),
                    "{id:?} {style:?}: an enabled icon button shows the disabled look: {rects:?}"
                );
            }
        }
    }

    /// One frame at `time` seconds with `events`; returns what it painted.
    fn frame_at(
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> Vec<egui::epaint::ClippedShape> {
        let mut out = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out.shapes
    }

    /// Every glyph a frame painted: its text, colour, opacity and centre.
    fn glyphs(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, Color32, f32, Pos2)> {
        fn walk(shape: &Shape, out: &mut Vec<(String, Color32, f32, Pos2)>) {
            match shape {
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                Shape::Text(t) => out.push((
                    t.galley.text().to_owned(),
                    t.fallback_color,
                    t.opacity_factor,
                    t.visual_bounding_rect().center(),
                )),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for c in shapes {
            walk(&c.shape, &mut out);
        }
        out
    }

    /// Whether a frame painted a spinner (its faint circle).
    fn spun(shapes: &[egui::epaint::ClippedShape]) -> bool {
        shapes.iter().any(|c| matches!(c.shape, Shape::Circle(_)))
    }

    /// The operation buttons, each showing `op`; returns their responses.
    fn operation_buttons(ui: &mut Ui, op: Operation) -> [Response; 4] {
        [
            Button::new("Push")
                .primary()
                .icon(icon::ARROW_UP)
                .operation(op)
                .show(ui),
            Button::new("Pull").operation(op).show(ui),
            IconButton::new(icon::ARROWS_CLOCKWISE, "Refresh")
                .operation(op)
                .show(ui),
            ToolButton::new(icon::ARROW_DOWN, "Fetch")
                .operation(op)
                .show(ui),
        ]
    }

    /// Override O1: busy, success and failure never change a button's size, also a button with
    /// no icon of its own.
    #[test]
    fn operation_buttons_keep_their_size_in_every_state() {
        let ctx = testkit::ctx();
        let sizes = |op: Operation, t: f64| {
            let mut rects = [Rect::NOTHING; 4];
            for _ in 0..2 {
                let _ = frame_at(&ctx, t, Vec::new(), |ui| {
                    rects = operation_buttons(ui, op).map(|r| r.rect);
                });
            }
            rects.map(|r| r.size())
        };
        let rest = sizes(Operation::Idle, 10.0);
        for (op, t) in [
            (Operation::Running, 10.0),
            (Operation::Running, 11.0),
            (Operation::Succeeded { at: 11.0 }, 11.5),
            (Operation::Failed { at: 11.0 }, 11.1),
        ] {
            assert_eq!(sizes(op, t), rest, "{op:?}");
        }
    }

    /// A running operation's button keeps focus but ignores clicks, as a loading button does.
    #[test]
    fn a_running_operation_ignores_clicks() {
        let ctx = testkit::ctx();
        let mut clock = 1.0;
        for op in [Operation::Idle, Operation::Running] {
            let mut rects = [Rect::NOTHING; 4];
            let mut clicked = [false; 4];
            let mut show = |ui: &mut Ui, rects: &mut [Rect; 4]| {
                for (i, r) in operation_buttons(ui, op).iter().enumerate() {
                    rects[i] = r.rect;
                    clicked[i] |= r.clicked();
                }
            };
            let mut tick = || {
                clock += 0.1;
                clock
            };
            let _ = frame_at(&ctx, tick(), Vec::new(), |ui| show(ui, &mut rects));
            for i in 0..4 {
                let at = rects[i].center();
                let _ = frame_at(&ctx, tick(), testkit::press(at), |ui| {
                    show(ui, &mut rects);
                });
                let _ = frame_at(&ctx, tick(), testkit::release(at), |ui| {
                    show(ui, &mut rects);
                });
            }
            assert_eq!(clicked, [op == Operation::Idle; 4], "{op:?}");
        }
    }

    /// A running operation's spinner starts with the click and is at full strength after `BASE`
    /// (at once with reduced motion); nothing running has none.
    #[test]
    fn the_spinner_fades_in_from_the_click() {
        let ctx = testkit::ctx();
        theme::apply(
            &ctx,
            theme::ThemeId::Midnight,
            theme::Appearance {
                reduce_motion: false,
                ..theme::Appearance::STILL
            },
        );
        let id = Id::new("spinner-fade");
        let at = |time: f64, loading: bool| {
            let mut fade = None;
            let _ = frame_at(&ctx, time, Vec::new(), |_| {
                fade = spinner_fade(&ctx, id, loading);
            });
            fade
        };
        assert_eq!(at(1.0, false), None);
        assert_eq!(at(2.0, true), Some(0.0), "begins at the click, not later");
        let mid = at(2.0 + f64::from(motion::time(motion::BASE)) / 2.0, true);
        assert!(mid.is_some_and(|f| f > 0.0 && f < 1.0), "{mid:?}");
        let full = at(2.0 + f64::from(motion::time(motion::BASE)) + 0.01, true);
        assert_eq!(full, Some(1.0));
        assert_eq!(at(3.0, false), None);
        motion::set_reduced(true);
        assert_eq!(at(4.0, true), Some(1.0), "full strength at once");
        motion::set_reduced(false);
    }

    /// Running shows the spinner (at once); success then shows a check in
    /// `success`, failure a warning mark in `danger` (the label colour on a filled button), each
    /// for `OUTCOME_HOLD`, and then the icon is back.
    #[test]
    fn an_operation_shows_its_spinner_then_its_outcome_then_the_icon() {
        let ctx = testkit::ctx();
        theme::apply(
            &ctx,
            theme::ThemeId::Midnight,
            theme::Appearance {
                reduce_motion: false,
                ..theme::Appearance::STILL
            },
        );
        let p = *theme::palette(&ctx);
        let tool = |op: Operation, t: f64| {
            frame_at(&ctx, t, Vec::new(), |ui| {
                ToolButton::new(icon::ARROW_DOWN, "Fetch")
                    .operation(op)
                    .show(ui);
            })
        };
        let _ = tool(Operation::Idle, 1.0);
        assert!(spun(&tool(Operation::Running, 2.0)), "a spinner at once");
        assert!(spun(&tool(Operation::Running, 2.5)), "and while it runs");
        let has = |shapes: &[egui::epaint::ClippedShape], glyph: &str| {
            glyphs(shapes)
                .into_iter()
                .find(|(t, _, o, _)| t == glyph && *o > 0.0)
                .map(|(_, c, o, _)| (c, o))
        };
        let done = 3.0;
        let mid = done + f64::from(OUTCOME_HOLD / 2.0);
        let after = done + f64::from(OUTCOME_HOLD) + 0.01;
        let ok = tool(Operation::Succeeded { at: done }, mid);
        assert_eq!(has(&ok, icon::CHECK), Some((p.success, 1.0)));
        assert_eq!(
            has(&ok, icon::ARROW_DOWN),
            None,
            "the check replaces the icon"
        );
        let failed = tool(Operation::Failed { at: done }, mid);
        assert_eq!(has(&failed, icon::WARNING_CIRCLE), Some((p.danger, 1.0)));
        for op in [
            Operation::Succeeded { at: done },
            Operation::Failed { at: done },
        ] {
            let rest = tool(op, after);
            assert_eq!(
                has(&rest, icon::ARROW_DOWN),
                Some((p.text_dim, 1.0)),
                "{op:?}"
            );
            assert_eq!(has(&rest, icon::CHECK), None);
            assert_eq!(has(&rest, icon::WARNING_CIRCLE), None);
        }
        let primary = frame_at(&ctx, after + 1.0, Vec::new(), |ui| {
            Button::new("Push")
                .primary()
                .icon(icon::ARROW_UP)
                .operation(Operation::Succeeded { at: after })
                .show(ui);
        });
        assert_eq!(
            has(&primary, icon::CHECK),
            Some((p.success, 1.0)),
            "a tonal primary has no fill to keep its own colour on"
        );
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
    }

    /// The outcome pops in and the failure mark shakes sideways at first; with reduced motion it
    /// shows at full strength and in place from the first frame to the end of the hold.
    #[test]
    fn reduced_motion_shows_outcomes_without_a_pop_or_a_shake() {
        let ctx = testkit::ctx();
        let mark_at = |op: Operation, t: f64| {
            let marked = frame_at(&ctx, t, Vec::new(), |ui| {
                IconButton::new(icon::ARROWS_CLOCKWISE, "Refresh")
                    .operation(op)
                    .show(ui);
            });
            let rest = frame_at(&ctx, t, Vec::new(), |ui| {
                IconButton::new(icon::ARROWS_CLOCKWISE, "Refresh").show(ui);
            });
            let icon_x = glyphs(&rest)[0].3.x;
            glyphs(&marked)
                .into_iter()
                .find(|(g, ..)| g == icon::WARNING_CIRCLE)
                .map(|(_, _, o, c)| (o, c.x - icon_x))
        };
        let failed = Operation::Failed { at: 5.0 };
        for reduced in [false, true] {
            theme::apply(
                &ctx,
                theme::ThemeId::Midnight,
                theme::Appearance {
                    reduce_motion: reduced,
                    ..theme::Appearance::STILL
                },
            );
            let (o, _) = mark_at(failed, 5.02).expect("a mark on the first frames");
            let (_, dx) = mark_at(failed, 5.0 + f64::from(SHAKE) / 4.0).expect("a mark");
            if reduced {
                assert!((o - 1.0).abs() < f32::EPSILON, "full strength at once: {o}");
                assert!(dx.abs() < 0.01, "no shake: {dx}");
            } else {
                assert!(o < 1.0, "pops in: {o}");
                assert!(dx.abs() > 0.5, "shakes: {dx}");
            }
            let (o, dx) = mark_at(failed, 5.0 + f64::from(OUTCOME_HOLD) - 0.02).expect("held");
            assert_eq!(
                (o < 1.0, dx.abs() < 0.01),
                (!reduced, true),
                "end of the hold"
            );
        }
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
    }

    /// A11Y-15: a tool is a plain button unless it switches something on and off.
    #[test]
    fn a_tool_is_a_toggle_only_when_it_switches_something() {
        use egui::accesskit::Toggled;

        use super::super::testkit;
        let ctx = testkit::ctx();
        let nodes = testkit::spoken(&ctx, |ui| {
            ToolButton::new(icon::ARROWS_CLOCKWISE, "Fetch").show(ui);
            ToolButton::new(icon::STACK, "Stashes")
                .toggled(true)
                .show(ui);
            ToolButton::new(icon::SIDEBAR, "Sidebar")
                .toggled(false)
                .show(ui);
        });
        let toggled = |label| testkit::find(&nodes, label).expect(label).toggled;
        assert_eq!(toggled("Fetch"), None, "Fetch is a plain button");
        assert_eq!(toggled("Stashes"), Some(Toggled::True));
        assert_eq!(toggled("Sidebar"), Some(Toggled::False));
    }
    /// F1: at the default Helpful level a toolbar tool with a shortcut shows its tooltip (a
    /// shortcut is what the screen does not show); one without says only what its label does,
    /// so it waits for All.
    /// A help mark only explains, so its text shows whatever the Tooltips level, on hover and on
    /// keyboard focus, and a screen reader hears it as the mark's description.
    #[test]
    fn a_help_mark_explains_on_hover_and_focus_at_every_level() {
        use crate::kit::overlay::{TooltipLevel, set_tooltip_level};
        use crate::kit::testkit;
        let text = "Older commits use it too.";
        set_tooltip_level(TooltipLevel::Off);
        assert_eq!(
            testkit::painted_after_hover(pos2(8.0, 8.0), text, |ui| {
                let _ = HelpMark::new("Dates", text).show(ui);
            }),
            1,
            "hovering shows the explanation even with tooltips off"
        );
        let ctx = testkit::ctx();
        let mut painted = 0;
        for i in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let r = HelpMark::new("Dates", text).show(ui);
                if i == 0 {
                    r.request_focus();
                }
            });
            out.textures_delta.clear();
            painted = out
                .shapes
                .iter()
                .filter(|c| matches!(&c.shape, Shape::Text(t) if t.galley.text() == text))
                .count();
        }
        set_tooltip_level(TooltipLevel::Helpful);
        assert_eq!(painted, 1, "keyboard focus shows the explanation");
        let nodes = testkit::spoken(&testkit::ctx(), |ui| {
            let _ = HelpMark::new("Dates", text).show(ui);
        });
        let mark = testkit::find(&nodes, "About Dates").expect("the mark is named");
        assert_eq!(mark.description.as_deref(), Some(text));
    }

    /// An info mark set apart from an explanation mark beside it is drawn in the accent colour;
    /// a plain one stays faint like the explanation mark.
    #[test]
    fn an_info_mark_set_apart_is_drawn_in_the_accent_colour() {
        use crate::kit::testkit;
        let glyph_colour = |apart: bool| {
            let ctx = testkit::ctx();
            let p = theme::palette(&ctx);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mark = HelpMark::new("Pictures", "Shown beside authors.").notice(false);
                let _ = if apart { mark.apart() } else { mark }.show(ui);
            });
            out.textures_delta.clear();
            let colour = out
                .shapes
                .iter()
                .find_map(|c| match &c.shape {
                    Shape::Text(t) if t.galley.text() == icon::INFO => Some(t.fallback_color),
                    _ => None,
                })
                .expect("the info glyph is painted");
            (colour, p.accent, p.text_faint)
        };
        let (plain, _, faint) = glyph_colour(false);
        assert_eq!(plain, faint);
        let (apart, accent, _) = glyph_colour(true);
        assert_eq!(apart, accent);
    }

    #[test]
    fn a_tool_with_a_shortcut_shows_its_tooltip_at_the_default_level() {
        use crate::kit::overlay::{TooltipLevel, set_tooltip_level};
        use crate::kit::testkit;
        let tip = "Fetch all remotes";
        let tool = |ui: &mut Ui, shortcut: Option<Shortcut>| {
            let mut tool = ToolButton::new(icon::ARROWS_CLOCKWISE, "Fetch").tooltip(tip);
            if let Some(c) = shortcut {
                tool = tool.shortcut(c);
            }
            let _ = tool.show(ui);
        };
        let at = pos2(30.0, 20.0);
        set_tooltip_level(TooltipLevel::Helpful);
        assert_eq!(
            testkit::painted_after_hover(at, tip, |ui| tool(ui, Some("f".into()))),
            1,
            "Helpful shows a tool's tooltip with its shortcut"
        );
        assert_eq!(
            testkit::painted_after_hover(at, tip, |ui| tool(ui, None)),
            0,
            "Helpful leaves out a description of a labelled tool"
        );
        set_tooltip_level(TooltipLevel::All);
        assert_eq!(
            testkit::painted_after_hover(at, tip, |ui| tool(ui, None)),
            1
        );
        set_tooltip_level(TooltipLevel::Helpful);
    }
    fn link_nodes(ctx: &egui::Context, add: impl FnMut(&mut Ui)) -> Vec<testkit::Spoken> {
        testkit::spoken(ctx, add)
    }

    /// WCAG 2.5.8: a link is at least 24 points tall, and a link to a web page also ends in an
    /// arrow.
    #[test]
    fn a_link_is_24_points_tall_and_a_web_link_ends_in_an_arrow() {
        let ctx = testkit::ctx();
        let (mut plain, mut web) = (Rect::NOTHING, Rect::NOTHING);
        let _ = frame(&ctx, Vec::new(), |ui| {
            plain = LinkButton::new("Details")
                .text_size(theme::metrics().small_size)
                .show(ui)
                .rect;
            web = LinkButton::new("Details")
                .text_size(theme::metrics().small_size)
                .url("https://example.com/")
                .show(ui)
                .rect;
        });
        assert!(plain.height() >= theme::MIN_TARGET, "{plain:?}");
        assert!(web.height() >= theme::MIN_TARGET, "{web:?}");
        assert!(web.width() > plain.width() + 4.0, "the arrow: {web:?}");
    }

    /// A web link shows its address in a tooltip at the default level and offers "Copy link" in
    /// its context menu; a link inside Canopy does neither.
    #[test]
    fn a_web_link_names_its_address_and_copies_it() {
        use crate::kit::overlay::{TooltipLevel, set_tooltip_level};
        let url = "https://example.com/releases";
        set_tooltip_level(TooltipLevel::Helpful);
        let link = |ui: &mut Ui, url: Option<&str>| {
            let mut link = LinkButton::new("Releases");
            if let Some(url) = url {
                link = link.url(url);
            }
            link.show(ui);
        };
        let at = pos2(20.0, 12.0);
        assert_eq!(
            testkit::painted_after_hover(at, url, |ui| link(ui, Some(url))),
            1,
            "the tooltip names the address"
        );
        assert_eq!(
            testkit::painted_after_hover(at, url, |ui| link(ui, None)),
            0
        );

        let ctx = testkit::ctx();
        for (target, offered) in [(Some(url), true), (None, false)] {
            let ctx = if offered { &ctx } else { &testkit::ctx() };
            let _ = frame(ctx, Vec::new(), |ui| link(ui, target));
            let _ = frame(ctx, testkit::secondary_click(at), |ui| link(ui, target));
            let nodes = link_nodes(ctx, |ui| link(ui, target));
            assert_eq!(
                testkit::find(&nodes, "Copy link").is_some(),
                offered,
                "{target:?}"
            );
        }
        // Choosing it copies the address.
        let item = testkit::find(&link_nodes(&ctx, |ui| link(ui, Some(url))), "Copy link")
            .and_then(|n| n.bounds)
            .expect("the item")
            .center();
        testkit::copied(&ctx, testkit::press(item), |ui| link(ui, Some(url)));
        let copied = testkit::copied(&ctx, testkit::release(item), |ui| link(ui, Some(url)));
        assert_eq!(copied, vec![url.to_owned()]);
    }

    /// A link takes Tab focus and Enter on it clicks it.
    #[test]
    fn enter_on_a_focused_link_clicks_it() {
        let ctx = testkit::ctx();
        let mut clicks = 0;
        let mut show = |ctx: &egui::Context, events: Vec<egui::Event>| {
            let _ = frame(ctx, events, |ui| {
                if LinkButton::new("Details").show(ui).clicked() {
                    clicks += 1;
                }
            });
        };
        show(&ctx, Vec::new());
        show(&ctx, testkit::key(egui::Key::Tab));
        show(&ctx, Vec::new());
        show(&ctx, testkit::key(egui::Key::Enter));
        show(&ctx, Vec::new());
        assert_eq!(clicks, 1);
    }
}
