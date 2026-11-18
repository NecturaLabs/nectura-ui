//! Rows: [`ListRow`], [`TreeRow`] with its animated disclosure, [`Disclosure`],
//! [`SectionHeader`], and [`Selection`] for Shift and Ctrl clicks (design spec 4.16).
//!
//! Rows snap: selection changes at once and the hover ground follows the pointer within `FAST`.
//! Selection is a translucent wash with the theme's mark over it (`Design::selection`: a leading
//! bar, a ring or an inner glow); the keyboard cursor on an unselected row, and keyboard focus on
//! any row, is an inset 1.5 pt focus ring. Every row is focusable and activates with Enter or Space.

use std::collections::BTreeSet;
use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, FontFamily, FontId, Id, Key, Layout, Modifiers, Painter,
    Rect, Response, Sense, Shape, Ui, UiBuilder, Widget, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::button::IconButton;
use super::chip::Badge;
use super::overlay::TooltipExt;
use super::{ForceState, KitResponse, Size, Tone, anim, focus_visible, paint};
use crate::motion;
use crate::theme::{self, Palette};

/// Which list a row belongs to: its height, ground and label treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RowKind {
    /// Lists in panes (`row_height`), rounded.
    #[default]
    List,
    /// File rows in the changes and commit lists (`row_height`), rounded.
    File,
    /// History table rows (`history_row_height`): full-span, zebra in Dense.
    History,
    /// Sidebar items (`sidebar_row_height`) on the chrome: dim labels, the strong hover,
    /// semibold when selected.
    Sidebar,
    /// Pull and merge request rows (`request_row_height`).
    Request,
    /// Command palette and cheatsheet rows (`palette_row_height`).
    Palette,
}

impl RowKind {
    /// The row height in the active metrics.
    pub fn height(self) -> f32 {
        let m = theme::metrics();
        match self {
            Self::List | Self::File => m.row_height,
            Self::History => m.history_row_height,
            Self::Sidebar => m.sidebar_row_height,
            Self::Request => m.request_row_height,
            Self::Palette => m.palette_row_height,
        }
    }

    /// Tables span the full width with square ends; lists are rounded.
    fn rounded(self) -> bool {
        self != Self::History
    }

    fn chrome(self) -> bool {
        self == Self::Sidebar
    }

    /// The surface the row sits on by default.
    fn ground(self, p: &Palette) -> Color32 {
        let s = p.s();
        if self.chrome() { s.panel } else { s.pane }
    }
}

/// The opaque colour of a row's background: its ground with the zebra, selection, hover and
/// press overlays on top. The graph paints node rings and avatar rings in it.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "zebra, selection, hover and press are independent overlays"
)]
pub fn ground(
    p: &Palette,
    base: Color32,
    zebra: bool,
    selected: bool,
    hover: f32,
    pressed: bool,
    strong_hover: bool,
) -> Color32 {
    let mut c = base;
    if zebra {
        c = theme::over(p.zebra(), c);
    }
    if selected {
        c = theme::over(p.selected(), c);
    }
    if hover > 0.0 {
        let h = if strong_hover {
            p.hover_strong()
        } else {
            p.hover()
        };
        c = theme::over(h.gamma_multiply(hover), c);
    }
    if pressed {
        c = theme::over(p.pressed(), c);
    }
    c
}

/// Whether row `index` (0-based) of a history table gets the zebra stripe: even rows counting
/// from one, in Dense only.
pub fn zebra(index: usize) -> bool {
    theme::metrics().zebra && index % 2 == 1
}

/// What a row's content paints with: its rect, the content rect inside the padding, and the
/// colours of its state.
#[derive(Debug, Clone, Copy)]
pub struct RowCtx {
    pub rect: Rect,
    /// `rect` less the horizontal padding.
    pub content: Rect,
    /// Hover, 0 to 1.
    pub hover: f32,
    pub selected: bool,
    /// The opaque background colour (for rings drawn on the row).
    pub ground: Color32,
    /// Label colour.
    pub text: Color32,
    /// Secondary text colour.
    pub dim: Color32,
    /// Icon colour: `text_faint` at rest, `text` on hover, `accent` when selected.
    pub icon: Color32,
    /// SHA and other accentuated trailing text: `text_faint`, `accent` when selected.
    pub accent: Color32,
}

/// Horizontal padding inside a row.
pub const PAD: f32 = 10.0;
/// Gap between a row's icon, label, detail and trailing text.
const GAP: f32 = 8.0;
/// A disclosure chevron's column: its inset from the row's left edge, its width, and the gap
/// after it in a tree row (a disclosure's body starts at their sum, under the header's icon or,
/// without one, its label).
const CHEVRON_INSET: f32 = 6.0;
const CHEVRON_WIDTH: f32 = 12.0;
const CHEVRON_GAP: f32 = 4.0;
/// The gap after a section header's chevron.
const SECTION_CHEVRON_GAP: f32 = 6.0;

/// Where the icon of a sidebar section's item sits from the row's left edge: one tree level right
/// of the section header's icon, so the header reads as the items' parent.
fn section_item_x() -> f32 {
    CHEVRON_INSET + CHEVRON_WIDTH + SECTION_CHEVRON_GAP + theme::metrics().tree_indent
}

/// The [`ListRow::indent`] of a sidebar section's item. The selection and hover fills keep the
/// row's full width; only the content moves.
pub fn section_item_indent() -> f32 {
    section_item_x() - PAD
}

/// The [`TreeRow::indent`] of a sidebar section's top-level row (a remote): its label lines up
/// with the section's other items.
pub fn section_tree_indent() -> f32 {
    section_item_x() - CHEVRON_INSET - CHEVRON_WIDTH - CHEVRON_GAP
}

/// Everything rows share up to their content: allocation, interaction, the ground layers.
struct Frame {
    rect: Rect,
    response: Response,
    painter: Painter,
    ctx: RowCtx,
    radius: CornerRadius,
    cursor: bool,
    /// Keyboard focus is on the row and shown: the ring is drawn on a selected row too.
    focused: bool,
    enabled: bool,
}

struct FrameSpec {
    id_salt: Id,
    kind: RowKind,
    height: f32,
    selected: bool,
    cursor: bool,
    zebra: bool,
    enabled: bool,
    joined: (bool, bool),
    ground: Option<Color32>,
    read_only: bool,
    /// Whether Tab stops on the row (false: a palette result, reached by the arrows only).
    focusable: bool,
    /// The list's single Tab stop and the row's place in it.
    roving: Option<(Roving, usize)>,
    force: Option<ForceState>,
}

fn frame(ui: &mut Ui, spec: &FrameSpec) -> Frame {
    let c = ui.ctx().clone();
    let p = theme::palette(&c);
    let m = theme::metrics();
    let enabled = spec.enabled && ui.is_enabled();
    let mut width = ui.available_width();
    if !width.is_finite() {
        width = theme::metrics().sidebar_width;
    }
    let rect = ui.allocate_space(vec2(width, spec.height)).1;
    let id = ui.id().with(spec.id_salt);
    // Register even disabled rows: a roving arrow must skip them, not target an inert row.
    if let Some((list, index)) = spec.roving {
        list.register(&c, index, enabled);
    }
    // In a list with one Tab stop only that row (or the one with focus) takes focus from Tab.
    let tab_stop = enabled
        && match spec.roving {
            Some((list, index)) => list.takes_tab(ui.ctx(), index, Some(id)),
            None => spec.focusable,
        };
    // A read-only row takes focus (to be read, or copied from) but no clicks, hover or press.
    let live = enabled && !spec.read_only;
    let sense = match (live, tab_stop) {
        (true, true) => Sense::click(),
        (true, false) => Sense::CLICK,
        (false, true) => Sense::focusable_noninteractive(),
        (false, false) => Sense::hover(),
    };
    let response = ui.interact(rect, id, sense);
    let f = spec.force.unwrap_or_default();
    // The pointer anywhere over the row (its trailing actions included) hovers it.
    let hovered = live && (f.hover || ui.rect_contains_pointer(rect));
    let pressed = live && (f.pressed || response.is_pointer_button_down_on());
    let hover = match spec.force {
        Some(f) => f32::from(u8::from(hovered)) * f.progress.unwrap_or(1.0),
        None => anim::ease_bool(
            &c,
            response.id.with("kit-row-hover"),
            hovered,
            motion::FAST,
            motion::FAST,
        ),
    };
    let focused = f.focus || (response.has_focus() && focus_visible(&c));
    let cursor = spec.cursor || focused;
    let base = spec.ground.unwrap_or_else(|| spec.kind.ground(p));
    let strong = spec.kind.chrome();
    let ground = ground(p, base, spec.zebra, spec.selected, hover, pressed, strong);
    let r = if spec.kind.rounded() { m.radius_sm } else { 0 };
    let radius = CornerRadius {
        nw: if spec.joined.0 { 0 } else { r },
        ne: if spec.joined.0 { 0 } else { r },
        sw: if spec.joined.1 { 0 } else { r },
        se: if spec.joined.1 { 0 } else { r },
    };
    let clip = rect.intersect(ui.clip_rect());
    // A disabled row's grounds dim; its content stays readable in `disabled_text`.
    let painter = if enabled {
        ui.painter().with_clip_rect(clip)
    } else {
        super::content_painter(ui).with_clip_rect(clip)
    };
    if ui.is_rect_visible(rect) {
        let mut ground_painter = painter.clone();
        if !enabled {
            ground_painter.multiply_opacity(theme::DISABLED_ALPHA);
        }
        let painter = &ground_painter;
        if spec.zebra {
            painter.rect_filled(rect, radius, p.zebra());
        }
        if spec.selected {
            painter.rect_filled(rect, radius, p.selected());
        }
        if hover > 0.0 {
            let h = if strong { p.hover_strong() } else { p.hover() };
            painter.rect_filled(rect, radius, h.gamma_multiply(hover));
            if !spec.selected {
                paint::hover_mark(painter, rect, radius, p, p.s(), hover);
            }
        }
        if pressed {
            painter.rect_filled(rect, radius, p.pressed());
        }
        if spec.selected {
            paint::selection_mark(painter, rect, radius, p);
        }
    }
    let lit = hover.max(f32::from(u8::from(spec.selected)));
    let rest_text = if strong { p.text_dim } else { p.text };
    let ctx = if enabled {
        RowCtx {
            rect,
            content: rect.shrink2(vec2(PAD, 0.0)),
            hover,
            selected: spec.selected,
            ground,
            text: rest_text.lerp_to_gamma(p.text, lit),
            dim: p.text_dim,
            icon: if spec.selected {
                p.accent
            } else {
                p.text_faint.lerp_to_gamma(p.text, hover)
            },
            accent: if spec.selected {
                p.accent
            } else {
                p.text_faint
            },
        }
    } else {
        let off = p.disabled_text;
        RowCtx {
            rect,
            content: rect.shrink2(vec2(PAD, 0.0)),
            hover,
            selected: spec.selected,
            ground,
            text: off,
            dim: off,
            icon: off,
            accent: off,
        }
    };
    Frame {
        rect,
        response,
        painter,
        ctx,
        radius,
        cursor,
        focused,
        enabled,
    }
}

impl Frame {
    /// The keyboard cursor ring on an unselected row, and the focus ring on any row, drawn over
    /// the content.
    fn finish(&self) {
        if self.cursor && (!self.ctx.selected || self.focused) {
            let p = theme::palette(self.painter.ctx());
            paint::inset_ring(&self.painter, self.rect, self.radius, p.focus);
        }
    }
}

/// The most of a row's width a [`ListRow::whole_detail`] may take from its label.
const WHOLE_DETAIL_SHARE: f32 = 0.6;

/// Paints `text` on one line from `x`, ellipsized at `right`; returns where it ends.
fn paint_text(
    painter: &Painter,
    x: f32,
    right: f32,
    cy: f32,
    text: &str,
    font: FontId,
    color: Color32,
) -> f32 {
    if right <= x {
        return x;
    }
    let mut job = LayoutJob::simple(text.to_owned(), font, color, right - x);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let g = painter.layout_job(job);
    let w = g.size().x;
    painter.galley(pos2(x, cy - g.size().y / 2.0), g, color);
    x + w
}

fn row_widget_info(
    ui: &Ui,
    response: &Response,
    enabled: bool,
    selected: bool,
    current: bool,
    label: &str,
    role: egui::accesskit::Role,
) {
    response.widget_info(|| {
        WidgetInfo::selected(WidgetType::SelectableLabel, enabled, selected, label)
    });
    // A list or tree item is selected, not toggled.
    ui.ctx().accesskit_node_builder(response.id, |n| {
        n.set_role(role);
        n.clear_toggled();
        n.set_selected(selected);
        if current {
            n.set_aria_current(egui::accesskit::AriaCurrent::True);
        }
    });
}

fn with_tip(
    response: Response,
    enabled: bool,
    tooltip: Option<&str>,
    reason: Option<&str>,
) -> Response {
    let tip = if enabled { tooltip } else { reason.or(tooltip) };
    // A row can be as wide as the window: its tooltip follows the pointer along it.
    match tip {
        Some(t) => response.kit_tooltip(t).at_pointer().show(),
        None => response,
    }
}

/// Whether the widget with keyboard focus lay inside `area` of `ui`'s layer last frame (a row's
/// actions, which have no ids of their own the row knows).
fn focus_within(ui: &Ui, area: Rect) -> bool {
    let ctx = ui.ctx();
    ctx.memory(egui::Memory::focused)
        .and_then(|id| ctx.read_response(id))
        .is_some_and(|r| r.layer_id == ui.layer_id() && area.contains_rect(r.rect))
}

// ------------------------------------------------------------------------------------ ListRow

/// A row's hover-revealed actions.
type Actions<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

/// A row of a list or table. Content comes from slots (`icon`, `label`, `detail`, `trailing`)
/// or a painter closure ([`ListRow::show_with`]); hover-revealed actions from
/// [`ListRow::actions`].
#[must_use = "call .show(ui), .show_with(ui, ..) or ui.add(..)"]
pub struct ListRow<'a> {
    id_salt: Id,
    kind: RowKind,
    height: Option<f32>,
    selected: bool,
    current: bool,
    cursor: bool,
    zebra: bool,
    enabled: bool,
    joined: (bool, bool),
    ground: Option<Color32>,
    icon: Option<String>,
    label: String,
    accessible: Option<String>,
    detail: Option<String>,
    whole_detail: bool,
    trailing: Option<String>,
    badge: Option<Badge>,
    widgets: Option<(f32, Actions<'a>)>,
    read_only: bool,
    focusable: bool,
    roving: Option<(Roving, usize)>,
    indent: f32,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    actions: Option<Actions<'a>>,
    force: Option<ForceState>,
}

impl std::fmt::Debug for ListRow<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListRow")
            .field("id_salt", &self.id_salt)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl<'a> ListRow<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        Self {
            id_salt: Id::new(id_salt),
            kind: RowKind::List,
            height: None,
            selected: false,
            current: false,
            cursor: false,
            zebra: false,
            enabled: true,
            joined: (false, false),
            ground: None,
            icon: None,
            label: String::new(),
            accessible: None,
            detail: None,
            whole_detail: false,
            trailing: None,
            badge: None,
            widgets: None,
            read_only: false,
            focusable: true,
            roving: None,
            indent: 0.0,
            tooltip: None,
            disabled_reason: None,
            actions: None,
            force: None,
        }
    }

    /// The list the row belongs to: height, ground and label treatment.
    pub fn kind(mut self, kind: RowKind) -> Self {
        self.kind = kind;
        self
    }
    /// The spec's name for [`ListRow::kind`].
    pub fn height_from(self, kind: RowKind) -> Self {
        self.kind(kind)
    }
    /// A height of its own instead of the kind's (a two-line row).
    pub fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }
    /// What screen readers call the row when it differs from the painted label, or when the
    /// row paints its own content ([`ListRow::show_with`]) and has no label slot. The row
    /// reports itself once, under this name.
    pub fn accessible_name(mut self, name: impl Into<String>) -> Self {
        self.accessible = Some(name.into());
        self
    }
    /// A badge at the right, before the trailing text (a count, a status).
    pub fn trailing_badge(mut self, badge: Badge) -> Self {
        self.badge = Some(badge);
        self
    }
    /// Widgets at the right edge, always shown (unlike [`ListRow::actions`]), in a slot `width`
    /// wide laid out right to left.
    pub fn trailing_ui(mut self, width: f32, add: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.widgets = Some((width, Box::new(add)));
        self
    }
    /// The row can be focused and read but not clicked: no hover, no press.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
    /// Whether Tab stops on the row (default on). Off for rows the arrows reach from a field that
    /// keeps focus (the palette's results).
    pub fn focusable(mut self, focusable: bool) -> Self {
        self.focusable = focusable;
        self
    }
    /// The row is row `index` of a list with one Tab stop ([`Roving`]): Tab stops on one row of
    /// the list and the arrows move between its rows.
    pub fn roving(mut self, list: Roving, index: usize) -> Self {
        self.roving = Some((list, index));
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    /// The row is the current one of its kind (the checked-out branch), apart from the
    /// selection: its icon in the accent and its label weighted as if selected, and announced
    /// as current.
    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }
    /// The keyboard cursor is on this row (views with their own cursor model).
    pub fn cursor(mut self, cursor: bool) -> Self {
        self.cursor = cursor;
        self
    }
    /// The row's position in its table, for Dense's zebra stripes.
    pub fn index(mut self, index: usize) -> Self {
        self.zebra = self.kind == RowKind::History && zebra(index);
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// The rows above and below are selected too: square the shared corners so the selection
    /// reads as one block.
    pub fn joined(mut self, above: bool, below: bool) -> Self {
        self.joined = (above, below);
        self
    }
    /// The surface under the row when it is not the kind's usual one.
    pub fn ground(mut self, ground: Color32) -> Self {
        self.ground = Some(ground);
        self
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    /// The label, also the row's AccessKit name.
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }
    pub fn detail(mut self, text: impl Into<String>) -> Self {
        self.detail = Some(text.into());
        self
    }
    /// The detail is cut last: when the row is too narrow, the label gives way to it (up to
    /// `WHOLE_DETAIL_SHARE` of the row).
    pub fn whole_detail(mut self) -> Self {
        self.whole_detail = true;
        self
    }
    /// Right-aligned mono text (a count, a SHA): `text_faint`, `accent` when selected.
    pub fn trailing(mut self, text: impl Into<String>) -> Self {
        self.trailing = Some(text.into());
        self
    }
    /// Extra left indent for the content.
    pub fn indent(mut self, indent: f32) -> Self {
        self.indent = indent;
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
    /// Actions shown at the right edge while the row is hovered, selected or focused (they fade
    /// in over `FAST`). Add `IconButton::size(Size::Sm)`s right to left.
    pub fn actions(mut self, add: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.actions = Some(Box::new(add));
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    fn spec(&self) -> FrameSpec {
        FrameSpec {
            id_salt: self.id_salt,
            kind: self.kind,
            height: self.height.unwrap_or_else(|| self.kind.height()),
            selected: self.selected,
            cursor: self.cursor,
            zebra: self.zebra,
            enabled: self.enabled,
            joined: self.joined,
            ground: self.ground,
            read_only: self.read_only,
            focusable: self.focusable,
            roving: self.roving,
            force: self.force,
        }
    }

    /// Shows the row with its slots.
    pub fn show(self, ui: &mut Ui) -> Response {
        self.show_with(ui, |_, _| {})
    }

    /// Shows the row, then `paint` draws custom content (after the slots, under the cursor ring).
    pub fn show_with(mut self, ui: &mut Ui, paint: impl FnOnce(&RowCtx, &Painter)) -> Response {
        let m = theme::metrics();
        let f = frame(ui, &self.spec());
        if let Some((list, index)) = self.roving
            && f.enabled
        {
            list.row(ui, &f.response, index, false);
        }
        let rc = f.ctx;
        let cy = rc.rect.center().y;
        let actions = self.actions.take();
        let widgets = self.widgets.take();
        let actions_w = if actions.is_some() {
            (m.control_sm + 2.0) * 2.0
        } else {
            0.0
        };
        let widgets_w = widgets.as_ref().map_or(0.0, |(w, _)| *w + GAP);
        if ui.is_rect_visible(f.rect) {
            let mut x = rc.content.left() + self.indent;
            let mut right = rc.content.right() - actions_w - widgets_w;
            if let Some(g) = &self.icon {
                let color = if self.current && f.enabled {
                    Tone::Accent.color(theme::palette(ui.ctx()))
                } else {
                    rc.icon
                };
                let glyph = f.painter.layout_no_wrap(
                    g.clone(),
                    theme::icon_font(m.icon_md, rc.selected || self.current),
                    color,
                );
                f.painter.galley(
                    pos2(x, cy - glyph.size().y / 2.0),
                    Arc::clone(&glyph),
                    color,
                );
                x += glyph.size().x + GAP;
            }
            if let Some(t) = &self.trailing {
                let g = f.painter.layout_no_wrap(
                    t.clone(),
                    FontId::new(m.sha_size, FontFamily::Monospace),
                    rc.accent,
                );
                right -= g.size().x;
                f.painter
                    .galley(pos2(right, cy - g.size().y / 2.0), g, rc.accent);
                right -= GAP;
            }
            if let Some(b) = &self.badge {
                let size = b.measure(ui);
                right -= size.x;
                b.paint(
                    ui,
                    Rect::from_min_size(pos2(right, cy - size.y / 2.0), size),
                );
                right -= GAP;
            }
            let family = match (self.selected || self.current, self.kind.chrome()) {
                (true, true) => theme::semibold(),
                (true, false) => theme::medium(),
                _ => FontFamily::Proportional,
            };
            let detail_room = self
                .detail
                .as_ref()
                .filter(|_| self.whole_detail)
                .map_or(0.0, |d| {
                    f.painter
                        .layout_no_wrap(d.clone(), FontId::proportional(m.meta_size), rc.dim)
                        .size()
                        .x
                        + GAP
                })
                // Past this the label would vanish: the detail is cut then.
                .min(((right - x) * WHOLE_DETAIL_SHARE).max(0.0));
            let end = paint_text(
                &f.painter,
                x,
                right - detail_room,
                cy,
                &self.label,
                FontId::new(m.body_size, family),
                rc.text,
            );
            if let Some(d) = &self.detail {
                paint_text(
                    &f.painter,
                    end + GAP,
                    right,
                    cy,
                    d,
                    FontId::proportional(m.meta_size),
                    rc.dim,
                );
            }
            paint(&rc, &f.painter);
        }
        f.finish();
        if let Some((width, add)) = widgets {
            let area = Rect::from_min_max(
                pos2(rc.content.right() - actions_w - width, f.rect.top()),
                pos2(rc.content.right() - actions_w, f.rect.bottom()),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(area)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            if !f.enabled {
                child.disable();
            }
            child.spacing_mut().item_spacing.x = theme::XS;
            add(&mut child);
        }
        if let Some(add) = actions {
            let area = Rect::from_min_max(
                pos2(rc.content.right() - actions_w, f.rect.top()),
                pos2(rc.content.right(), f.rect.bottom()),
            );
            // Tab goes from the row to its actions: they stay while one of them has focus, or
            // the focus they hold would vanish with them (A11Y-2).
            let action_focused = focus_within(ui, area);
            let shown = f.enabled
                && (rc.hover > 0.0
                    || self.selected
                    || f.response.has_focus()
                    || f.cursor
                    || action_focused);
            let reveal = if action_focused {
                1.0
            } else {
                anim::ease_bool(
                    ui.ctx(),
                    f.response.id.with("kit-row-actions"),
                    shown,
                    motion::FAST,
                    motion::FAST,
                )
            };
            // The child is made whether or not the actions show, so the widgets after the row
            // keep their automatic ids (and their focus) as the actions come and go.
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(area)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            if reveal > 0.0 {
                child.multiply_opacity(reveal);
                child.spacing_mut().item_spacing.x = 2.0;
                add(&mut child);
            }
        }
        row_widget_info(
            ui,
            &f.response,
            f.enabled,
            self.selected,
            self.current,
            self.accessible.as_deref().unwrap_or(&self.label),
            egui::accesskit::Role::ListItem,
        );
        with_tip(
            f.response,
            f.enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
        )
    }
}

impl Widget for ListRow<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------ TreeRow

/// Where the keyboard asked to move from a tree row: Left on a closed row goes to its parent,
/// Right on an open one into its first child. The view moves its cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeNav {
    Parent,
    FirstChild,
}

/// A row in a tree: a [`ListRow`] with an indent per level and a disclosure chevron that turns
/// over `BASE` (space is kept for it on leaves). Left collapses or asks for the parent; Right
/// expands or asks for the first child.
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct TreeRow<'a> {
    id_salt: Id,
    depth: usize,
    open: Option<&'a mut bool>,
    kind: RowKind,
    icon: Option<String>,
    icon_tone: Option<Tone>,
    label: String,
    accessible: Option<String>,
    trailing: Option<String>,
    indent: f32,
    selected: bool,
    cursor: bool,
    enabled: bool,
    toggle_on_click: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    roving: Option<(Roving, usize)>,
    force: Option<ForceState>,
}

impl std::fmt::Debug for TreeRow<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeRow")
            .field("id_salt", &self.id_salt)
            .field("depth", &self.depth)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<'a> TreeRow<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug, depth: usize) -> Self {
        Self {
            id_salt: Id::new(id_salt),
            depth,
            open: None,
            kind: RowKind::Sidebar,
            icon: None,
            icon_tone: None,
            label: String::new(),
            accessible: None,
            trailing: None,
            indent: 0.0,
            selected: false,
            cursor: false,
            enabled: true,
            toggle_on_click: false,
            tooltip: None,
            disabled_reason: None,
            roving: None,
            force: None,
        }
    }
    /// The row is row `index` of a tree with one Tab stop ([`Roving`]).
    pub fn roving(mut self, list: Roving, index: usize) -> Self {
        self.roving = Some((list, index));
        self
    }
    /// Colours the icon by `tone` (a CI check's red, amber or green).
    pub fn icon_tone(mut self, tone: Tone) -> Self {
        self.icon_tone = Some(tone);
        self
    }
    /// Shown on hover (a truncated name in full, what the row stands for).
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    /// Why the row is disabled, shown as its tooltip while it is.
    pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
        self.disabled_reason = Some(text.into());
        self
    }
    /// The row has children, shown while `open`.
    pub fn open(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }
    pub fn kind(mut self, kind: RowKind) -> Self {
        self.kind = kind;
        self
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = text.into();
        self
    }
    /// What screen readers call the row when the painted label has more than words in it (a
    /// remote's name with its host's icon glyph).
    pub fn accessible_name(mut self, name: impl Into<String>) -> Self {
        self.accessible = Some(name.into());
        self
    }
    pub fn trailing(mut self, text: impl Into<String>) -> Self {
        self.trailing = Some(text.into());
        self
    }
    /// Extra left indent for the content (the fills keep the full width), on top of the depth's.
    pub fn indent(mut self, indent: f32) -> Self {
        self.indent = indent;
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
    pub fn cursor(mut self, cursor: bool) -> Self {
        self.cursor = cursor;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// A click anywhere on the row opens or closes it (disclosures, group rows).
    pub fn toggle_on_click(mut self, on: bool) -> Self {
        self.toggle_on_click = on;
        self
    }
    pub(crate) fn show_forced(
        mut self,
        ui: &mut Ui,
        force: ForceState,
    ) -> KitResponse<Option<TreeNav>> {
        self.force = Some(force);
        self.show(ui)
    }

    pub fn show(self, ui: &mut Ui) -> KitResponse<Option<TreeNav>> {
        let m = theme::metrics();
        let p = theme::palette(ui.ctx());
        let spec = FrameSpec {
            id_salt: self.id_salt,
            kind: self.kind,
            height: self.kind.height(),
            selected: self.selected,
            cursor: self.cursor,
            zebra: false,
            enabled: self.enabled,
            joined: (false, false),
            ground: None,
            read_only: false,
            focusable: true,
            roving: self.roving,
            force: self.force,
        };
        let f = frame(ui, &spec);
        let rc = f.ctx;
        let id = f.response.id;
        let has_children = self.open.is_some();
        let mut open_now = self.open.as_deref().copied().unwrap_or(false);
        let chevron_x = f.rect.left()
            + CHEVRON_INSET
            + self.indent
            + crate::count_f32(self.depth) * m.tree_indent;
        let chevron = Rect::from_center_size(
            pos2(chevron_x + CHEVRON_WIDTH / 2.0, f.rect.center().y),
            vec2(16.0, f.rect.height()),
        );
        let mut toggled = false;
        if has_children && f.enabled {
            // The pointer's target only: the row itself opens and closes from the keyboard, so
            // the chevron is no Tab stop and no unnamed node of its own (A11Y-5, A11Y-6).
            let hit = ui.interact(chevron, id.with("kit-tree-chevron"), Sense::CLICK);
            if hit.clicked() {
                toggled = true;
            }
        }
        if has_children
            && f.enabled
            && ((self.toggle_on_click && f.response.clicked()) || f.response.double_clicked())
        {
            toggled = true;
        }
        let mut nav = None;
        if let Some((list, index)) = self.roving
            && f.enabled
        {
            list.row(ui, &f.response, index, true);
        }
        if f.response.has_focus() && f.enabled {
            ui.memory_mut(|mem| {
                mem.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: self.roving.is_some(),
                        ..Default::default()
                    },
                );
            });
            let (right, left) = ui.input_mut(|i| {
                (
                    i.consume_key(Modifiers::NONE, Key::ArrowRight),
                    i.consume_key(Modifiers::NONE, Key::ArrowLeft),
                )
            });
            if right {
                if has_children && !open_now {
                    toggled = true;
                } else if has_children {
                    nav = Some(TreeNav::FirstChild);
                }
            }
            if left {
                if has_children && open_now {
                    toggled = true;
                } else {
                    nav = Some(TreeNav::Parent);
                }
            }
        }
        if toggled {
            open_now = !open_now;
        }
        if let Some(open) = self.open {
            *open = open_now;
        }
        let turn = match self.force.and_then(|f| f.progress) {
            Some(t) if has_children => {
                if open_now {
                    t
                } else {
                    1.0 - t
                }
            }
            _ => anim::ease_bool(
                ui.ctx(),
                id.with("kit-tree-turn"),
                open_now,
                motion::BASE,
                motion::BASE,
            ),
        };
        if ui.is_rect_visible(f.rect) {
            let cy = f.rect.center().y;
            if has_children {
                let color = if f.enabled {
                    p.text_faint.lerp_to_gamma(p.text_dim, rc.hover)
                } else {
                    rc.dim
                };
                paint_chevron(&f.painter, chevron.center(), turn, color);
            }
            let mut x = chevron_x + CHEVRON_WIDTH + CHEVRON_GAP;
            if let Some(g) = &self.icon {
                let color = match self.icon_tone {
                    Some(t) if f.enabled => t.color(p),
                    _ => rc.icon,
                };
                let glyph = f.painter.layout_no_wrap(
                    g.clone(),
                    theme::icon_font(m.icon_md, rc.selected),
                    color,
                );
                f.painter.galley(
                    pos2(x, cy - glyph.size().y / 2.0),
                    Arc::clone(&glyph),
                    color,
                );
                x += glyph.size().x + GAP;
            }
            let mut right = rc.content.right();
            if let Some(t) = &self.trailing {
                let color = if f.enabled { p.text_faint } else { rc.dim };
                let g = f.painter.layout_no_wrap(
                    t.clone(),
                    FontId::new(m.micro_size, FontFamily::Monospace),
                    color,
                );
                right -= g.size().x;
                f.painter
                    .galley(pos2(right, cy - g.size().y / 2.0), g, color);
                right -= GAP;
            }
            let family = match (self.selected, self.kind.chrome()) {
                (true, true) => theme::semibold(),
                (true, false) => theme::medium(),
                _ => FontFamily::Proportional,
            };
            paint_text(
                &f.painter,
                x,
                right,
                cy,
                &self.label,
                FontId::new(m.body_size, family),
                rc.text,
            );
        }
        f.finish();
        let mut response = f.response;
        if toggled {
            response.mark_changed();
        }
        let spoken = self.accessible.as_deref().unwrap_or(&self.label);
        row_widget_info(
            ui,
            &response,
            f.enabled,
            self.selected,
            false,
            spoken,
            egui::accesskit::Role::TreeItem,
        );
        if has_children {
            let name = super::a11y::disclosure_name(spoken, open_now);
            ui.ctx().accesskit_node_builder(response.id, |n| {
                n.set_expanded(open_now);
                n.set_label(name);
            });
        }
        let response = with_tip(
            response,
            f.enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
        );
        KitResponse {
            response,
            value: nav,
        }
    }
}

/// The disclosure chevron (`CARET_RIGHT`, 12 pt) at `center`, turned by `turn` quarter turns.
fn paint_chevron(painter: &Painter, center: egui::Pos2, turn: f32, color: Color32) {
    let g = painter.layout_no_wrap(
        icon::CARET_RIGHT.to_owned(),
        FontId::proportional(12.0),
        color,
    );
    let pos = center - g.size() / 2.0;
    let shape = egui::epaint::TextShape::new(pos, g, color)
        .with_angle_and_anchor(turn * std::f32::consts::FRAC_PI_2, Align2::CENTER_CENTER);
    painter.add(Shape::Text(shape));
}

/// The children of an open tree row (or disclosure): they grow open and shut over `BASE`,
/// except lists of more than 40 and with reduced motion, which snap. Returns what `add`
/// returned while any of them shows.
pub fn tree_children<R>(
    ui: &mut Ui,
    id: Id,
    open: bool,
    count: usize,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let openness = if count > 40 {
        f32::from(u8::from(open))
    } else {
        anim::ease_bool(
            ui.ctx(),
            id.with("kit-tree-body"),
            open,
            motion::BASE,
            motion::BASE,
        )
    };
    if openness <= 0.0 {
        return None;
    }
    let key = id.with("kit-tree-body-height");
    let full: Option<f32> = ui.ctx().data(|d| d.get_temp(key));
    let top = ui.cursor().top();
    let width = ui.available_width();
    let shown_h = match full {
        Some(h) if openness < 1.0 => h * openness,
        _ => f32::INFINITY,
    };
    let max_rect = Rect::from_min_size(pos2(ui.cursor().left(), top), vec2(width, f32::INFINITY));
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(id.with("kit-tree-body-ui"))
            .max_rect(max_rect)
            .layout(Layout::top_down(Align::Min)),
    );
    if shown_h.is_finite() {
        let clip = Rect::from_min_size(
            pos2(ui.max_rect().left(), top),
            vec2(ui.max_rect().width(), shown_h),
        );
        child.set_clip_rect(clip.intersect(ui.clip_rect()));
    }
    let r = add(&mut child);
    let measured = child.min_rect().height();
    ui.ctx().data_mut(|d| d.insert_temp(key, measured));
    let height = if openness < 1.0 {
        measured * openness
    } else {
        measured
    };
    ui.allocate_rect(
        Rect::from_min_size(
            pos2(max_rect.left(), top),
            vec2(child.min_rect().width(), height),
        ),
        Sense::hover(),
    );
    Some(r)
}

// --------------------------------------------------------------------------------- Disclosure

/// A titled section that opens and closes (the commit information block, toast and SSH details,
/// CI checks): a [`TreeRow`] header that toggles on click, then its body.
#[must_use = "call .show(ui, ..)"]
pub struct Disclosure<'a> {
    id_salt: Id,
    label: String,
    open: &'a mut bool,
    icon: Option<String>,
    icon_tone: Option<Tone>,
}

impl std::fmt::Debug for Disclosure<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Disclosure")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<'a> Disclosure<'a> {
    pub fn new(
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        label: impl Into<String>,
        open: &'a mut bool,
    ) -> Self {
        Self {
            id_salt: Id::new(id_salt),
            label: label.into(),
            open,
            icon: None,
            icon_tone: None,
        }
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    /// Colours the header's icon by `tone` (the checks header's red, amber or green).
    pub fn icon_tone(mut self, tone: Tone) -> Self {
        self.icon_tone = Some(tone);
        self
    }
    pub fn show<R>(self, ui: &mut Ui, body: impl FnOnce(&mut Ui) -> R) -> KitResponse<Option<R>> {
        let id = ui.id().with(self.id_salt);
        let mut open = *self.open;
        let mut row = TreeRow::new(self.id_salt, 0)
            .kind(RowKind::List)
            .label(self.label.clone())
            .toggle_on_click(true)
            .open(&mut open);
        if let Some(g) = &self.icon {
            row = row.icon(g);
        }
        if let Some(t) = self.icon_tone {
            row = row.icon_tone(t);
        }
        let r = row.show(ui);
        *self.open = open;
        // A disclosure is a collapsing header, not an item of a tree.
        let name = super::a11y::disclosure_name(&self.label, open);
        r.response
            .widget_info(|| WidgetInfo::labeled(WidgetType::CollapsingHeader, true, &name));
        ui.ctx().accesskit_node_builder(r.response.id, |n| {
            n.clear_selected();
            n.set_expanded(open);
            n.set_label(name.as_str());
        });
        let value = tree_children(ui, id, open, 0, |ui| {
            ui.add_space(theme::XS);
            let r = ui
                .horizontal(|ui| {
                    ui.add_space(CHEVRON_INSET + CHEVRON_WIDTH + CHEVRON_GAP);
                    ui.vertical(body).inner
                })
                .inner;
            ui.add_space(theme::XS);
            r
        });
        KitResponse {
            response: r.response,
            value,
        }
    }
}

// ------------------------------------------------------------------------------ SectionHeader

/// A sidebar section's header: an uppercase label with an optional glyph, chevron, count and an
/// action that shows on hover. Click (or Enter) opens and closes a collapsible one.
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct SectionHeader<'a> {
    title: String,
    icon: Option<String>,
    count: Option<usize>,
    open: Option<&'a mut bool>,
    action: Option<(String, String)>,
    first: bool,
    force: Option<ForceState>,
}

impl std::fmt::Debug for SectionHeader<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SectionHeader")
            .field("title", &self.title)
            .finish_non_exhaustive()
    }
}

impl<'a> SectionHeader<'a> {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            icon: None,
            count: None,
            open: None,
            action: None,
            first: false,
            force: None,
        }
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    pub fn count(mut self, n: usize) -> Self {
        self.count = Some(n);
        self
    }
    /// Makes the section collapsible.
    pub fn open(mut self, open: &'a mut bool) -> Self {
        self.open = Some(open);
        self
    }
    /// An icon button at the right that shows on hover; `label` is its tooltip.
    pub fn action(mut self, glyph: &str, label: impl Into<String>) -> Self {
        self.action = Some((glyph.to_owned(), label.into()));
        self
    }
    /// The first section: no gap above it.
    pub fn first(mut self, first: bool) -> Self {
        self.first = first;
        self
    }
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> KitResponse<bool> {
        self.force = Some(force);
        self.show(ui)
    }

    /// Shows the header; the value is whether its action was clicked.
    pub fn show(self, ui: &mut Ui) -> KitResponse<bool> {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let m = theme::metrics();
        if !self.first {
            ui.add_space(m.section_gap);
        }
        let width = ui.available_width();
        let rect = ui.allocate_space(vec2(width, m.section_header_height)).1;
        let collapsible = self.open.is_some();
        let sense = if collapsible {
            Sense::click()
        } else {
            Sense::hover()
        };
        let mut response = ui.interact(rect, ui.id().with(("kit-section", &self.title)), sense);
        let f = self.force.unwrap_or_default();
        let hovered = f.hover || ui.rect_contains_pointer(rect);
        let hover = match self.force {
            Some(f) => f32::from(u8::from(hovered)) * f.progress.unwrap_or(1.0),
            None => anim::ease_bool(
                &ctx,
                response.id.with("kit-section-hover"),
                hovered,
                motion::FAST,
                motion::FAST_OUT,
            ),
        };
        let mut open_now = self.open.as_deref().copied().unwrap_or(true);
        if collapsible && response.clicked() {
            open_now = !open_now;
            response.mark_changed();
        }
        if let Some(open) = self.open {
            *open = open_now;
        }
        let focused = f.focus || (response.has_focus() && focus_visible(&ctx));
        let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
        let cy = rect.center().y;
        let label_color = p.text_faint.lerp_to_gamma(p.text_dim, hover);
        let mut x = rect.left() + CHEVRON_INSET;
        if collapsible {
            let turn = anim::ease_bool(
                &ctx,
                response.id.with("kit-section-turn"),
                open_now,
                motion::BASE,
                motion::BASE,
            );
            paint_chevron(
                &painter,
                pos2(x + CHEVRON_WIDTH / 2.0, cy),
                turn,
                label_color,
            );
            x += CHEVRON_WIDTH + SECTION_CHEVRON_GAP;
        }
        if let Some(g) = &self.icon {
            let glyph = painter.layout_no_wrap(g.clone(), FontId::proportional(13.0), p.text_faint);
            painter.galley(
                pos2(x, cy - glyph.size().y / 2.0),
                Arc::clone(&glyph),
                p.text_faint,
            );
            x += glyph.size().x + 6.0;
        }
        let title = caps_galley(ui, &self.title, label_color);
        painter.galley(pos2(x, cy - title.size().y / 2.0), title, label_color);
        let action_w = if self.action.is_some() { 22.0 } else { 0.0 };
        if let Some(n) = self.count {
            let g = painter.layout_no_wrap(
                n.to_string(),
                FontId::new(m.micro_size, FontFamily::Monospace),
                p.text_faint,
            );
            let right = rect.right() - 6.0 - action_w;
            painter.galley(
                pos2(right - g.size().x, cy - g.size().y / 2.0),
                g,
                p.text_faint,
            );
        }
        if focused {
            paint::inset_ring(&painter, rect, CornerRadius::same(m.radius_sm), p.focus);
        }
        let acted = if let Some((glyph, label)) = &self.action {
            // The action is always there for the keyboard and AccessKit, right after the header
            // in the focus order (Tab and Shift+Tab reach it); it shows while the header is
            // hovered or focused, and fully while the action itself has focus.
            let action_key = response.id.with("kit-section-action-id");
            let action_focused = ctx
                .data(|d| d.get_temp::<Id>(action_key))
                .is_some_and(|id| ctx.memory(|m| m.has_focus(id)));
            let shown = hovered || focused || action_focused;
            let reveal = match self.force {
                Some(_) => f32::from(u8::from(shown)),
                None => anim::ease_bool(
                    &ctx,
                    response.id.with("kit-section-action"),
                    shown,
                    motion::FAST,
                    motion::FAST,
                ),
            };
            let area = Rect::from_min_max(
                pos2(rect.right() - action_w - 2.0, rect.top()),
                pos2(rect.right() - 2.0, rect.bottom()),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(area)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            child.multiply_opacity(if action_focused { 1.0 } else { reveal });
            let action = IconButton::new(glyph, label.clone())
                .size(Size::Xs)
                .id_salt(("kit-section-action", &self.title))
                .show(&mut child);
            ctx.data_mut(|d| d.insert_temp(action_key, action.id));
            action.clicked()
        } else {
            false
        };
        if collapsible {
            let name = super::a11y::disclosure_name(&self.title, open_now);
            response.widget_info(|| WidgetInfo::labeled(WidgetType::CollapsingHeader, true, &name));
            ctx.accesskit_node_builder(response.id, |n| {
                n.set_expanded(open_now);
                n.set_label(name.as_str());
            });
        } else {
            response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &self.title));
        }
        KitResponse {
            response,
            value: acted,
        }
    }
}

/// A section label (section headers, table column headers) in the active theme's label style
/// ([`theme::section_label`]: capitals or title case, its weight and tracking).
pub fn caps_galley(ui: &Ui, text: &str, color: Color32) -> Arc<egui::Galley> {
    ui.painter().layout_job(theme::section_label(text, color))
}

// ---------------------------------------------------------------------------------- Selection

/// A multi-selection of rows by index with its anchor: a click selects one row, Ctrl toggles
/// one, Shift selects the range from the anchor (Ctrl+Shift adds it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    pub rows: BTreeSet<usize>,
    pub anchor: Option<usize>,
}

impl Selection {
    /// Applies a click on row `index` with `modifiers`.
    pub fn click(&mut self, index: usize, modifiers: Modifiers) {
        let ctrl = modifiers.command || modifiers.ctrl;
        match (ctrl, modifiers.shift, self.anchor) {
            (_, true, Some(anchor)) => {
                let range = anchor.min(index)..=anchor.max(index);
                if !ctrl {
                    self.rows.clear();
                }
                self.rows.extend(range);
            }
            (true, _, _) => {
                if !self.rows.remove(&index) {
                    self.rows.insert(index);
                }
                self.anchor = Some(index);
            }
            _ => {
                self.rows.clear();
                self.rows.insert(index);
                self.anchor = Some(index);
            }
        }
    }

    pub fn contains(&self, index: usize) -> bool {
        self.rows.contains(&index)
    }

    /// Whether the rows above and below `index` are selected too (for [`ListRow::joined`]).
    pub fn neighbours(&self, index: usize) -> (bool, bool) {
        (
            index > 0 && self.rows.contains(&(index - 1)),
            self.rows.contains(&(index + 1)),
        )
    }
}

// ------------------------------------------------------------------------------------- Roving

/// One Tab stop for a list (design spec 4.16): Tab stops on one row of the list, the arrows move
/// between its rows, and the next Tab leaves the list, however long it is. The stop is the row
/// the user was last on, else [`Roving::new`]'s `initial` row, else the first; with
/// [`Roving::follow`] it is the selected row, and focus follows the selection while it is in the
/// list. Make one each frame before the rows and pass it to each with [`ListRow::roving`] or
/// [`TreeRow::roving`]; [`Roving::moved`] says where an arrow went, for views whose selection
/// moves with the keyboard. Rows that are not shown (a virtualised list) cost nothing: when the
/// stop is scrolled away the first row shown stands in for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Roving {
    id: Id,
    len: usize,
    columns: usize,
}

/// What a [`Roving`] list keeps between frames.
#[derive(Debug, Clone, Default)]
struct RovingState {
    /// The row Tab stops on.
    stop: usize,
    /// The user has been on the list: `stop` is theirs, not the caller's `initial`.
    visited: bool,
    /// The row that last had focus, and its widget.
    focused: Option<(usize, Id)>,
    /// The row to give focus to when it is next shown.
    pending: Option<usize>,
    /// The row an arrow moved from and to this frame.
    moved: Option<(usize, usize)>,
    /// Whether the stop row was shown this frame, and the frame before.
    stop_shown: bool,
    stop_shown_before: bool,
    /// A row stood in for the stop this frame (the stop was not shown the frame before).
    stand_in: bool,
    /// Enabled rows observed in the last render (and those already shown this render).
    enabled: Vec<bool>,
}

/// Find the next enabled row along the requested axis. Unknown virtualized rows stay eligible
/// until they are rendered, so scrolling can still bring them into view.
fn next_enabled_row(
    from: usize,
    len: usize,
    by: usize,
    forward: bool,
    enabled: &[bool],
) -> Option<usize> {
    let mut at = if forward {
        from.checked_add(by)
    } else {
        from.checked_sub(by)
    };
    while let Some(index) = at.filter(|&i| i < len) {
        if enabled.get(index).copied().unwrap_or(true) {
            return Some(index);
        }
        at = if forward {
            index.checked_add(by)
        } else {
            index.checked_sub(by)
        };
    }
    None
}

impl Roving {
    /// The list `id_salt` of `len` rows, for this frame. `initial` is the row Tab stops on before
    /// the user has been on the list (the checked-out branch).
    pub fn new(
        ctx: &egui::Context,
        id_salt: impl std::hash::Hash + std::fmt::Debug,
        len: usize,
        initial: Option<usize>,
    ) -> Self {
        let id = Id::new(("kit-roving", Id::new(id_salt)));
        let last = len.saturating_sub(1);
        ctx.data_mut(|d| {
            let st = d.get_temp_mut_or_default::<RovingState>(id);
            st.moved = None;
            st.stop_shown_before = st.stop_shown;
            st.stop_shown = false;
            st.stand_in = false;
            st.enabled.resize(len, true);
            st.enabled.truncate(len);
            if !st.visited
                && let Some(i) = initial
            {
                st.stop = i;
            }
            st.stop = st.stop.min(last);
            st.pending = st.pending.map(|p| p.min(last));
            if !st.enabled.get(st.stop).copied().unwrap_or(true) {
                st.stop = next_enabled_row(st.stop, len, 1, true, &st.enabled)
                    .or_else(|| next_enabled_row(st.stop, len, 1, false, &st.enabled))
                    .unwrap_or(st.stop);
            }
        });
        Self {
            id,
            len,
            columns: 1,
        }
    }

    /// The list's selection follows the keyboard (History): call it with the selected row before
    /// that row is shown. The selected row is the stop, and while another row of the list has
    /// focus (the selection moved with j and k, a click, a search) focus moves to it.
    pub fn follow(&self, ctx: &egui::Context, selected: usize) {
        let selected = selected.min(self.len.saturating_sub(1));
        let focused = ctx.data_mut(|d| d.get_temp_mut_or_default::<RovingState>(self.id).focused);
        let in_list = focused.is_some_and(|(_, id)| ctx.memory(|m| m.has_focus(id)));
        ctx.data_mut(|d| {
            let st = d.get_temp_mut_or_default::<RovingState>(self.id);
            st.stop = selected;
            st.visited = true;
            if in_list && focused.is_some_and(|(at, _)| at != selected) {
                st.pending = Some(selected);
            }
        });
    }

    /// The rows flow in `n` columns (a grid of cards): Left and Right move by one, Up and Down by
    /// a row of `n`.
    #[must_use]
    pub fn columns(mut self, n: usize) -> Self {
        self.columns = n.max(1);
        self
    }

    /// The rows an arrow moved from and to this frame, once the rows have been shown.
    pub fn moved(&self, ctx: &egui::Context) -> Option<(usize, usize)> {
        ctx.data_mut(|d| d.get_temp_mut_or_default::<RovingState>(self.id).moved)
    }

    /// Moves focus to row `index` when it is next shown (a tree row's parent or first child).
    pub fn focus(&self, ctx: &egui::Context, index: usize) {
        let index = index.min(self.len.saturating_sub(1));
        ctx.data_mut(|d| {
            let st = d.get_temp_mut_or_default::<RovingState>(self.id);
            st.stop = index;
            st.visited = true;
            st.pending = Some(index);
        });
    }

    /// Register a row's availability even when disabled (and therefore not interactive).
    /// The next frame knows which rows an arrow must skip. A row disabled while a focus
    /// transfer is pending forwards that transfer rather than trapping it.
    fn register(&self, ctx: &egui::Context, index: usize, enabled: bool) {
        ctx.data_mut(|d| {
            let st = d.get_temp_mut_or_default::<RovingState>(self.id);
            if let Some(available) = st.enabled.get_mut(index) {
                *available = enabled;
            }
            if !enabled && st.pending == Some(index) {
                let forward = index >= st.stop;
                st.pending = next_enabled_row(index, self.len, 1, forward, &st.enabled);
            }
        });
    }

    /// Whether row `index` (widget `id`, when known before it is shown) takes focus from Tab
    /// this frame: the stop, the row with focus, the row focus is moving to, or the first row
    /// shown while the stop is scrolled away.
    pub(crate) fn takes_tab(&self, ctx: &egui::Context, index: usize, id: Option<Id>) -> bool {
        let last = ctx
            .data_mut(|d| d.get_temp_mut_or_default::<RovingState>(self.id).focused)
            .filter(|(at, _)| *at == index)
            .map(|(_, f)| f);
        let focused = id.or(last).is_some_and(|f| ctx.memory(|m| m.has_focus(f)));
        ctx.data_mut(|d| {
            let st = d.get_temp_mut_or_default::<RovingState>(self.id);
            if index == st.stop {
                st.stop_shown = true;
                return true;
            }
            if focused || st.pending == Some(index) {
                return true;
            }
            if !st.stop_shown_before && !st.stand_in {
                st.stand_in = true;
                return true;
            }
            false
        })
    }

    /// Row `index` was shown with `response`: gives it the focus that is moving to it, makes it
    /// the stop when the user is on it, and moves along the list on the arrows while it has
    /// focus (Left and Right stay with a `tree` row, which opens and closes on them).
    pub(crate) fn row(&self, ui: &Ui, response: &Response, index: usize, tree: bool) {
        let ctx = ui.ctx();
        let pending = ctx.data_mut(|d| d.get_temp_mut_or_default::<RovingState>(self.id).pending);
        if pending == Some(index) {
            response.request_focus();
            ctx.data_mut(|d| d.get_temp_mut_or_default::<RovingState>(self.id).pending = None);
        }
        let focused = response.has_focus() || pending == Some(index);
        if !(focused || response.clicked()) {
            return;
        }
        let horizontal = self.columns > 1;
        let target = if focused {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    response.id,
                    egui::EventFilter {
                        vertical_arrows: true,
                        horizontal_arrows: horizontal || tree,
                        ..Default::default()
                    },
                );
            });
            let step = |forward: Key, back: Key, by: usize| {
                ui.input_mut(|i| {
                    if i.consume_key(Modifiers::NONE, forward) {
                        Some((by, true))
                    } else if i.consume_key(Modifiers::NONE, back) {
                        Some((by, false))
                    } else {
                        None
                    }
                })
            };
            let mut move_by = step(Key::ArrowDown, Key::ArrowUp, self.columns);
            if horizontal {
                move_by = move_by.or_else(|| step(Key::ArrowRight, Key::ArrowLeft, 1));
            }
            move_by.and_then(|(by, forward)| {
                ctx.data_mut(|d| {
                    let st = d.get_temp_mut_or_default::<RovingState>(self.id);
                    next_enabled_row(index, self.len, by, forward, &st.enabled)
                })
            })
        } else {
            None
        };
        if let Some(t) = target
            && !horizontal
        {
            // Bring the row into view before it is asked for focus (it may not be shown yet).
            let rows = crate::count_f32(t) - crate::count_f32(index);
            ui.scroll_to_rect(
                response
                    .rect
                    .translate(vec2(0.0, rows * response.rect.height())),
                None,
            );
        }
        let clicked = response.clicked() && !focused;
        ctx.data_mut(|d| {
            let st = d.get_temp_mut_or_default::<RovingState>(self.id);
            st.stop = index;
            st.visited = true;
            if focused {
                st.focused = Some((index, response.id));
            } else if clicked {
                // A click puts keyboard focus on the row, as in any list: the arrows go on from
                // it and Shift+F10 opens its menu. The row takes it next frame, as the stop.
                st.pending = Some(index);
            }
            if let Some(t) = target {
                st.pending = Some(t);
                st.moved = Some((index, t));
            }
        });
    }
}

// ------------------------------------------------------------------------------------ gallery

/// The gallery's Lists section: every `ListRow` state, tree rows open and closed at three depths, a
/// disclosure and section headers.
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
    ui.spacing_mut().item_spacing.y = 2.0;
    small(
        ui,
        "List rows: rest, hover, selected, selected + hover, cursor, pressed, disabled",
    );
    let states: [(&str, Option<ForceState>, bool, bool, bool); 7] = [
        ("Rest", None, false, false, true),
        ("Hover", Some(ForceState::HOVER), false, false, true),
        ("Selected", None, true, false, true),
        (
            "Selected and hovered",
            Some(ForceState::HOVER),
            true,
            false,
            true,
        ),
        ("Keyboard cursor", None, false, true, true),
        ("Pressed", Some(ForceState::PRESSED), false, false, true),
        ("Disabled", None, false, false, false),
    ];
    for (i, (name, force, selected, cursor, enabled)) in states.into_iter().enumerate() {
        let row = ListRow::new(("gallery-row", i))
            .kind(RowKind::File)
            .icon(icon::FILE_TEXT)
            .label(name)
            .detail("src/ui")
            .trailing("+4 −1")
            .selected(selected)
            .cursor(cursor)
            .enabled(enabled)
            .disabled_reason("Not available");
        match force {
            Some(f) => row.show_forced(ui, f),
            None => row.show(ui),
        };
    }
    small(
        ui,
        "Palette row (indented, tooltip), request row on the chrome ground",
    );
    ListRow::new("gallery-row-palette")
        .height_from(RowKind::Palette)
        .icon(icon::ARROWS_CLOCKWISE)
        .label("Fetch all remotes")
        .indent(theme::XS)
        .tooltip("Runs git fetch --all")
        .show(ui);
    ListRow::new("gallery-row-request")
        .kind(RowKind::Request)
        .ground(theme::surfaces(ui.ctx()).panel)
        .label("Add dark mode")
        .detail("#42 · feature/dark-mode")
        .show_with(ui, |rc, painter| {
            // Custom content gets the row's opaque ground for rings drawn on it.
            let c = pos2(rc.content.right() - 12.0, rc.rect.center().y);
            painter.circle_filled(c, 7.0, rc.ground);
            painter.circle_filled(c, 5.0, rc.accent);
        });
    small(ui, "With hover actions (revealed on the selected row)");
    ListRow::new("gallery-row-actions")
        .kind(RowKind::File)
        .icon(icon::FILE_CODE)
        .label("main.rs")
        .detail("src")
        .selected(true)
        .actions(|ui| {
            IconButton::new(icon::MINUS, "Unstage")
                .size(Size::Sm)
                .id_salt("gallery-row-unstage")
                .show(ui);
            IconButton::new(icon::ARROW_COUNTER_CLOCKWISE, "Discard")
                .size(Size::Sm)
                .id_salt("gallery-row-discard")
                .show(ui);
        })
        .show(ui);
    small(
        ui,
        "History rows (zebra on even rows in Dense), joined selection",
    );
    let mut sel = Selection::default();
    sel.click(1, Modifiers::NONE);
    sel.click(2, Modifiers::SHIFT);
    ui.spacing_mut().item_spacing.y = 0.0;
    for i in 0..4 {
        let (above, below) = sel.neighbours(i);
        ListRow::new(("gallery-history", i))
            .kind(RowKind::History)
            .index(i)
            .label(format!("feat(ui): commit number {i}"))
            .trailing(format!(
                "{:07x}",
                0x00ab_cdef_u32.wrapping_mul(u32::try_from(i).map_or(0, |i| i + 3))
            ))
            .selected(sel.contains(i))
            .joined(above, below)
            .show(ui);
    }
    ui.spacing_mut().item_spacing.y = 2.0;
    small(
        ui,
        "Tree rows: open and closed at depth 0 to 2, mid-turn chevron",
    );
    let id = ui.id().with("gallery-tree");
    let mut state: [bool; 2] = ui.ctx().data(|d| d.get_temp(id)).unwrap_or([true, false]);
    let [mut origin, mut upstream] = state;
    TreeRow::new("gallery-tree-origin", 0)
        .icon(icon::CLOUD)
        .label("origin")
        .trailing("3")
        .open(&mut origin)
        .show(ui);
    tree_children(ui, id.with("origin"), origin, 3, |ui| {
        TreeRow::new("gallery-tree-main", 1)
            .icon(icon::GIT_BRANCH)
            .label("main")
            .selected(true)
            .show(ui);
        TreeRow::new("gallery-tree-feature", 1)
            .icon(icon::FOLDER_SIMPLE)
            .label("feature")
            .open(&mut upstream)
            .show(ui);
        TreeRow::new("gallery-tree-leaf", 2)
            .icon(icon::GIT_BRANCH)
            .label("feature/dark-mode")
            .show_forced(ui, ForceState::HOVER);
    });
    TreeRow::new("gallery-tree-cursor", 1)
        .icon(icon::GIT_BRANCH)
        .label("keyboard cursor")
        .cursor(true)
        .show(ui);
    TreeRow::new("gallery-tree-disabled", 1)
        .icon(icon::GIT_BRANCH)
        .label("disabled")
        .enabled(false)
        .show(ui);
    TreeRow::new("gallery-tree-mid", 0)
        .icon(icon::CLOUD)
        .label("upstream (chevron at 0.5)")
        .open(&mut true)
        .show_forced(
            ui,
            ForceState {
                progress: Some(0.5),
                ..ForceState::default()
            },
        );
    state = [origin, upstream];
    ui.ctx().data_mut(|d| d.insert_temp(id, state));
    small(ui, "Disclosure");
    let did = ui.id().with("gallery-disclosure");
    let mut open: bool = ui.ctx().data(|d| d.get_temp(did)).unwrap_or(true);
    Disclosure::new("gallery-disclosure", "Commit information", &mut open)
        .icon(icon::INFO)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("Author, committer and parents.")
                    .size(m.meta_size)
                    .color(p.text_dim),
            );
        });
    ui.ctx().data_mut(|d| d.insert_temp(did, open));
    let cid = ui.id().with("gallery-checks");
    let mut checks: bool = ui.ctx().data(|d| d.get_temp(cid)).unwrap_or(false);
    Disclosure::new("gallery-checks", "Checks: 1 failing", &mut checks)
        .icon(icon::X_CIRCLE)
        .icon_tone(Tone::Danger)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("build (ubuntu-latest) failed")
                    .size(m.meta_size)
                    .color(p.text_dim),
            );
        });
    ui.ctx().data_mut(|d| d.insert_temp(cid, checks));
    small(
        ui,
        "Rows: trailing badge and widgets, read-only and taller, a tree row with a tooltip",
    );
    ListRow::new("gallery-row-badge")
        .kind(RowKind::Request)
        .icon(icon::GIT_PULL_REQUEST)
        .label("Retry with backoff")
        .detail("#482")
        .trailing_badge(Badge::new("Draft"))
        .trailing_ui(54.0, |ui| {
            IconButton::new(icon::ARROW_SQUARE_OUT, "Open on GitHub")
                .size(Size::Xs)
                .id_salt("gallery-row-open")
                .show(ui);
            IconButton::new(icon::COPY, "Copy link")
                .size(Size::Xs)
                .id_salt("gallery-row-copy")
                .show(ui);
        })
        .accessible_name("Pull request 482, Retry with backoff, draft")
        .show(ui);
    ListRow::new("gallery-row-read-only")
        .height(m.row_height * 1.5)
        .icon(icon::KEY)
        .label("SHA256:6m3U4…")
        .detail("ed25519, added 2 days ago")
        .read_only(true)
        .show(ui);
    let _ = TreeRow::new("gallery-tree-tip", 0)
        .kind(RowKind::List)
        .icon(icon::CLOUD)
        .icon_tone(Tone::Accent)
        .label("origin")
        .tooltip("git@github.com:necturalabs/canopy.git")
        .show(ui);
    let _ = TreeRow::new("gallery-tree-off", 0)
        .kind(RowKind::List)
        .icon(icon::CLOUD_SLASH)
        .label("backup")
        .enabled(false)
        .disabled_reason("This remote has no URL")
        .show(ui);
    small(
        ui,
        "Section headers: rest, hover with action, focus, static",
    );
    let hid = ui.id().with("gallery-sections");
    let mut opens: [bool; 3] = ui.ctx().data(|d| d.get_temp(hid)).unwrap_or([true; 3]);
    SectionHeader::new("Branches")
        .icon(icon::GIT_BRANCH)
        .count(6)
        .open(&mut opens[0])
        .first(true)
        .show(ui);
    SectionHeader::new("Remotes")
        .icon(icon::CLOUD)
        .count(1)
        .open(&mut opens[1])
        .action(icon::PLUS, "Add remote…")
        .show_forced(ui, ForceState::HOVER);
    SectionHeader::new("Tags")
        .icon(icon::TAG)
        .count(3)
        .open(&mut opens[2])
        .show_forced(ui, ForceState::FOCUS);
    SectionHeader::new("Stashes")
        .icon(icon::ARCHIVE)
        .count(1)
        .show(ui);
    ui.ctx().data_mut(|d| d.insert_temp(hid, opens));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Density, Style};

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }

    fn run(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out
    }

    /// A row too narrow for its label and detail cuts the label first when asked to keep the
    /// detail whole.
    #[test]
    fn a_whole_detail_survives_a_long_label() {
        let detail = "Sam Okafor · Aug 13, 2026";
        for (whole, kept) in [(true, true), (false, false)] {
            let ctx = ctx();
            let out = run(&ctx, Vec::new(), |ui| {
                ui.set_max_width(320.0);
                let row = ListRow::new("long")
                    .label("a summary far too long to share the row with anything else at all")
                    .detail(detail);
                (if whole { row.whole_detail() } else { row }).show(ui);
            });
            let shown = out
                .shapes
                .iter()
                .any(|c| matches!(&c.shape, Shape::Text(t) if t.galley.text() == detail));
            assert_eq!(shown, kept, "whole detail: {whole}");
        }
    }

    /// The label of the node with keyboard focus after one frame of `f` with `events`, and the
    /// frame after it (focus moves at the end of the frame that read the key).
    fn focus_after(
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> Option<String> {
        run(ctx, events, &mut f);
        let out = run(ctx, Vec::new(), &mut f);
        let update = out.platform_output.accesskit_update.expect("accesskit");
        update
            .nodes
            .iter()
            .find(|(id, _)| *id == update.focus)
            .and_then(|(_, n)| n.label().map(str::to_owned))
    }

    fn shift_tab() -> egui::Event {
        egui::Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::SHIFT,
        }
    }

    /// A11Y-3: a list is one Tab stop: Tab enters it on one row and the next Tab leaves it, the
    /// arrows move between its rows, and coming back lands on the row the user was last on.
    #[test]
    fn a_list_is_one_tab_stop_and_the_arrows_move_inside_it() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let mut moves = Vec::new();
        let mut show = |ui: &mut Ui| {
            super::super::Button::new("Before").show(ui);
            let list = Roving::new(ui.ctx(), "test-list", 6, Some(2));
            for i in 0..6 {
                ListRow::new(("row", i))
                    .label(format!("Row {i}"))
                    .roving(list, i)
                    .show(ui);
            }
            if let Some(m) = list.moved(ui.ctx()) {
                moves.push(m);
            }
            super::super::Button::new("After").show(ui);
        };
        run(&ctx, Vec::new(), &mut show);
        let mut path = Vec::new();
        for events in [
            vec![key(Key::Tab)],
            vec![key(Key::Tab)],
            vec![key(Key::ArrowDown)],
            vec![key(Key::ArrowDown)],
            vec![key(Key::ArrowUp)],
            vec![key(Key::Tab)],
            vec![shift_tab()],
            vec![shift_tab()],
        ] {
            path.push(focus_after(&ctx, events, &mut show).unwrap_or_default());
        }
        assert_eq!(
            path,
            [
                "Before", "Row 2", "Row 3", "Row 4", "Row 3", "After", "Row 3", "Before"
            ],
            "Tab enters on the stop and leaves the list; the arrows move inside it"
        );
        assert_eq!(moves, [(2, 3), (3, 4), (4, 3)], "the view hears each move");
    }

    /// Disabled rows must not become the destination of an arrow: before this fix, the
    /// pending target stayed on the disabled row and the roving list stopped navigating.
    #[test]
    fn roving_arrows_skip_disabled_rows_in_both_directions() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let mut moves = Vec::new();
        let mut show = |ui: &mut Ui| {
            super::super::Button::new("Before").show(ui);
            let list = Roving::new(ui.ctx(), "disabled-list", 6, Some(0));
            for i in 0..6 {
                ListRow::new(("disabled-row", i))
                    .label(format!("Row {i}"))
                    .enabled(!matches!(i, 1 | 2 | 4))
                    .roving(list, i)
                    .show(ui);
            }
            if let Some(m) = list.moved(ui.ctx()) {
                moves.push(m);
            }
            super::super::Button::new("After").show(ui);
        };
        run(&ctx, Vec::new(), &mut show);
        let mut path = Vec::new();
        for keypress in [
            Key::Tab,
            Key::Tab,
            Key::ArrowDown,
            Key::ArrowDown,
            Key::ArrowDown,
            Key::ArrowUp,
            Key::ArrowUp,
        ] {
            path.push(focus_after(&ctx, vec![key(keypress)], &mut show).unwrap_or_default());
        }
        assert_eq!(
            path,
            [
                "Before", "Row 0", "Row 3", "Row 5", "Row 5", "Row 3", "Row 0"
            ]
        );
        assert_eq!(moves, [(0, 3), (3, 5), (5, 3), (3, 0)]);
    }

    /// usersim F14: a click puts keyboard focus on the row it selected, so the arrows move on from
    /// it and Shift+F10 opens its menu; the row paints its focus ring even though it is selected.
    #[test]
    fn a_click_focuses_the_row_and_the_arrows_continue_from_it() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let rects = std::cell::RefCell::new(Vec::new());
        let moves = std::cell::RefCell::new(Vec::new());
        let mut show = |ui: &mut Ui| {
            rects.borrow_mut().clear();
            let list = Roving::new(ui.ctx(), "click-list", 6, Some(0));
            for i in 0..6 {
                let r = ListRow::new(("click-row", i))
                    .label(format!("Row {i}"))
                    .roving(list, i)
                    .show(ui);
                rects.borrow_mut().push(r.rect);
            }
            if let Some(m) = list.moved(ui.ctx()) {
                moves.borrow_mut().push(m);
            }
        };
        run(&ctx, Vec::new(), &mut show);
        let at = rects.borrow()[4].center();
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        run(&ctx, vec![egui::Event::PointerMoved(at)], &mut show);
        run(&ctx, vec![button(true), button(false)], &mut show);
        // Two quiet frames: the row takes the focus the click asked for.
        let focused = focus_after(&ctx, Vec::new(), &mut show);
        assert_eq!(
            focused.as_deref(),
            Some("Row 4"),
            "the clicked row has focus"
        );
        let next = focus_after(&ctx, vec![key(Key::ArrowDown)], &mut show);
        assert_eq!(next.as_deref(), Some("Row 5"), "the arrow moves on from it");
        assert_eq!(*moves.borrow(), [(4, 5)]);
    }

    /// usersim F14: a selected row with keyboard focus shows the focus ring over its selection.
    #[test]
    fn a_selected_row_with_focus_paints_the_focus_ring() {
        let ctx = ctx();
        let ring = theme::palette(&ctx).focus;
        let has_ring = |out: &egui::FullOutput| {
            fn walk(s: &Shape, ring: Color32) -> bool {
                match s {
                    Shape::Rect(r) => r.stroke.color == ring && r.stroke.width > 0.0,
                    Shape::Vec(v) => v.iter().any(|s| walk(s, ring)),
                    _ => false,
                }
            }
            out.shapes.iter().any(|c| walk(&c.shape, ring))
        };
        let resting = run(&ctx, Vec::new(), |ui| {
            ListRow::new("ring").label("Row").selected(true).show(ui);
        });
        assert!(!has_ring(&resting), "a selected row at rest has no ring");
        let focused = run(&ctx, Vec::new(), |ui| {
            ListRow::new("ring")
                .label("Row")
                .selected(true)
                .show_forced(ui, ForceState::FOCUS);
        });
        assert!(has_ring(&focused), "a focused selected row shows the ring");
    }

    /// A11Y-2: with motion on, Tab from a row goes to its hover action and on to the next control;
    /// the action does not fade away (and take focus with it) once the row lost focus.
    #[test]
    fn tab_goes_through_a_rows_actions_with_motion_on() {
        let ctx = ctx();
        ctx.enable_accesskit();
        theme::apply(
            &ctx,
            theme::ThemeId::Midnight,
            theme::Appearance {
                reduce_motion: false,
                ..theme::Appearance::STILL
            },
        );
        let mut show = |ui: &mut Ui| {
            for i in 0..2 {
                ListRow::new(("acting", i))
                    .label(format!("File {i}"))
                    .actions(move |ui| {
                        IconButton::new(icon::PLUS, format!("Stage file {i}"))
                            .size(Size::Sm)
                            .show(ui);
                    })
                    .show(ui);
            }
            super::super::Button::new("Commit").show(ui);
        };
        run(&ctx, Vec::new(), &mut show);
        let mut path = Vec::new();
        for _ in 0..5 {
            path.push(focus_after(&ctx, vec![key(Key::Tab)], &mut show).unwrap_or_default());
            // Quiet frames, past the action's fade: focus stays where Tab put it.
            for _ in 0..20 {
                assert_eq!(
                    focus_after(&ctx, Vec::new(), &mut show).unwrap_or_default(),
                    *path.last().unwrap(),
                    "focus moved by itself after {path:?}"
                );
            }
        }
        assert_eq!(
            path,
            ["File 0", "Stage file 0", "File 1", "Stage file 1", "Commit"],
            "{path:?}"
        );
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
    }

    /// A11Y-5, A11Y-6: a tree row's chevron is the pointer's target only, not a Tab stop of its
    /// own nor an unnamed node.
    #[test]
    fn a_tree_rows_chevron_is_no_tab_stop() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let mut open = false;
        let mut show = |ui: &mut Ui| {
            let _ = TreeRow::new("origin", 0)
                .label("origin")
                .open(&mut open)
                .show(ui);
            super::super::Button::new("After").show(ui);
        };
        run(&ctx, Vec::new(), &mut show);
        let first = focus_after(&ctx, vec![key(Key::Tab)], &mut show);
        let second = focus_after(&ctx, vec![key(Key::Tab)], &mut show);
        assert!(first.is_some_and(|l| l.starts_with("origin")));
        assert_eq!(second.as_deref(), Some("After"));
    }

    #[test]
    fn rows_take_their_kind_height_in_every_density_and_style() {
        let ctx = ctx();
        for density in Density::ALL {
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
                let mut h = Vec::new();
                run(&ctx, Vec::new(), |ui| {
                    h = vec![
                        ListRow::new(1)
                            .kind(RowKind::History)
                            .show(ui)
                            .rect
                            .height(),
                        ListRow::new(2).kind(RowKind::File).show(ui).rect.height(),
                        TreeRow::new(3, 0).show(ui).response.rect.height(),
                        SectionHeader::new("x")
                            .first(true)
                            .show(ui)
                            .response
                            .rect
                            .height(),
                    ];
                });
                assert_eq!(
                    h,
                    vec![
                        m.history_row_height,
                        m.row_height,
                        m.sidebar_row_height,
                        m.section_header_height
                    ],
                    "{density:?} {style:?}"
                );
            }
        }
    }

    #[test]
    fn zebra_stripes_even_rows_in_dense_only() {
        let _ = ctx();
        assert!(!zebra(1));
        theme::with_look(Style::Standard, Density::Dense, || {
            assert!(!zebra(0));
            assert!(zebra(1));
            assert!(!zebra(2));
        });
    }

    #[test]
    fn the_ground_composites_the_row_state() {
        let p = theme::ThemeId::Midnight.palette();
        let pane = p.standard.pane;
        assert_eq!(ground(p, pane, false, false, 0.0, false, false), pane);
        let sel = ground(p, pane, false, true, 0.0, false, false);
        assert_eq!(sel, theme::over(p.selected(), pane));
        let both = ground(p, pane, false, true, 1.0, false, false);
        assert_eq!(both, theme::over(p.hover(), sel));
    }

    #[test]
    fn rows_activate_from_the_keyboard_unless_disabled() {
        let ctx = ctx();
        for enabled in [true, false] {
            let mut clicked = false;
            for i in 0..3 {
                let events = if i == 1 {
                    vec![key(Key::Enter)]
                } else {
                    Vec::new()
                };
                run(&ctx, events, |ui| {
                    let r = ListRow::new("r").label("Row").enabled(enabled).show(ui);
                    if i == 0 {
                        ui.memory_mut(|m| m.request_focus(r.id));
                    }
                    clicked |= r.clicked();
                });
            }
            assert_eq!(clicked, enabled);
        }
    }

    #[test]
    fn a_row_speaks_its_accessible_name_once() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let named = |ui: &mut Ui| {
            ListRow::new("named")
                .label("api.md")
                .accessible_name("api.md, added")
                .show(ui)
                .id
        };
        let mut id = Id::NULL;
        let _ = run(&ctx, Vec::new(), |ui| id = named(ui));
        // Tab moves focus onto the row: it says its name once, the accessible one.
        let out = run(&ctx, vec![key(Key::Tab)], |ui| {
            named(ui);
        });
        assert_eq!(ctx.memory(egui::Memory::focused), Some(id));
        let nodes = out
            .platform_output
            .accesskit_update
            .expect("accesskit")
            .nodes;
        let named: Vec<_> = nodes
            .iter()
            .filter(|(_, n)| n.role() == egui::accesskit::Role::ListItem)
            .map(|(_, n)| n.label().map(str::to_owned))
            .collect();
        assert_eq!(named, vec![Some("api.md, added".to_owned())]);
        let focus: Vec<_> = out
            .platform_output
            .events
            .iter()
            .filter(|e| matches!(e, egui::output::OutputEvent::FocusGained(_)))
            .map(|e| e.widget_info().label.clone())
            .collect();
        assert_eq!(focus, vec![Some("api.md, added".to_owned())]);
    }

    #[test]
    fn a_read_only_row_takes_focus_but_no_clicks_or_hover() {
        let ctx = ctx();
        let mut rect = Rect::NOTHING;
        let mut clicked = false;
        let _ = super::super::testkit::frame(&ctx, Vec::new(), |ui| {
            rect = ListRow::new("ro")
                .label("SHA256:…")
                .read_only(true)
                .show(ui)
                .rect;
        });
        let at = rect.center();
        super::super::testkit::click(&ctx, at, |ui| {
            clicked |= ListRow::new("ro")
                .label("SHA256:…")
                .read_only(true)
                .show(ui)
                .clicked();
        });
        assert!(!clicked);
        let p = theme::palette(&ctx);
        let shapes = super::super::testkit::frame(
            &ctx,
            vec![egui::Event::PointerMoved(rect.center())],
            |ui| {
                ListRow::new("ro")
                    .label("SHA256:…")
                    .read_only(true)
                    .show(ui);
            },
        );
        let (fills, _) = super::super::testkit::painted_colors(&shapes);
        assert!(
            !fills
                .iter()
                .any(|(_, c)| *c == p.hover() || *c == p.pressed()),
            "{fills:?}"
        );
    }

    #[test]
    fn rows_take_a_height_of_their_own_and_a_trailing_slot() {
        let ctx = ctx();
        let m = theme::metrics();
        let mut inside = false;
        let mut height = 0.0;
        let _ = super::super::testkit::frame(&ctx, Vec::new(), |ui| {
            let r = ListRow::new("tall")
                .label("Two lines")
                .height(m.row_height * 2.0)
                .trailing_ui(40.0, |ui| {
                    inside = ui.max_rect().width() <= 40.0 + 0.5;
                })
                .show(ui);
            height = r.rect.height();
        });
        assert_eq!(height, m.row_height * 2.0);
        assert!(inside, "the trailing widgets get the slot they asked for");
    }

    #[test]
    fn tree_rows_open_with_right_and_ask_for_the_parent_with_left() {
        let ctx = ctx();
        let mut open = false;
        let mut navs = Vec::new();
        let presses = [
            None,
            Some(Key::ArrowRight),
            Some(Key::ArrowRight),
            Some(Key::ArrowLeft),
            Some(Key::ArrowLeft),
        ];
        let mut opens = Vec::new();
        for (i, k) in presses.into_iter().enumerate() {
            run(&ctx, k.map(key).into_iter().collect(), |ui| {
                let r = TreeRow::new("t", 0)
                    .label("origin")
                    .open(&mut open)
                    .show(ui);
                if i == 0 {
                    ui.memory_mut(|m| m.request_focus(r.response.id));
                }
                navs.push(r.value);
            });
            opens.push(open);
        }
        assert_eq!(opens, [false, true, true, false, false]);
        assert_eq!(
            navs,
            [
                None,
                None,
                Some(TreeNav::FirstChild),
                None,
                Some(TreeNav::Parent)
            ]
        );
    }

    #[test]
    fn a_leaf_asks_for_its_parent_and_never_opens() {
        let ctx = ctx();
        let mut nav = None;
        for i in 0..3 {
            let events = match i {
                1 => vec![key(Key::ArrowRight)],
                2 => vec![key(Key::ArrowLeft)],
                _ => Vec::new(),
            };
            run(&ctx, events, |ui| {
                let r = TreeRow::new("leaf", 1).label("main").show(ui);
                if i == 0 {
                    ui.memory_mut(|m| m.request_focus(r.response.id));
                }
                if i == 1 {
                    assert_eq!(r.value, None);
                }
                nav = r.value;
            });
        }
        assert_eq!(nav, Some(TreeNav::Parent));
    }

    #[test]
    fn a_section_header_toggles_on_enter_and_reports_its_action() {
        let ctx = ctx();
        let mut open = true;
        for i in 0..3 {
            let events = if i == 1 {
                vec![key(Key::Enter)]
            } else {
                Vec::new()
            };
            run(&ctx, events, |ui| {
                let r = SectionHeader::new("Branches").open(&mut open).show(ui);
                if i == 0 {
                    ui.memory_mut(|m| m.request_focus(r.response.id));
                }
                assert!(!r.value);
            });
        }
        assert!(!open);
    }

    #[test]
    fn tree_children_snap_with_reduced_motion() {
        let ctx = ctx();
        let id = Id::new("children");
        let mut shown = None;
        run(&ctx, Vec::new(), |ui| {
            shown = tree_children(ui, id, true, 3, |ui| ui.label("child").rect.height());
        });
        assert!(shown.is_some());
        run(&ctx, Vec::new(), |ui| {
            shown = tree_children(ui, id, false, 3, |ui| ui.label("child").rect.height());
        });
        assert!(shown.is_none(), "closed children disappear at once");
    }

    #[test]
    fn hover_lands_at_once_with_reduced_motion() {
        let ctx = ctx();
        let mut rect = Rect::NOTHING;
        let mut hovers = Vec::new();
        for i in 0..3 {
            let events = if i == 1 {
                vec![egui::Event::PointerMoved(rect.center())]
            } else {
                Vec::new()
            };
            run(&ctx, events, |ui| {
                let r = ListRow::new("h")
                    .label("Row")
                    .show_with(ui, |rc, _| hovers.push(rc.hover));
                rect = r.rect;
            });
        }
        // The frame the pointer arrives in already paints the full hover ground.
        assert_eq!(hovers, vec![0.0, 1.0, 1.0]);
    }

    #[test]
    fn selection_follows_click_ctrl_and_shift() {
        let mut s = Selection::default();
        s.click(3, Modifiers::NONE);
        assert_eq!(s.rows.iter().copied().collect::<Vec<_>>(), [3]);
        s.click(6, Modifiers::SHIFT);
        assert_eq!(s.rows.iter().copied().collect::<Vec<_>>(), [3, 4, 5, 6]);
        assert_eq!(s.anchor, Some(3));
        s.click(5, Modifiers::COMMAND);
        assert_eq!(s.rows.iter().copied().collect::<Vec<_>>(), [3, 4, 6]);
        s.click(1, Modifiers::SHIFT | Modifiers::COMMAND);
        assert_eq!(
            s.rows.iter().copied().collect::<Vec<_>>(),
            [1, 2, 3, 4, 5, 6]
        );
        s.click(9, Modifiers::NONE);
        assert_eq!(s.rows.iter().copied().collect::<Vec<_>>(), [9]);
        assert_eq!(s.neighbours(9), (false, false));
        s.click(10, Modifiers::SHIFT);
        assert_eq!(s.neighbours(9), (false, true));
        assert_eq!(s.neighbours(10), (true, false));
    }

    #[test]
    fn rows_announce_their_role_selection_and_expansion() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let out = run(&ctx, Vec::new(), |ui| {
            ListRow::new("a").label("main.rs").selected(true).show(ui);
            TreeRow::new("b", 0)
                .label("origin")
                .open(&mut true)
                .show(ui);
            SectionHeader::new("Branches").open(&mut false).show(ui);
        });
        let update = out.platform_output.accesskit_update.expect("accesskit");
        let node = |label: &str| {
            update
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .map_or_else(|| panic!("no node {label}"), |(_, n)| n.clone())
        };
        let row = node("main.rs");
        assert_eq!(row.role(), egui::accesskit::Role::ListItem);
        assert_eq!(row.is_selected(), Some(true));
        // A11Y-17: Linux screen readers hear the state in the name.
        let tree = node(&super::super::a11y::disclosure_name("origin", true));
        assert_eq!(tree.role(), egui::accesskit::Role::TreeItem);
        assert_eq!(tree.is_expanded(), Some(true));
        let header = node(&super::super::a11y::disclosure_name("Branches", false));
        assert_eq!(header.is_expanded(), Some(false));
    }

    #[test]
    fn hover_reveals_nothing_while_disabled() {
        let ctx = ctx();
        let mut grounds = Vec::new();
        run(&ctx, Vec::new(), |ui| {
            ListRow::new("d")
                .enabled(false)
                .show_with(ui, |rc, _| grounds.push(rc.ground));
            ListRow::new("e")
                .enabled(false)
                .show_forced(ui, ForceState::HOVER);
        });
        let p = theme::palette(&ctx);
        assert_eq!(grounds, vec![p.standard.pane]);
    }

    /// Override O2 (shell F1): Tab from a section header lands on its action
    /// (the test gives it "Add remote…"),
    /// which stays shown while it has focus, the next Tab leaves it and Shift+Tab comes back to
    /// it; with full and reduced motion.
    #[test]
    fn tab_reaches_a_section_headers_action() {
        let ctx = ctx();
        ctx.enable_accesskit();
        for reduced in [true, false] {
            theme::apply(
                &ctx,
                theme::ThemeId::Midnight,
                theme::Appearance {
                    reduce_motion: reduced,
                    ..theme::Appearance::STILL
                },
            );
            let mut open = true;
            // One frame: the focused node's label, and whether the action's glyph was painted.
            let mut frame = |events: Vec<egui::Event>| {
                let out = run(&ctx, events, |ui| {
                    let _ = SectionHeader::new(format!("Remotes {reduced}"))
                        .count(1)
                        .open(&mut open)
                        .action(icon::PLUS, "Add remote…")
                        .show(ui);
                    let _ = super::super::button::Button::new("After").show(ui);
                });
                let painted = out
                    .shapes
                    .iter()
                    .any(|c| matches!(&c.shape, Shape::Text(t) if t.galley.text() == icon::PLUS));
                let update = out.platform_output.accesskit_update.expect("accesskit");
                let label = update
                    .nodes
                    .iter()
                    .find(|(id, _)| *id == update.focus)
                    .and_then(|(_, n)| n.label().map(str::to_owned));
                (label, painted)
            };
            let _ = frame(Vec::new());
            let mut focused = Vec::new();
            let back = egui::Event::Key {
                key: Key::Tab,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::SHIFT,
            };
            for step in [key(Key::Tab), key(Key::Tab), key(Key::Tab), back] {
                // Focus moves at the end of the frame that read the Tab.
                let _ = frame(vec![step]);
                focused.push(frame(Vec::new()).0);
                // A few quiet frames: the action must not vanish once the header lost focus.
                for _ in 0..3 {
                    let (still, painted) = frame(Vec::new());
                    assert_eq!(&still, focused.last().unwrap(), "focus moved by itself");
                    if still.as_deref() == Some("Add remote…") {
                        assert!(painted, "the focused action is painted (reduced {reduced})");
                    }
                }
            }
            let labels: Vec<_> = focused.iter().map(|l| l.as_deref()).collect();
            assert_eq!(
                labels,
                [
                    Some(
                        super::super::a11y::disclosure_name(&format!("Remotes {reduced}"), true)
                            .as_str()
                    ),
                    Some("Add remote…"),
                    Some("After"),
                    Some("Add remote…")
                ],
                "reduced {reduced}"
            );
        }
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
    }

    /// V22: a disclosure's header icon takes its tone (the checks header's red, amber or green).
    #[test]
    fn a_disclosure_header_icon_keeps_its_tone() {
        let ctx = ctx();
        let p = *theme::palette(&ctx);
        for (tone, color) in [
            (Tone::Danger, p.danger),
            (Tone::Warning, p.warning),
            (Tone::Success, p.success),
        ] {
            let out = run(&ctx, Vec::new(), |ui| {
                let mut open = false;
                let _ = Disclosure::new("checks", "2 checks failed", &mut open)
                    .icon(egui_phosphor::regular::X_CIRCLE)
                    .icon_tone(tone)
                    .show(ui, |_| ());
            });
            let icon = out.shapes.iter().find_map(|c| match &c.shape {
                Shape::Text(t) if t.galley.text() == egui_phosphor::regular::X_CIRCLE => {
                    Some(t.fallback_color)
                }
                _ => None,
            });
            assert_eq!(icon, Some(color), "{tone:?}");
        }
    }
}
