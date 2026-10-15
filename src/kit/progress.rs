//! [`Progress`] (determinate, indeterminate and the one-point line under search inputs),
//! [`Spinner`] and [`Skeleton`]. Design spec 4.10.
//!
//! All three repaint only while they are painted (`request_repaint_after(16 ms)` from their own
//! paint), so a hidden or finished indicator costs nothing. With reduced motion the indeterminate
//! fill stands still at the start and says "Working…", the spinner pulses instead of turning (its
//! status must still show) and skeletons freeze at the midpoint of their pulse.

use std::time::Duration;

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Mesh, Painter, Pos2, Rect, Response, Sense, Shape,
    Stroke, Ui, Widget, WidgetInfo, WidgetType, pos2, vec2,
};

use super::{ForceState, Size};
use crate::motion;
use crate::theme::{self, Palette};

/// Width of the indeterminate fill, as a share of the track.
const INDETERMINATE_SHARE: f32 = 0.34;
/// The least a started bar shows, so it reads as begun.
const MIN_STARTED: f32 = 0.02;
/// The line variant's band width, as a share of the line.
const LINE_SHARE: f32 = 0.5;
/// Skeleton bar widths, cycling.
const SKELETON_WIDTHS: [f32; 3] = [0.40, 0.62, 0.78];
/// A skeleton bar is the `pressed` overlay laid this many times over its ground at rest and at
/// the top of its pulse.
const SKELETON_LAYERS: (i32, i32) = (2, 3);
/// The label reduced motion adds to an indeterminate bar.
pub const WORKING: &str = "Working…";

/// Repaints in 16 ms: the indicators' own loop, only while they are painted.
fn keep_moving(ctx: &egui::Context) {
    ctx.request_repaint_after(Duration::from_millis(16));
}

/// The phase (0 to 1) of a loop of `period` seconds at the current time.
fn loop_phase(ctx: &egui::Context, period: f32) -> f32 {
    let t = ctx.input(|i| i.time);
    crate::to_f32((t % f64::from(period)) / f64::from(period))
}

/// A fill that eases towards larger values over `PROGRESS_FILL` and snaps to smaller ones, so the bar
/// never visibly runs backwards.
fn forward(ctx: &egui::Context, id: Id, target: f32) -> f32 {
    #[derive(Clone, Copy)]
    struct Fill {
        from: f32,
        to: f32,
        at: f64,
    }
    let key = id.with("kit-progress-fill");
    let now = ctx.input(|i| i.time);
    let seconds = motion::time(motion::PROGRESS_FILL);
    let (shown, moving) = ctx.data_mut(|d| {
        let st = d.get_temp_mut_or_insert_with(key, || Fill {
            from: target,
            to: target,
            at: now,
        });
        let at_now = |st: &Fill| {
            if seconds <= 0.0 {
                return st.to;
            }
            let k = (crate::to_f32(now - st.at) / seconds).clamp(0.0, 1.0);
            st.from + (st.to - st.from) * motion::EASE_OUT.at(k)
        };
        if target > st.to {
            *st = Fill {
                from: at_now(st),
                to: target,
                at: now,
            };
        } else if target < st.to {
            *st = Fill {
                from: target,
                to: target,
                at: now,
            };
        }
        let shown = at_now(st);
        (shown, (shown - st.to).abs() > 1e-4)
    });
    if moving {
        ctx.request_repaint();
    }
    shown
}

/// Whether a loading indicator should show: only after `LOADING_DELAY` of loading, and then for
/// at least `LOADING_MIN` even if loading ends sooner (no flash). `id` keeps the state. An
/// indicator that is drawn only while something loads (not asked at all once it has loaded)
/// starts a new wait the next time it is asked after a pass without it.
pub fn loading_visible(ctx: &egui::Context, id: Id, loading: bool) -> bool {
    #[derive(Clone, Copy, Default)]
    struct Gate {
        since: Option<f64>,
        shown_at: Option<f64>,
        /// The pass that last asked.
        pass: Option<u64>,
    }
    let key = id.with("kit-loading-gate");
    let now = ctx.input(|i| i.time);
    let pass = ctx.cumulative_pass_nr();
    let delay = f64::from(motion::time(motion::LOADING_DELAY));
    let min = f64::from(motion::time(motion::LOADING_MIN));
    let (visible, wake) = ctx.data_mut(|d| {
        let g = d.get_temp_mut_or_default::<Gate>(key);
        if g.pass.is_some_and(|last| last + 1 < pass) {
            g.since = None;
            g.shown_at = None;
        }
        g.pass = Some(pass);
        if loading {
            let since = *g.since.get_or_insert(now);
            if now - since >= delay {
                g.shown_at.get_or_insert(now);
                (true, None)
            } else {
                (false, Some(delay - (now - since)))
            }
        } else {
            g.since = None;
            match g.shown_at {
                Some(at) if now - at < min => (true, Some(min - (now - at))),
                _ => {
                    g.shown_at = None;
                    (false, None)
                }
            }
        }
    });
    if let Some(s) = wake {
        ctx.request_repaint_after(Duration::from_secs_f64(s.max(0.0)));
    }
    visible
}

// ------------------------------------------------------------------------------------ Progress

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Determinate(f32),
    Indeterminate,
    /// A 1 pt sweeping band under search inputs and the palette.
    Line,
}

/// A progress bar. `Progress::new(0.4).label("Receiving objects").value_text("12 of 340")`.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Progress {
    kind: Kind,
    label: Option<String>,
    value_text: Option<String>,
    width: Option<f32>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl Progress {
    /// A determinate bar at `fraction` (0 to 1).
    pub fn new(fraction: f32) -> Self {
        Self::of(Kind::Determinate(if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        }))
    }

    /// Work of unknown length: a fill sweeping along the track.
    pub fn indeterminate() -> Self {
        Self::of(Kind::Indeterminate)
    }

    /// The 1 pt band under search inputs and the palette while results load.
    pub fn line() -> Self {
        Self::of(Kind::Line)
    }

    fn of(kind: Kind) -> Self {
        Self {
            kind,
            label: None,
            value_text: None,
            width: None,
            id_salt: None,
            force: None,
        }
    }

    /// What is in progress, shown above the bar and spoken as its name.
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }
    /// The amount, right of the label ("62%", "12 of 340").
    pub fn value_text(mut self, text: impl Into<String>) -> Self {
        self.value_text = Some(text.into());
        self
    }
    /// The bar's width (default: the available width).
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    /// Shows the bar with `force.progress` as the sweep's phase (the gallery and tests only).
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let m = theme::metrics();
        let reduced = motion::reduced();
        let width = self.width.unwrap_or_else(|| ui.available_width()).max(24.0);
        let value_text = match (&self.value_text, self.kind) {
            (Some(v), _) => Some(v.clone()),
            (None, Kind::Indeterminate) if reduced => Some(WORKING.to_owned()),
            _ => None,
        };
        let label_font = FontId::new(m.meta_size, theme::medium());
        let value_font = if self.kind == Kind::Indeterminate && self.value_text.is_none() {
            FontId::new(m.meta_size, egui::FontFamily::Proportional)
        } else {
            FontId::new(m.sha_size, egui::FontFamily::Monospace)
        };
        let has_row = self.label.is_some() || value_text.is_some();
        let row_h = if has_row {
            m.meta_size + theme::XS + 3.0
        } else {
            0.0
        };
        let bar_h = if self.kind == Kind::Line {
            1.0
        } else {
            m.progress_height
        };
        let (rect, response) = ui.allocate_exact_size(vec2(width, row_h + bar_h), Sense::hover());
        let id = self.id_salt.map_or(response.id, |s| ui.id().with(s));
        let bar = Rect::from_min_size(pos2(rect.left(), rect.bottom() - bar_h), vec2(width, bar_h));
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if let Some(l) = &self.label {
                painter.text(
                    pos2(rect.left(), rect.top()),
                    Align2::LEFT_TOP,
                    l,
                    label_font,
                    p.text_dim,
                );
            }
            if let Some(v) = &value_text {
                let color = if self.value_text.is_some() {
                    p.text
                } else {
                    p.text_dim
                };
                painter.text(
                    pos2(rect.right(), rect.top()),
                    Align2::RIGHT_TOP,
                    v,
                    value_font,
                    color,
                );
            }
            let phase = self.force.and_then(|f| f.progress);
            match self.kind {
                Kind::Determinate(f) => {
                    let shown = if self.force.is_some() {
                        f
                    } else {
                        forward(&ctx, id, f)
                    };
                    paint_track(painter, bar, s.switch_off);
                    if f > 0.0 {
                        let w = (shown.max(MIN_STARTED) * width).min(width);
                        paint_span(painter, bar, bar.left(), bar.left() + w, s.primary.top);
                    }
                }
                Kind::Indeterminate => {
                    paint_track(painter, bar, s.switch_off);
                    let share = INDETERMINATE_SHARE * width;
                    let t = match phase {
                        Some(t) => t,
                        None if reduced => 0.25,
                        None => {
                            keep_moving(&ctx);
                            loop_phase(&ctx, motion::INDETERMINATE_PERIOD)
                        }
                    };
                    // From one width before the track to three widths in (-100% to 300%).
                    let left = bar.left() - share + 4.0 * share * motion::EASE_IN_OUT.at(t);
                    paint_span(painter, bar, left, left + share, s.primary.top);
                }
                Kind::Line => {
                    let (waited, _) = super::overlay::presence(&ctx, id);
                    let t = match phase {
                        Some(t) => Some(t),
                        None if reduced => Some(0.5),
                        None if waited < motion::LINE_DELAY => {
                            ctx.request_repaint_after(Duration::from_secs_f32(
                                motion::LINE_DELAY - waited,
                            ));
                            None
                        }
                        None => {
                            keep_moving(&ctx);
                            Some(
                                ((waited - motion::LINE_DELAY) % motion::LINE_PERIOD)
                                    / motion::LINE_PERIOD,
                            )
                        }
                    };
                    if let Some(t) = t {
                        let band = LINE_SHARE * width;
                        let left = bar.left() - band + (width + band) * motion::EASE_IN_OUT.at(t);
                        paint_band(&painter.with_clip_rect(bar), bar, left, band, p.text_faint);
                    }
                }
            }
        }
        let name = self.label.clone().unwrap_or_else(|| "Progress".to_owned());
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(WidgetType::ProgressIndicator, true, &name);
            if let Kind::Determinate(f) = self.kind {
                info.value = Some(f64::from(f) * 100.0);
            }
            info
        });
        response
    }
}

impl Widget for Progress {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

fn paint_track(painter: &Painter, bar: Rect, color: Color32) {
    painter.rect_filled(bar, CornerRadius::same(theme::FULL), color);
}

/// The part of `[left, right]` inside the track, with fully rounded ends (so a fill entering or
/// leaving the track keeps the track's own rounded end).
fn paint_span(painter: &Painter, bar: Rect, left: f32, right: f32, color: Color32) {
    let (l, r) = (left.max(bar.left()), right.min(bar.right()));
    if r - l <= 0.5 {
        return;
    }
    let span = Rect::from_min_max(pos2(l, bar.top()), pos2(r, bar.bottom()));
    painter.rect_filled(span, CornerRadius::same(theme::FULL), color);
}

/// The line variant's band: transparent, `color`, transparent across `width` from `left`.
fn paint_band(painter: &Painter, bar: Rect, left: f32, width: f32, color: Color32) {
    let mut mesh = Mesh::default();
    let clear = Color32::TRANSPARENT;
    for (i, (x, c)) in [
        (left, clear),
        (left + width / 2.0, color),
        (left + width, clear),
    ]
    .into_iter()
    .enumerate()
    {
        mesh.colored_vertex(pos2(x, bar.top()), c);
        mesh.colored_vertex(pos2(x, bar.bottom()), c);
        if i > 0 {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a mesh holds far fewer than 2^32 vertices"
            )]
            let b = (2 * i) as u32;
            mesh.add_triangle(b - 2, b - 1, b);
            mesh.add_triangle(b - 1, b, b + 1);
        }
    }
    painter.add(Shape::mesh(mesh));
}

// ------------------------------------------------------------------------------------- Spinner

/// A spinner: a 270 degree arc in `focus` over a faint full circle, one turn per 0.8 s.
/// `Spinner::new().size(Size::Sm).label("pull requests").active(loading).show(ui)`.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Spinner {
    size: Size,
    label: Option<String>,
    active: Option<bool>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

impl Spinner {
    pub fn new() -> Self {
        Self {
            size: Size::Md,
            label: None,
            active: None,
            id_salt: None,
            force: None,
        }
    }
    /// Xs 12, Sm 14, Md 16 (default), Lg 20.
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    /// What is loading: spoken as "Loading <thing>".
    pub fn label(mut self, thing: impl Into<String>) -> Self {
        self.label = Some(thing.into());
        self
    }
    /// Shows the spinner only after `LOADING_DELAY` of `loading` and keeps it at least
    /// `LOADING_MIN` ([`loading_visible`]); the space stays reserved either way. Without this the
    /// spinner always shows.
    pub fn active(mut self, loading: bool) -> Self {
        self.active = Some(loading);
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    /// Shows the spinner at turn `force.progress` (the gallery and tests only).
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    /// The spinner's diameter and stroke width.
    pub fn geometry(size: Size) -> (f32, f32) {
        match size {
            Size::Xs => (12.0, 1.5),
            Size::Sm => (14.0, 1.5),
            Size::Md => (theme::metrics().spinner, 2.0),
            Size::Lg => (20.0, 2.0),
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let (side, stroke) = Self::geometry(self.size);
        let (rect, response) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
        let id = self.id_salt.map_or(response.id, |s| ui.id().with(s));
        let visible = self.active.is_none_or(|on| loading_visible(&ctx, id, on));
        if visible && ui.is_rect_visible(rect) {
            let p = theme::palette(&ctx);
            let turn = self.force.and_then(|f| f.progress);
            paint_spinner(ui.painter(), rect.center(), side, stroke, p, turn);
        }
        let name = match &self.label {
            Some(l) => format!("Loading {l}"),
            None => "Loading".to_owned(),
        };
        response.widget_info(|| WidgetInfo::labeled(WidgetType::ProgressIndicator, true, &name));
        response
    }
}

impl Widget for Spinner {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// Paints a spinner of diameter `side` around `center`. `turn` fixes the rotation (0 to 1);
/// otherwise it turns with the clock, or pulses in place with reduced motion.
pub(crate) fn paint_spinner(
    painter: &Painter,
    center: Pos2,
    side: f32,
    width: f32,
    p: &Palette,
    turn: Option<f32>,
) {
    let ctx = painter.ctx();
    let reduced = motion::reduced();
    let (angle, alpha) = match turn {
        Some(t) => (t * std::f32::consts::TAU, 1.0),
        None if reduced => {
            // Still: the arc stays put and its strength pulses 0.5 to 1 once a second.
            keep_moving(ctx);
            let k = loop_phase(ctx, motion::PULSE_PERIOD);
            (0.0, 0.75 + 0.25 * (k * std::f32::consts::TAU).cos())
        }
        None => {
            keep_moving(ctx);
            (
                loop_phase(ctx, motion::SPIN_PERIOD) * std::f32::consts::TAU,
                1.0,
            )
        }
    };
    let r = (side - width) / 2.0;
    painter.circle_stroke(center, r, Stroke::new(width, Palette::tint(p.focus, 66)));
    let color = p.focus.gamma_multiply(alpha);
    let start = angle - std::f32::consts::FRAC_PI_2;
    let points: Vec<Pos2> = (0..=27)
        .map(|i| {
            let a = start + crate::int_f32(i) / 27.0 * std::f32::consts::TAU * 0.75;
            center + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    // Round caps.
    for end in [points[0], points[points.len() - 1]] {
        painter.circle_filled(end, width / 2.0, color);
    }
    painter.add(Shape::line(points, Stroke::new(width, color)));
}

// ------------------------------------------------------------------------------------ Skeleton

/// The translucent `c` (premultiplied) laid `n` times over itself, as one colour: its colour
/// and alpha both scale by `(1 - (1 - a)^n) / a`.
fn layered(c: Color32, n: i32) -> Color32 {
    let a = f32::from(c.a()) / 255.0;
    if a <= 0.0 {
        return c;
    }
    c.gamma_multiply((1.0 - (1.0 - a).powi(n)) / a)
}

/// Placeholder rows while a list first loads (never on refreshes). `Skeleton::rows(6).show(ui)`.
/// The rows appear after `LOADING_DELAY` of being shown (at once with reduced motion), so a quick
/// load never flashes them.
#[derive(Debug, Clone, Copy)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Skeleton {
    rows: usize,
    row_height: Option<f32>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl Skeleton {
    pub fn rows(rows: usize) -> Self {
        Self {
            rows,
            row_height: None,
            id_salt: None,
            force: None,
        }
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    /// The real row height of the list it stands in for (default `row_height`).
    pub fn row_height(mut self, height: f32) -> Self {
        self.row_height = Some(height);
        self
    }
    /// Shows the pulse at `force.progress` (the gallery and tests only).
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    /// How far the pulse is from rest (0) to strong (1) at loop phase `t`; reduced motion
    /// freezes it at the midpoint.
    pub fn pulse(t: Option<f32>) -> f32 {
        match t {
            Some(t) => 0.5 - 0.5 * (t * std::f32::consts::TAU).cos(),
            None => 0.5,
        }
    }

    /// The bar fill at loop phase `t`: the `pressed` overlay laid twice over the ground at rest
    /// and three times at the top of the pulse (`None`, reduced motion: half way). Translucent,
    /// so a bar reads alike on pane and chrome, dark or light (about 1.3 to 1.6:1); one layer
    /// of the hover overlays, as before, faded into dark grounds.
    pub fn fill_at(p: &Palette, t: Option<f32>) -> Color32 {
        let (rest, strong) = SKELETON_LAYERS;
        layered(p.pressed(), rest).lerp_to_gamma(layered(p.pressed(), strong), Self::pulse(t))
    }

    /// Whether the rows are due: shown for `LOADING_DELAY` without a break (a skeleton not
    /// painted last pass starts its wait again).
    fn due(ctx: &egui::Context, id: Id) -> bool {
        #[derive(Clone, Copy)]
        struct Wait {
            since: f64,
            pass: u64,
        }
        let now = ctx.input(|i| i.time);
        let pass = ctx.cumulative_pass_nr();
        let delay = motion::time(motion::LOADING_DELAY);
        let since = ctx.data_mut(|d| {
            let w = d.get_temp_mut_or_insert_with(id.with("kit-skeleton-wait"), || Wait {
                since: now,
                pass,
            });
            if w.pass + 1 < pass {
                w.since = now;
            }
            w.pass = pass;
            w.since
        });
        let waited = crate::to_f32(now - since);
        if waited < delay {
            ctx.request_repaint_after(Duration::from_secs_f32(delay - waited));
            false
        } else {
            true
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let m = theme::metrics();
        let row_h = self.row_height.unwrap_or(m.row_height);
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(
            vec2(width, row_h * crate::count_f32(self.rows)),
            Sense::hover(),
        );
        let id = self.id_salt.map_or(response.id, |s| ui.id().with(s));
        let due = self.force.is_some() || Self::due(&ctx, id);
        if due && ui.is_rect_visible(rect) {
            let t = match self.force.and_then(|f| f.progress) {
                Some(t) => Some(t),
                None if motion::reduced() => None,
                None => {
                    keep_moving(&ctx);
                    Some(loop_phase(&ctx, motion::SKELETON_PERIOD))
                }
            };
            let color = Self::fill_at(p, t);
            let bar_h = (row_h * 0.42).round().max(6.0);
            for i in 0..self.rows {
                let share = SKELETON_WIDTHS[i % SKELETON_WIDTHS.len()];
                let top = rect.top() + row_h * crate::count_f32(i) + (row_h - bar_h) / 2.0;
                let bar = Rect::from_min_size(
                    pos2(rect.left() + theme::MD, top),
                    vec2((width - 2.0 * theme::MD).max(0.0) * share, bar_h),
                );
                ui.painter()
                    .rect_filled(bar, CornerRadius::same(m.radius_sm), color);
            }
        }
        response
            .widget_info(|| WidgetInfo::labeled(WidgetType::ProgressIndicator, true, "Loading"));
        response
    }
}

impl Widget for Skeleton {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's Progress section: determinate stops, indeterminate and line bars at rest and
/// mid-sweep, the spinner sizes, a labelled wait and skeleton rows.
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
    let width = ui.available_width().min(360.0);
    ui.spacing_mut().item_spacing.y = theme::MD;
    Progress::new(0.62)
        .label("Fetching origin")
        .value_text("62%")
        .width(width)
        .id_salt("g-fetch")
        .show_forced(ui, ForceState::default());
    Progress::indeterminate()
        .label("Computing graph layout")
        .value_text("12,480 commits")
        .width(width)
        .id_salt("g-graph")
        .show_forced(
            ui,
            ForceState {
                progress: Some(0.35),
                ..ForceState::default()
            },
        );
    small(ui, "Determinate: 0%, 2%, 40%, 100%");
    for (i, f) in [0.0, 0.02, 0.4, 1.0].into_iter().enumerate() {
        Progress::new(f)
            .width(width)
            .id_salt(("g-stop", i))
            .show_forced(ui, ForceState::default());
    }
    small(
        ui,
        "Indeterminate at 0, 0.5 and 1 of a sweep; reduced motion; line",
    );
    for (i, t) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        Progress::indeterminate()
            .width(width)
            .id_salt(("g-indet", i))
            .show_forced(
                ui,
                ForceState {
                    progress: Some(t),
                    ..ForceState::default()
                },
            );
    }
    Progress::indeterminate()
        .label("Reduced motion")
        .value_text(WORKING)
        .width(width)
        .id_salt("g-reduced")
        .show_forced(
            ui,
            ForceState {
                progress: Some(0.25),
                ..ForceState::default()
            },
        );
    Progress::line().width(width).id_salt("g-line").show_forced(
        ui,
        ForceState {
            progress: Some(0.5),
            ..ForceState::default()
        },
    );
    small(ui, "Spinners: Xs, Sm, Md, Lg");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::MD;
        for (i, size) in Size::ALL.into_iter().enumerate() {
            Spinner::new()
                .size(size)
                .id_salt(("g-spin", i))
                .show_forced(
                    ui,
                    ForceState {
                        progress: Some(0.1 * crate::count_f32(i)),
                        ..ForceState::default()
                    },
                );
        }
    });
    ui.horizontal(|ui| {
        Spinner::new()
            .label("pull requests")
            .active(true)
            .id_salt("g-spin-label")
            .show_forced(ui, ForceState::default());
        ui.label(
            egui::RichText::new("Loading pull requests")
                .size(m.meta_size)
                .color(p.text_dim),
        );
    });
    small(ui, "Skeleton rows (rest, midpoint, strong)");
    for (i, t) in [0.0, 0.25, 0.5].into_iter().enumerate() {
        Skeleton::rows(2)
            .row_height(m.history_row_height)
            .id_salt(("g-skeleton", i))
            .show_forced(
                ui,
                ForceState {
                    progress: Some(t),
                    ..ForceState::default()
                },
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn frame(ctx: &egui::Context, time: f64, mut f: impl FnMut(&mut Ui)) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out
    }

    fn fills(out: &egui::FullOutput) -> Vec<Rect> {
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Rect(r)
                    if r.fill == theme::ThemeId::Midnight.palette().standard.primary.top =>
                {
                    Some(r.rect)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn determinate_fill_follows_the_fraction_with_a_visible_minimum() {
        let ctx = context();
        let mut width = 0.0;
        for (f, expect) in [(0.4, 0.4), (0.001, MIN_STARTED), (1.0, 1.0)] {
            let out = frame(&ctx, 0.0, |ui| {
                width = Progress::new(f)
                    .width(200.0)
                    .id_salt(f.to_bits())
                    .show(ui)
                    .rect
                    .width();
            });
            let got = fills(&out);
            assert_eq!(got.len(), 1, "{f}: {got:?}");
            assert!(
                (got[0].width() - expect * width).abs() < 0.01,
                "{f}: {got:?}"
            );
        }
        let out = frame(&ctx, 0.0, |ui| {
            Progress::new(0.0).width(200.0).show(ui);
        });
        assert!(fills(&out).is_empty(), "nothing started, nothing filled");
    }

    #[test]
    fn the_fill_never_runs_backwards() {
        let ctx = egui::Context::default();
        theme::apply(
            &ctx,
            theme::ThemeId::Midnight,
            theme::Appearance {
                reduce_motion: false,
                ..theme::Appearance::STILL
            },
        );
        let id = Id::new("bar");
        let mut out = Vec::new();
        for (time, target) in [(0.0, 0.2), (0.05, 0.8), (0.1, 0.8), (0.5, 0.8), (0.55, 0.1)] {
            let _ = frame(&ctx, time, |ui| {
                out.push(forward(ui.ctx(), id, target));
            });
        }
        assert_eq!(out[0], 0.2);
        assert_eq!(out[1], 0.2, "a rise starts where the fill is: {out:?}");
        assert!(out[2] > 0.2 && out[2] < 0.8, "eases up: {out:?}");
        assert_eq!(out[3], 0.8, "lands after PROGRESS_FILL: {out:?}");
        assert_eq!(out[4], 0.1, "a smaller value snaps: {out:?}");
        motion::set_reduced(true);
    }

    #[test]
    fn reduced_motion_stops_every_loop() {
        let ctx = context();
        let shapes_at = |time: f64| {
            let out = frame(&ctx, time, |ui| {
                Progress::indeterminate().width(200.0).show(ui);
                Spinner::new().show(ui);
                Skeleton::rows(2).show(ui);
            });
            let fill = fills(&out);
            let wake = out
                .viewport_output
                .values()
                .map(|v| v.repaint_delay)
                .min()
                .unwrap_or(Duration::MAX);
            (fill, wake)
        };
        let (a, _) = shapes_at(0.0);
        let (b, _) = shapes_at(0.7);
        assert_eq!(a, b, "the indeterminate fill stands still");
        assert!(!a.is_empty(), "the indeterminate bar painted nothing");
        // The indeterminate bar says it is working when it cannot move.
        let out = frame(&ctx, 0.0, |ui| {
            Progress::indeterminate().width(200.0).show(ui);
        });
        assert!(out.shapes.iter().any(|c| matches!(&c.shape,
            Shape::Text(t) if t.galley.text() == WORKING)));
        assert_eq!(Skeleton::pulse(None), 0.5);
    }

    #[test]
    fn the_progress_label_is_dim_and_its_value_strong() {
        let ctx = context();
        let p = theme::palette(&ctx);
        let out = frame(&ctx, 0.0, |ui| {
            Progress::new(0.5)
                .label("Receiving objects")
                .value_text("12 of 340")
                .show(ui);
        });
        let color = |text: &str| {
            out.shapes.iter().find_map(|c| match &c.shape {
                Shape::Text(t) if t.galley.text() == text => Some(t.fallback_color),
                _ => None,
            })
        };
        assert_eq!(color("Receiving objects"), Some(p.text_dim));
        assert_eq!(color("12 of 340"), Some(p.text));
    }

    #[test]
    fn skeleton_pulses_between_two_and_three_pressed_layers() {
        let p = theme::ThemeId::Daylight.palette();
        assert_eq!(Skeleton::fill_at(p, Some(0.0)), layered(p.pressed(), 2));
        assert_eq!(Skeleton::fill_at(p, Some(0.5)), layered(p.pressed(), 3));
        assert_eq!(layered(p.pressed(), 1), p.pressed());
        let g = p.standard.pane;
        let twice = theme::over(p.pressed(), theme::over(p.pressed(), g));
        let once = theme::over(layered(p.pressed(), 2), g);
        for (a, b) in [
            (twice.r(), once.r()),
            (twice.g(), once.g()),
            (twice.b(), once.b()),
        ] {
            assert!(a.abs_diff(b) <= 1, "{twice:?} vs {once:?}");
        }
    }

    #[test]
    fn skeleton_rows_can_be_seen_on_every_ground() {
        for id in theme::ThemeId::ALL {
            let p = id.palette();
            for style in theme::Style::ALL {
                let s = p.surfaces(style);
                theme::with_look(style, theme::Density::Comfortable, || {
                    // Reduced motion holds the midpoint: it must read as well as the pulse.
                    for t in [Some(0.0), None] {
                        let fill = Skeleton::fill_at(p, t);
                        for g in [s.pane, s.panel] {
                            let k = theme::contrast(theme::over(fill, g), g);
                            assert!(k >= 1.25, "{id:?} {style:?} {t:?}: {k:.2}:1");
                        }
                    }
                });
            }
        }
    }

    #[test]
    fn skeleton_rows_wait_for_the_loading_delay() {
        let ctx = egui::Context::default();
        theme::apply(
            &ctx,
            theme::ThemeId::Midnight,
            theme::Appearance {
                reduce_motion: false,
                ..theme::Appearance::STILL
            },
        );
        let bars = |time: f64| {
            let out = frame(&ctx, time, |ui| {
                Skeleton::rows(3).id_salt("wait").show(ui);
            });
            out.shapes
                .iter()
                .filter(|c| matches!(c.shape, Shape::Rect(_)))
                .count()
        };
        assert_eq!(bars(0.0), 0, "nothing before LOADING_DELAY");
        assert_eq!(bars(0.1), 0);
        assert_eq!(bars(0.2), 3, "the rows after it");
        motion::set_reduced(true);
    }

    #[test]
    fn indeterminate_fill_sweeps_from_before_to_after_the_track() {
        let ctx = context();
        let span = |t: f32| {
            let out = frame(&ctx, 0.0, |ui| {
                Progress::indeterminate().width(300.0).show_forced(
                    ui,
                    ForceState {
                        progress: Some(t),
                        ..ForceState::default()
                    },
                );
            });
            fills(&out)
        };
        assert!(span(0.0).is_empty(), "starts one width before the track");
        assert!(span(1.0).is_empty(), "ends past the track");
        let mid = span(0.5);
        assert_eq!(mid.len(), 1);
        assert!((mid[0].width() - 0.34 * 300.0).abs() < 0.5, "{mid:?}");
    }

    #[test]
    fn loading_shows_after_the_delay_and_stays_the_minimum() {
        let ctx = egui::Context::default();
        motion::set_reduced(false);
        let id = Id::new("load");
        let at = |t: f64, loading: bool| {
            let mut v = false;
            let _ = frame(&ctx, t, |ui| v = loading_visible(ui.ctx(), id, loading));
            v
        };
        assert!(!at(0.0, true), "not before the delay");
        assert!(!at(0.1, true));
        assert!(at(0.2, true), "after LOADING_DELAY");
        assert!(at(0.3, false), "kept for LOADING_MIN");
        assert!(!at(0.7, false), "then gone");
        assert!(!at(0.8, true), "a new wait starts over");
        motion::set_reduced(true);
    }

    /// A view that draws its spinner only while it waits (the diff, the commit details) never
    /// passes `loading: false`; its next wait must still start over, or every later load would
    /// flash the spinner at once (O2).
    #[test]
    fn an_indicator_drawn_only_while_loading_waits_again() {
        let ctx = egui::Context::default();
        motion::set_reduced(false);
        let id = Id::new("load");
        let at = |t: f64| {
            let mut v = false;
            let _ = frame(&ctx, t, |ui| v = loading_visible(ui.ctx(), id, true));
            v
        };
        assert!(!at(0.0), "not before the delay");
        assert!(at(0.2), "after LOADING_DELAY");
        // Loaded: the view stops drawing the indicator.
        let _ = frame(&ctx, 0.3, |_| {});
        let _ = frame(&ctx, 0.4, |_| {});
        assert!(!at(5.0), "the next load waits again");
        assert!(!at(5.1));
        assert!(at(5.2), "and shows after LOADING_DELAY");
        motion::set_reduced(true);
    }

    #[test]
    fn indicators_report_progress_roles() {
        let ctx = context();
        ctx.enable_accesskit();
        let out = frame(&ctx, 0.0, |ui| {
            Progress::new(0.5).label("Fetching origin").show(ui);
            Spinner::new().label("pull requests").show(ui);
        });
        let nodes = out
            .platform_output
            .accesskit_update
            .expect("accesskit")
            .nodes;
        let found = |label: &str| {
            nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .map(|(_, n)| (n.role(), n.numeric_value()))
        };
        assert_eq!(
            found("Fetching origin"),
            Some((egui::accesskit::Role::ProgressIndicator, Some(50.0)))
        );
        assert_eq!(
            found("Loading pull requests").map(|f| f.0),
            Some(egui::accesskit::Role::ProgressIndicator)
        );
    }
}
