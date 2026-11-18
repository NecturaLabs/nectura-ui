//! Document tabs (concept `.h-tab`, design spec 4.8): the repository tabs in the title bar.
//!
//! The active tab is marked in the theme's tab style (`active_tab`: lifted, underlined or ruled),
//! the others transparent until hovered. A tab shows a folder icon (a spinner while its
//! repository is busy), its name, the ahead/behind counts in mono `accent`, and a close button on
//! hover and on the active tab; a middle click closes too.
//!
//! A tab is as wide as its whole name needs. Only when the tabs together need more room than the
//! strip has do the longest ones give up width (all of them down to the same width, the shorter
//! names and the active tab untouched), and only then are names ellipsised, with the full name in
//! the tooltip (a tooltip that would only repeat a name in full does not show). When even that is
//! not enough (`SQUEEZE_FLOOR`) scrolling makes the room instead, so every tab is as wide as its
//! name needs again (at most `SCROLL_CAP`) and the strip scrolls:
//! scroll buttons before and after it (disabled at the ends), a list of every open tab after
//! them, the wheel, and a fade at each edge that hides more. The active tab always scrolls into
//! view when it changes, when tabs open or close and when the strip is resized. The controls
//! exist only while the strip overflows. A tab that goes away collapses to nothing over
//! `TAB_CLOSE` instead of vanishing, so its neighbours slide into its place; reduced motion
//! removes it at once.
//!
//! `DocTabs` reports what the user asked for and changes nothing itself: the caller switches or
//! closes, and the tab it removes animates out on the next frames.

use std::fmt::Write as _;

use egui::accesskit::Role;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Color32, CornerRadius, FontFamily, FontId, Id, Rect, Response, Sense, Ui, UiBuilder, Vec2,
    WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::button::icon_button_side;
use super::overlay::{self, TooltipExt};
use super::progress::Spinner;
use super::{ForceState, IconButton, Interact, KitResponse, Size, anim, menu, paint};
use crate::motion;
use crate::theme::{self, Elevation, Fill, Style, TabStyle};

/// Padding inside a tab: left of the icon, right of the close button.
const PAD_LEFT: f32 = 10.0;
const PAD_RIGHT: f32 = 6.0;
/// Gap between a tab's icon, label, counts and close button.
const GAP: f32 = 7.0;
/// The narrowest a tab is, however short its name.
const MIN_WIDTH: f32 = 100.0;
/// The narrowest the strip squeezes a tab for its name's sake before it scrolls instead: room for
/// about seven characters of the name beside the icon and the close button.
const SQUEEZE_FLOOR: f32 = 120.0;
/// The widest a tab gets in a scrolling strip: a very long name is ellipsised there (the tooltip
/// has it whole) so one tab cannot take a whole screen of scrolling.
const SCROLL_CAP: f32 = 280.0;
/// The most of the strip the active tab may keep for itself while the others are squeezed.
const ACTIVE_SHARE: f32 = 0.6;
/// Room beyond a name's measured width, so a tab never ellipsises a name that fits it exactly.
const NAME_SLACK: f32 = 1.0;
/// How far of the viewport a scroll button moves the tabs.
const SCROLL_STEP: f32 = 0.7;
/// The list of tabs takes at most this share of the window's height.
const LIST_SHARE: f32 = 0.6;
const MIN_LIST: f32 = 160.0;
/// Gap between two tabs.
const BETWEEN: f32 = 4.0;
/// The ahead/behind counts (mono, `accent`).
const COUNT_SIZE: f32 = 11.5;

/// One tab: a stable key (it keeps the tab's identity when others open or close), a name, and
/// what it shows besides.
#[derive(Debug, Clone)]
pub struct DocTab<'a> {
    key: Id,
    name: &'a str,
    icon: &'a str,
    tooltip: Option<String>,
    detail: Option<String>,
    ahead: usize,
    behind: usize,
    busy: bool,
}

impl<'a> DocTab<'a> {
    /// A tab for `key` (the repository's path, say) named `name`.
    pub fn new(key: impl std::hash::Hash + std::fmt::Debug, name: &'a str) -> Self {
        Self {
            key: Id::new(("kit-doc-tab", key)),
            name,
            icon: icon::FOLDER_SIMPLE,
            tooltip: None,
            detail: None,
            ahead: 0,
            behind: 0,
            busy: false,
        }
    }
    /// The Phosphor glyph before the name (default: a folder), such as the repository's own icon.
    #[must_use]
    pub fn icon(mut self, glyph: &'a str) -> Self {
        self.icon = glyph;
        self
    }
    /// The tooltip's first line (default: the name).
    #[must_use]
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    /// A dim second tooltip line (the path, the shortcut that switches to it).
    #[must_use]
    pub fn detail(mut self, text: impl Into<String>) -> Self {
        self.detail = Some(text.into());
        self
    }
    /// Commits to push and to pull; nothing shows when both are zero.
    #[must_use]
    pub fn sync(mut self, ahead: usize, behind: usize) -> Self {
        self.ahead = ahead;
        self.behind = behind;
        self
    }
    /// The repository is working: a spinner takes the folder's place.
    #[must_use]
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    fn counts(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.ahead > 0 {
            parts.push(format!("↑{}", self.ahead));
        }
        if self.behind > 0 {
            parts.push(format!("↓{}", self.behind));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    fn spoken(&self) -> String {
        let mut label = self.name.to_owned();
        if self.ahead > 0 || self.behind > 0 {
            let _ = write!(label, ", {} to push, {} to pull", self.ahead, self.behind);
        }
        if self.busy {
            label.push_str(", working");
        }
        label
    }
}

/// What the user asked of the tabs this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocTabsEvent {
    /// Switch to the tab at this index (a click, or Enter or Space on a focused tab).
    Select(usize),
    /// Close the tab at this index (its close button, or a middle click).
    Close(usize),
}

/// The tab strip: `DocTabs::new(&tabs, active).max_width(w).show(ui)`.
#[must_use = "call .show(ui)"]
#[derive(Debug)]
pub struct DocTabs<'a> {
    tabs: &'a [DocTab<'a>],
    active: usize,
    id_salt: Id,
    max_width: Option<f32>,
    force: Option<(usize, ForceState)>,
    offset: Option<f32>,
}

/// A tab as it was drawn last frame, so one that disappears can collapse where it stood.
#[derive(Debug, Clone)]
struct Drawn {
    key: Id,
    name: String,
    width: f32,
}

/// A closed tab collapsing to nothing: before the tab now at `index`, from `width`, since `start`.
#[derive(Debug, Clone)]
struct Closing {
    key: Id,
    name: String,
    width: f32,
    index: usize,
    start: f64,
}

#[derive(Debug, Clone, Default)]
struct StripState {
    drawn: Vec<Drawn>,
    closing: Vec<Closing>,
    active: Option<Id>,
    /// Keep the active tab in view until the tabs stop moving: it changed, tabs opened or
    /// closed, or the strip was resized.
    reveal: bool,
    /// A tab had keyboard focus last frame: when focus leaves the strip the active tab is
    /// brought back into view.
    focus_in: bool,
    /// The width the scrolling tabs had room in last frame.
    viewport: f32,
    /// A scroll button was pressed: points to move the tabs by next frame (negative: towards the
    /// start).
    step: Option<f32>,
}

/// How the tabs share a strip: each tab's width, and whether they still overflow it.
#[derive(Debug, Clone, PartialEq)]
struct Fit {
    widths: Vec<f32>,
    overflow: bool,
}

/// The one width the longest of `natural` give up down to, so that all fit in `budget` (infinite
/// when they already do): water-filling, where the shortest names keep what they want and the
/// rest split what is left.
fn shared_cap(natural: &[f32], budget: f32) -> f32 {
    let mut sorted = natural.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mut left = budget;
    for (done, w) in sorted.iter().enumerate() {
        let share = left / crate::count_f32(sorted.len() - done);
        if *w > share {
            return share;
        }
        left -= w;
    }
    f32::INFINITY
}

/// Shares `room` between tabs that want `natural` widths, `gap` apart. All of them get what they
/// want when it fits. Otherwise the widest give up width first, down to one shared width, never
/// below `floor`; the `keep` tab (the active one) gives up nothing, so the tab in use stays
/// readable. Past the floor `overflow` says the strip must scroll, and the tabs go back to the
/// width they want, at most `SCROLL_CAP`.
fn fit(natural: &[f32], room: f32, gap: f32, floor: f32, keep: Option<usize>) -> Fit {
    let gaps = gap * crate::count_f32(natural.len().saturating_sub(1));
    let budget = room - gaps;
    let total: f32 = natural.iter().sum();
    if total <= budget {
        return Fit {
            widths: natural.to_vec(),
            overflow: false,
        };
    }
    if natural.len() <= 1 {
        // A lone tab has nothing to scroll to: it takes the room there is.
        return Fit {
            widths: natural.iter().map(|w| w.min(room.max(0.0))).collect(),
            overflow: false,
        };
    }
    let keep = keep.filter(|k| *k < natural.len());
    let others: Vec<f32> = natural
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != keep)
        .map(|(_, w)| *w)
        .collect();
    let kept = keep.map_or(0.0, |k| natural[k].min((room * ACTIVE_SHARE).max(floor)));
    let cap = shared_cap(&others, budget - kept).max(floor);
    let widths: Vec<f32> = natural
        .iter()
        .enumerate()
        .map(|(i, w)| if Some(i) == keep { kept } else { w.min(cap) })
        .collect();
    if widths.iter().sum::<f32>() > budget + 0.01 {
        // Squeezing is not enough, so the strip scrolls: and scrolling makes room, so the names
        // go back to whole (the very long ones to `SCROLL_CAP`).
        return Fit {
            widths: natural.iter().map(|w| w.min(SCROLL_CAP)).collect(),
            overflow: true,
        };
    }
    Fit {
        widths,
        overflow: false,
    }
}

impl<'a> DocTabs<'a> {
    pub fn new(tabs: &'a [DocTab<'a>], active: usize) -> Self {
        Self {
            tabs,
            active,
            id_salt: Id::new("kit-doc-tabs"),
            max_width: None,
            force: None,
            offset: None,
        }
    }
    /// The room for the tabs and, while they overflow, their scroll buttons and list (default: all
    /// the room there is). Names are ellipsised only when it is less than the tabs need.
    pub fn max_width(mut self, width: f32) -> Self {
        self.max_width = Some(width);
        self
    }
    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Id::new(salt);
        self
    }
    /// Forces a state on the tab at `index` (the gallery only).
    pub(crate) fn force(mut self, index: usize, force: ForceState) -> Self {
        self.force = Some((index, force));
        self
    }

    /// Scrolls the strip to `x` points from its start (the gallery only).
    pub(crate) fn offset(mut self, x: f32) -> Self {
        self.offset = Some(x);
        self
    }

    /// Shows the strip; the value is what the user asked for, if anything.
    pub fn show(self, ui: &mut Ui) -> KitResponse<Option<DocTabsEvent>> {
        // A row of its own as tall as the tabs, whatever layout it is placed in.
        let height = theme::metrics()
            .doc_tab_height
            .max(icon_button_side(Size::Sm));
        let width = self.max_width.unwrap_or_else(|| ui.available_width());
        ui.allocate_ui_with_layout(
            vec2(width.max(0.0), height),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| self.row(ui),
        )
        .inner
    }

    fn row(self, ui: &mut Ui) -> KitResponse<Option<DocTabsEvent>> {
        // The strip only scrolls sideways, so an ordinary (vertical) mouse wheel turns it too.
        ui.style_mut().always_scroll_the_only_direction = true;
        let ctx = ui.ctx().clone();
        let m = theme::metrics();
        let id = ui.id().with(self.id_salt);
        let state_id = id.with("state");
        let mut state: StripState = ctx.data(|d| d.get_temp(state_id)).unwrap_or_default();
        let now = ctx.input(|i| i.time);
        let seconds = motion::time(motion::TAB_CLOSE);

        // Tabs drawn last frame that are gone now collapse where they stood.
        let keys: Vec<Id> = self.tabs.iter().map(|t| t.key).collect();
        if seconds > 0.0 {
            for (i, d) in state.drawn.iter().enumerate() {
                if !keys.contains(&d.key) && !state.closing.iter().any(|c| c.key == d.key) {
                    // Its place: before the first tab that followed it and is still here.
                    let index = state.drawn[i + 1..]
                        .iter()
                        .find_map(|after| keys.iter().position(|k| *k == after.key))
                        .unwrap_or(keys.len());
                    state.closing.push(Closing {
                        key: d.key,
                        name: d.name.clone(),
                        width: d.width,
                        index,
                        start: now,
                    });
                }
            }
        }
        state.closing.retain(|c| {
            seconds > 0.0 && crate::to_f32(now - c.start) < seconds && !keys.contains(&c.key)
        });
        if !state.closing.is_empty() {
            ctx.request_repaint();
        }

        let active_key = self.tabs.get(self.active).map(|t| t.key);
        let changed = active_key != state.active
            || state.drawn.len() != self.tabs.len()
            || state.drawn.iter().zip(&keys).any(|(d, k)| d.key != *k);
        state.active = active_key;

        // Each tab's width: what its name needs, shared out when the strip lacks the room.
        let room = self
            .max_width
            .unwrap_or_else(|| ui.available_width())
            .max(0.0);
        let naturals: Vec<f32> = self.tabs.iter().map(|t| natural_width(ui, t)).collect();
        let keep = (self.active < self.tabs.len()).then_some(self.active);
        let shared = fit(&naturals, room, BETWEEN, SQUEEZE_FLOOR, keep);
        let glide = motion::time(motion::FAST_OUT);
        let mut settled = true;
        let widths: Vec<f32> = self
            .tabs
            .iter()
            .zip(&shared.widths)
            .map(|(tab, target)| {
                let w = ctx.animate_value_with_time(id.with(("width", tab.key)), *target, glide);
                settled &= (w - target).abs() < 0.5;
                w
            })
            .collect();

        // The scroll buttons and the list take room from the strip only while it overflows.
        let spacing = ui.spacing().item_spacing.x;
        let side = icon_button_side(Size::Sm);
        let controls = if shared.overflow {
            3.0 * side + 3.0 * spacing
        } else {
            0.0
        };
        let viewport = (room - controls).max(0.0);
        if shared.overflow && (viewport - state.viewport).abs() > 0.5 {
            state.reveal = true;
        }
        state.viewport = viewport;
        state.reveal |= changed;
        let reveal = state.reveal;
        let step = state.step.take();

        let mut event = None;
        let mut drawn = Vec::with_capacity(self.tabs.len());
        let mut focus_in = false;
        let height = m.doc_tab_height;
        let slot = |ui: &mut Ui| {
            shared
                .overflow
                .then(|| ui.allocate_space(vec2(side, side)).1)
        };
        let before = slot(ui);
        let mut area = egui::ScrollArea::horizontal()
            .id_salt(id.with("scroll"))
            .max_width(viewport)
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden);
        if let Some(x) = self.offset {
            area = area.horizontal_scroll_offset(x);
        }
        let scroll = area.show(ui, |ui| {
            if let Some(by) = step {
                ui.scroll_with_delta_animation(vec2(-by, 0.0), motion::scroll_animation());
            }
            let list = ui.scope_builder(
                UiBuilder::new()
                    .id_salt(id.with("list"))
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = BETWEEN;
                    for i in 0..=self.tabs.len() {
                        for c in state.closing.iter().filter(|c| c.index == i) {
                            closing_tab(ui, c, now, seconds, height);
                        }
                        let Some(tab) = self.tabs.get(i) else {
                            break;
                        };
                        let force = self.force.filter(|(at, _)| *at == i).map(|(_, f)| f);
                        let (r, e) = doc_tab(
                            ui,
                            tab,
                            i == self.active,
                            force,
                            widths.get(i).copied().unwrap_or(MIN_WIDTH),
                        );
                        focus_in |= r.has_focus();
                        if (reveal && i == self.active) || r.gained_focus() {
                            // Beyond the fade, so the tab is not left under it.
                            ui.scroll_to_rect_animation(
                                r.rect.expand2(vec2(paint::SIDE_FADE, 0.0)),
                                None,
                                motion::scroll_animation(),
                            );
                        }
                        if e.is_some() {
                            event = e.map(|e| match e {
                                TabEvent::Select => DocTabsEvent::Select(i),
                                TabEvent::Close => DocTabsEvent::Close(i),
                            });
                        }
                        drawn.push(Drawn {
                            key: tab.key,
                            name: tab.name.to_owned(),
                            width: r.rect.width(),
                        });
                    }
                },
            );
            ctx.accesskit_node_builder(list.response.id, |n| {
                n.set_role(Role::TabList);
                n.set_label("Repositories");
            });
            list.response
        });
        state.reveal = reveal && !settled;
        // Tab walked along the strip and scrolled it; when focus moves on (or Esc drops it) the
        // tab in use returns to view.
        if state.focus_in && !focus_in {
            state.reveal = true;
            ctx.request_repaint();
        }
        state.focus_in = focus_in;

        // Fade the edges that hide more tabs.
        paint::scroll_fade_x(ui, &scroll, strip_ground(&ctx));

        if shared.overflow {
            let after = slot(ui);
            let all = slot(ui);
            let hidden_left = scroll.state.offset.x;
            let hidden_right = scroll.content_size.x - scroll.inner_rect.width() - hidden_left;
            let page = scroll.inner_rect.width() * SCROLL_STEP;
            let mut press = |ui: &mut Ui, rect: Option<Rect>, glyph, label, enabled, by| {
                let Some(rect) = rect else { return };
                let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
                if IconButton::new(glyph, label)
                    .size(Size::Sm)
                    .enabled(enabled)
                    .id_salt(id.with(label))
                    .show(&mut child)
                    .clicked()
                {
                    state.step = Some(by);
                    ctx.request_repaint();
                }
            };
            press(
                ui,
                before,
                icon::CARET_LEFT,
                "Scroll tabs left",
                hidden_left > 0.5,
                -page,
            );
            press(
                ui,
                after,
                icon::CARET_RIGHT,
                "Scroll tabs right",
                hidden_right > 0.5,
                page,
            );
            if let Some(rect) = all {
                let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
                let button = IconButton::new(icon::CARET_DOWN, "All repositories")
                    .size(Size::Sm)
                    .id_salt(id.with("all"))
                    .show(&mut child);
                if button.clicked() {
                    ctx.data_mut(|d| d.insert_temp(id.with("all-opened"), true));
                }
                if let Some(Some(i)) = menu::menu(&button, |m| {
                    all_tabs(m, self.tabs, self.active, id.with("all-opened"))
                }) {
                    event = Some(DocTabsEvent::Select(i));
                }
            }
        }

        state.drawn = drawn;
        ctx.data_mut(|d| d.insert_temp(state_id, state));
        KitResponse {
            response: scroll.inner,
            value: event,
        }
    }
}

/// The list of every open tab: its icon and name, the active one checked. Picks the index.
fn all_tabs(
    m: &mut menu::Menu<'_>,
    tabs: &[DocTab<'_>],
    active: usize,
    opened: Id,
) -> Option<usize> {
    let ctx = m.ui.ctx().clone();
    // Opened just now: the active tab is brought into sight (a sizing pass shows nothing).
    let first = !m.ui.is_sizing_pass()
        && ctx
            .data_mut(|d| d.remove_temp::<bool>(opened))
            .unwrap_or(false);
    // Tabs may share a name (two folders called "app"), so each item has its own identity.
    let items: Vec<_> = tabs
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let item = m.item(t.icon, t.name).checked(i == active);
            (i, item.id_salt(("kit-doc-tab-list", i)))
        })
        .collect();
    // A long list scrolls inside the menu.
    let room = (ctx.content_rect().height() * LIST_SHARE).max(MIN_LIST);
    let ground = overlay::ground(&ctx, overlay::Layer::Menu);
    let mut picked = None;
    overlay::scroll_body(m.ui, "kit-doc-tab-list", room, ground, |ui| {
        for (i, item) in items {
            let r = item.show(ui);
            if i == active && first {
                r.scroll_to_me(Some(egui::Align::Center));
            }
            if r.clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// What the tab strip sits on: the deck's chrome in Standard, the window's ground in Deeper (whose
/// tab strip is not a bar of its own).
pub fn strip_ground(ctx: &egui::Context) -> Fill {
    let s = theme::surfaces(ctx);
    match theme::style() {
        Style::Standard => s.deck,
        Style::Deeper => Fill::flat(s.bg),
    }
}

enum TabEvent {
    Select,
    Close,
}

/// The counts' text laid out in `accent`: the tab's width and its painting both need it.
fn counts_galley(ui: &Ui, tab: &DocTab<'_>) -> Option<std::sync::Arc<egui::Galley>> {
    let p = theme::palette(ui.ctx());
    tab.counts().map(|text| {
        ui.painter().layout_no_wrap(
            text,
            FontId::new(COUNT_SIZE, FontFamily::Monospace),
            p.accent,
        )
    })
}

/// The width a tab needs to show its whole name and counts, at least `MIN_WIDTH`.
fn natural_width(ui: &Ui, tab: &DocTab<'_>) -> f32 {
    let m = theme::metrics();
    let counts = counts_galley(ui, tab);
    let name_w = ui
        .painter()
        .layout_no_wrap(
            tab.name.to_owned(),
            FontId::new(m.body_size, theme::medium()),
            Color32::PLACEHOLDER,
        )
        .size()
        .x;
    let fixed = PAD_LEFT
        + m.icon_sm
        + GAP
        + counts.map_or(0.0, |g| g.size().x + GAP)
        + GAP
        + Size::Xs.height()
        + PAD_RIGHT;
    (fixed + name_w + NAME_SLACK).max(MIN_WIDTH)
}

/// One tab, `width` wide: its name is ellipsised only when `width` is less than it needs.
fn doc_tab(
    ui: &mut Ui,
    tab: &DocTab<'_>,
    selected: bool,
    force: Option<ForceState>,
    width: f32,
) -> (Response, Option<TabEvent>) {
    let ctx = ui.ctx().clone();
    let p = theme::palette(&ctx);
    let m = theme::metrics();
    let counts = counts_galley(ui, tab);
    let close_side = Size::Xs.height();
    let (_, rect) = ui.allocate_space(vec2(width, m.doc_tab_height));
    let resp = ui.interact(rect, tab.key, Sense::click());
    let st = Interact::of(&ctx, &resp, true, force);
    // The pointer over the close button still hovers the tab.
    let hover = st
        .hover
        .max(if force.is_none() && ui.rect_contains_pointer(rect) {
            anim::ease_bool(
                &ctx,
                tab.key.with("kit-doc-tab-over"),
                true,
                motion::FAST,
                motion::FAST_OUT,
            )
        } else {
            0.0
        });
    let radius = CornerRadius::same(m.doc_tab_radius);
    let mut event = None;
    if ui.is_rect_visible(rect) {
        let painter = ui.painter().clone();
        if selected {
            active_tab(&painter, rect, radius, hover);
        } else if hover > 0.0 {
            painter.rect_filled(rect, radius, p.hover().gamma_multiply(hover));
        }
        let cy = rect.center().y;
        let mut x = rect.left() + PAD_LEFT;
        let icon_rect =
            Rect::from_center_size(pos2(x + m.icon_sm / 2.0, cy), Vec2::splat(m.icon_sm));
        if tab.busy {
            // A child, so the spinner takes no room from the strip's layout.
            ui.new_child(UiBuilder::new().max_rect(icon_rect)).add(
                Spinner::new()
                    .size(Size::Xs)
                    .label(tab.name.to_owned())
                    .id_salt(tab.key.with("busy")),
            );
        } else {
            let color = if selected {
                p.text_dim
            } else {
                p.text_faint.lerp_to_gamma(p.text_dim, hover)
            };
            let g = painter.layout_no_wrap(
                tab.icon.to_owned(),
                theme::icon_font(m.icon_sm, selected),
                color,
            );
            painter.galley(icon_rect.center() - g.size() / 2.0, g, color);
        }
        x += m.icon_sm + GAP;
        let close_left = rect.right() - PAD_RIGHT - close_side;
        let counts_w = counts.as_ref().map_or(0.0, |g| g.size().x + GAP);
        let label_right = close_left - GAP - counts_w;
        let label_color = if selected {
            p.text
        } else {
            p.text_dim.lerp_to_gamma(p.text, hover)
        };
        let g = painter.layout_job(name_job(tab.name, label_color, label_right - x));
        let label_w = g.size().x;
        painter.galley(pos2(x, cy - g.size().y / 2.0), g, label_color);
        if let Some(g) = counts {
            painter.galley(pos2(x + label_w + GAP, cy - g.size().y / 2.0), g, p.accent);
        }
        if selected || hover > 0.0 {
            let at = Rect::from_min_size(
                pos2(close_left, cy - close_side / 2.0),
                Vec2::splat(close_side),
            );
            let mut slot = ui.new_child(UiBuilder::new().max_rect(at));
            if !selected {
                slot.multiply_opacity(hover);
            }
            if IconButton::new(icon::X, format!("Close {}", tab.name))
                .size(Size::Xs)
                .id_salt(tab.key.with("close"))
                .show(&mut slot)
                .clicked()
            {
                event = Some(TabEvent::Close);
            }
        }
        if st.focus > 0.0 {
            paint::inset_ring(&painter, rect, radius, p.focus.gamma_multiply(st.focus));
        }
    }
    if event.is_none() {
        if resp.clicked() {
            event = Some(TabEvent::Select);
        } else if resp.middle_clicked() {
            event = Some(TabEvent::Close);
        }
    }
    let spoken = tab.spoken();
    resp.widget_info(|| WidgetInfo::selected(WidgetType::Button, true, selected, &spoken));
    ctx.accesskit_node_builder(resp.id, |n| {
        n.set_role(Role::Tab);
        n.clear_toggled();
        n.set_selected(selected);
    });
    // The tooltip says something the tab does not: the whole name of a shortened one, or the
    // detail and the text its caller chose. A fully visible name is not repeated.
    let cut = width + 0.5 < natural_width(ui, tab);
    if !(cut || tab.detail.is_some() || tab.tooltip.is_some()) {
        return (resp, event);
    }
    let mut tip = resp.kit_tooltip(tab.tooltip.clone().unwrap_or_else(|| tab.name.to_owned()));
    if let Some(d) = &tab.detail {
        tip = tip.detail(d.clone());
    }
    let resp = tip.show();
    (resp, event)
}

/// The active tab's mark in the active theme's [`TabStyle`]: `Lifted` the `tab` fill with its
/// `tab_edge` ring (and `Elevation::Tab` in Deeper); `Underline` no fill, a `focus` underline (the
/// hover wash while hovered).
pub(crate) fn active_tab(painter: &egui::Painter, rect: Rect, radius: CornerRadius, hover: f32) {
    let p = theme::palette(painter.ctx());
    let style = theme::style();
    let s = p.surfaces(style);
    let lifted = || {
        paint::surface(
            painter,
            rect,
            radius,
            p,
            style,
            Elevation::Tab,
            s.tab,
            Some(s.tab_edge),
        );
    };
    match theme::active_design().active_tab {
        TabStyle::Lifted => lifted(),
        TabStyle::Underline { width } => {
            if hover > 0.0 {
                painter.rect_filled(rect, radius, p.hover().gamma_multiply(hover));
            }
            let inset = f32::from(radius.sw).max(theme::XS);
            let bar = Rect::from_min_max(
                pos2(rect.left() + inset, rect.bottom() - width),
                pos2(rect.right() - inset, rect.bottom()),
            );
            let round = CornerRadius::same(crate::saturate_u8((width / 2.0).round()));
            painter.rect_filled(bar, round, p.focus);
        }
    }
}

/// The name on one line, ellipsised to `width`.
fn name_job(name: &str, color: Color32, width: f32) -> LayoutJob {
    let m = theme::metrics();
    let mut job = LayoutJob::simple(
        name.to_owned(),
        FontId::new(m.body_size, theme::medium()),
        color,
        width.max(1.0),
    );
    job.wrap = TextWrapping {
        max_width: width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    job
}

/// A closed tab collapsing: its width eases to zero while its name fades.
fn closing_tab(ui: &mut Ui, c: &Closing, now: f64, seconds: f32, height: f32) {
    let t = (crate::to_f32(now - c.start) / seconds).clamp(0.0, 1.0);
    let k = motion::EASE_OUT.at(t);
    let width = c.width * (1.0 - k);
    // The gap after it closes with it.
    let (rect, _) =
        ui.allocate_exact_size(vec2((width - BETWEEN * k).max(0.0), height), Sense::hover());
    let p = theme::palette(ui.ctx());
    let m = theme::metrics();
    let painter = ui.painter_at(rect);
    let color = p.text_dim.gamma_multiply(1.0 - k);
    let g = painter.layout_job(name_job(
        &c.name,
        color,
        (c.width - PAD_LEFT - m.icon_sm - GAP - PAD_RIGHT).max(1.0),
    ));
    painter.galley(
        pos2(
            rect.left() + PAD_LEFT + m.icon_sm + GAP,
            rect.center().y - g.size().y / 2.0,
        ),
        g,
        color,
    );
}

// ------------------------------------------------------------------------------------- gallery

/// The gallery's tabs: an active tab with counts, a busy one, a hovered one, a long name, and a
/// strip too narrow for its tabs (scrolled part-way, both edges fading).
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
    let tabs = [
        DocTab::new("showcase", "showcase")
            .sync(2, 0)
            .detail("~/src/showcase · Ctrl+1"),
        DocTab::new("canopy", "canopy")
            .busy(true)
            .detail("~/src/canopy · Ctrl+2"),
        DocTab::new("website", "website")
            .icon(icon::GLOBE)
            .sync(0, 3)
            .detail("~/src/website · Ctrl+3"),
        DocTab::new("long", "a-repository-with-a-very-long-name")
            .tooltip("a-repository-with-a-very-long-name")
            .detail("~/src/a-repository-with-a-very-long-name · Ctrl+4"),
    ];
    small(
        ui,
        "On the title bar: active with counts, busy, hovered; every name whole while there is room",
    );
    on_deck(ui, |ui| {
        let _ = DocTabs::new(&tabs[..3], 0)
            .id_salt("gallery-doc-tabs")
            .force(2, ForceState::HOVER)
            .show(ui);
    });
    ui.add_space(theme::SM);
    small(
        ui,
        "Short of room: the longest name gives up width first and is ellipsised",
    );
    on_deck(ui, |ui| {
        let _ = DocTabs::new(&tabs, 0)
            .id_salt("gallery-doc-tabs-squeezed")
            .max_width(SQUEEZE_FLOOR * 3.0 + 160.0)
            .show(ui);
    });
    ui.add_space(theme::SM);
    small(
        ui,
        "Overflow: scroll buttons (the left one disabled at the start), the list of every tab, the right edge fades",
    );
    let many = [
        DocTab::new("showcase", "showcase").sync(2, 0),
        DocTab::new("canopy", "canopy").busy(true),
        DocTab::new("website", "website")
            .icon(icon::GLOBE)
            .sync(0, 3),
        DocTab::new("api", "api-gateway"),
        DocTab::new("infra", "infrastructure"),
        DocTab::new("docs", "docs-site"),
        DocTab::new("mobile", "mobile-app"),
        DocTab::new("design", "design-system"),
    ];
    on_deck(ui, |ui| {
        let _ = DocTabs::new(&many, 0)
            .id_salt("gallery-doc-tabs-overflow")
            .max_width((ui.available_width() * 0.7).max(SQUEEZE_FLOOR * 3.0))
            .show(ui);
    });
    ui.add_space(theme::SM);
    small(ui, "Overflow, scrolled part-way: both edges fade");
    on_deck(ui, |ui| {
        let _ = DocTabs::new(&many, 3)
            .id_salt("gallery-doc-tabs-overflow-scrolled")
            .max_width((ui.available_width() * 0.7).max(SQUEEZE_FLOOR * 3.0))
            .offset(SQUEEZE_FLOOR * 1.5)
            .show(ui);
    });
}

/// A strip of the title bar's deck, the tabs' own ground, for the gallery.
fn on_deck(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    let m = theme::metrics();
    let s = theme::surfaces(ui.ctx());
    let slot = ui.painter().add(egui::Shape::Noop);
    let r = egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(crate::points_i8(theme::SM), 4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(m.doc_tab_height);
            add(ui);
        });
    let rect = r.response.rect;
    let radius = CornerRadius::same(m.radius);
    ui.painter().set(
        slot,
        egui::Shape::Vec(vec![
            paint::fill_shape(ui.ctx(), rect, radius, strip_ground(ui.ctx())),
            egui::epaint::RectShape::stroke(
                rect,
                radius,
                egui::Stroke::new(paint::SEAM, s.deck_edge),
                egui::StrokeKind::Inside,
            )
            .into(),
        ]),
    );
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

    fn frame(
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        f: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let mut f = f;
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, 200.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| f(ui),
        );
        out.textures_delta.clear();
        out
    }

    fn text_rect(out: &egui::FullOutput, text: &str) -> Option<Rect> {
        fn find(shape: &egui::Shape, text: &str) -> Option<Rect> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.visual_bounding_rect()),
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        out.shapes.iter().find_map(|c| find(&c.shape, text))
    }

    #[test]
    fn a_click_selects_and_a_middle_click_closes() {
        let ctx = context();
        let tabs = [DocTab::new(1, "alpha"), DocTab::new(2, "beta")];
        let mut at = egui::Pos2::ZERO;
        let mut got = Vec::new();
        for (n, button) in [
            (0, None),
            (1, Some(egui::PointerButton::Primary)),
            (2, None),
            (3, Some(egui::PointerButton::Middle)),
        ] {
            let mut events = vec![egui::Event::PointerMoved(at)];
            if let Some(b) = button {
                for pressed in [true, false] {
                    events.push(egui::Event::PointerButton {
                        pos: at,
                        button: b,
                        pressed,
                        modifiers: egui::Modifiers::default(),
                    });
                }
            }
            let out = frame(&ctx, f64::from(n) * 0.1, events, |ui| {
                if let Some(e) = DocTabs::new(&tabs, 0).show(ui).value {
                    got.push(e);
                }
            });
            if n == 0 {
                at = text_rect(&out, "beta").expect("beta tab").center();
            }
        }
        assert_eq!(got, vec![DocTabsEvent::Select(1), DocTabsEvent::Close(1)]);
    }

    #[test]
    fn a_tab_paints_its_own_icon() {
        let ctx = context();
        let tabs = [
            DocTab::new(1, "alpha").icon(icon::ROCKET_LAUNCH),
            DocTab::new(2, "beta"),
        ];
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&tabs, 0).show(ui);
        });
        assert!(
            text_rect(&out, icon::ROCKET_LAUNCH).is_some(),
            "chosen icon"
        );
        assert!(
            text_rect(&out, icon::FOLDER_SIMPLE).is_some(),
            "default folder"
        );
    }

    #[test]
    fn the_tabs_announce_a_tab_list_with_the_selected_tab() {
        let ctx = context();
        ctx.enable_accesskit();
        let tabs = [DocTab::new(1, "alpha").sync(2, 1), DocTab::new(2, "beta")];
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&tabs, 0).show(ui);
        });
        let nodes = out
            .platform_output
            .accesskit_update
            .expect("accesskit")
            .nodes;
        assert!(
            nodes
                .iter()
                .any(|(_, n)| n.role() == Role::TabList && n.label() == Some("Repositories"))
        );
        let tab = |name: &str| {
            nodes
                .iter()
                .find(|(_, n)| {
                    n.role() == Role::Tab && n.label().is_some_and(|l| l.starts_with(name))
                })
                .map(|(_, n)| (n.is_selected(), n.label().map(str::to_owned)))
        };
        assert_eq!(
            tab("alpha"),
            Some((Some(true), Some("alpha, 2 to push, 1 to pull".to_owned())))
        );
        assert_eq!(tab("beta").map(|t| t.0), Some(Some(false)));
        assert!(
            nodes.iter().any(|(_, n)| n.label() == Some("Close alpha")),
            "the active tab's close button is named"
        );
    }

    #[test]
    fn a_closed_tab_collapses_before_it_is_gone() {
        let ctx = context();
        motion::set_reduced(false);
        let both = [DocTab::new(1, "alpha"), DocTab::new(2, "beta")];
        let one = [DocTab::new(2, "beta")];
        let left_of_beta = |out: &egui::FullOutput| text_rect(out, "beta").map(|r| r.left());
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&both, 0).show(ui);
        });
        let before = left_of_beta(&out).expect("beta");
        // Half-way through the collapse, alpha still takes some room and beta has not arrived.
        let _ = frame(&ctx, 1.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&one, 0).show(ui);
        });
        let out = frame(
            &ctx,
            1.0 + f64::from(motion::TAB_CLOSE) / 2.0,
            Vec::new(),
            |ui| {
                let _ = DocTabs::new(&one, 0).show(ui);
            },
        );
        let middle = left_of_beta(&out).expect("beta");
        let out = frame(&ctx, 2.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&one, 0).show(ui);
        });
        let after = left_of_beta(&out).expect("beta");
        assert!(
            after < middle && middle < before,
            "{before} {middle} {after}"
        );
        assert!(
            text_rect(&out, "alpha").is_none(),
            "the collapsed tab is gone"
        );
    }

    #[test]
    fn a_closed_tab_goes_at_once_with_reduced_motion() {
        let ctx = context();
        motion::set_reduced(true);
        let both = [DocTab::new(1, "alpha"), DocTab::new(2, "beta")];
        let one = [DocTab::new(2, "beta")];
        let _ = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&both, 0).show(ui);
        });
        let out = frame(&ctx, 0.05, Vec::new(), |ui| {
            let _ = DocTabs::new(&one, 0).show(ui);
        });
        assert!(text_rect(&out, "alpha").is_none());
        motion::set_reduced(false);
    }

    /// The painted galley of the text `name`, and whether it was cut to an ellipsis.
    fn name_shown(
        out: &egui::FullOutput,
        name: &str,
    ) -> Option<(std::sync::Arc<egui::Galley>, bool)> {
        fn find(shape: &egui::Shape, name: &str) -> Option<std::sync::Arc<egui::Galley>> {
            match shape {
                egui::Shape::Text(t) if t.galley.text() == name => {
                    Some(std::sync::Arc::clone(&t.galley))
                }
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, name)),
                _ => None,
            }
        }
        let g = out.shapes.iter().find_map(|c| find(&c.shape, name))?;
        let cut = g.rows.iter().any(|r| r.text().ends_with('…'));
        Some((g, cut))
    }

    fn numbered(count: usize) -> Vec<String> {
        (0..count).map(|i| format!("repository-{i:02}")).collect()
    }

    fn tabs_of(names: &[String]) -> Vec<DocTab<'_>> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| DocTab::new(i, n.as_str()))
            .collect()
    }

    /// The AccessKit nodes a frame produced, as (label, disabled).
    fn labelled(out: &egui::FullOutput) -> Vec<(String, bool)> {
        out.platform_output
            .accesskit_update
            .as_ref()
            .expect("accesskit")
            .nodes
            .iter()
            .filter_map(|(_, n)| n.label().map(|l| (l.to_owned(), n.is_disabled())))
            .collect()
    }

    #[test]
    fn fit_gives_every_tab_the_width_its_name_needs_when_there_is_room() {
        let got = fit(&[120.0, 400.0, 300.0], 900.0, 4.0, SQUEEZE_FLOOR, None);
        assert_eq!(
            got,
            Fit {
                widths: vec![120.0, 400.0, 300.0],
                overflow: false
            }
        );
    }

    #[test]
    fn fit_takes_width_from_the_longest_names_first() {
        // 120 + 400 + 300 + 8 of gaps against 700: 220 to give up, all of it from the two long ones.
        let got = fit(&[120.0, 400.0, 300.0], 700.0, 4.0, SQUEEZE_FLOOR, None);
        assert!(!got.overflow);
        assert!((got.widths[0] - 120.0).abs() < f32::EPSILON, "{got:?}");
        assert!((got.widths[1] - got.widths[2]).abs() < 0.01, "{got:?}");
        let total: f32 = got.widths.iter().sum::<f32>() + 8.0;
        assert!((total - 700.0).abs() < 0.01, "{got:?}");
        assert!(got.widths[1] < 300.0 && got.widths[1] > SQUEEZE_FLOOR);
    }

    #[test]
    fn fit_leaves_the_active_tab_whole_and_squeezes_the_others() {
        let natural = [300.0, 280.0, 260.0];
        let plain = fit(&natural, 740.0, 4.0, SQUEEZE_FLOOR, None);
        let kept = fit(&natural, 740.0, 4.0, SQUEEZE_FLOOR, Some(0));
        assert!(!kept.overflow);
        assert!((kept.widths[0] - 300.0).abs() < f32::EPSILON, "{kept:?}");
        assert!(kept.widths[0] > plain.widths[0], "{plain:?} {kept:?}");
        assert!(kept.widths[1] < plain.widths[1], "{plain:?} {kept:?}");
        // However long its name, it takes no more than its share of the strip.
        let huge = fit(&[2000.0, 280.0], 800.0, 4.0, SQUEEZE_FLOOR, Some(0));
        assert!(huge.widths[0] <= 800.0 * ACTIVE_SHARE + 0.01, "{huge:?}");
    }

    #[test]
    fn fit_scrolls_beyond_the_floor_and_gives_the_names_their_width_back() {
        let natural = [300.0, 200.0, 120.0, 130.0, 400.0, 100.0];
        let got = fit(&natural, 500.0, 4.0, SQUEEZE_FLOOR, None);
        assert!(got.overflow);
        let expect: Vec<f32> = natural.iter().map(|w| w.min(SCROLL_CAP)).collect();
        assert_eq!(got.widths, expect);
        // One tab fewer than the squeeze can hold is not an overflow yet.
        let fits = fit(&natural[2..5], 500.0, 4.0, SQUEEZE_FLOOR, None);
        assert!(!fits.overflow, "{fits:?}");
    }

    #[test]
    fn fit_lets_a_lone_tab_take_the_room_without_scrolling() {
        let got = fit(&[640.0], 300.0, 4.0, SQUEEZE_FLOOR, None);
        assert_eq!(
            got,
            Fit {
                widths: vec![300.0],
                overflow: false
            }
        );
        assert_eq!(
            fit(&[], 300.0, 4.0, SQUEEZE_FLOOR, None).widths,
            Vec::<f32>::new()
        );
    }

    #[test]
    fn a_long_name_shows_in_full_while_the_strip_has_room() {
        let ctx = context();
        let name = "a-repository-with-a-name-far-longer-than-the-old-widest-tab-allowed";
        let tabs = [DocTab::new(1, name), DocTab::new(2, "beta")];
        let mut width = 0.0;
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let r = DocTabs::new(&tabs, 0).max_width(880.0).show(ui);
            width = r.response.rect.width();
        });
        let (g, cut) = name_shown(&out, name).expect("the name");
        assert!(!cut, "ellipsised although the strip has room");
        assert!(g.size().x > 400.0, "the whole name is {}", g.size().x);
        assert!(width < 880.0, "the strip is {width} wide");
    }

    #[test]
    fn names_are_ellipsised_only_when_the_strip_lacks_the_room() {
        let ctx = context();
        let name = "a-repository-with-a-name-far-longer-than-the-old-widest-tab-allowed";
        let tabs = [DocTab::new(1, name), DocTab::new(2, "beta")];
        let mut width = 0.0;
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let r = DocTabs::new(&tabs, 0).max_width(400.0).show(ui);
            width = r.response.rect.width();
        });
        let (_, cut) = name_shown(&out, name).expect("the name");
        assert!(cut, "the long name has to give way");
        assert!(width <= 400.5, "the strip is {width} wide");
        let (_, cut) = name_shown(&out, "beta").expect("beta");
        assert!(!cut, "the short name keeps its width");
    }

    #[test]
    fn the_scroll_controls_exist_only_while_the_tabs_overflow() {
        let ctx = context();
        ctx.enable_accesskit();
        let names = numbered(14);
        let tabs = tabs_of(&names);
        let few = tabs_of(&names[..3]);
        let glyphs = [icon::CARET_LEFT, icon::CARET_RIGHT, icon::CARET_DOWN];
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&few, 0).max_width(800.0).show(ui);
        });
        for glyph in glyphs {
            assert!(
                text_rect(&out, glyph).is_none(),
                "a control without overflow"
            );
        }
        let out = frame(&ctx, 0.1, Vec::new(), |ui| {
            let _ = DocTabs::new(&tabs, 0).max_width(800.0).show(ui);
        });
        for glyph in glyphs {
            assert!(text_rect(&out, glyph).is_some(), "a control is missing");
        }
        // At the start the left button has nothing to scroll to, so it is unavailable (on Linux
        // its name says so too); the right one and the list are available.
        let nodes = labelled(&out);
        let disabled = |label: &str| {
            nodes
                .iter()
                .find(|(l, _)| l.starts_with(label))
                .unwrap_or_else(|| panic!("{label}: {nodes:?}"))
                .1
        };
        assert!(disabled("Scroll tabs left"));
        assert!(!disabled("Scroll tabs right"));
        assert!(!disabled("All repositories"));
        // Back to room: the controls go again.
        let out = frame(&ctx, 0.2, Vec::new(), |ui| {
            let _ = DocTabs::new(&few, 0).max_width(800.0).show(ui);
        });
        for glyph in glyphs {
            assert!(text_rect(&out, glyph).is_none(), "a control stayed");
        }
    }

    #[test]
    fn overflowing_tabs_stay_inside_the_room_they_were_given() {
        let ctx = context();
        let names = numbered(14);
        let tabs = tabs_of(&names);
        let mut right = 0.0;
        let _ = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&tabs, 0).max_width(600.0).show(ui);
            right = ui.min_rect().right();
        });
        assert!(right <= 600.5, "the strip and its controls reach {right}");
    }

    /// Frames at 20 ms steps from `from`, to let a scroll glide finish.
    fn settle(
        ctx: &egui::Context,
        from: f64,
        events: &mut Vec<egui::Event>,
        mut f: impl FnMut(&mut Ui),
    ) -> egui::FullOutput {
        let mut out = frame(ctx, from, std::mem::take(events), &mut f);
        for n in 1..40 {
            out = frame(ctx, from + f64::from(n) * 0.02, Vec::new(), &mut f);
        }
        out
    }

    fn click(at: egui::Pos2) -> Vec<egui::Event> {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        vec![egui::Event::PointerMoved(at), button(true), button(false)]
    }

    #[test]
    fn the_scroll_buttons_move_the_tabs_a_page_at_a_time() {
        let ctx = context();
        let names = numbered(14);
        let tabs = tabs_of(&names);
        let last = names[13].as_str();
        let mut none = Vec::new();
        let out = settle(&ctx, 0.0, &mut none, |ui| {
            let _ = DocTabs::new(&tabs, 0).max_width(700.0).show(ui);
        });
        let first = text_rect(&out, &names[0]).expect("first").left();
        let right = text_rect(&out, icon::CARET_RIGHT)
            .expect("right button")
            .center();
        // Hover first, then press, like a pointer does.
        let mut events = vec![egui::Event::PointerMoved(right)];
        let _ = settle(&ctx, 2.0, &mut events, |ui| {
            let _ = DocTabs::new(&tabs, 0).max_width(700.0).show(ui);
        });
        let mut events = click(right);
        let out = settle(&ctx, 4.0, &mut events, |ui| {
            let _ = DocTabs::new(&tabs, 0).max_width(700.0).show(ui);
        });
        let moved = first - text_rect(&out, &names[0]).map_or(f32::NEG_INFINITY, |r| r.left());
        assert!(
            moved > 300.0,
            "one click moved the tabs {moved} points; the last one is at {:?}",
            text_rect(&out, last)
        );
    }

    #[test]
    fn the_wheel_scrolls_the_strip() {
        let ctx = context();
        let names = numbered(14);
        let tabs = tabs_of(&names);
        let mut none = Vec::new();
        let out = settle(&ctx, 0.0, &mut none, |ui| {
            let _ = DocTabs::new(&tabs, 0).max_width(700.0).show(ui);
        });
        let before = text_rect(&out, &names[1]).expect("tab").center();
        // The pointer rests on the strip a frame before the wheel turns.
        let mut events = vec![egui::Event::PointerMoved(before)];
        let _ = settle(&ctx, 2.0, &mut events, |ui| {
            let _ = DocTabs::new(&tabs, 0).max_width(700.0).show(ui);
        });
        let mut events = vec![egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            phase: egui::TouchPhase::Move,
            delta: vec2(0.0, -200.0),
            modifiers: egui::Modifiers::default(),
        }];
        let out = settle(&ctx, 4.0, &mut events, |ui| {
            let _ = DocTabs::new(&tabs, 0).max_width(700.0).show(ui);
        });
        let after = text_rect(&out, &names[1]).map_or(f32::INFINITY, |r| r.center().x);
        assert!(after < before.x - 100.0, "{} -> {after}", before.x);
    }

    #[test]
    fn the_active_tab_is_scrolled_into_view_when_it_changes() {
        let ctx = context();
        let names = numbered(14);
        let tabs = tabs_of(&names);
        let mut none = Vec::new();
        for active in [0, 13, 6, 0] {
            let out = settle(&ctx, f64::from(active) * 10.0, &mut none, |ui| {
                let _ = DocTabs::new(&tabs, usize::try_from(active).expect("index"))
                    .max_width(700.0)
                    .show(ui);
            });
            let name = &names[usize::try_from(active).expect("index")];
            let at = text_rect(&out, name).unwrap_or_else(|| panic!("{name} not painted"));
            assert!(
                at.left() >= 0.0 && at.right() <= 700.0,
                "tab {active} is at {at:?}, outside the strip"
            );
        }
    }

    /// usersim F15: Tab walks focus along the strip and scrolls to each tab; once focus has left
    /// the strip (Esc) the active tab comes back into view.
    #[test]
    fn the_active_tab_returns_to_view_when_focus_leaves_the_strip() {
        let ctx = context();
        let names = numbered(14);
        let tabs = tabs_of(&names);
        let mut none = Vec::new();
        let mut show = |ui: &mut Ui| {
            let _ = DocTabs::new(&tabs, 0).max_width(700.0).show(ui);
        };
        let inside = |out: &egui::FullOutput, i: usize| {
            text_rect(out, &names[i]).is_some_and(|r| r.left() >= 0.0 && r.right() <= 700.0)
        };
        let out = settle(&ctx, 0.0, &mut none, &mut show);
        assert!(inside(&out, 0), "the active tab starts in view");
        // Tab walks onto the first tab, then along the strip to the thirteenth.
        let tab = |time: f64| {
            let key = |pressed| egui::Event::Key {
                key: egui::Key::Tab,
                physical_key: Some(egui::Key::Tab),
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            (time, vec![key(true), key(false)])
        };
        for n in 0..13 {
            let (time, events) = tab(10.0 + f64::from(n) * 0.5);
            frame(&ctx, time, events, &mut show);
        }
        let out = settle(&ctx, 20.0, &mut none, &mut show);
        assert!(inside(&out, 12), "the focused tab is scrolled to");
        assert!(!inside(&out, 0), "the active tab is out of view meanwhile");
        // Esc: egui drops focus, and the strip shows the active tab again.
        let esc = |pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: Some(egui::Key::Escape),
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let mut events = vec![esc(true), esc(false)];
        let out = settle(&ctx, 30.0, &mut events, &mut show);
        assert_eq!(
            ctx.memory(egui::Memory::focused),
            None,
            "Esc dropped the focus"
        );
        assert!(inside(&out, 0), "the active tab is back in view");
    }

    /// Runs the strip of `tabs` (the third active) in a window `height` tall at `time`.
    fn list_frame(
        ctx: &egui::Context,
        tabs: &[DocTab<'_>],
        height: f32,
        time: f64,
        events: Vec<egui::Event>,
        got: &mut Vec<DocTabsEvent>,
    ) -> egui::FullOutput {
        let mut out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(900.0, height))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                if let Some(e) = DocTabs::new(tabs, 2).max_width(700.0).show(ui).value {
                    got.push(e);
                }
            },
        );
        out.textures_delta.clear();
        out
    }

    /// Opens the list of the tabs in a window `height` tall; what the open list painted.
    fn opened_list(
        tabs: &[DocTab<'_>],
        height: f32,
        got: &mut Vec<DocTabsEvent>,
    ) -> (egui::Context, egui::FullOutput) {
        let ctx = context();
        let out = list_frame(&ctx, tabs, height, 0.0, Vec::new(), got);
        let list = text_rect(&out, icon::CARET_DOWN)
            .expect("list button")
            .center();
        let moved = vec![egui::Event::PointerMoved(list)];
        let _ = list_frame(&ctx, tabs, height, 0.1, moved, got);
        let _ = list_frame(&ctx, tabs, height, 0.2, click(list), got);
        let mut out = list_frame(&ctx, tabs, height, 0.3, Vec::new(), got);
        for n in 0..10 {
            let time = 0.4 + f64::from(n) * 0.1;
            out = list_frame(&ctx, tabs, height, time, Vec::new(), got);
        }
        (ctx, out)
    }

    #[test]
    fn the_list_names_every_tab_and_picks_one() {
        let names = numbered(14);
        let tabs = tabs_of(&names);
        let mut got = Vec::new();
        let (ctx, out) = opened_list(&tabs, 1200.0, &mut got);
        // The last tab is far out of the strip's sight: only the list can be showing it.
        let pick = text_rect(&out, &names[13])
            .expect("the list shows every tab")
            .center();
        let moved = vec![egui::Event::PointerMoved(pick)];
        let _ = list_frame(&ctx, &tabs, 1200.0, 3.0, moved, &mut got);
        // Press, then release a frame later, as a menu item expects.
        let mut press_release = click(pick);
        let release = press_release.split_off(2);
        let _ = list_frame(&ctx, &tabs, 1200.0, 3.1, press_release, &mut got);
        let _ = list_frame(&ctx, &tabs, 1200.0, 3.2, release, &mut got);
        let _ = list_frame(&ctx, &tabs, 1200.0, 3.3, Vec::new(), &mut got);
        assert_eq!(got, vec![DocTabsEvent::Select(13)]);
    }

    #[test]
    fn a_long_list_scrolls_instead_of_leaving_the_window() {
        let names = numbered(14);
        let tabs = tabs_of(&names);
        let mut got = Vec::new();
        let (_, out) = opened_list(&tabs, 300.0, &mut got);
        let lowest = names
            .iter()
            .filter_map(|n| text_rect(&out, n))
            .map(|r| r.bottom())
            .fold(0.0_f32, f32::max);
        assert!(lowest > 0.0 && lowest <= 300.0, "the list reaches {lowest}");
        assert!(
            text_rect(&out, &names[13]).is_none(),
            "the last item waits below the fold"
        );
    }

    /// How many times `text` is painted once the pointer has rested on the first tab of `tabs`
    /// (in a strip `room` wide) for longer than the tooltip delay.
    fn painted_while_hovered(tabs: &[DocTab<'_>], room: f32, text: &str) -> usize {
        fn count(shape: &egui::Shape, text: &str) -> usize {
            match shape {
                egui::Shape::Text(t) => usize::from(t.galley.text() == text),
                egui::Shape::Vec(v) => v.iter().map(|s| count(s, text)).sum(),
                _ => 0,
            }
        }
        let ctx = context();
        let mut shown = 0;
        for i in 0..12 {
            let events = if i == 0 {
                vec![egui::Event::PointerMoved(pos2(40.0, 15.0))]
            } else {
                Vec::new()
            };
            let out = frame(&ctx, f64::from(i) * 0.2, events, |ui| {
                let _ = DocTabs::new(tabs, 0).max_width(room).show(ui);
            });
            shown = out.shapes.iter().map(|c| count(&c.shape, text)).sum();
        }
        shown
    }

    #[test]
    fn a_tooltip_that_only_repeats_the_visible_name_is_not_shown() {
        let tabs = [DocTab::new(1, "alpha")];
        assert_eq!(painted_while_hovered(&tabs, 800.0, "alpha"), 1, "the label");
    }

    #[test]
    fn the_tooltip_shows_the_whole_name_of_a_shortened_tab() {
        let name = "a-repository-with-a-name-far-longer-than-the-strip-has-room-for";
        let tabs = [DocTab::new(1, name), DocTab::new(2, "beta")];
        assert_eq!(
            painted_while_hovered(&tabs, 400.0, name),
            2,
            "label and tip"
        );
    }

    #[test]
    fn the_tooltip_shows_a_detail_or_a_chosen_text() {
        let tabs = [DocTab::new(1, "alpha").detail("~/src/alpha")];
        assert_eq!(painted_while_hovered(&tabs, 800.0, "~/src/alpha"), 1);
        assert_eq!(painted_while_hovered(&tabs, 800.0, "alpha"), 2);
        let tabs = [DocTab::new(1, "alpha").tooltip("alpha  ·  Fetching…")];
        assert_eq!(
            painted_while_hovered(&tabs, 800.0, "alpha  ·  Fetching…"),
            1
        );
    }

    #[test]
    fn a_busy_tab_keeps_its_width() {
        let ctx = context();
        let tabs = [DocTab::new(1, "alpha").busy(true), DocTab::new(2, "beta")];
        let out = frame(&ctx, 0.0, Vec::new(), |ui| {
            let _ = DocTabs::new(&tabs, 1).show(ui);
        });
        let alpha = text_rect(&out, "alpha").expect("alpha");
        let beta = text_rect(&out, "beta").expect("beta");
        assert!(
            alpha.right() + MIN_WIDTH / 2.0 < beta.left(),
            "the busy tab overlaps the next: {alpha:?} {beta:?}"
        );
    }

    /// The overflow fades melt into what the strip stands on: the deck in Standard, the bare
    /// ground in Deeper (whose tab strip is not a bar).
    #[test]
    fn overflow_fades_use_the_ground_the_strip_stands_on() {
        let tabs: Vec<DocTab<'_>> = (0..12).map(|i| DocTab::new(i, "repository")).collect();
        for style in [Style::Standard, Style::Deeper] {
            let ctx = context();
            theme::apply(
                &ctx,
                theme::ThemeId::Midnight,
                theme::Appearance {
                    style,
                    ..theme::Appearance::STILL
                },
            );
            let s = theme::surfaces(&ctx);
            let ground = match style {
                Style::Standard => s.deck.top,
                Style::Deeper => s.bg,
            };
            let edge = ground.gamma_multiply(paint::SCROLL_FADE_ALPHA);
            let out = frame(&ctx, 0.0, Vec::new(), |ui| {
                let _ = DocTabs::new(&tabs, 0)
                    .max_width(400.0)
                    .offset(300.0)
                    .show(ui);
            });
            let fades = out
                .shapes
                .iter()
                .filter(|c| match &c.shape {
                    egui::Shape::Mesh(m) => m.vertices.iter().any(|v| v.color == edge),
                    _ => false,
                })
                .count();
            assert_eq!(
                fades, 2,
                "{style:?}: both edges hide tabs and fade into the ground"
            );
        }
    }
}
