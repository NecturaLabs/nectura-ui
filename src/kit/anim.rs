//! Animation helpers for the kit: eased booleans and values, springs, a sliding rectangle, the
//! press scale and [`Flip`]. Durations, curves and spring constants come from `ui::motion`. With
//! reduced motion every helper returns its end state at once. Nothing here repaints unless a
//! value is moving.

use egui::epaint::ClippedShape;
use egui::layers::{PaintList, ShapeIdx};
use egui::{Context, Id, IdMap, Pos2, Rect, Shape, Ui, Vec2, pos2};

use crate::motion::{self, Spring};

/// Eases towards `on` over `in_s` when turning on and `out_s` when turning off, along the theme's
/// curve (`motion::enter_curve`); 0 to 1. Zero durations (and reduced motion) switch at once.
pub fn ease_bool(ctx: &Context, id: Id, on: bool, in_s: f32, out_s: f32) -> f32 {
    let seconds = motion::time(if on { in_s } else { out_s });
    ctx.animate_bool_with_time_and_easing(id, on, seconds, motion::enter_curve())
}

/// [`ease_bool`] with one duration both ways.
pub fn ease(ctx: &Context, id: Id, on: bool, seconds: f32) -> f32 {
    ease_bool(ctx, id, on, seconds, seconds)
}

/// Eases a number towards `target` over `seconds` (instant with reduced motion).
pub fn ease_value(ctx: &Context, id: Id, target: f32, seconds: f32) -> f32 {
    ctx.animate_value_with_time(id, target, motion::time(seconds))
}

#[derive(Debug, Clone, Copy)]
struct SpringState {
    x: f32,
    v: f32,
    at: f64,
}

/// One exact step of a damped spring (any damping ratio) from displacement `d = x - target` and
/// velocity `v` over `dt` seconds. Returns the new displacement and velocity.
pub fn spring_step(d: f32, v: f32, s: Spring, dt: f32) -> (f32, f32) {
    let (w, z) = (s.omega, s.zeta);
    if dt <= 0.0 {
        return (d, v);
    }
    if (z - 1.0).abs() < 1e-4 {
        let e = (-w * dt).exp();
        let c = v + w * d;
        let x = (d + c * dt) * e;
        let vel = (c - w * (d + c * dt)) * e;
        (x, vel)
    } else if z < 1.0 {
        let wd = w * (1.0 - z * z).sqrt();
        let e = (-z * w * dt).exp();
        let (sin, cos) = (wd * dt).sin_cos();
        let b = (v + z * w * d) / wd;
        let x = e * (d * cos + b * sin);
        let vel = e * (-z * w * (d * cos + b * sin) + (-d * wd * sin + b * wd * cos));
        (x, vel)
    } else {
        let r = (z * z - 1.0).sqrt();
        let r1 = -w * (z - r);
        let r2 = -w * (z + r);
        let c2 = (v - r1 * d) / (r2 - r1);
        let c1 = d - c2;
        let (e1, e2) = ((r1 * dt).exp(), (r2 * dt).exp());
        (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    }
}

/// A spring towards `target`, its state kept per `id`. The first call returns `target`; reduced
/// motion always does. Frame-rate independent (the exact solution over the real elapsed time),
/// repaints only while moving and snaps once within 0.01 and slower than 0.1 per second.
pub fn spring(ctx: &Context, id: Id, target: f32, s: Spring) -> f32 {
    if motion::reduced() {
        ctx.data_mut(|d| d.remove::<SpringState>(id));
        return target;
    }
    let now = ctx.input(|i| i.time);
    let mut moving = false;
    let x = ctx.data_mut(|d| {
        let st = d.get_temp_mut_or_insert_with(id, || SpringState {
            x: target,
            v: 0.0,
            at: now,
        });
        let dt = crate::to_f32(now - st.at).clamp(0.0, 1.0 / 20.0);
        let (dx, v) = spring_step(st.x - target, st.v, s, dt);
        moving = dx.abs() > 0.01 || v.abs() > 0.1;
        *st = if moving {
            SpringState {
                x: target + dx,
                v,
                at: now,
            }
        } else {
            SpringState {
                x: target,
                v: 0.0,
                at: now,
            }
        };
        st.x
    });
    if moving {
        ctx.request_repaint();
    }
    x
}

/// A rectangle that slides to `target`: the edge moving towards the target leads
/// (`INDICATOR_LEAD`), the other trails (`INDICATOR_TRAIL`), so it stretches then settles. Paint it
/// with pixel rounding off.
pub fn slide_rect(ctx: &Context, id: Id, target: Rect) -> Rect {
    let (left_s, right_s) = if slide_direction(ctx, id, target) {
        (motion::INDICATOR_TRAIL, motion::INDICATOR_LEAD)
    } else {
        (motion::INDICATOR_LEAD, motion::INDICATOR_TRAIL)
    };
    let left = spring(ctx, id.with("kit-slide-l"), target.left(), left_s);
    let right = spring(ctx, id.with("kit-slide-r"), target.right(), right_s);
    let top = spring(
        ctx,
        id.with("kit-slide-t"),
        target.top(),
        motion::INDICATOR_LEAD,
    );
    let bottom = spring(
        ctx,
        id.with("kit-slide-b"),
        target.bottom(),
        motion::INDICATOR_LEAD,
    );
    Rect::from_min_max(pos2(left, top), pos2(right.max(left), bottom.max(top)))
}

/// Whether a slide to `target` travels rightwards: set when the target moves (rightwards when
/// first shown) and kept while it stays put, so the leading edge keeps leading until it settles.
fn slide_direction(ctx: &Context, id: Id, target: Rect) -> bool {
    let key = id.with("kit-slide-target");
    ctx.data_mut(|d| {
        let (last, right) = d.get_temp::<(Rect, bool)>(key).unwrap_or((target, true));
        let moved = target.center().x - last.center().x;
        let right = if moved.abs() < 0.5 {
            right
        } else {
            moved > 0.0
        };
        d.insert_temp(key, (target, right));
        right
    })
}

/// The content scale of a pressed control (`scale` while held, 1 otherwise), on the `PRESS`
/// spring; always 1 with reduced motion.
pub fn press_scale(ctx: &Context, id: Id, held: bool, scale: f32) -> f32 {
    if motion::reduced() {
        return 1.0;
    }
    spring(
        ctx,
        id.with("kit-press-scale"),
        if held { scale } else { 1.0 },
        motion::PRESS,
    )
}

/// One keyed item of a [`Flip`] list as it was last shown.
#[derive(Debug, Clone)]
struct FlipItem {
    /// Where it was laid out, from the list's origin.
    at: Vec2,
    /// The displacement its slide started from (zero at rest), and when.
    from: Vec2,
    since: f64,
    /// When it appeared, while it fades in.
    born: Option<f64>,
    /// What it is when it moves to another key (a file going from Unstaged to Staged).
    alias: Option<Id>,
    /// What it painted, from the list's origin: its fade-out if it leaves.
    shapes: Vec<ClippedShape>,
}

impl FlipItem {
    /// How far from its place it paints at `now`.
    fn offset(&self, now: f64) -> Vec2 {
        let seconds = motion::time(motion::ROW_MOVE);
        if seconds <= 0.0 || self.from == Vec2::ZERO {
            return Vec2::ZERO;
        }
        let t = (crate::to_f32(now - self.since) / seconds).clamp(0.0, 1.0);
        self.from * (1.0 - motion::EASE_OUT.at(t))
    }

    /// Its opacity at `now` while it fades in.
    fn opacity(&self, now: f64) -> f32 {
        let seconds = motion::time(motion::ROW_IN);
        match self.born {
            Some(born) if seconds > 0.0 => {
                motion::EASE_OUT.at((crate::to_f32(now - born) / seconds).clamp(0.0, 1.0))
            }
            _ => 1.0,
        }
    }
}

/// A row that left the list, fading out where it was.
#[derive(Debug, Clone)]
struct Ghost {
    shapes: Vec<ClippedShape>,
    since: f64,
}

#[derive(Debug, Clone, Default)]
struct FlipState {
    generation: Option<u64>,
    items: IdMap<FlipItem>,
    ghosts: Vec<Ghost>,
}

/// Rows of a keyed list that move instead of jumping when the data under the list changes (a
/// file staged, commits fetched, a branch switched): a row whose place changed slides there from
/// where it was painted, a new row fades in where it lands, and a row that left fades out where
/// it was, under the rows that take its place. Only a new `generation` (the data changed)
/// animates: scrolling, resizing and other re-layouts move rows at once, and so does everything
/// with reduced motion. Layout is final at once; only the painting moves, so nothing else jumps.
///
/// `begin` at the top of the list, wrap each row in [`Flip::item`] (keys stable across frames,
/// unique in the list), then [`Flip::end`]. Rows not drawn (a virtual list's rows out of view)
/// are forgotten, not faded out, when `exists` says they are still in the list.
#[derive(Debug)]
pub struct Flip {
    id: Id,
    origin: Pos2,
    generation: u64,
    changed: bool,
    now: f64,
    state: FlipState,
    shown: IdMap<FlipItem>,
    /// Where the faded rows paint, under the list's rows.
    under: ShapeIdx,
    clip: Rect,
    moving: bool,
    /// What the rows stand on: a sliding row is backed with it, so it passes over the rows it
    /// crosses instead of showing through them.
    ground: Option<egui::Color32>,
}

impl Flip {
    /// Starts a frame of the list `id`, whose rows' places are measured from `origin` (a point
    /// that moves with the list's content, such as the top of a scrolled list's content), for
    /// data `generation`.
    pub fn begin(ui: &Ui, id: Id, generation: u64, origin: Pos2) -> Self {
        let ctx = ui.ctx();
        let state = ctx.data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<FlipState>(id)));
        let changed = state.generation.is_some_and(|g| g != generation);
        Self {
            id,
            origin,
            generation,
            changed,
            now: ctx.input(|i| i.time),
            state,
            shown: IdMap::default(),
            under: ui.painter().add(Shape::Noop),
            clip: ui.clip_rect(),
            moving: false,
            ground: None,
        }
    }

    /// The colour the rows stand on (an opaque fill): a row sliding past others is backed with it.
    #[must_use]
    pub fn ground(mut self, ground: egui::Color32) -> Self {
        self.ground = Some(ground);
        self
    }

    /// Lays out one row with `add` at the cursor and paints it where it should show this frame.
    /// `alias` names what the row is apart from its place in the list (a file's path), so a row
    /// whose key changed (the file moved to another section) slides from its old row.
    pub fn item<R>(
        &mut self,
        ui: &mut Ui,
        key: Id,
        alias: Option<Id>,
        add: impl FnOnce(&mut Ui) -> R,
    ) -> R {
        let at = ui.cursor().min - self.origin;
        let now = self.now;
        let previous = self.state.items.remove(&key).or_else(|| {
            // A row new under its key that was in the list under another: it moved.
            let alias = alias.filter(|_| self.changed)?;
            let old = self
                .state
                .items
                .iter()
                .find(|(_, item)| item.alias == Some(alias))
                .map(|(k, _)| *k)?;
            self.state
                .items
                .remove(&old)
                .map(|item| FlipItem { born: None, ..item })
        });
        let (from, since, born) = match previous {
            Some(p) if (p.at - at).length() > 0.5 => {
                if self.changed {
                    (p.at + p.offset(now) - at, now, p.born)
                } else {
                    (Vec2::ZERO, now, None)
                }
            }
            Some(p) => (p.from, p.since, p.born),
            None if self.changed => (Vec2::ZERO, now, Some(now)),
            None => (Vec2::ZERO, now, None),
        };
        let mut item = FlipItem {
            at,
            from,
            since,
            born,
            alias,
            shapes: Vec::new(),
        };
        let offset = item.offset(now);
        let opacity = item.opacity(now);
        if offset == Vec2::ZERO {
            item.from = Vec2::ZERO;
        }
        if opacity >= 1.0 {
            item.born = None;
        }
        self.moving |= offset != Vec2::ZERO || opacity < 1.0;
        let layer = ui.layer_id();
        let ctx = ui.ctx().clone();
        let next =
            |ctx: &Context| ctx.graphics(|g| g.get(layer).map_or(ShapeIdx(0), PaintList::next_idx));
        let backing = self
            .ground
            .filter(|_| offset != Vec2::ZERO)
            .map(|ground| (ui.painter().add(Shape::Noop), ground));
        let top = ui.cursor().top();
        let start = next(&ctx);
        let before = ui.opacity();
        if opacity < 1.0 {
            ui.multiply_opacity(opacity);
        }
        let inner = add(ui);
        ui.set_opacity(before);
        let end = next(&ctx);
        if let Some((slot, ground)) = backing {
            let row = Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().top());
            ui.painter()
                .with_clip_rect(self.clip)
                .set(slot, Shape::rect_filled(row.translate(offset), 0.0, ground));
        }
        if !motion::reduced() {
            let home = self.origin.to_vec2();
            item.shapes = ctx.graphics(|g| {
                g.get(layer).map_or_else(Vec::new, |list| {
                    list.all_entries()
                        .skip(start.0)
                        .take(end.0.saturating_sub(start.0))
                        .map(|c| {
                            let mut c = c.clone();
                            c.shape.translate(-home);
                            c
                        })
                        .collect()
                })
            });
        }
        if offset != Vec2::ZERO {
            // A row's own clip (its cell, its row) moves with it, within the list's.
            let list_clip = self.clip;
            ctx.graphics_mut(|g| {
                let list = g.entry(layer);
                for i in start.0..end.0 {
                    list.mutate_shape(ShapeIdx(i), |c| {
                        c.shape.translate(offset);
                        c.clip_rect = c.clip_rect.translate(offset).intersect(list_clip);
                    });
                }
            });
        }
        self.shown.insert(key, item);
        inner
    }

    /// Ends the frame: rows that left the list (`exists` is false for their key) fade out where
    /// they were.
    pub fn end(mut self, ui: &Ui, exists: impl Fn(Id) -> bool) {
        let ctx = ui.ctx();
        if self.changed && !motion::reduced() {
            for (key, item) in std::mem::take(&mut self.state.items) {
                if !exists(key) && !item.shapes.is_empty() {
                    self.state.ghosts.push(Ghost {
                        shapes: item.shapes,
                        since: self.now,
                    });
                }
            }
        }
        let seconds = motion::time(motion::ROW_OUT);
        let now = self.now;
        self.state
            .ghosts
            .retain(|g| seconds > 0.0 && crate::to_f32(now - g.since) < seconds);
        let home = self.origin.to_vec2();
        let mut faded = Vec::new();
        for ghost in &self.state.ghosts {
            let t = (crate::to_f32(now - ghost.since) / seconds).clamp(0.0, 1.0);
            let opacity = 1.0 - motion::EASE_OUT.at(t);
            for c in &ghost.shapes {
                let mut shape = c.shape.clone();
                shape.translate(home);
                egui::epaint::shape_transform::adjust_colors(&mut shape, move |color| {
                    if *color != egui::Color32::PLACEHOLDER {
                        *color = color.gamma_multiply(opacity);
                    }
                });
                faded.push(shape);
            }
        }
        if !faded.is_empty() {
            ui.painter()
                .with_clip_rect(self.clip)
                .set(self.under, Shape::Vec(faded));
        }
        if self.moving || !self.state.ghosts.is_empty() {
            ctx.request_repaint();
        }
        let state = FlipState {
            generation: Some(self.generation),
            items: self.shown,
            ghosts: self.state.ghosts,
        };
        ctx.data_mut(|d| d.insert_temp(self.id, state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[expect(
        clippy::while_float,
        reason = "the simulation steps a fixed 1/120 s up to three seconds"
    )]
    fn settle(s: Spring) -> (f32, f32) {
        // From 0 towards 1 at 120 Hz: the peak and the time to settle.
        let (mut d, mut v, mut peak, mut t) = (-1.0_f32, 0.0_f32, 0.0_f32, 0.0_f32);
        while t < 3.0 {
            (d, v) = spring_step(d, v, s, 1.0 / 120.0);
            t += 1.0 / 120.0;
            peak = peak.max(1.0 + d);
            if d.abs() < 0.01 && v.abs() < 0.1 {
                break;
            }
        }
        (peak, t)
    }

    #[test]
    fn springs_overshoot_as_specified() {
        let (peak, t) = settle(motion::SWITCH);
        assert!(peak > 1.05 && peak < 1.12, "switch peak {peak}");
        assert!(t < 0.45, "switch settles in {t}");
        let (peak, _) = settle(motion::PRESS);
        assert!(peak <= 1.0 + 1e-4, "press never overshoots: {peak}");
        let (peak, _) = settle(motion::INDICATOR_TRAIL);
        assert!(peak > 1.0 && peak < 1.03, "indicator {peak}");
    }

    #[test]
    fn spring_step_is_frame_rate_independent() {
        let s = motion::SWITCH;
        let (a, _) = spring_step(-1.0, 0.0, s, 0.1);
        let (mut d, mut v) = (-1.0, 0.0);
        for _ in 0..10 {
            (d, v) = spring_step(d, v, s, 0.01);
        }
        assert!((a - d).abs() < 1e-4, "{a} vs {d}");
        let over = Spring {
            omega: 20.0,
            zeta: 1.6,
        };
        let (a, _) = spring_step(-1.0, 0.0, over, 0.2);
        let (mut d, mut v) = (-1.0, 0.0);
        for _ in 0..20 {
            (d, v) = spring_step(d, v, over, 0.01);
        }
        assert!((a - d).abs() < 1e-4);
    }

    /// REVIEW-KIT-4: a slide keeps the direction it set out in until the target moves again, so
    /// the edge moving towards the target leads on every frame, not just the first.
    #[test]
    fn a_slide_keeps_its_direction_until_the_target_moves() {
        let ctx = Context::default();
        let id = Id::new("slide");
        let at = |x: f32| Rect::from_min_size(pos2(x, 0.0), egui::vec2(60.0, 2.0));
        assert!(
            slide_direction(&ctx, id, at(200.0)),
            "first shown: rightwards"
        );
        assert!(
            !slide_direction(&ctx, id, at(100.0)),
            "the target moved left"
        );
        for _ in 0..5 {
            assert!(
                !slide_direction(&ctx, id, at(100.0)),
                "still travelling left"
            );
        }
        assert!(
            slide_direction(&ctx, id, at(300.0)),
            "the target moved right"
        );
        assert!(slide_direction(&ctx, id, at(300.0)));
    }

    #[test]
    fn reduced_motion_lands_at_once() {
        let ctx = Context::default();
        // Other tests install the still appearance too, so this global never flips back.
        motion::set_reduced(true);
        let id = Id::new("t");
        assert_eq!(spring(&ctx, id, 5.0, motion::SWITCH), 5.0);
        assert_eq!(spring(&ctx, id, 9.0, motion::SWITCH), 9.0);
        assert_eq!(press_scale(&ctx, id, true, 0.97), 1.0);
    }

    const ROWS: [egui::Color32; 4] = [
        egui::Color32::RED,
        egui::Color32::GREEN,
        egui::Color32::BLUE,
        egui::Color32::YELLOW,
    ];

    /// One frame at `t` of a [`Flip`] list of 30 pt rows (row `k` a rect filled with `ROWS[k]`)
    /// for data `generation`: every painted rect's colour (alpha included) and top, from the
    /// list's top.
    fn flip_frame(
        ctx: &Context,
        t: f64,
        generation: u64,
        keys: &[usize],
    ) -> Vec<(egui::Color32, f32)> {
        let mut top = 0.0;
        let mut out = ctx.run_ui(
            egui::RawInput {
                time: Some(t),
                ..Default::default()
            },
            |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                top = ui.cursor().top();
                let mut flip = Flip::begin(ui, Id::new("list"), generation, ui.cursor().min);
                for &k in keys {
                    flip.item(ui, Id::new(("row", k)), None, |ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(100.0, 30.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 0.0, ROWS[k]);
                    });
                }
                flip.end(ui, |key| keys.iter().any(|&k| Id::new(("row", k)) == key));
            },
        );
        out.textures_delta.clear();
        let mut rects = Vec::new();
        let mut stack: Vec<Shape> = out.shapes.into_iter().map(|c| c.shape).collect();
        while let Some(shape) = stack.pop() {
            match shape {
                Shape::Vec(inner) => stack.extend(inner),
                Shape::Rect(r) => rects.push((r.fill, r.rect.top() - top)),
                _ => {}
            }
        }
        rects
    }

    /// The top of the rect painted in `color` (any alpha), and its alpha.
    fn painted(rects: &[(egui::Color32, f32)], color: egui::Color32) -> Option<(f32, u8)> {
        rects
            .iter()
            .find(|(c, _)| c.to_opaque() == color.to_opaque() && c.a() > 0)
            .map(|(c, y)| (*y, c.a()))
    }

    /// O1 (owner): rows the data moves slide from where they were, a new row fades in where it
    /// lands and a row that left fades out in its old place, then everything rests where the
    /// layout put it; nothing moves for a re-layout without new data.
    #[test]
    fn flip_rows_slide_fade_in_and_fade_out_when_the_data_changes() {
        let ctx = Context::default();
        motion::set_reduced(false);
        let [red, green, blue, yellow] = ROWS;
        let first = flip_frame(&ctx, 0.0, 1, &[0, 1, 2]);
        assert_eq!(painted(&first, green), Some((30.0, 255)));
        // Red leaves, yellow arrives: green is laid out at 0 but still paints at 30.
        let change = flip_frame(&ctx, 1.0, 2, &[1, 2, 3]);
        assert_eq!(
            painted(&change, green),
            Some((30.0, 255)),
            "green starts where it was"
        );
        assert_eq!(painted(&change, blue), Some((60.0, 255)));
        assert_eq!(
            painted(&change, red),
            Some((0.0, 255)),
            "red fades out where it was"
        );
        assert_eq!(painted(&change, yellow), None, "yellow starts transparent");
        let mid = flip_frame(&ctx, 1.08, 2, &[1, 2, 3]);
        let (y, _) = painted(&mid, green).expect("green");
        assert!(y > 0.0 && y < 30.0, "green slides: {y}");
        let (y, a) = painted(&mid, yellow).expect("yellow fades in");
        assert!(
            (y - 60.0).abs() < 0.01 && a < 255,
            "yellow fades in where it lands: {y} {a}"
        );
        let (_, a) = painted(&mid, red).expect("red fading");
        assert!(a < 255, "red fades: {a}");
        let rest = flip_frame(&ctx, 2.0, 2, &[1, 2, 3]);
        assert_eq!(painted(&rest, green), Some((0.0, 255)));
        assert_eq!(painted(&rest, blue), Some((30.0, 255)));
        assert_eq!(painted(&rest, yellow), Some((60.0, 255)));
        assert_eq!(painted(&rest, red), None, "red is gone");
        // The same data laid out again in another order: no motion.
        let relaid = flip_frame(&ctx, 2.1, 2, &[2, 1, 3]);
        assert_eq!(painted(&relaid, blue), Some((0.0, 255)));
        assert_eq!(painted(&relaid, green), Some((30.0, 255)));
        motion::set_reduced(true);
    }

    /// A sliding row is backed with the list's ground, under its own paint, so the rows it
    /// crosses do not show through it.
    #[test]
    fn a_sliding_row_is_backed_with_the_ground() {
        let ctx = Context::default();
        motion::set_reduced(false);
        let ground = egui::Color32::BLACK;
        let frame = |t: f64, generation: u64, keys: &[usize]| {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    time: Some(t),
                    ..Default::default()
                },
                |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let mut flip = Flip::begin(ui, Id::new("list"), generation, ui.cursor().min)
                        .ground(ground);
                    for &k in keys {
                        flip.item(ui, Id::new(("row", k)), None, |ui| {
                            let (rect, _) = ui
                                .allocate_exact_size(egui::vec2(100.0, 30.0), egui::Sense::hover());
                            ui.painter().rect_filled(rect, 0.0, ROWS[k]);
                        });
                    }
                    flip.end(ui, |_| true);
                },
            );
            out.textures_delta.clear();
            out.shapes
                .into_iter()
                .filter_map(|c| match c.shape {
                    Shape::Rect(r) => Some((r.fill, r.rect.top())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let still = frame(0.0, 1, &[0, 1]);
        assert!(
            still.iter().all(|(c, _)| *c != ground),
            "no backing at rest"
        );
        let moving = frame(1.0, 2, &[1, 0]);
        let backing = moving.iter().position(|(c, _)| *c == ground);
        let red = moving.iter().position(|(c, _)| *c == ROWS[0]);
        let (Some(backing), Some(red)) = (backing, red) else {
            panic!("a backing and the row: {moving:?}");
        };
        assert!(backing < red, "the backing paints under the row");
        motion::set_reduced(true);
    }

    #[test]
    fn flip_rows_land_at_once_with_reduced_motion() {
        let ctx = Context::default();
        motion::set_reduced(true);
        let (red, green, yellow) = (ROWS[0], ROWS[1], ROWS[3]);
        flip_frame(&ctx, 0.0, 1, &[0, 1, 2]);
        let change = flip_frame(&ctx, 1.0, 2, &[1, 2, 3]);
        assert_eq!(painted(&change, green), Some((0.0, 255)));
        assert_eq!(painted(&change, yellow), Some((60.0, 255)));
        assert_eq!(painted(&change, red), None);
    }
}
