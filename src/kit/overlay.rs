//! Floating layers: the surface every one of them shares ([`floating`]: soft shadow, the glass
//! hook, the overlay fill, Deeper's top highlight and the 1 pt ring), [`Tooltip`], [`Popover`],
//! [`Dialog`] and its [`scrim`], and the motion they share: entrances that fade and grow from the
//! anchor side, and exits that fade out what the layer showed last. Design spec 4.11, 4.13, 4.14.
//!
//! **Glass.** [`backdrop`] is the one place the backdrop blur plugs in: every floating layer except
//! tooltips asks it for the blurred backdrop, copies it before its shadow and draws it between
//! the shadow and its fill, and paints the translucent overlay over it when it gets one. With
//! Translucent surfaces off, or the blur unavailable, it returns `None` and the layer is opaque.
//!
//! **Exits.** A floating layer records what it painted each frame it is open ([`record_exit`]);
//! the frame it is no longer shown, an egui plugin keeps painting that snapshot as it fades (and
//! shrinks or slides) for the exit's duration. Nothing is recorded with reduced motion. A snapshot
//! drops the glass backdrop: the blur callback cannot fade or move with it, so a closing layer
//! fades out as its translucent fill alone.

use std::sync::Arc;

use egui::emath::TSTransform;
use egui::epaint::{ClippedShape, RectShape};
use egui::layers::ShapeIdx;
use egui::{
    Align, Align2, Color32, Context, CornerRadius, FontId, Frame, Id, InnerResponse, Key, LayerId,
    Layout, Margin, Order, Painter, Popup, PopupCloseBehavior, PopupKind, Pos2, Rangef, Rect,
    RectAlign, Response, RichText, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2,
    vec2,
};
use egui_phosphor::regular as icon;

use super::Shortcut;
use super::{Button, ForceState, IconButton, Size, paint};
use crate::motion;
use crate::theme::{self, Elevation, Fill};

/// Room kept between a dialog card and the window edges (both sides together).
const DIALOG_MARGIN: f32 = 28.0;
/// The dialog's glyph disc.
const DISC: f32 = 30.0;
/// Gap between a tooltip or popover and its anchor.
const TOOLTIP_GAP: f32 = 8.0;
pub(crate) const POPOVER_GAP: f32 = 6.0;
/// Inherited bounds for controls whose popup must stay inside a containing surface.
pub const POPUP_BOUNDS_TAG: &str = "canopy-popup-bounds";
/// Widest tooltip.
const TOOLTIP_WIDTH: f32 = 260.0;
/// A tooltip's shortcut keycaps (spec 4.11: `KeyChord` at 18 pt).
const TOOLTIP_KEYCAP: f32 = 18.0;
/// Pointer-opened popups grow from this scale and drop this far from the anchor edge.
pub(crate) const POPUP_SCALE: f32 = 0.95;
const POPUP_DROP: f32 = 4.0;
/// Dialogs (and the settings screen, a dialog-sized shell) grow from this scale and exit towards
/// the second.
pub const DIALOG_SCALE: f32 = 0.94;
pub const DIALOG_EXIT_SCALE: f32 = 0.97;
/// Tooltips grow from this scale and rise this far.
const TOOLTIP_SCALE: f32 = 0.96;
const TOOLTIP_RISE: f32 = 3.0;
/// Room after the last line of a scrolling body, so its end never sits on the card's edge.
pub const SCROLL_END_ROOM: f32 = theme::SM;
/// A scrolling body's bar stands this far in from the edge of the card (spec 4.21's 2 pt margin).
const BAR_INSET: f32 = 2.0;
/// Thickness of a dialog's reading-progress line ([`Dialog::progress_lane`]).
const PROGRESS: f32 = 2.0;

/// What a floating surface is: decides its radius, fill and whether it may be glass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Layer {
    Menu,
    Popover,
    Dialog,
    Toast,
    /// Never glass; the `tooltip` fill (dark in light themes) and a smaller radius.
    Tooltip,
}

impl Layer {
    pub fn radius(self) -> CornerRadius {
        let m = theme::metrics();
        match self {
            Self::Tooltip => CornerRadius::same(m.radius_sm.saturating_add(2)),
            _ => CornerRadius::same(m.radius_lg),
        }
    }

    fn may_be_glass(self) -> bool {
        self != Self::Tooltip
    }
}

// --------------------------------------------------------------------------- surface and glass

/// The glass hook: what the backdrop blur paints under a floating layer at `rect` (design spec
/// 7.4: `Shape::Callback`s into `glass/gl.rs`), its capture to go before the layer's shadow and
/// its composite after it, before the fill, so the layer never blurs its own shadow; or `None`
/// for an opaque layer (Translucent surfaces off, the blur unavailable, or the frame's glass
/// layers used up).
pub(crate) fn backdrop(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
) -> Option<super::glass::Backdrop> {
    super::glass::backdrop_shapes(painter, rect, radius)
}

/// What a floating layer of `layer` is filled with this frame: the overlay at the theme's glass
/// opacity when its backdrop is blurred (Translucent surfaces on and the blur running), else the
/// opaque overlay (tooltips: the `tooltip` fill). Content that paints over the layer's ground (row
/// washes, scroll fades) uses this, so it matches the surface in both paths. A layer past the
/// frame's glass budget falls back to opaque; its content then fades into a slightly lighter tint
/// of the same overlay, which reads the same.
pub fn ground(ctx: &Context, layer: Layer) -> Color32 {
    let p = theme::palette(ctx);
    let s = theme::surfaces(ctx);
    if layer == Layer::Tooltip {
        s.tooltip.top
    } else if theme::appearance().glass && super::glass::available() && p.glass_alpha > 0 {
        p.glass(s)
    } else {
        s.overlay
    }
}

/// A vertical scrolling body for an overlay, a dialog or a settings page: at most `max_height`
/// tall, its scrollbar inset from the card's edge, and edge fades into `ground` (see [`ground`])
/// wherever more is scrolled out of sight, so a body never ends on a line cut through its glyphs
/// without a cue. When the content overflows, `SCROLL_END_ROOM` follows it, so its last line
/// scrolls clear of the card's edge; a body that fits keeps its own height.
pub fn scroll_body<R>(
    ui: &mut Ui,
    id_salt: impl std::hash::Hash + std::fmt::Debug,
    max_height: f32,
    ground: Color32,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::scroll_area::ScrollAreaOutput<R> {
    let id = ui.make_persistent_id(id_salt);
    let overflow_key = id.with("kit-scroll-body-overflows");
    let overflowed = ui.data(|d| d.get_temp::<bool>(overflow_key).unwrap_or(false));
    let out = ui
        .scope(|ui| {
            ui.spacing_mut().scroll.bar_outer_margin = BAR_INSET;
            egui::ScrollArea::vertical()
                .id_salt(id)
                .max_height(max_height)
                // A scrolling body takes all of `max_height`, whatever size its area had last
                // frame.
                .min_scrolled_height(max_height)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    let r = add(ui);
                    if overflowed {
                        ui.add_space(SCROLL_END_ROOM);
                    }
                    r
                })
        })
        .inner;
    let overflows = out.content_size.y > out.inner_rect.height() + 0.5;
    if overflows != overflowed {
        ui.data_mut(|d| d.insert_temp(overflow_key, overflows));
        ui.ctx().request_repaint();
    }
    paint::scroll_fade(ui, &out, ground);
    out
}

/// Everything a floating surface of `layer` paints at `rect`, bottom first: the `Floating`
/// shadow, the glass backdrop (if any), the fill (translucent over glass), the top highlight
/// (Deeper dark) and the 1 pt ring.
pub(crate) fn surface_shapes(painter: &Painter, rect: Rect, layer: Layer) -> Vec<Shape> {
    let ctx = painter.ctx();
    let p = theme::palette(ctx);
    let style = theme::style();
    let s = p.surfaces(style);
    let radius = layer.radius();
    let material = p.material(style, Elevation::Floating);
    let mut shapes = paint::shadow_shapes(rect, radius, p, &material);
    let fill = if layer == Layer::Tooltip {
        s.tooltip
    } else {
        let glass = layer
            .may_be_glass()
            .then(|| backdrop(painter, rect, radius))
            .flatten();
        match glass {
            Some(b) => {
                // The backdrop is copied before the layer's own shadow is painted, so the glass
                // blurs what is behind the layer, not its shadow (G3), and drawn over the shadow.
                shapes.insert(0, b.capture);
                shapes.push(b.composite);
                Fill::flat(p.glass(s))
            }
            None => Fill::flat(s.overlay),
        }
    };
    shapes.push(paint::fill_shape(ctx, rect, radius, fill));
    if material.highlight > 0 {
        let inset = f32::from(radius.nw.max(radius.ne)).max(1.0);
        let w = 1.0 / ctx.pixels_per_point();
        shapes.push(Shape::hline(
            Rangef::new(rect.left() + inset, rect.right() - inset),
            rect.top() + w * 0.5,
            Stroke::new(w, theme::highlight_line(material.highlight)),
        ));
    }
    shapes.push(
        RectShape::stroke(
            rect,
            radius,
            Stroke::new(paint::SEAM, p.floating_ring(s)),
            StrokeKind::Inside,
        )
        .into(),
    );
    shapes
}

/// Lays `add` out inside a floating surface of `layer` with `margin` around it, then paints the
/// surface underneath (in a slot reserved before the content, so it sits below it). Works inside
/// a popup's area or inline (the gallery).
pub fn floating<R>(
    ui: &mut Ui,
    layer: Layer,
    margin: Margin,
    add: impl FnOnce(&mut Ui) -> R,
) -> InnerResponse<R> {
    let slot = ui.painter().add(Shape::Noop);
    let bounds_id = ui.id().with(("popup-host-bounds", layer));
    let measured = ui.ctx().data(|d| d.get_temp::<Rect>(bounds_id));
    let r = Frame::new().inner_margin(margin).show(ui, |ui| {
        if layer == Layer::Dialog {
            let bounds =
                measured.unwrap_or_else(|| ui.max_rect().intersect(ui.ctx().content_rect()));
            ui.scope_builder(
                UiBuilder::new().ui_stack_info(
                    egui::UiStackInfo::default().with_tag_value(POPUP_BOUNDS_TAG, bounds),
                ),
                add,
            )
            .inner
        } else {
            add(ui)
        }
    });
    if layer == Layer::Dialog && measured != Some(r.response.rect) {
        ui.ctx()
            .data_mut(|d| d.insert_temp(bounds_id, r.response.rect));
        ui.ctx().request_repaint();
    }
    let shapes = surface_shapes(ui.painter(), r.response.rect, layer);
    ui.painter().set(slot, Shape::Vec(shapes));
    InnerResponse::new(r.inner, r.response)
}

/// The enclosing dialog or main panel, rather than the field's narrow layout row.
pub(crate) fn popup_bounds(ui: &Ui) -> Rect {
    let screen = ui.ctx().content_rect();
    if let Some(bounds) = ui
        .stack()
        .iter()
        .find_map(|node| node.tags().get_downcast::<Rect>(POPUP_BOUNDS_TAG).copied())
    {
        return bounds.intersect(screen);
    }
    ui.stack()
        .iter()
        .find(|node| {
            matches!(
                node.kind(),
                Some(
                    egui::UiKind::CentralPanel
                        | egui::UiKind::LeftPanel
                        | egui::UiKind::RightPanel
                        | egui::UiKind::Window
                )
            )
        })
        .map_or(screen, |node| node.max_rect.intersect(screen))
}

/// The room a modal card (a dialog, the settings screen) may fill: the window inside `XL` at the
/// sides, below the title bar's tab strip and above the status bar, `SM` clear of each, so the
/// card never covers the tab strip or the status line however small the window.
pub fn modal_room(ctx: &Context) -> Rect {
    let m = theme::metrics();
    let screen = ctx.content_rect();
    let top = (screen.top() + m.tab_strip_height + theme::SM).min(screen.center().y);
    let bottom = (screen.bottom() - m.status_bar_height - theme::SM).max(top);
    Rect::from_min_max(
        pos2(screen.left() + theme::XL, top),
        pos2(
            (screen.right() - theme::XL).max(screen.left() + theme::XL),
            bottom,
        ),
    )
}

/// The scrim behind dialogs, at `alpha` of its strength (its fade).
pub fn scrim(painter: &Painter, rect: Rect, alpha: f32) {
    let p = theme::palette(painter.ctx());
    painter.rect_filled(rect, CornerRadius::ZERO, p.scrim.gamma_multiply(alpha));
}

// --------------------------------------------------------------------------------- motion

/// How long the overlay `id` has been shown in an unbroken run of frames, and whether this is
/// the first frame of the run (it was not shown the frame before).
pub(crate) fn presence(ctx: &Context, id: Id) -> (f32, bool) {
    #[derive(Clone, Copy)]
    struct Run {
        since: f64,
        last: u64,
    }
    let now = ctx.input(|i| i.time);
    let pass = ctx.cumulative_pass_nr();
    let (since, fresh) = ctx.data_mut(|d| {
        let key = id.with("kit-shown-since");
        let fresh = d.get_temp::<Run>(key).is_none_or(|r| r.last + 1 < pass);
        let run = d.get_temp_mut_or_insert_with(key, || Run {
            since: now,
            last: pass,
        });
        if fresh {
            run.since = now;
        }
        run.last = pass;
        (run.since, fresh)
    });
    (crate::to_f32(now - since), fresh)
}

/// How far an entrance has come: `fade` (opacity, 0 to 1) and `grow` (the transform's progress
/// along the spring curve, which overshoots 1 a little).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entrance {
    pub fade: f32,
    pub grow: f32,
}

impl Entrance {
    pub const DONE: Self = Self {
        fade: 1.0,
        grow: 1.0,
    };

    /// The entrance of `id`, `fade_s` for the opacity (`EASE_OUT`) and `grow_s` for the transform
    /// (`SPRING_CURVE`), counted from its first frame. Done at once with reduced motion or when
    /// `instant` (keyboard-opened menus); `progress` fixes it (the gallery).
    pub fn of(
        ctx: &Context,
        id: Id,
        fade_s: f32,
        grow_s: f32,
        instant: bool,
        progress: Option<f32>,
    ) -> Self {
        if let Some(t) = progress {
            return Self::at(t, t);
        }
        let (elapsed, _) = presence(ctx, id);
        let (fade_s, grow_s) = (motion::time(fade_s), motion::time(grow_s));
        if instant || (fade_s <= 0.0 && grow_s <= 0.0) {
            return Self::DONE;
        }
        let k = |s: f32| {
            if s <= 0.0 {
                1.0
            } else {
                (elapsed / s).clamp(0.0, 1.0)
            }
        };
        let (a, b) = (k(fade_s), k(grow_s));
        if a < 1.0 || b < 1.0 {
            ctx.request_repaint();
        }
        Self::at(a, b)
    }

    /// The entrance at linear times `fade_t` and `grow_t` (0 to 1).
    pub fn at(fade_t: f32, grow_t: f32) -> Self {
        Self {
            fade: motion::EASE_OUT.at(fade_t),
            grow: motion::SPRING_CURVE.at(grow_t),
        }
    }

    /// The transform at this point: scaled from `from` about `origin` and offset by `offset`
    /// (both reached at `grow == 1`).
    pub fn transform(self, origin: Pos2, from: f32, offset: Vec2) -> TSTransform {
        let scale = from + (1.0 - from) * self.grow;
        let o = origin.to_vec2();
        TSTransform::from_translation(offset * (1.0 - self.grow) + o)
            * TSTransform::from_scaling(scale)
            * TSTransform::from_translation(-o)
    }
}

/// Where the next shape on `layer` will go.
pub fn next_shape(ctx: &Context, layer: LayerId) -> ShapeIdx {
    ctx.graphics(|g| {
        g.get(layer)
            .map_or(ShapeIdx(0), egui::layers::PaintList::next_idx)
    })
}

/// Transforms every shape painted on `layer` since `start`.
pub fn transform_since(ctx: &Context, layer: LayerId, start: ShapeIdx, tf: TSTransform) {
    if tf == TSTransform::IDENTITY {
        return;
    }
    ctx.graphics_mut(|g| {
        let list = g.entry(layer);
        let end = list.next_idx();
        list.transform_range(start, end, tf);
    });
}

/// Transforms the shapes painted on `layer` from `start` up to (not including) `end`: one card
/// of several laid out in a row or a stack.
pub fn transform_range(
    ctx: &Context,
    layer: LayerId,
    start: ShapeIdx,
    end: ShapeIdx,
    tf: TSTransform,
) {
    if tf == TSTransform::IDENTITY {
        return;
    }
    ctx.graphics_mut(|g| g.entry(layer).transform_range(start, end, tf));
}

/// How a closed layer leaves: fade over `seconds` while its moving part scales towards `scale`
/// about `origin` and slides by `offset`; its fixed part (a scrim) only fades.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExitMotion {
    pub seconds: f32,
    pub scale: f32,
    pub origin: Pos2,
    pub offset: Vec2,
    /// Vanish at once if the frame it closed in had a press, scroll or key (tooltips).
    pub instant_on_input: bool,
}

impl ExitMotion {
    /// Menus and popovers: fade with a small shrink towards the anchor side.
    pub fn popup(origin: Pos2) -> Self {
        Self {
            seconds: motion::EXIT_POPUP,
            scale: POPUP_SCALE,
            origin,
            offset: Vec2::ZERO,
            instant_on_input: false,
        }
    }
}

#[derive(Clone)]
struct Snapshot {
    key: Id,
    order: Order,
    fixed: Arc<[ClippedShape]>,
    moving: Arc<[ClippedShape]>,
    motion: ExitMotion,
    pass: u64,
}

#[derive(Clone)]
struct Exit {
    snap: Snapshot,
    started: f64,
}

#[derive(Clone, Default)]
struct Exits {
    open: Vec<Snapshot>,
    leaving: Vec<Exit>,
}

fn exits_id() -> Id {
    Id::new("canopy-kit-exits")
}

/// Paints closing layers at the end of every pass.
struct ExitPlugin;

impl egui::Plugin for ExitPlugin {
    fn debug_name(&self) -> &'static str {
        "canopy-kit-exits"
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        paint_exits(ui.ctx());
    }
}

/// Records what `key` painted on `layer` this frame (`fixed` then `moving` index ranges), so it
/// can fade out when it stops being shown. Does nothing with reduced motion.
pub fn record_exit(
    ctx: &Context,
    key: Id,
    layer: LayerId,
    fixed: std::ops::Range<usize>,
    moving: std::ops::Range<usize>,
    motion: ExitMotion,
) {
    if motion::time(motion.seconds) <= 0.0 {
        return;
    }
    let installed = Id::new("canopy-kit-exit-plugin");
    if !ctx.data(|d| d.get_temp::<bool>(installed).unwrap_or(false)) {
        ctx.add_plugin(ExitPlugin);
        ctx.data_mut(|d| d.insert_temp(installed, true));
    }
    let take = |r: std::ops::Range<usize>| -> Vec<ClippedShape> {
        ctx.graphics(|g| {
            g.get(layer).map_or_else(Vec::new, |l| {
                l.all_entries()
                    .skip(r.start)
                    .take(r.end.saturating_sub(r.start))
                    .map(|c| ClippedShape {
                        clip_rect: c.clip_rect,
                        shape: without_callbacks(c.shape.clone()),
                    })
                    .collect()
            })
        })
    };
    let snap = Snapshot {
        key,
        order: layer.order,
        fixed: Arc::from(take(fixed)),
        moving: Arc::from(take(moving)),
        motion,
        pass: ctx.cumulative_pass_nr(),
    };
    ctx.data_mut(|d| {
        let e = d.get_temp_mut_or_default::<Exits>(exits_id());
        e.open.retain(|s| s.key != key);
        e.leaving.retain(|x| x.snap.key != key);
        e.open.push(snap);
    });
}

/// `shape` with every paint callback (the glass backdrop) taken out: a callback paints at the
/// place and opacity it was made with, so it could neither fade nor move with an exit.
fn without_callbacks(shape: Shape) -> Shape {
    match shape {
        Shape::Callback(_) => Shape::Noop,
        Shape::Vec(shapes) => Shape::Vec(shapes.into_iter().map(without_callbacks).collect()),
        other => other,
    }
}

/// Starts the exits of layers not shown this pass and paints the ones still leaving.
fn paint_exits(ctx: &Context) {
    let pass = ctx.cumulative_pass_nr();
    let now = ctx.input(|i| i.time);
    let interrupted = ctx.input(|i| {
        i.pointer.any_pressed()
            || i.smooth_scroll_delta != Vec2::ZERO
            || i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))
    });
    let leaving = ctx.data_mut(|d| {
        let e = d.get_temp_mut_or_default::<Exits>(exits_id());
        let (shown, closed): (Vec<_>, Vec<_>) = e.open.drain(..).partition(|s| s.pass == pass);
        e.open = shown;
        for snap in closed {
            if !(snap.motion.instant_on_input && interrupted) {
                e.leaving.push(Exit { snap, started: now });
            }
        }
        e.leaving
            .retain(|x| crate::to_f32(now - x.started) < x.snap.motion.seconds);
        e.leaving.clone()
    });
    for x in &leaving {
        let t = (crate::to_f32(now - x.started) / x.snap.motion.seconds).clamp(0.0, 1.0);
        let k = motion::EASE_OUT.at(t);
        let alpha = 1.0 - k;
        let m = x.snap.motion;
        let scale = 1.0 + (m.scale - 1.0) * k;
        let o = m.origin.to_vec2();
        let tf = TSTransform::from_translation(m.offset * k + o)
            * TSTransform::from_scaling(scale)
            * TSTransform::from_translation(-o);
        let mut painter = ctx.layer_painter(LayerId::new(x.snap.order, x.snap.key.with("exit")));
        painter.set_opacity(alpha);
        for (shapes, tf) in [(&x.snap.fixed, TSTransform::IDENTITY), (&x.snap.moving, tf)] {
            for c in shapes.iter() {
                let mut shape = c.shape.clone();
                shape.transform(tf);
                painter.with_clip_rect(tf.mul_rect(c.clip_rect)).add(shape);
            }
        }
    }
    if !leaving.is_empty() {
        ctx.request_repaint();
    }
}

// --------------------------------------------------------------------------------- tooltip

/// Which tooltips show (Settings → Appearance → Tooltips).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TooltipLevel {
    /// None, except why a control is unavailable.
    Off,
    /// What the screen does not show: an icon-only control's name, why a control is unavailable,
    /// cut-off text, shortcuts and details such as full dates, addresses and paths.
    #[default]
    Helpful,
    /// Also descriptions of labelled controls and how to use them.
    All,
}

impl TooltipLevel {
    pub const ALL: [Self; 3] = [Self::Off, Self::Helpful, Self::All];

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Helpful => "Helpful",
            Self::All => "All",
        }
    }
}

/// How long the pointer rests on a control before its tooltip shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum TooltipDelay {
    Quick,
    #[default]
    Normal,
    Slow,
}

impl TooltipDelay {
    pub const ALL: [Self; 3] = [Self::Quick, Self::Normal, Self::Slow];

    pub fn label(self) -> &'static str {
        match self {
            Self::Quick => "Quick",
            Self::Normal => "Normal",
            Self::Slow => "Slow",
        }
    }

    /// The rest before a tooltip shows, in seconds.
    pub fn seconds(self) -> f32 {
        match self {
            Self::Quick => 0.15,
            Self::Normal => motion::TOOLTIP_DELAY,
            Self::Slow => 1.0,
        }
    }
}

#[cfg(not(any(test, feature = "testkit")))]
static TOOLTIP_LEVEL: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(1);

// Tests run in parallel threads, each with its own setting.
#[cfg(any(test, feature = "testkit"))]
thread_local! {
    static TOOLTIP_LEVEL_LOCAL: std::sync::atomic::AtomicU8 =
        const { std::sync::atomic::AtomicU8::new(1) };
}

fn tooltip_level_flag<R>(f: impl FnOnce(&std::sync::atomic::AtomicU8) -> R) -> R {
    #[cfg(not(any(test, feature = "testkit")))]
    return f(&TOOLTIP_LEVEL);
    #[cfg(any(test, feature = "testkit"))]
    return TOOLTIP_LEVEL_LOCAL.with(f);
}

/// Which tooltips show from now on (kept current by the app every frame).
pub fn set_tooltip_level(level: TooltipLevel) {
    let n = match level {
        TooltipLevel::Off => 0,
        TooltipLevel::Helpful => 1,
        TooltipLevel::All => 2,
    };
    tooltip_level_flag(|l| l.store(n, std::sync::atomic::Ordering::Relaxed));
}

/// Which tooltips show.
pub fn tooltip_level() -> TooltipLevel {
    match tooltip_level_flag(|l| l.load(std::sync::atomic::Ordering::Relaxed)) {
        0 => TooltipLevel::Off,
        2 => TooltipLevel::All,
        _ => TooltipLevel::Helpful,
    }
}

/// Whether a tooltip shows at `level`: why a control is unavailable always does, an `extra` one
/// only at All, and none other at Off.
fn tooltip_shows(level: TooltipLevel, enabled: bool, extra: bool) -> bool {
    match level {
        _ if !enabled => true,
        TooltipLevel::Off => false,
        TooltipLevel::Helpful => !extra,
        TooltipLevel::All => true,
    }
}

/// Kit tooltips on any response: `response.kit_tooltip("Fetch all remotes").shortcut("f").show()`.
pub trait TooltipExt {
    fn kit_tooltip(self, text: impl Into<String>) -> Tooltip;
}

impl TooltipExt for Response {
    fn kit_tooltip(self, text: impl Into<String>) -> Tooltip {
        Tooltip {
            response: self,
            text: text.into(),
            detail: None,
            shortcut: None,
            at_pointer: false,
            extra: false,
            unavailable: false,
            always: false,
        }
    }
}

/// A tooltip (concept `.k-tip`): the `tooltip` surface, 12 pt medium text, an optional dim second
/// line and the shortcut as keycaps. Shown after `TOOLTIP_DELAY` (at once within `TOOLTIP_SKIP` of
/// another), above the control and flipped to stay in the window, for enabled and disabled
/// controls alike (a disabled one says why). [`Tooltip::at_pointer`] places it by the pointer on
/// a wide control instead.
#[derive(Debug)]
#[must_use = "call .show()"]
pub struct Tooltip {
    response: Response,
    text: String,
    detail: Option<String>,
    shortcut: Option<Shortcut>,
    at_pointer: bool,
    extra: bool,
    unavailable: bool,
    always: bool,
}

/// How far above and below the pointer a pointer-placed tooltip may still sit against the control
/// (a row is shorter than this, so its tooltip clears the whole row).
const POINTER_REACH: f32 = 24.0;

/// What a pointer-placed tooltip on `control` is placed against: the control's column under the
/// pointer, cut to `POINTER_REACH` above and below it (the control itself without a pointer).
fn pointer_anchor(control: Rect, pointer: Option<Pos2>) -> Rect {
    let Some(at) = pointer else {
        return control;
    };
    let reach = Rangef::new(at.y - POINTER_REACH, at.y + POINTER_REACH);
    let y = control.y_range().intersection(reach);
    let y = if y.span() >= 0.0 {
        y
    } else {
        Rangef::point(at.y)
    };
    let x = at.x.clamp(control.left(), control.right());
    Rect::from_x_y_ranges(Rangef::point(x), y)
}

/// Above first, then below, then the sides.
const TOOLTIP_ALIGNS: [RectAlign; 4] = [
    RectAlign::TOP,
    RectAlign::BOTTOM,
    RectAlign::RIGHT,
    RectAlign::LEFT,
];

impl Tooltip {
    /// A dim second line (what the action affects, or why it is unavailable).
    pub fn detail(mut self, text: impl Into<String>) -> Self {
        self.detail = Some(text.into());
        self
    }
    /// The command's keys as keycaps after the text.
    pub fn shortcut(mut self, command: impl Into<Shortcut>) -> Self {
        self.shortcut = Some(command.into());
        self
    }
    /// Placed by the pointer: above (or below) the control in the pointer's column, following it
    /// along the control, for wide rows where a tooltip centred on the whole control lands far
    /// from the pointer. On a control taller than a row it sits within `POINTER_REACH` of the
    /// pointer.
    pub fn at_pointer(mut self) -> Self {
        self.at_pointer = true;
        self
    }
    /// Says nothing the screen does not already show, or only how to use the control: shown only
    /// with Tooltips set to All ([`TooltipLevel`]).
    pub fn extra(mut self, extra: bool) -> Self {
        self.extra = extra;
        self
    }

    /// The text says why the control is unavailable, though its response is enabled (a kit
    /// control draws itself disabled without disabling its ui): shown at every level.
    pub fn unavailable(mut self, unavailable: bool) -> Self {
        self.unavailable = unavailable;
        self
    }

    /// The control only shows this tooltip (a help mark): shown at every level, and while the
    /// control has keyboard focus as well as on hover.
    pub fn always(mut self) -> Self {
        self.always = true;
        self
    }

    pub fn show(self) -> Response {
        let Self {
            response,
            text,
            detail,
            shortcut,
            at_pointer,
            extra,
            unavailable,
            always,
        } = self;
        if !always && !tooltip_shows(tooltip_level(), response.enabled() && !unavailable, extra) {
            return response;
        }
        let mut tip = if response.enabled() {
            egui::containers::Tooltip::for_enabled(&response)
        } else {
            egui::containers::Tooltip::for_disabled(&response)
        };
        if always {
            let open = response.has_focus()
                || egui::containers::Tooltip::should_show_tooltip(&response, true);
            tip.popup = tip.popup.open(open);
        }
        tip.popup = tip
            .popup
            .frame(Frame::NONE)
            .gap(TOOLTIP_GAP)
            .align(RectAlign::TOP)
            .align_alternatives(&TOOLTIP_ALIGNS)
            .width(TOOLTIP_WIDTH);
        if at_pointer {
            // In window coordinates, like egui's own anchor (the control's layer may be moving,
            // such as a dialog growing in).
            let control = response
                .ctx
                .layer_transform_to_global(response.layer_id)
                .map_or(response.interact_rect, |t| t * response.interact_rect);
            let pointer = response.ctx.pointer_hover_pos();
            tip.popup = tip.popup.anchor(pointer_anchor(control, pointer));
        }
        let anchor = tip.popup.get_anchor_rect().unwrap_or(response.rect);
        let id = response.id.with("kit-tooltip");
        tip.show(|ui| {
            let layer = ui.layer_id();
            let ctx = ui.ctx().clone();
            let e = Entrance::of(&ctx, id, motion::FAST_OUT, motion::BASE, false, None);
            ui.multiply_opacity(e.fade);
            let start = next_shape(&ctx, layer);
            let r = tooltip_body(ui, &text, detail.as_deref(), shortcut);
            let rect = r.rect;
            let below = rect.center().y > anchor.center().y;
            let origin = if below {
                rect.center_top()
            } else {
                rect.center_bottom()
            };
            let rise = if below { -TOOLTIP_RISE } else { TOOLTIP_RISE };
            transform_since(
                &ctx,
                layer,
                start,
                e.transform(origin, TOOLTIP_SCALE, vec2(0.0, rise)),
            );
            let end = next_shape(&ctx, layer);
            record_exit(
                &ctx,
                id,
                layer,
                0..0,
                start.0..end.0,
                ExitMotion {
                    seconds: motion::TOOLTIP_OUT,
                    scale: 1.0,
                    origin,
                    offset: Vec2::ZERO,
                    instant_on_input: true,
                },
            );
        });
        response
    }
}

/// The keycaps of `command`'s keys: the single key that works outside text fields when there is
/// one, else the chord, split into its keys.
pub(crate) fn chord_keys(command: impl Into<Shortcut>) -> Vec<String> {
    let command = command.into();
    let hint = command.hint();
    let chord = hint.rsplit(" · ").next().unwrap_or_default();
    split_chord(chord)
}

/// `Ctrl+Shift+F` as `["Ctrl", "Shift", "F"]`; a `+` key stays a key (`Ctrl++`).
pub(crate) fn split_chord(chord: &str) -> Vec<String> {
    let mut keys = Vec::new();
    let mut rest = chord;
    while !rest.is_empty() {
        // The first character is a key even when it is "+"; a "+" after it separates keys.
        if let Some((i, _)) = rest.char_indices().skip(1).find(|&(_, c)| c == '+') {
            let (key, tail) = rest.split_at(i);
            keys.push(key.to_owned());
            rest = tail.strip_prefix('+').unwrap_or(tail);
        } else {
            keys.push(rest.to_owned());
            break;
        }
    }
    keys
}

/// A tooltip's content and surface at the cursor of `ui` (inside its popup, or inline).
pub(crate) fn tooltip_body(
    ui: &mut Ui,
    text: &str,
    detail: Option<&str>,
    shortcut: Option<Shortcut>,
) -> Response {
    let m = theme::metrics();
    let keys = shortcut.map(chord_keys).unwrap_or_default();
    let pad = m.tooltip_pad;
    floating(ui, Layer::Tooltip, pad, |ui| {
        let p = theme::palette(ui.ctx());
        ui.set_max_width(TOOLTIP_WIDTH - f32::from(pad.left + pad.right));
        ui.spacing_mut().item_spacing = vec2(theme::SM, 2.0);
        let font = FontId::new(m.tooltip_size, theme::medium());
        // The keycaps sit on the first line's centre: the text drops by half the difference.
        let line = ui.fonts_mut(|f| f.row_height(&font));
        let drop = if keys.is_empty() {
            0.0
        } else {
            ((TOOLTIP_KEYCAP - line) / 2.0).max(0.0)
        };
        ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
            ui.vertical(|ui| {
                ui.add_space(drop);
                ui.add(egui::Label::new(RichText::new(text).font(font).color(p.on_tooltip)).wrap());
                if let Some(d) = detail {
                    ui.add(
                        egui::Label::new(
                            RichText::new(d)
                                .size(m.tooltip_size)
                                .color(p.on_tooltip_dim),
                        )
                        .wrap(),
                    );
                }
            });
            if !keys.is_empty() {
                // Top-aligned like the text (a stock `horizontal` would centre the caps in a
                // control-high row, below the first line).
                ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for k in &keys {
                        tooltip_keycap(ui, k);
                    }
                });
            }
        });
    })
    .response
}

/// A keycap inside a tooltip: 18 pt, the theme's keycap in dark themes; on the dark tooltip of a
/// light theme, a dark cap stepped up from the tooltip with no bottom line.
fn tooltip_keycap(ui: &mut Ui, key: &str) {
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    let font = FontId::new(m.chord_size, egui::FontFamily::Monospace);
    let (fill, text, ring, bottom) = (
        s.tooltip_key,
        s.on_tooltip_key,
        s.tooltip_key_edge,
        s.tooltip_key_base,
    );
    let g = ui
        .painter()
        .layout_no_wrap(key.to_owned(), font, Color32::PLACEHOLDER);
    let h = TOOLTIP_KEYCAP;
    let w = (g.size().x + 10.0).max(h);
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let radius = CornerRadius::same(m.keycap_radius.min(4));
    let painter = ui.painter();
    painter.rect_filled(rect, radius, fill);
    painter.rect_stroke(rect, radius, Stroke::new(1.0, ring), StrokeKind::Inside);
    if let Some(b) = bottom {
        painter.hline(
            Rangef::new(rect.left() + 2.0, rect.right() - 2.0),
            rect.bottom() - 1.5,
            Stroke::new(1.0, b),
        );
    }
    painter.galley(rect.center() - g.size() / 2.0, g, text);
}

// --------------------------------------------------------------------------------- popover

/// A small panel anchored to a control (diff options, author card, theme quick-pick): the overlay
/// surface with `card_pad` inside, 6 pt from the anchor and at least as wide. Clicking the anchor
/// toggles it; a click outside or Esc closes it and gives focus back to the anchor. One popover is
/// open at a time.
#[must_use = "call .show(ui, ..)"]
#[derive(Debug)]
pub struct Popover<'a> {
    id: Id,
    anchor: &'a Response,
    width: Option<f32>,
    open: Option<&'a mut bool>,
    force: Option<ForceState>,
}

impl<'a> Popover<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug, anchor: &'a Response) -> Self {
        Self {
            id: anchor.id.with(("kit-popover", Id::new(id_salt))),
            anchor,
            width: None,
            open: None,
            force: None,
        }
    }
    /// The id to close it by ([`egui::Popup::close_id`]).
    pub fn id(&self) -> Id {
        self.id
    }
    /// The content width (default: the content's own, at least the anchor's).
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    /// Keeps the open state in `open` instead of toggling on anchor clicks.
    pub fn open(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }
    /// Shows it with forced states (`OPEN` keeps it open; the kit's tests only).
    #[cfg(test)]
    pub(crate) fn force(mut self, force: ForceState) -> Self {
        self.force = Some(force);
        self
    }

    pub fn show<R>(self, add: impl FnOnce(&mut Ui) -> R) -> Option<InnerResponse<R>> {
        let Popover {
            id,
            anchor,
            width,
            open,
            force,
        } = self;
        let ctx = anchor.ctx.clone();
        let current = Id::new("canopy-kit-open-popover");
        let was_open = Popup::is_id_open(&ctx, id);
        let mut popup = Popup::from_toggle_button_response(anchor)
            .id(id)
            .kind(PopupKind::Popup)
            .frame(Frame::NONE)
            .gap(POPOVER_GAP)
            .align(RectAlign::BOTTOM_START)
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside);
        if let Some(w) = width {
            popup = popup.width(w);
        }
        let forced = force.is_some_and(|f| f.open);
        if forced {
            popup = popup.open(true);
        } else if let Some(o) = open {
            popup = popup.open_bool(o);
        }
        let keyboard = anchor.clicked() && !anchor.clicked_by(egui::PointerButton::Primary);
        let anchor_rect = anchor.rect;
        let progress = force.and_then(|f| f.progress);
        let shown = popup.show(|ui| {
            let layer = ui.layer_id();
            let e = Entrance::of(ui.ctx(), id, motion::BASE, motion::SLOW, keyboard, progress);
            ui.multiply_opacity(e.fade);
            let start = next_shape(ui.ctx(), layer);
            let pad = crate::points_i8(theme::metrics().card_pad);
            let r = floating(ui, Layer::Popover, Margin::same(pad), |ui| {
                let inner = anchor_rect.width() - 2.0 * f32::from(pad);
                ui.set_min_width(width.unwrap_or(0.0).max(inner));
                add(ui)
            });
            let rect = r.response.rect;
            let (origin, drop) = popup_origin(rect, anchor_rect);
            transform_since(
                ui.ctx(),
                layer,
                start,
                e.transform(origin, POPUP_SCALE, drop),
            );
            if !forced {
                let end = next_shape(ui.ctx(), layer);
                record_exit(
                    ui.ctx(),
                    id,
                    layer,
                    0..0,
                    start.0..end.0,
                    ExitMotion::popup(origin),
                );
            }
            r.inner
        });
        let now_open = !forced && shown.is_some() && Popup::is_id_open(&ctx, id);
        if now_open && !was_open {
            // One popover at a time: opening this one closes the one open before.
            let before: Option<Id> = ctx.data(|d| d.get_temp(current));
            if let Some(o) = before.filter(|o| *o != id) {
                Popup::close_id(&ctx, o);
            }
            ctx.data_mut(|d| d.insert_temp(current, id));
        } else if was_open && !now_open && !forced {
            anchor.request_focus();
        }
        shown
    }
}

/// The corner of a popup at `rect` nearest its anchor, and the offset it drops in from (towards
/// the anchor).
pub(crate) fn popup_origin(rect: Rect, anchor: Rect) -> (Pos2, Vec2) {
    let above = rect.center().y < anchor.center().y;
    let x = if anchor.center().x <= rect.center().x {
        rect.left()
    } else {
        rect.right()
    };
    if above {
        (pos2(x, rect.bottom()), vec2(0.0, POPUP_DROP))
    } else {
        (pos2(x, rect.top()), vec2(0.0, -POPUP_DROP))
    }
}

// ---------------------------------------------------------------------------------- dialog

/// A dialog's width: confirmations or forms (spec 2.5), or a document to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DialogWidth {
    #[default]
    Confirm,
    Form,
    /// A document (the terms, the privacy policy): its text at the [`reading_measure`] inside
    /// the dialog's padding, never wider than the [`modal_room`].
    Reading,
}

impl DialogWidth {
    /// The card's width in points, before a small window narrows it further.
    pub fn points(self, ctx: &Context) -> f32 {
        let m = theme::metrics();
        match self {
            Self::Confirm => m.dialog_width,
            Self::Form => m.dialog_width_form,
            Self::Reading => {
                (reading_measure(ctx) + 2.0 * m.dialog_pad).min(modal_room(ctx).width())
            }
        }
    }
}

/// How many characters of prose a line at the reading measure holds.
pub const READING_CHARS: f32 = 70.0;

/// The width of a line of document text (`subtitle_size`) holding [`READING_CHARS`] characters
/// of ordinary prose, measured on a sample sentence rather than one glyph.
pub fn reading_measure(ctx: &Context) -> f32 {
    const SAMPLE: &str = "Canopy sends crash data only if you agree, and asks before it sends.";
    let font = FontId::proportional(theme::metrics().subtitle_size);
    let width = ctx.fonts_mut(|f| {
        f.layout_no_wrap(SAMPLE.to_owned(), font, Color32::PLACEHOLDER)
            .size()
            .x
    });
    (width / crate::count_f32(SAMPLE.chars().count()) * READING_CHARS).round()
}

/// What the user did with a dialog this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogResult {
    /// Still open.
    Open,
    /// Cancel, Esc, the close button or a click on the scrim.
    Cancel,
    /// The confirming button (or Enter, outside destructive dialogs).
    Confirm,
}

/// Which control takes focus when a dialog opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialFocus {
    Cancel,
    Confirm,
    /// The body requests focus itself (a form's first field).
    Body,
}

/// The footer's buttons, set inside [`Dialog::footer`].
#[derive(Debug, Default)]
pub struct Footer {
    cancel: Option<String>,
    confirm: Option<String>,
    confirm_icon: Option<String>,
    confirm_enabled: bool,
    confirm_loading: bool,
    confirm_reason: Option<String>,
    /// Two equal choices ([`Footer::choice`]): both Secondary, and Enter picks neither.
    neutral: bool,
}

impl Footer {
    /// A Cancel button.
    pub fn cancel(&mut self) -> &mut Self {
        self.cancel = Some("Cancel".to_owned());
        self
    }
    /// A Cancel button with another label ("Close", "Not now").
    pub fn cancel_label(&mut self, label: impl Into<String>) -> &mut Self {
        self.cancel = Some(label.into());
        self
    }
    /// The confirming button, rightmost: primary, or Danger in destructive dialogs. Label it verb
    /// and object ("Delete branch").
    pub fn confirm(&mut self, label: impl Into<String>) -> &mut Self {
        self.confirm = Some(label.into());
        self.confirm_enabled = true;
        self
    }
    /// A leading glyph on the confirming button.
    pub fn confirm_icon(&mut self, glyph: &str) -> &mut Self {
        self.confirm_icon = Some(glyph.to_owned());
        self
    }
    /// Disables the confirming button, saying why in its tooltip.
    pub fn confirm_disabled(&mut self, reason: impl Into<String>) -> &mut Self {
        self.confirm_enabled = false;
        self.confirm_reason = Some(reason.into());
        self
    }
    /// The confirmed operation is running.
    pub fn confirm_loading(&mut self, loading: bool) -> &mut Self {
        self.confirm_loading = loading;
        self
    }
    /// Two equal answers to a question with no safe default (consent): both Secondary, the same
    /// size, neither focused unless [`Dialog::initial_focus`] says so, and Enter picks neither.
    /// `first` (rightmost) answers `Confirm`, `second` answers `Cancel`.
    pub fn choice(&mut self, first: impl Into<String>, second: impl Into<String>) -> &mut Self {
        self.confirm = Some(first.into());
        self.confirm_enabled = true;
        self.cancel = Some(second.into());
        self.neutral = true;
        self
    }
}

/// What a dialog shows between its description and its footer.
type DialogBody<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;
/// A footer row of the dialog's own ([`Dialog::custom_footer`]), answering the dialog.
type DialogFooter<'a> = Box<dyn FnOnce(&mut Ui) -> DialogResult + 'a>;

/// A modal dialog (concept `.k-dialog`) over the scrim:
/// `Dialog::new("delete-branch").title("Delete branch 'feature/x'?").icon(icon::TRASH).destructive()
/// .description("…").footer(|f| { f.cancel().confirm("Delete branch"); }).show(ctx)`.
///
/// Focus moves into it on open (Cancel in destructive dialogs, else the confirming button, unless
/// the body takes it), stays inside while it is open and returns to the control that had it when
/// the dialog closes through its result. Esc and a click on the scrim cancel; Enter confirms except
/// in destructive dialogs, where only the focused button answers.
#[must_use = "call .show(ctx)"]
#[expect(
    missing_debug_implementations,
    reason = "holds the content closures, which have no Debug"
)]
pub struct Dialog<'a> {
    id: Id,
    title: String,
    icon: Option<String>,
    destructive: bool,
    description: Option<String>,
    body: Option<DialogBody<'a>>,
    footer: Footer,
    width: DialogWidth,
    closable: bool,
    blocking: bool,
    initial_focus: Option<InitialFocus>,
    force: Option<ForceState>,
    label: Option<String>,
    header: Option<DialogBody<'a>>,
    fill_height: bool,
    progress_lane: bool,
    scroll_salt: Option<Id>,
    custom_footer: Option<DialogFooter<'a>>,
}

impl<'a> Dialog<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            id: Id::new(("kit-dialog", Id::new(id_salt))),
            title: String::new(),
            icon: None,
            destructive: false,
            description: None,
            body: None,
            footer: Footer::default(),
            width: DialogWidth::Confirm,
            closable: false,
            blocking: false,
            initial_focus: None,
            force: None,
            label: None,
            header: None,
            fill_height: false,
            progress_lane: false,
            scroll_salt: None,
            custom_footer: None,
        }
    }
    /// The question or task, as a title (destructive: the question, "Delete branch 'x'?").
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }
    /// A glyph in a 30 pt disc before the title.
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    /// Destroys something: danger disc and button, focus starts on Cancel, Enter does not confirm.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
    /// What happens and whether it can be undone (13 pt, dim).
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.description = Some(text.into());
        self
    }
    /// Anything else between the description and the footer; scrolls when the card is taller
    /// than the window.
    pub fn body(mut self, add: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.body = Some(Box::new(add));
        self
    }
    pub fn footer(mut self, f: impl FnOnce(&mut Footer)) -> Self {
        f(&mut self.footer);
        self
    }
    pub fn width(mut self, width: DialogWidth) -> Self {
        self.width = width;
        self
    }
    /// A close button top-right (non-blocking dialogs only).
    pub fn closable(mut self) -> Self {
        self.closable = true;
        self
    }
    pub fn initial_focus(mut self, focus: InitialFocus) -> Self {
        self.initial_focus = Some(focus);
        self
    }
    /// Must be answered with its buttons: a click on the scrim (or Esc reaching the modal) does
    /// not cancel it (the start-up consent, git's credential prompts).
    pub fn blocking(mut self) -> Self {
        self.blocking = true;
        self
    }
    /// What screen readers call the dialog when its title is left empty, as when a
    /// [`Self::header`] takes the title's place.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }
    /// Controls in the title row after the title (the legal reader's document tabs), left to
    /// right and centred on the row, up to the close button. With an empty title they start
    /// right after the icon.
    pub fn header(mut self, add: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.header = Some(Box::new(add));
        self
    }
    /// The card is as tall as a dialog may be (the modal room, at most
    /// [`theme::SETTINGS_MAX_HEIGHT`]) and the body fills what the title row and the footer leave,
    /// however short it is: a document to read.
    pub fn fill_height(mut self) -> Self {
        self.fill_height = true;
        self
    }
    /// A seam across the card under the title row, and along it a 2 pt `accent` line as long as
    /// the share of the body scrolled past: how far the reader has come.
    pub fn progress_lane(mut self) -> Self {
        self.progress_lane = true;
        self
    }
    /// Keys the body's scroll position by `salt` too, so a dialog that switches what its body
    /// shows (the reader's two documents) keeps a position for each.
    pub fn scroll_id(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.scroll_salt = Some(Id::new(salt));
        self
    }
    /// The footer row is `add`'s, left to right, in place of [`Self::footer`]'s buttons
    /// (the reader's Back beside the consent's answers). What it returns answers the dialog, as
    /// the buttons would: `Open` keeps it open, and focus returns to its trigger otherwise. No
    /// control takes focus when it opens unless `add` asks for it.
    pub fn custom_footer(mut self, add: impl FnOnce(&mut Ui) -> DialogResult + 'a) -> Self {
        self.custom_footer = Some(Box::new(add));
        self
    }
    /// Shows the card where the cursor is, without the modal layer, scrim or focus handling, with
    /// forced states (the gallery only).
    pub(crate) fn show_inline(mut self, ui: &mut Ui, force: ForceState) -> DialogResult {
        self.force = Some(force);
        let max_h = ui.available_height().max(120.0);
        let (result, _) = self.card(ui, max_h, false);
        result
    }

    /// Shows the dialog for this frame. Call it every frame while the dialog is open; stop once
    /// it returns `Cancel` or `Confirm`.
    pub fn show(self, ctx: &Context) -> DialogResult {
        let id = self.id;
        let blocking = self.blocking;
        let screen = ctx.content_rect();
        let (_, fresh) = presence(ctx, id);
        if fresh {
            let mut back = super::a11y::FocusReturn::default();
            back.remember(ctx);
            ctx.data_mut(|d| d.insert_temp(id.with("kit-trigger"), back));
        }
        let e = Entrance::of(ctx, id, motion::SLOW, motion::SLOW, false, None);
        let fade = e.fade;
        // Centred on the window, as tall as fits between the tab strip and the status bar.
        let room = modal_room(ctx);
        let max_h = (2.0 * (screen.center().y - room.top()).min(room.bottom() - screen.center().y))
            .min(screen.height() - DIALOG_MARGIN)
            .max(120.0);
        let modal = egui::Modal::new(id.with("modal"))
            .backdrop_color(theme::palette(ctx).scrim.gamma_multiply(fade))
            .frame(Frame::NONE)
            .show(ctx, |ui| {
                let layer = ui.layer_id();
                // The scrim egui painted is the fixed part of the exit; the card moves.
                let start = next_shape(ui.ctx(), layer);
                ui.multiply_opacity(fade);
                let (result, rect) = self.card(ui, max_h, fresh);
                let center = rect.center();
                transform_since(
                    ui.ctx(),
                    layer,
                    start,
                    e.transform(center, DIALOG_SCALE, Vec2::ZERO),
                );
                let end = next_shape(ui.ctx(), layer);
                record_exit(
                    ui.ctx(),
                    id,
                    layer,
                    0..start.0,
                    start.0..end.0,
                    ExitMotion {
                        seconds: motion::EXIT_DIALOG,
                        scale: DIALOG_EXIT_SCALE,
                        origin: center,
                        offset: Vec2::ZERO,
                        instant_on_input: false,
                    },
                );
                result
            });
        let mut result = modal.inner;
        if result == DialogResult::Open && !blocking && modal.should_close() {
            result = DialogResult::Cancel;
        }
        if result != DialogResult::Open {
            Self::restore_focus_id(ctx, id);
        }
        result
    }

    /// Returns focus when an app-level handler closes the dialog without drawing its result.
    pub fn restore_focus(ctx: &Context, id_salt: impl std::hash::Hash + std::fmt::Debug) {
        Self::restore_focus_id(ctx, Id::new(("kit-dialog", Id::new(id_salt))));
    }

    fn restore_focus_id(ctx: &Context, id: Id) {
        let back =
            ctx.data_mut(|d| d.remove_temp::<super::a11y::FocusReturn>(id.with("kit-trigger")));
        if let Some(back) = back {
            // Escape and the closing modal clear focus at the end of this pass.
            let pending = DialogFocusReturn {
                back,
                layer: LayerId::new(Order::Foreground, id.with("modal")),
                closed: ctx.cumulative_pass_nr(),
            };
            if ctx
                .with_plugin::<DialogFocusReturn, _>(|plugin| {
                    *plugin = pending.clone();
                })
                .is_none()
            {
                ctx.add_plugin(pending);
            }
            ctx.request_repaint();
        }
    }

    /// The card at the cursor, at most `max_h` tall. Returns the result and the card's rect.
    fn card(self, ui: &mut Ui, max_h: f32, fresh: bool) -> (DialogResult, Rect) {
        let Dialog {
            id,
            title,
            icon: glyph,
            destructive,
            description,
            body,
            footer,
            width,
            closable,
            blocking: _,
            initial_focus,
            force,
            label,
            header,
            fill_height,
            progress_lane,
            scroll_salt,
            custom_footer,
        } = self;
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let m = theme::metrics();
        let screen = ctx.content_rect();
        // A document takes all the height a dialog may have; everything else its content's.
        let max_h = if fill_height {
            max_h.min(theme::SETTINGS_MAX_HEIGHT)
        } else {
            max_h
        };
        let spoken = if title.is_empty() {
            label.unwrap_or_default()
        } else {
            title.clone()
        };
        let w = width
            .points(&ctx)
            .min(screen.width() - DIALOG_MARGIN)
            .min(ui.available_width().max(DISC * 4.0));
        let pad = m.dialog_pad;
        let mut result = DialogResult::Open;
        let enter = ctx.input(|i| i.key_pressed(Key::Enter));
        let r = floating(
            ui,
            Layer::Dialog,
            Margin::same(crate::points_i8(pad)),
            |ui| {
                // The card's own node: its controls are announced as its parts, and screen
                // readers keep to it while it is open (A11Y-7). The gallery's inline card is not
                // modal.
                if force.is_some() {
                    let role = if destructive {
                        egui::accesskit::Role::AlertDialog
                    } else {
                        egui::accesskit::Role::Dialog
                    };
                    super::a11y::group(ui, role, &spoken);
                } else {
                    super::a11y::dialog(ui, &spoken, destructive);
                }
                ui.set_width(w - 2.0 * pad);
                // Stacked whatever the caller's layout is.
                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing = vec2(theme::SM, m.dialog_gap);
                    let top = ui.cursor().top();
                    // Title row.
                    let title_row = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        if let Some(g) = &glyph {
                            let (rect, _) =
                                ui.allocate_exact_size(Vec2::splat(DISC), Sense::hover());
                            let (tint, color) = if destructive {
                                (p.danger_tint(), p.danger)
                            } else {
                                (p.accent_tint(), p.accent)
                            };
                            ui.painter().circle_filled(rect.center(), DISC / 2.0, tint);
                            ui.painter().text(
                                rect.center(),
                                Align2::CENTER_CENTER,
                                g,
                                FontId::proportional(m.icon_md),
                                color,
                            );
                        }
                        // The close button sits at the right end, past the title.
                        let close_w = if closable {
                            m.control_sm + 2.0 + ui.spacing().item_spacing.x
                        } else {
                            0.0
                        };
                        let title_w = (ui.available_width() - close_w).max(0.0);
                        ui.allocate_ui_with_layout(
                            vec2(title_w, DISC),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.set_min_width(title_w);
                                if !title.is_empty() {
                                    let text = egui::Label::new(
                                        RichText::new(&title)
                                            .font(theme::heading(m.title_size))
                                            .color(p.text),
                                    );
                                    // Beside header controls the title keeps to one line.
                                    ui.add(if header.is_some() {
                                        text.truncate()
                                    } else {
                                        text.wrap()
                                    });
                                }
                                if let Some(add) = header {
                                    ui.spacing_mut().item_spacing.x = theme::MD;
                                    add(ui);
                                }
                            },
                        );
                        if closable
                            && IconButton::new(icon::X, "Close")
                                .size(Size::Sm)
                                .id_salt(id.with("close"))
                                .show(ui)
                                .clicked()
                        {
                            result = DialogResult::Cancel;
                        }
                    });
                    // The lane under the title row, across the whole card.
                    let full = Rangef::new(ui.max_rect().left() - pad, ui.max_rect().right() + pad);
                    let lane = title_row.response.rect.bottom() + m.dialog_gap / 2.0;
                    if progress_lane {
                        paint::seam_h(ui.painter(), full, lane, s.border);
                    }
                    // Description and body, scrolling inside the card when it would be too tall.
                    let used = ui.cursor().top() - top;
                    let footer_h = m.control + theme::XS;
                    let room = (max_h - 2.0 * pad - used - footer_h - m.dialog_gap).max(48.0);
                    let has_body = description.is_some() || body.is_some();
                    let scroll_id =
                        scroll_salt.map_or(id.with("body"), |salt| id.with(("body", salt)));
                    let scrolled = if has_body {
                        // A document's scroll bar runs in the card's right padding, beside the
                        // text column instead of over its line ends.
                        let column = ui.available_width();
                        let bar_room = if width == DialogWidth::Reading {
                            pad - 2.0 * BAR_INSET
                        } else {
                            0.0
                        };
                        let mut area = ui.available_rect_before_wrap();
                        area.max.x += bar_room;
                        // A child, so the bar's room does not widen the card.
                        let mut inside = ui.new_child(
                            UiBuilder::new()
                                .id_salt("kit-dialog-body")
                                .max_rect(area)
                                .layout(Layout::top_down(Align::Min)),
                        );
                        let out = scroll_body(
                            &mut inside,
                            scroll_id,
                            room,
                            ground(&ctx, Layer::Dialog),
                            |ui| {
                                ui.set_max_width(column);
                                ui.spacing_mut().item_spacing.y = m.dialog_gap;
                                if let Some(d) = &description {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(d)
                                                .size(m.body_size)
                                                .line_height(Some((m.body_size * 1.5).round()))
                                                .color(p.text_dim),
                                        )
                                        .wrap(),
                                    );
                                }
                                if let Some(b) = body {
                                    b(ui);
                                }
                            },
                        );
                        let body_h = inside.min_rect().height();
                        ui.allocate_exact_size(vec2(column, body_h), Sense::hover());
                        let scrollable = out.content_size.y - out.inner_rect.height();
                        if progress_lane && scrollable > 0.5 {
                            let t = (out.state.offset.y / scrollable).clamp(0.0, 1.0);
                            let line = Rect::from_min_max(
                                pos2(full.min, lane - PROGRESS / 2.0),
                                pos2(full.min + full.span() * t, lane + PROGRESS / 2.0),
                            );
                            ui.painter().rect_filled(line, CornerRadius::ZERO, p.accent);
                        }
                        // A document's body keeps the card at its full height.
                        if fill_height {
                            ui.add_space((room - body_h).max(0.0));
                        }
                        scrollable > 0.5
                    } else {
                        false
                    };
                    // Footer: 4 pt more above it, a seam only when the body scrolls.
                    ui.add_space(theme::XS);
                    let (row, _) = ui
                        .allocate_exact_size(vec2(ui.available_width(), m.control), Sense::hover());
                    if scrolled {
                        paint::seam_h(
                            ui.painter(),
                            Rangef::new(row.left() - pad, row.right() + pad),
                            row.top() - theme::XS - m.dialog_gap / 2.0,
                            s.border,
                        );
                    }
                    if let Some(add) = custom_footer {
                        let answer = ui
                            .scope_builder(
                                UiBuilder::new()
                                    .max_rect(row)
                                    .layout(Layout::left_to_right(Align::Center)),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = theme::SM;
                                    add(ui)
                                },
                            )
                            .inner;
                        if answer != DialogResult::Open {
                            result = answer;
                        }
                    } else {
                        // The buttons sit at the right but are made left to right, Cancel first,
                        // so Tab meets them in reading order (the row is placed by its width last
                        // frame).
                        let width_key = id.with("kit-footer-width");
                        let last_w: f32 = ctx.data(|d| d.get_temp(width_key)).unwrap_or(0.0);
                        let slot = Rect::from_min_max(
                            pos2((row.right() - last_w).max(row.left()), row.top()),
                            row.max,
                        );
                        let mut cancel_id = None;
                        let mut confirm_id = None;
                        let buttons = ui.scope_builder(
                            UiBuilder::new()
                                .max_rect(slot)
                                .layout(Layout::left_to_right(Align::Center)),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = theme::SM;
                                if let Some(label) = &footer.cancel {
                                    let b = Button::new(label.as_str())
                                        .secondary()
                                        .size(Size::Md)
                                        .id_salt(id.with("cancel"));
                                    let forced = force.filter(|f| f.focus && destructive);
                                    let r = match forced {
                                        Some(f) => b.show_forced(ui, f),
                                        None => b.show(ui),
                                    };
                                    if r.clicked() {
                                        result = DialogResult::Cancel;
                                    }
                                    cancel_id = Some(r.id);
                                }
                                if let Some(label) = &footer.confirm {
                                    let mut b = Button::new(label.as_str())
                                        .size(Size::Md)
                                        .enabled(footer.confirm_enabled)
                                        .loading(footer.confirm_loading)
                                        .id_salt(id.with("confirm"));
                                    b = if footer.neutral {
                                        b.secondary()
                                    } else if destructive {
                                        b.danger()
                                    } else {
                                        b.primary()
                                    };
                                    if let Some(g) = &footer.confirm_icon {
                                        b = b.icon(g);
                                    }
                                    if let Some(r) = &footer.confirm_reason {
                                        b = b.disabled_reason(r.as_str());
                                    }
                                    let forced = force.filter(|f| f.focus && !destructive);
                                    let r = match forced {
                                        Some(f) => b.show_forced(ui, f),
                                        None => b.show(ui),
                                    };
                                    if r.clicked() {
                                        result = DialogResult::Confirm;
                                    }
                                    confirm_id = Some(r.id);
                                }
                            },
                        );
                        let width = buttons.response.rect.width();
                        if (width - last_w).abs() > 0.5 {
                            ctx.data_mut(|d| d.insert_temp(width_key, width));
                            ctx.request_repaint();
                        }
                        if fresh {
                            let want = initial_focus.unwrap_or(if footer.neutral {
                                InitialFocus::Body
                            } else if destructive {
                                InitialFocus::Cancel
                            } else {
                                InitialFocus::Confirm
                            });
                            let target = match want {
                                InitialFocus::Cancel => cancel_id.or(confirm_id),
                                InitialFocus::Confirm => confirm_id.or(cancel_id),
                                InitialFocus::Body => None,
                            };
                            if let Some(t) = target {
                                ctx.memory_mut(|m| m.request_focus(t));
                                // Enter presses this button: it shows, whatever opened the dialog.
                                super::show_focus(&ctx);
                            }
                        }
                        // Enter confirms only when no control has focus (or Confirm itself has
                        // it): a focused button, checkbox, select or field answers Enter on its
                        // own, and must not also confirm the dialog (the credential prompt's
                        // "Show" must not submit the password).
                        let focus_free = ctx
                            .memory(egui::Memory::focused)
                            .is_none_or(|f| Some(f) == confirm_id);
                        if result == DialogResult::Open
                            && force.is_none()
                            && !destructive
                            && !footer.neutral
                            && enter
                            && footer.confirm.is_some()
                            && footer.confirm_enabled
                            && !footer.confirm_loading
                            && focus_free
                        {
                            result = DialogResult::Confirm;
                        }
                    }
                });
            },
        );
        (result, r.response.rect)
    }
}

/// Gives focus back after the closed modal has left egui's focus filter.
#[derive(Clone)]
struct DialogFocusReturn {
    back: super::a11y::FocusReturn,
    layer: LayerId,
    closed: u64,
}

impl egui::Plugin for DialogFocusReturn {
    fn debug_name(&self) -> &'static str {
        "canopy-dialog-focus-return"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx();
        if ctx.cumulative_pass_nr() > self.closed + 1 {
            self.back.restore(ctx, self.layer);
        } else {
            ctx.request_repaint();
        }
    }
}

// --------------------------------------------------------------------------------- gallery

/// The gallery's Overlays section, drawn inline and forced open: tooltips with keycaps, a
/// popover, both dialogs on a local scrim, their entrances part-way, and the toast cards.
pub(crate) fn gallery(ui: &mut Ui) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let small = |ui: &mut Ui, text: &str| {
        ui.label(RichText::new(text).size(m.small_size).color(p.text_faint));
    };
    small(ui, "Tooltips: plain, with a shortcut, with a second line");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(theme::MD, theme::SM);
        tooltip_body(ui, "Settings", None, None);
        tooltip_body(ui, "Fetch all remotes", None, Some("f".into()));
        tooltip_body(
            ui,
            "Push main to origin",
            Some("2 commits ahead"),
            Some("Ctrl+Enter".into()),
        );
    });
    small(ui, "Tooltip entrance at 0.3, 0.6 and 1");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::MD;
        for t in [0.3, 0.6, 1.0] {
            entering(ui, t, TOOLTIP_SCALE, vec2(0.0, TOOLTIP_RISE), |ui| {
                tooltip_body(ui, "Fetch all remotes", None, Some("f".into()));
            });
        }
    });
    ui.add_space(theme::SM);
    small(ui, "Popover");
    let pad = crate::points_i8(m.card_pad);
    floating(ui, Layer::Popover, Margin::same(pad), |ui| {
        ui.set_width(240.0);
        ui.label(
            RichText::new("Diff options")
                .font(FontId::new(m.body_size, theme::semibold()))
                .color(p.text),
        );
        ui.label(
            RichText::new("Context lines, whitespace and word diff apply to every file.")
                .size(m.meta_size)
                .color(p.text_dim),
        );
        ui.horizontal(|ui| {
            Button::new("Reset")
                .ghost()
                .size(Size::Sm)
                .id_salt("g-pop-reset")
                .show(ui);
            Button::new("Apply")
                .primary()
                .size(Size::Sm)
                .id_salt("g-pop-apply")
                .show(ui);
        });
    });
    ui.add_space(theme::SM);
    small(
        ui,
        "Dialogs on a scrim: destructive (Cancel focused) and normal",
    );
    for (i, destructive) in [true, false].into_iter().enumerate() {
        let width = ui.available_width();
        let height = if destructive { 250.0 } else { 220.0 };
        let (area, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        // A busy backdrop so the scrim reads.
        let painter = ui.painter_at(area);
        for row in 0..crate::saturate_usize(height / m.history_row_height) {
            let y = area.top()
                + crate::count_f32(row) * m.history_row_height
                + m.history_row_height / 2.0;
            painter.text(
                pos2(area.left() + theme::MD, y),
                Align2::LEFT_CENTER,
                [
                    "docs: add API reference",
                    "feat(api): add ETag headers",
                    "fix(ui): improve contrast",
                ][row % 3],
                FontId::proportional(m.body_size),
                p.text,
            );
        }
        scrim(&painter, area, 1.0);
        let inner = area.shrink(theme::LG);
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::top_down(Align::Center)),
            |ui| {
                let d = if destructive {
                    Dialog::new(("g-dialog", i))
                        .title("Delete branch feature/dark-mode?")
                        .icon(icon::WARNING)
                        .destructive()
                        .description(
                            "feature/dark-mode has 1 commit that is on no other branch. You can \
                             restore it from the reflog for 90 days.",
                        )
                        .footer(|f| {
                            f.cancel()
                                .confirm("Delete branch")
                                .confirm_icon(icon::TRASH);
                        })
                } else {
                    Dialog::new(("g-dialog", i))
                        .title("Rename branch")
                        .icon(icon::PENCIL_SIMPLE)
                        .closable()
                        .description("Renames feature/auth locally. Its upstream keeps its name.")
                        .footer(|f| {
                            f.cancel().confirm("Rename");
                        })
                };
                d.show_inline(ui, ForceState::FOCUS);
            },
        );
        // Below the whole scrim, however tall the card came out.
        ui.advance_cursor_after_rect(area);
    }
    small(
        ui,
        "Reading dialog: tabs after the title, full height, the progress lane and its own footer",
    );
    reading_example(ui);
    small(ui, "Live: hover the icon, open the popover, open a dialog");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::LG;
        IconButton::new(icon::ARROWS_CLOCKWISE, "Fetch")
            .id_salt("g-live-tip")
            .show(ui)
            .kit_tooltip("Fetch all remotes")
            .detail("origin and upstream")
            .shortcut("f")
            .show();
        let anchor = Button::new("Diff options")
            .caret()
            .id_salt("g-live-popover")
            .show(ui);
        Popover::new("g-live-popover", &anchor)
            .width(240.0)
            .show(|ui| {
                ui.label(
                    RichText::new("Context lines, whitespace and word diff apply to every file.")
                        .size(m.meta_size)
                        .color(p.text_dim),
                );
            });
        let open_key = ui.id().with("g-live-dialog-open");
        if Button::new("Open dialog")
            .id_salt("g-live-dialog")
            .show(ui)
            .clicked()
        {
            ui.ctx().data_mut(|d| d.insert_temp(open_key, true));
        }
        if ui
            .ctx()
            .data(|d| d.get_temp::<bool>(open_key).unwrap_or(false))
        {
            let result = Dialog::new(open_key)
                .title("Create tag")
                .icon(icon::TAG)
                .width(DialogWidth::Form)
                .description("Tags 38b14e7 on main.")
                .body(|ui| {
                    ui.label(
                        RichText::new("v1.4.0")
                            .font(FontId::monospace(m.mono_size))
                            .color(p.text),
                    );
                })
                .initial_focus(InitialFocus::Confirm)
                .footer(|f| {
                    f.cancel_label("Not now").confirm("Create tag");
                })
                .show(ui.ctx());
            if result != DialogResult::Open {
                ui.ctx().data_mut(|d| d.remove::<bool>(open_key));
            }
        }
        let read_key = ui.id().with("g-live-reading-open");
        if Button::new("Open document")
            .id_salt("g-live-reading")
            .show(ui)
            .clicked()
        {
            ui.ctx().data_mut(|d| d.insert_temp(read_key, true));
        }
        if ui
            .ctx()
            .data(|d| d.get_temp::<bool>(read_key).unwrap_or(false))
        {
            let result = Dialog::new(read_key)
                .title("Privacy Policy")
                .closable()
                .width(DialogWidth::Reading)
                .body(|ui| {
                    for text in [
                        "Canopy runs on your machine. It talks to the git executable for your \
                         repositories and, only when you use those features, to GitHub or \
                         GitLab for pull requests and author pictures.",
                        "A reading dialog sets its text at the reading measure, about seventy \
                         characters a line, so a long document reads comfortably at any \
                         window width.",
                    ] {
                        ui.label(RichText::new(text).size(m.subtitle_size).color(p.text));
                        ui.add_space(theme::SM);
                    }
                })
                .footer(|f| {
                    f.confirm("Close");
                })
                .show(ui.ctx());
            if result != DialogResult::Open {
                ui.ctx().data_mut(|d| d.remove::<bool>(read_key));
            }
        }
    });
    ui.add_space(theme::SM);
    small(ui, "Dialog entrance at 0.3, 0.6 and 1");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::MD;
        for (i, t) in [0.3, 0.6, 1.0].into_iter().enumerate() {
            entering(ui, t, DIALOG_SCALE, Vec2::ZERO, |ui| {
                ui.set_max_width(180.0);
                Dialog::new(("g-dialog-motion", i))
                    .title("Discard changes?")
                    .destructive()
                    .footer(|f| {
                        f.confirm("Discard");
                    })
                    .show_inline(ui, ForceState::default());
            });
        }
    });
    ui.add_space(theme::SM);
    super::toast::gallery(ui);
}

/// The gallery's reading dialog on a scrim, as the legal reader uses it: two documents as tabs
/// after the icon, the full height of its area, the progress lane over a body that scrolls, and a
/// footer of its own (Back beside the answers).
fn reading_example(ui: &mut Ui) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let width = ui.available_width();
    let (area, _) = ui.allocate_exact_size(vec2(width, 340.0), Sense::hover());
    scrim(&ui.painter_at(area), area, 1.0);
    let tab_key = ui.id().with("g-reading-tab");
    let mut tab: u8 = ui.data(|d| d.get_temp(tab_key)).unwrap_or(0);
    let options = [(0_u8, "Terms of Service"), (1_u8, "Privacy Policy")];
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(area.shrink(theme::LG))
            .layout(Layout::top_down(Align::Center)),
        |ui| {
            Dialog::new("g-reading")
                .label("Terms of Service")
                .icon(icon::SCROLL)
                .width(DialogWidth::Reading)
                .fill_height()
                .progress_lane()
                .scroll_id(tab)
                .header(|ui| {
                    super::choice::UnderlineTabs::new(&mut tab, &options)
                        .label("Documents")
                        .id_salt("g-reading-tabs")
                        .show(ui);
                })
                .body(|ui| {
                    for (i, heading) in ["Your licence", "What you may not do", "Updates"]
                        .into_iter()
                        .enumerate()
                    {
                        if i > 0 {
                            ui.add_space(theme::MD);
                        }
                        ui.label(
                            RichText::new(heading)
                                .font(theme::heading(m.headline_size))
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(
                                "A paragraph of the document at the reading measure, long enough \
                                 to wrap over several lines so the body scrolls under the lane.",
                            )
                            .size(m.subtitle_size)
                            .color(p.text),
                        );
                    }
                })
                .custom_footer(|ui| {
                    Button::new("Back")
                        .ghost()
                        .size(Size::Md)
                        .icon(icon::ARROW_LEFT)
                        .id_salt("g-reading-back")
                        .show(ui);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        Button::new("I agree")
                            .primary()
                            .size(Size::Md)
                            .id_salt("g-reading-agree")
                            .show(ui);
                        Button::new("Quit")
                            .secondary()
                            .size(Size::Md)
                            .id_salt("g-reading-quit")
                            .show(ui);
                    });
                    DialogResult::Open
                })
                .show_inline(ui, ForceState::default());
        },
    );
    ui.data_mut(|d| d.insert_temp(tab_key, tab));
    ui.advance_cursor_after_rect(area);
}

/// Paints `add` as an entrance at linear time `t` would show it (the gallery's motion rows).
fn entering(ui: &mut Ui, t: f32, from: f32, offset: Vec2, add: impl FnOnce(&mut Ui)) {
    let e = Entrance::at(t, t);
    let layer = ui.layer_id();
    let start = next_shape(ui.ctx(), layer);
    let r = ui.scope(|ui| {
        ui.multiply_opacity(e.fade);
        add(ui);
    });
    let origin = r.response.rect.center();
    transform_since(ui.ctx(), layer, start, e.transform(origin, from, offset));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        let ctx = Context::default();
        crate::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn run(
        ctx: &Context,
        events: Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 640.0))),
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out
    }

    fn key(k: Key) -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        }]
    }

    #[test]
    fn chords_split_into_keys() {
        assert_eq!(split_chord("Ctrl+Shift+F"), ["Ctrl", "Shift", "F"]);
        assert_eq!(split_chord("f"), ["f"]);
        assert_eq!(split_chord("Ctrl++"), ["Ctrl", "+"]);
        assert_eq!(split_chord("+"), ["+"]);
        assert_eq!(chord_keys("f"), ["f"]);
    }

    #[test]
    fn chords_with_symbol_keys_split_into_keys() {
        // Zoom out's label is "Ctrl+−" (a minus sign), and arrows are "↓" and "↑": keys that take
        // more than one byte.
        assert_eq!(split_chord("Ctrl+−"), ["Ctrl", "−"]);
        assert_eq!(split_chord("↓"), ["↓"]);
        assert_eq!(split_chord("↑+Ctrl"), ["↑", "Ctrl"]);
        assert_eq!(chord_keys("Ctrl+−").last().map(String::as_str), Some("−"));
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "the ring width is the exact token value, not a computed one"
    )]
    fn floating_surfaces_have_shadow_fill_and_ring() {
        let ctx = context();
        let out = run(&ctx, Vec::new(), |ui| {
            floating(ui, Layer::Menu, Margin::same(4), |ui| {
                ui.label("Item");
            });
        });
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let mut flat = Vec::new();
        for c in &out.shapes {
            if let Shape::Vec(v) = &c.shape {
                flat.extend(v.iter().cloned());
            }
        }
        let rects: Vec<&RectShape> = flat
            .iter()
            .filter_map(|s| match s {
                Shape::Rect(r) => Some(r),
                _ => None,
            })
            .collect();
        assert!(rects.iter().any(|r| r.blur_width > 0.0), "a soft shadow");
        assert!(
            rects.iter().any(|r| r.fill == s.overlay),
            "the opaque overlay"
        );
        assert!(
            rects
                .iter()
                .any(|r| r.stroke.color == p.floating_ring(s) && r.stroke.width == 1.0),
            "a 1 pt ring"
        );
        assert!(
            rects.iter().all(
                |r| r.corner_radius == CornerRadius::same(theme::metrics().radius_lg)
                    || r.blur_width > 0.0
            ),
            "radius_lg"
        );
    }

    #[test]
    fn exit_snapshots_drop_the_glass_backdrop() {
        let callback = || {
            Shape::Callback(egui::PaintCallback {
                rect: Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)),
                callback: Arc::new(()),
            })
        };
        let fill = Shape::rect_filled(
            Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)),
            0.0,
            Color32::RED,
        );
        let out = without_callbacks(Shape::Vec(vec![
            callback(),
            fill.clone(),
            Shape::Vec(vec![callback()]),
        ]));
        let Shape::Vec(out) = out else {
            panic!("a list stays a list");
        };
        assert!(matches!(out[0], Shape::Noop));
        assert!(matches!(out[1], Shape::Rect(_)));
        assert!(matches!(&out[2], Shape::Vec(v) if matches!(v[0], Shape::Noop)));
        assert!(matches!(without_callbacks(callback()), Shape::Noop));
        assert!(matches!(without_callbacks(fill), Shape::Rect(_)));
    }

    #[test]
    fn the_glass_hook_is_opaque_without_the_blur() {
        // No OpenGL context in tests, so the blur is unavailable whatever the setting says.
        let ctx = context();
        let _ = run(&ctx, Vec::new(), |ui| {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::splat(50.0));
            assert!(backdrop(ui.painter(), rect, CornerRadius::ZERO).is_none());
        });
    }

    #[test]
    fn entrances_are_instant_with_reduced_motion() {
        let ctx = context();
        let _ = run(&ctx, Vec::new(), |ui| {
            let e = Entrance::of(
                ui.ctx(),
                Id::new("x"),
                motion::BASE,
                motion::SLOW,
                false,
                None,
            );
            assert_eq!(e, Entrance::DONE);
            assert_eq!(
                e.transform(pos2(10.0, 10.0), 0.9, vec2(0.0, 4.0)),
                TSTransform::IDENTITY
            );
        });
        // Nothing is recorded for an exit either.
        let _ = run(&ctx, Vec::new(), |ui| {
            let ctx = ui.ctx();
            record_exit(
                ctx,
                Id::new("x"),
                LayerId::background(),
                0..0,
                0..0,
                ExitMotion::popup(Pos2::ZERO),
            );
            let exits = ctx
                .data(|d| d.get_temp::<Exits>(exits_id()))
                .unwrap_or_default();
            assert!(exits.open.is_empty());
        });
    }

    #[test]
    fn entrances_start_small_and_land_on_identity() {
        let a = Entrance::at(0.0, 0.0);
        assert_eq!(a.fade, 0.0);
        let tf = a.transform(Pos2::ZERO, POPUP_SCALE, vec2(0.0, -POPUP_DROP));
        assert!((tf.scaling - POPUP_SCALE).abs() < 1e-6);
        assert_eq!(tf.translation, vec2(0.0, -POPUP_DROP));
        let done = Entrance::at(1.0, 1.0);
        assert_eq!(
            done.transform(pos2(5.0, 5.0), 0.5, vec2(3.0, 3.0)),
            TSTransform::IDENTITY
        );
    }

    #[test]
    fn closed_layers_fade_out_from_their_last_frame() {
        let ctx = context();
        motion::set_reduced(false);
        let key = Id::new("menu");
        let mut layer = LayerId::background();
        for frame in 0..3 {
            let open = frame < 2;
            let out = run(&ctx, Vec::new(), |root| {
                let ctx = root.ctx().clone();
                if open {
                    egui::Area::new(Id::new("area")).show(&ctx, |ui| {
                        let ctx = ui.ctx().clone();
                        let ctx = &ctx;
                        layer = ui.layer_id();
                        let start = next_shape(ctx, layer);
                        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 60.0));
                        ui.painter()
                            .extend(surface_shapes(ui.painter(), rect, Layer::Menu));
                        let end = next_shape(ctx, layer);
                        record_exit(
                            ctx,
                            key,
                            layer,
                            0..0,
                            start.0..end.0,
                            ExitMotion::popup(pos2(10.0, 10.0)),
                        );
                    });
                }
            });
            if frame == 2 {
                // Nothing shows the menu any more, yet its surface is still painted, fading.
                assert!(
                    out.shapes
                        .iter()
                        .any(|c| matches!(&c.shape, Shape::Rect(r) if r.blur_width > 0.0)),
                    "the closed menu is still painted while it fades"
                );
            }
        }
        motion::set_reduced(true);
    }

    #[test]
    fn escape_from_a_dialog_returns_focus_to_its_keyboard_trigger() {
        let ctx = context();
        let mut open = false;
        let mut frame = |events| {
            let mut trigger = Id::NULL;
            let _ = run(&ctx, events, |ui| {
                let button = Button::new("Stash").show(ui);
                trigger = button.id;
                if button.clicked() {
                    open = true;
                }
                if open
                    && Dialog::new("stash-focus-test")
                        .title("Stash changes")
                        .initial_focus(InitialFocus::Body)
                        .body(|ui| {
                            ui.text_edit_singleline(&mut String::new());
                        })
                        .footer(|f| {
                            f.cancel().confirm("Stash");
                        })
                        .show(ui.ctx())
                        != DialogResult::Open
                {
                    open = false;
                }
            });
            trigger
        };
        frame(Vec::new());
        let trigger = frame(key(Key::Tab));
        assert_eq!(ctx.memory(egui::Memory::focused), Some(trigger));
        frame(key(Key::Enter));
        frame(Vec::new());
        frame(key(Key::Escape));
        frame(Vec::new());
        frame(Vec::new());
        assert_eq!(ctx.memory(egui::Memory::focused), Some(trigger));
        // The plugin must rearm for every dialog closure, not just the first.
        frame(key(Key::Enter));
        frame(Vec::new());
        frame(key(Key::Escape));
        frame(Vec::new());
        frame(Vec::new());
        assert_eq!(ctx.memory(egui::Memory::focused), Some(trigger));
    }

    #[test]
    fn destructive_dialog_focuses_cancel_and_ignores_enter() {
        let ctx = context();
        let show = |ctx: &Context| {
            Dialog::new("delete")
                .title("Delete branch 'x'?")
                .destructive()
                .footer(|f| {
                    f.cancel().confirm("Delete branch");
                })
                .show(ctx)
        };
        let mut result = DialogResult::Open;
        for _ in 0..3 {
            let _ = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
        }
        assert_eq!(result, DialogResult::Open);
        let focused = ctx.memory(egui::Memory::focused);
        let _ = run(&ctx, key(Key::Enter), |ui| result = show(ui.ctx()));
        // Enter clicked the focused Cancel button, never Delete.
        assert_ne!(result, DialogResult::Confirm, "focus {focused:?}");
        assert_eq!(result, DialogResult::Cancel);
    }

    /// usersim F13: the button Enter will press (Cancel in a destructive dialog, the primary in
    /// any other) shows its focus ring when the dialog opens, also after a pointer click opened it.
    #[test]
    fn a_dialog_shows_which_button_enter_will_press() {
        let ring = |out: &egui::FullOutput, color: Color32| {
            fn has(shape: &Shape, c: Color32) -> bool {
                match shape {
                    Shape::Vec(v) => v.iter().any(|s| has(s, c)),
                    Shape::Rect(r) => {
                        r.stroke.width > 0.0 && r.stroke.color.to_opaque() == c.to_opaque()
                    }
                    _ => false,
                }
            }
            out.shapes.iter().any(|c| has(&c.shape, color))
        };
        for destructive in [true, false] {
            let ctx = context();
            let show = |ctx: &Context| {
                let d = Dialog::new("ask").title("Sure?").footer(|f| {
                    f.cancel().confirm("Go");
                });
                if destructive { d.destructive() } else { d }.show(ctx)
            };
            // The pointer opened it: no key has been pressed.
            let at = pos2(5.0, 5.0);
            let press = egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            };
            let _ = run(&ctx, vec![egui::Event::PointerMoved(at), press], |_| {});
            let mut out = run(&ctx, Vec::new(), |ui| {
                show(ui.ctx());
            });
            for _ in 0..4 {
                out = run(&ctx, Vec::new(), |ui| {
                    show(ui.ctx());
                });
            }
            let focus = theme::palette(&ctx).focus;
            assert!(
                ring(&out, focus),
                "destructive {destructive}: no focus ring on the default button"
            );
        }
    }

    #[test]
    fn enter_confirms_and_esc_cancels_a_normal_dialog() {
        for (k, expect) in [
            (Key::Escape, DialogResult::Cancel),
            (Key::Enter, DialogResult::Confirm),
        ] {
            let ctx = context();
            let show = |ctx: &Context| {
                Dialog::new("rename")
                    .title("Rename branch")
                    .footer(|f| {
                        f.cancel().confirm("Rename");
                    })
                    .show(ctx)
            };
            let mut result = DialogResult::Open;
            for _ in 0..3 {
                let _ = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
            }
            assert_eq!(result, DialogResult::Open);
            let _ = run(&ctx, key(k), |ui| result = show(ui.ctx()));
            assert_eq!(result, expect, "{k:?}");
        }
    }

    #[test]
    fn a_neutral_choice_ignores_enter_and_a_blocking_dialog_ignores_esc() {
        let ctx = context();
        let show = |ctx: &Context| {
            Dialog::new("consent")
                .title("Send crash reports?")
                .blocking()
                .footer(|f| {
                    f.choice("Send crash reports", "Don't send");
                })
                .show(ctx)
        };
        let mut result = DialogResult::Open;
        for _ in 0..3 {
            let _ = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
        }
        assert_eq!(
            ctx.memory(egui::Memory::focused),
            None,
            "neither choice is focused"
        );
        for k in [Key::Enter, Key::Escape] {
            let _ = run(&ctx, key(k), |ui| result = show(ui.ctx()));
            assert_eq!(result, DialogResult::Open, "{k:?}");
        }
        // Both choices are drawn alike: two Secondary buttons, no primary fill.
        let out = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
        let p = theme::palette(&ctx);
        let primary = p.surfaces(theme::style()).primary.ends()[0];
        let fills_primary = out.shapes.iter().any(|c| {
            fn has(shape: &Shape, c: Color32) -> bool {
                match shape {
                    Shape::Vec(v) => v.iter().any(|s| has(s, c)),
                    Shape::Rect(r) => r.fill == c,
                    _ => false,
                }
            }
            has(&c.shape, primary)
        });
        assert!(!fills_primary, "no choice is drawn as the primary button");
    }

    #[test]
    fn a_disabled_confirm_ignores_enter() {
        let ctx = context();
        let show = |ctx: &Context| {
            Dialog::new("form")
                .title("Create tag")
                .initial_focus(InitialFocus::Body)
                .footer(|f| {
                    f.cancel()
                        .confirm("Create tag")
                        .confirm_disabled("Name the tag first");
                })
                .show(ctx)
        };
        let mut result = DialogResult::Open;
        for _ in 0..3 {
            let _ = run(&ctx, Vec::new(), |ui| result = show(ui.ctx()));
        }
        let _ = run(&ctx, key(Key::Enter), |ui| result = show(ui.ctx()));
        assert_eq!(result, DialogResult::Open);
    }

    #[test]
    fn dialogs_report_their_role_and_title() {
        dialog_owns_its_controls_and_is_modal();
        dialog_reports_role_and_title();
    }

    /// REVIEW-KIT-1: Enter on a focused body control (the credential prompt's "Show" eye, a
    /// Browse button) answers that control only; it does not also confirm the dialog.
    #[test]
    fn enter_on_a_body_control_does_not_confirm() {
        let ctx = context();
        let mut browsed = 0;
        let mut show = |ui: &mut Ui| {
            Dialog::new("clone")
                .title("Clone a repository")
                .initial_focus(InitialFocus::Body)
                .body(|ui| {
                    if Button::new("Browse").id_salt("browse").show(ui).clicked() {
                        browsed += 1;
                    }
                })
                .footer(|f| {
                    f.cancel().confirm("Clone");
                })
                .show(ui.ctx())
        };
        let mut result = DialogResult::Open;
        for _ in 0..3 {
            let _ = run(&ctx, Vec::new(), |ui| result = show(ui));
        }
        // Tab to the body's button (focus starts nowhere: the body asked for it).
        let _ = run(&ctx, key(Key::Tab), |ui| result = show(ui));
        let _ = run(&ctx, Vec::new(), |ui| result = show(ui));
        let _ = run(&ctx, key(Key::Enter), |ui| result = show(ui));
        assert_eq!(
            result,
            DialogResult::Open,
            "Enter on Browse confirmed the dialog"
        );
        assert_eq!(browsed, 1, "Enter pressed Browse");
    }

    /// A11Y-7: the dialog node owns its controls (they are its children, not the window's) and
    /// is modal, so a screen reader keeps to it.
    fn dialog_owns_its_controls_and_is_modal() {
        let ctx = context();
        ctx.enable_accesskit();
        let mut out = None;
        for _ in 0..2 {
            out = Some(run(&ctx, Vec::new(), |ui| {
                let _ = Dialog::new("modal")
                    .title("Delete tag 'v1'?")
                    .destructive()
                    .footer(|f| {
                        f.cancel().confirm("Delete tag");
                    })
                    .show(ui.ctx());
            }));
        }
        let nodes = out
            .and_then(|o| o.platform_output.accesskit_update)
            .expect("accesskit")
            .nodes;
        let (_, dialog) = nodes
            .iter()
            .find(|(_, n)| n.role() == egui::accesskit::Role::AlertDialog)
            .expect("the dialog");
        assert!(dialog.is_modal(), "the dialog is modal");
        // Its controls are below it in the tree, however deep.
        let mut below = Vec::new();
        let mut stack: Vec<_> = dialog.children().to_vec();
        while let Some(id) = stack.pop() {
            if let Some((_, n)) = nodes.iter().find(|(i, _)| *i == id) {
                below.extend(n.label().map(str::to_owned));
                stack.extend(n.children().iter().copied());
            }
        }
        for control in ["Cancel", "Delete tag"] {
            assert!(below.iter().any(|l| l == control), "{control} in {below:?}");
        }
    }

    fn dialog_reports_role_and_title() {
        let ctx = context();
        ctx.enable_accesskit();
        let mut out = None;
        for _ in 0..2 {
            out = Some(run(&ctx, Vec::new(), |ui| {
                let _ = Dialog::new("delete")
                    .title("Delete branch 'x'?")
                    .destructive()
                    .footer(|f| {
                        f.cancel().confirm("Delete branch");
                    })
                    .show(ui.ctx());
            }));
        }
        let nodes = out
            .and_then(|o| o.platform_output.accesskit_update)
            .expect("accesskit")
            .nodes;
        assert!(
            nodes
                .iter()
                .any(|(_, n)| n.role() == egui::accesskit::Role::AlertDialog
                    && n.label() == Some("Delete branch 'x'?")),
            "the dialog node"
        );
        assert!(
            nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Delete branch"))
        );
    }

    #[test]
    fn popovers_open_from_their_anchor_and_close_on_escape() {
        let ctx = context();
        let mut shown = false;
        let mut anchor_rect = Rect::NOTHING;
        let frame = |events: Vec<egui::Event>, shown: &mut bool, anchor_rect: &mut Rect| {
            let _ = run(&ctx, events, |ui| {
                let r = Button::new("Options").id_salt("anchor").show(ui);
                *anchor_rect = r.rect;
                *shown = Popover::new("opts", &r)
                    .show(|ui| {
                        ui.label("Inside");
                    })
                    .is_some();
            });
        };
        frame(Vec::new(), &mut shown, &mut anchor_rect);
        assert!(!shown);
        let c = anchor_rect.center();
        frame(
            vec![
                egui::Event::PointerMoved(c),
                egui::Event::PointerButton {
                    pos: c,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ],
            &mut shown,
            &mut anchor_rect,
        );
        frame(
            vec![egui::Event::PointerButton {
                pos: c,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
            &mut shown,
            &mut anchor_rect,
        );
        frame(Vec::new(), &mut shown, &mut anchor_rect);
        assert!(shown, "a click on the anchor opens it");
        frame(key(Key::Escape), &mut shown, &mut anchor_rect);
        frame(Vec::new(), &mut shown, &mut anchor_rect);
        assert!(!shown, "Esc closes it");
    }

    #[test]
    fn forced_popovers_stay_open() {
        let ctx = context();
        let mut shown = false;
        for _ in 0..2 {
            let _ = run(&ctx, Vec::new(), |ui| {
                let r = Button::new("Options").show(ui);
                shown = Popover::new("opts", &r)
                    .force(ForceState::OPEN)
                    .show(|ui| {
                        ui.label("Inside");
                    })
                    .is_some();
            });
        }
        assert!(shown);
    }

    #[test]
    fn a_scrolling_body_fades_where_it_hides_more_and_ends_clear_of_the_edge() {
        let ctx = context();
        let ground = ground(&ctx, Layer::Dialog);
        assert_eq!(
            ground,
            theme::surfaces(&ctx).overlay,
            "opaque without glass"
        );
        let mut sizes = Vec::new();
        let mut faded = false;
        for _ in 0..3 {
            let out = run(&ctx, Vec::new(), |ui| {
                let o = scroll_body(ui, "body", 100.0, ground, |ui| {
                    for i in 0..40 {
                        ui.label(format!("line {i}"));
                    }
                });
                sizes.push((o.content_size.y, o.inner_rect.height()));
            });
            let edge = ground.gamma_multiply(0.85);
            faded = out.shapes.iter().any(|c| {
                matches!(&c.shape, Shape::Mesh(m) if m.vertices.iter().any(|v| v.color == edge))
            });
        }
        assert!(faded, "the bottom edge fades into the ground");
        assert_eq!(sizes[2].1, 100.0, "a scrolling body takes all its room");
        // The room, plus the item gap egui puts before any space.
        let gap = ctx.global_style().spacing.item_spacing.y;
        assert_eq!(
            sizes[2].0 - sizes[0].0,
            SCROLL_END_ROOM + gap,
            "room after the last line once it overflows: {sizes:?}"
        );
        // A body that fits keeps its own height and fades nowhere.
        let mut fit = Vec::new();
        for _ in 0..3 {
            let out = run(&ctx, Vec::new(), |ui| {
                let o = scroll_body(ui, "short", 100.0, ground, |ui| {
                    ui.label("one line");
                });
                fit.push(o.content_size.y);
            });
            assert!(!out.shapes.iter().any(|c| matches!(c.shape, Shape::Mesh(_))));
        }
        assert_eq!(fit[0], fit[2]);
    }

    #[test]
    fn a_tall_dialog_keeps_clear_of_the_tab_strip_and_the_status_bar() {
        let ctx = context();
        let m = theme::metrics();
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(1024.0, 520.0));
        let mut card = Rect::NOTHING;
        for frame in 0..4 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(f64::from(frame)),
                    ..Default::default()
                },
                |ui| {
                    let _ = Dialog::new("tall")
                        .title("A long list")
                        .body(|ui| {
                            for i in 0..80 {
                                ui.label(format!("line {i}"));
                            }
                        })
                        .footer(|f| {
                            f.cancel().confirm("Done");
                        })
                        .show(ui.ctx());
                },
            );
            out.textures_delta.clear();
            for c in &out.shapes {
                if let Shape::Vec(v) = &c.shape {
                    for s in v {
                        if let Shape::Rect(r) = s
                            && r.stroke.width > 0.0
                            && r.corner_radius == CornerRadius::same(m.radius_lg)
                        {
                            card = r.rect;
                        }
                    }
                }
            }
        }
        assert!(card.is_positive(), "the card was painted");
        assert!(card.top() >= m.tab_strip_height, "{card:?}");
        assert!(
            card.bottom() <= screen.bottom() - m.status_bar_height,
            "{card:?}"
        );
    }

    /// Override O3: a reading dialog is the reading measure plus its padding wide, the measure
    /// holds about `READING_CHARS` characters of prose, and a narrow window keeps the card
    /// inside the modal room.
    #[test]
    fn a_reading_dialog_sets_its_text_at_the_measure_within_the_modal_room() {
        let ctx = context();
        let m = theme::metrics();
        let card_at = |screen: Rect| {
            let mut card = Rect::NOTHING;
            for frame in 0..4 {
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(screen),
                        time: Some(f64::from(frame)),
                        ..Default::default()
                    },
                    |ui| {
                        let _ = Dialog::new("reading")
                            .title("Privacy Policy")
                            .width(DialogWidth::Reading)
                            .body(|ui| {
                                ui.label("What leaves your machine, and when.");
                            })
                            .footer(|f| {
                                f.confirm("Close");
                            })
                            .show(ui.ctx());
                    },
                );
                out.textures_delta.clear();
                for c in &out.shapes {
                    if let Shape::Vec(v) = &c.shape {
                        for s in v {
                            if let Shape::Rect(r) = s
                                && r.stroke.width > 0.0
                                && r.corner_radius == CornerRadius::same(m.radius_lg)
                            {
                                card = r.rect;
                            }
                        }
                    }
                }
            }
            card
        };
        let wide = card_at(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0)));
        let measure = reading_measure(&ctx);
        assert!(
            (wide.width() - (measure + 2.0 * m.dialog_pad)).abs() < 1.0,
            "{wide:?} for a measure of {measure}"
        );
        // A different 70-character sentence fills the measure to within a tenth.
        let line = "Pictures are cached for twelve hours, and failures are remembered too.";
        assert_eq!(line.chars().count(), 70);
        let prose = ctx.fonts_mut(|f| {
            f.layout_no_wrap(
                line.to_owned(),
                FontId::proportional(m.subtitle_size),
                Color32::PLACEHOLDER,
            )
            .size()
            .x
        });
        assert!((prose / measure - 1.0).abs() < 0.1, "{prose} vs {measure}");
        let narrow = Rect::from_min_size(Pos2::ZERO, vec2(560.0, 600.0));
        let card = card_at(narrow);
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(narrow),
                time: Some(9.0),
                ..Default::default()
            },
            |ui| {
                let room = modal_room(ui.ctx());
                assert!(card.is_positive() && card.width() <= room.width() + 0.5);
                assert!(card.left() >= room.left() - 0.5 && card.right() <= room.right() + 0.5);
            },
        );
    }

    /// The card a dialog painted this frame: its ringed, `radius_lg` rectangle.
    fn painted_card(out: &egui::FullOutput) -> Rect {
        let m = theme::metrics();
        let mut card = Rect::NOTHING;
        for c in &out.shapes {
            if let Shape::Vec(v) = &c.shape {
                for s in v {
                    if let Shape::Rect(r) = s
                        && r.stroke.width > 0.0
                        && r.corner_radius == CornerRadius::same(m.radius_lg)
                    {
                        card = r.rect;
                    }
                }
            }
        }
        card
    }

    /// Where `text` was painted.
    fn painted_text(out: &egui::FullOutput, text: &str) -> Option<Rect> {
        fn find(shape: &Shape, text: &str) -> Option<Rect> {
            match shape {
                Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
                Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        out.shapes.iter().find_map(|c| find(&c.shape, text))
    }

    /// The legal reader's dialog (P-6): controls after the title in the title row, named for
    /// screen readers by `label` when the title is empty.
    #[test]
    fn header_controls_follow_the_title_in_its_row() {
        let ctx = context();
        ctx.enable_accesskit();
        let show = |ctx: &Context, title: &str| {
            Dialog::new("tabs")
                .title(title)
                .label("Privacy Policy")
                .icon(icon::SHIELD_CHECK)
                .closable()
                .header(|ui| {
                    ui.label("Terms");
                    ui.label("Privacy");
                })
                .footer(|f| {
                    f.confirm("Close");
                })
                .show(ctx)
        };
        let mut out = None;
        for _ in 0..3 {
            out = Some(run(&ctx, Vec::new(), |ui| {
                let _ = show(ui.ctx(), "Legal");
            }));
        }
        let out = out.expect("shown");
        let title = painted_text(&out, "Legal").expect("the title");
        let terms = painted_text(&out, "Terms").expect("the first header control");
        let privacy = painted_text(&out, "Privacy").expect("the second header control");
        assert!(title.right() < terms.left() && terms.right() < privacy.left());
        assert!(
            (terms.center().y - title.center().y).abs() < 2.0,
            "one row: {title:?} {terms:?}"
        );

        // No title: the header starts the row, and the label names the dialog.
        let mut out = None;
        for _ in 0..3 {
            out = Some(run(&ctx, Vec::new(), |ui| {
                let _ = show(ui.ctx(), "");
            }));
        }
        let out = out.expect("shown");
        let card = painted_card(&out);
        let terms = painted_text(&out, "Terms").expect("the header");
        assert!(
            terms.left() < card.left() + theme::metrics().dialog_pad + DISC + theme::XL,
            "right after the icon: {terms:?} in {card:?}"
        );
        let nodes = out
            .platform_output
            .accesskit_update
            .expect("accesskit")
            .nodes;
        assert!(
            nodes
                .iter()
                .any(|(_, n)| n.role() == egui::accesskit::Role::Dialog
                    && n.label() == Some("Privacy Policy")),
            "the dialog is named by its label"
        );
    }

    /// A document's dialog is as tall as a dialog may be even when its text is short, and its
    /// footer sits at the bottom of the card.
    #[test]
    fn a_full_height_dialog_takes_the_room_however_short_its_body() {
        let ctx = context();
        let mut out = None;
        for frame in 0..4 {
            let mut shown = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
                    time: Some(f64::from(frame)),
                    ..Default::default()
                },
                |ui| {
                    let _ = Dialog::new("short")
                        .title("Notes")
                        .width(DialogWidth::Reading)
                        .fill_height()
                        .body(|ui| {
                            ui.label("One line.");
                        })
                        .footer(|f| {
                            f.confirm("Close");
                        })
                        .show(ui.ctx());
                },
            );
            shown.textures_delta.clear();
            out = Some(shown);
        }
        let out = out.expect("shown");
        let card = painted_card(&out);
        let room = modal_room(&ctx);
        assert!(
            card.height() >= room.height().min(theme::SETTINGS_MAX_HEIGHT) - 2.0 * DIALOG_MARGIN,
            "{card:?} in {room:?}"
        );
        let close = painted_text(&out, "Close").expect("the footer");
        assert!(
            card.bottom() - close.bottom() < theme::metrics().dialog_pad + theme::XL,
            "the footer is at the bottom: {close:?} in {card:?}"
        );
    }

    /// The progress lane: a seam under the title row and, along it, an `accent` line as long as
    /// the share of the body scrolled past.
    #[test]
    fn the_progress_lane_follows_the_scroll() {
        let ctx = context();
        let accent = theme::palette(&ctx).accent;
        let mut time = 0.0;
        let mut frame = |events: Vec<egui::Event>| {
            time += 0.5;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 900.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let _ = Dialog::new("document")
                        .title("Terms")
                        .width(DialogWidth::Reading)
                        .fill_height()
                        .progress_lane()
                        .body(|ui| {
                            for i in 0..200 {
                                ui.label(format!("Paragraph {i}"));
                            }
                        })
                        .footer(|f| {
                            f.confirm("Close");
                        })
                        .show(ui.ctx());
                },
            );
            out.textures_delta.clear();
            let line = out.shapes.iter().find_map(|c| {
                fn find(shape: &Shape, accent: Color32) -> Option<Rect> {
                    match shape {
                        Shape::Rect(r) if r.fill == accent && r.rect.height() <= PROGRESS => {
                            Some(r.rect)
                        }
                        Shape::Vec(v) => v.iter().find_map(|s| find(s, accent)),
                        _ => None,
                    }
                }
                find(&c.shape, accent)
            });
            (painted_card(&out), line)
        };
        for _ in 0..3 {
            frame(Vec::new());
        }
        let (card, line) = frame(Vec::new());
        assert!(
            line.is_none_or(|l| l.width() < 1.0),
            "nothing read yet: {line:?}"
        );
        let over = card.center();
        for _ in 0..6 {
            frame(vec![
                egui::Event::PointerMoved(over),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, -100_000.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
        }
        let (card, line) = frame(Vec::new());
        let line = line.expect("the progress line once scrolled");
        assert!(
            (line.left() - card.left()).abs() < 1.0 && (line.right() - card.right()).abs() < 1.0,
            "read to the end, the line spans the card: {line:?} {card:?}"
        );
    }

    /// A custom footer answers the dialog with what it returns, and a blocking dialog leaves
    /// Esc to it.
    #[test]
    fn a_custom_footer_answers_the_dialog() {
        let ctx = context();
        let show = |ctx: &Context, back: &mut bool| {
            Dialog::new("consent")
                .title("Terms")
                .blocking()
                .custom_footer(|ui| {
                    *back = ui.button("Back").clicked() || ui.input(|i| i.key_pressed(Key::Escape));
                    if *back {
                        DialogResult::Cancel
                    } else {
                        DialogResult::Open
                    }
                })
                .show(ctx)
        };
        let mut back = false;
        let mut result = DialogResult::Open;
        for _ in 0..3 {
            let _ = run(&ctx, Vec::new(), |ui| result = show(ui.ctx(), &mut back));
        }
        assert_eq!(result, DialogResult::Open);
        assert_eq!(
            ctx.memory(egui::Memory::focused),
            None,
            "nothing is focused"
        );
        let _ = run(&ctx, key(Key::Enter), |ui| {
            result = show(ui.ctx(), &mut back);
        });
        assert_eq!(result, DialogResult::Open, "Enter answers nothing");
        let _ = run(&ctx, key(Key::Escape), |ui| {
            result = show(ui.ctx(), &mut back);
        });
        assert!(back);
        assert_eq!(result, DialogResult::Cancel);
    }

    #[test]
    fn light_themes_get_the_dark_opaque_tooltip() {
        for id in theme::ThemeId::ALL
            .into_iter()
            .filter(|t| !t.palette().dark)
        {
            let ctx = Context::default();
            crate::fonts::install(&ctx);
            theme::apply(&ctx, id, theme::Appearance::STILL);
            let s = theme::surfaces(&ctx);
            let p = theme::palette(&ctx);
            assert!(!p.dark);
            assert_eq!(s.tooltip.top.a(), 255, "opaque");
            assert!(
                theme::contrast(s.tooltip.top, Color32::BLACK)
                    < theme::contrast(s.tooltip.top, Color32::WHITE),
                "dark on a light theme"
            );
            assert!(
                theme::contrast(p.on_tooltip, s.tooltip.top) >= 4.5,
                "readable"
            );
            let out = run(&ctx, Vec::new(), |ui| {
                tooltip_body(ui, "Fetch from all remotes", None, None);
            });
            let fills: Vec<Color32> = out
                .shapes
                .iter()
                .flat_map(|c| match &c.shape {
                    Shape::Vec(v) => v.clone(),
                    other => vec![other.clone()],
                })
                .filter_map(|sh| match sh {
                    Shape::Rect(r) if r.fill != Color32::TRANSPARENT => Some(r.fill),
                    _ => None,
                })
                .collect();
            assert!(fills.contains(&s.tooltip.top), "{id:?} {fills:?}");
        }
    }

    #[test]
    fn a_choice_footer_takes_focus_in_reading_order() {
        let ctx = context();
        let mut texts: Vec<(String, Rect)> = Vec::new();
        let mut focused = Vec::new();
        for frame in 0..6 {
            let events = if frame >= 3 {
                key(Key::Tab)
            } else {
                Vec::new()
            };
            let out = run(&ctx, events, |ui| {
                let _ = Dialog::new("consent")
                    .title("Send crash reports?")
                    .footer(|f| {
                        f.choice("Send crash reports", "Don't send");
                    })
                    .show(ui.ctx());
            });
            texts = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Text(t) => Some((t.galley.text().to_owned(), t.visual_bounding_rect())),
                    _ => None,
                })
                .collect();
            if frame >= 3
                && let Some(id) = ctx.memory(egui::Memory::focused)
                && let Some(r) = ctx.read_response(id)
            {
                focused.push(r.rect);
            }
        }
        let at = |label: &str| {
            texts
                .iter()
                .find(|(t, _)| t == label)
                .map(|(_, r)| r.center())
                .expect(label)
        };
        let (send, dont) = (at("Send crash reports"), at("Don't send"));
        assert!(dont.x < send.x, "Don't send is on the left");
        assert!(focused.len() >= 2, "{focused:?}");
        assert!(
            focused[0].contains(dont),
            "Tab reaches the left answer first: {focused:?}"
        );
        assert!(focused[1].contains(send), "then the right one: {focused:?}");
    }

    #[test]
    fn a_pointer_anchor_is_the_rows_column_under_the_pointer() {
        let row = Rect::from_min_size(pos2(0.0, 100.0), vec2(1200.0, 28.0));
        let a = pointer_anchor(row, Some(pos2(1000.0, 110.0)));
        assert_eq!(a.x_range(), Rangef::point(1000.0));
        assert_eq!(a.y_range(), row.y_range(), "a row is cleared whole");
        let tall = Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 600.0));
        let a = pointer_anchor(tall, Some(pos2(50.0, 300.0)));
        assert_eq!(
            a.y_range(),
            Rangef::new(300.0 - POINTER_REACH, 300.0 + POINTER_REACH)
        );
        assert_eq!(pointer_anchor(row, None), row);
        let outside = pointer_anchor(row, Some(pos2(1300.0, 110.0)));
        assert_eq!(
            outside.x_range(),
            Rangef::point(row.right()),
            "kept on the row"
        );
    }

    /// Where the tooltip text `text` was painted, once the pointer has rested at `pointer` over a
    /// 1000 pt wide row for longer than the tooltip delay.
    fn tooltip_over_wide_row(at_pointer: bool, pointer: Pos2) -> (Rect, Option<Rect>) {
        let ctx = context();
        let text = "Fix the parser";
        let mut row = Rect::NOTHING;
        let mut found = None;
        for i in 0..10 {
            let events = if i == 1 {
                vec![egui::Event::PointerMoved(pointer)]
            } else {
                Vec::new()
            };
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 640.0))),
                    time: Some(f64::from(i) * 0.2),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.add_space(200.0);
                    let r = ui.allocate_response(vec2(1000.0, 28.0), Sense::hover());
                    row = r.rect;
                    let t = r.kit_tooltip(text);
                    let _ = if at_pointer { t.at_pointer() } else { t }.show();
                },
            );
            out.textures_delta.clear();
            found = out.shapes.iter().find_map(|c| match &c.shape {
                Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
                _ => None,
            });
        }
        (row, found)
    }

    /// Whether `tip` is painted after the pointer rests on what `add` shows, at `level`.
    fn tooltip_shown(
        level: TooltipLevel,
        tip: &str,
        mut add: impl FnMut(&mut Ui) -> Response,
    ) -> bool {
        let ctx = context();
        set_tooltip_level(level);
        let mut at = Pos2::ZERO;
        let mut found = false;
        for i in 0..12 {
            let events = if i == 1 {
                vec![egui::Event::PointerMoved(at)]
            } else {
                Vec::new()
            };
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0))),
                    time: Some(f64::from(i) * 0.2),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.add_space(200.0);
                    at = add(ui).rect.center();
                },
            );
            out.textures_delta.clear();
            found = out
                .shapes
                .iter()
                .any(|c| matches!(&c.shape, Shape::Text(t) if t.galley.text() == tip));
        }
        set_tooltip_level(TooltipLevel::Helpful);
        found
    }

    /// Helpful (the default) leaves out a tooltip that only describes a labelled control, keeps
    /// an icon's name and a shortcut, and All shows everything; Off keeps only why a control is
    /// unavailable.
    #[test]
    fn the_tooltip_level_decides_which_tooltips_show() {
        let described = |ui: &mut Ui| Button::new("Stash").tooltip("Stash changes").show(ui);
        let shortcut = |ui: &mut Ui| {
            Button::new("Fetch")
                .tooltip("Fetch all remotes")
                .shortcut("f")
                .show(ui)
        };
        let icon = |ui: &mut Ui| IconButton::new(icon::GEAR_SIX, "Settings").show(ui);
        let hint = |ui: &mut Ui| {
            let r = ui.allocate_response(vec2(120.0, 24.0), Sense::click());
            r.kit_tooltip("Double-click to check out")
                .extra(true)
                .show()
        };
        assert!(!tooltip_shown(
            TooltipLevel::Helpful,
            "Stash changes",
            described
        ));
        assert!(tooltip_shown(TooltipLevel::All, "Stash changes", described));
        assert!(tooltip_shown(
            TooltipLevel::Helpful,
            "Fetch all remotes",
            shortcut
        ));
        assert!(tooltip_shown(TooltipLevel::Helpful, "Settings", icon));
        assert!(!tooltip_shown(TooltipLevel::Off, "Settings", icon));
        assert!(!tooltip_shown(
            TooltipLevel::Helpful,
            "Double-click to check out",
            hint
        ));
        assert!(tooltip_shown(
            TooltipLevel::All,
            "Double-click to check out",
            hint
        ));
        // Why a control is unavailable shows at every level.
        for level in TooltipLevel::ALL {
            assert!(tooltip_shows(level, false, true), "{level:?}");
        }
    }

    /// views-1 F4: on a wide row the kit tooltip can sit by the pointer, above the row, instead
    /// of centred over the whole row far from the pointer.
    #[test]
    fn a_pointer_placed_tooltip_sits_by_the_pointer_above_the_row() {
        let pointer = pos2(900.0, 214.0);
        let (row, tip) = tooltip_over_wide_row(true, pointer);
        let tip = tip.expect("the tooltip shows");
        assert!(
            (tip.center().x - pointer.x).abs() < 120.0,
            "by the pointer: {tip:?}"
        );
        assert!(tip.bottom() <= row.top(), "above the row: {tip:?} {row:?}");
        let (row, tip) = tooltip_over_wide_row(false, pointer);
        let tip = tip.expect("the tooltip shows");
        assert!(
            (tip.center().x - row.center().x).abs() < 120.0,
            "without it, centred on the control: {tip:?}"
        );
    }

    /// A tooltip's shortcut keycaps sit on its first line, not above it.
    #[test]
    fn tooltip_keycaps_centre_on_the_first_line() {
        let ctx = context();
        let out = run(&ctx, Vec::new(), |ui| {
            tooltip_body(ui, "Fetch all remotes", None, Some("f".into()));
            tooltip_body(ui, "Push main", Some("2 commits ahead"), Some("f".into()));
        });
        let shapes: Vec<Shape> = out
            .shapes
            .iter()
            .flat_map(|c| match &c.shape {
                Shape::Vec(v) => v.clone(),
                other => vec![other.clone()],
            })
            .collect();
        // The first line's centre: the galley's first row, where it was placed.
        let line = |text: &str| {
            shapes
                .iter()
                .find_map(|s| match s {
                    Shape::Text(t) if t.galley.text() == text => {
                        let row = t
                            .galley
                            .rows
                            .first()
                            .map(egui::epaint::text::PlacedRow::rect)?;
                        Some(t.pos.y + row.center().y)
                    }
                    _ => None,
                })
                .expect(text)
        };
        let mut caps: Vec<f32> = shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Rect(r) if (r.rect.height() - TOOLTIP_KEYCAP).abs() < 0.01 => {
                    Some(r.rect.center().y)
                }
                _ => None,
            })
            .collect();
        // A keycap is a fill and a ring.
        caps.dedup();
        assert_eq!(caps.len(), 2, "{caps:?}");
        for (label, cap) in [("Fetch all remotes", caps[0]), ("Push main", caps[1])] {
            let d = (line(label) - cap).abs();
            assert!(d <= 1.0, "{label}: {d} pt off the keycap's centre");
        }
    }

    /// H15: in a light theme a tooltip over rows has a visible edge: its dark fill stands at least
    /// 3:1 from every row ground it can cover (the pane, the panel, zebra, hover and selection).
    #[test]
    fn light_theme_tooltips_stand_out_from_every_row() {
        for id in theme::ThemeId::ALL {
            let p = id.palette();
            if p.dark {
                continue;
            }
            for style in theme::Style::ALL {
                let s = p.surfaces(style);
                for base in [s.pane, s.panel, s.elevated] {
                    for wash in [
                        Color32::TRANSPARENT,
                        p.zebra(),
                        p.hover(),
                        p.hover_strong(),
                        p.selected(),
                    ] {
                        let ground = theme::over(wash, base);
                        let k = theme::contrast(s.tooltip.top, ground);
                        assert!(k >= 3.0, "{id:?} {style:?}: {k:.2}:1 on {ground:?}");
                    }
                }
            }
        }
    }
}
