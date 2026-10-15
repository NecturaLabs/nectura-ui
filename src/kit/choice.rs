//! [`Segmented`] (with its sliding thumb), [`ViewTabs`], [`UnderlineTabs`] (with its sliding
//! indicator) and [`Swatches`]. Design spec 4.7 and 4.8.
//!
//! The thumb and the indicator are painted once, under the labels, and slide to the chosen option
//! on the indicator springs, measured from the control's own left edge so scrolling never animates
//! them. Only the chosen option is a Tab stop; arrow keys move the choice.

use std::sync::Arc;

use egui::accesskit::Role;
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Galley, Id, Key, Modifiers, PointerButton, Rect,
    Response, Sense, Ui, UiBuilder, Vec2, Widget, WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::Shortcut;
use super::button::tip as tip_for;
use super::chip::{Badge, count_text};
use super::overlay::TooltipExt;
use super::toggle::{
    arrow_step, claim_arrows, component_id, gallery_caption, gallery_row, group_node, remembered,
    step,
};
use super::{ForceState, IconButton, Interact, KitResponse, Size, anim, paint};
use crate::motion;
use crate::theme::{self, Elevation};

/// Gap between a segment's or tab's icon, label, count and close button.
const ITEM_GAP: f32 = 7.0;
/// The underline indicator's thickness.
const INDICATOR: f32 = 2.0;
/// Space between underline tabs.
const TAB_GAP: f32 = 2.0;
/// Underline tabs: padding left of the content, and right of it with and without a close button.
const TAB_PAD_LEFT: f32 = 10.0;
const TAB_PAD_RIGHT_CLOSE: f32 = 6.0;
/// The ahead/behind count on a tab (mono, `accent`).
const TAB_COUNT_SIZE: f32 = 11.5;

/// A slid rectangle, measured from `origin` so moving the whole control never animates it. A
/// forced progress places it part-way from the previous option (`from`) instead.
fn slide(
    ctx: &egui::Context,
    id: Id,
    origin: egui::Pos2,
    target: Rect,
    from: Option<Rect>,
    force: Option<ForceState>,
) -> Rect {
    let local = target.translate(-origin.to_vec2());
    let r = match (force.and_then(|f| f.progress), from) {
        (Some(k), Some(from)) => {
            let from = from.translate(-origin.to_vec2());
            Rect::from_min_max(from.min.lerp(local.min, k), from.max.lerp(local.max, k))
        }
        (Some(_), None) | (None, _) if force.is_some() => local,
        _ => anim::slide_rect(ctx, id, local),
    };
    r.translate(origin.to_vec2())
}

// ----------------------------------------------------------------------------------- Segmented

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Content {
    Text,
    IconAndText,
    IconOnly,
}

/// Mutually exclusive options in a recessed track; a raised thumb slides to the chosen one.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Segmented<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    options: Vec<(T, Option<&'a str>, &'a str)>,
    content: Content,
    badges: Vec<(usize, usize)>,
    /// Segments that keep room for a two-digit count while they show none.
    badge_room: Vec<usize>,
    strong_badges: bool,
    label: String,
    equal: bool,
    compact: bool,
    size: Size,
    enabled: bool,
    disabled_reason: Option<String>,
    /// Options that cannot be chosen, with why.
    off: Vec<(usize, Option<String>)>,
    tips: Vec<(usize, String)>,
    shortcuts: Vec<(usize, Shortcut)>,
    id_salt: Option<Id>,
    force: Option<(usize, ForceState)>,
}

impl<'a, T: PartialEq + Clone> Segmented<'a, T> {
    /// Text segments: (value, label).
    pub fn new(value: &'a mut T, options: &'a [(T, &'a str)]) -> Self {
        let options = options.iter().map(|(v, l)| (v.clone(), None, *l)).collect();
        Self::build(value, options, Content::Text)
    }

    /// Icon-only segments: (value, glyph, name); the name is the tooltip and accessible label.
    pub fn icons(value: &'a mut T, options: &'a [(T, &'a str, &'a str)]) -> Self {
        let options = options
            .iter()
            .map(|(v, g, l)| (v.clone(), Some(*g), *l))
            .collect();
        Self::build(value, options, Content::IconOnly)
    }

    /// Segments with a leading icon and a label: (value, glyph, label).
    pub fn with_icons(value: &'a mut T, options: &'a [(T, &'a str, &'a str)]) -> Self {
        let options = options
            .iter()
            .map(|(v, g, l)| (v.clone(), Some(*g), *l))
            .collect();
        Self::build(value, options, Content::IconAndText)
    }

    fn build(value: &'a mut T, options: Vec<(T, Option<&'a str>, &'a str)>, c: Content) -> Self {
        Self {
            value,
            options,
            content: c,
            badges: Vec::new(),
            badge_room: Vec::new(),
            strong_badges: false,
            label: String::new(),
            equal: false,
            compact: false,
            size: Size::Md,
            enabled: true,
            disabled_reason: None,
            off: Vec::new(),
            tips: Vec::new(),
            shortcuts: Vec::new(),
            id_salt: None,
            force: None,
        }
    }

    /// A count badge on segment `index` (hidden when `n` is 0).
    pub fn badge(mut self, index: usize, n: usize) -> Self {
        if n > 0 {
            self.badges.push((index, n));
        }
        self
    }
    /// Segment `index` keeps the room of a two-digit count badge while it shows none, so the
    /// control (and whatever sits after it) does not change width as the count comes and goes.
    pub fn badge_room(mut self, index: usize) -> Self {
        self.badge_room.push(index);
        self
    }
    /// Count badges on the `primary` fill (the view switcher's counts, spec 4.18 `strong`).
    pub fn strong_badges(mut self, strong: bool) -> Self {
        self.strong_badges = strong;
        self
    }
    /// Only the chosen option shows its label; the others keep their icon (and count), with
    /// their name as a tooltip. For icon-and-label segments in a narrow window.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }
    /// Whether option `index` can be chosen; `reason` says why not (its tooltip). The rest of
    /// the control stays live.
    pub fn option_enabled(mut self, index: usize, enabled: bool, reason: Option<&str>) -> Self {
        self.off.retain(|(i, _)| *i != index);
        if !enabled {
            self.off.push((index, reason.map(str::to_owned)));
        }
        self
    }
    /// A tooltip on option `index` (an icon-only option's tooltip is its name otherwise).
    pub fn option_tooltip(mut self, index: usize, text: impl Into<String>) -> Self {
        self.tips.push((index, text.into()));
        self
    }
    /// The command option `index` stands for: its keys join that option's tooltip.
    pub fn option_shortcut(mut self, index: usize, command: impl Into<Shortcut>) -> Self {
        self.shortcuts.push((index, command.into()));
        self
    }
    /// The group's accessible name (what the options choose).
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
    /// Every segment as wide as the widest.
    pub fn equal(mut self) -> Self {
        self.equal = true;
        self
    }
    /// `Sm` for headers (diff file header); `Md` by default.
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
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
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }
    /// Forces a state on segment `index` (the gallery and tests only). A forced progress shows
    /// the thumb part-way from the first segment to the chosen one.
    pub(crate) fn show_forced(mut self, ui: &mut Ui, index: usize, force: ForceState) -> Response {
        self.force = Some((index, force));
        self.show(ui)
    }

    /// Shows the control; the response is the group's and reports `changed` when the choice
    /// moved.
    pub fn show(self, ui: &mut Ui) -> Response {
        struct Seg {
            icon: Option<Arc<Galley>>,
            text: Option<Arc<Galley>>,
            badge: Option<Badge>,
            width: f32,
        }

        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let live = |i: usize| enabled && !self.off.iter().any(|(at, _)| *at == i);
        let group_id = component_id(ui, "kit-segmented", self.id_salt);
        let seg_h = self.size.height() - 2.0;
        let inset = m.segment_track_pad + paint::SEAM;
        let font = FontId::new(m.body_size, theme::medium());
        let pad = if self.size == Size::Md {
            m.segment_pad
        } else {
            m.segment_pad - 3.0
        };
        let chosen = self.options.iter().position(|(v, _, _)| v == self.value);

        let segs: Vec<Seg> = self
            .options
            .iter()
            .enumerate()
            .map(|(i, (_, glyph, label))| {
                let icon = glyph.map(|g| {
                    ui.painter().layout_no_wrap(
                        g.to_owned(),
                        theme::icon_font(m.icon_md, chosen == Some(i)),
                        Color32::PLACEHOLDER,
                    )
                });
                // Compact: only the chosen option keeps its name.
                let named = match self.content {
                    Content::IconOnly => false,
                    Content::IconAndText => !self.compact || chosen == Some(i),
                    Content::Text => true,
                };
                let text = named.then(|| {
                    ui.painter().layout_no_wrap(
                        (*label).to_owned(),
                        font.clone(),
                        Color32::PLACEHOLDER,
                    )
                });
                let badge = self.badges.iter().find(|(at, _)| *at == i).map(|(_, n)| {
                    let b = Badge::new(count_text(*n)).mono();
                    if self.strong_badges { b.strong() } else { b }
                });
                let parts = [
                    icon.as_ref().map(|g| g.size().x),
                    text.as_ref().map(|g| g.size().x),
                    badge.as_ref().map(|b| b.measure(ui).x),
                ];
                let n = parts.iter().flatten().count();
                let content: f32 = parts.iter().flatten().sum::<f32>()
                    + ITEM_GAP * crate::count_f32(n.saturating_sub(1));
                // The room the widest count ("99+") would take, when this segment keeps it free: the
                // content stays centred in it.
                let held = if self.badge_room.contains(&i) {
                    let widest = Badge::new(count_text(100)).mono().measure(ui).x;
                    let shown = badge.as_ref().map_or(0.0, |b| b.measure(ui).x);
                    (widest - shown).max(0.0)
                        + if badge.is_none() && n > 0 {
                            ITEM_GAP
                        } else {
                            0.0
                        }
                } else {
                    0.0
                };
                let width = if text.is_none() && badge.is_none() && held == 0.0 {
                    (content + 2.0 * (pad - 3.0)).max(seg_h + 4.0)
                } else {
                    content + held + 2.0 * pad
                };
                Seg {
                    icon,
                    text,
                    badge,
                    width,
                }
            })
            .collect();
        let widest = segs.iter().map(|s| s.width).fold(0.0, f32::max);
        let widths: Vec<f32> = segs
            .iter()
            .map(|s| if self.equal { widest } else { s.width })
            .collect();
        let total = vec2(
            widths.iter().sum::<f32>() + 2.0 * inset,
            seg_h + 2.0 * inset,
        );
        let (track, _) = ui.allocate_exact_size(total, Sense::hover());
        let mut rects = Vec::with_capacity(segs.len());
        let mut x = track.left() + inset;
        for w in &widths {
            rects.push(Rect::from_min_size(
                pos2(x, track.top() + inset),
                vec2(*w, seg_h),
            ));
            x += w;
        }

        // One Tab stop: the chosen option, or the first that can be chosen.
        let stop = chosen
            .filter(|i| live(*i))
            .or_else(|| (0..rects.len()).find(|i| live(*i)));
        let mut responses = Vec::with_capacity(rects.len());
        let inner = ui.new_child(UiBuilder::new().max_rect(track));
        group_node(&inner, Role::RadioGroup, &self.label, enabled);
        for (i, rect) in rects.iter().enumerate() {
            let sense = match (live(i), stop == Some(i)) {
                (false, _) => Sense::hover(),
                (true, true) => Sense::click(),
                (true, false) => Sense::CLICK,
            };
            let r = inner.interact(*rect, group_id.with(i), sense);
            // Every segment's node now, in order (only the Tab stop would get one here).
            ctx.accesskit_node_builder(r.id, |_| {});
            responses.push(r);
        }

        let mut changed = false;
        let mut now = chosen;
        for (i, r) in responses.iter().enumerate() {
            if r.clicked() && now != Some(i) {
                now = Some(i);
                changed = true;
            }
        }
        let focused = responses.iter().position(Response::has_focus);
        if let Some(at) = focused {
            claim_arrows(ui, responses[at].id, true, false);
        }
        if enabled
            && let Some(at) = focused
            && let Some(delta) = arrow_step(ui, false, true)
        {
            // Arrows skip the options that cannot be chosen.
            let len = responses.len();
            let mut next = at;
            for _ in 0..len {
                next = step(next, delta, len);
                if live(next) {
                    break;
                }
            }
            if live(next) && next != at {
                now = Some(next);
                responses[next].request_focus();
                changed = true;
            }
        }
        if changed && let Some(i) = now {
            *self.value = self.options[i].0.clone();
        }

        if ui.is_rect_visible(track) {
            // A disabled control keeps its track; its thumb (the value) turns to the flat
            // `disabled_fill` in its `disabled_edge`, and the labels stay readable, all at full
            // strength.
            let painter = super::content_painter(ui);
            let ink = painter.clone();
            let off_text = p.disabled_text;
            let radius = CornerRadius::same(m.radius);
            paint::fill(
                &painter,
                track,
                CornerRadius::same(m.radius.saturating_add(2)),
                theme::Fill::flat(s.track),
            );
            paint::rim(
                &painter,
                track,
                CornerRadius::same(m.radius.saturating_add(2)),
                s.track_edge,
            );
            if let Some(i) = now {
                let force = self.force.map(|(_, f)| f);
                let thumb = slide(
                    &ctx,
                    group_id.with("kit-seg-thumb"),
                    track.min,
                    rects[i],
                    rects.first().copied(),
                    force,
                );
                if enabled {
                    paint::surface(
                        &ink,
                        thumb,
                        radius,
                        p,
                        style,
                        Elevation::Thumb,
                        s.thumb,
                        None,
                    );
                } else {
                    paint::fill(&ink, thumb, radius, theme::Fill::flat(s.disabled_fill));
                    paint::rim(&ink, thumb, radius, s.disabled_edge);
                }
            }
            for (i, (seg, rect)) in segs.iter().zip(&rects).enumerate() {
                let force = self.force.filter(|(at, _)| *at == i).map(|(_, f)| f);
                let st = Interact::of(&ctx, &responses[i], live(i), force);
                let selected = anim::ease(
                    &ctx,
                    group_id.with(("kit-seg-on", i)),
                    now == Some(i),
                    motion::BASE,
                );
                let color = match (live(i), now == Some(i)) {
                    (true, _) => p.text_dim.lerp_to_gamma(p.text, selected.max(st.hover)),
                    (false, true) => p.text_dim,
                    (false, false) => off_text,
                };
                let parts = [
                    seg.icon.as_ref().map(|g| g.size().x),
                    seg.text.as_ref().map(|g| g.size().x),
                    seg.badge.as_ref().map(|b| b.measure(ui).x),
                ];
                let n = parts.iter().flatten().count();
                let content: f32 = parts.iter().flatten().sum::<f32>()
                    + ITEM_GAP * crate::count_f32(n.saturating_sub(1));
                let mut x = rect.center().x - content / 2.0;
                let cy = rect.center().y;
                if let Some(g) = &seg.icon {
                    ink.galley(pos2(x, cy - g.size().y / 2.0), Arc::clone(g), color);
                    x += g.size().x + ITEM_GAP;
                }
                if let Some(g) = &seg.text {
                    ink.galley(pos2(x, cy - g.size().y / 2.0), Arc::clone(g), color);
                    x += g.size().x + ITEM_GAP;
                }
                if let Some(b) = &seg.badge {
                    let size = b.measure(ui);
                    b.paint(ui, Rect::from_min_size(pos2(x, cy - size.y / 2.0), size));
                }
                if st.focus > 0.0 {
                    paint::inset_ring(&painter, *rect, radius, p.focus.gamma_multiply(st.focus));
                }
            }
        }

        for (i, r) in responses.into_iter().enumerate() {
            let label = self.options[i].2;
            let on = now == Some(i);
            let ok = live(i);
            r.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, ok, on, label));
            let named = segs[i].text.is_some();
            let tip = self
                .tips
                .iter()
                .find(|(at, _)| *at == i)
                .map(|(_, t)| t.as_str())
                .or((!named).then_some(label));
            let reason = self
                .off
                .iter()
                .find(|(at, _)| *at == i)
                .and_then(|(_, r)| r.as_deref())
                .or(self.disabled_reason.as_deref());
            let shortcut = self
                .shortcuts
                .iter()
                .find(|(at, _)| *at == i)
                .map(|(_, c)| c.clone());
            let r = if ok {
                r
            } else {
                r.on_hover_cursor(egui::CursorIcon::NotAllowed)
            };
            tip_for(r, ok, tip, reason, shortcut, named);
        }
        let mut group = ui.interact(track, group_id, Sense::hover());
        if changed {
            group.mark_changed();
        }
        group
    }
}

impl<T: PartialEq + Clone> Widget for Segmented<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// The History / Changes / Requests switcher in the action bar: the segmented control with icons
/// and labels (concept 1 puts a `.k-seg` at the start of the toolbar).
pub type ViewTabs<'a, T> = Segmented<'a, T>;

// ------------------------------------------------------------------------------------ Swatches

/// Diameter of a colour swatch.
const SWATCH: f32 = 22.0;

/// A choice between colours: a row of round swatches, each named for screen readers and in its
/// tooltip. The chosen one is ringed in `text`; like [`Segmented`], only it is a Tab stop and the
/// arrow keys move the choice. A value none of the swatches holds (set elsewhere, by a slider)
/// rings none.
#[must_use = "call .show(ui) or ui.add(..)"]
#[derive(Debug)]
pub struct Swatches<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    options: &'a [(T, Color32, &'a str)],
    label: String,
    id_salt: Option<Id>,
}

impl<'a, T: PartialEq + Clone> Swatches<'a, T> {
    pub fn new(value: &'a mut T, options: &'a [(T, Color32, &'a str)]) -> Self {
        Self {
            value,
            options,
            label: String::new(),
            id_salt: None,
        }
    }

    /// The group's accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let enabled = ui.is_enabled();
        let group_id = component_id(ui, "kit-swatches", self.id_salt);
        let gap = theme::SM;
        let n = self.options.len();
        let total = vec2(
            SWATCH * crate::count_f32(n) + gap * crate::count_f32(n.saturating_sub(1)),
            SWATCH,
        );
        let (track, _) = ui.allocate_exact_size(total + vec2(4.0, 4.0), Sense::hover());
        let chosen = self.options.iter().position(|(v, _, _)| v == self.value);
        let stop = chosen.unwrap_or(0);
        let inner = ui.new_child(UiBuilder::new().max_rect(track));
        group_node(&inner, Role::RadioGroup, &self.label, enabled);
        let rects: Vec<Rect> = (0..n)
            .map(|i| {
                let x = track.left() + 2.0 + (SWATCH + gap) * crate::count_f32(i);
                Rect::from_min_size(pos2(x, track.top() + 2.0), Vec2::splat(SWATCH))
            })
            .collect();
        let responses: Vec<Response> = rects
            .iter()
            .enumerate()
            .map(|(i, rect)| {
                let sense = match (enabled, stop == i) {
                    (false, _) => Sense::hover(),
                    (true, true) => Sense::click(),
                    (true, false) => Sense::CLICK,
                };
                let r = inner.interact(*rect, group_id.with(i), sense);
                ctx.accesskit_node_builder(r.id, |_| {});
                r
            })
            .collect();

        let mut now = chosen;
        let mut changed = false;
        for (i, r) in responses.iter().enumerate() {
            if r.clicked() && now != Some(i) {
                now = Some(i);
                changed = true;
            }
        }
        let focused = responses.iter().position(Response::has_focus);
        if let Some(at) = focused {
            claim_arrows(ui, responses[at].id, true, false);
        }
        if enabled
            && let Some(at) = focused
            && let Some(delta) = arrow_step(ui, false, true)
        {
            let next = step(at, delta, n);
            if next != at {
                now = Some(next);
                responses[next].request_focus();
                changed = true;
            }
        }
        if changed && let Some(i) = now {
            *self.value = self.options[i].0.clone();
        }

        if ui.is_rect_visible(track) {
            let painter = super::content_painter(ui);
            let s = p.s();
            for (i, rect) in rects.iter().enumerate() {
                let st = Interact::of(&ctx, &responses[i], enabled, None);
                let on = anim::ease(
                    &ctx,
                    group_id.with(("kit-swatch-on", i)),
                    now == Some(i),
                    motion::BASE,
                );
                let centre = rect.center();
                let radius = SWATCH / 2.0;
                painter.circle_filled(centre, radius - 3.0 * on, self.options[i].1);
                // The swatch's own edge, so a colour close to the ground still has a shape.
                painter.circle_stroke(
                    centre,
                    radius - 3.0 * on - 0.5,
                    egui::Stroke::new(1.0, s.edge.gamma_multiply(0.6)),
                );
                let ring = p
                    .text_dim
                    .lerp_to_gamma(p.text, on)
                    .gamma_multiply(on.max(st.hover * 0.6));
                if ring.a() > 0 {
                    painter.circle_stroke(centre, radius - 0.75, egui::Stroke::new(1.5, ring));
                }
                if st.focus > 0.0 {
                    painter.circle_stroke(
                        centre,
                        radius + 1.5,
                        egui::Stroke::new(2.0, p.focus.gamma_multiply(st.focus)),
                    );
                }
            }
        }

        for (i, r) in responses.into_iter().enumerate() {
            let name = self.options[i].2;
            let on = now == Some(i);
            r.widget_info(|| WidgetInfo::selected(WidgetType::RadioButton, enabled, on, name));
            tip_for(r, enabled, Some(name), None, None, false);
        }
        let mut group = ui.interact(track, group_id, Sense::hover());
        if changed {
            group.mark_changed();
        }
        group
    }
}

impl<T: PartialEq + Clone> Widget for Swatches<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------- UnderlineTabs

/// In-pane tabs (legal documents, detail sub-views): labels over a seam, with a 2 pt indicator
/// that slides to the active tab. Tabs may carry an icon, a count and a close button.
#[derive(Debug)]
#[must_use = "call .show(ui)"]
pub struct UnderlineTabs<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    options: Vec<(T, Option<&'a str>, &'a str)>,
    counts: Vec<(usize, String)>,
    label: String,
    closable: bool,
    enabled: bool,
    id_salt: Option<Id>,
    force: Option<(usize, ForceState)>,
}

impl<'a, T: PartialEq + Clone> UnderlineTabs<'a, T> {
    /// Text tabs: (value, label).
    pub fn new(value: &'a mut T, options: &'a [(T, &'a str)]) -> Self {
        let options = options.iter().map(|(v, l)| (v.clone(), None, *l)).collect();
        Self::build(value, options)
    }

    /// Tabs with a leading icon: (value, glyph, label).
    pub fn with_icons(value: &'a mut T, options: &'a [(T, &'a str, &'a str)]) -> Self {
        let options = options
            .iter()
            .map(|(v, g, l)| (v.clone(), Some(*g), *l))
            .collect();
        Self::build(value, options)
    }

    fn build(value: &'a mut T, options: Vec<(T, Option<&'a str>, &'a str)>) -> Self {
        Self {
            value,
            options,
            counts: Vec::new(),
            label: String::new(),
            closable: false,
            enabled: true,
            id_salt: None,
            force: None,
        }
    }

    /// A short count after tab `index`'s label, such as ahead/behind (`↑2`).
    pub fn count(mut self, index: usize, text: impl Into<String>) -> Self {
        self.counts.push((index, text.into()));
        self
    }
    /// The tab list's accessible name.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }
    /// Each tab gets a close button (and closes on a middle click); `show` returns which.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
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
    pub(crate) fn show_forced(
        mut self,
        ui: &mut Ui,
        index: usize,
        force: ForceState,
    ) -> KitResponse<Option<T>> {
        self.force = Some((index, force));
        self.show(ui)
    }

    /// Shows the tabs. The response is the tab list's (`changed` when the active tab moved); the
    /// value is the tab whose close button was pressed, if any.
    pub fn show(self, ui: &mut Ui) -> KitResponse<Option<T>> {
        struct Tab {
            icon: Option<Arc<Galley>>,
            text: Arc<Galley>,
            count: Option<Arc<Galley>>,
            width: f32,
        }

        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let s = theme::surfaces(&ctx);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let list_id = component_id(ui, "kit-underline-tabs", self.id_salt);
        let h = m.underline_tab_height;
        let font = FontId::new(m.body_size, theme::medium());
        let close_side = Size::Xs.height();
        let chosen_at = self.options.iter().position(|(v, _, _)| v == self.value);
        let pad_right = if self.closable {
            TAB_PAD_RIGHT_CLOSE
        } else {
            TAB_PAD_LEFT
        };

        let tabs: Vec<Tab> = self
            .options
            .iter()
            .enumerate()
            .map(|(i, (_, glyph, label))| {
                let icon = glyph.map(|g| {
                    ui.painter().layout_no_wrap(
                        g.to_owned(),
                        theme::icon_font(m.icon_md, chosen_at == Some(i)),
                        Color32::PLACEHOLDER,
                    )
                });
                let text = ui.painter().layout_no_wrap(
                    (*label).to_owned(),
                    font.clone(),
                    Color32::PLACEHOLDER,
                );
                let count = self.counts.iter().find(|(at, _)| *at == i).map(|(_, c)| {
                    ui.painter().layout_no_wrap(
                        c.clone(),
                        FontId::new(TAB_COUNT_SIZE, FontFamily::Monospace),
                        Color32::PLACEHOLDER,
                    )
                });
                let mut width = TAB_PAD_LEFT + text.size().x + pad_right;
                if let Some(g) = &icon {
                    width += g.size().x + ITEM_GAP;
                }
                if let Some(g) = &count {
                    width += g.size().x + ITEM_GAP;
                }
                if self.closable {
                    width += close_side + ITEM_GAP;
                }
                Tab {
                    icon,
                    text,
                    count,
                    width,
                }
            })
            .collect();
        let row_w = ui.available_width().max(
            tabs.iter().map(|t| t.width).sum::<f32>() + TAB_GAP * crate::count_f32(tabs.len()),
        );
        let (row, _) = ui.allocate_exact_size(vec2(row_w, h), Sense::hover());
        let mut rects = Vec::with_capacity(tabs.len());
        let mut x = row.left();
        for t in &tabs {
            rects.push(Rect::from_min_size(pos2(x, row.top()), vec2(t.width, h)));
            x += t.width + TAB_GAP;
        }
        let chosen = self.options.iter().position(|(v, _, _)| v == self.value);
        let mut responses = Vec::with_capacity(rects.len());
        let mut closes = Vec::new();
        let mut closed = None;
        let inner = ui.new_child(UiBuilder::new().max_rect(row));
        group_node(&inner, Role::TabList, &self.label, enabled);
        for (i, rect) in rects.iter().enumerate() {
            let sense = match (enabled, chosen.unwrap_or(0) == i) {
                (false, _) => Sense::hover(),
                (true, true) => Sense::click(),
                (true, false) => Sense::CLICK,
            };
            let r = inner.interact(*rect, list_id.with(i), sense);
            // Every tab's node now, in order (only the Tab stop would get one here).
            ctx.accesskit_node_builder(r.id, |_| {});
            if self.closable && enabled && r.clicked_by(PointerButton::Middle) {
                closed = Some(i);
            }
            responses.push(r);
        }

        let mut changed = false;
        let mut now = chosen;
        for (i, r) in responses.iter().enumerate() {
            if r.clicked() && now != Some(i) {
                now = Some(i);
                changed = true;
            }
        }
        let focused = responses.iter().position(Response::has_focus);
        if let Some(at) = focused {
            claim_arrows(ui, responses[at].id, true, false);
        }
        if enabled && let Some(at) = focused {
            let jump = ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::Home) {
                    Some(0)
                } else if i.consume_key(Modifiers::NONE, Key::End) {
                    Some(responses.len().saturating_sub(1))
                } else {
                    None
                }
            });
            let next =
                jump.or_else(|| arrow_step(ui, false, true).map(|d| step(at, d, responses.len())));
            if let Some(next) = next {
                now = Some(next);
                responses[next].request_focus();
                changed = true;
            }
        }
        if changed && let Some(i) = now {
            *self.value = self.options[i].0.clone();
        }

        if ui.is_rect_visible(row) {
            // Disabled: the disabled tokens at full strength (the indicator in the neutral
            // disabled mark, never a dimmed accent).
            let painter = super::content_painter(ui);
            let ink = painter.clone();
            let off_text = p.disabled_text;
            let indicator = if enabled {
                p.focus
            } else {
                super::disabled_mark(p, s)
            };
            paint::seam_h(
                &painter,
                row.x_range(),
                row.bottom() - paint::SEAM * 0.5,
                s.border,
            );
            let top_radius = CornerRadius {
                nw: m.radius,
                ne: m.radius,
                sw: 0,
                se: 0,
            };
            for (i, (tab, rect)) in tabs.iter().zip(&rects).enumerate() {
                let force = self.force.filter(|(at, _)| *at == i).map(|(_, f)| f);
                let st = Interact::of(&ctx, &responses[i], enabled, force);
                let active = now == Some(i);
                if st.hover > 0.0 {
                    painter.rect_filled(*rect, top_radius, p.hover().gamma_multiply(st.hover));
                }
                // Disabled: the labels stay readable (the active one in `text_dim`).
                let (color, icon_color, count_color) = match (enabled, active) {
                    (true, true) => (p.text, p.accent, p.accent),
                    (true, false) => (
                        p.text_dim.lerp_to_gamma(p.text, st.hover),
                        p.text_faint,
                        p.accent,
                    ),
                    (false, true) => (p.text_dim, p.text_dim, p.text_dim),
                    (false, false) => (off_text, off_text, off_text),
                };
                let mut x = rect.left() + TAB_PAD_LEFT;
                let cy = rect.center().y;
                if let Some(g) = &tab.icon {
                    ink.galley(pos2(x, cy - g.size().y / 2.0), Arc::clone(g), icon_color);
                    x += g.size().x + ITEM_GAP;
                }
                ink.galley(
                    pos2(x, cy - tab.text.size().y / 2.0),
                    Arc::clone(&tab.text),
                    color,
                );
                x += tab.text.size().x + ITEM_GAP;
                if let Some(g) = &tab.count {
                    ink.galley(pos2(x, cy - g.size().y / 2.0), Arc::clone(g), count_color);
                    x += g.size().x + ITEM_GAP;
                }
                if self.closable {
                    closes.push(Rect::from_center_size(
                        pos2(x + close_side / 2.0, cy),
                        Vec2::splat(close_side),
                    ));
                }
                if st.focus > 0.0 {
                    paint::inset_ring(
                        &painter,
                        *rect,
                        top_radius,
                        p.focus.gamma_multiply(st.focus),
                    );
                }
            }
            if let Some(i) = now {
                let bar = Rect::from_min_max(
                    pos2(rects[i].left(), row.bottom() - INDICATOR),
                    pos2(rects[i].right(), row.bottom()),
                );
                let from = rects.first().map(|r| {
                    Rect::from_min_max(pos2(r.left(), bar.top()), pos2(r.right(), bar.bottom()))
                });
                let bar = slide(
                    &ctx,
                    list_id.with("kit-tab-indicator"),
                    row.min,
                    bar,
                    from,
                    self.force.map(|(_, f)| f),
                );
                painter.rect_filled(
                    bar,
                    CornerRadius {
                        nw: 2,
                        ne: 2,
                        sw: 0,
                        se: 0,
                    },
                    indicator,
                );
            }
        }
        for (i, rect) in closes.into_iter().enumerate() {
            let name = format!("Close {}", self.options[i].2);
            let r = ui.put(
                rect,
                IconButton::new(icon::X, name)
                    .size(Size::Xs)
                    .enabled(enabled)
                    .id_salt(list_id.with(("close", i))),
            );
            if r.clicked() {
                closed = Some(i);
            }
        }

        for (i, r) in responses.into_iter().enumerate() {
            let label = self.options[i].2;
            let on = now == Some(i);
            r.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, enabled, on, label));
            ctx.accesskit_node_builder(r.id, |n| {
                n.set_role(Role::Tab);
                n.clear_toggled();
                n.set_selected(on);
            });
        }
        let mut list = ui.interact(row, list_id, Sense::hover());
        if changed {
            list.mark_changed();
        }
        KitResponse {
            response: list,
            value: closed.map(|i| self.options[i].0.clone()),
        }
    }
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's segmented controls and tabs.
pub(crate) fn gallery(ui: &mut Ui) {
    gallery_caption(ui, "Segmented: text, live (click or arrows)");
    remembered(ui, "seg-text", 0_u8, |ui, v| {
        Segmented::new(v, &[(0, "Unified"), (1, "Split")])
            .label("Diff layout")
            .id_salt("g-seg0")
            .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(ui, "View tabs: icons, labels and a count badge");
    remembered(ui, "seg-view", 0_u8, |ui, v| {
        ViewTabs::with_icons(
            v,
            &[
                (0, icon::CLOCK_COUNTER_CLOCKWISE, "History"),
                (1, icon::PENCIL_SIMPLE, "Changes"),
                (2, icon::GIT_PULL_REQUEST, "Pull requests"),
            ],
        )
        .badge(1, 4)
        .label("View")
        .id_salt("g-seg1")
        .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(
        ui,
        "View tabs, compact: strong count, shortcuts, one option off with its reason",
    );
    remembered(ui, "seg-view-compact", 1_u8, |ui, v| {
        ViewTabs::with_icons(
            v,
            &[
                (0, icon::CLOCK_COUNTER_CLOCKWISE, "History"),
                (1, icon::PENCIL_SIMPLE, "Changes"),
                (2, icon::GIT_PULL_REQUEST, "Requests"),
            ],
        )
        .badge(1, 12)
        .strong_badges(true)
        .compact(true)
        .option_shortcut(0, "1")
        .option_shortcut(1, "2")
        .option_enabled(2, false, Some("No GitHub or GitLab remote"))
        .label("View")
        .id_salt("g-seg1c")
        .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(
        ui,
        "Diff layout on a new file: Side by side off, Unified still chosen",
    );
    remembered(ui, "seg-one-column", 0_u8, |ui, v| {
        Segmented::with_icons(
            v,
            &[
                (0, icon::ROWS, "Unified"),
                (1, icon::COLUMNS, "Side by side"),
            ],
        )
        .size(Size::Sm)
        .option_enabled(1, false, Some("A new file has nothing to set beside it"))
        .option_tooltip(0, "One column")
        .label("Diff layout")
        .id_salt("g-seg-one-col")
        .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(ui, "Icons only, small, equal; hover, focus, disabled");
    gallery_row(ui, |ui| {
        let layouts = [
            (0_u8, icon::ROWS, "Unified"),
            (1, icon::COLUMNS, "Side by side"),
        ];
        remembered(ui, "seg-icons", 0_u8, |ui, v| {
            Segmented::icons(v, &layouts)
                .label("Layout")
                .id_salt("g-seg2")
                .show(ui);
        });
        Segmented::new(&mut 1_u8, &[(0, "All"), (1, "Mine"), (2, "Open")])
            .size(Size::Sm)
            .equal()
            .id_salt("g-seg3")
            .show(ui);
        Segmented::new(&mut 0_u8, &[(0, "Hover"), (1, "Other")])
            .id_salt("g-seg4")
            .show_forced(ui, 1, ForceState::HOVER);
        Segmented::new(&mut 0_u8, &[(0, "Focus"), (1, "Other")])
            .id_salt("g-seg5")
            .show_forced(ui, 0, ForceState::FOCUS);
        Segmented::new(&mut 0_u8, &[(0, "Disabled"), (1, "Other")])
            .enabled(false)
            .disabled_reason("Open a repository first")
            .id_salt("g-seg6")
            .show(ui);
    });
    ui.add_space(theme::XS);
    gallery_caption(ui, "Thumb sliding to the third segment at t = 0, 0.5, 1");
    gallery_row(ui, |ui| {
        for (i, t) in [0.0_f32, 0.5, 1.0].into_iter().enumerate() {
            Segmented::new(&mut 2_u8, &[(0, "One"), (1, "Two"), (2, "Three")])
                .id_salt(("g-seg-m", i))
                .show_forced(
                    ui,
                    2,
                    ForceState {
                        progress: Some(t),
                        ..ForceState::default()
                    },
                );
        }
    });
    ui.add_space(theme::SM);
    gallery_caption(
        ui,
        "Underline tabs: icons, count, close; live (close a tab, + to reopen)",
    );
    remembered(ui, "tabs-docs", (0_u8, 3_u8), |ui, (v, open)| {
        let all = [
            (0_u8, icon::FOLDER, "showcase"),
            (1, icon::FOLDER, "canopy"),
            (2, icon::FOLDER, "dotfiles"),
        ];
        let shown: Vec<(u8, &str, &str)> = all.iter().take(usize::from(*open)).copied().collect();
        let out = UnderlineTabs::with_icons(v, &shown)
            .count(0, "↑2")
            .closable(true)
            .label("Repositories")
            .id_salt("g-tabs0")
            .show(ui);
        if out.value.is_some() && *open > 1 {
            *open -= 1;
            *v = (*v).min(*open - 1);
        }
        out.response
            .kit_tooltip("Middle-click a tab to close it")
            .extra(true)
            .show();
        if *open < 3 {
            let reopen = IconButton::new(icon::PLUS, "Reopen a tab")
                .size(Size::Sm)
                .id_salt("g-tabs-reopen")
                .show(ui);
            if reopen.clicked() {
                *open += 1;
            }
        }
    });
    ui.add_space(theme::XS);
    gallery_caption(
        ui,
        "Text tabs: hover on the second, focus on the first, disabled",
    );
    UnderlineTabs::new(&mut 0_u8, &[(0, "Privacy"), (1, "Terms"), (2, "Notices")])
        .id_salt("g-tabs1")
        .show_forced(ui, 1, ForceState::HOVER);
    UnderlineTabs::new(&mut 0_u8, &[(0, "Privacy"), (1, "Terms"), (2, "Notices")])
        .id_salt("g-tabs2")
        .show_forced(ui, 0, ForceState::FOCUS);
    UnderlineTabs::new(&mut 0_u8, &[(0, "Privacy"), (1, "Terms")])
        .enabled(false)
        .id_salt("g-tabs3")
        .show(ui);
    gallery_caption(ui, "Indicator sliding to the third tab at t = 0.5");
    UnderlineTabs::new(&mut 2_u8, &[(0, "Privacy"), (1, "Terms"), (2, "Notices")])
        .id_salt("g-tabs4")
        .show_forced(
            ui,
            2,
            ForceState {
                progress: Some(0.5),
                ..ForceState::default()
            },
        );
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Toggled;

    use super::super::testkit::{self, find, frame, key, spoken};
    use super::*;

    const OPTS: [(u8, &str); 3] = [(0, "History"), (1, "Changes"), (2, "Requests")];

    /// a segment that keeps room for its count does not change the control's width
    /// as the count appears, grows to two digits and goes.
    #[test]
    fn a_segment_with_badge_room_keeps_the_controls_width_as_its_count_comes_and_goes() {
        let ctx = testkit::ctx();
        let width = |count: usize, room: bool| {
            let mut v = 0_u8;
            let mut w = 0.0;
            frame(&ctx, Vec::new(), |ui| {
                let mut tabs = Segmented::with_icons(
                    &mut v,
                    &[(0, "", "History"), (1, "", "Changes"), (2, "", "Requests")],
                )
                .badge(1, count);
                if room {
                    tabs = tabs.badge_room(1);
                }
                w = tabs.show(ui).rect.width();
            });
            w
        };
        let counts = [0, 4, 12, 99, 150];
        let widths: Vec<f32> = counts.iter().map(|n| width(*n, true)).collect();
        assert!(
            widths.iter().all(|w| (w - widths[0]).abs() < 0.5),
            "{widths:?} with room"
        );
        assert!(
            (width(0, false) - width(4, false)).abs() > 4.0,
            "without room the control grows with its count"
        );
    }

    #[test]
    fn segmented_chooses_by_click_and_arrows() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut track = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            track = Segmented::new(&mut v, &OPTS).show(ui).rect;
        });
        // Click the right third.
        let at = pos2(track.right() - 8.0, track.center().y);
        testkit::click(&ctx, at, |ui| {
            Segmented::new(&mut v, &OPTS).show(ui);
        });
        assert_eq!(v, 2);
        // A key press and its release, with a button after the control that egui's own arrow
        // navigation could move focus to.
        let press = |k: Key, v: &mut u8| {
            let mut changed = false;
            frame(&ctx, key(k), |ui| {
                changed = Segmented::new(v, &OPTS).show(ui).changed();
                super::super::Button::new("After").show(ui);
            });
            frame(&ctx, Vec::new(), |ui| {
                Segmented::new(v, &OPTS).show(ui);
                super::super::Button::new("After").show(ui);
            });
            changed
        };
        press(Key::Tab, &mut v);
        assert!(press(Key::ArrowLeft, &mut v));
        assert_eq!(v, 1, "Left moves to the previous segment");
        press(Key::ArrowLeft, &mut v);
        assert_eq!(v, 0, "focus followed the choice");
        press(Key::ArrowRight, &mut v);
        assert_eq!(v, 1);
    }

    #[test]
    fn unsalted_segmented_controls_keep_their_own_choice() {
        let ctx = testkit::ctx();
        let (mut a, mut b) = (0_u8, 0_u8);
        let mut second = Rect::NOTHING;
        let show = |ui: &mut Ui, a: &mut u8, b: &mut u8| {
            Segmented::new(a, &OPTS).show(ui);
            Segmented::new(b, &OPTS).show(ui).rect
        };
        frame(&ctx, Vec::new(), |ui| second = show(ui, &mut a, &mut b));
        testkit::click(&ctx, pos2(second.right() - 8.0, second.center().y), |ui| {
            show(ui, &mut a, &mut b);
        });
        assert_eq!((a, b), (0, 2));
    }

    #[test]
    fn disabled_segmented_ignores_input() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut track = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            track = Segmented::new(&mut v, &OPTS).enabled(false).show(ui).rect;
        });
        testkit::click(&ctx, pos2(track.right() - 8.0, track.center().y), |ui| {
            Segmented::new(&mut v, &OPTS).enabled(false).show(ui);
        });
        frame(&ctx, key(Key::Tab), |ui| {
            Segmented::new(&mut v, &OPTS).enabled(false).show(ui);
        });
        frame(&ctx, key(Key::ArrowRight), |ui| {
            Segmented::new(&mut v, &OPTS).enabled(false).show(ui);
        });
        assert_eq!(v, 0);
    }

    #[test]
    fn an_option_that_is_off_cannot_be_chosen_and_arrows_skip_it() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let show = |ui: &mut Ui, v: &mut u8| {
            Segmented::new(v, &OPTS)
                .option_enabled(1, false, Some("Not here"))
                .show(ui)
        };
        let mut track = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| track = show(ui, &mut v).rect);
        // Changes, the middle third, is off: a click there leaves History chosen.
        testkit::click(&ctx, track.center(), |ui| {
            show(ui, &mut v);
        });
        assert_eq!(v, 0);
        frame(&ctx, key(Key::Tab), |ui| {
            show(ui, &mut v);
        });
        frame(&ctx, key(Key::ArrowRight), |ui| {
            show(ui, &mut v);
        });
        assert_eq!(v, 2, "the arrow skips the option that is off");
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut v);
        });
        assert!(find(&nodes, "Changes").expect("spoken").disabled);
        assert!(!find(&nodes, "Requests").expect("spoken").disabled);
    }

    #[test]
    fn compact_segments_name_only_the_chosen_option() {
        let ctx = testkit::ctx();
        let opts = [(0_u8, icon::ROWS, "History"), (1, icon::COLUMNS, "Changes")];
        let texts = |compact: bool| {
            let shapes = frame(&ctx, Vec::new(), |ui| {
                Segmented::with_icons(&mut 1_u8, &opts)
                    .compact(compact)
                    .show(ui);
            });
            let mut out = Vec::new();
            for c in &shapes {
                if let egui::Shape::Text(t) = &c.shape {
                    out.push(t.galley.text().to_owned());
                }
            }
            out
        };
        let full = texts(false);
        assert!(full.iter().any(|t| t == "History") && full.iter().any(|t| t == "Changes"));
        let compact = texts(true);
        assert!(compact.iter().any(|t| t == "Changes"), "{compact:?}");
        assert!(!compact.iter().any(|t| t == "History"), "{compact:?}");
    }

    /// A11Y-4: a segmented control and a tab list hold their options (they were empty groups,
    /// with the options elsewhere in the tree).
    #[test]
    fn groups_hold_their_options() {
        let ctx = testkit::ctx();
        let mut v = 1_u8;
        let names: Vec<String> = OPTS.iter().map(|(_, n)| (*n).to_owned()).collect();
        let segments = testkit::parts(&ctx, "View", |ui| {
            Segmented::new(&mut v, &OPTS).label("View").show(ui);
        });
        assert_eq!(segments, names, "the segments are the group's parts");
        let tabs = testkit::parts(&ctx, "Documents", |ui| {
            UnderlineTabs::new(&mut v, &OPTS)
                .label("Documents")
                .show(ui);
        });
        assert_eq!(tabs, names, "the tabs are the tab list's parts");
    }

    #[test]
    fn segmented_speaks_as_a_radio_group() {
        let ctx = testkit::ctx();
        let mut v = 1_u8;
        let nodes = spoken(&ctx, |ui| {
            Segmented::new(&mut v, &OPTS).label("View").show(ui);
        });
        assert_eq!(find(&nodes, "View").map(|n| n.role), Some(Role::RadioGroup));
        let on = find(&nodes, "Changes").expect("spoken");
        assert_eq!(
            (on.role, on.toggled),
            (Role::RadioButton, Some(Toggled::True))
        );
        let off = find(&nodes, "History").expect("spoken");
        assert_eq!(off.toggled, Some(Toggled::False));
        let mut layout = 0_u8;
        let nodes = spoken(&ctx, |ui| {
            Segmented::icons(&mut layout, &[(0, icon::ROWS, "Unified")]).show(ui);
        });
        assert!(find(&nodes, "Unified").is_some(), "icon segments are named");
    }

    const SWATCHES: [(u16, Color32, &str); 3] = [
        (0, Color32::RED, "Red"),
        (120, Color32::GREEN, "Green"),
        (240, Color32::BLUE, "Blue"),
    ];

    #[test]
    fn swatches_choose_by_click_and_arrows() {
        let ctx = testkit::ctx();
        let mut v = 0_u16;
        let mut row = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            row = Swatches::new(&mut v, &SWATCHES).show(ui).rect;
        });
        testkit::click(&ctx, pos2(row.right() - 13.0, row.center().y), |ui| {
            Swatches::new(&mut v, &SWATCHES).show(ui);
        });
        assert_eq!(v, 240);
        let press = |k: Key, v: &mut u16| {
            frame(&ctx, key(k), |ui| {
                Swatches::new(v, &SWATCHES).show(ui);
                super::super::Button::new("After").show(ui);
            });
            frame(&ctx, Vec::new(), |ui| {
                Swatches::new(v, &SWATCHES).show(ui);
                super::super::Button::new("After").show(ui);
            });
        };
        press(Key::Tab, &mut v);
        press(Key::ArrowLeft, &mut v);
        assert_eq!(v, 120, "Left moves to the previous swatch");
        press(Key::ArrowLeft, &mut v);
        assert_eq!(v, 0, "focus followed the choice");
        press(Key::ArrowRight, &mut v);
        assert_eq!(v, 120);
    }

    #[test]
    fn disabled_swatches_ignore_input() {
        let ctx = testkit::ctx();
        let mut v = 0_u16;
        let mut row = Rect::NOTHING;
        let show = |ui: &mut Ui, v: &mut u16| {
            ui.add_enabled_ui(false, |ui| Swatches::new(v, &SWATCHES).show(ui).rect)
                .inner
        };
        frame(&ctx, Vec::new(), |ui| row = show(ui, &mut v));
        testkit::click(&ctx, pos2(row.right() - 13.0, row.center().y), |ui| {
            show(ui, &mut v);
        });
        for k in [Key::Tab, Key::ArrowRight] {
            frame(&ctx, key(k), |ui| {
                show(ui, &mut v);
            });
        }
        assert_eq!(v, 0);
    }

    #[test]
    fn swatches_speak_as_a_radio_group() {
        let ctx = testkit::ctx();
        let mut v = 120_u16;
        let nodes = spoken(&ctx, |ui| {
            Swatches::new(&mut v, &SWATCHES).label("Accent").show(ui);
        });
        assert_eq!(
            find(&nodes, "Accent").map(|n| n.role),
            Some(Role::RadioGroup)
        );
        let on = find(&nodes, "Green").expect("spoken");
        assert_eq!(
            (on.role, on.toggled),
            (Role::RadioButton, Some(Toggled::True))
        );
        assert_eq!(
            find(&nodes, "Red").and_then(|n| n.toggled),
            Some(Toggled::False)
        );
        // A hue between the swatches (a slider's choice) checks none of them.
        let mut between = 60_u16;
        let nodes = spoken(&ctx, |ui| {
            Swatches::new(&mut between, &SWATCHES)
                .label("Accent")
                .show(ui);
        });
        for name in ["Red", "Green", "Blue"] {
            assert_eq!(
                find(&nodes, name).and_then(|n| n.toggled),
                Some(Toggled::False),
                "{name}"
            );
        }
    }

    #[test]
    fn segmented_height_follows_the_metrics() {
        for density in theme::Density::ALL {
            for style in theme::Style::ALL {
                let ctx = testkit::ctx_with(theme::ThemeId::Midnight, style, density);
                let m = theme::metrics();
                let mut h = 0.0;
                frame(&ctx, Vec::new(), |ui| {
                    h = Segmented::new(&mut 0_u8, &OPTS).show(ui).rect.height();
                });
                assert_eq!(h, m.control - 2.0 + 2.0 * (m.segment_track_pad + 1.0));
            }
        }
    }

    #[test]
    fn thumb_lands_at_once_with_reduced_motion() {
        let ctx = testkit::ctx();
        let thumb = |v: u8| {
            let mut v = v;
            let s = theme::surfaces(&ctx);
            let shapes = frame(&ctx, Vec::new(), |ui| {
                Segmented::new(&mut v, &OPTS).id_salt("t").show(ui);
            });
            shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Rect(r) if r.fill == s.thumb.top && r.blur_width == 0.0 => {
                        Some(r.rect)
                    }
                    _ => None,
                })
                .expect("thumb painted")
        };
        let a = thumb(0);
        let b = thumb(2);
        assert!(
            b.left() > a.right(),
            "the thumb is on the third segment the next frame"
        );
    }

    #[test]
    fn underline_tabs_switch_close_and_speak_as_tabs() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut row = Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| {
            row = UnderlineTabs::new(&mut v, &OPTS).show(ui).response.rect;
        });
        frame(&ctx, key(Key::Tab), |ui| {
            UnderlineTabs::new(&mut v, &OPTS).show(ui);
        });
        frame(&ctx, key(Key::End), |ui| {
            UnderlineTabs::new(&mut v, &OPTS).show(ui);
        });
        assert_eq!(v, 2, "End jumps to the last tab");
        frame(&ctx, key(Key::ArrowRight), |ui| {
            UnderlineTabs::new(&mut v, &OPTS).show(ui);
        });
        assert_eq!(v, 0, "Right from the last tab wraps");
        let nodes = spoken(&ctx, |ui| {
            UnderlineTabs::new(&mut v, &OPTS)
                .label("Documents")
                .closable(true)
                .show(ui);
        });
        assert_eq!(
            find(&nodes, "Documents").map(|n| n.role),
            Some(Role::TabList)
        );
        let tab = find(&nodes, "History").expect("spoken");
        assert_eq!((tab.role, tab.selected), (Role::Tab, Some(true)));
        assert!(find(&nodes, "Close Changes").is_some());
        // Middle click closes.
        let at = pos2(row.left() + 20.0, row.center().y);
        let mut closed = None;
        let events = vec![
            egui::Event::PointerMoved(at),
            egui::Event::PointerButton {
                pos: at,
                button: PointerButton::Middle,
                pressed: true,
                modifiers: Modifiers::NONE,
            },
        ];
        let release = vec![egui::Event::PointerButton {
            pos: at,
            button: PointerButton::Middle,
            pressed: false,
            modifiers: Modifiers::NONE,
        }];
        for events in [events, release] {
            frame(&ctx, events, |ui| {
                let r = UnderlineTabs::new(&mut v, &OPTS).closable(true).show(ui);
                closed = closed.or(r.value);
            });
        }
        assert_eq!(closed, Some(0));
    }
}
