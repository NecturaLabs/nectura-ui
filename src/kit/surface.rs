//! Surfaces (design spec 4.20, 7.1 and the owner's amendments): [`PaneHeader`] (a table pane's
//! header strip, whose top edge is its own colour in Deeper), [`Deck`] (the top bars), [`Card`],
//! [`Banner`], [`Callout`], [`EmptyState`] and [`InsetBox`]. Panes themselves are painted by
//! [`paint::pane`] and [`paint::pane_focus`].

use std::sync::Arc;

use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Layout, Margin, Painter, Rect, Response,
    Sense, Shape, Stroke, Ui, UiBuilder, Widget, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::Shortcut;
use super::button::{Button, LinkButton};
use super::chip::KeyChord;
use super::progress::Skeleton;
use super::{ForceState, Interact, KitResponse, Size, Tone, paint};
use crate::art::{self, Art};
use crate::motion;
use crate::theme::{self, Elevation, Fill, Palette, Style};

/// Seconds since something was first shown in an unbroken run of frames: restarts when a frame
/// passes without it.
fn shown_for(ctx: &egui::Context, id: Id) -> f32 {
    let key = id.with("kit-shown-since");
    let frame = ctx.cumulative_frame_nr();
    let now = ctx.input(|i| i.time);
    let since = ctx.data_mut(|d| {
        let (last, since) = d.get_temp::<(u64, f64)>(key).unwrap_or((u64::MAX, now));
        let since = if last.wrapping_add(1) >= frame && last != u64::MAX {
            since
        } else {
            now
        };
        d.insert_temp(key, (frame, since));
        since
    });
    crate::to_f32(now - since)
}

/// Progress of an appearance over `seconds` (`EASE_OUT`), restarting whenever the thing was not
/// shown in the frame before; 1 at once with reduced motion.
fn appear(ctx: &egui::Context, id: Id, seconds: f32) -> f32 {
    let seconds = motion::time(seconds);
    let elapsed = shown_for(ctx, id);
    if seconds <= 0.0 {
        return 1.0;
    }
    let t = (elapsed / seconds).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    motion::EASE_OUT.at(t)
}

// --------------------------------------------------------------------------------- PaneHeader

/// How tall a pane header is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeaderKind {
    /// A table's column header (`table_header_height`).
    #[default]
    Table,
    /// The detail pane's header (`pane_header_height`).
    Detail,
    /// The diff's file header (`file_header_height`).
    File,
}

/// The header strip at the top of a table pane (History's columns). Standard: the pane itself
/// over a `border` seam, as concept 1. Deeper: a lifted `pane_header` tone over a seam tinted
/// with `focus`, so the table's top edge never reads as one of the neutral pane edges; its top
/// corners follow the card.
#[derive(Debug, Clone, Copy)]
#[must_use = "call .show(ui, ..)"]
pub struct PaneHeader {
    kind: HeaderKind,
    top_of_pane: bool,
}

impl PaneHeader {
    pub fn new(kind: HeaderKind) -> Self {
        Self {
            kind,
            top_of_pane: true,
        }
    }

    /// The header is the first thing in its pane (default): it takes the card's top corners.
    pub fn top_of_pane(mut self, top: bool) -> Self {
        self.top_of_pane = top;
        self
    }

    pub fn height(self) -> f32 {
        let m = theme::metrics();
        match self.kind {
            HeaderKind::Table => m.table_header_height,
            HeaderKind::Detail => m.pane_header_height,
            HeaderKind::File => m.file_header_height,
        }
    }

    /// Paints the strip in `rect` (its seam on the bottom edge).
    pub fn paint(self, painter: &Painter, rect: Rect) {
        let p = theme::palette(painter.ctx());
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let r = if self.top_of_pane && style == Style::Deeper {
            m.pane_radius
        } else {
            0
        };
        let radius = CornerRadius {
            nw: r,
            ne: r,
            sw: 0,
            se: 0,
        };
        painter.rect_filled(rect, radius, s.pane_header);
        paint::seam_h(
            painter,
            rect.x_range(),
            rect.bottom() - 0.5,
            s.pane_header_seam,
        );
    }

    /// Shows the strip across the available width; `add` lays out its content left to right,
    /// vertically centred, 12 pt in.
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> KitResponse<R> {
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, self.height()), Sense::hover());
        self.paint(ui.painter(), rect);
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink2(vec2(theme::MD, 0.0)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        let value = add(&mut child);
        KitResponse { response, value }
    }
}

/// A table column's header label: uppercase, `micro_size` semibold, tracked, `text_faint`.
pub fn column_label(ui: &mut Ui, text: &str) -> Response {
    let p = theme::palette(ui.ctx());
    let g = super::row::caps_galley(ui, text, p.text_faint);
    let (rect, response) = ui.allocate_exact_size(g.size(), Sense::hover());
    ui.painter().galley(rect.min, g, p.text_faint);
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, text));
    response
}

// --------------------------------------------------------------------------------------- Deck

/// The top bars: the repository tab strip and, with a repository open, the action bar.
/// Standard paints each as flat chrome over its own seam. Deeper leaves the tab strip on the
/// window's ground and floats the action bar as a card: the deck's lifted gradient, highlight and
/// shadow with the panes' edge and radius, inset by the pane gutter so the tools sit in line with
/// the cards below.
#[derive(Debug, Clone, Copy)]
#[must_use = "call .paint(painter, rect)"]
pub struct Deck {
    action_bar: bool,
    style: Option<Style>,
}

impl Deck {
    pub fn new(action_bar: bool) -> Self {
        Self {
            action_bar,
            style: None,
        }
    }

    /// Paints in `style` instead of the active one (the gallery's side-by-side strip).
    pub fn style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Deeper's action bar card for a window whose content spans `screen`.
    pub fn action_card(screen: Rect) -> Rect {
        let m = theme::metrics();
        let top = screen.top() + m.tab_strip_height;
        Rect::from_min_max(
            pos2(screen.left() + m.pane_gutter, top),
            pos2(screen.right() - m.pane_gutter, top + m.action_bar_height),
        )
    }

    /// The deck's height in the active metrics.
    pub fn height(self) -> f32 {
        let m = theme::metrics();
        m.tab_strip_height
            + if self.action_bar {
                m.action_bar_height
            } else {
                0.0
            }
    }

    /// Paints the deck across the top of `screen` (over the window's ground, which the caller has
    /// painted); returns the rect the bars take.
    pub fn paint(self, painter: &Painter, screen: Rect) -> Rect {
        let p = theme::palette(painter.ctx());
        let style = self.style.unwrap_or_else(theme::style);
        let s = p.surfaces(style);
        let m = theme::metrics();
        let deck = Rect::from_min_size(screen.min, vec2(screen.width(), self.height()));
        if style == Style::Deeper {
            if self.action_bar {
                paint::surface(
                    painter,
                    Self::action_card(screen),
                    CornerRadius::same(m.pane_radius),
                    p,
                    style,
                    Elevation::Deck,
                    s.deck,
                    s.pane_edge,
                );
            }
        } else {
            // The bar that meets the content loses its seam in a theme that parts regions by
            // tone; the tab strip above the action bar keeps it (both are the deck's fill).
            let strip = Rect::from_min_size(screen.min, vec2(screen.width(), m.tab_strip_height));
            paint::deck_in(painter, strip, p, s, style);
            if self.action_bar {
                let bar = Rect::from_min_size(
                    pos2(screen.left(), strip.bottom()),
                    vec2(screen.width(), m.action_bar_height),
                );
                paint::deck_in(painter, bar, p, s, style);
            }
        }
        deck
    }
}

// --------------------------------------------------------------------------------------- Card

/// A framed group: welcome action cards, settings theme cards, conflict cards. Standard: `panel`
/// with a 1 pt `border` ring and no shadow. Deeper: `pane` with the `pane_edge` ring and the pane
/// shadow. An interactive card hovers (`hover` over its fill, `edge` ring), selects (`focus` ring
/// 1.5 pt and the accent wash) and shows the outside focus ring; it never lifts. An interactive
/// card is announced as a button, or as one choice of a group (a radio button, checked or not)
/// once [`Self::selected`] is set; cards in a grid share one Tab stop through [`Self::roving`].
#[derive(Debug, Clone)]
#[must_use = "call .show(ui, ..)"]
pub struct Card {
    id_salt: Option<Id>,
    interactive: bool,
    selected: Option<bool>,
    roving: Option<(super::row::Roving, usize)>,
    enabled: bool,
    ring: Option<Tone>,
    width: Option<f32>,
    compact: bool,
    label: Option<String>,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    force: Option<ForceState>,
}

impl Card {
    pub fn new() -> Self {
        Self {
            id_salt: None,
            interactive: false,
            selected: None,
            roving: None,
            enabled: true,
            ring: None,
            width: None,
            compact: false,
            label: None,
            tooltip: None,
            disabled_reason: None,
            force: None,
        }
    }
    /// The card is one button: hover, press, focus, and `clicked()` on its response.
    pub fn interactive(mut self, on: bool) -> Self {
        self.interactive = on;
        self
    }
    /// The card is one choice of a group (the theme cards), chosen or not.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = Some(selected);
        self
    }
    /// The card is card `index` of the grid `list`: the grid is one Tab stop, and the arrows move
    /// between its cards (give the [`super::row::Roving`] its `columns`).
    pub fn roving(mut self, list: super::row::Roving, index: usize) -> Self {
        self.roving = Some((list, index));
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// A ring in the tone's colour (conflict cards: warning unresolved, success resolved).
    pub fn ring(mut self, tone: Tone) -> Self {
        self.ring = Some(tone);
        self
    }
    /// The card's outer width (default: what its content needs).
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    /// Tight padding (`SM` instead of `card_pad`) for small cards that hold a picture more than
    /// text: the settings theme cards, swatches.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }
    /// The AccessKit name of an interactive card.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
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
    pub(crate) fn force(mut self, force: ForceState) -> Self {
        self.force = Some(force);
        self
    }

    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> KitResponse<R> {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let sense = match (self.interactive, enabled, self.roving) {
            (true, true, Some((list, index))) if !list.takes_tab(&ctx, index, None) => Sense::CLICK,
            (true, true, _) => Sense::click(),
            _ => Sense::hover(),
        };
        let slot = ui.painter().add(Shape::Noop);
        let mut builder = UiBuilder::new()
            .sense(sense)
            .layout(Layout::top_down(Align::Min));
        if let Some(id) = self.id_salt {
            builder = builder.id_salt(id);
        }
        // A disabled card's content sits in a disabled `Ui`, so the kit controls in it paint the
        // disabled tokens and nothing in it answers a click (a disabled `Ui` is not faded twice).
        let off = !enabled && self.interactive;
        if off && ui.is_enabled() {
            builder = builder.disabled();
        }
        let pad = if self.compact { theme::SM } else { m.card_pad };
        let inner = ui.scope_builder(builder, |ui| {
            egui::Frame::new()
                .inner_margin(Margin::same(crate::points_i8(pad)))
                .show(ui, |ui| {
                    if let Some(w) = self.width {
                        ui.set_width((w - 2.0 * pad).max(0.0));
                    }
                    add(ui)
                })
                .inner
        });
        let response = inner.response;
        let rect = response.rect;
        let st = if self.interactive {
            Interact::of(&ctx, &response, enabled, self.force)
        } else {
            Interact {
                hover: 0.0,
                press: 0.0,
                held: false,
                focus: 0.0,
            }
        };
        let radius = CornerRadius::same(m.card_radius);
        let (base, rest_ring, level) = if off {
            // Disabled: the flat `disabled_fill` in its `disabled_edge` ring, like a disabled
            // button; no elevation, selection or hover.
            (s.disabled_fill, s.disabled_edge, None)
        } else if style == Style::Deeper {
            (
                s.pane,
                s.pane_edge.unwrap_or(s.border),
                Some(Elevation::Pane),
            )
        } else {
            (s.panel, s.border, None)
        };
        let selected = self.selected == Some(true) && !off;
        let mut fill = base;
        if selected {
            fill = theme::over(p.accent_tint(), fill);
        }
        let lit = st.hover.max(st.press);
        if lit > 0.0 {
            let over = p
                .hover()
                .lerp_to_gamma(p.pressed(), st.press)
                .gamma_multiply(lit);
            fill = theme::over(over, fill);
        }
        let ring = match (selected, self.ring) {
            (true, _) => None,
            (false, Some(t)) if !off => Some(t.color(p)),
            (false, _) => Some(rest_ring.lerp_to_gamma(s.edge, lit)),
        };
        let painter = &super::content_painter(ui);
        let mut shapes = if let Some(level) = level {
            paint::surface_shapes(
                painter,
                rect,
                radius,
                p,
                style,
                level,
                Fill::flat(fill),
                ring,
            )
        } else {
            let mut v = vec![Shape::rect_filled(rect, radius, fill)];
            if let Some(c) = ring {
                v.push(Shape::rect_stroke(
                    rect,
                    radius,
                    Stroke::new(paint::SEAM, c),
                    egui::StrokeKind::Inside,
                ));
            }
            v
        };
        if selected {
            shapes.push(Shape::rect_stroke(
                rect,
                radius,
                Stroke::new(paint::INSET_FOCUS_WIDTH, p.focus),
                egui::StrokeKind::Inside,
            ));
        }
        painter.set(slot, Shape::Vec(shapes));
        if st.focus > 0.0 {
            paint::focus_ring(
                painter,
                rect,
                radius,
                p.focus.gamma_multiply(st.focus),
                Color32::TRANSPARENT,
            );
        }
        if self.interactive {
            let label = self.label.clone().unwrap_or_default();
            response.widget_info(|| match self.selected {
                Some(on) => WidgetInfo::selected(WidgetType::RadioButton, enabled, on, &label),
                None => WidgetInfo::labeled(WidgetType::Button, enabled, &label),
            });
            if let Some((list, index)) = self.roving
                && enabled
            {
                list.row(ui, &response, index, false);
            }
        }
        let response = super::button::tip(
            response,
            enabled,
            self.tooltip.as_deref(),
            self.disabled_reason.as_deref(),
            None,
            true,
        );
        KitResponse {
            response,
            value: inner.inner,
        }
    }
}

impl Default for Card {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------------------------------- Banner

fn tone_glyph(tone: Tone) -> &'static str {
    match tone {
        Tone::Neutral | Tone::Accent => icon::INFO,
        Tone::Success => icon::CHECK_CIRCLE,
        Tone::Warning => icon::WARNING,
        Tone::Danger => icon::WARNING_OCTAGON,
    }
}

/// What a callout in `tone` is called to screen readers, which do not see its icon.
fn tone_name(tone: Tone) -> &'static str {
    match tone {
        Tone::Neutral | Tone::Accent => "Note",
        Tone::Success => "Success",
        Tone::Warning => "Warning",
        Tone::Danger => "Error",
    }
}

/// One action of a [`Banner`] or a [`Callout`]: a ghost button with an optional glyph, which may
/// be unavailable for a reason its tooltip gives ("Resolve the 2 conflicts first").
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct NoticeAction {
    label: String,
    icon: Option<String>,
    icon_tone: Option<Tone>,
    enabled: bool,
    disabled_reason: Option<String>,
}

impl NoticeAction {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            icon_tone: None,
            enabled: true,
            disabled_reason: None,
        }
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    /// The glyph in a tone's colour (`Danger` for "Abort merge…").
    pub fn icon_tone(mut self, tone: Tone) -> Self {
        self.icon_tone = Some(tone);
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// Why it is unavailable, shown on hover while disabled.
    pub fn disabled_reason(mut self, text: impl Into<String>) -> Self {
        self.disabled_reason = Some(text.into());
        self
    }
    /// The ghost button, `Sm`, salted by `salt`.
    fn show(&self, ui: &mut Ui, salt: Id) -> Response {
        let mut b = Button::new(self.label.clone())
            .ghost()
            .size(Size::Sm)
            .enabled(self.enabled)
            .id_salt(salt);
        if let Some(g) = &self.icon {
            b = b.icon(g);
        }
        if let Some(t) = self.icon_tone {
            b = b.icon_tone(t);
        }
        if let Some(r) = &self.disabled_reason {
            b = b.disabled_reason(r.as_str());
        }
        b.show(ui)
    }
}

impl From<&str> for NoticeAction {
    fn from(label: &str) -> Self {
        Self::new(label)
    }
}

impl From<String> for NoticeAction {
    fn from(label: String) -> Self {
        Self::new(label)
    }
}

/// A full-width notice at the top of a view (a merge or rebase in progress): the tone's tint
/// over the chrome, a seam in the tone below, icon, title, body and up to two ghost actions at
/// the right, each of which may be disabled with a reason. It grows in over `BASE` when it
/// appears. Every view shows a repository state with this one layout.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui)"]
pub struct Banner {
    tone: Tone,
    title: String,
    body: Option<String>,
    icon: Option<String>,
    actions: Vec<NoticeAction>,
    id_salt: Id,
    force: Option<ForceState>,
}

impl Banner {
    pub fn new(tone: Tone, title: impl Into<String>) -> Self {
        let title = title.into();
        Self {
            tone,
            id_salt: Id::new(("kit-banner", title.clone())),
            title,
            body: None,
            icon: None,
            actions: Vec::new(),
            force: None,
        }
    }
    pub fn body(mut self, text: impl Into<String>) -> Self {
        self.body = Some(text.into());
        self
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    /// An action button at the right (at most two; the first is the main one, rightmost): a
    /// label, or a [`NoticeAction`] with a glyph or a reason it is unavailable.
    pub fn action(mut self, action: impl Into<NoticeAction>) -> Self {
        if self.actions.len() < 2 {
            self.actions.push(action.into());
        }
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Id::new(salt);
        self
    }
    pub(crate) fn force(mut self, force: ForceState) -> Self {
        self.force = Some(force);
        self
    }

    /// Shows the banner; the value is the index of the action clicked.
    pub fn show(self, ui: &mut Ui) -> KitResponse<Option<usize>> {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let m = theme::metrics();
        let id = ui.id().with(self.id_salt);
        let grow = match self.force.and_then(|f| f.progress) {
            Some(t) => t,
            None => appear(&ctx, id, motion::BASE),
        };
        let full = m.pane_header_height;
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, full * grow), Sense::hover());
        let mut clicked = None;
        if rect.height() > 0.0 {
            let tone_c = self.tone.color(p);
            let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
            painter.rect_filled(
                rect,
                CornerRadius::ZERO,
                theme::over(self.tone.tint(p), s.panel),
            );
            paint::seam_h(
                &painter,
                rect.x_range(),
                rect.bottom() - 0.5,
                Palette::tint(tone_c, 90),
            );
            // The content slides down with the growing edge.
            let full_rect =
                Rect::from_min_size(pos2(rect.left(), rect.bottom() - full), vec2(width, full));
            let cy = full_rect.center().y;
            let mut x = full_rect.left() + theme::MD;
            let glyph = self.icon.as_deref().unwrap_or(tone_glyph(self.tone));
            let g =
                painter.layout_no_wrap(glyph.to_owned(), FontId::proportional(m.icon_lg), tone_c);
            painter.galley(pos2(x, cy - g.size().y / 2.0), Arc::clone(&g), tone_c);
            x += g.size().x + theme::SM;
            let t = painter.layout_no_wrap(
                self.title.clone(),
                FontId::new(m.body_size, theme::semibold()),
                p.text,
            );
            painter.galley(pos2(x, cy - t.size().y / 2.0), Arc::clone(&t), p.text);
            x += t.size().x + theme::SM;
            let mut child = ui.new_child(
                UiBuilder::new()
                    .max_rect(full_rect.shrink2(vec2(theme::SM, 0.0)))
                    .layout(Layout::right_to_left(Align::Center)),
            );
            child.set_clip_rect(rect.intersect(ui.clip_rect()));
            for (i, action) in self.actions.iter().enumerate() {
                if action.show(&mut child, self.id_salt.with(i)).clicked() {
                    clicked = Some(i);
                }
            }
            let right = child.min_rect().left() - theme::SM;
            if let Some(body) = &self.body {
                let b = crate::text::single_line_galley(
                    ui,
                    body.clone(),
                    FontId::proportional(m.body_size),
                    p.text_dim,
                    (right - x).max(0.0),
                );
                painter.galley(pos2(x, cy - b.size().y / 2.0), b, p.text_dim);
            }
        }
        let spoken = match &self.body {
            Some(b) => format!("{}: {b}", self.title),
            None => self.title.clone(),
        };
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &spoken));
        KitResponse {
            response,
            value: clicked,
        }
    }
}

// ------------------------------------------------------------------------------------ Callout

/// What [`Callout::show_with`] draws between the text and the actions.
type CalloutContent<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

/// Padding inside a callout (spec 4.20: 10 x 12) and an inset box (8 x 10).
const CALLOUT_PAD: Margin = Margin::symmetric(12, 10);
const INSET_PAD: Margin = Margin::symmetric(10, 8);
/// Gap between an empty state's keys and what they do (as between two chords of a sequence).
const HINT_GAP: f32 = 6.0;

/// An inline note in a pane or dialog: the tone's tint, an icon, optional title, wrapped text and
/// up to two ghost actions under it, their labels in line with the text. No side bar.
#[derive(Debug, Clone)]
#[must_use = "call .show(ui)"]
pub struct Callout {
    tone: Tone,
    text: String,
    title: Option<String>,
    icon: Option<String>,
    actions: Vec<NoticeAction>,
    id_salt: Option<Id>,
}

impl Callout {
    pub fn new(tone: Tone, text: impl Into<String>) -> Self {
        Self {
            tone,
            text: text.into(),
            title: None,
            icon: None,
            actions: Vec::new(),
            id_salt: None,
        }
    }
    /// An action under the text (at most two; the first is the main one, leftmost).
    pub fn action(mut self, action: impl Into<NoticeAction>) -> Self {
        if self.actions.len() < 2 {
            self.actions.push(action.into());
        }
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        self.show_actions(ui).response
    }

    /// Shows the callout; the value is the index of the action clicked.
    pub fn show_actions(self, ui: &mut Ui) -> KitResponse<Option<usize>> {
        self.show_inner(ui, None)
    }

    /// Shows the callout with `add`'s content between its text and its actions (a short list,
    /// keys and what they do); the value is the index of the action clicked.
    pub fn show_with(self, ui: &mut Ui, add: impl FnOnce(&mut Ui)) -> KitResponse<Option<usize>> {
        self.show_inner(ui, Some(Box::new(add)))
    }

    fn show_inner(
        self,
        ui: &mut Ui,
        content: Option<CalloutContent<'_>>,
    ) -> KitResponse<Option<usize>> {
        let p = theme::palette(ui.ctx());
        let m = theme::metrics();
        let tone_c = self.tone.color(p);
        let glyph = self
            .icon
            .clone()
            .unwrap_or_else(|| tone_glyph(self.tone).to_owned());
        let salt = self
            .id_salt
            .unwrap_or_else(|| Id::new(("kit-callout", &self.text)));
        let mut clicked = None;
        let response = egui::Frame::new()
            .fill(self.tone.tint(p))
            .corner_radius(CornerRadius::same(m.radius))
            .inner_margin(CALLOUT_PAD)
            .show(ui, |ui| {
                // A group named for the tone, holding the title, text and actions: the icon is
                // painted, not spoken (a private-use glyph screen readers would spell out).
                super::a11y::group(ui, egui::accesskit::Role::Group, tone_name(self.tone));
                ui.horizontal_top(|ui| {
                    ui.spacing_mut().item_spacing.x = theme::SM;
                    let icon =
                        ui.painter()
                            .layout_no_wrap(glyph, FontId::proportional(m.icon_lg), tone_c);
                    let (at, _) = ui.allocate_exact_size(icon.size(), Sense::hover());
                    ui.painter().galley(at.min, icon, tone_c);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        if let Some(t) = &self.title {
                            ui.label(
                                egui::RichText::new(t)
                                    .font(FontId::new(m.body_size, theme::semibold()))
                                    .color(p.text),
                            );
                        }
                        if !self.text.is_empty() {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(&self.text)
                                        .size(m.body_size)
                                        .color(p.text),
                                )
                                .wrap(),
                            );
                        }
                        if let Some(add) = content {
                            ui.add_space(theme::XS);
                            add(ui);
                        }
                        if !self.actions.is_empty() {
                            ui.add_space(theme::XS);
                            // Ghost buttons pulled left so their labels line up with the text.
                            let mut row = ui.available_rect_before_wrap();
                            row.min.x -= m.button_pad_x_sm - 2.0;
                            row.set_height(Size::Sm.height());
                            ui.scope_builder(
                                UiBuilder::new()
                                    .max_rect(row)
                                    .layout(Layout::left_to_right(Align::Center)),
                                |ui| {
                                    ui.spacing_mut().item_spacing.x = 2.0;
                                    for (i, action) in self.actions.iter().enumerate() {
                                        if action.show(ui, salt.with(i)).clicked() {
                                            clicked = Some(i);
                                        }
                                    }
                                },
                            );
                        }
                    });
                });
            })
            .response;
        KitResponse {
            response,
            value: clicked,
        }
    }
}

impl Widget for Callout {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ----------------------------------------------------------------------------------- InsetBox

/// A recessed box for code and values in dialogs: `field` fill, `border` ring, `radius_sm`,
/// padding 8 by 10.
#[derive(Debug, Clone, Copy, Default)]
#[must_use = "call .show(ui, ..)"]
pub struct InsetBox;

impl InsetBox {
    pub fn new() -> Self {
        Self
    }

    #[expect(
        clippy::unused_self,
        reason = "`InsetBox::new().show(ui, ..)` reads like every other kit builder"
    )]
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> KitResponse<R> {
        let s = theme::surfaces(ui.ctx());
        let m = theme::metrics();
        let r = egui::Frame::new()
            .fill(s.field)
            .stroke(Stroke::new(paint::SEAM, s.border))
            .corner_radius(CornerRadius::same(m.radius_sm))
            .inner_margin(INSET_PAD)
            .show(ui, add);
        KitResponse {
            response: r.response,
            value: r.inner,
        }
    }
}

// --------------------------------------------------------------------------------- EmptyState

/// What an empty state's buttons did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyAction {
    /// The primary action (Clear filters, Retry, the first-run call to action).
    Primary,
    /// The link under it (Details, Learn more).
    Link,
}

/// A pane with nothing to show: art (or, in a compact pane, an icon), a title, at most two lines
/// of body, a primary action, an optional link and keycap hints, centred a little above the
/// middle. The art grows in once (still with reduced motion).
#[derive(Debug, Clone)]
#[must_use = "call .show(ui)"]
pub struct EmptyState {
    title: String,
    body: Option<String>,
    art: Option<Art>,
    icon: Option<String>,
    action: Option<String>,
    action_min_width: f32,
    link: Option<String>,
    hints: Vec<(Shortcut, String)>,
    compact: bool,
    id_salt: Id,
}

impl EmptyState {
    pub fn new(title: impl Into<String>) -> Self {
        let title = title.into();
        Self {
            id_salt: Id::new(("kit-empty", title.clone())),
            title,
            body: None,
            art: None,
            icon: None,
            action: None,
            action_min_width: 0.0,
            link: None,
            hints: Vec::new(),
            compact: false,
        }
    }

    /// A filtered list with no match: "Clear filters".
    pub fn filtered(title: impl Into<String>) -> Self {
        Self::new(title)
            .icon(icon::FUNNEL_X)
            .body("Nothing matches the current filters.")
            .action("Clear filters")
    }

    /// Something failed: the error art, `message` as the body, Retry and Details.
    pub fn error(title: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(title)
            .art(art::empty_error())
            .icon(icon::WARNING_OCTAGON)
            .body(message)
            .action("Retry")
            .link("Details")
    }

    pub fn body(mut self, text: impl Into<String>) -> Self {
        self.body = Some(text.into());
        self
    }
    /// The illustration (`ui/art.rs`), 96 pt tall.
    pub fn art(mut self, art: Art) -> Self {
        self.art = Some(art);
        self
    }
    /// The glyph shown instead of the art in compact panes (and when there is no art).
    pub fn icon(mut self, glyph: &str) -> Self {
        self.icon = Some(glyph.to_owned());
        self
    }
    pub fn action(mut self, label: impl Into<String>) -> Self {
        self.action = Some(label.into());
        self
    }
    /// Minimum width of the primary action, bounded by the available pane.
    pub fn action_min_width(mut self, width: f32) -> Self {
        self.action_min_width = width.max(0.0);
        self
    }
    pub fn link(mut self, label: impl Into<String>) -> Self {
        self.link = Some(label.into());
        self
    }
    /// A keycap hint under the actions: the command's keys, then `what` it does.
    pub fn hint(mut self, command: impl Into<Shortcut>, what: impl Into<String>) -> Self {
        self.hints.push((command.into(), what.into()));
        self
    }
    /// A small pane: the icon instead of the art.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Id::new(salt);
        self
    }

    /// Fills the available space; the value is the button clicked, if any.
    pub fn show(self, ui: &mut Ui) -> KitResponse<Option<EmptyAction>> {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let m = theme::metrics();
        let outer = ui.available_rect_before_wrap();
        let id = ui.id().with(self.id_salt);
        let key = id.with("kit-empty-height");
        let width = outer.width().min(MAX_WIDTH);
        let measured: f32 = ctx.data(|d| d.get_temp(key)).unwrap_or(0.0);
        let top = (outer.center().y - measured / 2.0 - outer.height() * 0.06).max(outer.top());
        let area = Rect::from_min_size(
            pos2(outer.center().x - width / 2.0, top),
            vec2(width, outer.bottom() - top),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(id)
                .max_rect(area)
                .layout(Layout::top_down(Align::Center)),
        );
        let mut clicked = None;
        child.spacing_mut().item_spacing.y = theme::SM;
        let compact = self.compact || outer.height() < 260.0;
        match (self.art, compact) {
            (Some(a), false) => {
                art::show(&mut child, a, vec2(128.0, 96.0));
            }
            _ => {
                if let Some(g) = &self.icon {
                    child.label(egui::RichText::new(g).size(m.icon_hero).color(p.text_faint));
                }
            }
        }
        child.add_space(theme::XS);
        child.label(
            egui::RichText::new(&self.title)
                .font(theme::heading(m.subtitle_size))
                .color(p.text),
        );
        if let Some(body) = &self.body {
            let mut job = egui::text::LayoutJob::simple(
                body.clone(),
                FontId::proportional(m.body_size),
                p.text_dim,
                width,
            );
            job.wrap.max_rows = 2;
            job.halign = Align::Center;
            child.add(egui::Label::new(job));
        }
        if self.action.is_some() || self.link.is_some() {
            child.add_space(theme::XS);
        }
        if let Some(a) = &self.action
            && Button::new(a.clone())
                .primary()
                .min_width(self.action_min_width.min(width))
                .id_salt((self.id_salt, "action"))
                .show(&mut child)
                .clicked()
        {
            clicked = Some(EmptyAction::Primary);
        }
        if let Some(l) = &self.link
            && LinkButton::new(l.clone()).show(&mut child).clicked()
        {
            clicked = Some(EmptyAction::Link);
        }
        for (command, what) in &self.hints {
            child.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = HINT_GAP;
                KeyChord::command(command.clone()).show(ui);
                ui.label(
                    egui::RichText::new(what)
                        .size(m.meta_size)
                        .color(p.text_faint),
                );
            });
        }
        let used = child.min_rect().height();
        if (used - measured).abs() > 0.5 {
            // Centred on last frame's height: lay out again with this one.
            ctx.data_mut(|d| d.insert_temp(key, used));
            ctx.request_discard("empty state measured");
        }
        let response = ui.allocate_rect(outer, Sense::hover());
        let spoken = match &self.body {
            Some(b) => format!("{}. {b}", self.title),
            None => self.title.clone(),
        };
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, &spoken));
        KitResponse {
            response,
            value: clicked,
        }
    }
}

/// An empty state's content column is at most this wide.
const MAX_WIDTH: f32 = 340.0;

// ------------------------------------------------------------------------------------ gallery

/// A gallery card: its name, forced state, selected, tone ring and enabled.
type CardCase<'a> = (&'a str, Option<ForceState>, bool, Option<Tone>, bool);

/// The gallery's Surfaces section: cards in every state, banners and callouts per tone, an empty
/// state, an inset box, skeleton rows, pane frames (flush, and floating with and without the
/// focused edge), the table header strip and the top bars in both styles.
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
    small(
        ui,
        "Cards: static, interactive rest, hover, pressed, selected, focus, warning ring, disabled",
    );
    let card_body = |ui: &mut Ui, title: &str| {
        ui.label(
            egui::RichText::new(title)
                .font(theme::heading(m.subtitle_size))
                .color(p.text),
        );
        ui.label(
            egui::RichText::new("Open a folder")
                .size(m.meta_size)
                .color(p.text_dim),
        );
    };
    let w = 132.0;
    let states: [CardCase<'_>; 8] = [
        ("Static", None, false, None, true),
        ("Rest", None, false, None, true),
        ("Hover", Some(ForceState::HOVER), false, None, true),
        ("Pressed", Some(ForceState::PRESSED), false, None, true),
        ("Selected", None, true, None, true),
        ("Focus", Some(ForceState::FOCUS), false, None, true),
        ("Unresolved", None, false, Some(Tone::Warning), true),
        ("Disabled", None, false, None, false),
    ];
    // Cards size to their content, so a row cannot wrap them: four to a row.
    for (r, chunk) in states.chunks(4).enumerate() {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = vec2(theme::SM, theme::SM);
            for (j, (name, force, selected, ring, enabled)) in chunk.iter().enumerate() {
                let i = r * 4 + j;
                if i == 0 {
                    Card::new()
                        .width(w)
                        .tooltip("A static card")
                        .show(ui, |ui| card_body(ui, name));
                    continue;
                }
                let mut c = Card::new()
                    .interactive(true)
                    .enabled(*enabled)
                    .width(w)
                    .label(*name)
                    .disabled_reason("Not available")
                    .id_salt(("gallery-card", i));
                if *selected {
                    c = c.selected(true);
                }
                if let Some(t) = ring {
                    c = c.ring(*t);
                }
                if let Some(f) = force {
                    c = c.force(*f);
                }
                c.show(ui, |ui| card_body(ui, name));
            }
        });
        ui.add_space(theme::SM);
    }
    small(
        ui,
        "Choice cards: one Tab stop, the arrows move between them, the chosen one is checked",
    );
    let chosen_id = ui.id().with("gallery-choice");
    let mut chosen: usize = ui.ctx().data(|d| d.get_temp(chosen_id)).unwrap_or(0);
    let choices = ["Midnight", "Daylight", "Observatory"];
    let grid = super::row::Roving::new(ui.ctx(), "gallery-choice", choices.len(), Some(chosen))
        .columns(choices.len());
    ui.horizontal_top(|ui| {
        super::a11y::group(ui, egui::accesskit::Role::RadioGroup, "Gallery theme");
        ui.spacing_mut().item_spacing = vec2(theme::SM, theme::SM);
        for (i, name) in choices.into_iter().enumerate() {
            let r = Card::new()
                .interactive(true)
                .selected(chosen == i)
                .roving(grid, i)
                .width(w)
                .label(format!("{name} theme"))
                .id_salt(("gallery-choice", i))
                .show(ui, |ui| card_body(ui, name));
            if r.response.clicked() {
                chosen = i;
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(chosen_id, chosen));
    ui.add_space(theme::SM);
    small(ui, "Banners per tone");
    for tone in Tone::ALL {
        let mut b = Banner::new(tone, format!("{} banner", tone.label()))
            .body("A merge is in progress on main.")
            .id_salt(("gallery-banner", tone));
        if tone == Tone::Warning {
            b = b
                .action(
                    NoticeAction::new("Continue")
                        .enabled(false)
                        .disabled_reason("Resolve the 2 conflicts first"),
                )
                .action(
                    NoticeAction::new("Abort merge…")
                        .icon(icon::X_CIRCLE)
                        .icon_tone(Tone::Danger),
                );
        }
        b.show(ui);
    }
    small(
        ui,
        "Banner growing in (0.5, its full height outlined), custom icon",
    );
    {
        // The half-grown banner sits in a slot of its full height, outlined, so it reads as
        // growing rather than cut by the caption above.
        let full = m.pane_header_height;
        let (slot, _) = ui.allocate_exact_size(vec2(ui.available_width(), full), Sense::hover());
        let s = theme::surfaces(ui.ctx());
        ui.painter().rect_stroke(
            slot,
            CornerRadius::ZERO,
            Stroke::new(paint::SEAM, s.border),
            egui::StrokeKind::Inside,
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(slot)
                .layout(Layout::top_down(Align::Min)),
        );
        let _ = Banner::new(Tone::Accent, "Rebasing feature/auth")
            .icon(icon::GIT_PULL_REQUEST)
            .body("Step 2 of 5")
            .id_salt("gallery-banner-grow")
            .force(ForceState {
                progress: Some(0.5),
                ..ForceState::default()
            })
            .show(&mut child);
    }
    small(ui, "Callouts per tone");
    for tone in [Tone::Accent, Tone::Success, Tone::Warning, Tone::Danger] {
        let c = Callout::new(
            tone,
            "Canopy signs commits with the key your Git configuration names.",
        )
        .title(tone.label())
        .id_salt(("gallery-callout", tone));
        match tone {
            Tone::Accent => {
                c.icon(icon::SEAL_CHECK).show(ui);
            }
            Tone::Warning => {
                let _ = c
                    .action("Open settings")
                    .action(
                        NoticeAction::new("Sign now")
                            .enabled(false)
                            .disabled_reason("No signing key is set"),
                    )
                    .show_actions(ui);
            }
            _ => {
                c.show(ui);
            }
        }
    }
    small(ui, "Compact card");
    Card::new()
        .compact(true)
        .width(132.0)
        .show(ui, |ui| card_body(ui, "Compact"));
    small(ui, "Inset box, skeleton rows");
    InsetBox::new().show(ui, |ui| {
        ui.label(
            egui::RichText::new("git push --force-with-lease origin main")
                .font(FontId::monospace(m.mono_size))
                .color(p.text),
        );
    });
    Skeleton::rows(3)
        .row_height(super::row::RowKind::History.height())
        .show_forced(
            ui,
            ForceState {
                progress: Some(0.25),
                ..ForceState::default()
            },
        );
    let last_id = ui.id().with("gallery-empty-last");
    let last: Option<String> = ui.ctx().data(|d| d.get_temp(last_id));
    small(
        ui,
        &format!(
            "Empty states: filtered, error with art, compact first run (last action: {})",
            last.as_deref().unwrap_or("none")
        ),
    );
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 300.0), Sense::hover());
    let third = (rect.width() - 2.0 * theme::SM) / 3.0;
    let empties = [
        EmptyState::filtered("No commits match")
            .hint("Ctrl+K", "Search commands")
            .id_salt("gallery-empty-filtered"),
        EmptyState::error("Could not load history", "git log exited with status 128.")
            .id_salt("gallery-empty-error"),
        EmptyState::new("No repository")
            .art(art::empty_history())
            .icon(icon::FOLDER_OPEN)
            .body("Open a folder to begin.")
            .action("Open…")
            .link("Clone instead")
            .compact(true)
            .id_salt("gallery-empty-compact"),
    ];
    for (i, empty) in empties.into_iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(
                rect.left() + crate::count_f32(i) * (third + theme::SM),
                rect.top(),
            ),
            vec2(third, rect.height()),
        );
        let s = theme::surfaces(ui.ctx());
        let fill = if i == 2 { s.panel } else { s.pane };
        let shown = gallery_pane(
            ui,
            r,
            fill,
            0.0,
            Margin::same(crate::points_i8(theme::SM)),
            |ui| empty.show(ui).value,
        );
        // Standard's flush panes part only where they meet; here each sits apart, so its seams
        // run all the way round.
        if theme::style() == Style::Standard {
            paint::rim(ui.painter(), r, CornerRadius::ZERO, s.border);
        }
        if let Some(action) = shown {
            ui.ctx()
                .data_mut(|d| d.insert_temp(last_id, format!("{action:?} in pane {i}")));
        }
    }
    small(
        ui,
        "Panes: Standard and Deeper, focused and not, with the table header strip",
    );
    ui.horizontal_top(|ui| {
        let density = theme::density();
        for style in Style::ALL {
            for focused in [false, true] {
                theme::with_look(style, density, || pane_sample(ui, style, focused));
            }
        }
    });
    small(ui, "Detail and file headers (not at the top of a pane)");
    PaneHeader::new(HeaderKind::Detail)
        .top_of_pane(false)
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("docs: add API reference")
                    .font(FontId::new(m.body_size, theme::semibold()))
                    .color(p.text),
            );
        });
    PaneHeader::new(HeaderKind::File)
        .top_of_pane(false)
        .show(ui, |ui| column_label(ui, "api.md"));
    small(ui, "Top bars: Standard and Deeper");
    let density = theme::density();
    for style in Style::ALL {
        theme::with_look(style, density, || deck_sample(ui, style));
    }
}

/// A pane in the gallery, painted as the window paints its content pane: [`paint::pane`] over
/// `card` in `fill`, `add` inside `margin` and clipped to the card, and in Deeper the
/// focused-pane edge at `focus` (0 to 1) over the content, so a header strip cannot hide it.
fn gallery_pane<R>(
    ui: &mut Ui,
    card: Rect,
    fill: Color32,
    focus: f32,
    margin: Margin,
    add: impl FnOnce(&mut Ui) -> R,
) -> R {
    let p = theme::palette(ui.ctx());
    let painter = ui.painter().clone();
    let radius = paint::pane(&painter, card, p, fill, 0.0);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(card - margin)
            .layout(Layout::top_down(Align::Min)),
    );
    let clip = card.shrink(f32::from(radius.nw) * 0.3);
    child.set_clip_rect(clip.intersect(ui.clip_rect()));
    let value = add(&mut child);
    if theme::style() == Style::Deeper {
        paint::pane_focus(&painter, card, radius, p.focus, focus);
    }
    value
}

/// A miniature window corner: ground, a pane with the table header strip and two rows.
fn pane_sample(ui: &mut Ui, style: Style, focused: bool) {
    let p = theme::palette(ui.ctx());
    let s = p.surfaces(style);
    let m = theme::metrics();
    let (outer, _) = ui.allocate_exact_size(vec2(140.0, 120.0), Sense::hover());
    ui.painter().rect_filled(outer, CornerRadius::ZERO, s.bg);
    let card = outer.shrink(m.pane_gutter);
    let focus = f32::from(u8::from(focused));
    gallery_pane(ui, card, s.pane, focus, Margin::ZERO, |ui| {
        PaneHeader::new(HeaderKind::Table).show(ui, |ui| {
            column_label(ui, if focused { "Focused" } else { "Graph" });
        });
        for i in 0..2 {
            let (r, _) = ui.allocate_exact_size(
                vec2(ui.available_width(), m.history_row_height),
                Sense::hover(),
            );
            if i == 0 {
                ui.painter()
                    .rect_filled(r, CornerRadius::ZERO, p.selected());
            }
            ui.painter().text(
                r.left_center() + vec2(theme::MD, 0.0),
                Align2::LEFT_CENTER,
                format!("{} row", style.label()),
                FontId::proportional(m.body_size),
                p.text,
            );
        }
    });
}

/// The top bars of a window in `style`: tab strip (a tab, the search trigger) and the action bar.
fn deck_sample(ui: &mut Ui, style: Style) {
    let p = theme::palette(ui.ctx());
    let s = p.surfaces(style);
    let m = theme::metrics();
    let deck = Deck::new(true).style(style);
    let width = ui.available_width();
    let (outer, _) = ui.allocate_exact_size(vec2(width, deck.height() + 24.0), Sense::hover());
    let painter = ui.painter().with_clip_rect(outer.intersect(ui.clip_rect()));
    painter.rect_filled(outer, CornerRadius::ZERO, s.bg);
    let rect = deck.paint(&painter, outer);
    let tab = Rect::from_min_size(
        pos2(
            rect.left() + theme::MD,
            rect.top() + (m.tab_strip_height - m.doc_tab_height) / 2.0,
        ),
        vec2(130.0, m.doc_tab_height),
    );
    paint::surface(
        &painter,
        tab,
        CornerRadius::same(m.doc_tab_radius),
        p,
        style,
        Elevation::Tab,
        s.tab,
        Some(s.tab_edge),
    );
    painter.text(
        tab.left_center() + vec2(theme::SM + 2.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{} showcase", icon::FOLDER_SIMPLE),
        FontId::new(m.body_size, theme::medium()),
        p.text,
    );
    let search = Rect::from_min_size(
        pos2(
            rect.right() - theme::MD - m.search_trigger.x,
            rect.top() + (m.tab_strip_height - m.search_trigger.y) / 2.0,
        ),
        m.search_trigger,
    );
    paint::surface(
        &painter,
        search,
        CornerRadius::same(m.radius),
        p,
        style,
        Elevation::Field,
        Fill::flat(s.field),
        Some(s.edge),
    );
    painter.text(
        search.left_center() + vec2(theme::SM + 2.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{}  Search commands", icon::MAGNIFYING_GLASS),
        FontId::proportional(m.body_size),
        p.text_faint,
    );
    let bar = Rect::from_min_max(pos2(rect.left(), rect.top() + m.tab_strip_height), rect.max);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(bar.shrink2(vec2(theme::MD, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.spacing_mut().item_spacing.x = m.toolbar_gap;
    for (i, (glyph, label)) in [
        (icon::ARROWS_CLOCKWISE, "Fetch"),
        (icon::ARROW_DOWN, "Pull"),
        (icon::ARROW_UP, "Push"),
    ]
    .into_iter()
    .enumerate()
    {
        let t = super::ToolButton::new(glyph, label);
        if i == 1 {
            t.show_forced(&mut child, ForceState::HOVER);
        } else {
            t.show(&mut child);
        }
    }
    super::button::tool_divider(&mut child);
    super::ToolButton::new(icon::GIT_BRANCH, "Branch").show(&mut child);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Density;

    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
        ctx
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

    fn apply(ctx: &egui::Context, style: Style, density: Density) {
        theme::apply(
            ctx,
            theme::ThemeId::Midnight,
            theme::Appearance {
                style,
                density,
                ..theme::Appearance::STILL
            },
        );
    }

    /// A callout is a group named for its tone that holds its title, text and actions, each
    /// spoken once, and never its icon glyph (a private-use character screen readers spell out).
    #[test]
    fn a_callout_speaks_its_title_and_text_once() {
        let ctx = ctx();
        let show = |ui: &mut Ui| {
            Callout::new(Tone::Warning, "Pushing rewrites the remote branch.")
                .title("Force push")
                .action("Learn more")
                .show(ui);
        };
        assert_eq!(
            super::super::testkit::parts(&ctx, "Warning", show),
            [
                "Force push",
                "Pushing rewrites the remote branch.",
                "Learn more"
            ]
        );
        for n in super::super::testkit::spoken(&ctx, show) {
            let Some(label) = n.label.or(n.value) else {
                continue;
            };
            assert!(
                !label
                    .chars()
                    .any(|c| ('\u{e000}'..='\u{f8ff}').contains(&c)),
                "a glyph is spoken: {label:?}"
            );
            assert!(!label.starts_with("Force push:"), "spoken twice: {label:?}");
        }
    }

    #[test]
    fn the_table_header_takes_its_own_top_edge_in_deeper() {
        let ctx = ctx();
        for style in Style::ALL {
            apply(&ctx, style, Density::Comfortable);
            let s = theme::surfaces(&ctx);
            let out = run(&ctx, Vec::new(), |ui| {
                PaneHeader::new(HeaderKind::Table).show(ui, |_| {});
            });
            let fills: Vec<Color32> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Rect(r) => Some(r.fill),
                    _ => None,
                })
                .collect();
            assert!(fills.contains(&s.pane_header), "{style:?} {fills:?}");
            if style == Style::Deeper {
                // Distinct from every neutral edge and from the panes around it.
                for neutral in [
                    s.border,
                    s.edge,
                    s.pane_edge.unwrap_or(s.border),
                    s.deck_edge,
                ] {
                    assert_ne!(s.pane_header_seam, neutral);
                }
                assert_ne!(s.pane_header, s.pane);
                assert_ne!(s.pane_header, s.bg);
            } else {
                assert_eq!(s.pane_header_seam, s.border);
            }
        }
    }

    #[test]
    fn the_deck_spans_the_bars_of_the_active_density() {
        let ctx = ctx();
        for density in Density::ALL {
            for style in Style::ALL {
                apply(&ctx, style, density);
                let m = theme::metrics();
                assert_eq!(Deck::new(false).height(), m.tab_strip_height);
                assert_eq!(
                    Deck::new(true).height(),
                    m.tab_strip_height + m.action_bar_height
                );
            }
        }
    }

    /// Standard paints both bars across the window; Deeper leaves the tab strip on the ground and
    /// floats only the action bar, as a card inset by the pane gutter (as the window does).
    #[test]
    fn deeper_floats_only_the_action_bar_as_a_card() {
        let ctx = ctx();
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        for style in Style::ALL {
            apply(&ctx, style, Density::Comfortable);
            let m = theme::metrics();
            let out = run(&ctx, Vec::new(), |ui| {
                let _ = Deck::new(true).paint(ui.painter(), screen);
            });
            let rects: Vec<Rect> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    Shape::Rect(r) if r.fill.a() > 0 => Some(r.rect),
                    _ => None,
                })
                .collect();
            let across = |y: f32| {
                rects.iter().any(|r| {
                    r.left() <= screen.left()
                        && r.right() >= screen.right()
                        && r.y_range().contains(y)
                })
            };
            let strip_y = m.tab_strip_height / 2.0;
            let bar_y = m.tab_strip_height + m.action_bar_height / 2.0;
            match style {
                Style::Standard => {
                    assert!(across(strip_y) && across(bar_y), "{rects:?}");
                }
                Style::Deeper => {
                    assert!(!across(strip_y) && !across(bar_y), "{rects:?}");
                    assert!(rects.contains(&Deck::action_card(screen)), "{rects:?}");
                    assert_eq!(Deck::action_card(screen).left(), m.pane_gutter);
                }
            }
        }
    }

    #[test]
    fn interactive_cards_click_from_the_keyboard_and_not_when_disabled() {
        let ctx = ctx();
        for enabled in [true, false] {
            let mut clicked = false;
            for i in 0..4 {
                let events = if i == 2 {
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
                run(&ctx, events, |ui| {
                    let r = Card::new()
                        .interactive(true)
                        .enabled(enabled)
                        .label("Open")
                        .id_salt("card")
                        .show(ui, |ui| ui.label("Open a folder"));
                    if i == 1 {
                        ui.memory_mut(|m| m.request_focus(r.response.id));
                    }
                    clicked |= r.response.clicked();
                });
            }
            assert_eq!(clicked, enabled, "enabled {enabled}");
        }
    }

    #[test]
    fn a_card_paints_its_surface_under_its_content() {
        let ctx = ctx();
        let out = run(&ctx, Vec::new(), |ui| {
            Card::new().show(ui, |ui| ui.label("Inside"));
        });
        let s = theme::surfaces(&ctx);
        let first_text = out
            .shapes
            .iter()
            .position(|c| matches!(c.shape, Shape::Text(_)));
        let card = out.shapes.iter().position(|c| match &c.shape {
            Shape::Vec(v) => v
                .iter()
                .any(|s2| matches!(s2, Shape::Rect(r) if r.fill == s.panel)),
            _ => false,
        });
        assert!(card.is_some() && first_text.is_some());
        assert!(card < first_text, "the surface comes first");
    }

    #[test]
    fn banners_and_skeletons_hold_still_with_reduced_motion() {
        let ctx = ctx();
        let mut heights = Vec::new();
        run(&ctx, Vec::new(), |ui| {
            heights.push(
                Banner::new(Tone::Warning, "Merging")
                    .show(ui)
                    .response
                    .rect
                    .height(),
            );
        });
        assert_eq!(heights, vec![theme::metrics().pane_header_height]);
        let p = theme::palette(&ctx);
        let mut out = run(&ctx, Vec::new(), |ui| {
            Skeleton::rows(3).show(ui);
        });
        let fills: Vec<Color32> = out
            .shapes
            .drain(..)
            .filter_map(|c| match c.shape {
                Shape::Rect(r) => Some(r.fill),
                _ => None,
            })
            .collect();
        assert_eq!(
            fills,
            vec![Skeleton::fill_at(p, None); 3],
            "shown at once, at the midpoint"
        );
        // Once egui has settled its first frames, a still skeleton asks for no repaint.
        let mut settled = out;
        for _ in 0..3 {
            settled = run(&ctx, Vec::new(), |ui| {
                Banner::new(Tone::Warning, "Merging").show(ui);
                Skeleton::rows(3).show(ui);
            });
        }
        assert!(
            settled
                .viewport_output
                .values()
                .all(|v| v.repaint_delay > std::time::Duration::from_secs(1)),
            "{:?}",
            settled
                .viewport_output
                .values()
                .map(|v| v.repaint_delay)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn surface_shapes_match_what_surface_paints() {
        let ctx = ctx();
        for id in [theme::ThemeId::Midnight, theme::ThemeId::Daylight] {
            for style in Style::ALL {
                theme::apply(
                    &ctx,
                    id,
                    theme::Appearance {
                        style,
                        ..theme::Appearance::STILL
                    },
                );
                let p = theme::palette(&ctx);
                let s = p.surfaces(style);
                let rect = Rect::from_min_size(pos2(20.0, 20.0), vec2(120.0, 40.0));
                let radius = CornerRadius::same(10);
                for level in Elevation::ALL {
                    let painted = run(&ctx, Vec::new(), |ui| {
                        paint::surface(
                            ui.painter(),
                            rect,
                            radius,
                            p,
                            style,
                            level,
                            s.raised,
                            Some(s.edge),
                        );
                    });
                    let built = run(&ctx, Vec::new(), |ui| {
                        let shapes = paint::surface_shapes(
                            ui.painter(),
                            rect,
                            radius,
                            p,
                            style,
                            level,
                            s.raised,
                            Some(s.edge),
                        );
                        ui.painter().extend(shapes);
                    });
                    let shapes = |o: &egui::FullOutput| {
                        // Path colours are closures, so compare what each shape is.
                        format!(
                            "{:?}",
                            o.shapes.iter().map(|c| &c.shape).collect::<Vec<_>>()
                        )
                    };
                    assert_eq!(
                        shapes(&painted),
                        shapes(&built),
                        "{id:?} {style:?} {level:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_empty_state_centres_itself_in_its_first_frame() {
        let ctx = ctx();
        let title_y = || {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 600.0))),
                    ..Default::default()
                },
                |ui| {
                    EmptyState::new("Nothing here")
                        .icon(icon::FUNNEL_X)
                        .body("Body")
                        .action("Clear filters")
                        .show(ui);
                },
            );
            out.textures_delta.clear();
            out.shapes.iter().find_map(|c| match &c.shape {
                Shape::Text(t) => Some(t.pos.y),
                _ => None,
            })
        };
        let first = title_y();
        let settled = title_y();
        // The first frame lays out twice, so it already sits where later frames do.
        assert!(first.is_some());
        assert_eq!(first, settled);
    }

    #[test]
    fn surfaces_report_themselves_to_screen_readers() {
        let ctx = ctx();
        ctx.enable_accesskit();
        let out = run(&ctx, Vec::new(), |ui| {
            Banner::new(Tone::Warning, "Merge in progress").show(ui);
            Callout::new(Tone::Accent, "Signed").show(ui);
            Card::new()
                .interactive(true)
                .label("Clone a repository")
                .show(ui, |ui| ui.label("Clone"));
            Skeleton::rows(1).show(ui);
        });
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
        for want in [
            "Merge in progress",
            "Signed",
            "Clone a repository",
            "Loading",
        ] {
            assert!(labels.iter().any(|l| l == want), "{want}: {labels:?}");
        }
    }

    #[test]
    fn a_callout_announces_each_text_once_and_names_unavailable_actions() {
        use super::super::testkit;
        let ctx = ctx();
        let show = |ui: &mut Ui| {
            Callout::new(Tone::Warning, "Set up an SSH agent before retrying.")
                .title("Authentication needed")
                .action(
                    NoticeAction::new("Retry")
                        .enabled(false)
                        .disabled_reason("No SSH agent is available"),
                )
                .show(ui);
        };
        let parts = testkit::parts(&ctx, "Warning", show);
        for text in [
            "Authentication needed",
            "Set up an SSH agent before retrying.",
        ] {
            assert_eq!(
                parts.iter().filter(|part| *part == text).count(),
                1,
                "{parts:?}"
            );
        }
        let nodes = testkit::spoken(&ctx, show);
        let retry = nodes
            .iter()
            .find(|node| {
                node.label
                    .as_deref()
                    .is_some_and(|name| name.starts_with("Retry"))
            })
            .expect("disabled Retry action");
        assert!(retry.disabled);
        assert_eq!(
            retry.description.as_deref(),
            Some("No SSH agent is available")
        );
    }

    /// A11Y-15: an action card is a button; a card that is one choice of a group (a theme) is a
    /// radio button, checked or not, never a toggle button.
    #[test]
    fn cards_are_buttons_or_choices_never_toggles() {
        use egui::accesskit::{Role, Toggled};

        use super::super::testkit;
        let ctx = ctx();
        let nodes = testkit::spoken(&ctx, |ui| {
            Card::new()
                .interactive(true)
                .label("Clone a repository")
                .show(ui, |ui| ui.label("Clone"));
            for (name, on) in [("Midnight theme", true), ("Daylight theme", false)] {
                Card::new()
                    .interactive(true)
                    .selected(on)
                    .label(name)
                    .id_salt(name)
                    .show(ui, |ui| ui.label(name));
            }
        });
        let action = testkit::find(&nodes, "Clone a repository").expect("action card");
        assert_eq!((action.role, action.toggled), (Role::Button, None));
        for (name, on) in [
            ("Midnight theme", Toggled::True),
            ("Daylight theme", Toggled::False),
        ] {
            let choice = testkit::find(&nodes, name).expect(name);
            assert_eq!(
                (choice.role, choice.toggled),
                (Role::RadioButton, Some(on)),
                "{name}"
            );
        }
    }

    /// A grid of choice cards is one Tab stop; the arrows move between the cards, by one to the
    /// side and by a row up and down.
    #[test]
    fn a_card_grid_is_one_tab_stop_with_arrows_inside() {
        use egui::Key;

        use super::super::testkit;
        let ctx = ctx();
        let names = ["A", "B", "C", "D", "E", "F"];
        let mut show = |ui: &mut Ui| {
            Button::new("Before").show(ui);
            let grid = super::super::row::Roving::new(ui.ctx(), "test-grid", 6, Some(1)).columns(3);
            for row in names.chunks(3).enumerate() {
                ui.horizontal(|ui| {
                    for (j, name) in row.1.iter().enumerate() {
                        Card::new()
                            .interactive(true)
                            .selected(row.0 * 3 + j == 1)
                            .roving(grid, row.0 * 3 + j)
                            .label(*name)
                            .id_salt(*name)
                            .show(ui, |ui| ui.label(*name));
                    }
                });
            }
            Button::new("After").show(ui);
        };
        ctx.enable_accesskit();
        run(&ctx, Vec::new(), &mut show);
        let mut path = Vec::new();
        for k in [
            Key::Tab,
            Key::Tab,
            Key::ArrowRight,
            Key::ArrowDown,
            Key::ArrowLeft,
            Key::ArrowUp,
            Key::Tab,
        ] {
            run(&ctx, testkit::key(k), &mut show);
            let out = run(&ctx, Vec::new(), &mut show);
            let update = out.platform_output.accesskit_update.expect("accesskit");
            let label = update
                .nodes
                .iter()
                .find(|(id, _)| *id == update.focus)
                .and_then(|(_, n)| n.label().map(str::to_owned));
            path.push(label.unwrap_or_default());
        }
        assert_eq!(path, ["Before", "B", "C", "F", "E", "B", "After"]);
    }
}
