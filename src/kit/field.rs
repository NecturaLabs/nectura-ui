//! Text inputs: [`TextField`], [`SearchField`], [`TextArea`], [`CodeArea`] and [`NumberField`].
//! Design spec 4.2.
//!
//! Each wraps an `egui::TextEdit` with no frame of its own: the kit reserves the field's rect and
//! a paint slot under it, shows the edit, then fills the slot (the well) and paints the edge, the
//! focus or error halo, the leading icon and the trailing slot (clear, reveal, keycap, spinner,
//! counter, stepper) once the edit's hover and focus are known. A label above, and a helper or
//! error message below, are optional. Text fields always show focus, whatever the input modality.
//!
//! **The well** (`look`). A field is a well one step off whatever surface it sits on: a
//! translucent wash of `text` in a dark theme, as strong as every text colour allows on every
//! surface; in a light theme a plate as light as the lightest surface, edged. Either way a field
//! reads as a field on the pane, the chrome, a card or a dialog alike. Deeper adds the `Field`
//! material's inner shadow along the top (a recess). Text stops at the well's inner edge, clear of the trailing slot.
//!
//! **The edge** is one even hairline at rest, as quiet as the edges of the buttons and selects
//! beside it: the quietest mix of `edge` and `control_border` that still traces the well on every
//! surface (`QUIET_EDGE_MIN`); high contrast keeps an input outline of 4.5:1. Hover takes it to
//! `control_border`, focus to `focus` with the halo, an error to `danger_base`. Disabled fields use
//! the theme's disabled tokens; read-only ones have no well and the theme's plain edge, so neither
//! looks like a field waiting for input.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::RangeInclusive;

use egui::accesskit::Invalid;
use egui::epaint::tessellator::path::rounded_rectangle;
use egui::epaint::{CornerRadiusF32, PathShape, PathStroke};
use egui::{
    Align, Color32, CornerRadius, FontFamily, FontId, Galley, Id, Key, Layout, Margin, Modifiers,
    Painter, Rect, Response, Sense, Shape, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2,
    Widget, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::Shortcut;
use super::button::{spinner_due, tip};
use super::chip::Keycap;
use super::overlay::TooltipExt;
use super::toggle::{component_id, gallery_caption, remembered};
use super::{ForceState, IconButton, KitResponse, Size, anim, paint};
use crate::motion;
use crate::theme::{self, Elevation, FocusStyle, Palette, Style};

/// The focus and error halo, drawn outside the edge (3 pt; focus at 26%, error at 18%).
pub(crate) const HALO: f32 = 3.0;
pub(crate) const FOCUS_HALO_ALPHA: u8 = 66;
const ERROR_HALO_ALPHA: u8 = 46;
/// The thickness of the focus line under a borderless search.
const FOCUS_LINE: f32 = 2.0;
/// The well: `text` at this alpha at most (less where a text colour would not read), and this
/// much more while hovered.
const WELL_ALPHA: u8 = 16;
const HOVER_ALPHA: u8 = 6;
/// The resting edge outside high contrast: as quiet as the other controls' edges, just enough to
/// trace the well on every surface. High contrast keeps the input-outline target.
const QUIET_EDGE_MIN: f64 = 1.5;
/// Text areas: the column guide shows once a line comes this close to it.
const GUIDE_REACH: f32 = theme::XL;
/// The summary counter turns `warning` past 50 characters and `danger` past 72.
const COUNTER_WARN: usize = 51;
const COUNTER_DANGER: usize = 73;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Line,
    Search,
    Area {
        rows: RangeInclusive<usize>,
    },
    Code {
        rows: RangeInclusive<usize>,
        line_numbers: bool,
    },
}

impl Kind {
    fn multiline(&self) -> bool {
        matches!(self, Self::Area { .. } | Self::Code { .. })
    }
}

/// The colours a field paints with, on any surface (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FieldLook {
    /// The well at rest and hovered: translucent, laid over the surface under the field.
    pub wash: Color32,
    pub hover_wash: Color32,
    /// The read-only edge, and the resting edge of a field waiting for input.
    pub side: Color32,
    pub base: Color32,
}

/// The contrast targets of the theme's gate (`theme.rs` tests): text, `text_dim`, `text_faint`
/// and input outlines; strict themes ask for more.
fn targets(p: &Palette) -> (f64, f64, f64, f64) {
    if p.strict {
        (10.0, 7.0, 5.5, 4.5)
    } else {
        (7.0, 4.5, 4.5, 3.0)
    }
}

/// The surfaces a field can sit on in `style`: the pane, the chrome, cards, floating layers and
/// Deeper's window ground.
pub(crate) fn grounds(p: &Palette, style: Style) -> [Color32; 5] {
    let s = p.surfaces(style);
    [s.pane, s.panel, s.elevated, s.overlay, s.bg]
}

/// How a field looks in theme `p` and `style`. Cached per theme and style.
pub(crate) fn look(p: &'static Palette, style: Style) -> FieldLook {
    // By the palette itself: a theme's high contrast and each custom theme share its name.
    thread_local! {
        static CACHE: RefCell<HashMap<(usize, Style), FieldLook>> = RefCell::default();
    }
    let key = (std::ptr::from_ref(p).addr(), style);
    if let Some(l) = CACHE.with(|c| c.borrow().get(&key).copied()) {
        return l;
    }
    let s = p.surfaces(style);
    let grounds = grounds(p, style);
    let (text_min, dim_min, faint_min, rim_min) = targets(p);
    let rim_min = if p.strict { rim_min } else { QUIET_EDGE_MIN };
    let reads_over = |wash: Color32| {
        grounds.iter().all(|g| {
            let c = theme::over(wash, *g);
            theme::contrast(p.text, c) >= text_min
                && theme::contrast(p.text_dim, c) >= dim_min
                && theme::contrast(p.text_faint, c) >= faint_min
        })
    };
    // As much of `text` as every text colour allows on every surface.
    let strongest = |most: u8| {
        (0..=most)
            .rev()
            .map(|a| Palette::tint(p.text, a))
            .find(|w| reads_over(*w))
            .unwrap_or(Color32::TRANSPARENT)
    };
    // A dark theme's well is a faint lift of the surface; a light theme's is a plate as light as
    // the lightest surface it can sit on, which reads as a place to type, where a grey well read
    // as a disabled control.
    let (wash, hover_wash) = if p.dark {
        (
            strongest(WELL_ALPHA),
            strongest(WELL_ALPHA.saturating_add(HOVER_ALPHA)),
        )
    } else {
        let plate = grounds
            .into_iter()
            .max_by(|a, b| theme::luminance(*a).total_cmp(&theme::luminance(*b)))
            .unwrap_or(s.pane);
        (plate, plate)
    };
    let wells = grounds.map(|g| theme::over(wash, g));
    let base = (0..=20)
        .map(|i| theme::mix(s.edge, p.control_border, crate::count_f32(i) / 20.0))
        .find(|e| {
            grounds
                .iter()
                .chain(&wells)
                .all(|g| theme::contrast(*e, *g) >= rim_min)
        })
        .unwrap_or(p.control_border);
    let side = if p.strict { base } else { s.edge };
    let l = FieldLook {
        wash,
        hover_wash,
        side,
        base,
    };
    CACHE.with(|c| c.borrow_mut().insert(key, l));
    l
}

/// A field's 1 pt edge inside `rect`: `side` around, turning to `base` over the bottom corners,
/// so the bottom edge is the strong one.
pub(crate) fn edge_shape(rect: Rect, radius: CornerRadius, side: Color32, base: Color32) -> Shape {
    let width = paint::edge();
    let mut points = Vec::new();
    rounded_rectangle(
        &mut points,
        rect.shrink(width / 2.0),
        CornerRadiusF32::from(radius),
    );
    let ramp = f32::from(radius.sw.max(radius.se)).max(2.0);
    let from = rect.bottom() - ramp;
    let stroke = PathStroke::new_uv(width, move |_bounds: Rect, at: egui::Pos2| {
        side.lerp_to_gamma(base, ((at.y - from) / ramp).clamp(0.0, 1.0))
    });
    Shape::Path(PathShape {
        points,
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke,
    })
}

/// A field's box: the `fill` (a translucent well or an opaque disabled fill), the halo `(colour,
/// alpha, strength)` outside, and the edge (the theme's edge width), `side` turning to `base` along
/// the bottom. The halo takes the theme's focus style (`Design::focus`): a tinted band (`Ring`) or
/// a solid ring past a gap of the ground (`Double`, high contrast).
pub(crate) fn paint_box(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    edge: (Color32, Color32),
    halo: Option<(Color32, u8, f32)>,
) {
    if let Some((color, alpha, t)) = halo {
        let grow = |by: f32| radius + CornerRadius::same(crate::saturate_u8(by.round()));
        match theme::active_design().focus {
            FocusStyle::Ring { .. } => {
                let w = HALO * t;
                painter.rect_stroke(
                    rect.expand(w / 2.0),
                    grow(w / 2.0),
                    Stroke::new(w, Palette::tint(color, alpha)),
                    StrokeKind::Middle,
                );
            }
            FocusStyle::Double { width } => {
                let w = width * t;
                painter.rect_stroke(
                    rect.expand(1.0 + w / 2.0),
                    grow(1.0 + w / 2.0),
                    Stroke::new(w, color),
                    StrokeKind::Middle,
                );
            }
        }
    }
    if edge.0 == edge.1 {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(paint::edge(), edge.0),
            StrokeKind::Inside,
        );
    } else {
        painter.add(edge_shape(rect, radius, edge.0, edge.1));
    }
}

/// Everything the fields share.
#[derive(Debug)]
struct Core<'t> {
    text: &'t mut String,
    kind: Kind,
    label: Option<String>,
    name: Option<String>,
    placeholder: String,
    helper: Option<String>,
    error: Option<String>,
    icon: Option<String>,
    password: bool,
    clearable: bool,
    loading: bool,
    read_only: bool,
    enabled: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    width: Option<f32>,
    size: Size,
    counter: bool,
    guide: Option<usize>,
    shortcut: Option<Shortcut>,
    borderless: bool,
    /// A − and + pair at the trailing end, with their names and whether each can step.
    stepper: Option<Stepper>,
    id_salt: Option<Id>,
    id: Option<Id>,
    force: Option<ForceState>,
}

#[derive(Debug, Clone)]
struct Stepper {
    less: String,
    more: String,
    can_less: bool,
    can_more: bool,
}

/// What showing a field produced.
#[derive(Debug)]
struct Shown {
    response: Response,
    /// Ctrl+Enter submitted a multi-line field.
    submitted: bool,
    /// The stepper's − (-1) or + (1) was clicked.
    stepped: i8,
}

impl<'t> Core<'t> {
    fn new(text: &'t mut String, kind: Kind) -> Self {
        Self {
            text,
            kind,
            label: None,
            name: None,
            placeholder: String::new(),
            helper: None,
            error: None,
            icon: None,
            password: false,
            clearable: false,
            loading: false,
            read_only: false,
            enabled: true,
            tooltip: None,
            disabled_reason: None,
            width: None,
            size: Size::Lg,
            counter: false,
            guide: None,
            shortcut: None,
            borderless: false,
            stepper: None,
            id_salt: None,
            id: None,
            force: None,
        }
    }

    /// The field's id: the caller's, else from its salt or its place in `ui`.
    fn id(&self, ui: &Ui) -> Id {
        self.id
            .unwrap_or_else(|| component_id(ui, "kit-field", self.id_salt))
    }

    fn show(self, ui: &mut Ui) -> Shown {
        let enabled = self.enabled && ui.is_enabled();
        let id = self.id(ui);
        // Label, field and message stack in a slot of their own size that the parent's layout
        // places (at the right of a right-to-left row, as a select's). The stack's height is last
        // frame's; a change is measured and redrawn at once.
        let m = theme::metrics();
        let width = self
            .width
            .unwrap_or_else(|| ui.available_width())
            .max(m.control * 2.0);
        let height_id = id.with("kit-field-height");
        let height = ui.data(|d| d.get_temp::<f32>(height_id)).unwrap_or(
            if self.borderless && self.kind == Kind::Search {
                m.palette_input_height
            } else {
                self.size.height()
            },
        );
        let shown =
            ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), |ui| {
                // A disabled field is not `Ui::disable`d (that fades its text past reading): it
                // paints in the theme's disabled tokens, and the edit stops taking input.
                ui.spacing_mut().item_spacing.y = 0.0;
                self.show_inner(ui, id, enabled)
            });
        let used = shown.response.rect.height();
        if (used - height).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(height_id, used));
            // A top-down parent places the slot at its cursor whatever its height (the usual
            // case, and a growing text area's every frame), so only other layouts redraw.
            if ui.layout().main_dir() != egui::Direction::TopDown {
                ui.ctx().request_discard("field measured");
            }
        }
        shown.inner
    }

    fn show_inner(self, ui: &mut Ui, id: Id, enabled: bool) -> Shown {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let look = look(p, style);
        let width = self
            .width
            .unwrap_or_else(|| ui.available_width())
            .max(m.control * 2.0);
        let message_font = FontId::new(m.meta_size, FontFamily::Proportional);

        // Labels and icons paint at full strength even in a disabled `Ui` (in disabled tokens).
        let ink = super::content_painter(ui);
        let (label_color, faint, text_dim) = if enabled {
            (p.text_dim, p.text_faint, p.text_dim)
        } else {
            (p.disabled_text, p.disabled_text, p.disabled_text)
        };
        let label_id = self.label.as_ref().map(|text| {
            let g = ink.layout_no_wrap(
                text.clone(),
                FontId::new(m.meta_size, theme::medium()),
                label_color,
            );
            let (rect, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            ink.galley(rect.min, g, label_color);
            let r = ui.interact(rect, id.with("label"), Sense::hover());
            r.widget_info(|| WidgetInfo::labeled(WidgetType::Label, enabled, text));
            ui.add_space(m.field_label_gap);
            r.id
        });

        // The borderless search is the surface's own input (the command palette, the
        // cheatsheet): the palette's input height and title-sized text.
        let surface_input = self.borderless && self.kind == Kind::Search;
        let font = match self.kind {
            Kind::Code { .. } => FontId::new(m.mono_size, FontFamily::Monospace),
            _ if surface_input => FontId::new(m.title_size, FontFamily::Proportional),
            _ => FontId::new(m.body_size, FontFamily::Proportional),
        };
        let line_h = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().extra_text_line_spacing;
        let pad_y = if self.kind.multiline() {
            theme::SM
        } else {
            0.0
        };
        let height = match &self.kind {
            Kind::Area { rows } | Kind::Code { rows, .. } => {
                let min = crate::count_f32(*rows.start()) * line_h + 2.0 * pad_y;
                let max = crate::count_f32((*rows.end()).max(*rows.start())) * line_h + 2.0 * pad_y;
                let content: f32 = ctx
                    .data(|d| d.get_temp(id.with("kit-area-content")))
                    .unwrap_or(min);
                let target = content.clamp(min, max);
                anim::ease_value(&ctx, id.with("kit-area-height"), target, motion::BASE)
            }
            _ if surface_input => m.palette_input_height,
            _ => self.size.height(),
        };
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let radius = CornerRadius::same(m.radius);
        // The well goes here, under the text, once hover is known.
        let painter = if enabled {
            ui.painter().clone()
        } else {
            ink.clone()
        };
        let well_slot = painter.add(Shape::Noop);
        let bare = self.borderless;
        let interactive = enabled && !self.read_only;
        let material = p.material(style, Elevation::Field);
        if !bare && interactive && material.recess > 0 {
            // Deeper's recess: an inner shadow along the top of the well, under the text.
            paint::recess(&painter, rect, radius, p.shadow, material.recess);
        }

        // The slots either side of the text.
        let icon_glyph = match self.kind {
            Kind::Search => Some(icon::MAGNIFYING_GLASS.to_owned()),
            _ => self.icon.clone(),
        };
        let mut left = m.field_pad_x;
        if icon_glyph.is_some() {
            left += m.icon_md + theme::SM;
        }
        let gutter = match &self.kind {
            Kind::Code {
                line_numbers: true, ..
            } => {
                let lines =
                    self.text.lines().count().max(1) + usize::from(self.text.ends_with('\n'));
                let digits = lines.to_string().len().max(2);
                let w = ui.fonts_mut(|f| {
                    f.layout_no_wrap("0".repeat(digits), font.clone(), Color32::PLACEHOLDER)
                        .size()
                        .x
                });
                Some(w + 2.0 * theme::SM)
            }
            _ => None,
        };
        if let Some(g) = gutter {
            left = g + theme::SM;
        }
        let filled = !self.text.is_empty();
        let counter = (self.counter && filled).then(|| {
            let n = self.text.chars().count();
            let color = if n >= COUNTER_DANGER {
                p.danger
            } else if n >= COUNTER_WARN {
                p.warning
            } else {
                p.text_faint
            };
            let g = ui.painter().layout_no_wrap(
                n.to_string(),
                FontId::new(m.small_size, FontFamily::Monospace),
                color,
            );
            (g, color)
        });
        let keycap = (self.kind == Kind::Search && !filled)
            .then_some(self.shortcut)
            .flatten()
            .map(|c| {
                // The bare key when there is one (concept 1 shows `/`), else the chord.
                let hint = c.hint();
                Keycap::new(hint.rsplit(" · ").next().unwrap_or_default())
            });
        let clears = self.kind == Kind::Search || self.clearable;
        // The small buttons are drawn at `Size::Xs`, with a hit area of at least 24 points
        // (WCAG 2.5.8).
        let button = Size::Xs.height().max(theme::MIN_TARGET);
        let trail_pad = if self.kind == Kind::Search || self.stepper.is_some() {
            theme::XS
        } else {
            m.field_pad_x
        };
        let mut right = trail_pad;
        if let Some(k) = &keycap {
            right += k.measure_on(ui.painter()).x + theme::SM;
        } else if clears {
            right += button + theme::SM;
        }
        if let Some((g, _)) = &counter {
            right += g.size().x + theme::SM;
        }
        if self.password {
            right += button + theme::SM;
        }
        if self.loading {
            right += m.icon_md + theme::SM;
        }
        if self.stepper.is_some() {
            right += 2.0 * button + theme::XS + theme::SM;
        }
        // Shown or not, and the frame the field was last drawn: a secret starts hidden whenever
        // its field comes back (the next credential prompt reuses the same id).
        let reveal_id = id.with("kit-field-reveal");
        let now = ctx.cumulative_frame_nr();
        let revealed = self.password
            && ctx
                .data(|d| d.get_temp::<(bool, u64)>(reveal_id))
                .is_some_and(|(on, seen)| on && seen.saturating_add(1) >= now);
        if self.password {
            ctx.data_mut(|d| d.insert_temp(reveal_id, (revealed, now)));
        }

        // The edit itself, in a frameless child over the reserved rect.
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        {
            let v = child.visuals_mut();
            v.weak_text_color = Some(faint);
            v.selection.bg_fill = p.selected();
            v.text_cursor.stroke = Stroke::new(2.0, p.accent);
        }
        if !enabled {
            child.set_opacity(ink.opacity());
        }
        let multiline = self.kind.multiline();
        if !multiline {
            // A line too long for the field stops at the well's inner edge (the caret's width
            // past it), clear of the trailing slot, as a native field's does.
            let inner = Rect::from_x_y_ranges(
                (rect.left() + left - 1.0)..=(rect.right() - right + 1.0),
                rect.y_range(),
            );
            child.set_clip_rect(child.clip_rect().intersect(inner));
        }
        let text_color = if !enabled {
            p.disabled_text
        } else if self.read_only {
            p.text_dim
        } else {
            p.text
        };
        let margin = Margin {
            left: crate::points_i8(left.round()),
            right: crate::points_i8(right.round()),
            top: crate::points_i8(pad_y),
            bottom: crate::points_i8(pad_y),
        };
        let edit_id = edit_id(id);
        let mut edit = if multiline {
            TextEdit::multiline(self.text)
        } else {
            TextEdit::singleline(self.text)
        }
        .id(edit_id)
        .frame(egui::Frame::NONE.inner_margin(margin))
        .font(font.clone())
        .text_color(text_color)
        .hint_text(self.placeholder.clone())
        .interactive(interactive)
        .password(self.password && !revealed)
        .desired_width(width);
        if let Kind::Code { .. } = self.kind {
            edit = edit.code_editor();
        }
        let (mut response, content_h, galley, galley_pos) = if multiline {
            let rows = match &self.kind {
                Kind::Area { rows } | Kind::Code { rows, .. } => *rows.start(),
                _ => 1,
            };
            let out = egui::ScrollArea::vertical()
                .id_salt(id.with("kit-area-scroll"))
                .max_height(height)
                .auto_shrink(false)
                .show(&mut child, |ui| {
                    edit.desired_rows(rows)
                        .min_size(vec2(width, height))
                        .show(ui)
                });
            let o = out.inner;
            let content = o.galley.size().y + 2.0 * pad_y;
            // The edit is as tall as its text; the field is what shows of it.
            let mut response = o.response.response;
            response.rect = rect;
            (response, content, o.galley, o.galley_pos)
        } else {
            let o = edit
                .min_size(vec2(width, height))
                .vertical_align(Align::Center)
                .show(&mut child);
            let h = o.galley.size().y;
            (o.response.response, h, o.galley, o.galley_pos)
        };
        if multiline {
            ctx.data_mut(|d| d.insert_temp(id.with("kit-area-content"), content_h));
        }

        // Esc in a search field clears it first, then lets go of focus.
        let escaped = ui.input(|i| i.key_pressed(Key::Escape));
        if self.kind == Kind::Search && escaped && response.lost_focus() && filled {
            self.text.clear();
            response.mark_changed();
            response.request_focus();
        }
        let submitted = multiline
            && response.has_focus()
            && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter));

        let force = self.force.unwrap_or_default();
        let hovered = interactive && (force.hover || response.hovered());
        let focused = force.focus || response.has_focus();
        let focus_t = match self.force {
            Some(f) => f32::from(u8::from(focused)) * f.progress.unwrap_or(1.0),
            None => anim::ease_bool(&ctx, id.with("kit-field-focus"), focused, motion::BASE, 0.0),
        };
        let hover_t = match self.force {
            Some(f) => f32::from(u8::from(hovered)) * f.progress.unwrap_or(1.0),
            None => anim::ease_bool(
                &ctx,
                id.with("kit-field-hover"),
                hovered,
                motion::FAST,
                motion::FAST_OUT,
            ),
        };
        let error = self.error.as_deref().filter(|e| !e.is_empty());

        // The well, the edge and the halo.
        let fill = if !enabled {
            Some(s.disabled_fill)
        } else if self.read_only {
            None
        } else {
            Some(
                look.wash
                    .lerp_to_gamma(look.hover_wash, hover_t * (1.0 - focus_t)),
            )
        };
        if !bare && let Some(fill) = fill {
            painter.set(well_slot, Shape::rect_filled(rect, radius, fill));
        }
        let edge = if !enabled {
            (s.disabled_edge, s.disabled_edge)
        } else if self.read_only {
            (look.side, look.side)
        } else if error.is_some() {
            (p.danger_base, p.danger_base)
        } else {
            let to = |c: Color32| {
                c.lerp_to_gamma(p.control_border, hover_t)
                    .lerp_to_gamma(p.focus, focus_t)
            };
            (to(look.base), to(look.base))
        };
        let halo = if error.is_some() && interactive {
            Some((p.danger_base, ERROR_HALO_ALPHA, 1.0_f32.max(focus_t)))
        } else if interactive && focus_t > 0.0 {
            Some((p.focus, FOCUS_HALO_ALPHA, focus_t))
        } else {
            None
        };
        if !bare {
            paint_box(&painter, rect, radius, edge, halo);
        } else if interactive && focus_t > 0.0 {
            // The surface's own input has no box to ring: the line under it, over the seam the
            // surface draws there, takes the focus colour.
            let line = Rect::from_min_max(pos2(rect.left(), rect.bottom() - FOCUS_LINE), rect.max);
            painter.rect_filled(line, 0.0, p.focus.gamma_multiply(focus_t));
        }

        if let Some(glyph) = icon_glyph {
            // The leading icon brightens with focus, like the text it introduces.
            let tint = faint.lerp_to_gamma(text_dim, focus_t);
            let g = ink.layout_no_wrap(glyph, FontId::proportional(m.icon_md), tint);
            let y = if multiline {
                rect.top() + pad_y + (line_h - g.size().y) / 2.0
            } else {
                rect.center().y - g.size().y / 2.0
            };
            ink.galley(pos2(rect.left() + m.field_pad_x, y), g, tint);
        }
        if let Some(gutter) = gutter {
            paint_line_numbers(&painter, rect, gutter, &galley, galley_pos, &font, p);
        }
        if let Some(col) = self.guide {
            let w = ui.fonts_mut(|f| {
                f.layout_no_wrap("0".repeat(col), font.clone(), Color32::PLACEHOLDER)
                    .size()
                    .x
            });
            let x = rect.left() + left + w;
            // A writing aid, not a fixture: it shows while the field is being written in, or
            // once a line reaches it, so an idle description has no stray rule in it.
            let reached = galley
                .rows
                .iter()
                .any(|r| galley_pos.x + r.rect().right() >= x - GUIDE_REACH);
            let shown = if reached { 1.0 } else { focus_t };
            if x < rect.right() - right && shown > 0.0 {
                let dashes = Shape::dashed_line(
                    &[pos2(x, rect.top() + 2.0), pos2(x, rect.bottom() - 2.0)],
                    Stroke::new(paint::SEAM, s.border.gamma_multiply(shown)),
                    1.0,
                    2.0,
                );
                painter.extend(dashes);
            }
        }

        // Trailing slot, right to left: the stepper, the search keycap or clear button, the
        // show-password toggle, the spinner, then the counter.
        let mut x = rect.right() - trail_pad;
        let cy = if multiline {
            rect.top() + pad_y + line_h / 2.0
        } else {
            rect.center().y
        };
        let slot =
            |x: f32| Rect::from_min_size(pos2(x - button, cy - button / 2.0), Vec2::splat(button));
        let mut stepped = 0;
        if let Some(st) = &self.stepper {
            for (glyph, name, can, step, salt) in [
                (icon::PLUS, &st.more, st.can_more, 1, "kit-field-more"),
                (icon::MINUS, &st.less, st.can_less, -1, "kit-field-less"),
            ] {
                let at = slot(x);
                let r = ui.put(
                    at,
                    IconButton::new(glyph, name.clone())
                        .size(Size::Xs)
                        .min_target(button)
                        .enabled(interactive && can)
                        .id_salt(id.with(salt)),
                );
                if r.clicked() {
                    stepped = step;
                }
                x = at.left() - theme::XS / 2.0;
            }
            x -= theme::SM;
        }
        if let Some(k) = &keycap {
            let size = k.measure_on(&painter);
            let cap = Rect::from_min_size(pos2(x - size.x, cy - size.y / 2.0), size);
            k.paint_on(&painter, cap);
            x = cap.left() - theme::SM;
        } else if clears {
            // Held while the button has focus or the pointer, and through the frame Tab leaves
            // the text for it (the text has let go of focus by then), so Tab can reach it.
            let engaged_id = id.with("kit-field-clear-engaged");
            let engaged = ctx
                .data(|d| d.get_temp::<bool>(engaged_id))
                .unwrap_or(false);
            let show = filled
                && interactive
                && (self.kind == Kind::Search
                    || hovered
                    || focused
                    || response.lost_focus()
                    || engaged);
            let reveal = match self.force {
                Some(_) => f32::from(u8::from(show)),
                None => anim::ease(&ctx, id.with("kit-field-clear"), show, motion::FAST),
            };
            let at = slot(x);
            if reveal > 0.0 {
                let mut slot_ui = ui.new_child(UiBuilder::new().max_rect(at));
                slot_ui.multiply_opacity(reveal);
                let name = if self.kind == Kind::Search {
                    "Clear search"
                } else {
                    "Clear"
                };
                let clear = slot_ui.put(
                    at,
                    IconButton::new(icon::X, name)
                        .size(Size::Xs)
                        .min_target(button)
                        .id_salt(id.with("kit-field-clear-button")),
                );
                let held = clear.has_focus() || clear.hovered();
                ctx.data_mut(|d| d.insert_temp(engaged_id, held));
                if clear.clicked() {
                    self.text.clear();
                    response.mark_changed();
                    response.request_focus();
                }
            }
            x = at.left() - theme::SM;
        }
        if self.password {
            let at = slot(x);
            let (glyph, name) = if revealed {
                (icon::EYE_SLASH, "Hide secret")
            } else {
                (icon::EYE, "Show secret")
            };
            let toggle = ui.put(
                at,
                IconButton::new(glyph, name)
                    .size(Size::Xs)
                    .min_target(button)
                    .enabled(enabled)
                    .id_salt(id.with("kit-field-reveal-button")),
            );
            if toggle.clicked() {
                ctx.data_mut(|d| d.insert_temp(reveal_id, (!revealed, now)));
            }
            x = at.left() - theme::SM;
        }
        if spinner_due(&ctx, id, self.loading) {
            let r = m.icon_md / 2.0;
            paint::spinner(
                &painter,
                pos2(x - r, cy),
                r * 0.66,
                paint::SPINNER_WIDTH,
                p.text_faint,
            );
            x -= m.icon_md + theme::SM;
        }
        if let Some((g, color)) = counter {
            let color = if enabled { color } else { p.disabled_text };
            ink.galley(pos2(x - g.size().x, cy - g.size().y / 2.0), g, color);
        }

        // Helper or error below.
        let message = error
            .map(|e| (e, true))
            .or(self.helper.as_deref().map(|h| (h, false)));
        if let Some((text, is_error)) = message {
            ui.add_space(m.field_label_gap);
            let color = if is_error && enabled {
                p.danger
            } else {
                text_dim
            };
            let mut job_x = 0.0;
            let icon_g = is_error.then(|| {
                ink.layout_no_wrap(
                    icon::WARNING_CIRCLE.to_owned(),
                    FontId::proportional(m.icon_sm),
                    color,
                )
            });
            if let Some(g) = &icon_g {
                job_x = g.size().x + m.field_label_gap;
            }
            let g = ink.layout(
                text.to_owned(),
                message_font,
                color,
                (width - job_x).max(1.0),
            );
            let (row, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            if let Some(ig) = icon_g {
                let first_row = g.rows.first().map_or(g.size().y, |r| r.rect().height());
                ink.galley(
                    pos2(row.left(), row.top() + (first_row - ig.size().y) / 2.0),
                    ig,
                    color,
                );
            }
            ink.galley(pos2(row.left() + job_x, row.top()), g, color);
        }

        if let Some(label_id) = label_id {
            response = response.labelled_by(label_id);
        }
        let name = self.name.clone().or_else(|| self.label.clone());
        let read_only = self.read_only;
        let description = error.map(str::to_owned).or(self.helper.clone());
        let has_error = error.is_some();
        ctx.accesskit_node_builder(response.id, |n| {
            if let Some(name) = &name {
                n.set_label(name.clone());
            }
            if read_only {
                n.set_read_only();
            }
            if multiline {
                n.set_role(egui::accesskit::Role::MultilineTextInput);
            }
            if let Some(d) = &description {
                n.set_description(d.clone());
            }
            if has_error {
                n.set_invalid(Invalid::True);
            }
        });
        // A value wider than the well shows whole in a tooltip (never a hidden secret).
        let clipped = (enabled
            && !multiline
            && !self.password
            && galley.size().x > rect.width() - left - right + 0.5)
            .then(|| clipped_tip(self.text));
        let response = tip(
            response,
            enabled,
            self.tooltip.as_deref().filter(|_| clipped.is_none()),
            self.disabled_reason.as_deref(),
            None,
            true,
        );
        let response = match clipped {
            Some(full) => {
                let mut tooltip = response.kit_tooltip(full);
                if let Some(about) = &self.tooltip {
                    tooltip = tooltip.detail(about.clone());
                }
                tooltip.show()
            }
            None => response,
        };
        Shown {
            response,
            submitted,
            stepped,
        }
    }
}

/// A tooltip's longest value: more is cut with an ellipsis, so a pasted page is not echoed whole.
const CLIPPED_TIP_CHARS: usize = 600;

/// The text of a value too wide for its field, for its tooltip.
fn clipped_tip(value: &str) -> String {
    let mut chars = value.chars();
    let shown: String = chars.by_ref().take(CLIPPED_TIP_CHARS).collect();
    if chars.next().is_some() {
        format!("{shown}…")
    } else {
        shown
    }
}

/// Line numbers in the code area's gutter (`gutter` wide), one per logical line, right-aligned
/// and level with the text rows, with a seam between the gutter and the code.
fn paint_line_numbers(
    painter: &Painter,
    rect: Rect,
    gutter: f32,
    galley: &Galley,
    galley_pos: egui::Pos2,
    font: &FontId,
    p: &Palette,
) {
    let s = theme::surfaces(painter.ctx());
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(rect.shrink(1.0)));
    let right = rect.left() + gutter - theme::SM;
    let mut line = 1;
    let mut starts_line = true;
    for row in &galley.rows {
        if starts_line {
            let g = painter.layout_no_wrap(line.to_string(), font.clone(), p.text_faint);
            let y = galley_pos.y + row.pos.y + (row.rect().height() - g.size().y) / 2.0;
            painter.galley(pos2(right - g.size().x, y), g, p.text_faint);
            line += 1;
        }
        starts_line = row.ends_with_newline;
    }
    paint::seam_v(
        &painter,
        rect.left() + gutter,
        rect.y_range().shrink(1.0),
        s.border,
    );
}

// ----------------------------------------------------------------------------------- builders

/// The setters the fields share, as an `impl` block for `$field` with the setters it is given:
/// `common_setters!(TextArea: label, name, placeholder)`. Each field lists the setters it is used
/// with, so none is dead code; give a field another one by adding it to its list.
macro_rules! common_setters {
    ($field:ident: $($setter:ident),+ $(,)?) => {
        impl $field<'_> {
            $(common_setters!(@ $setter);)+
        }
    };
    (@ label) => {
        /// A label above the field (also its accessible name).
        pub fn label(mut self, text: impl Into<String>) -> Self {
            self.core.label = Some(text.into());
            self
        }
    };
    (@ name) => {
        /// The accessible name when no label is shown.
        pub fn name(mut self, text: impl Into<String>) -> Self {
            self.core.name = Some(text.into());
            self
        }
    };
    (@ placeholder) => {
        pub fn placeholder(mut self, text: impl Into<String>) -> Self {
            self.core.placeholder = text.into();
            self
        }
    };
    (@ helper) => {
        /// A hint below the field (hidden while there is an error).
        pub fn helper(mut self, text: impl Into<String>) -> Self {
            self.core.helper = Some(text.into());
            self
        }
    };
    (@ error) => {
        /// An error below the field with a danger edge and halo. Set it on blur or submit, never
        /// on each keystroke.
        pub fn error(mut self, text: Option<&str>) -> Self {
            self.core.error = text.map(str::to_owned);
            self
        }
    };
    (@ read_only) => {
        /// Text that can be selected and copied but not changed: flush on its surface with a
        /// quiet edge, `text_dim`, no caret.
        pub fn read_only(mut self, read_only: bool) -> Self {
            self.core.read_only = read_only;
            self
        }
    };
    (@ enabled) => {
        pub fn enabled(mut self, enabled: bool) -> Self {
            self.core.enabled = enabled;
            self
        }
    };
    (@ tooltip) => {
        pub fn tooltip(mut self, text: impl Into<String>) -> Self {
            self.core.tooltip = Some(text.into());
            self
        }
    };
    (@ disabled_reason) => {
        pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
            self.core.disabled_reason = Some(text.into());
            self
        }
    };
    (@ width) => {
        /// Field width; the available width by default.
        pub fn width(mut self, width: f32) -> Self {
            self.core.width = Some(width);
            self
        }
    };
    (@ id_salt) => {
        pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
            self.core.id_salt = Some(Id::new(salt));
            self
        }
    };
    (@ id) => {
        /// The field's own id, set by the caller (not derived from the parent `Ui`), so code
        /// elsewhere can focus it: request focus on [`edit_id`]`(id)`.
        pub fn id(mut self, id: Id) -> Self {
            self.core.id = Some(id);
            self
        }
    };
}

/// The id of the text edit inside a field whose id was set with `.id(id)`: what
/// `Memory::request_focus` takes to put the caret in it.
pub fn edit_id(id: Id) -> Id {
    id.with("kit-edit")
}

/// A one-line text input: `TextField::new(&mut name).label("Name").show(ui)`.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct TextField<'t> {
    core: Core<'t>,
}

impl<'t> TextField<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self {
            core: Core::new(text, Kind::Line),
        }
    }
    /// A leading Phosphor glyph in `text_faint` (`text_dim` while focused).
    pub fn icon(mut self, glyph: &str) -> Self {
        self.core.icon = Some(glyph.to_owned());
        self
    }
    /// Hides the text (secrets), with a button that shows it.
    pub fn password(mut self, password: bool) -> Self {
        self.core.password = password;
        self
    }
    /// A clear button at the trailing end while the field holds text and is hovered or focused.
    pub fn clearable(mut self, clearable: bool) -> Self {
        self.core.clearable = clearable;
        self
    }
    /// Busy (checking the value): a spinner in the trailing slot after `LOADING_DELAY`.
    pub fn loading(mut self, loading: bool) -> Self {
        self.core.loading = loading;
        self
    }
    /// `Sm` for compact fields; `Lg` by default.
    pub fn size(mut self, size: Size) -> Self {
        self.core.size = size;
        self
    }
    /// A right-aligned character count: `warning` past 50, `danger` past 72 (commit subject).
    pub fn counter(mut self, counter: bool) -> Self {
        self.core.counter = counter;
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.core.force = Some(force);
        self.show(ui)
    }
    pub fn show(self, ui: &mut Ui) -> Response {
        self.core.show(ui).response
    }
}

common_setters!(TextField: label, name, placeholder, helper, error, read_only, enabled, tooltip, disabled_reason, width, id_salt);

impl Widget for TextField<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// A filter or search input: magnifier, the shortcut's keycap while empty, a clear button once
/// filled; Esc clears, then lets go of focus.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct SearchField<'t> {
    core: Core<'t>,
}

impl<'t> SearchField<'t> {
    pub fn new(text: &'t mut String) -> Self {
        let mut core = Core::new(text, Kind::Search);
        // Filters sit in toolbars and lists: one size below a form field.
        core.size = Size::Md;
        Self { core }
    }
    /// Busy (results are loading): a spinner in the trailing slot after `LOADING_DELAY`.
    pub fn loading(mut self, loading: bool) -> Self {
        self.core.loading = loading;
        self
    }
    /// No fill, edge or halo: the field is part of the surface around it, at the command
    /// palette's input height with title-sized text (the palette's and cheatsheet's input over
    /// their seam).
    pub fn borderless(mut self, borderless: bool) -> Self {
        self.core.borderless = borderless;
        self
    }
    /// The command that focuses this field; its chord shows as a keycap while the field is empty.
    pub fn shortcut(mut self, command: impl Into<Shortcut>) -> Self {
        self.core.shortcut = Some(command.into());
        self
    }
    pub fn size(mut self, size: Size) -> Self {
        self.core.size = size;
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.core.force = Some(force);
        self.show(ui)
    }
    pub fn show(self, ui: &mut Ui) -> Response {
        self.core.show(ui).response
    }
}

common_setters!(SearchField: name, placeholder, read_only, width, id_salt, id);

impl Widget for SearchField<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// Multi-line text (commit description, comments) that grows with its content between a minimum
/// and maximum number of rows, then scrolls. Ctrl+Enter submits.
#[derive(Debug)]
#[must_use = "call .show(ui)"]
pub struct TextArea<'t> {
    core: Core<'t>,
}

impl<'t> TextArea<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self {
            core: Core::new(text, Kind::Area { rows: 3..=12 }),
        }
    }
    /// The rows it shows when empty, and the most it grows to (3 to 12 by default).
    pub fn rows(mut self, rows: RangeInclusive<usize>) -> Self {
        self.core.kind = Kind::Area { rows };
        self
    }
    /// A dotted guide at this column (72 for commit bodies).
    pub fn guide(mut self, column: usize) -> Self {
        self.core.guide = Some(column);
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> KitResponse<bool> {
        self.core.force = Some(force);
        self.show(ui)
    }
    /// The value is whether Ctrl+Enter submitted the text this frame.
    pub fn show(self, ui: &mut Ui) -> KitResponse<bool> {
        let shown = self.core.show(ui);
        KitResponse {
            response: shown.response,
            value: shown.submitted,
        }
    }
}

common_setters!(TextArea: label, name, placeholder, enabled, disabled_reason, id_salt);

/// A [`TextArea`] for code: monospace, Tab inserts a tab, optional line numbers (the conflict
/// hand-edit box).
#[derive(Debug)]
#[must_use = "call .show(ui)"]
pub struct CodeArea<'t> {
    core: Core<'t>,
}

impl<'t> CodeArea<'t> {
    pub fn new(text: &'t mut String) -> Self {
        Self {
            core: Core::new(
                text,
                Kind::Code {
                    rows: 3..=16,
                    line_numbers: false,
                },
            ),
        }
    }
    pub fn rows(mut self, rows: RangeInclusive<usize>) -> Self {
        if let Kind::Code { line_numbers, .. } = self.core.kind {
            self.core.kind = Kind::Code { rows, line_numbers };
        }
        self
    }
    pub fn line_numbers(mut self, on: bool) -> Self {
        if let Kind::Code { rows, .. } = &self.core.kind {
            self.core.kind = Kind::Code {
                rows: rows.clone(),
                line_numbers: on,
            };
        }
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> KitResponse<bool> {
        self.core.force = Some(force);
        self.show(ui)
    }
    /// The value is whether Ctrl+Enter submitted the text this frame.
    pub fn show(self, ui: &mut Ui) -> KitResponse<bool> {
        let shown = self.core.show(ui);
        KitResponse {
            response: shown.response,
            value: shown.submitted,
        }
    }
}

common_setters!(CodeArea: name, id_salt);

// -------------------------------------------------------------------------------- NumberField

/// What a number field keeps while it is being typed in: the text, and the error once it failed
/// to apply.
/// How a [`NumberField`] reads typed text back into a number.
type Parse<'v> = dyn Fn(&str) -> Option<f64> + 'v;

#[derive(Debug, Clone, Default)]
struct NumberDraft {
    text: String,
    error: Option<String>,
}

/// A number typed into a field, with − and + steppers at its trailing end: `NumberField::new(&mut
/// page, 200.0..=50_000.0).step(100.0).name("Commits per page").show(ui)`. What is typed applies
/// on Enter or when the field loses focus (clamped to the range, and stored as the format shows
/// it: a whole-number field rounds 12.7 to 13), Esc restores the value, and
/// text that is not a number keeps the draft with an error until it is fixed. A click selects the
/// whole value, so typing replaces it. Up and Down step while it is focused (Page Up and Page
/// Down ten steps), and each stepper turns off at its end of the range. The response reports
/// `changed` when the value changed.
#[must_use = "call .show(ui)"]
pub struct NumberField<'v> {
    value: &'v mut f64,
    range: RangeInclusive<f64>,
    step: f64,
    format: Box<dyn Fn(f64) -> String + 'v>,
    parse: Box<Parse<'v>>,
    label: Option<String>,
    name: Option<String>,
    helper: Option<String>,
    less: Option<String>,
    more: Option<String>,
    enabled: bool,
    disabled_reason: Option<String>,
    tooltip: Option<String>,
    width: Option<f32>,
    size: Size,
    id_salt: Option<Id>,
    id: Option<Id>,
    force: Option<ForceState>,
}

impl std::fmt::Debug for NumberField<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumberField")
            .field("value", &self.value)
            .field("range", &self.range)
            .field("step", &self.step)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl<'v> NumberField<'v> {
    /// A whole number in `range`, stepping by 1.
    pub fn new(value: &'v mut f64, range: RangeInclusive<f64>) -> Self {
        Self {
            value,
            range,
            step: 1.0,
            format: Box::new(|v| format!("{v:.0}")),
            parse: Box::new(|t| t.replace([',', '_', ' '], "").parse::<f64>().ok()),
            label: None,
            name: None,
            helper: None,
            less: None,
            more: None,
            enabled: true,
            disabled_reason: None,
            tooltip: None,
            width: None,
            size: Size::Lg,
            id_salt: None,
            id: None,
            force: None,
        }
    }
    /// How far a stepper click or an arrow key moves the value (no steppers when 0).
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }
    /// How the value is written (`"125%"` for a zoom of 1.25), and how typed text is read back.
    pub fn format(
        mut self,
        format: impl Fn(f64) -> String + 'v,
        parse: impl Fn(&str) -> Option<f64> + 'v,
    ) -> Self {
        self.format = Box::new(format);
        self.parse = Box::new(parse);
        self
    }
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }
    /// The accessible name when no label is shown (the steppers say "Increase `<name>`" and
    /// "Decrease `<name>`" unless [`NumberField::step_names`] names them).
    pub fn name(mut self, text: impl Into<String>) -> Self {
        self.name = Some(text.into());
        self
    }
    /// What the − and + buttons are called ("Fewer commits per page", "More commits per page").
    pub fn step_names(mut self, less: impl Into<String>, more: impl Into<String>) -> Self {
        self.less = Some(less.into());
        self.more = Some(more.into());
        self
    }
    pub fn helper(mut self, text: impl Into<String>) -> Self {
        self.helper = Some(text.into());
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
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    /// The field's own id, so code elsewhere can focus it ([`edit_id`]).
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let Self {
            value,
            range,
            step,
            format,
            parse,
            label,
            name,
            helper,
            less,
            more,
            enabled,
            disabled_reason,
            tooltip,
            width,
            size,
            id_salt,
            id,
            force,
        } = self;
        let id = id.unwrap_or_else(|| component_id(ui, "kit-field", id_salt));
        let edit = edit_id(id);
        let draft_key = id.with("kit-number-draft");
        let (lo, hi) = (*range.start(), *range.end());
        let clamp = |v: f64| v.clamp(lo, hi);
        let interactive = enabled && ui.is_enabled();
        let focused = interactive && ui.memory(|m| m.has_focus(edit));
        let before = *value;

        // Arrow keys step while the field has focus (before the edit sees them).
        if focused && step > 0.0 {
            let delta = ui.input_mut(|i| {
                let mut d = 0.0;
                for (key, n) in [
                    (Key::ArrowUp, 1.0),
                    (Key::ArrowDown, -1.0),
                    (Key::PageUp, 10.0),
                    (Key::PageDown, -10.0),
                ] {
                    if i.consume_key(Modifiers::NONE, key) {
                        d += n;
                    }
                }
                d
            });
            if delta != 0.0 {
                *value = clamp(*value + delta * step);
                ui.data_mut(|d| d.remove::<NumberDraft>(draft_key));
            }
        }

        let draft: Option<NumberDraft> = ui.data(|d| d.get_temp(draft_key));
        let mut text = draft
            .as_ref()
            .map_or_else(|| format(*value), |d| d.text.clone());
        let error = draft.as_ref().and_then(|d| d.error.clone());
        let spoken = name.clone().or_else(|| label.clone()).unwrap_or_default();
        let mut core = Core::new(&mut text, Kind::Line);
        core.label = label;
        core.name = name;
        core.helper = helper;
        core.error = error;
        core.enabled = enabled;
        core.disabled_reason = disabled_reason;
        core.tooltip = tooltip;
        core.width = width;
        core.size = size;
        core.id = Some(id);
        core.force = force;
        core.stepper = (step > 0.0).then(|| Stepper {
            less: less.unwrap_or_else(|| format!("Decrease {spoken}").trim().to_owned()),
            more: more.unwrap_or_else(|| format!("Increase {spoken}").trim().to_owned()),
            can_less: *value > lo,
            can_more: *value < hi,
        });
        let shown = core.show(ui);
        let mut response = shown.response;

        if response.gained_focus() || response.clicked() {
            // A click selects the whole value, so typing replaces it.
            if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), response.id) {
                let all = egui::text::CCursorRange::two(
                    egui::text::CCursor::new(0),
                    egui::text::CCursor::new(text.chars().count()),
                );
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), response.id);
            }
        }
        if shown.stepped != 0 {
            *value = clamp(*value + f64::from(shown.stepped) * step);
            ui.data_mut(|d| d.remove::<NumberDraft>(draft_key));
        } else if response.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                ui.data_mut(|d| d.remove::<NumberDraft>(draft_key));
            } else {
                match parse(text.trim()) {
                    Some(v) if v.is_finite() => {
                        // Stored as the field shows it: a whole-number field keeps no fraction
                        // it rounds away on screen.
                        let v = clamp(v);
                        *value = parse(&format(v)).filter(|s| s.is_finite()).map_or(v, clamp);
                        ui.data_mut(|d| d.remove::<NumberDraft>(draft_key));
                    }
                    _ => {
                        let error =
                            format!("Enter a number from {} to {}.", format(lo), format(hi));
                        ui.data_mut(|d| {
                            d.insert_temp(
                                draft_key,
                                NumberDraft {
                                    text: text.clone(),
                                    error: Some(error),
                                },
                            );
                        });
                    }
                }
            }
        } else if response.changed() {
            // Keep what is being typed (and any error until it is fixed).
            let error = draft.and_then(|d| d.error);
            ui.data_mut(|d| d.insert_temp(draft_key, NumberDraft { text, error }));
        }
        if (*value - before).abs() > f64::EPSILON {
            response.mark_changed();
        }
        let current = format(*value);
        ui.ctx().accesskit_node_builder(response.id, |n| {
            n.set_role(egui::accesskit::Role::SpinButton);
            n.set_numeric_value(*value);
            n.set_min_numeric_value(lo);
            n.set_max_numeric_value(hi);
            if step > 0.0 {
                n.set_numeric_value_step(step);
            }
            n.set_value(current.clone());
        });
        response
    }
}

impl Widget for NumberField<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's fields: every text field state, number fields with steppers, the search field
/// empty and filled, the text area with its counter and guide, and the code area.
pub(crate) fn gallery(ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = theme::MD;
    let cols = |ui: &mut Ui, f: &mut dyn FnMut(&mut Ui, usize)| {
        ui.columns(2, |cols| {
            for (i, col) in cols.iter_mut().enumerate() {
                col.spacing_mut().item_spacing.y = theme::MD;
                f(col, i);
            }
        });
    };
    gallery_caption(ui, "Text field states");
    cols(ui, &mut |ui, i| {
        if i == 0 {
            TextField::new(&mut String::new())
                .label("Rest, empty")
                .placeholder("feature/…")
                .id_salt("g-f0")
                .show(ui);
            TextField::new(&mut "feature/search".to_owned())
                .label("Hover")
                .id_salt("g-f1")
                .show_forced(ui, ForceState::HOVER);
            TextField::new(&mut "feature/search".to_owned())
                .label("Focused")
                .id_salt("g-f2")
                .show_forced(ui, ForceState::FOCUS);
            TextField::new(&mut "feature/dark mode".to_owned())
                .label("Error")
                .error(Some("Branch names cannot contain spaces."))
                .id_salt("g-f3")
                .show(ui);
        } else {
            remembered(ui, "field-live", "Ada Lovelace".to_owned(), |ui, v| {
                TextField::new(v)
                    .label("Live, with an icon, a clear button and a helper")
                    .icon(icon::USER)
                    .clearable(true)
                    .helper("Used for new commits in this repository.")
                    .id_salt("g-f4")
                    .show(ui);
            });
            TextField::new(&mut "ada@example.com".to_owned())
                .label("Disabled")
                .enabled(false)
                .disabled_reason("Set by your global Git configuration")
                .id_salt("g-f5")
                .show(ui);
            TextField::new(&mut "https://github.com/necturalabs/canopy.git".to_owned())
                .label("Read-only")
                .read_only(true)
                .tooltip("Copy it with Ctrl+C")
                .id_salt("g-f6")
                .show(ui);
            TextField::new(&mut "hunter2".to_owned())
                .label("Secret, small, checking")
                .loading(true)
                .password(true)
                .size(Size::Sm)
                .id_salt("g-f7")
                .show(ui);
        }
    });
    gallery_caption(
        ui,
        "Numbers with steppers: live, focused, disabled, at its largest, small",
    );
    cols(ui, &mut |ui, i| {
        if i == 0 {
            remembered(ui, "number-live", 2000.0_f64, |ui, v| {
                NumberField::new(v, 200.0..=50_000.0)
                    .step(100.0)
                    .label("Commits per page")
                    .step_names("Fewer commits per page", "More commits per page")
                    .helper("Up and Down step by 100; Page Up and Page Down by 1000.")
                    .tooltip("How many commits History loads at a time")
                    .width(180.0)
                    .id_salt("g-n0")
                    .show(ui);
            });
            NumberField::new(&mut 1.6, 0.8..=1.6)
                .step(0.1)
                .format(
                    |v| format!("{:.0}%", v * 100.0),
                    |t| {
                        t.trim_end_matches('%')
                            .trim()
                            .parse::<f64>()
                            .ok()
                            .map(|v| v / 100.0)
                    },
                )
                .label("Zoom, at its largest")
                .width(180.0)
                .id_salt("g-n1")
                .show(ui);
        } else {
            NumberField::new(&mut 2000.0, 200.0..=50_000.0)
                .step(100.0)
                .label("Focused")
                .width(180.0)
                .id_salt("g-n2")
                .show_forced(ui, ForceState::FOCUS);
            NumberField::new(&mut 30.0, 0.0..=120.0)
                .step(5.0)
                .label("Disabled")
                .disabled_reason("Automatic fetching is off")
                .enabled(false)
                .width(180.0)
                .id_salt("g-n3")
                .show(ui);
            NumberField::new(&mut 4.0, 1.0..=16.0)
                .name("Tab width, small")
                .size(Size::Sm)
                .width(140.0)
                .id(Id::new("gallery-number-small"))
                .show(ui);
        }
    });
    gallery_caption(
        ui,
        "Search: empty with keycap, filled with clear (live), focused",
    );
    cols(ui, &mut |ui, i| {
        if i == 0 {
            SearchField::new(&mut String::new())
                .placeholder("Filter commits, authors, SHAs…")
                .shortcut("Ctrl+F · /")
                .name("Filter commits")
                .id_salt("g-s0")
                .show(ui);
            SearchField::new(&mut String::new())
                .placeholder("Search…")
                .shortcut("Ctrl+F · /")
                .id_salt("g-s1")
                .show_forced(ui, ForceState::FOCUS);
        } else {
            remembered(ui, "search-live", "dark mode".to_owned(), |ui, v| {
                SearchField::new(v)
                    .placeholder("Filter branches…")
                    .shortcut("Ctrl+F · /")
                    .id_salt("g-s2")
                    .show(ui);
            });
            SearchField::new(&mut "auth".to_owned())
                .placeholder("Filter branches…")
                .size(Size::Sm)
                .loading(true)
                .id_salt("g-s3")
                .show(ui);
        }
    });
    gallery_caption(
        ui,
        "Search, borderless (part of the surface around it, as in the command palette)",
    );
    SearchField::new(&mut String::new())
        .placeholder("Type a command or search…")
        .borderless(true)
        .id(Id::new("gallery-search-bare"))
        .show(ui);
    gallery_caption(
        ui,
        "Borderless, focused: the line under it takes the focus colour",
    );
    SearchField::new(&mut "menu".to_owned())
        .borderless(true)
        .id(Id::new("gallery-search-bare-focus"))
        .show_forced(ui, ForceState::FOCUS);
    gallery_caption(
        ui,
        "Subject counter at 50 and 73 characters (text stops clear of it); body, idle",
    );
    cols(ui, &mut |ui, i| {
        let text = if i == 0 { 50 } else { 73 };
        TextField::new(&mut "x".repeat(text))
            .counter(true)
            .name("Subject")
            .id_salt(("g-sum", i))
            .show(ui);
    });
    remembered(
        ui,
        "area-live",
        "Explain why, not what: the diff already says what.".to_owned(),
        |ui, v| {
            TextArea::new(v)
                .rows(3..=12)
                .guide(72)
                .placeholder("Body")
                .name("Body")
                .id_salt("g-area")
                .show(ui);
        },
    );
    gallery_caption(ui, "Body with a line past column 72: the guide shows");
    TextArea::new(
        &mut "Retry with backoff: the mobile clients drop the first request after a resume, so the \
              fetch now waits and tries again."
            .to_owned(),
    )
    .rows(3..=12)
    .guide(72)
    .name("Body, long")
    .id_salt("g-area-long")
    .show(ui);
    gallery_caption(ui, "Code area with line numbers (live), focused");
    cols(ui, &mut |ui, i| {
        if i == 0 {
            remembered(
                ui,
                "code-live",
                "Self {\n    attempts: 5,\n    backoff_ms: 500,\n}".to_owned(),
                |ui, v| {
                    CodeArea::new(v)
                        .rows(3..=8)
                        .line_numbers(true)
                        .name("Resolved text")
                        .id_salt("g-code")
                        .show(ui);
                },
            );
        } else {
            CodeArea::new(&mut "fn main() {}".to_owned())
                .rows(3..=3)
                .name("Focused code")
                .id_salt("g-code-f")
                .show_forced(ui, ForceState::FOCUS);
            TextArea::new(&mut "Looks good to me.".to_owned())
                .rows(2..=2)
                .label("Comment, hover")
                .id_salt("g-area-h")
                .show_forced(ui, ForceState::HOVER);
            TextArea::new(&mut String::new())
                .rows(2..=2)
                .placeholder("Leave a comment")
                .enabled(false)
                .disabled_reason("Comments need a hosting account")
                .id_salt("g-area-d")
                .show(ui);
        }
    });
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;

    use super::super::testkit::{self, find, frame, key, key_with, spoken};
    use super::*;

    fn type_text(text: &str) -> Vec<egui::Event> {
        vec![egui::Event::Text(text.to_owned())]
    }

    #[test]
    fn fields_follow_the_metrics_in_every_density_and_style() {
        for density in theme::Density::ALL {
            for style in Style::ALL {
                let ctx = testkit::ctx_with(theme::ThemeId::Daylight, style, density);
                let m = theme::metrics();
                let mut heights = Vec::new();
                frame(&ctx, Vec::new(), |ui| {
                    heights.push(TextField::new(&mut String::new()).show(ui).rect.height());
                    heights.push(SearchField::new(&mut String::new()).show(ui).rect.height());
                    heights.push(NumberField::new(&mut 1.0, 0.0..=9.0).show(ui).rect.height());
                });
                assert_eq!(
                    heights,
                    vec![m.control_lg, m.control, m.control_lg],
                    "{density:?} {style:?}"
                );
            }
        }
    }

    /// O10: a field is a well a step off every surface it can sit on, every text colour reads
    /// on it there (the placeholder is `text_faint`: 4.5:1, 5.5:1 strict), and its resting edge
    /// traces the well on every surface without the weight of an input outline; high contrast
    /// keeps the outline (4.5:1). In every theme.
    #[test]
    fn a_field_is_a_readable_well_with_a_quiet_accessible_edge_in_every_theme() {
        for (id, high) in theme::ThemeId::ALL
            .into_iter()
            .flat_map(|t| [(t, false), (t, true)])
        {
            let p = id.shown_palette(high);
            let (text_min, dim_min, faint_min, rim_min) = targets(p);
            let rim_min = if p.strict { rim_min } else { QUIET_EDGE_MIN };
            for style in Style::ALL {
                let l = look(p, style);
                for (i, ground) in grounds(p, style).into_iter().enumerate() {
                    let at = format!("{} high contrast {high} {style:?} ground {i}", p.name);
                    for wash in [l.wash, l.hover_wash] {
                        let well = theme::over(wash, ground);
                        assert!(theme::contrast(p.text, well) >= text_min, "{at}: text");
                        assert!(
                            theme::contrast(p.text_dim, well) >= dim_min,
                            "{at}: text_dim"
                        );
                        assert!(
                            theme::contrast(p.text_faint, well) >= faint_min,
                            "{at}: placeholder"
                        );
                    }
                    // At rest; hovered, the whole edge turns `control_border`.
                    let well = theme::over(l.wash, ground);
                    for against in [ground, well] {
                        assert!(
                            theme::contrast(l.base, against) >= rim_min,
                            "{at}: the edge traces the field"
                        );
                    }
                    if p.dark {
                        assert!(
                            theme::contrast(well, ground) > 1.05,
                            "{at}: the well is a step off its surface ({:.3})",
                            theme::contrast(well, ground)
                        );
                    } else {
                        // A light theme's plate is as light as any surface, so it stands off the
                        // darker ones and, on the lightest, by its edge (checked above).
                        assert!(
                            theme::luminance(well) >= theme::luminance(ground) - 1e-9,
                            "{at}: the plate is never darker than its surface"
                        );
                    }
                    assert!(
                        theme::contrast(l.side, ground) <= theme::contrast(l.base, ground) + 0.01,
                        "{at}: a read-only edge is no louder than a field waiting for input"
                    );
                    assert!(
                        theme::contrast(l.base, ground)
                            <= theme::contrast(p.control_border, ground) + 0.01,
                        "{at}: rest is no louder than hover"
                    );
                }
            }
        }
    }

    #[test]
    fn typing_edits_and_disabled_or_read_only_does_not() {
        let ctx = testkit::ctx();
        for (enabled, read_only, expect) in
            [(true, false, "ab"), (false, false, "a"), (true, true, "a")]
        {
            let mut text = "a".to_owned();
            let show = |ui: &mut Ui, text: &mut String| {
                TextField::new(text)
                    .enabled(enabled)
                    .read_only(read_only)
                    .id_salt("t")
                    .show(ui)
                    .id
            };
            let mut id = Id::NULL;
            frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
            ctx.memory_mut(|m| m.request_focus(id));
            frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
            frame(&ctx, type_text("b"), |ui| id = show(ui, &mut text));
            assert_eq!(text, expect, "enabled {enabled}, read-only {read_only}");
        }
    }

    /// The flat fill colours a frame painted (rects, also inside `Shape::Vec`).
    fn fills(shapes: &[egui::epaint::ClippedShape]) -> Vec<Color32> {
        fn walk(shape: &Shape, out: &mut Vec<Color32>) {
            match shape {
                Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                Shape::Rect(r) if r.fill.a() > 0 => out.push(r.fill),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for c in shapes {
            walk(&c.shape, &mut out);
        }
        out
    }

    #[test]
    fn disabled_and_read_only_fields_do_not_look_editable() {
        let ctx = testkit::ctx();
        let p = theme::palette(&ctx);
        let s = p.s();
        let l = look(p, Style::Standard);
        let painted = |enabled: bool, read_only: bool| {
            let mut text = "value".to_owned();
            fills(&frame(&ctx, Vec::new(), |ui| {
                TextField::new(&mut text)
                    .enabled(enabled)
                    .read_only(read_only)
                    .show(ui);
            }))
        };
        assert!(
            painted(true, false).contains(&l.wash),
            "an editable field is a well"
        );
        let disabled = painted(false, false);
        assert!(
            disabled.contains(&s.disabled_fill),
            "disabled: the disabled fill"
        );
        assert!(!disabled.contains(&l.wash));
        assert!(
            painted(true, true).is_empty(),
            "read-only: no well, flush on its surface"
        );
    }

    #[test]
    fn a_clearable_field_offers_to_clear_only_when_it_holds_text() {
        let ctx = testkit::ctx();
        for (text, offered) in [("", false), ("feature/x", true)] {
            let mut text = text.to_owned();
            let nodes = spoken(&ctx, |ui| {
                TextField::new(&mut text)
                    .clearable(true)
                    .id_salt("c")
                    .show_forced(ui, ForceState::FOCUS);
            });
            assert_eq!(find(&nodes, "Clear").is_some(), offered, "{text:?}");
        }
    }

    #[test]
    fn escape_clears_a_search_then_lets_go() {
        let ctx = testkit::ctx();
        let mut text = "auth".to_owned();
        let show = |ui: &mut Ui, text: &mut String| SearchField::new(text).id_salt("s").show(ui).id;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
        frame(&ctx, key(Key::Escape), |ui| id = show(ui, &mut text));
        assert_eq!(text, "");
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
        assert!(ctx.memory(|m| m.has_focus(id)), "the first Esc keeps focus");
        frame(&ctx, key(Key::Escape), |ui| id = show(ui, &mut text));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
        assert!(!ctx.memory(|m| m.has_focus(id)), "the second Esc lets go");
    }

    #[test]
    fn ctrl_enter_submits_a_text_area() {
        let ctx = testkit::ctx();
        let mut text = "body".to_owned();
        let show = |ui: &mut Ui, text: &mut String| {
            let r = TextArea::new(text).id_salt("a").show(ui);
            (r.response.id, r.value)
        };
        let (mut id, mut submitted) = (Id::NULL, false);
        frame(&ctx, Vec::new(), |ui| (id, _) = show(ui, &mut text));
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| (id, _) = show(ui, &mut text));
        frame(&ctx, key_with(Key::Enter, Modifiers::COMMAND), |ui| {
            (id, submitted) = show(ui, &mut text);
        });
        assert!(submitted);
        assert_eq!(text, "body", "Ctrl+Enter adds no newline");
    }

    #[test]
    fn a_text_area_grows_with_its_content_within_its_rows() {
        let ctx = testkit::ctx();
        let height = |text: &str| {
            let mut text = text.to_owned();
            let mut h = 0.0;
            for _ in 0..3 {
                frame(&ctx, Vec::new(), |ui| {
                    h = TextArea::new(&mut text)
                        .rows(2..=4)
                        .id_salt("grow")
                        .show(ui)
                        .response
                        .rect
                        .height();
                });
            }
            h
        };
        let two = height("");
        let four = height("1\n2\n3\n4");
        let many = height("1\n2\n3\n4\n5\n6\n7\n8");
        assert!(four > two, "{four} > {two}");
        assert!(
            (many - four).abs() < 1.0,
            "stops growing at the last row: {many} vs {four}"
        );
    }

    #[test]
    fn fields_are_named_and_errors_are_announced() {
        let ctx = testkit::ctx();
        let mut text = "feature/dark mode".to_owned();
        let nodes = spoken(&ctx, |ui| {
            TextField::new(&mut text)
                .label("Branch name")
                .error(Some("Branch names cannot contain spaces."))
                .show(ui);
        });
        let field = nodes
            .iter()
            .find(|n| n.role == Role::TextInput)
            .expect("a text input");
        assert_eq!(field.label.as_deref(), Some("Branch name"));
        let mut code = String::new();
        let nodes = spoken(&ctx, |ui| {
            CodeArea::new(&mut code).name("Resolved text").show(ui);
        });
        assert_eq!(
            find(&nodes, "Resolved text").map(|n| n.role),
            Some(Role::MultilineTextInput)
        );
    }

    /// The dashes a frame painted (the column guide is the only dashed line a text area draws).
    fn dashes(shapes: &[egui::epaint::ClippedShape]) -> usize {
        shapes
            .iter()
            .filter(|c| matches!(c.shape, Shape::LineSegment { .. }))
            .count()
    }

    #[test]
    fn the_column_guide_shows_only_while_writing_or_when_a_line_reaches_it() {
        let ctx = testkit::ctx();
        let mut short = "Explain why".to_owned();
        let shapes = frame(&ctx, Vec::new(), |ui| {
            TextArea::new(&mut short).guide(72).show(ui);
        });
        assert_eq!(
            dashes(&shapes),
            0,
            "an idle short description shows no guide"
        );
        let mut long = "x".repeat(90);
        let shapes = frame(&ctx, Vec::new(), |ui| {
            ui.set_width(2000.0);
            TextArea::new(&mut long).guide(72).show(ui);
        });
        assert!(
            dashes(&shapes) > 0,
            "a line past the column shows the guide"
        );
        let mut focused = "Explain why".to_owned();
        let shapes = frame(&ctx, Vec::new(), |ui| {
            TextArea::new(&mut focused)
                .guide(72)
                .show_forced(ui, ForceState::FOCUS);
        });
        assert!(dashes(&shapes) > 0, "writing shows the guide");
    }

    #[test]
    fn text_that_runs_past_a_field_stops_clear_of_its_counter() {
        let ctx = testkit::ctx();
        let mut text = "x".repeat(80);
        let shapes = frame(&ctx, Vec::new(), |ui| {
            ui.set_width(300.0);
            TextField::new(&mut text).counter(true).show(ui);
        });
        let text_shape = |wanted: &dyn Fn(&str) -> bool| {
            shapes.iter().find_map(|c| match &c.shape {
                Shape::Text(t) if wanted(t.galley.text()) => Some((c.clip_rect, t.pos.x)),
                _ => None,
            })
        };
        let (_, counter_left) = text_shape(&|s| s == "80").expect("the counter is painted");
        let (clip, _) = text_shape(&|s| s.starts_with("xxxx")).expect("the text is painted");
        assert!(
            clip.right() < counter_left,
            "the text is cut at {} before the counter at {counter_left}",
            clip.right()
        );
    }

    #[test]
    fn a_field_with_a_caller_set_id_can_be_focused_from_elsewhere() {
        let ctx = testkit::ctx();
        let id = Id::new("history-filter");
        let mut text = String::new();
        let mut focused = false;
        for i in 0..3 {
            if i == 1 {
                ctx.memory_mut(|m| m.request_focus(edit_id(id)));
            }
            frame(&ctx, Vec::new(), |ui| {
                // Nested in a scope, so an id derived from the parent `Ui` would differ.
                ui.scope(|ui| {
                    focused = SearchField::new(&mut text).id(id).show(ui).has_focus();
                });
            });
        }
        assert!(focused);
    }

    /// KF-F9: a field placed straight into a right-to-left row (a settings row's control) sits
    /// at the row's right at its own width, as a select does, with its label and message.
    #[test]
    fn a_field_in_a_right_to_left_row_sits_at_the_right() {
        let ctx = testkit::ctx();
        for labelled in [false, true] {
            let mut text = "main".to_owned();
            let (mut field, mut row) = (Rect::NOTHING, Rect::NOTHING);
            for _ in 0..2 {
                frame(&ctx, Vec::new(), |ui| {
                    ui.set_width(600.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        row = ui.max_rect();
                        let mut f = TextField::new(&mut text).width(200.0).id_salt("rtl");
                        if labelled {
                            f = f.label("Branch").helper("The default branch");
                        }
                        field = f.show(ui).rect;
                    });
                });
            }
            assert!(
                (field.right() - row.right()).abs() < 0.5,
                "labelled {labelled}: the field ends at {} in a row ending at {}",
                field.right(),
                row.right()
            );
        }
    }

    #[test]
    fn a_borderless_search_paints_no_field_or_border() {
        let ctx = testkit::ctx();
        let well = look(theme::palette(&ctx), Style::Standard).wash;
        let painted = |bare: bool| {
            let shapes = frame(&ctx, Vec::new(), |ui| {
                SearchField::new(&mut String::new())
                    .borderless(bare)
                    .show(ui);
            });
            fills(&shapes).contains(&well)
                || shapes.iter().any(|c| match &c.shape {
                    Shape::Rect(r) => r.stroke.width > 0.0,
                    Shape::Path(p) => p.stroke.width > 0.0,
                    _ => false,
                })
        };
        assert!(painted(false));
        assert!(!painted(true));
    }

    /// usersim2 N4: a borderless search (the palette's and the cheatsheet's input) has no box to
    /// ring, so focus, by Tab or by a click, turns the line under it to the focus colour
    /// (docs/design.md: a ring for keyboard and pointer alike).
    #[test]
    fn a_borderless_search_shows_focus_by_keyboard_and_pointer() {
        for by_pointer in [false, true] {
            let ctx = testkit::ctx();
            let focus = theme::palette(&ctx).focus;
            let mut text = "menu".to_owned();
            let mut show = |ctx: &egui::Context, events: Vec<egui::Event>| {
                let mut rect = Rect::NOTHING;
                let shapes = frame(ctx, events, |ui| {
                    rect = SearchField::new(&mut text)
                        .borderless(true)
                        .id_salt("surface")
                        .show(ui)
                        .rect;
                });
                (fills(&shapes), rect)
            };
            let (_, rect) = show(&ctx, Vec::new());
            let at = rect.left_center() + vec2(40.0, 0.0);
            assert!(
                !show(&ctx, Vec::new()).0.contains(&focus),
                "unfocused, nothing is ringed"
            );
            let events = if by_pointer {
                testkit::press(at)
            } else {
                key(Key::Tab)
            };
            show(&ctx, events);
            if by_pointer {
                show(&ctx, testkit::release(at));
            }
            let seen = show(&ctx, Vec::new()).0;
            assert!(
                seen.contains(&focus),
                "focus (by pointer: {by_pointer}) shows in the focus colour: {seen:?}"
            );
        }
    }

    #[test]
    fn unsalted_fields_keep_their_own_state() {
        let ctx = testkit::ctx();
        let (mut a, mut b) = (String::new(), String::new());
        let show = |ui: &mut Ui, a: &mut String, b: &mut String| {
            let first = TextField::new(a).show(ui).id;
            let second = TextField::new(b).show(ui).id;
            (first, second)
        };
        let mut ids = (Id::NULL, Id::NULL);
        frame(&ctx, Vec::new(), |ui| ids = show(ui, &mut a, &mut b));
        assert_ne!(ids.0, ids.1);
        ctx.memory_mut(|m| m.request_focus(ids.1));
        frame(&ctx, Vec::new(), |ui| ids = show(ui, &mut a, &mut b));
        frame(&ctx, type_text("x"), |ui| ids = show(ui, &mut a, &mut b));
        assert_eq!((a.as_str(), b.as_str()), ("", "x"));
    }

    #[test]
    fn a_secret_can_be_shown_and_hidden_again() {
        let ctx = testkit::ctx();
        let mut text = "hunter2".to_owned();
        let show = |ui: &mut Ui, text: &mut String| {
            TextField::new(text)
                .password(true)
                .width(240.0)
                .id_salt("p")
                .show(ui)
                .rect
        };
        let mut rect = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| rect = show(ui, &mut text));
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut text);
        });
        assert!(
            find(&nodes, "Show secret").is_some(),
            "the toggle offers to show it"
        );
        let m = theme::metrics();
        let at = pos2(
            rect.right() - m.field_pad_x - theme::MIN_TARGET / 2.0,
            rect.center().y,
        );
        testkit::click(&ctx, at, |ui| {
            show(ui, &mut text);
        });
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut text);
        });
        assert!(
            find(&nodes, "Hide secret").is_some(),
            "a click shows the secret"
        );
        assert_eq!(text, "hunter2", "the toggle never edits the text");
        // RK-6: the next prompt with the same field starts hidden again.
        frame(&ctx, Vec::new(), |_| {});
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut text);
        });
        assert!(
            find(&nodes, "Show secret").is_some(),
            "a field shown again after it was gone hides its secret"
        );
    }

    #[test]
    fn focus_halo_is_instant_with_reduced_motion() {
        let ctx = testkit::ctx();
        let p = theme::palette(&ctx);
        let halo = Palette::tint(p.focus, FOCUS_HALO_ALPHA);
        let mut text = String::new();
        let strokes = |ctx: &egui::Context, text: &mut String| {
            let mut id = Id::NULL;
            let shapes = frame(ctx, Vec::new(), |ui| {
                id = TextField::new(text).id_salt("h").show(ui).id;
            });
            let painted = shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Rect(r) => Some((r.stroke.color, r.stroke.width)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            (id, painted)
        };
        let (id, _) = strokes(&ctx, &mut text);
        ctx.memory_mut(|m| m.request_focus(id));
        let (_, painted) = strokes(&ctx, &mut text);
        assert!(
            painted.contains(&(halo, HALO)),
            "the focused field shows its whole halo on the first frame: {painted:?}"
        );
    }

    /// A focused field's halo takes the theme's focus style: high contrast's solid ring past a gap,
    /// and the edges come in the theme's edge width.
    #[test]
    fn the_focus_halo_takes_the_themes_focus_style() {
        let strokes = |theme: theme::ThemeId| {
            let ctx = testkit::ctx_with(theme, Style::Standard, theme::Density::Comfortable);
            let mut text = String::new();
            let mut id = Id::NULL;
            let _ = frame(&ctx, Vec::new(), |ui| {
                id = TextField::new(&mut text).id_salt("h").show(ui).id;
            });
            ctx.memory_mut(|m| m.request_focus(id));
            let shapes = frame(&ctx, Vec::new(), |ui| {
                TextField::new(&mut text).id_salt("h").show(ui);
            });
            let p = theme::active_palette();
            shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Rect(r) if r.stroke.width > 0.0 => {
                        Some((r.stroke.color == p.focus, r.stroke.width))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        theme::set_high_contrast(true);
        let contrast = strokes(theme::ThemeId::Midnight);
        theme::set_high_contrast(false);
        assert!(
            contrast.contains(&(true, 2.5)),
            "a solid 2.5 pt focus ring: {contrast:?}"
        );
    }

    /// WCAG 2.4.7 and 1.4.11: a focused field turns its whole edge to the focus colour, which
    /// stands 3:1 (4.5:1 strict) from every surface and well, whether it took focus from the
    /// keyboard or the pointer.
    #[test]
    fn a_focused_field_shows_a_focus_colour_edge_for_keyboard_and_pointer() {
        for (id, high) in theme::ThemeId::ALL
            .into_iter()
            .flat_map(|t| [(t, false), (t, true)])
        {
            let p = id.shown_palette(high);
            let (_, _, _, rim_min) = targets(p);
            for style in Style::ALL {
                let l = look(p, style);
                for ground in grounds(p, style) {
                    let well = theme::over(l.wash, ground);
                    for against in [ground, well] {
                        assert!(
                            theme::contrast(p.focus, against) >= rim_min,
                            "{} {high} {style:?}: the focus edge",
                            p.name
                        );
                    }
                }
            }
        }
        let edge_in_focus = |how: &dyn Fn(&egui::Context, Rect, &mut String)| {
            let ctx = testkit::ctx();
            let p = theme::palette(&ctx);
            let mut text = String::new();
            let mut rect = Rect::NOTHING;
            frame(&ctx, Vec::new(), |ui| {
                rect = TextField::new(&mut text).id_salt("f").show(ui).rect;
            });
            how(&ctx, rect, &mut text);
            let shapes = frame(&ctx, Vec::new(), |ui| {
                TextField::new(&mut text).id_salt("f").show(ui);
            });
            shapes.iter().any(|c| {
                matches!(&c.shape, Shape::Rect(r) if r.stroke.color == p.focus && r.stroke.width > 0.0)
            })
        };
        let by_pointer = |ctx: &egui::Context, rect: Rect, text: &mut String| {
            testkit::click(ctx, rect.center(), |ui| {
                TextField::new(text).id_salt("f").show(ui);
            });
        };
        let by_keyboard = |ctx: &egui::Context, _: Rect, text: &mut String| {
            frame(ctx, key(Key::Tab), |ui| {
                TextField::new(text).id_salt("f").show(ui);
            });
        };
        assert!(edge_in_focus(&by_pointer), "pointer focus shows the edge");
        assert!(edge_in_focus(&by_keyboard), "keyboard focus shows the edge");
    }

    /// WCAG 3.3.1 and 4.1.2: an error is said in words and an icon under the field, and the
    /// field is announced invalid with the message as its description, never by colour alone.
    /// No error, or an empty one, shows and announces nothing.
    #[test]
    fn an_error_is_words_and_an_icon_and_announced() {
        let ctx = testkit::ctx();
        let message = "Branch names cannot contain spaces.";
        let painted = |error: Option<&str>| {
            let mut text = "a b".to_owned();
            let shapes = frame(&ctx, Vec::new(), |ui| {
                TextField::new(&mut text).error(error).id_salt("e").show(ui);
            });
            let has = |wanted: &str| shapes.iter().any(|c| paints(&c.shape, wanted));
            (has(message), has(icon::WARNING_CIRCLE))
        };
        assert_eq!(painted(Some(message)), (true, true), "words and an icon");
        assert_eq!(painted(None), (false, false), "none before there is one");
        assert_eq!(
            painted(Some("")),
            (false, false),
            "an empty message is none"
        );
        let announced = |error: Option<&str>| {
            let mut text = "a b".to_owned();
            let nodes = spoken(&ctx, |ui| {
                TextField::new(&mut text)
                    .label("Name")
                    .error(error)
                    .id_salt("e")
                    .show(ui);
            });
            let node = nodes
                .into_iter()
                .find(|n| n.role == Role::TextInput)
                .expect("a text input");
            (node.invalid, node.description)
        };
        assert_eq!(announced(Some(message)), (true, Some(message.to_owned())));
        assert_eq!(announced(None), (false, None));
        assert_eq!(announced(Some("")), (false, None));
    }

    /// Whether `shape` paints exactly `text`.
    fn paints(shape: &Shape, text: &str) -> bool {
        match shape {
            Shape::Text(t) => t.galley.text() == text,
            Shape::Vec(v) => v.iter().any(|s| paints(s, text)),
            _ => false,
        }
    }

    /// WCAG 2.5.8: the buttons in a field (clear, reveal the secret, the steppers) answer a
    /// pointer over at least 24 points square, in every density.
    #[test]
    fn the_buttons_in_a_field_have_a_target_of_24_points() {
        for density in theme::Density::ALL {
            let ctx = testkit::ctx_with(theme::ThemeId::Midnight, Style::Standard, density);
            let mut search = "auth".to_owned();
            let mut secret = "hunter2".to_owned();
            let mut number = 300.0;
            let nodes = spoken(&ctx, |ui| {
                SearchField::new(&mut search).id_salt("s").show(ui);
                TextField::new(&mut secret)
                    .password(true)
                    .id_salt("p")
                    .show(ui);
                NumberField::new(&mut number, 100.0..=900.0)
                    .step_names("Fewer", "More")
                    .id_salt("n")
                    .show(ui);
            });
            for name in ["Clear search", "Show secret", "Fewer", "More"] {
                let bounds = find(&nodes, name)
                    .and_then(|n| n.bounds)
                    .unwrap_or_else(|| panic!("{name} is in its field"));
                assert!(
                    bounds.width() >= theme::MIN_TARGET - 0.01
                        && bounds.height() >= theme::MIN_TARGET - 0.01,
                    "{density:?}: {name} is {:?}",
                    bounds.size()
                );
            }
        }
    }

    /// A clear button that shows only while the field is hovered or focused stays while Tab
    /// moves focus onto it, so a keyboard user can press it.
    #[test]
    fn the_clear_button_can_be_reached_with_tab() {
        let ctx = testkit::ctx();
        let mut text = "feature/x".to_owned();
        let show = |ui: &mut Ui, text: &mut String| {
            TextField::new(text)
                .clearable(true)
                .id_salt("c")
                .show(ui)
                .id
        };
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut text));
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| _ = show(ui, &mut text));
        frame(&ctx, key(Key::Tab), |ui| _ = show(ui, &mut text));
        for _ in 0..3 {
            frame(&ctx, Vec::new(), |ui| _ = show(ui, &mut text));
        }
        assert!(!ctx.memory(|m| m.has_focus(id)), "focus left the text");
        let nodes = spoken(&ctx, |ui| _ = show(ui, &mut text));
        assert!(find(&nodes, "Clear").is_some(), "the button it moved onto");
        frame(&ctx, key(Key::Enter), |ui| _ = show(ui, &mut text));
        assert_eq!(text, "", "Enter on the focused button clears the field");
    }

    /// Cut-off text: a value wider than its field shows whole in a tooltip (at the default
    /// Helpful level), a value that fits does not, and a secret never does.
    #[test]
    fn a_value_wider_than_its_field_shows_whole_in_a_tooltip() {
        let long = "a-branch-name-much-longer-than-the-field-it-is-typed-into/with/more";
        // The field paints its own text once; a tooltip is a second painting.
        let painted = |value: &str, password: bool| {
            let mut text = value.to_owned();
            testkit::painted_after_hover(pos2(60.0, 16.0), value, |ui| {
                TextField::new(&mut text)
                    .password(password)
                    .width(120.0)
                    .id_salt("t")
                    .show(ui);
            })
        };
        assert_eq!(painted(long, false), 2, "the field's text and the tooltip");
        assert_eq!(painted("main", false), 1, "a value that fits has none");
        assert_eq!(
            painted(long, true),
            0,
            "a secret is neither shown nor repeated"
        );
    }

    /// Cut-off values in a tooltip stop at a sensible length.
    #[test]
    fn a_very_long_value_is_cut_in_its_tooltip() {
        assert_eq!(clipped_tip("short"), "short");
        let long = "x".repeat(CLIPPED_TIP_CHARS + 50);
        let tip = clipped_tip(&long);
        assert_eq!(tip.chars().count(), CLIPPED_TIP_CHARS + 1);
        assert!(tip.ends_with('…'));
    }

    // ------------------------------------------------------------------------ NumberField

    fn number(ui: &mut Ui, v: &mut f64) -> Response {
        NumberField::new(v, 200.0..=1000.0)
            .step(100.0)
            .name("Commits per page")
            .id_salt("n")
            .show(ui)
    }

    #[test]
    fn steppers_move_the_value_and_stop_at_the_ends_of_the_range() {
        let ctx = testkit::ctx();
        let mut v = 900.0;
        let mut rect = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| rect = number(ui, &mut v).rect);
        let nodes = spoken(&ctx, |ui| {
            number(ui, &mut v);
        });
        assert!(find(&nodes, "Increase Commits per page").is_some());
        assert!(find(&nodes, "Decrease Commits per page").is_some());
        // + is the trailing button, inset by XS.
        let plus = pos2(
            rect.right() - theme::XS - Size::Xs.height() / 2.0,
            rect.center().y,
        );
        for _ in 0..3 {
            testkit::click(&ctx, plus, |ui| {
                number(ui, &mut v);
            });
        }
        assert!((v - 1000.0).abs() < f64::EPSILON, "clamped at the top: {v}");
        let nodes = spoken(&ctx, |ui| {
            number(ui, &mut v);
        });
        // On Linux an unavailable button says so in its name (a11y::unavailable).
        let more = nodes
            .iter()
            .find(|n| {
                matches!(
                    n.label.as_deref(),
                    Some("Increase Commits per page" | "Increase Commits per page, unavailable")
                )
            })
            .expect("the + button");
        assert!(more.disabled, "+ turns off at the top of the range");
        let minus = plus - vec2(Size::Xs.height() + theme::XS / 2.0, 0.0);
        testkit::click(&ctx, minus, |ui| {
            number(ui, &mut v);
        });
        assert!((v - 900.0).abs() < f64::EPSILON, "− steps down: {v}");
    }

    #[test]
    fn typed_numbers_apply_on_enter_and_wrong_ones_keep_an_error() {
        let ctx = testkit::ctx();
        let mut v = 400.0;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = number(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = number(ui, &mut v).id);
        // Focus selected the whole value, so typing replaces it.
        frame(&ctx, type_text("5000"), |ui| id = number(ui, &mut v).id);
        frame(&ctx, key(Key::Enter), |ui| id = number(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = number(ui, &mut v).id);
        assert!(
            (v - 1000.0).abs() < f64::EPSILON,
            "applied and clamped: {v}"
        );

        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = number(ui, &mut v).id);
        frame(&ctx, type_text("lots"), |ui| id = number(ui, &mut v).id);
        frame(&ctx, key(Key::Enter), |ui| id = number(ui, &mut v).id);
        let nodes = spoken(&ctx, |ui| {
            number(ui, &mut v);
        });
        assert!(
            (v - 1000.0).abs() < f64::EPSILON,
            "a wrong number changes nothing"
        );
        assert!(
            nodes.iter().any(|n| n.role == Role::SpinButton),
            "spoken as a spin button"
        );
        let shapes = frame(&ctx, Vec::new(), |ui| {
            number(ui, &mut v);
        });
        let said = shapes.iter().any(|c| match &c.shape {
            Shape::Text(t) => t
                .galley
                .text()
                .starts_with("Enter a number from 200 to 1000"),
            _ => false,
        });
        assert!(said, "the error says what to type");
    }

    /// RK-12: a typed number is stored as the field shows it: a whole-number field never keeps
    /// the fraction it rounds away on screen.
    #[test]
    fn a_typed_number_is_stored_as_it_is_shown() {
        let ctx = testkit::ctx();
        let mut v = 400.0;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = number(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = number(ui, &mut v).id);
        let replace = |text: &str| {
            let mut events = key_with(Key::A, Modifiers::COMMAND);
            events.extend(type_text(text));
            events
        };
        frame(&ctx, replace("512.7"), |ui| id = number(ui, &mut v).id);
        frame(&ctx, key(Key::Enter), |ui| id = number(ui, &mut v).id);
        assert!((v - 513.0).abs() < f64::EPSILON, "stored as shown: {v}");
        let mut zoom = 1.0;
        let percent = |ui: &mut Ui, z: &mut f64| {
            NumberField::new(z, 0.5..=2.0)
                .step(0.1)
                .format(
                    |z| format!("{:.0}%", z * 100.0),
                    |t| {
                        t.trim_end_matches('%')
                            .parse::<f64>()
                            .ok()
                            .map(|p| p / 100.0)
                    },
                )
                .id_salt("zoom")
                .show(ui)
        };
        frame(&ctx, Vec::new(), |ui| id = percent(ui, &mut zoom).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = percent(ui, &mut zoom).id);
        frame(&ctx, replace("125%"), |ui| id = percent(ui, &mut zoom).id);
        frame(&ctx, key(Key::Enter), |ui| id = percent(ui, &mut zoom).id);
        assert!(
            (zoom - 1.25).abs() < 1e-9,
            "finer than the step when shown so: {zoom}"
        );
    }

    #[test]
    fn arrow_keys_step_a_focused_number() {
        let ctx = testkit::ctx();
        let mut v = 500.0;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = number(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = number(ui, &mut v).id);
        let mut changed = false;
        frame(&ctx, key(Key::ArrowUp), |ui| {
            changed = number(ui, &mut v).changed();
        });
        assert!(changed);
        assert!((v - 600.0).abs() < f64::EPSILON, "Up steps up: {v}");
        frame(&ctx, key(Key::PageDown), |ui| {
            number(ui, &mut v);
        });
        assert!(
            (v - 200.0).abs() < f64::EPSILON,
            "Page Down, ten steps, clamped: {v}"
        );
    }
}
