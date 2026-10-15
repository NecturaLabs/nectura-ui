//! `SplitHandle`: the handle between two panes of a custom split (design spec 4.22), such as the
//! history table over the commit details or the resolver's columns.
//!
//! At rest the handle is the 1 pt `border` seam between the panes (or nothing, where the panes are
//! Deeper's cards over a gutter). Its hit zone is 8 pt wide around the seam. Hovered for 100 ms,
//! the seam turns `focus` at 150 and 2 pt wide over `FAST`; dragged or focused from the keyboard
//! it is `focus` at full strength. The pointer shows the resize cursor. A drag moves the split
//! with the pointer, a double click resets it to its default size, and with keyboard focus the
//! arrows move it 8 pt (Shift: 32 pt) and Home / End take it to its limits. Screen readers hear a
//! splitter with its name and its size in points.
//!
//! ```ignore
//! let r = SplitHandle::new("details", &mut height, SplitAxis::Rows)
//!     .after(true)
//!     .range(120.0..=520.0)
//!     .default_size(260.0)
//!     .label("Resize the commit details")
//!     .show(ui, seam_y, ui.max_rect().x_range());
//! ```

use std::ops::RangeInclusive;

use egui::accesskit::Role;
use egui::{
    Context, CursorIcon, Id, Key, Modifiers, Rangef, Rect, Response, Sense, Stroke, Ui, WidgetInfo,
    WidgetType, pos2, vec2,
};

use super::{ForceState, anim, focus_visible, paint};
use crate::motion;
use crate::theme::{self, Palette};

/// Width of the zone around the seam that takes the pointer.
pub const HIT: f32 = 8.0;
/// The seam's width while hovered, dragged or focused.
pub const ACTIVE_WIDTH: f32 = 2.0;
/// How strong the seam's `focus` tint is while hovered (dragging and focus use full strength).
pub const HOVER_ALPHA: u8 = 150;
/// How long the pointer rests on the handle before it lights up, so passing over it does not
/// flash.
pub const HOVER_DELAY: f32 = 0.1;
/// How far one arrow press moves the split, and one Shift+arrow press.
pub const KEY_STEP: f32 = 8.0;
pub const KEY_STEP_LARGE: f32 = 32.0;

/// Which way the split divides its area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    /// Panes side by side: a vertical seam, dragged left and right.
    Columns,
    /// Panes one above the other: a horizontal seam, dragged up and down.
    Rows,
}

/// The handle of a split: `SplitHandle::new(id, &mut size, axis).show(ui, seam, span)`.
#[must_use = "call .show(ui, seam, span)"]
pub struct SplitHandle<'a> {
    id_salt: Id,
    size: &'a mut f32,
    axis: SplitAxis,
    after: bool,
    range: RangeInclusive<f32>,
    default_size: Option<f32>,
    label: String,
    seam: bool,
    force: Option<ForceState>,
}

impl std::fmt::Debug for SplitHandle<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SplitHandle")
            .field("size", &*self.size)
            .field("axis", &self.axis)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<'a> SplitHandle<'a> {
    /// A handle resizing the pane whose size (width for columns, height for rows, in points) is
    /// `size`: by default the pane before the seam (left or above).
    pub fn new(
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        size: &'a mut f32,
        axis: SplitAxis,
    ) -> Self {
        Self {
            id_salt: Id::new(("kit-split", id_salt)),
            size,
            axis,
            after: false,
            range: 0.0..=f32::INFINITY,
            default_size: None,
            label: "Resize".to_owned(),
            seam: true,
            force: None,
        }
    }

    /// `size` is the pane after the seam (right or below): dragging towards it shrinks it.
    pub fn after(mut self, after: bool) -> Self {
        self.after = after;
        self
    }

    /// The smallest and largest the pane may be.
    pub fn range(mut self, range: RangeInclusive<f32>) -> Self {
        self.range = range;
        self
    }

    /// The size a double click restores.
    pub fn default_size(mut self, size: f32) -> Self {
        self.default_size = Some(size);
        self
    }

    /// What screen readers call the handle ("Resize the commit details").
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    /// Whether the 1 pt seam shows at rest (default on). Off where the panes are separated by
    /// something else, such as Deeper's gutter between two cards.
    pub fn seam(mut self, seam: bool) -> Self {
        self.seam = seam;
        self
    }

    /// The id [`Self::show`] gives the handle made with `id_salt` when it is shown in `ui`, to
    /// read it before it is shown ([`dragged_size`], `Context::read_response`).
    pub fn id_in(ui: &Ui, id_salt: impl std::hash::Hash + std::fmt::Debug) -> Id {
        ui.id().with(Id::new(("kit-split", id_salt)))
    }

    /// Forces hover, press (dragging) or focus (the gallery and tests).
    pub(crate) fn force(mut self, force: ForceState) -> Self {
        self.force = Some(force);
        self
    }

    /// Shows the handle on the seam at `at` (x for columns, y for rows) along `span` (the seam's
    /// extent across the split). Returns the handle's response, marked changed when the size moved.
    pub fn show(self, ui: &Ui, at: f32, span: Rangef) -> Response {
        let ctx = ui.ctx().clone();
        let id = ui.id().with(self.id_salt);
        let rect = match self.axis {
            SplitAxis::Columns => {
                Rect::from_x_y_ranges(Rangef::new(at - HIT / 2.0, at + HIT / 2.0), span)
            }
            SplitAxis::Rows => {
                Rect::from_x_y_ranges(span, Rangef::new(at - HIT / 2.0, at + HIT / 2.0))
            }
        };
        let mut response = ui.interact(rect, id, Sense::click_and_drag());
        let lo = *self.range.start();
        let hi = self.range.end().max(lo);
        let sign = if self.after { -1.0 } else { 1.0 };
        let along = |p: egui::Pos2| match self.axis {
            SplitAxis::Columns => p.x,
            SplitAxis::Rows => p.y,
        };

        let before = *self.size;
        let mut size = before;
        // A drag moves the split with the pointer from where it was taken.
        let grab = id.with("grab");
        // From where the button went down: a drag starts only once the pointer has moved a little.
        if response.drag_started()
            && let Some(p) = ctx
                .input(|i| i.pointer.press_origin())
                .or_else(|| response.interact_pointer_pos())
        {
            ctx.data_mut(|d| d.insert_temp(grab, (along(p), size)));
        }
        if response.dragged()
            && let (Some(p), Some((from, start))) = (
                response.interact_pointer_pos(),
                ctx.data(|d| d.get_temp::<(f32, f32)>(grab)),
            )
        {
            size = start + (along(p) - from) * sign;
        }
        if response.drag_stopped() {
            ctx.data_mut(|d| d.remove::<(f32, f32)>(grab));
        }
        if response.double_clicked()
            && let Some(d) = self.default_size
        {
            size = d;
        }
        // The pointer never takes keyboard focus: after a drag the arrows keep moving through the
        // lists. Tab reaches the handle, and then the arrows resize.
        if response.has_focus() {
            size = keys(ui, id, self.axis, size, sign, (lo, hi));
        }
        size = size.clamp(lo, hi);
        if (size - before).abs() > f32::EPSILON {
            *self.size = size;
            response.mark_changed();
        }

        if response.hovered() || response.dragged() {
            ctx.set_cursor_icon(match self.axis {
                SplitAxis::Columns => CursorIcon::ResizeHorizontal,
                SplitAxis::Rows => CursorIcon::ResizeVertical,
            });
        }

        let look = Look::of(&ctx, &response, id, self.force);
        if ui.is_rect_visible(rect) {
            paint_seam(ui, self.axis, at, span, look, self.seam);
        }

        let label = self.label;
        let spoken = format!("{} points", size.round());
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(WidgetType::Other, true, &label);
            info.current_text_value = Some(spoken.clone());
            info
        });
        ctx.accesskit_node_builder(response.id, |n| {
            n.set_role(Role::Splitter);
            n.set_orientation(match self.axis {
                SplitAxis::Columns => egui::accesskit::Orientation::Horizontal,
                SplitAxis::Rows => egui::accesskit::Orientation::Vertical,
            });
            n.set_numeric_value(f64::from(size));
            n.set_min_numeric_value(f64::from(lo));
            if hi.is_finite() {
                n.set_max_numeric_value(f64::from(hi));
            }
            n.set_numeric_value_step(f64::from(KEY_STEP));
        });
        response
    }
}

/// Where a drag of the handle `id` ([`SplitHandle::id_in`]) puts its pane this frame, read before
/// the panes are laid out so they follow the pointer instead of a frame behind it; `None` while it
/// is not being dragged. `after` and `range` are the handle's own.
pub fn dragged_size(
    ctx: &Context,
    id: Id,
    axis: SplitAxis,
    after: bool,
    range: RangeInclusive<f32>,
) -> Option<f32> {
    if !ctx.read_response(id)?.dragged() {
        return None;
    }
    let (from, start) = ctx.data(|d| d.get_temp::<(f32, f32)>(id.with("grab")))?;
    let p = ctx.input(|i| i.pointer.interact_pos())?;
    let along = match axis {
        SplitAxis::Columns => p.x,
        SplitAxis::Rows => p.y,
    };
    let sign = if after { -1.0 } else { 1.0 };
    let lo = *range.start();
    Some((start + (along - from) * sign).clamp(lo, range.end().max(lo)))
}

/// The arrows (Shift: a larger step), Home and End, while the handle has keyboard focus.
fn keys(ui: &Ui, id: Id, axis: SplitAxis, size: f32, sign: f32, (lo, hi): (f32, f32)) -> f32 {
    let (less, more) = match axis {
        SplitAxis::Columns => (Key::ArrowLeft, Key::ArrowRight),
        SplitAxis::Rows => (Key::ArrowUp, Key::ArrowDown),
    };
    super::toggle::claim_arrows(ui, id, axis == SplitAxis::Columns, axis == SplitAxis::Rows);
    ui.input_mut(|i| {
        let mut size = size;
        for (key, dir) in [(less, -1.0), (more, 1.0)] {
            if i.consume_key(Modifiers::SHIFT, key) {
                size += dir * sign * KEY_STEP_LARGE;
            } else if i.consume_key(Modifiers::NONE, key) {
                size += dir * sign * KEY_STEP;
            }
        }
        if i.consume_key(Modifiers::NONE, Key::Home) {
            size = lo;
        }
        if i.consume_key(Modifiers::NONE, Key::End) && hi.is_finite() {
            size = hi;
        }
        size
    })
}

/// What the seam paints from: the delayed hover (0 to 1) and whether it is held or focused.
#[derive(Debug, Clone, Copy)]
struct Look {
    hover: f32,
    active: f32,
}

impl Look {
    fn of(ctx: &Context, response: &Response, id: Id, force: Option<ForceState>) -> Self {
        if let Some(f) = force {
            let k = f.progress.unwrap_or(1.0);
            let on = |b: bool| f32::from(u8::from(b)) * k;
            return Self {
                hover: on(f.hover),
                active: on(f.pressed || f.focus),
            };
        }
        let since = id.with("hover-since");
        let now = ctx.input(|i| i.time);
        let hovered = response.hovered() || response.dragged();
        let rested = if hovered {
            let start = ctx.data_mut(|d| *d.get_temp_mut_or(since, now));
            let waited = crate::to_f32(now - start);
            if waited < HOVER_DELAY && !motion::reduced() {
                ctx.request_repaint_after(std::time::Duration::from_secs_f32(HOVER_DELAY - waited));
                false
            } else {
                true
            }
        } else {
            ctx.data_mut(|d| d.remove::<f64>(since));
            false
        };
        let active = response.dragged() || (response.has_focus() && focus_visible(ctx));
        Self {
            hover: anim::ease_bool(
                ctx,
                id.with("hover"),
                rested,
                motion::FAST,
                motion::FAST_OUT,
            ),
            active: anim::ease_bool(
                ctx,
                id.with("active"),
                active,
                motion::FAST,
                motion::FAST_OUT,
            ),
        }
    }
}

/// The seam: `border` at 1 pt at rest (if shown), growing to 2 pt in `focus` at 150 when hovered
/// and full `focus` when dragged or focused.
fn paint_seam(ui: &Ui, axis: SplitAxis, at: f32, span: Rangef, look: Look, seam: bool) {
    let p = theme::palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let painter = ui.painter();
    let lit = look.hover.max(look.active);
    if seam && lit < 1.0 {
        match axis {
            SplitAxis::Columns => paint::seam_v(painter, at, span, s.border),
            SplitAxis::Rows => paint::seam_h(painter, span, at, s.border),
        }
    }
    if lit <= 0.0 {
        return;
    }
    let hover = Palette::tint(p.focus, HOVER_ALPHA);
    let color = hover
        .lerp_to_gamma(p.focus, look.active)
        .gamma_multiply(lit);
    let width = paint::SEAM + (ACTIVE_WIDTH - paint::SEAM) * lit;
    let line = match axis {
        SplitAxis::Columns => {
            Rect::from_x_y_ranges(Rangef::new(at - width / 2.0, at + width / 2.0), span)
        }
        SplitAxis::Rows => {
            Rect::from_x_y_ranges(span, Rangef::new(at - width / 2.0, at + width / 2.0))
        }
    };
    painter.rect_filled(line, egui::CornerRadius::ZERO, color);
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's handles: a column split at rest, hovered, dragged and focused, one between two
/// Deeper-style cards with no seam at rest, and a live row split to drag.
pub(crate) fn gallery(ui: &mut Ui) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let states = [
        ("Rest", None, true),
        ("Hover", Some(ForceState::HOVER), true),
        ("Dragging", Some(ForceState::PRESSED), true),
        ("Focus", Some(ForceState::FOCUS), true),
        ("Between cards", Some(ForceState::HOVER), false),
    ];
    let count = crate::count_f32(states.len());
    let width = ui.available_width();
    let cell = (width - theme::SM * (count - 1.0)) / count;
    let height = 72.0;
    let (strip, _) = ui.allocate_exact_size(
        vec2(width, height + m.small_size + theme::SM),
        Sense::hover(),
    );
    let radius = egui::CornerRadius::same(m.radius_sm);
    for (i, (name, force, seam)) in states.into_iter().enumerate() {
        let x = strip.left() + crate::count_f32(i) * (cell + theme::SM);
        let area = Rect::from_min_size(pos2(x, strip.top()), vec2(cell, height));
        let mid = area.center().x;
        if seam {
            ui.painter().rect_filled(area, radius, s.pane);
            ui.painter().rect_stroke(
                area,
                radius,
                Stroke::new(paint::SEAM, s.border),
                egui::StrokeKind::Inside,
            );
        } else {
            // Two cards over the window's ground: the gutter itself separates them.
            ui.painter().rect_filled(area, radius, s.bg);
            let gap = m.pane_gutter.max(theme::XS) / 2.0;
            for card in [
                Rect::from_min_max(area.min, pos2(mid - gap, area.bottom())),
                Rect::from_min_max(pos2(mid + gap, area.top()), area.max),
            ] {
                ui.painter().rect_filled(card, radius, s.pane);
                ui.painter().rect_stroke(
                    card,
                    radius,
                    Stroke::new(paint::SEAM, s.pane_edge.unwrap_or(s.border)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        let mut size = cell / 2.0;
        let mut handle = SplitHandle::new(("gallery", i), &mut size, SplitAxis::Columns)
            .seam(seam)
            .label(format!("Gallery split, {name}"));
        if let Some(f) = force {
            handle = handle.force(f);
        }
        let _ = handle.show(ui, mid, area.y_range().shrink(1.0));
        ui.painter().text(
            pos2(mid, area.bottom() + theme::XS),
            egui::Align2::CENTER_TOP,
            name,
            egui::FontId::proportional(m.small_size),
            p.text_faint,
        );
    }
    ui.add_space(theme::SM);
    ui.label(
        egui::RichText::new(
            "Live: drag the seam, double-click it to reset, or focus it and use the arrows",
        )
        .size(m.small_size)
        .color(p.text_faint),
    );
    // The lower pane's height is the size, as the commit details below the history table.
    let key = Id::new("kit-split-gallery-live");
    let mut size: f32 = ui.data(|d| d.get_temp(key)).unwrap_or(84.0);
    let (area, _) = ui.allocate_exact_size(vec2(width, 140.0), Sense::hover());
    ui.painter().rect_filled(area, radius, s.panel);
    let at = area.bottom() - size;
    ui.painter().rect_filled(
        Rect::from_min_max(pos2(area.left(), at), area.max),
        egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: m.radius_sm,
            se: m.radius_sm,
        },
        s.pane,
    );
    ui.painter().rect_stroke(
        area,
        radius,
        Stroke::new(paint::SEAM, s.border),
        egui::StrokeKind::Inside,
    );
    let r = SplitHandle::new("gallery-live", &mut size, SplitAxis::Rows)
        .after(true)
        .range(24.0..=area.height() - 24.0)
        .default_size(84.0)
        .label("Gallery split, live")
        .show(ui, at, area.x_range());
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, size));
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, Pos2, RawInput};

    use super::*;
    use crate::kit::testkit;

    fn frame(ctx: &Context, time: f64, events: Vec<Event>, f: impl FnMut(&mut Ui)) {
        let mut f = f;
        let mut out = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
    }

    /// A column split whose seam sits at its size (a sidebar from x = 0).
    fn handle(ui: &Ui, size: &mut f32) -> Response {
        let at = *size;
        SplitHandle::new("t", size, SplitAxis::Columns)
            .range(100.0..=400.0)
            .default_size(200.0)
            .label("Resize the sidebar")
            .show(ui, at, Rangef::new(0.0, 300.0))
    }

    #[test]
    fn a_drag_moves_the_split_with_the_pointer_within_its_range() {
        let ctx = testkit::ctx();
        let mut size = 200.0;
        let mut t = 0.0;
        let mut step = |events: Vec<Event>, size: &mut f32| {
            t += 0.05;
            frame(&ctx, t, events, |ui| {
                let at = *size;
                let _ = SplitHandle::new("t", size, SplitAxis::Columns)
                    .range(100.0..=400.0)
                    .show(ui, at, Rangef::new(0.0, 300.0));
            });
        };
        step(vec![Event::PointerMoved(pos2(200.0, 50.0))], &mut size);
        step(testkit::press(pos2(200.0, 50.0)), &mut size);
        step(vec![Event::PointerMoved(pos2(230.0, 50.0))], &mut size);
        step(vec![Event::PointerMoved(pos2(260.0, 50.0))], &mut size);
        assert!((size - 260.0).abs() < 0.01, "{size}");
        step(vec![Event::PointerMoved(pos2(900.0, 50.0))], &mut size);
        assert!((size - 400.0).abs() < 0.01, "clamped to the range: {size}");
        step(testkit::release(pos2(900.0, 50.0)), &mut size);
    }

    /// REVIEW-KIT-2: a drag with the pointer leaves keyboard focus where it was, so the arrows
    /// go on moving through the lists instead of resizing the pane.
    #[test]
    fn a_pointer_drag_does_not_take_keyboard_focus() {
        let ctx = testkit::ctx();
        let mut size = 200.0;
        let mut t = 0.0;
        let mut step = |events: Vec<Event>, size: &mut f32| {
            t += 0.05;
            frame(&ctx, t, events, |ui| {
                handle(ui, size);
            });
        };
        step(vec![Event::PointerMoved(pos2(200.0, 50.0))], &mut size);
        step(testkit::press(pos2(200.0, 50.0)), &mut size);
        step(vec![Event::PointerMoved(pos2(230.0, 50.0))], &mut size);
        step(testkit::release(pos2(230.0, 50.0)), &mut size);
        step(Vec::new(), &mut size);
        assert!((size - 230.0).abs() < 0.01, "dragged: {size}");
        assert_eq!(
            ctx.memory(egui::Memory::focused),
            None,
            "the handle took focus"
        );
        let before = size;
        step(testkit::key(Key::ArrowLeft), &mut size);
        assert!(
            (size - before).abs() < f32::EPSILON,
            "the arrows resized it"
        );
    }

    #[test]
    fn a_pane_after_the_seam_shrinks_as_the_seam_moves_towards_it() {
        let ctx = testkit::ctx();
        let mut height = 150.0;
        let seam = 400.0 - 150.0;
        let mut t = 0.0;
        let mut step = |events: Vec<Event>, height: &mut f32| {
            t += 0.05;
            frame(&ctx, t, events, |ui| {
                let _ = SplitHandle::new("rows", height, SplitAxis::Rows)
                    .after(true)
                    .show(ui, seam, Rangef::new(0.0, 600.0));
            });
        };
        step(vec![Event::PointerMoved(pos2(50.0, seam))], &mut height);
        step(testkit::press(pos2(50.0, seam)), &mut height);
        step(
            vec![Event::PointerMoved(pos2(50.0, seam + 20.0))],
            &mut height,
        );
        step(
            vec![Event::PointerMoved(pos2(50.0, seam + 40.0))],
            &mut height,
        );
        assert!((height - 110.0).abs() < 0.01, "{height}");
    }

    #[test]
    fn a_double_click_restores_the_default_size() {
        let ctx = testkit::ctx();
        let mut size = 320.0;
        let at = pos2(320.0, 50.0);
        let mut t = 0.0;
        for events in [
            vec![Event::PointerMoved(at)],
            testkit::press(at),
            testkit::release(at),
            testkit::press(at),
            testkit::release(at),
        ] {
            t += 0.05;
            frame(&ctx, t, events, |ui| {
                let _ = handle(ui, &mut size);
            });
        }
        assert!((size - 200.0).abs() < 0.01, "{size}");
    }

    #[test]
    fn the_arrows_move_a_focused_split_and_shift_moves_it_further() {
        let ctx = testkit::ctx();
        let mut size = 200.0;
        let mut id = None;
        frame(&ctx, 0.0, Vec::new(), |ui| {
            id = Some(handle(ui, &mut size).id);
        });
        ctx.memory_mut(|m| m.request_focus(id.expect("shown")));
        frame(&ctx, 0.1, testkit::key(Key::ArrowRight), |ui| {
            let _ = handle(ui, &mut size);
        });
        assert!((size - (200.0 + KEY_STEP)).abs() < 0.01, "{size}");
        frame(
            &ctx,
            0.2,
            testkit::key_with(Key::ArrowLeft, Modifiers::SHIFT),
            |ui| {
                let _ = handle(ui, &mut size);
            },
        );
        assert!(
            (size - (200.0 + KEY_STEP - KEY_STEP_LARGE)).abs() < 0.01,
            "{size}"
        );
        frame(&ctx, 0.3, testkit::key(Key::Home), |ui| {
            let _ = handle(ui, &mut size);
        });
        assert!((size - 100.0).abs() < 0.01, "{size}");
        frame(&ctx, 0.4, testkit::key(Key::End), |ui| {
            let _ = handle(ui, &mut size);
        });
        assert!((size - 400.0).abs() < 0.01, "{size}");
    }

    #[test]
    fn the_handle_is_a_named_splitter_with_its_size() {
        let ctx = testkit::ctx();
        let mut size = 200.0;
        let nodes = testkit::spoken(&ctx, |ui| {
            let _ = handle(ui, &mut size);
        });
        let n = testkit::find(&nodes, "Resize the sidebar").expect("spoken");
        assert_eq!(n.role, Role::Splitter);
        assert_eq!(n.numeric, Some(200.0));
    }

    /// Hovered, the seam is `focus` at `HOVER_ALPHA` (translucent, so lighter on a light ground);
    /// dragged, full `focus`.
    #[test]
    fn the_hovered_seam_is_a_translucent_focus_and_the_dragged_one_full_focus() {
        for (force, alpha) in [(ForceState::HOVER, HOVER_ALPHA), (ForceState::PRESSED, 255)] {
            let ctx = testkit::ctx_with(
                theme::ThemeId::Daylight,
                theme::Style::Standard,
                theme::Density::Comfortable,
            );
            let p = theme::palette(&ctx);
            let shapes = testkit::frame(&ctx, Vec::new(), |ui| {
                let mut size = 200.0;
                let _ = SplitHandle::new("c", &mut size, SplitAxis::Columns)
                    .force(force)
                    .show(ui, 200.0, Rangef::new(0.0, 100.0));
            });
            let fills: Vec<egui::Color32> = shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Rect(r) if r.fill.a() > 0 => Some(r.fill),
                    _ => None,
                })
                .collect();
            assert_eq!(fills, vec![Palette::tint(p.focus, alpha)], "{force:?}");
        }
    }

    /// A view can lay its panes out where the pointer has dragged the seam before it shows the
    /// handle, so the panes do not trail the pointer by a frame.
    #[test]
    fn a_drag_is_known_before_the_handle_is_shown_again() {
        let ctx = testkit::ctx();
        let mut size = 200.0;
        let mut id = None;
        let mut before = None;
        let mut t = 0.0;
        let mut step = |events: Vec<Event>, size: &mut f32, before: &mut Option<f32>| {
            t += 0.05;
            frame(&ctx, t, events, |ui| {
                let handle = SplitHandle::id_in(ui, "t");
                *before = dragged_size(ui.ctx(), handle, SplitAxis::Columns, false, 100.0..=400.0);
                id = Some(handle);
                let at = *size;
                let _ = SplitHandle::new("t", size, SplitAxis::Columns)
                    .range(100.0..=400.0)
                    .show(ui, at, Rangef::new(0.0, 300.0));
            });
        };
        step(
            vec![Event::PointerMoved(pos2(200.0, 50.0))],
            &mut size,
            &mut before,
        );
        assert_eq!(before, None, "not dragged");
        step(testkit::press(pos2(200.0, 50.0)), &mut size, &mut before);
        step(
            vec![Event::PointerMoved(pos2(230.0, 50.0))],
            &mut size,
            &mut before,
        );
        step(
            vec![Event::PointerMoved(pos2(260.0, 50.0))],
            &mut size,
            &mut before,
        );
        assert_eq!(
            before,
            Some(260.0),
            "the drag, read before the handle is shown"
        );
        assert!((size - 260.0).abs() < 0.01, "{size}");
        step(
            vec![Event::PointerMoved(pos2(900.0, 50.0))],
            &mut size,
            &mut before,
        );
        assert_eq!(before, Some(400.0), "within the range");
        step(testkit::release(pos2(900.0, 50.0)), &mut size, &mut before);
        step(Vec::new(), &mut size, &mut before);
        assert_eq!(before, None, "released");
        let shown = id.expect("shown");
        assert_eq!(
            ctx.read_response(shown).map(|r| r.id),
            Some(shown),
            "id_in is the id the handle is shown with"
        );
    }

    /// Passing over the handle does not light it: only resting on it for `HOVER_DELAY` does.
    #[test]
    fn the_seam_lights_up_only_after_the_pointer_rests() {
        let ctx = testkit::ctx();
        motion::set_reduced(false);
        // At rest the seam is a line; lit, it is a filled bar.
        let lit = |shapes: &[egui::epaint::ClippedShape]| {
            shapes
                .iter()
                .any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill.a() > 0))
        };
        let mut size = 200.0;
        let run = |time: f64, events: Vec<Event>, size: &mut f32| {
            let mut out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 400.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let _ = handle(ui, size);
                },
            );
            out.textures_delta.clear();
            out.shapes
        };
        let first = run(0.0, vec![Event::PointerMoved(pos2(200.0, 50.0))], &mut size);
        assert!(!lit(&first), "lit at once");
        let _ = run(0.05, Vec::new(), &mut size);
        let _ = run(0.12, Vec::new(), &mut size);
        let later = run(0.6, Vec::new(), &mut size);
        motion::set_reduced(true);
        assert!(lit(&later), "never lit");
    }
}
