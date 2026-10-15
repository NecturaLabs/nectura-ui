//! [`Slider`]: a 4 pt rail filled up to a round thumb that grows under the pointer and shows its
//! value in a bubble while dragged. Design spec 4.9; replaces `egui::Slider`.

use std::ops::RangeInclusive;
use std::sync::Arc;

use egui::emath::Numeric;
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Id, Key, LayerId, Order, Rect, Response, Sense, Ui,
    Vec2, Widget, WidgetInfo, pos2, vec2,
};

use super::button::tip;
use super::toggle::{claim_arrows, gallery_caption, gallery_row, remembered};
use super::{ForceState, Interact, paint};
use crate::motion;
use crate::theme::{self, Elevation, Palette};

/// The thumb's scale under the pointer or while dragged, reached on `SPRING_CURVE` over 0.2 s.
const THUMB_GROW: f32 = 1.12;
const THUMB_GROW_TIME: f32 = 0.2;
/// Space between the value bubble and the thumb.
const BUBBLE_GAP: f32 = 8.0;

/// A value chosen along a range by dragging, clicking or the arrow keys.
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Slider<'a, T: Numeric> {
    value: &'a mut T,
    range: RangeInclusive<T>,
    step: Option<f64>,
    label: String,
    header: bool,
    value_text: Option<Box<dyn Fn(T) -> String + 'a>>,
    width: Option<f32>,
    enabled: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
    force: Option<ForceState>,
}

impl<T: Numeric> std::fmt::Debug for Slider<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Slider")
            .field("value", &self.value.to_f64())
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<'a, T: Numeric> Slider<'a, T> {
    pub fn new(value: &'a mut T, range: RangeInclusive<T>) -> Self {
        Self {
            value,
            range,
            step: None,
            label: String::new(),
            header: false,
            value_text: None,
            width: None,
            enabled: true,
            tooltip: None,
            disabled_reason: None,
            id_salt: None,
            force: None,
        }
    }
    /// Values snap to multiples of `step` from the range's start (integers default to 1).
    pub fn step(mut self, step: f64) -> Self {
        self.step = Some(step);
        self
    }
    /// The slider's accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
    /// Draws the label left and the value right above the rail (concept `.lrow`).
    pub fn header(mut self, header: bool) -> Self {
        self.header = header;
        self
    }
    /// How the value reads in the bubble, the header and to assistive technology.
    pub fn value_text(mut self, f: impl Fn(T) -> String + 'a) -> Self {
        self.value_text = Some(Box::new(f));
        self
    }
    /// Rail width; the available width by default.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
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
    /// `pressed` shows the drag state with its bubble (the gallery and tests only).
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    fn bounds(&self) -> (f64, f64) {
        let (a, b) = (self.range.start().to_f64(), self.range.end().to_f64());
        (a.min(b), a.max(b))
    }

    fn step_size(&self) -> Option<f64> {
        self.step
            .filter(|s| *s > 0.0)
            .or(T::INTEGRAL.then_some(1.0))
    }

    /// `v` snapped to the step and clamped to the range.
    fn snap(&self, v: f64) -> f64 {
        let (lo, hi) = self.bounds();
        let v = match self.step_size() {
            Some(s) => lo + ((v - lo) / s).round() * s,
            None => v,
        };
        v.clamp(lo, hi)
    }

    fn text_of(&self, v: T) -> String {
        match &self.value_text {
            Some(f) => f(v),
            None if T::INTEGRAL => format!("{}", v.to_f64()),
            None => format!("{:.2}", v.to_f64()),
        }
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let (lo, hi) = self.bounds();
        let width = self
            .width
            .unwrap_or_else(|| ui.available_width())
            .max(m.slider_thumb * 4.0);
        let header_font = FontId::new(m.body_size, theme::medium());
        let header = self.header.then(|| {
            let label = ui.painter().layout_no_wrap(
                self.label.clone(),
                header_font.clone(),
                Color32::PLACEHOLDER,
            );
            let value = ui.painter().layout_no_wrap(
                self.text_of(*self.value),
                FontId::new(m.body_size, FontFamily::Monospace),
                Color32::PLACEHOLDER,
            );
            (label, value)
        });
        if let Some((label, value)) = &header {
            let h = label.size().y.max(value.size().y);
            let (row, _) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
            let (painter, color) = if enabled {
                (ui.painter().clone(), p.text)
            } else {
                (super::content_painter(ui), p.disabled_text)
            };
            painter.galley(row.left_top(), Arc::clone(label), color);
            painter.galley(
                pos2(row.right() - value.size().x, row.top()),
                Arc::clone(value),
                color,
            );
        }
        let sense = if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let size = vec2(width, m.slider_hit);
        let (rect, mut response) = match self.id_salt {
            Some(id) => {
                let rect = ui.allocate_space(size).1;
                (rect, ui.interact(rect, ui.id().with(id), sense))
            }
            None => ui.allocate_exact_size(size, sense),
        };
        let r = m.slider_thumb / 2.0;
        let rail_x = egui::Rangef::new(rect.left() + r, rect.right() - r);
        let to_x = |v: f64| {
            let t = if hi > lo { (v - lo) / (hi - lo) } else { 0.0 };
            rail_x.min + rail_x.span() * crate::to_f32(t)
        };
        let from_x = |x: f32| lo + f64::from((x - rail_x.min) / rail_x.span().max(1.0)) * (hi - lo);

        let mut v = self.value.to_f64().clamp(lo, hi);
        if enabled {
            if let Some(pos) = response.interact_pointer_pos()
                && (response.dragged() || response.clicked() || response.drag_started())
            {
                v = self.snap(from_x(pos.x));
                response.request_focus();
            }
            if response.has_focus() {
                claim_arrows(ui, response.id, true, false);
                let unit = self.step_size().unwrap_or((hi - lo) / 100.0);
                let keys = ui.input_mut(|i| {
                    let mut delta = 0.0;
                    let mut jump = None;
                    for (key, sign) in [(Key::ArrowRight, 1.0), (Key::ArrowLeft, -1.0)] {
                        if i.consume_key(egui::Modifiers::SHIFT, key) {
                            delta += sign * 10.0;
                        } else if i.consume_key(egui::Modifiers::NONE, key) {
                            delta += sign;
                        }
                    }
                    if i.consume_key(egui::Modifiers::NONE, Key::Home) {
                        jump = Some(lo);
                    }
                    if i.consume_key(egui::Modifiers::NONE, Key::End) {
                        jump = Some(hi);
                    }
                    (delta, jump)
                });
                match keys {
                    (_, Some(j)) => v = j,
                    (d, None) if d != 0.0 => v = self.snap(v + d * unit),
                    _ => {}
                }
            }
            if (v - self.value.to_f64()).abs() > f64::EPSILON {
                *self.value = T::from_f64(v);
                response.mark_changed();
            }
        }
        let shown = *self.value;
        let v = shown.to_f64().clamp(lo, hi);

        let force = self.force.unwrap_or_default();
        let st = Interact::of(&ctx, &response, enabled, self.force);
        let dragging = enabled && (force.pressed || response.dragged());
        let grow_on = enabled && (dragging || st.hover > 0.0 || force.hover);
        let grow = match self.force {
            Some(f) => f32::from(u8::from(grow_on)) * f.progress.unwrap_or(1.0),
            None => ctx.animate_bool_with_time_and_easing(
                response.id.with("kit-slider-grow"),
                grow_on,
                motion::time(THUMB_GROW_TIME),
                |t| motion::SPRING_CURVE.at(t),
            ),
        };

        if ui.is_rect_visible(rect) {
            let painter = super::content_painter(ui);
            let full = CornerRadius::same(theme::FULL);
            let cy = rect.center().y;
            let rail = Rect::from_x_y_ranges(
                rail_x,
                egui::Rangef::new(cy - m.slider_rail / 2.0, cy + m.slider_rail / 2.0),
            );
            let x = to_x(v);
            // Disabled: the rail in `disabled_edge`, its filled part and the thumb's ring in the
            // disabled mark, the thumb a flat `disabled_fill` (no accent, no shadow).
            let mark = super::disabled_mark(p, s);
            painter.rect_filled(
                rail,
                full,
                if enabled {
                    s.switch_off
                } else {
                    s.disabled_edge
                },
            );
            let filled = Rect::from_min_max(rail.min, pos2(x, rail.bottom()));
            if filled.width() > 0.0 {
                let fill = if enabled {
                    s.primary
                } else {
                    theme::Fill::flat(mark)
                };
                paint::fill(&painter, filled, full, fill);
            }
            let thumb_r = r * (1.0 + (THUMB_GROW - 1.0) * grow);
            let thumb = Rect::from_center_size(pos2(x, cy), Vec2::splat(thumb_r * 2.0));
            if enabled {
                // The same white knob as the switch: its material (contact shadow, and in light
                // themes the knob ring that keeps it apart from a pale rail).
                paint::surface(
                    &painter,
                    thumb,
                    full,
                    p,
                    theme::style(),
                    Elevation::Knob,
                    s.knob,
                    s.knob_edge,
                );
            } else {
                painter.rect_filled(thumb, full, s.disabled_fill);
                paint::rim(&painter, thumb, full, mark);
            }
            if st.focus > 0.0 {
                paint::focus_ring(
                    &painter,
                    thumb,
                    full,
                    p.focus.gamma_multiply(st.focus),
                    s.pane.gamma_multiply(st.focus),
                );
            }
            if dragging {
                bubble(ui, response.id, thumb, &self.text_of(shown), p);
            }
        }

        let label = self.label.clone();
        let (min, max, step) = (lo, hi, self.step_size());
        let spoken = self.text_of(shown);
        response.widget_info(|| {
            let mut info = WidgetInfo::slider(enabled, v, &label);
            info.current_text_value = Some(spoken.clone());
            info
        });
        ctx.accesskit_node_builder(response.id, |n| {
            n.set_min_numeric_value(min);
            n.set_max_numeric_value(max);
            if let Some(s) = step {
                n.set_numeric_value_step(s);
            }
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

impl<T: Numeric> Widget for Slider<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The value above the dragged thumb, on the tooltip surface, painted over everything.
fn bubble(ui: &Ui, id: Id, thumb: Rect, text: &str, p: &Palette) {
    let m = theme::metrics();
    let s = theme::surfaces(ui.ctx());
    let painter = ui
        .ctx()
        .layer_painter(LayerId::new(Order::Tooltip, id.with("kit-slider-bubble")));
    let g = painter.layout_no_wrap(
        text.to_owned(),
        FontId::new(m.small_size, FontFamily::Monospace),
        p.on_tooltip,
    );
    let pad = m.tooltip_pad;
    let size = g.size() + vec2(pad.sum().x, pad.sum().y);
    let rect = Rect::from_center_size(
        pos2(thumb.center().x, thumb.top() - BUBBLE_GAP - size.y / 2.0),
        size,
    );
    let radius = CornerRadius::same(m.radius_sm.saturating_add(2));
    paint::surface(
        &painter,
        rect,
        radius,
        p,
        theme::style(),
        Elevation::Floating,
        s.tooltip,
        Some(p.floating_ring(s)),
    );
    painter.galley(
        pos2(
            rect.left() + f32::from(pad.left),
            rect.top() + f32::from(pad.top),
        ),
        g,
        p.on_tooltip,
    );
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's sliders: rest, hover, dragging with its bubble, focus, disabled, live.
pub(crate) fn gallery(ui: &mut Ui) {
    let lines = |v: u32| format!("{v} lines");
    gallery_caption(
        ui,
        "Live, with a header (drag, click, arrows, Shift, Home, End)",
    );
    remembered(ui, "slider", 3_u32, |ui, v| {
        Slider::new(v, 0..=10)
            .label("Diff context lines")
            .header(true)
            .value_text(|v| format!("{v}"))
            .tooltip("Lines of unchanged code around each change")
            .id_salt("g-sl0")
            .show(ui);
    });
    ui.add_space(theme::SM);
    let states: [(&str, Option<ForceState>, bool); 5] = [
        ("Rest", None, true),
        ("Hover", Some(ForceState::HOVER), true),
        ("Dragging", Some(ForceState::PRESSED), true),
        ("Focus", Some(ForceState::FOCUS), true),
        ("Disabled", None, false),
    ];
    gallery_row(ui, |ui| {
        for (i, (name, force, enabled)) in states.into_iter().enumerate() {
            ui.vertical(|ui| {
                gallery_caption(ui, name);
                ui.add_space(theme::XL);
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "the gallery shows a handful of states"
                )]
                let mut v = 3 + i as u32 * 4;
                let slider = Slider::new(&mut v, 0..=25)
                    .label(name)
                    .value_text(lines)
                    .width(96.0)
                    .enabled(enabled)
                    .disabled_reason("Diff options are locked while comparing")
                    .id_salt(("g-sl", i));
                match force {
                    Some(f) => slider.show_forced(ui, f),
                    None => slider.show(ui),
                };
            });
        }
    });
    ui.add_space(theme::XS);
    gallery_caption(ui, "Zoom, continuous with a step, 150 wide");
    remembered(ui, "zoom", 1.0_f32, |ui, v| {
        Slider::new(v, 0.8..=1.6)
            .step(0.05)
            .label("Zoom")
            .value_text(|v| format!("{:.0}%", v * 100.0))
            .width(150.0)
            .id_salt("g-sl-zoom")
            .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(ui, "Thumb growing under the pointer at t = 0, 0.5, 1");
    gallery_row(ui, |ui| {
        for (i, t) in [0.0_f32, 0.5, 1.0].into_iter().enumerate() {
            Slider::new(&mut 5_u32, 0..=10)
                .width(96.0)
                .id_salt(("g-sl-m", i))
                .show_forced(
                    ui,
                    ForceState {
                        hover: true,
                        progress: Some(t),
                        ..ForceState::default()
                    },
                );
        }
    });
}

#[cfg(test)]
mod tests {
    use egui::Modifiers;

    use super::super::testkit::{self, find, frame, key, key_with, spoken};
    use super::*;

    #[test]
    fn keys_step_jump_and_clamp() {
        let ctx = testkit::ctx();
        let mut v = 3_u32;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| {
            id = Slider::new(&mut v, 0..=25).show(ui).id;
        });
        ctx.memory_mut(|m| m.request_focus(id));
        let press = |events: Vec<egui::Event>, v: &mut u32| {
            frame(&ctx, events, |ui| {
                Slider::new(v, 0..=25).show(ui);
            });
        };
        press(key(Key::ArrowRight), &mut v);
        assert_eq!(v, 4);
        press(key_with(Key::ArrowRight, Modifiers::SHIFT), &mut v);
        assert_eq!(v, 14, "Shift steps by ten");
        press(key_with(Key::ArrowRight, Modifiers::SHIFT), &mut v);
        assert_eq!(v, 24);
        press(key(Key::ArrowRight), &mut v);
        press(key(Key::ArrowRight), &mut v);
        assert_eq!(v, 25, "clamped at the end");
        press(key(Key::Home), &mut v);
        assert_eq!(v, 0);
        press(key(Key::End), &mut v);
        assert_eq!(v, 25);
    }

    #[test]
    fn clicking_the_rail_snaps_to_the_step() {
        let ctx = testkit::ctx();
        let mut v = 1.0_f32;
        let mut rect = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            rect = Slider::new(&mut v, 0.0..=2.0)
                .step(0.5)
                .width(200.0)
                .show(ui)
                .rect;
        });
        let r = theme::metrics().slider_thumb / 2.0;
        // Three tenths along the rail: 0.6, snapped to 0.5.
        let at = pos2(
            rect.left() + r + (rect.width() - 2.0 * r) * 0.3,
            rect.center().y,
        );
        testkit::click(&ctx, at, |ui| {
            Slider::new(&mut v, 0.0..=2.0)
                .step(0.5)
                .width(200.0)
                .show(ui);
        });
        assert_eq!(v, 0.5);
    }

    #[test]
    fn disabled_slider_ignores_pointer_and_keys() {
        let ctx = testkit::ctx();
        let mut v = 3_u32;
        let mut r = None;
        frame(&ctx, Vec::new(), |ui| {
            r = Some(Slider::new(&mut v, 0..=25).enabled(false).show(ui));
        });
        let r = r.expect("shown");
        testkit::click(&ctx, r.rect.right_center(), |ui| {
            Slider::new(&mut v, 0..=25).enabled(false).show(ui);
        });
        ctx.memory_mut(|m| m.request_focus(r.id));
        frame(&ctx, key(Key::End), |ui| {
            Slider::new(&mut v, 0..=25).enabled(false).show(ui);
        });
        assert_eq!(v, 3);
    }

    #[test]
    fn slider_speaks_its_value_and_range() {
        let ctx = testkit::ctx();
        let mut v = 3_u32;
        let nodes = spoken(&ctx, |ui| {
            Slider::new(&mut v, 0..=25)
                .label("Context lines")
                .value_text(|v| format!("{v} lines"))
                .show(ui);
        });
        let n = find(&nodes, "Context lines").expect("spoken");
        assert_eq!(n.role, egui::accesskit::Role::Slider);
        assert_eq!(n.numeric, Some(3.0));
        assert_eq!(n.value.as_deref(), Some("3 lines"));
    }

    #[test]
    fn the_thumb_grows_at_once_with_reduced_motion() {
        let ctx = testkit::ctx();
        let thumb = |events: Vec<egui::Event>| {
            let mut rect = Rect::NOTHING;
            let mut v = 5_u32;
            let shapes = frame(&ctx, events, |ui| {
                rect = Slider::new(&mut v, 0..=10).width(200.0).show(ui).rect;
            });
            let knob = theme::surfaces(&ctx).knob.top;
            let r = shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Rect(r) if r.fill == knob && r.blur_width == 0.0 => {
                        Some(r.rect.width() / 2.0)
                    }
                    _ => None,
                })
                .expect("thumb painted");
            (r, rect)
        };
        let (rest, rect) = thumb(Vec::new());
        assert_eq!(rest, theme::metrics().slider_thumb / 2.0);
        let (hover, _) = thumb(vec![egui::Event::PointerMoved(rect.center())]);
        assert!(
            (hover / rest - THUMB_GROW).abs() < 1e-4,
            "the first hovered frame shows the grown thumb: {hover}"
        );
    }
}
