//! [`Select`]: a choice of one value from a list, replacing `egui::ComboBox`. Design spec 4.3.
//!
//! The trigger is drawn as a text field (the same well, edge, hover, focus halo, label, helper and
//! error as [`super::field`]), so a form's selects and fields read as one set, with the value and
//! a caret that turns over while the list is open; the list is a floating menu surface under the
//! trigger (above it when there is no room). Focus stays on the trigger: Up and Down open the list
//! and move the highlight, Home and End jump, Enter or Space choose, Esc closes and keeps focus,
//! and typing jumps to the first option that starts with what was typed (the buffer clears after
//! a second). Opening with the pointer animates in and closing fades out; from the keyboard both
//! are instant.
//!
//! A select never trims a label of ordinary length. The trigger widens to the value it shows, up to
//! the width the host has, and then wraps it onto more lines and grows taller; the list is as wide
//! as its longest row, up to its host, and then wraps the long rows, every row as tall as the
//! tallest. A label is cut only where it would take more than three rows, or past 4096 characters
//! (only a hostile value gets there): the trigger and each row then show their first rows ending in
//! an ellipsis, and the whole label is in a tooltip (its first 600 characters and how many there
//! are, when it is longer) and in what a screen reader hears.

use std::borrow::Cow;

use egui::accesskit::{Invalid, Role};
use egui::{
    Align, Align2, Color32, CornerRadius, Event, FontFamily, FontId, Id, Key, Layout, Modifiers,
    PointerButton, Popup, PopupCloseBehavior, RectAlign, Response, Sense, Ui, UiBuilder, Widget,
    WidgetInfo, WidgetType, pos2, vec2,
};
use egui_phosphor::regular as icon;

use super::chip::Badge;
use super::overlay::{self, Entrance, ExitMotion, TooltipExt as _};
use super::toggle::{component_id, gallery_caption, gallery_row, remembered};
use super::{ForceState, Interact, Size, Tone, anim, field, glass, paint};
use crate::motion;
use crate::theme::{self, Elevation, Fill};

/// The caret turns over in 0.25 s.
const CARET_TURN: f32 = 0.25;
/// The list: a popover's gap below the trigger and at most 8.5 rows tall. It is as wide as its
/// trigger or as its longest row, whichever is more, and lines up with the trigger's left edge
/// (its right edge when it would not fit to the right).
const LIST_GAP: f32 = overlay::POPOVER_GAP;
const LIST_ROWS: f32 = 8.5;
/// Menu items: inner padding and the gap before the check.
const ITEM_PAD: f32 = theme::SM;
/// Type-ahead: the buffer clears this long after the last key.
const TYPE_AHEAD_RESET: f64 = 1.0;

/// The most characters of a label the closed control and each list row lay out: a longer label
/// shows its first characters and an ellipsis, and its tooltip and accessible value say all of it.
/// Git caps a ref name near 4 KB, so only a hostile source (a crafted git config value) goes
/// past it: laid out whole and wrapped, a megabyte made the control 457,184 pt tall and took half
/// a second to lay out, and gave a scroll area around it a content height of that size
/// (NecturaLabs/Canopy#187).
const LABEL_MAX_CHARS: usize = 4096;

/// The most rows of text the closed control and each list row paint, with an ellipsis after the
/// last: the character cap alone still left a control 1,328 pt tall in a narrow host.
const SHOWN_ROWS: usize = 3;

/// The most characters of a label a tooltip says, then how many there are: laid out whole in a
/// 260 pt tooltip, a megabyte took 9.7 s to show and painted a galley 394,740 pt tall.
const TOOLTIP_MAX_CHARS: usize = 600;

/// What the closed trigger adds after the recommended value (the list marks it with a badge).
const RECOMMENDED_SUFFIX: &str = " (Recommended)";

/// What a select remembers between frames while its list is open.
#[derive(Debug, Clone, Default)]
struct ListState {
    highlight: Option<usize>,
    /// The last highlight came from the keyboard (scroll it into view).
    by_keyboard: bool,
    /// The list was opened from the keyboard: it shows and closes at once.
    opened_by_keyboard: bool,
    typed: String,
    typed_at: f64,
}

/// One value from a list: `Select::new(&mut minutes, &[(0, "Off"), (5, "Every 5 minutes")])`.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Select<'a, T: PartialEq + Clone> {
    value: &'a mut T,
    recommended: Option<T>,
    /// Accessible descriptions of options, read after the name (a note the label already shows).
    described: Vec<(T, &'a str)>,
    /// Options that cannot be chosen, with the tag that says why.
    unavailable: Vec<(T, &'a str)>,
    report_reselection: bool,
    /// In the order listed: the recommended value first.
    options: Cow<'a, [(T, &'a str)]>,
    label: Option<String>,
    name: Option<String>,
    placeholder: String,
    helper: Option<String>,
    error: Option<String>,
    width: Option<f32>,
    fit: bool,
    size: Size,
    enabled: bool,
    loading: bool,
    tooltip: Option<String>,
    disabled_reason: Option<String>,
    id_salt: Option<Id>,
    id: Option<Id>,
    force: Option<ForceState>,
    /// The last option stays below the scrolling rows, always in view.
    pin_last: bool,
}

impl<'a, T: PartialEq + Clone> Select<'a, T> {
    pub fn new(value: &'a mut T, options: &'a [(T, &'a str)]) -> Self {
        Self {
            value,
            recommended: None,
            described: Vec::new(),
            unavailable: Vec::new(),
            report_reselection: false,
            options: Cow::Borrowed(options),
            label: None,
            name: None,
            placeholder: String::new(),
            helper: None,
            error: None,
            width: None,
            fit: false,
            size: Size::Lg,
            enabled: true,
            loading: false,
            tooltip: None,
            disabled_reason: None,
            id_salt: None,
            id: None,
            force: None,
            pin_last: false,
        }
    }
    /// The tag of option `index` when it is unavailable.
    fn note_of(&self, index: usize) -> Option<&'a str> {
        let value = &self.options[index].0;
        self.unavailable
            .iter()
            .find(|(v, _)| v == value)
            .map(|(_, note)| *note)
    }

    /// The tag shown at the end of option `index`: its note when unavailable, else Recommended.
    fn tag_of(&self, index: usize) -> Option<Badge> {
        self.note_of(index)
            .map(|note| Badge::new(note).muted())
            .or_else(|| {
                (self.recommended.as_ref() == Some(&self.options[index].0))
                    .then(|| Badge::new("Recommended").tone(Tone::Accent))
            })
    }

    /// Whether the closed trigger says Recommended after option `index`: it is the recommended
    /// value and can be chosen (an unavailable one carries its note instead).
    fn marked_when_chosen(&self, index: usize) -> bool {
        self.note_of(index).is_none() && self.recommended.as_ref() == Some(&self.options[index].0)
    }

    /// What the closed trigger says for option `index`: its label, and Recommended after it when
    /// it is the recommended value, so the mark holds whether the value was chosen by hand,
    /// written by a button or read from a file.
    fn trigger_text(&self, index: usize) -> String {
        let label = self.options[index].1;
        if self.marked_when_chosen(index) {
            format!("{label}{RECOMMENDED_SUFFIX}")
        } else {
            label.to_owned()
        }
    }

    /// What the closed trigger paints for option `index`: [`Self::trigger_text`] with a label
    /// longer than [`LABEL_MAX_CHARS`] cut short.
    fn shown_text(&self, index: usize) -> String {
        let label = clipped(self.options[index].1);
        if self.marked_when_chosen(index) {
            format!("{label}{RECOMMENDED_SUFFIX}")
        } else {
            label.into_owned()
        }
    }

    /// The nearest option to `from` (exclusive) in the given direction that can be chosen; with no
    /// `from`, the first or the last one.
    fn available_from(&self, from: Option<usize>, forward: bool) -> Option<usize> {
        let n = self.options.len();
        let mut indices: Box<dyn Iterator<Item = usize>> = if forward {
            Box::new(from.map_or(0, |i| i + 1)..n)
        } else {
            Box::new((0..from.unwrap_or(n)).rev())
        };
        indices.find(|&i| self.note_of(i).is_none())
    }

    /// Marks this value with a Recommended badge in the list and an accessible description, lists
    /// it first, and has the closed trigger say "(Recommended)" after it while it is the value.
    pub fn recommended(mut self, value: T) -> Self {
        if let Some(at) = self.options.iter().position(|(v, _)| *v == value)
            && at > 0
        {
            self.options.to_mut()[..=at].rotate_right(1);
        }
        self.recommended = Some(value);
        self
    }
    /// Gives `value` an accessible description in the list, read after its name: for a mark the
    /// label already carries in text ("kitty (Default)").
    pub fn described(mut self, value: T, description: &'a str) -> Self {
        self.described.push((value, description));
        self
    }
    /// Marks `value` as unavailable ("Not installed"): listed dimmed with `note` as its tag, passed
    /// over by the arrow keys, Home, End and typing, not chosen by a click or Enter, and spoken as
    /// disabled with `note` as its description. It can still be the select's current value (a saved
    /// choice that has gone), which the closed control shows the same way.
    pub fn unavailable(mut self, value: T, note: &'a str) -> Self {
        self.unavailable.push((value, note));
        self
    }
    /// Reports an explicit selection even when it equals the current value, for pinning an
    /// inherited setting. Ordinary selects report only a different value.
    pub fn report_reselection(mut self) -> Self {
        self.report_reselection = true;
        self
    }
    /// Keeps the last option in view below the others, past a separator, however long the list
    /// is: a way out such as "Custom command…" after a list of programs. The keys still reach it
    /// in order.
    pub fn pin_last(mut self) -> Self {
        self.pin_last = true;
        self
    }
    /// A label above the trigger (also its accessible name).
    pub fn label(mut self, text: impl Into<String>) -> Self {
        self.label = Some(text.into());
        self
    }
    /// The accessible name when no label is shown.
    pub fn name(mut self, text: impl Into<String>) -> Self {
        self.name = Some(text.into());
        self
    }
    /// Shown in `text_faint` when the value is none of the options.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = text.into();
        self
    }
    /// A hint below the trigger (hidden while there is an error), as a text field shows one.
    pub fn helper(mut self, text: impl Into<String>) -> Self {
        self.helper = Some(text.into());
        self
    }
    /// An error below the trigger with a danger edge and halo, as a text field shows one.
    pub fn error(mut self, text: Option<&str>) -> Self {
        self.error = text.map(str::to_owned);
        self
    }
    /// Trigger width; the available width by default.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }
    /// The trigger is as wide as its longest option (or placeholder) needs, no wider ([`fit_width`]),
    /// and no wider than the room its host has.
    pub fn fit_to_options(mut self) -> Self {
        self.fit = true;
        self
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
    /// The options are still loading: a spinner replaces the caret and the list stays shut.
    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }
    /// Said on hover. A value the closed control cuts short says itself instead (its first 600
    /// characters and how many there are), so this is not shown while one is chosen.
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
    /// The select's own id, set by the caller (not derived from the parent `Ui`), so code
    /// elsewhere can focus its trigger ([`trigger_id`]) or open its list.
    pub fn id(mut self, id: Id) -> Self {
        self.id = Some(id);
        self
    }
    /// `open` shows the list (statically, at its final position); `hover` with `open` highlights
    /// the first option that is not chosen.
    pub(crate) fn show_forced(mut self, ui: &mut Ui, force: ForceState) -> Response {
        self.force = Some(force);
        self.show(ui)
    }

    /// Shows the trigger (and the list while open). The response is the trigger's and reports
    /// `changed` when a new value was chosen.
    pub fn show(mut self, ui: &mut Ui) -> Response {
        let width = match self.width {
            Some(w) => w,
            None if self.fit => {
                // The recommended value is the one that grows by its mark, so the trigger is as
                // wide for it as for the longest label, whichever is chosen.
                let marked: Vec<String> = (0..self.options.len())
                    .filter(|&i| self.marked_when_chosen(i))
                    .map(|i| self.shown_text(i))
                    .collect();
                let labels: Vec<Cow<'_, str>> =
                    self.options.iter().map(|(_, l)| clipped(l)).collect();
                // No wider than the host has: a label past it wraps, and a hostile one is cut.
                fit_width_of(
                    ui,
                    labels
                        .iter()
                        .map(AsRef::as_ref)
                        .chain(marked.iter().map(String::as_str))
                        .chain([self.placeholder.as_str()]),
                )
                .min(ui.available_width())
            }
            None => ui.available_width(),
        };
        // The value shown is cut short only past `LABEL_MAX_CHARS`: the trigger widens to it
        // (with its tag, when it is unavailable) up to the host's width, and beyond that the label
        // wraps, onto at most `SHOWN_ROWS` rows.
        let current = self.options.iter().position(|(v, _)| v == self.value);
        let text = current.map_or_else(|| self.placeholder.clone(), |i| self.shown_text(i));
        let tag = current.and_then(|i| self.note_of(i)).map_or(0.0, |note| {
            Badge::new(note).muted().measure(ui).x + ITEM_PAD
        });
        let need = fit_width_of(ui, [text.as_str()]) + tag;
        let width = width.max(need.min(ui.available_width()));
        self.width = Some(width);
        let id = self
            .id
            .unwrap_or_else(|| component_id(ui, "kit-select", self.id_salt));
        // Label, trigger and message stack like a text field's, in a slot of their own size that
        // the parent's layout places (at the right of a right-to-left row, centred in a centred
        // one). The stack's height is last frame's; a change is measured and redrawn at once.
        let height_id = id.with("kit-select-height");
        let height = ui
            .data(|d| d.get_temp::<f32>(height_id))
            .unwrap_or_else(|| self.size.height());
        let shown =
            ui.allocate_ui_with_layout(vec2(width, height), Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                self.show_inner(ui, id)
            });
        let used = shown.response.rect.height();
        if (used - height).abs() > 0.5 {
            ui.data_mut(|d| d.insert_temp(height_id, used));
            ui.ctx().request_discard("select measured");
        }
        shown.inner
    }

    /// The open list's width and the height of each of its rows, for a trigger `trigger_w` wide in
    /// a host `bounds_w` wide. The list is as wide as its widest row (a row is its label, its tag,
    /// the check and their padding) and no narrower than the trigger, but never wider than the
    /// host. Where a label does not fit even then it wraps, onto at most [`SHOWN_ROWS`] rows, and
    /// every row is as tall as the tallest, so the rows stay one uniform height for the virtualised
    /// scroll.
    ///
    /// Whether a label wraps is not decided here by a comparison of widths: the row's height comes
    /// from laying the label out at the room its row leaves it, the call the painter makes, so
    /// the two agree (egui itself lets a line run 0.5 pt past its wrap width).
    fn measure_list(&self, ui: &Ui, trigger_w: f32, bounds_w: f32) -> (f32, f32) {
        let m = theme::metrics();
        let ink = theme::palette(ui.ctx()).text;
        // The widest face a label is shown in (the chosen row is medium weight).
        let font = FontId::new(m.body_size, theme::medium());
        // A row's width apart from its label and tag: the padding either side, the one before the
        // check, and the check.
        let chrome = 3.0 * ITEM_PAD + m.icon_sm;
        let tag_room = |i: usize| {
            self.tag_of(i)
                .map_or(0.0, |tag| tag.measure(ui).x + ITEM_PAD)
        };
        let widest = self
            .options
            .iter()
            .enumerate()
            .map(|(i, (_, label))| {
                let g = ui
                    .painter()
                    .layout_no_wrap(clipped(label).into_owned(), font.clone(), ink);
                g.size().x + tag_room(i) + chrome
            })
            .fold(0.0, f32::max)
            + 2.0 * m.menu_pad;
        let list_w = trigger_w.max(widest).min(bounds_w);
        let mut item_h = m.menu_item_height;
        if widest > list_w {
            let text_w = list_w - 2.0 * m.menu_pad - chrome;
            for (i, (_, label)) in self.options.iter().enumerate() {
                let room = (text_w - tag_room(i)).max(1.0);
                let wrapped = ui
                    .painter()
                    .layout_job(row_job(label, font.clone(), ink, room));
                // The padding a one-line row has above and below its text.
                let line = wrapped.rows.first().map_or(0.0, |r| r.height());
                let pad = ((m.menu_item_height - line) / 2.0).max(0.0);
                item_h = item_h.max(wrapped.size().y + 2.0 * pad);
            }
        }
        (list_w, item_h)
    }

    /// [`Self::measure_list`], remembered while the options, the metrics and the two widths stay
    /// as they were: a list of every branch is not laid out again on every frame it is open.
    fn list_size(&self, ui: &Ui, id: Id, trigger_w: f32, bounds_w: f32) -> (f32, f32) {
        let m = theme::metrics();
        // Every byte of every label counts (a list rebuilt each frame is new text at old
        // addresses), so the key is a word-at-a-time multiply-and-xor over them, cheap enough to
        // compute every frame; a collision would show a list one frame stale, never unsafe.
        let mut key = 0_u64;
        for (i, (value, label)) in self.options.iter().enumerate() {
            key = mix_text(key, label);
            key = mix_text(key, self.note_of(i).unwrap_or_default());
            key = mix(key, u64::from(self.recommended.as_ref() == Some(value)));
        }
        for width in [
            m.body_size,
            m.menu_item_height,
            m.menu_pad,
            m.icon_sm,
            ui.ctx().pixels_per_point(),
            trigger_w,
            bounds_w,
        ] {
            key = mix(key, u64::from(width.to_bits()));
        }
        let cache_id = id.with("kit-select-list-size");
        if let Some((k, size)) = ui.data(|d| d.get_temp::<(u64, (f32, f32))>(cache_id))
            && k == key
        {
            return size;
        }
        let size = self.measure_list(ui, trigger_w, bounds_w);
        ui.data_mut(|d| d.insert_temp(cache_id, (key, size)));
        size
    }

    fn show_inner(self, ui: &mut Ui, id: Id) -> Response {
        let ctx = ui.ctx().clone();
        let p = theme::palette(&ctx);
        let style = theme::style();
        let s = p.surfaces(style);
        let m = theme::metrics();
        let enabled = self.enabled && ui.is_enabled();
        let popup_id = id.with("kit-select-list");
        let state_id = id.with("kit-select-state");
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let chosen = self.options.iter().position(|(v, _)| v == self.value);

        // Labels and text paint at full strength even in a disabled `Ui` (in disabled tokens).
        let ink = super::content_painter(ui);
        if let Some(text) = &self.label {
            let color = if enabled { p.text_dim } else { p.disabled_text };
            let g = ink.layout_no_wrap(
                text.clone(),
                FontId::new(m.meta_size, theme::medium()),
                color,
            );
            let (rect, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            ink.galley(rect.min, g, color);
            ui.add_space(m.field_label_gap);
        }

        let sense = if enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        // What the trigger shows is laid out first: a value wider than the control wraps onto
        // more lines and the control grows to hold them.
        let (placeholder, text_color) = if enabled {
            (p.text_faint, p.text)
        } else {
            (p.disabled_text, p.disabled_text)
        };
        // A chosen option that is unavailable is dimmed, with its tag before the caret.
        let tag = chosen
            .and_then(|i| self.note_of(i))
            .map(|note| Badge::new(note).muted());
        let tag_room = tag.as_ref().map_or(0.0, |t| t.measure(ui).x + ITEM_PAD);
        let (text, color) = match chosen {
            Some(i) if tag.is_some() => (clipped(self.options[i].1).into_owned(), placeholder),
            Some(i) => (clipped(self.options[i].1).into_owned(), text_color),
            None => (self.placeholder.clone(), placeholder),
        };
        let mark = chosen
            .filter(|&i| self.marked_when_chosen(i))
            .map(|_| RECOMMENDED_SUFFIX);
        let max = (width - m.field_pad_x - caret_room(m) - tag_room).max(1.0);
        let font = FontId::new(m.body_size, FontFamily::Proportional);
        let mut job = egui::text::LayoutJob::simple_singleline(text, font.clone(), color);
        if let Some(mark) = mark {
            // The mark is a note beside the value, so it reads quieter than the value does.
            job.append(
                mark,
                0.0,
                egui::TextFormat::simple(font.clone(), placeholder),
            );
        }
        job.wrap = rows_wrapped_at(max);
        let value_galley = ink.layout_job(job);
        // The value the trigger cuts short (past the character cap or the rows), whole for its
        // tooltip.
        let cut_value = chosen
            .map(|i| self.options[i].1)
            .filter(|label| is_cut(label) || value_galley.elided);
        let one_line = ink.layout_no_wrap(String::from("Ag"), font, color).size().y;
        let pad_y = ((self.size.height() - one_line) / 2.0).max(0.0);
        let size = vec2(
            width,
            self.size.height().max(value_galley.size().y + 2.0 * pad_y),
        );
        let (rect, mut trigger) = if self.id.is_some() {
            let rect = ui.allocate_space(size).1;
            (rect, ui.interact(rect, trigger_id(id), sense))
        } else {
            ui.allocate_exact_size(size, sense)
        };
        let force = self.force;
        // A forced list shows only while its trigger is on screen (egui would push it into view).
        let forced_open = force.is_some_and(|f| f.open) && ui.is_rect_visible(rect);
        let can_open = enabled && !self.loading;

        // Opening, closing and the keys, before painting so this frame shows the result.
        let mut open = forced_open || Popup::is_id_open(&ctx, popup_id);
        let mut st: ListState = ctx.data(|d| d.get_temp(state_id)).unwrap_or_default();
        let now = ctx.input(|i| i.time);
        let pointer_click = trigger.clicked_by(PointerButton::Primary);
        let key_click = trigger.clicked() && !pointer_click;
        let mut set_open: Option<bool> = None;
        let mut picked: Option<usize> = None;
        if can_open && !forced_open {
            // Give focus on press, so egui can install the arrow-key filter on release before
            // the next input frame. Re-requesting focus on release resets that filter.
            if trigger.is_pointer_button_down_on() && !trigger.has_focus() {
                trigger.request_focus();
            }
            if trigger.has_focus() {
                ctx.data_mut(|d| d.insert_temp(typing_id(), (trigger.id, popup_id)));
            }
            if trigger.has_focus() {
                // Up and Down drive the list, and while it is open Esc closes it instead of
                // taking focus away from the trigger.
                ui.memory_mut(|mem| {
                    mem.set_focus_lock_filter(
                        trigger.id,
                        egui::EventFilter {
                            vertical_arrows: true,
                            escape: open,
                            ..Default::default()
                        },
                    );
                });
            }
            if pointer_click {
                if !trigger.has_focus() {
                    trigger.request_focus();
                }
                set_open = Some(!open);
                st = ListState {
                    highlight: None,
                    by_keyboard: false,
                    ..ListState::default()
                };
            } else if trigger.has_focus() {
                let (down, up, home, end) = ui.input_mut(|i| {
                    (
                        i.consume_key(Modifiers::NONE, Key::ArrowDown),
                        i.consume_key(Modifiers::NONE, Key::ArrowUp),
                        open && i.consume_key(Modifiers::NONE, Key::Home),
                        open && i.consume_key(Modifiers::NONE, Key::End),
                    )
                });
                if !open && (down || up || key_click) {
                    set_open = Some(true);
                    st = ListState {
                        highlight: chosen
                            .filter(|&i| self.note_of(i).is_none())
                            .or_else(|| self.available_from(None, true)),
                        by_keyboard: true,
                        opened_by_keyboard: true,
                        ..ListState::default()
                    };
                } else if open {
                    let at = st.highlight.or(chosen);
                    // Whatever opened the list, a key puts the highlight in charge of the scroll.
                    st.by_keyboard |= down || up || home || end;
                    if down {
                        st.highlight = self.available_from(at, true).or(at);
                    } else if up {
                        st.highlight = self.available_from(at, false).or(at);
                    } else if home {
                        st.highlight = self.available_from(None, true).or(st.highlight);
                    } else if end {
                        st.highlight = self.available_from(None, false).or(st.highlight);
                    } else if key_click {
                        picked = st
                            .highlight
                            .or(chosen)
                            .filter(|&i| self.note_of(i).is_none());
                        set_open = Some(false);
                    }
                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                        set_open = Some(false);
                        trigger.request_focus();
                    }
                    let typed: String = ui.input_mut(|i| {
                        let mut out = String::new();
                        i.events.retain(|e| match e {
                            Event::Text(t) => {
                                out.push_str(t);
                                false
                            }
                            _ => true,
                        });
                        out
                    });
                    if !typed.trim().is_empty() {
                        if now - st.typed_at > TYPE_AHEAD_RESET {
                            st.typed.clear();
                        }
                        st.typed.push_str(&typed.to_lowercase());
                        st.typed_at = now;
                        if let Some(i) = self.options.iter().enumerate().position(|(i, (_, l))| {
                            self.note_of(i).is_none() && l.to_lowercase().starts_with(&st.typed)
                        }) {
                            st.highlight = Some(i);
                            st.by_keyboard = true;
                        }
                    }
                }
            }
        }
        if let Some(o) = set_open {
            if o {
                Popup::open_id(&ctx, popup_id);
            } else {
                Popup::close_id(&ctx, popup_id);
            }
            open = o;
        }
        if !can_open && !forced_open && open {
            Popup::close_id(&ctx, popup_id);
            open = false;
        }

        // The trigger.
        let ist = Interact::of(&ctx, &trigger, enabled, force.filter(|f| !f.open));
        let open_t = match force {
            Some(f) if f.open => f.progress.unwrap_or(1.0),
            Some(_) => 0.0,
            None => anim::ease(&ctx, id.with("kit-select-caret"), open, CARET_TURN),
        };
        let error = self.error.as_deref().filter(|e| !e.is_empty());
        if ui.is_rect_visible(rect) {
            // The trigger is a raised control, like a secondary button beside it; the up-down
            // caret says it opens a list.
            let painter = if enabled {
                ui.painter().clone()
            } else {
                super::content_painter(ui)
            };
            let radius = CornerRadius::same(m.radius);
            let ring_t = ist.focus;
            if enabled {
                let fill = super::button::blend(
                    super::button::blend(s.raised, s.raised_hover, ist.hover.max(open_t)),
                    s.raised_pressed,
                    ist.press,
                );
                let ring = if error.is_some() {
                    p.danger_base
                } else {
                    s.raised_edge
                };
                paint::surface(
                    &painter,
                    rect,
                    radius,
                    p,
                    style,
                    Elevation::Control,
                    fill,
                    Some(ring),
                );
                let focus = if error.is_some() {
                    p.danger_base
                } else {
                    p.focus
                };
                if ring_t > 0.0 {
                    paint::focus_ring(
                        &painter,
                        rect,
                        radius,
                        focus.gamma_multiply(ring_t),
                        Color32::TRANSPARENT,
                    );
                }
            } else {
                paint::fill(&painter, rect, radius, Fill::flat(s.disabled_fill));
                paint::rim(&painter, rect, radius, s.disabled_edge);
            }
            // The caret brightens while the pointer is on the trigger or the list is open.
            let faint = if enabled {
                p.text_faint
                    .lerp_to_gamma(p.text_dim, ist.hover.max(open_t))
            } else {
                p.disabled_text
            };
            let caret_x = rect.right() - theme::SM - m.icon_md / 2.0;
            if self.loading {
                paint::spinner(
                    &painter,
                    pos2(caret_x, rect.center().y),
                    m.icon_md * 0.33,
                    paint::SPINNER_WIDTH,
                    faint,
                );
            } else {
                let g = painter.layout_no_wrap(
                    icon::CARET_UP_DOWN.to_owned(),
                    FontId::proportional(m.icon_sm),
                    faint,
                );
                painter.galley(pos2(caret_x, rect.center().y) - g.size() / 2.0, g, faint);
            }
            if let Some(tag) = &tag {
                let size = tag.measure(ui);
                let right = rect.right() - caret_room(m);
                tag.paint(
                    ui,
                    egui::Rect::from_center_size(pos2(right - size.x / 2.0, rect.center().y), size),
                );
            }
            painter.galley(
                pos2(
                    rect.left() + m.field_pad_x,
                    rect.center().y - value_galley.size().y / 2.0,
                ),
                value_galley,
                color,
            );
        }

        // The list.
        if open {
            if forced_open && force.is_some_and(|f| f.hover) {
                st.highlight = (0..self.options.len()).find(|i| Some(*i) != chosen);
            }
            // A pointer-opened list grows in from the trigger and fades out when it closes (spec
            // 4.12 motion); a keyboard-opened one comes and goes at once, as does every list with
            // reduced motion.
            let keyboard = st.opened_by_keyboard;
            let progress = forced_open.then(|| force.and_then(|f| f.progress).unwrap_or(1.0));
            let anchor = rect;
            let bounds = overlay::popup_bounds(ui);
            let rows = crate::count_f32(self.options.len());
            let (list_w, item_h) = self.list_size(ui, popup_id, rect.width(), bounds.width());
            let pin = self.pin_last && self.options.len() > 1;
            let pin_h = if pin {
                item_h + super::menu::SEPARATOR_HEIGHT
            } else {
                0.0
            };
            // The pinned row has its own place below the scrolling ones.
            let visible = (rows - if pin { 1.0 } else { 0.0 }).min(LIST_ROWS);
            let desired_h = visible * item_h + pin_h + 2.0 * m.menu_pad;
            let below = (bounds.bottom() - anchor.bottom() - LIST_GAP).max(0.0);
            let above = (anchor.top() - LIST_GAP - bounds.top()).max(0.0);
            let opens_below = desired_h <= below || below >= above;
            let room = if opens_below { below } else { above };
            // A tiny host can leave no row-sized space on either side; keep the list inside it
            // even then, overlapping the trigger rather than making options unreachable.
            let list_h = desired_h.min(if room >= item_h + 2.0 * m.menu_pad {
                room
            } else {
                bounds.height()
            });
            // Lined up with the trigger's left edge; with its right edge when the list is wider
            // than the trigger and would not fit to the right; always inside the host.
            let left = if anchor.left() + list_w <= bounds.right() {
                anchor.left()
            } else {
                anchor.right() - list_w
            };
            let x = left.clamp(bounds.left(), (bounds.right() - list_w).max(bounds.left()));
            let y = if opens_below {
                anchor.bottom() + LIST_GAP
            } else {
                anchor.top() - LIST_GAP - list_h
            }
            .clamp(bounds.top(), (bounds.bottom() - list_h).max(bounds.top()));
            let popup = Popup::new(popup_id, ctx.clone(), anchor, ui.layer_id())
                .kind(egui::PopupKind::Popup)
                .at_position(pos2(x, y))
                .align(RectAlign::BOTTOM_START)
                .align_alternatives(&[])
                .gap(0.0)
                .width(list_w)
                .frame(egui::Frame::NONE)
                .close_behavior(PopupCloseBehavior::CloseOnClickOutside);
            // Egui's popup memory keeps it open and closes it on Esc or a click outside.
            let popup = if forced_open {
                popup.open(true)
            } else {
                popup.open_memory(None)
            };
            let options = &self.options;
            let list_name = self
                .label
                .clone()
                .or_else(|| self.name.clone())
                .unwrap_or_default();
            let shown = popup.show(|ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(bounds));
                let layer = ui.layer_id();
                let e = Entrance::of(
                    ui.ctx(),
                    popup_id,
                    motion::BASE,
                    motion::SLOW,
                    keyboard,
                    progress,
                );
                ui.multiply_opacity(e.fade);
                let start = overlay::next_shape(ui.ctx(), layer);
                let (list, _) = ui.allocate_exact_size(vec2(list_w, list_h), Sense::hover());
                let radius = CornerRadius::same(m.radius_lg);
                let painter = ui.painter();
                let fill = glass::layer_fill(painter, list, radius);
                paint::surface(
                    painter,
                    list,
                    radius,
                    p,
                    style,
                    Elevation::Floating,
                    Fill::flat(fill),
                    Some(p.floating_ring(s)),
                );
                let inner = list.shrink(m.menu_pad);
                let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
                // The options are the list box's parts.
                super::a11y::group(&child, Role::ListBox, &list_name);
                child.spacing_mut().item_spacing.y = 0.0;
                let mut chose = None;
                // The pinned row needs a scrolling row above it; a host too short for both
                // scrolls them all.
                let pinned = (pin && inner.height() >= item_h + pin_h).then(|| options.len() - 1);
                let scrolled = pinned.unwrap_or(options.len());
                let scroll_id = popup_id.with("scroll");
                let mut area = egui::ScrollArea::vertical()
                    .id_salt(scroll_id)
                    .max_height(inner.height() - if pinned.is_some() { pin_h } else { 0.0 })
                    .auto_shrink([false, true]);
                if pinned.is_some() {
                    // Egui keeps a scrolling area at least 64 points tall; with a pinned row
                    // below it, one row is enough, so the pinned row stays inside the list.
                    area = area.min_scrolled_height(item_h);
                }
                // Only the rows in view are built (a list of every branch can be long), so the
                // row the keyboard moved to is brought into view by the offset, not by the row.
                if let Some(i) = st.highlight.filter(|i| st.by_keyboard && *i < scrolled) {
                    // The scroll area hashes its salt once more to name its state.
                    let offset = egui::scroll_area::State::load(
                        ui.ctx(),
                        child.make_persistent_id(egui::IdSalt::new(scroll_id)),
                    )
                    .map_or(0.0, |state| state.offset.y);
                    let top = crate::count_f32(i) * item_h;
                    let view = inner.height() - if pinned.is_some() { pin_h } else { 0.0 };
                    let to = if top < offset {
                        top
                    } else if top + item_h > offset + view {
                        top + item_h - view
                    } else {
                        offset
                    };
                    if (to - offset).abs() > 0.5 {
                        area = area.vertical_scroll_offset(to);
                    }
                }
                let mut row_at = |ui: &mut Ui, i: usize| {
                    let (value, label) = &options[i];
                    {
                        let note = self.note_of(i);
                        let sense = if note.is_some() {
                            Sense::hover()
                        } else {
                            Sense::click()
                        };
                        let (row, r) = ui.allocate_exact_size(vec2(inner.width(), item_h), sense);
                        if note.is_none() && r.hovered() && ui.input(|i| i.pointer.is_moving()) {
                            st.highlight = Some(i);
                            st.by_keyboard = false;
                        }
                        let on = chosen == Some(i);
                        let radius = CornerRadius::same(m.radius_lg.saturating_sub(5).max(3));
                        if note.is_none() && st.highlight == Some(i) {
                            ui.painter().rect_filled(row, radius, p.hover_strong());
                        }
                        let font = if on {
                            FontId::new(m.body_size, theme::medium())
                        } else {
                            FontId::new(m.body_size, FontFamily::Proportional)
                        };
                        let recommended =
                            note.is_none() && self.recommended.as_ref() == Some(value);
                        let check_left = row.right() - ITEM_PAD - m.icon_sm;
                        let text_right = if let Some(tag) = self.tag_of(i) {
                            let size = tag.measure(ui);
                            let tag_rect = egui::Rect::from_center_size(
                                pos2(check_left - ITEM_PAD - size.x / 2.0, row.center().y),
                                size,
                            );
                            tag.paint(ui, tag_rect);
                            tag_rect.left() - ITEM_PAD
                        } else {
                            check_left - ITEM_PAD
                        };
                        let ink = if note.is_some() {
                            p.disabled_text
                        } else {
                            p.text
                        };
                        let galley = ui.painter().layout_job(row_job(
                            label,
                            font,
                            ink,
                            (text_right - row.left() - ITEM_PAD).max(1.0),
                        ));
                        let cut = is_cut(label) || galley.elided;
                        ui.painter().galley(
                            pos2(
                                row.left() + ITEM_PAD,
                                row.center().y - galley.size().y / 2.0,
                            ),
                            galley,
                            ink,
                        );
                        if on {
                            ui.painter().text(
                                pos2(row.right() - ITEM_PAD, row.center().y),
                                Align2::RIGHT_CENTER,
                                icon::CHECK,
                                FontId::proportional(m.icon_sm),
                                p.accent,
                            );
                        }
                        let r = if cut {
                            // Past its rows or the character cap, the row says all it can.
                            r.kit_tooltip(tooltip_text(label)).at_pointer().show()
                        } else {
                            r
                        };
                        if r.clicked() {
                            chose = Some(i);
                        }
                        r.widget_info(|| {
                            WidgetInfo::selected(WidgetType::SelectableLabel, true, on, *label)
                        });
                        ui.ctx().accesskit_node_builder(r.id, |n| {
                            n.set_role(Role::ListBoxOption);
                            n.clear_toggled();
                            n.set_selected(on);
                            if let Some(note) = note {
                                n.set_disabled();
                                n.set_description(note);
                            } else if recommended {
                                n.set_description("Recommended");
                            } else if let Some((_, note)) =
                                self.described.iter().find(|(v, _)| v == value)
                            {
                                n.set_description(*note);
                            }
                        });
                    }
                };
                let out = area.show_rows(&mut child, item_h, scrolled, |ui, range| {
                    for i in range {
                        row_at(ui, i);
                    }
                });
                if let Some(i) = pinned {
                    super::menu::separator(&mut child);
                    row_at(&mut child, i);
                }
                paint::scroll_fade(ui, &out, fill);
                let (origin, drop) = overlay::popup_origin(list, anchor);
                overlay::transform_since(
                    ui.ctx(),
                    layer,
                    start,
                    e.transform(origin, overlay::POPUP_SCALE, drop),
                );
                if !keyboard && !forced_open {
                    let end = overlay::next_shape(ui.ctx(), layer);
                    overlay::record_exit(
                        ui.ctx(),
                        popup_id,
                        layer,
                        0..0,
                        start.0..end.0,
                        ExitMotion::popup(origin),
                    );
                }
                chose
            });
            // A press that lands in the frame the list first opens leaves a modal over it (egui
            // ranks both on top and keeps the order they had), so raise the list until it clears
            // any modal.
            if let Some(shown) = &shown {
                let layer = shown.response.layer_id;
                if !ctx.memory(|m| m.is_above_modal_layer(layer)) {
                    ctx.move_to_top(layer);
                    ctx.request_repaint();
                }
            }
            if let Some(inner) = shown
                && let Some(i) = inner.inner
            {
                picked = Some(i);
                Popup::close_id(&ctx, popup_id);
            }
        }
        if let Some(i) = picked
            && (chosen != Some(i) || self.report_reselection)
        {
            *self.value = self.options[i].0.clone();
            trigger.mark_changed();
        }
        if !forced_open {
            ctx.data_mut(|d| d.insert_temp(state_id, st));
        }

        let value = self
            .options
            .iter()
            .position(|(v, _)| v == self.value)
            .map_or_else(|| self.placeholder.clone(), |i| self.trigger_text(i));
        let name = self
            .label
            .clone()
            .or_else(|| self.name.clone())
            .unwrap_or_default();
        let expanded = open;
        trigger.widget_info(|| {
            let mut info = WidgetInfo::labeled(WidgetType::ComboBox, enabled, &name);
            info.current_text_value = Some(value.clone());
            info
        });
        // The current option's note (an unavailable choice's "Not installed") follows the helper
        // or the error, as the list's options carry theirs; read after a pick, not before it.
        let note = self
            .options
            .iter()
            .position(|(v, _)| v == self.value)
            .and_then(|i| self.note_of(i));
        let description = match (
            error.map(str::to_owned).or_else(|| self.helper.clone()),
            note,
        ) {
            (Some(d), Some(note)) => Some(format!("{d} {note}")),
            (d, note) => d.or_else(|| note.map(str::to_owned)),
        };
        let has_error = error.is_some();
        ctx.accesskit_node_builder(trigger.id, |n| {
            n.set_expanded(expanded);
            n.set_has_popup(egui::accesskit::HasPopup::Listbox);
            if let Some(d) = &description {
                n.set_description(d.clone());
            }
            if has_error {
                n.set_invalid(Invalid::True);
            }
        });

        // Helper or error below, as a text field shows them.
        let message = error
            .map(|e| (e, true))
            .or(self.helper.as_deref().map(|h| (h, false)));
        if let Some((text, is_error)) = message {
            ui.add_space(m.field_label_gap);
            let color = if !enabled {
                p.disabled_text
            } else if is_error {
                p.danger
            } else {
                p.text_dim
            };
            let icon_g = is_error.then(|| {
                ink.layout_no_wrap(
                    icon::WARNING_CIRCLE.to_owned(),
                    FontId::proportional(m.icon_sm),
                    color,
                )
            });
            let indent = icon_g
                .as_ref()
                .map_or(0.0, |g| g.size().x + m.field_label_gap);
            let g = ink.layout(
                text.to_owned(),
                FontId::new(m.meta_size, FontFamily::Proportional),
                color,
                (width - indent).max(1.0),
            );
            let (row, _) = ui.allocate_exact_size(vec2(width, g.size().y), Sense::hover());
            if let Some(ig) = icon_g {
                let first = g.rows.first().map_or(g.size().y, |r| r.rect().height());
                ink.galley(
                    pos2(row.left(), row.top() + (first - ig.size().y) / 2.0),
                    ig,
                    color,
                );
            }
            ink.galley(pos2(row.left() + indent, row.top()), g, color);
        }

        // A value cut short says itself on hover (unless the control says why it is unavailable),
        // in place of the caller's own tooltip, and as text the screen does not show, not as an
        // extra description.
        let cut_tip = cut_value.map(tooltip_text);
        super::button::tip(
            trigger,
            enabled,
            cut_tip.as_deref().or(self.tooltip.as_deref()),
            self.disabled_reason.as_deref(),
            None,
            cut_value.is_none(),
        )
    }
}

impl<T: PartialEq + Clone> Widget for Select<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

/// `text` and its length folded into `hash`, a word at a time.
fn mix_text(mut hash: u64, text: &str) -> u64 {
    for chunk in text.as_bytes().chunks(8) {
        let mut word = [0_u8; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        hash = mix(hash, u64::from_le_bytes(word));
    }
    mix(hash, u64::try_from(text.len()).unwrap_or(u64::MAX))
}

/// One step of the cheap hash that keys the remembered list size.
fn mix(hash: u64, word: u64) -> u64 {
    (hash.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95)
}

/// Where the select that last had keyboard focus keeps its trigger and list.
fn typing_id() -> Id {
    Id::new("kit-select-typing")
}

/// Whether a focused select has its list open and so takes the letters typed (type-ahead). The
/// app's bare keys leave them alone, as for a text field.
pub fn takes_text(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<(Id, Id)>(typing_id()))
        .is_some_and(|(trigger, list)| {
            ctx.memory(|m| m.has_focus(trigger)) && Popup::is_id_open(ctx, list)
        })
}

/// The trigger of a select whose id was set with `.id(id)`: what `Memory::request_focus` takes.
pub fn trigger_id(id: Id) -> Id {
    id.with("kit-select-trigger")
}

/// What the trigger keeps right of its text: a gap, the caret and the inset after it.
fn caret_room(m: &theme::Metrics) -> f32 {
    theme::SM + m.icon_md + theme::SM
}

/// Whether `text` is longer than [`LABEL_MAX_CHARS`]. At most one character more than that is
/// visited, however long the text is.
fn is_cut(text: &str) -> bool {
    text.chars().nth(LABEL_MAX_CHARS).is_some()
}

/// `text` as a label is laid out: whole up to [`LABEL_MAX_CHARS`] characters, else its first ones
/// and an ellipsis (borrowed when nothing is cut). The cut falls between characters.
fn clipped(text: &str) -> Cow<'_, str> {
    if !is_cut(text) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.chars()
            .take(LABEL_MAX_CHARS)
            .chain(std::iter::once('\u{2026}'))
            .collect(),
    )
}

/// How the closed control and a list row wrap a label: at `width`, onto at most [`SHOWN_ROWS`]
/// rows, with an ellipsis after the last when the text goes on.
fn rows_wrapped_at(width: f32) -> egui::text::TextWrapping {
    egui::text::TextWrapping {
        max_rows: SHOWN_ROWS,
        ..egui::text::TextWrapping::wrap_at_width(width)
    }
}

/// A list row's label as the row paints it, and as the list measures it: [`clipped`] and wrapped by
/// [`rows_wrapped_at`] in the `room` its row leaves it.
fn row_job(label: &str, font: FontId, color: Color32, room: f32) -> egui::text::LayoutJob {
    let mut job =
        egui::text::LayoutJob::simple_singleline(clipped(label).into_owned(), font, color);
    job.wrap = rows_wrapped_at(room);
    job
}

/// What a tooltip says of `label`: all of it up to [`TOOLTIP_MAX_CHARS`] characters, else its
/// first ones, an ellipsis and how many characters there are.
fn tooltip_text(label: &str) -> Cow<'_, str> {
    if label.chars().nth(TOOLTIP_MAX_CHARS).is_none() {
        return Cow::Borrowed(label);
    }
    let head: String = label.chars().take(TOOLTIP_MAX_CHARS).collect();
    Cow::Owned(format!(
        "{head}\u{2026} {} characters",
        grouped(label.chars().count())
    ))
}

/// `n` with a comma between each three digits.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The trigger width that shows each of `labels` on one line (the first 4096 characters of each and
/// an ellipsis after them; the trigger cuts a longer one): the widest one plus the trigger's
/// padding and caret. For a row that sizes a select before showing it;
/// [`Select::fit_to_options`] does this for the select's own options.
pub fn fit_width<'l>(ui: &Ui, labels: impl IntoIterator<Item = &'l str>) -> f32 {
    let labels: Vec<Cow<'_, str>> = labels.into_iter().map(clipped).collect();
    fit_width_of(ui, labels.iter().map(AsRef::as_ref))
}

/// [`fit_width`] for texts the trigger paints as they are (already cut where they were long).
fn fit_width_of<'l>(ui: &Ui, labels: impl IntoIterator<Item = &'l str>) -> f32 {
    let m = theme::metrics();
    let font = FontId::new(m.body_size, FontFamily::Proportional);
    let widest = labels
        .into_iter()
        .map(|l| {
            ui.painter()
                .layout_no_wrap(l.to_owned(), font.clone(), Color32::PLACEHOLDER)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    (m.field_pad_x + widest + caret_room(m)).ceil()
}

// ------------------------------------------------------------------------------------- gallery

const PROGRAMS: [(u8, &str); 5] = [
    (0, "kitty (Default)"),
    (1, "Foot"),
    (2, "Alacritty"),
    (3, "Konsole"),
    (4, "Custom command…"),
];

const STRATEGIES: [(u8, &str); 3] = [
    (0, "Merge (fast-forward if possible)"),
    (1, "Rebase"),
    (2, "Fast-forward only"),
];

const LONG_LABELS: [(u8, &str); 3] = [
    (0, "Merge"),
    (1, "Don't rebase"),
    (
        2,
        "Rebase the local commits onto the remote branch and keep every merge commit",
    ),
];

/// The gallery's selects: live with a helper and with an error, one open, then hover, focus,
/// disabled, loading, placeholder and small.
pub(crate) fn gallery(ui: &mut Ui) {
    gallery_caption(
        ui,
        "Live (pointer or keyboard) with a label and a helper; an error; beside a text field",
    );
    // Top-aligned, as a form lines up labelled controls of different heights.
    ui.with_layout(
        Layout::left_to_right(Align::Min).with_main_wrap(true),
        |ui| {
            ui.spacing_mut().item_spacing = vec2(theme::LG, theme::SM);
            remembered(ui, "select-live", 1_u8, |ui, v| {
                Select::new(v, &STRATEGIES)
                    .label("Pull strategy")
                    .helper("Canopy passes it to git on every pull.")
                    .width(280.0)
                    .tooltip("How Pull combines your work with the remote's")
                    .id_salt("g-sel0")
                    .show(ui);
            });
            Select::new(&mut 9_u8, &STRATEGIES)
                .label("Required")
                .placeholder("Choose a strategy…")
                .error(Some("Choose how to combine the branches."))
                .width(240.0)
                .id_salt("g-sel6")
                .show(ui);
            field::TextField::new(&mut "origin".to_owned())
                .label("Text field, for comparison")
                .width(200.0)
                .id_salt("g-sel-field")
                .show(ui);
        },
    );
    ui.add_space(theme::SM);
    gallery_caption(ui, "Open, with the pointer on another option");
    Select::new(&mut 1_u8, &STRATEGIES)
        .width(280.0)
        .id_salt("g-sel7")
        .show_forced(
            ui,
            ForceState {
                open: true,
                hover: true,
                ..ForceState::default()
            },
        );
    // Room for the open list.
    ui.add_space(
        3.0 * theme::metrics().menu_item_height + 2.0 * theme::metrics().menu_pad + theme::MD,
    );
    gallery_caption(
        ui,
        "Unavailable options, open: dimmed with a tag, passed over by the keys, never chosen; \
         the last pinned below a separator",
    );
    Select::new(&mut 0_u8, &PROGRAMS)
        .unavailable(2, "Not installed")
        .unavailable(3, "Not installed")
        .width(280.0)
        .pin_last()
        .id_salt("g-sel8")
        .show_forced(
            ui,
            ForceState {
                open: true,
                ..ForceState::default()
            },
        );
    ui.add_space(
        5.0 * theme::metrics().menu_item_height
            + super::menu::SEPARATOR_HEIGHT
            + 2.0 * theme::metrics().menu_pad
            + theme::MD,
    );
    gallery_caption(
        ui,
        "A long label is never cut: the control widens to it, then wraps; the list wraps the same way",
    );
    gallery_row(ui, |ui| {
        Select::new(&mut 2_u8, &LONG_LABELS)
            .width(160.0)
            .id_salt("g-sel10")
            .show(ui);
    });
    ui.add_space(theme::SM);
    gallery_caption(
        ui,
        "A narrow control, open, with the recommended entry whole",
    );
    let mut chosen = 0_u8;
    let narrow = Select::new(&mut chosen, &LONG_LABELS)
        .width(120.0)
        .recommended(1)
        .id_salt("g-sel11");
    // Room for the list as it is: wrapped rows are taller than a one-line row.
    let (_, row_h) = narrow.measure_list(ui, 120.0, overlay::popup_bounds(ui).width());
    narrow.show_forced(
        ui,
        ForceState {
            open: true,
            ..ForceState::default()
        },
    );
    ui.add_space(
        crate::count_f32(LONG_LABELS.len()) * row_h + 2.0 * theme::metrics().menu_pad + theme::MD,
    );
    gallery_caption(ui, "A saved choice that has gone, closed");
    gallery_row(ui, |ui| {
        Select::new(&mut 2_u8, &PROGRAMS)
            .unavailable(2, "Not installed")
            .unavailable(3, "Not installed")
            .width(280.0)
            .id_salt("g-sel9")
            .show(ui);
    });
    ui.add_space(theme::SM);
    let w = 160.0;
    gallery_caption(ui, "Hover, focus, disabled");
    gallery_row(ui, |ui| {
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .id_salt("g-sel1")
            .show_forced(ui, ForceState::HOVER);
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .id_salt("g-sel2")
            .show_forced(ui, ForceState::FOCUS);
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .enabled(false)
            .disabled_reason("Set by the repository's configuration")
            .id_salt("g-sel3")
            .show(ui);
    });
    ui.add_space(theme::SM);
    gallery_caption(
        ui,
        "Loading, a placeholder (as wide as its longest option), small",
    );
    gallery_row(ui, |ui| {
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .loading(true)
            .id_salt("g-sel4")
            .show(ui);
        Select::new(&mut 9_u8, &STRATEGIES)
            .fit_to_options()
            .placeholder("Choose a strategy…")
            .name("Strategy")
            .id_salt("g-sel5")
            .show(ui);
        Select::new(&mut 1_u8, &STRATEGIES)
            .width(w)
            .size(Size::Sm)
            .id(Id::new("gallery-select-small"))
            .show(ui);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_recommended_value_is_listed_first() {
        let options = [(0, "Off"), (1, "Daily"), (2, "Weekly")];
        let mut value = 0;
        let order = |select: Select<'_, i32>| -> Vec<i32> {
            select.options.iter().map(|(v, _)| *v).collect()
        };
        assert_eq!(
            order(Select::new(&mut value, &options).recommended(2)),
            [2, 0, 1]
        );
        assert_eq!(
            order(Select::new(&mut value, &options).recommended(0)),
            [0, 1, 2]
        );
        assert_eq!(order(Select::new(&mut value, &options)), [0, 1, 2]);
    }

    use super::super::testkit::{self, frame, key, spoken};
    use super::*;

    #[test]
    fn a_select_with_a_caller_set_id_can_be_focused_from_elsewhere() {
        let ctx = testkit::ctx();
        let id = Id::new("pull-strategy");
        let mut v = 0_u8;
        let mut focused = false;
        for i in 0..3 {
            if i == 1 {
                ctx.memory_mut(|m| m.request_focus(trigger_id(id)));
            }
            frame(&ctx, Vec::new(), |ui| {
                ui.scope(|ui| {
                    focused = Select::new(&mut v, &STRATEGIES).id(id).show(ui).has_focus();
                });
            });
        }
        assert!(focused);
    }

    #[test]
    fn a_select_fitted_to_its_options_shows_the_longest_whole_and_no_more() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut fit = 0.0;
        let mut trigger = egui::Rect::NOTHING;
        let shapes = frame(&ctx, Vec::new(), |ui| {
            ui.set_width(600.0);
            fit = fit_width(ui, STRATEGIES.iter().map(|(_, l)| *l));
            trigger = Select::new(&mut v, &STRATEGIES)
                .fit_to_options()
                .id_salt("fit")
                .show(ui)
                .rect;
        });
        assert!(
            (trigger.width() - fit).abs() < 0.5,
            "{} vs {fit}",
            trigger.width()
        );
        assert!(fit < 600.0, "no wider than it needs");
        let longest = STRATEGIES[0].1;
        assert!(
            shapes.iter().any(|c| match &c.shape {
                egui::Shape::Text(t) => t.galley.text() == longest,
                _ => false,
            }),
            "the longest option is not cut short"
        );
    }

    fn show(ui: &mut Ui, v: &mut u8) -> Response {
        Select::new(v, &STRATEGIES)
            .name("Pull strategy")
            .id_salt("s")
            .show(ui)
    }

    fn open_id(ctx: &egui::Context) -> bool {
        Popup::is_any_open(ctx)
    }

    /// A11Y-4: an open list is a list box named like its select, holding its options.
    #[test]
    fn an_open_list_is_a_named_list_box_of_its_options() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut v);
        });
        assert!(
            nodes
                .iter()
                .any(|n| n.role == Role::ListBox && n.label.as_deref() == Some("Pull strategy")),
            "the list is named for its select"
        );
        let parts = testkit::parts(&ctx, "Pull strategy", |ui| {
            show(ui, &mut v);
        });
        let names: Vec<&str> = STRATEGIES.iter().map(|(_, l)| *l).collect();
        assert_eq!(parts, names, "the options are the list's parts");
    }

    /// An option can carry an accessible description its label already shows in words.
    #[test]
    fn a_described_option_is_spoken_with_its_description() {
        let ctx = testkit::ctx();
        let options = [(0_u8, "kitty (Default)"), (1, "Foot")];
        let mut v = 0_u8;
        let mut draw = |ui: &mut Ui| {
            Select::new(&mut v, &options)
                .described(0, "Default")
                .name("Terminal")
                .id_salt("d")
                .show(ui)
        };
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = draw(ui).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = draw(ui).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = draw(ui).id);
        let nodes = spoken(&ctx, |ui| {
            draw(ui);
        });
        let mut described: Vec<_> = nodes
            .iter()
            .filter(|n| n.role == Role::ListBoxOption)
            .map(|n| (n.label.as_deref(), n.description.as_deref()))
            .collect();
        described.sort_unstable();
        assert_eq!(
            described,
            [
                (Some("Foot"), None),
                (Some("kitty (Default)"), Some("Default"))
            ]
        );
    }

    const PICKS: [(u8, &str); 4] = [(0, "Alpha"), (1, "Beta"), (2, "Gamma"), (3, "Delta")];

    /// Beta and Gamma are listed but cannot be chosen.
    fn show_picks(ui: &mut Ui, v: &mut u8) -> Response {
        Select::new(v, &PICKS)
            .unavailable(1, "Not installed")
            .unavailable(2, "Not installed")
            .name("Pick")
            .id_salt("u")
            .show(ui)
    }

    /// The arrow keys, Home, End and typing pass over unavailable options, and Enter chooses only
    /// an available one.
    #[test]
    fn the_keyboard_passes_over_unavailable_options() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| {
            id = show_picks(ui, &mut v).id;
        });
        assert!(open_id(&ctx));
        // Down from Alpha lands on Delta, past Beta and Gamma.
        for k in [
            Key::ArrowDown,
            Key::ArrowUp,
            Key::End,
            Key::Home,
            Key::ArrowDown,
        ] {
            frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
            frame(&ctx, key(k), |ui| id = show_picks(ui, &mut v).id);
        }
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        frame(&ctx, key(Key::Enter), |ui| id = show_picks(ui, &mut v).id);
        assert_eq!(v, 3, "Delta, never Beta or Gamma");
        // Up opens the list at Delta and, past Gamma and Beta, goes to Alpha. Typing "g" would match
        // Gamma, which is unavailable: the highlight stays on Alpha.
        for k in [key(Key::ArrowUp), key(Key::ArrowUp)] {
            frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
            frame(&ctx, k, |ui| id = show_picks(ui, &mut v).id);
        }
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        frame(&ctx, vec![Event::Text("g".to_owned())], |ui| {
            id = show_picks(ui, &mut v).id;
        });
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        frame(&ctx, key(Key::Enter), |ui| id = show_picks(ui, &mut v).id);
        assert_eq!(
            v, 0,
            "Alpha: the arrows passed over Gamma and Beta, typing did not reach Gamma"
        );
    }

    #[test]
    fn a_click_on_an_unavailable_option_chooses_nothing() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut rect = egui::Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| rect = show_picks(ui, &mut v).rect);
        testkit::click(&ctx, rect.center(), |ui| {
            show_picks(ui, &mut v);
        });
        for _ in 0..2 {
            frame(&ctx, Vec::new(), |ui| {
                show_picks(ui, &mut v);
            });
        }
        let m = theme::metrics();
        let row = |index: f32| {
            pos2(
                rect.center().x,
                rect.bottom() + LIST_GAP + m.menu_pad + (index + 0.5) * m.menu_item_height,
            )
        };
        testkit::click(&ctx, row(1.0), |ui| {
            show_picks(ui, &mut v);
        });
        assert_eq!(v, 0, "Beta is not chosen");
        assert!(open_id(&ctx), "and the list stays open");
        testkit::click(&ctx, row(3.0), |ui| {
            show_picks(ui, &mut v);
        });
        assert_eq!(v, 3, "Delta is");
    }

    /// An unavailable option is spoken as disabled, with its tag as the description, and the
    /// closed control of a select whose current value is one shows the tag.
    #[test]
    fn unavailable_options_are_spoken_and_shown_as_such() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| {
            id = show_picks(ui, &mut v).id;
        });
        let nodes = spoken(&ctx, |ui| {
            show_picks(ui, &mut v);
        });
        let option = |label: &str| {
            nodes
                .iter()
                .find(|n| n.role == Role::ListBoxOption && n.label.as_deref() == Some(label))
                .unwrap_or_else(|| panic!("no option {label}"))
        };
        assert!(option("Beta").disabled);
        assert_eq!(option("Beta").description.as_deref(), Some("Not installed"));
        assert!(!option("Alpha").disabled);
        assert_eq!(option("Alpha").description, None);

        let ctx = testkit::ctx();
        let mut v = 1_u8;
        let shapes = frame(&ctx, Vec::new(), |ui| {
            show_picks(ui, &mut v);
        });
        assert!(
            painted(&shapes, "Not installed") && painted(&shapes, "Beta"),
            "the closed control shows the unavailable choice with its tag"
        );
        let nodes = spoken(&ctx, |ui| {
            show_picks(ui, &mut v);
        });
        let combo = nodes
            .iter()
            .find(|n| n.role == Role::ComboBox)
            .expect("the closed control is a combo box");
        assert_eq!(combo.description.as_deref(), Some("Not installed"));
        let mut v = 0_u8;
        let nodes = spoken(&ctx, |ui| {
            show_picks(ui, &mut v);
        });
        let combo = nodes.iter().find(|n| n.role == Role::ComboBox).unwrap();
        assert_eq!(combo.description, None, "an available choice says nothing");
    }

    /// The "Not installed" tag, in the closed control and in the open list, is secondary text: it
    /// sits behind the name beside it.
    #[test]
    fn the_unavailable_tag_is_painted_in_secondary_text() {
        fn tag_ink(shapes: &[egui::epaint::ClippedShape]) -> Vec<Color32> {
            fn walk(shape: &egui::Shape, out: &mut Vec<Color32>) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    egui::Shape::Text(t) if t.galley.text() == "Not installed" => {
                        out.push(t.fallback_color);
                    }
                    _ => {}
                }
            }
            let mut out = Vec::new();
            for c in shapes {
                walk(&c.shape, &mut out);
            }
            out
        }
        let ctx = testkit::ctx();
        let dim = theme::palette(&ctx).text_dim;
        let mut v = 1_u8;
        let closed = frame(&ctx, Vec::new(), |ui| {
            show_picks(ui, &mut v);
        });
        assert_eq!(tag_ink(&closed), [dim], "the closed control's tag");
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| {
            show_picks(ui, &mut v);
        });
        let open = frame(&ctx, Vec::new(), |ui| {
            show_picks(ui, &mut v);
        });
        assert_eq!(tag_ink(&open), [dim, dim], "both tags in the open list");
    }

    /// The frame that picks an available option over an unavailable current one no longer says
    /// "Not installed" of the closed control.
    #[test]
    fn the_description_follows_the_pick_in_the_same_frame() {
        let ctx = testkit::ctx();
        let mut v = 1_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show_picks(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| {
            show_picks(ui, &mut v);
        });
        frame(&ctx, Vec::new(), |ui| {
            show_picks(ui, &mut v);
        });
        let nodes = testkit::spoken_with(&ctx, key(Key::Enter), |ui| {
            show_picks(ui, &mut v);
        });
        assert_ne!(v, 1, "an available option was picked");
        let combo = nodes
            .iter()
            .find(|n| n.role == Role::ComboBox)
            .expect("the closed control is a combo box");
        assert_eq!(combo.description, None, "{combo:?}");
    }

    /// #225: whichever option is chosen is shown whole, so a label wider than an explicit width
    /// widens the control (up to the host), whether or not it carries a tag; a short one keeps it.
    #[test]
    fn a_chosen_label_outgrows_an_explicit_width_and_a_short_one_does_not() {
        let options = [
            (0_u8, "A rather long program name"),
            (1, "Another long name"),
            (2, "Off"),
        ];
        let width_of = |chosen: u8, tagged: bool, host: f32| {
            let ctx = testkit::ctx();
            let mut v = chosen;
            let mut w = 0.0;
            frame(&ctx, Vec::new(), |ui| {
                ui.set_width(host);
                let select = Select::new(&mut v, &options).width(80.0).name("Pick");
                let select = if tagged {
                    select.unavailable(1, "Not installed")
                } else {
                    select
                };
                w = select.show(ui).rect.width();
            });
            w
        };
        assert!(
            width_of(0, false, 600.0) > 150.0,
            "a long choice is shown whole"
        );
        assert!(
            width_of(1, false, 600.0) > 100.0,
            "also when others are tagged"
        );
        assert!(
            width_of(0, true, 600.0) > 150.0,
            "an available choice among tagged ones"
        );
        assert!(
            width_of(1, true, 600.0) > width_of(1, false, 600.0),
            "an unavailable choice is shown whole with its tag"
        );
        assert!(
            (width_of(2, false, 600.0) - 80.0).abs() < 0.5,
            "a short choice keeps the width it was given"
        );
        assert!(
            (width_of(0, false, 120.0) - 120.0).abs() < 0.5,
            "no wider than the host: beyond it the label wraps"
        );
    }

    /// A list wide enough for an unavailable option's label and its tag shows the label whole.
    #[test]
    fn an_unavailable_option_keeps_its_whole_label_beside_its_tag() {
        fn elided(shape: &egui::Shape, label: &str) -> Option<bool> {
            match shape {
                egui::Shape::Vec(v) => v.iter().find_map(|s| elided(s, label)),
                egui::Shape::Text(t) if t.galley.text() == label => Some(t.galley.elided),
                _ => None,
            }
        }
        let ctx = testkit::ctx();
        let options = [(0_u8, "Alpha"), (1, "GNOME Terminal"), (2, "Xfce Terminal")];
        let mut v = 0_u8;
        let show_it = |ui: &mut Ui, v: &mut u8| {
            Select::new(v, &options)
                .unavailable(1, "Not installed")
                .unavailable(2, "Not installed")
                .width(120.0)
                .name("Pick")
                .id_salt("w")
                .show(ui)
        };
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show_it(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show_it(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| {
            show_it(ui, &mut v);
        });
        let shapes = frame(&ctx, Vec::new(), |ui| {
            show_it(ui, &mut v);
        });
        for label in ["GNOME Terminal", "Xfce Terminal"] {
            let state = shapes.iter().find_map(|c| elided(&c.shape, label));
            assert_eq!(state, Some(false), "{label} is painted whole");
        }
    }

    /// REVIEW-KIT-11: a long list (every branch) builds only the rows in view.
    #[test]
    fn a_long_list_shows_only_the_rows_in_view() {
        let ctx = testkit::ctx();
        let labels: Vec<String> = (0..2000).map(|i| format!("branch-{i}")).collect();
        let options: Vec<(usize, &str)> = labels.iter().map(String::as_str).enumerate().collect();
        let mut v = 0_usize;
        let show_long = |ui: &mut Ui, v: &mut usize| {
            Select::new(v, &options)
                .name("Branch")
                .id_salt("long")
                .show(ui)
        };
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show_long(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show_long(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| {
            id = show_long(ui, &mut v).id;
        });
        let nodes = spoken(&ctx, |ui| {
            show_long(ui, &mut v);
        });
        let rows = nodes
            .iter()
            .filter(|n| n.role == Role::ListBoxOption)
            .count();
        assert!(rows > 0 && rows < 20, "{rows} rows built");
        // End reaches the last branch, and the list scrolls to it.
        frame(&ctx, key(Key::End), |ui| id = show_long(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = show_long(ui, &mut v).id);
        let nodes = spoken(&ctx, |ui| {
            show_long(ui, &mut v);
        });
        assert!(
            nodes
                .iter()
                .any(|n| n.label.as_deref() == Some("branch-1999")),
            "the last row is in view"
        );
    }

    /// #280: a pinned last option ("Custom command…" after a long list of programs) is in view
    /// as soon as the list opens, without scrolling; the keys still reach it in order and back.
    #[test]
    fn a_pinned_last_option_is_in_view_without_scrolling() {
        let ctx = testkit::ctx();
        let labels: Vec<String> = (0..30)
            .map(|i| format!("program-{i}"))
            .chain(std::iter::once("Custom command…".to_owned()))
            .collect();
        let options: Vec<(usize, &str)> = labels.iter().map(String::as_str).enumerate().collect();
        let last = options.len() - 1;
        for pinned in [false, true] {
            let mut v = 0_usize;
            let show_list = |ui: &mut Ui, v: &mut usize| {
                let select = Select::new(v, &options)
                    .name("Terminal")
                    .id_salt(("pinned", pinned));
                if pinned { select.pin_last() } else { select }.show(ui)
            };
            let mut id = Id::NULL;
            frame(&ctx, Vec::new(), |ui| id = show_list(ui, &mut v).id);
            ctx.memory_mut(|m| m.request_focus(id));
            frame(&ctx, Vec::new(), |ui| id = show_list(ui, &mut v).id);
            frame(&ctx, key(Key::ArrowDown), |ui| {
                id = show_list(ui, &mut v).id;
            });
            let nodes = spoken(&ctx, |ui| {
                show_list(ui, &mut v);
            });
            let shown = nodes
                .iter()
                .any(|n| n.label.as_deref() == Some("Custom command…"));
            assert_eq!(
                shown, pinned,
                "pinned {pinned}: the last row shows on opening"
            );
            if !pinned {
                continue;
            }
            // End reaches it, Up leaves it for the row above (scrolled into view), Down comes
            // back, Enter chooses it.
            frame(&ctx, key(Key::End), |ui| id = show_list(ui, &mut v).id);
            frame(&ctx, key(Key::ArrowUp), |ui| id = show_list(ui, &mut v).id);
            frame(&ctx, Vec::new(), |ui| id = show_list(ui, &mut v).id);
            let nodes = spoken(&ctx, |ui| {
                show_list(ui, &mut v);
            });
            assert!(
                nodes
                    .iter()
                    .any(|n| n.label.as_deref() == Some("program-29")),
                "Up from the pinned row brings the row above it into view"
            );
            frame(&ctx, key(Key::ArrowDown), |ui| {
                id = show_list(ui, &mut v).id;
            });
            frame(&ctx, key(Key::Enter), |ui| id = show_list(ui, &mut v).id);
            assert_eq!(v, last, "Enter on the pinned row chooses it");
        }
    }

    /// #280: a short list with its last option pinned is as tall as its rows and the separator,
    /// with no room left over for the pinned row counted twice.
    #[test]
    fn a_short_pinned_list_has_no_empty_room() {
        let ctx = testkit::ctx();
        let options = [(0_u8, "kitty"), (1, "Foot"), (2, "Custom command…")];
        let id = Id::new("pinned-short");
        let mut v = 0_u8;
        let mut draw = |ui: &mut Ui| {
            Select::new(&mut v, &options).id(id).pin_last().show(ui);
        };
        frame(&ctx, Vec::new(), &mut draw);
        ctx.memory_mut(|m| m.request_focus(trigger_id(id)));
        frame(&ctx, key(Key::ArrowDown), &mut draw);
        frame(&ctx, Vec::new(), &mut draw);
        let list = egui::containers::AreaState::load(&ctx, id.with("kit-select-list"))
            .expect("open select")
            .rect();
        let m = theme::metrics();
        let want =
            3.0 * m.menu_item_height + super::super::menu::SEPARATOR_HEIGHT + 2.0 * m.menu_pad;
        assert!(
            (list.height() - want).abs() < 0.5,
            "list {} high, want {want}",
            list.height()
        );
    }

    /// #280: in a short window (562 points wide, as 900x560 at zoom 1.6, and shorter) the list is
    /// limited by the window, not by its row count: wherever the control sits, the pinned row is
    /// either inside the list or, with no room for it and a scrolling row, not drawn at all; at
    /// the issue's size it is shown.
    #[test]
    fn a_pinned_row_stays_inside_a_list_the_window_limits() {
        let labels: Vec<String> = (0..13)
            .map(|i| format!("program-{i}"))
            .chain(std::iter::once("Custom command…".to_owned()))
            .collect();
        let options: Vec<(usize, &str)> = labels.iter().map(String::as_str).enumerate().collect();
        for screen_h in [100_u16, 150, 200, 250, 350] {
            for top_pt in (0..screen_h.saturating_sub(40)).step_by(20) {
                let (height, top) = (f32::from(screen_h), f32::from(top_pt));
                let ctx = testkit::ctx();
                let id = Id::new(("pinned-short-host", screen_h, top_pt));
                let mut v = 0_usize;
                let mut draw = |ui: &mut Ui| {
                    ui.add_space(top);
                    Select::new(&mut v, &options).id(id).pin_last().show(ui);
                };
                let input = |events: Vec<Event>| egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        vec2(562.0, height),
                    )),
                    events,
                    ..Default::default()
                };
                let mut run = |events: Vec<Event>| {
                    let mut out = ctx.run_ui(input(events), |ui| draw(ui));
                    out.textures_delta.clear();
                    out.shapes
                };
                run(Vec::new());
                ctx.memory_mut(|m| m.request_focus(trigger_id(id)));
                run(Vec::new());
                run(key(Key::ArrowDown));
                let shapes = run(Vec::new());
                let list = egui::containers::AreaState::load(&ctx, id.with("kit-select-list"))
                    .expect("open select")
                    .rect();
                let pinned: Vec<egui::Rect> = galleys(&shapes)
                    .into_iter()
                    .filter(|(_, g)| g.text() == "Custom command…")
                    .map(|(at, _)| at)
                    .collect();
                for at in &pinned {
                    assert!(
                        list.expand(0.5).contains_rect(*at),
                        "height {height}, top {top}: pinned row {at:?} outside the list {list:?}"
                    );
                }
                if screen_h == 350 {
                    assert!(
                        !pinned.is_empty(),
                        "top {top}: the pinned row shows at the issue's window size"
                    );
                }
            }
        }
    }

    #[test]
    fn keyboard_opens_moves_chooses_and_closes() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        assert!(open_id(&ctx), "Down opens the list");
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        assert_eq!(v, 0, "moving the highlight does not choose");
        let mut changed = false;
        frame(&ctx, key(Key::Enter), |ui| {
            changed = show(ui, &mut v).changed();
        });
        assert!(changed);
        assert_eq!(v, 1, "Enter chooses the highlighted option");
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        assert!(!open_id(&ctx), "and closes the list");
        assert!(
            ctx.memory(|m| m.has_focus(id)),
            "focus stays on the trigger"
        );
    }

    #[test]
    fn a_pointer_opened_select_can_be_finished_with_the_keyboard() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut rect = egui::Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| rect = show(ui, &mut v).rect);
        testkit::click(&ctx, rect.center(), |ui| {
            show(ui, &mut v);
        });
        assert!(open_id(&ctx));
        frame(&ctx, key(Key::ArrowDown), |ui| {
            show(ui, &mut v);
        });
        frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        frame(&ctx, key(Key::Enter), |ui| {
            show(ui, &mut v);
        });
        assert_eq!(
            v, 1,
            "Down and Enter choose the next option after a pointer click"
        );
        assert!(!open_id(&ctx));
    }

    #[test]
    fn a_select_list_stays_inside_its_dialog_and_scrolls_to_the_last_option() {
        for (width, before, after) in [
            (280.0, 140.0, 0.0),
            (280.0, 0.0, 280.0),
            (100.0, 80.0, 80.0),
        ] {
            let ctx = testkit::ctx();
            let id = Id::new("bounded-select");
            let mut value = 0_u8;
            let options = (0..20)
                .map(|i| (i, format!("Option {i}")))
                .collect::<Vec<_>>();
            let options = options
                .iter()
                .map(|(i, label)| (*i, label.as_str()))
                .collect::<Vec<_>>();
            let host = std::cell::Cell::new(egui::Rect::NOTHING);
            let trigger = std::cell::Cell::new(egui::Rect::NOTHING);
            let mut draw = |ui: &mut Ui| {
                host.set(
                    overlay::floating(ui, overlay::Layer::Dialog, egui::Margin::same(16), |ui| {
                        ui.set_width(width);
                        ui.add_space(before);
                        trigger.set(Select::new(&mut value, &options).id(id).show(ui).rect);
                        ui.add_space(after);
                    })
                    .response
                    .rect,
                );
            };
            for _ in 0..3 {
                frame(&ctx, Vec::new(), &mut draw);
            }
            let at = trigger.get().center();
            testkit::click(&ctx, at, &mut draw);
            for _ in 0..3 {
                frame(&ctx, Vec::new(), &mut draw);
            }
            let list = egui::containers::AreaState::load(&ctx, id.with("kit-select-list"))
                .expect("open select")
                .rect();
            assert!(
                host.get().contains_rect(list),
                "list {list:?} escapes host {:?}",
                host.get()
            );
            frame(&ctx, key(Key::End), &mut draw);
            frame(&ctx, Vec::new(), &mut draw);
            frame(&ctx, key(Key::Enter), &mut draw);
            assert_eq!(value, 19, "the bounded list still reaches every option");
        }
    }

    /// A real pointer can press and release inside one frame (a quick click, or a slow frame).
    /// Egui keeps every layer's rank between uses, so a list opened before sits under a modal
    /// that was pressed since; in that frame both ask for the top and the old rank won, leaving
    /// the list open but unseen and out of reach.
    #[test]
    fn a_list_opened_by_a_one_frame_click_in_a_modal_is_above_the_modal() {
        let ctx = testkit::ctx();
        let id = Id::new("modal-select");
        let list_id = id.with("kit-select-list");
        let mut value = 0_u8;
        let trigger = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut draw = |ui: &mut Ui| {
            egui::Modal::new(Id::new("select-in-modal")).show(ui.ctx(), |ui| {
                trigger.set(Select::new(&mut value, &STRATEGIES).id(id).show(ui).rect);
            });
        };
        let settle = |draw: &mut dyn FnMut(&mut Ui)| {
            for _ in 0..4 {
                frame(&ctx, Vec::new(), &mut *draw);
            }
        };
        let list_on_top = || {
            let list = egui::containers::AreaState::load(&ctx, list_id)
                .expect("open select")
                .rect();
            ctx.layer_id_at(list.center())
                == Some(egui::LayerId::new(egui::Order::Foreground, list_id))
        };
        settle(&mut draw);
        // Used once the ordinary way, then the modal is pressed again.
        testkit::click(&ctx, trigger.get().center(), &mut draw);
        settle(&mut draw);
        assert!(list_on_top(), "a list opened by a click is on top");
        frame(&ctx, key(Key::Escape), &mut draw);
        settle(&mut draw);
        assert!(!open_id(&ctx));
        testkit::click(&ctx, pos2(2.0, 2.0), &mut draw);
        settle(&mut draw);
        let at = trigger.get().center();
        let mut events = testkit::press(at);
        events.extend(testkit::release(at));
        frame(&ctx, events, &mut draw);
        settle(&mut draw);
        assert!(open_id(&ctx));
        assert!(list_on_top(), "the list is the top layer over the modal");
    }

    #[test]
    fn keys_after_a_pointer_open_scroll_the_list_to_the_highlight() {
        let ctx = testkit::ctx();
        let id = Id::new("scrolled-select");
        let mut value = 0_u8;
        let labels: Vec<String> = (0..20).map(|i| format!("Option {i}")).collect();
        let options: Vec<(u8, &str)> = (0..20_u8).zip(labels.iter().map(String::as_str)).collect();
        let trigger = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut draw = |ui: &mut Ui| {
            trigger.set(Select::new(&mut value, &options).id(id).show(ui).rect);
        };
        for _ in 0..3 {
            frame(&ctx, Vec::new(), &mut draw);
        }
        testkit::click(&ctx, trigger.get().center(), &mut draw);
        for _ in 0..3 {
            frame(&ctx, Vec::new(), &mut draw);
        }
        frame(&ctx, key(Key::End), &mut draw);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame(&ctx, Vec::new(), &mut draw);
        }
        let list = egui::containers::AreaState::load(&ctx, id.with("kit-select-list"))
            .expect("open select")
            .rect();
        let row_of = |shapes: &[egui::epaint::ClippedShape], label: &str| {
            shapes.iter().find_map(|c| match &c.shape {
                // The trigger shows the chosen value in the same words; only rows below it count.
                egui::Shape::Text(t)
                    if t.galley.text() == label && t.visual_bounding_rect().min.y >= list.min.y =>
                {
                    Some(t.visual_bounding_rect())
                }
                _ => None,
            })
        };
        let last = row_of(&shapes, "Option 19");
        assert!(
            last.is_some_and(|row| list.contains_rect(row)),
            "End shows the last option inside the list: {last:?} in {list:?}"
        );
        frame(&ctx, key(Key::Home), &mut draw);
        for _ in 0..3 {
            shapes = frame(&ctx, Vec::new(), &mut draw);
        }
        let first = row_of(&shapes, "Option 0");
        assert!(
            first.is_some_and(|row| list.contains_rect(row)),
            "Home scrolls back to the first option: {first:?} in {list:?}"
        );
        // Closed and opened again from the keyboard, the list starts at the chosen option.
        frame(&ctx, key(Key::End), &mut draw);
        frame(&ctx, key(Key::Escape), &mut draw);
        frame(&ctx, key(Key::ArrowDown), &mut draw);
        for _ in 0..3 {
            shapes = frame(&ctx, Vec::new(), &mut draw);
        }
        let chosen = row_of(&shapes, "Option 0");
        assert!(
            chosen.is_some_and(|row| list.contains_rect(row)),
            "a reopened list shows the chosen option: {chosen:?} in {list:?}"
        );
    }

    #[test]
    fn typing_after_a_pointer_open_reveals_the_matching_option() {
        let ctx = testkit::ctx();
        let id = Id::new("typed-scrolled-select");
        let mut value = 0_u8;
        let labels: Vec<String> = (0..20)
            .map(|i| {
                if i == 19 {
                    "Zulu".into()
                } else {
                    format!("Option {i}")
                }
            })
            .collect();
        let options: Vec<(u8, &str)> = (0..20_u8).zip(labels.iter().map(String::as_str)).collect();
        let trigger = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut draw = |ui: &mut Ui| {
            trigger.set(Select::new(&mut value, &options).id(id).show(ui).rect);
        };
        for _ in 0..3 {
            frame(&ctx, Vec::new(), &mut draw);
        }
        testkit::click(&ctx, trigger.get().center(), &mut draw);
        frame(&ctx, vec![Event::Text("z".into())], &mut draw);
        let mut shapes = Vec::new();
        for _ in 0..3 {
            shapes = frame(&ctx, Vec::new(), &mut draw);
        }
        let list = egui::containers::AreaState::load(&ctx, id.with("kit-select-list"))
            .expect("open select")
            .rect();
        let matched = shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "Zulu" => Some(t.visual_bounding_rect()),
            _ => None,
        });
        assert!(
            matched.is_some_and(|row| list.contains_rect(row)),
            "the typed match must be visible: {matched:?} in {list:?}"
        );
        frame(&ctx, key(Key::Enter), &mut draw);
        assert_eq!(value, 19);
        assert!(!open_id(&ctx));
    }

    #[test]
    fn escape_closes_and_keeps_focus_and_typing_jumps() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::Space), |ui| id = show(ui, &mut v).id);
        assert!(open_id(&ctx), "Space opens the list");
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, vec![Event::Text("f".into())], |ui| {
            id = show(ui, &mut v).id;
        });
        frame(&ctx, key(Key::Enter), |ui| id = show(ui, &mut v).id);
        assert_eq!(v, 2, "typing f highlighted Fast-forward only");
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::Escape), |ui| id = show(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        assert!(!open_id(&ctx), "Esc closes");
        assert!(
            ctx.memory(|m| m.has_focus(id)),
            "and keeps focus on the trigger"
        );
        assert_eq!(v, 2);
    }

    #[test]
    fn pointer_chooses_from_the_list() {
        let ctx = testkit::ctx();
        let mut v = 0_u8;
        let mut rect = egui::Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| rect = show(ui, &mut v).rect);
        testkit::click(&ctx, rect.center(), |ui| {
            show(ui, &mut v);
        });
        assert!(open_id(&ctx));
        // Two frames for the list's sizing pass, then click the third row.
        frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        let m = theme::metrics();
        let row = pos2(
            rect.center().x,
            rect.bottom() + LIST_GAP + m.menu_pad + 2.5 * m.menu_item_height,
        );
        testkit::click(&ctx, row, |ui| {
            show(ui, &mut v);
        });
        assert_eq!(v, 2);
        frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        assert!(!open_id(&ctx));
    }

    /// Whether a frame painted the option `label` (the list, open or leaving).
    fn painted(shapes: &[egui::epaint::ClippedShape], label: &str) -> bool {
        fn walk(shape: &egui::Shape, label: &str) -> bool {
            match shape {
                egui::Shape::Vec(v) => v.iter().any(|s| walk(s, label)),
                egui::Shape::Text(t) => t.galley.text() == label,
                _ => false,
            }
        }
        shapes.iter().any(|c| walk(&c.shape, label))
    }

    #[test]
    fn a_pointer_opened_list_fades_out_and_a_keyboard_one_vanishes() {
        let ctx = testkit::ctx();
        motion::set_reduced(false);
        let last = STRATEGIES[2].1;
        let mut v = 0_u8;
        let mut rect = egui::Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| rect = show(ui, &mut v).rect);
        testkit::click(&ctx, rect.center(), |ui| {
            show(ui, &mut v);
        });
        for _ in 0..12 {
            frame(&ctx, Vec::new(), |ui| {
                show(ui, &mut v);
            });
        }
        assert!(open_id(&ctx));
        // A click outside closes it; the list keeps painting for a moment as it leaves.
        testkit::click(
            &ctx,
            pos2(rect.right() + 300.0, rect.bottom() + 300.0),
            |ui| {
                show(ui, &mut v);
            },
        );
        let leaving = frame(&ctx, Vec::new(), |ui| {
            show(ui, &mut v);
        });
        assert!(!open_id(&ctx), "the click outside closed the list");
        assert!(painted(&leaving, last), "the closed list fades out");
        let mut gone = false;
        for _ in 0..30 {
            gone = !painted(
                &frame(&ctx, Vec::new(), |ui| {
                    show(ui, &mut v);
                }),
                last,
            );
        }
        assert!(gone, "and is gone once its exit has run");

        // Opened and closed from the keyboard: no exit.
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        assert!(open_id(&ctx), "Down opened it");
        frame(&ctx, key(Key::Escape), |ui| id = show(ui, &mut v).id);
        let after = frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        motion::set_reduced(true);
        assert!(!open_id(&ctx));
        assert!(
            !painted(&after, last),
            "a keyboard-opened list closes at once"
        );
    }

    #[test]
    fn unsalted_selects_keep_their_own_list() {
        let ctx = testkit::ctx();
        let (mut a, mut b) = (0_u8, 0_u8);
        let show = |ui: &mut Ui, a: &mut u8, b: &mut u8| {
            Select::new(a, &STRATEGIES).name("First").show(ui);
            Select::new(b, &STRATEGIES).name("Second").show(ui).rect
        };
        let mut second = egui::Rect::NOTHING;
        frame(&ctx, Vec::new(), |ui| second = show(ui, &mut a, &mut b));
        testkit::click(&ctx, second.center(), |ui| {
            show(ui, &mut a, &mut b);
        });
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut a, &mut b);
        });
        let expanded = |name: &str| testkit::find(&nodes, name).and_then(|n| n.expanded);
        assert_eq!(expanded("Second"), Some(true), "the clicked select opens");
        assert_eq!(expanded("First"), Some(false), "its neighbour stays shut");
    }

    #[test]
    fn disabled_and_loading_selects_stay_shut() {
        let ctx = testkit::ctx();
        for (enabled, loading) in [(false, false), (true, true)] {
            let mut v = 0_u8;
            let mut rect = egui::Rect::NOTHING;
            let s = |ui: &mut Ui, v: &mut u8| {
                Select::new(v, &STRATEGIES)
                    .enabled(enabled)
                    .loading(loading)
                    .id_salt(("d", enabled))
                    .show(ui)
            };
            frame(&ctx, Vec::new(), |ui| rect = s(ui, &mut v).rect);
            testkit::click(&ctx, rect.center(), |ui| {
                s(ui, &mut v);
            });
            assert!(!open_id(&ctx), "enabled {enabled}, loading {loading}");
        }
    }

    #[test]
    fn trigger_speaks_as_a_combo_box_with_its_value() {
        let ctx = testkit::ctx();
        let mut v = 1_u8;
        let nodes = spoken(&ctx, |ui| {
            show(ui, &mut v);
        });
        let n = nodes
            .iter()
            .find(|n| n.role == Role::ComboBox)
            .expect("a combo box");
        assert_eq!(n.label.as_deref(), Some("Pull strategy"));
        assert_eq!(n.value.as_deref(), Some("Rebase"));
        assert_eq!(n.expanded, Some(false));
    }

    /// The closed trigger says what the value is and whether it is the recommended one, however
    /// the value came to be: `value` is the only input.
    #[test]
    fn the_closed_trigger_marks_the_recommended_value_and_no_other() {
        let ctx = testkit::ctx();
        for (value, text, marked) in [
            (2_u8, "Fast-forward only (Recommended)", true),
            (1, "Rebase", false),
        ] {
            let mut v = value;
            let mut trigger = egui::Rect::NOTHING;
            let shapes = frame(&ctx, Vec::new(), |ui| {
                trigger = Select::new(&mut v, &STRATEGIES)
                    .recommended(2)
                    .fit_to_options()
                    .name("Pull strategy")
                    .id_salt(("marked", value))
                    .show(ui)
                    .rect;
            });
            assert!(
                painted(&shapes, text),
                "{value}: the trigger paints {text:?}"
            );
            assert_eq!(
                painted(&shapes, "Fast-forward only (Recommended)"),
                marked,
                "{value}: the mark follows the value"
            );
            let nodes = spoken(&ctx, |ui| {
                Select::new(&mut v, &STRATEGIES)
                    .recommended(2)
                    .name("Pull strategy")
                    .id_salt(("marked", value))
                    .show(ui);
            });
            let n = nodes
                .iter()
                .find(|n| n.role == Role::ComboBox)
                .expect("a combo box");
            assert_eq!(n.value.as_deref(), Some(text), "{value}: spoken as shown");
            assert!(trigger.width() > 0.0, "{value}: the trigger has a width");
        }
    }

    /// A trigger fitted to its options is as wide for the marked value as for the longest label,
    /// so choosing the recommended value never moves the controls beside it.
    #[test]
    fn a_fitted_trigger_keeps_its_width_when_the_marked_value_is_chosen() {
        let ctx = testkit::ctx();
        let width = |value: u8| {
            let mut v = value;
            let mut rect = egui::Rect::NOTHING;
            frame(&ctx, Vec::new(), |ui| {
                ui.set_width(600.0);
                rect = Select::new(&mut v, &STRATEGIES)
                    .recommended(1)
                    .fit_to_options()
                    .id_salt("fit-marked")
                    .show(ui)
                    .rect;
            });
            rect.width()
        };
        assert!(
            (width(1) - width(0)).abs() < 0.5,
            "{} vs {}",
            width(1),
            width(0)
        );
    }

    /// An unavailable recommended option carries its note, not the mark.
    #[test]
    fn an_unavailable_recommended_value_shows_its_note_not_the_mark() {
        let ctx = testkit::ctx();
        let mut v = 2_u8;
        let shapes = frame(&ctx, Vec::new(), |ui| {
            Select::new(&mut v, &STRATEGIES)
                .recommended(2)
                .unavailable(2, "Needs git 2.34")
                .id_salt("unavailable-marked")
                .show(ui);
        });
        assert!(!painted(&shapes, "Fast-forward only (Recommended)"));
        assert!(painted(&shapes, "Fast-forward only"));
    }

    /// A settings row puts its control at the right in a right-to-left layout: a fixed-width
    /// select sits there, at its width, as it did before it stacked a label and message.
    #[test]
    fn a_select_in_a_right_to_left_row_sits_at_the_right() {
        let ctx = testkit::ctx();
        let mut v = 1_u8;
        let (mut trigger, mut row) = (egui::Rect::NOTHING, egui::Rect::NOTHING);
        frame(&ctx, Vec::new(), |ui| {
            ui.set_width(600.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                row = ui.max_rect();
                trigger = Select::new(&mut v, &STRATEGIES)
                    .width(200.0)
                    .id_salt("rtl")
                    .show(ui)
                    .rect;
            });
        });
        assert!((trigger.width() - 200.0).abs() < 0.5, "{trigger:?}");
        assert!(
            (trigger.right() - row.right()).abs() < 0.5,
            "the trigger ends at {} in a row ending at {}",
            trigger.right(),
            row.right()
        );
    }

    /// A select's helper describes it and its error marks it invalid, as a text field's do.
    #[test]
    fn a_select_speaks_its_helper_and_its_error() {
        let ctx = testkit::ctx();
        ctx.enable_accesskit();
        let combo = |helper: &str, error: Option<&str>| {
            let mut v = 9_u8;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                Select::new(&mut v, &STRATEGIES)
                    .name("Strategy")
                    .helper(helper)
                    .error(error)
                    .id_salt("speaks")
                    .show(ui);
            });
            out.textures_delta.clear();
            out.platform_output
                .accesskit_update
                .and_then(|u| {
                    u.nodes
                        .into_iter()
                        .map(|(_, n)| n)
                        .find(|n| n.role() == Role::ComboBox)
                })
                .expect("a combo box")
        };
        let helped = combo("Canopy passes it to git.", None);
        assert_eq!(helped.description(), Some("Canopy passes it to git."));
        assert_eq!(helped.invalid(), None);
        let wrong = combo("Canopy passes it to git.", Some("Choose a strategy."));
        assert_eq!(
            wrong.description(),
            Some("Choose a strategy."),
            "the error wins"
        );
        assert_eq!(wrong.invalid(), Some(Invalid::True));
    }

    #[test]
    fn keyboard_lists_open_at_once_even_with_motion_on() {
        let ctx = testkit::ctx();
        // Motion on: a pointer-opened list would fade in; a keyboard-opened one never does.
        motion::set_reduced(false);
        let overlay = theme::surfaces(&ctx).overlay;
        let mut v = 0_u8;
        let mut id = Id::NULL;
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        ctx.memory_mut(|m| m.request_focus(id));
        frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        frame(&ctx, key(Key::ArrowDown), |ui| id = show(ui, &mut v).id);
        let shapes = frame(&ctx, Vec::new(), |ui| id = show(ui, &mut v).id);
        motion::set_reduced(true);
        let opaque = shapes.iter().any(|c| {
            matches!(&c.shape, egui::Shape::Rect(r) if r.fill == overlay && r.blur_width == 0.0)
        });
        assert!(opaque, "the list is fully shown on its first visible frame");
    }

    /// Every galley a frame painted: where it is and what it says.
    fn galleys(
        shapes: &[egui::epaint::ClippedShape],
    ) -> Vec<(egui::Rect, std::sync::Arc<egui::Galley>)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(egui::Rect, std::sync::Arc<egui::Galley>)>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(t) => {
                    out.push((t.visual_bounding_rect(), std::sync::Arc::clone(&t.galley)));
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for c in shapes {
            walk(&c.shape, &mut out);
        }
        out
    }

    /// Where a select in a dialog of `width` (with `before` of room above it and `after` below)
    /// put its closed control and, once clicked open, its list.
    struct Scene {
        host: egui::Rect,
        trigger: egui::Rect,
        list: egui::Rect,
        closed: Vec<(egui::Rect, std::sync::Arc<egui::Galley>)>,
        open: Vec<(egui::Rect, std::sync::Arc<egui::Galley>)>,
    }

    /// The options are shown by a select of `trigger_width` (`None`: the dialog's width), with
    /// `recommended` marked, `before` and `after` room in a dialog of `width`, `indent` from its
    /// left edge.
    #[expect(
        clippy::too_many_arguments,
        reason = "one scene, every knob the tests turn"
    )]
    fn scene(
        options: &[(u8, &str)],
        recommended: Option<u8>,
        chosen: u8,
        width: f32,
        trigger_width: Option<f32>,
        indent: f32,
        before: f32,
        after: f32,
    ) -> Scene {
        let ctx = testkit::ctx();
        let id = Id::new("scene-select");
        let mut value = chosen;
        let host = std::cell::Cell::new(egui::Rect::NOTHING);
        let trigger = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut draw = |ui: &mut Ui| {
            host.set(
                overlay::floating(ui, overlay::Layer::Dialog, egui::Margin::same(16), |ui| {
                    ui.set_width(width);
                    ui.add_space(before);
                    ui.horizontal(|ui| {
                        ui.add_space(indent);
                        let mut select = Select::new(&mut value, options).id(id);
                        if let Some(w) = trigger_width {
                            select = select.width(w);
                        }
                        if let Some(r) = recommended {
                            select = select.recommended(r);
                        }
                        trigger.set(select.show(ui).rect);
                    });
                    ui.add_space(after);
                })
                .response
                .rect,
            );
        };
        let mut closed = Vec::new();
        for _ in 0..3 {
            closed = galleys(&frame(&ctx, Vec::new(), &mut draw));
        }
        let at = trigger.get().center();
        testkit::click(&ctx, at, &mut draw);
        let mut open = Vec::new();
        for _ in 0..3 {
            open = galleys(&frame(&ctx, Vec::new(), &mut draw));
        }
        let list = egui::containers::AreaState::load(&ctx, id.with("kit-select-list"))
            .expect("open select")
            .rect();
        Scene {
            host: host.get(),
            trigger: trigger.get(),
            list,
            closed,
            open,
        }
    }

    const LONG: &str = "Rebase the local commits onto the remote branch, keeping merge commits \
                        and reapplying every one of them in turn";

    /// The galleys that say `label`, inside `area`.
    fn saying<'g>(
        galleys: &'g [(egui::Rect, std::sync::Arc<egui::Galley>)],
        label: &str,
        area: egui::Rect,
    ) -> Vec<&'g std::sync::Arc<egui::Galley>> {
        galleys
            .iter()
            .filter(|(at, g)| g.text() == label && area.expand(1.0).contains_rect(*at))
            .map(|(_, g)| g)
            .collect()
    }

    /// #225: no select trims an item. A label longer than the control is shown whole in the
    /// closed control (which widens to it) and in the open list.
    #[test]
    fn a_long_label_is_shown_whole_in_the_closed_control_and_the_list() {
        let options = [(0_u8, "Off"), (1, LONG)];
        let s = scene(&options, None, 1, 900.0, Some(120.0), 0.0, 0.0, 400.0);
        let closed = saying(&s.closed, LONG, s.trigger);
        assert_eq!(closed.len(), 1, "the closed control says the label");
        assert!(
            !closed[0].elided,
            "the closed control shows it without an ellipsis"
        );
        assert_eq!(closed[0].rows.len(), 1, "on one line, as the host has room");
        assert!(
            s.trigger.width() > 120.0,
            "the control widened: {:?}",
            s.trigger
        );
        assert!(s.host.contains_rect(s.trigger));
        let listed = saying(&s.open, LONG, s.list);
        assert_eq!(listed.len(), 1, "the list says the label");
        assert!(!listed[0].elided, "the list shows it without an ellipsis");
        assert_eq!(listed[0].rows.len(), 1);
    }

    /// #225: when the host is too narrow for the label on one line, it wraps, in the control (which
    /// grows taller) and in the list (every row as tall as the tallest), all inside the host, for
    /// as many rows as a label may take (three).
    #[test]
    fn a_label_wider_than_the_host_wraps_in_both_and_stays_inside() {
        let options = [(0_u8, "Off"), (1, LONG), (2, "Daily")];
        let s = scene(&options, None, 1, 360.0, None, 0.0, 0.0, 260.0);
        let one_line = Size::Lg.height();
        let closed = saying(&s.closed, LONG, s.trigger);
        assert_eq!(closed.len(), 1);
        assert!(!closed[0].elided, "wrapped, not cut");
        assert!(closed[0].rows.len() > 1, "wrapped onto more lines");
        assert!(
            s.trigger.height() > one_line,
            "the control grew: {:?}",
            s.trigger
        );
        assert!(
            s.host.contains_rect(s.trigger),
            "{:?} in {:?}",
            s.trigger,
            s.host
        );
        let listed = saying(&s.open, LONG, s.list);
        assert_eq!(listed.len(), 1);
        assert!(!listed[0].elided);
        assert!(listed[0].rows.len() > 1);
        assert!(s.host.contains_rect(s.list), "{:?} in {:?}", s.list, s.host);
    }

    /// #255: the recommended entry shows its whole label beside its badge, in a select narrower
    /// than the label.
    #[test]
    fn the_recommended_entry_keeps_its_whole_label_beside_the_badge() {
        let options = [(0_u8, "Merge"), (1, "Don't rebase"), (2, "Rebase")];
        let s = scene(&options, Some(1), 0, 600.0, Some(90.0), 0.0, 0.0, 200.0);
        let listed = saying(&s.open, "Don't rebase", s.list);
        assert_eq!(listed.len(), 1, "the recommended label is in the list");
        assert!(!listed[0].elided, "no ellipsis before the badge");
        assert_eq!(listed[0].rows.len(), 1);
        let badge = s
            .open
            .iter()
            .find(|(at, g)| g.text() == "Recommended" && s.list.contains_rect(*at))
            .expect("the badge");
        let label = s
            .open
            .iter()
            .find(|(at, g)| g.text() == "Don't rebase" && s.list.contains_rect(*at))
            .unwrap();
        assert!(
            label.0.right() <= badge.0.left(),
            "the label ends before the badge"
        );
        assert!(s.host.contains_rect(s.list));
    }

    /// The list lines up with its control: at least as wide, the same left edge; right-aligned to
    /// the control when it would not fit to the right; inside the host either way.
    #[test]
    fn the_list_lines_up_with_its_control() {
        let options = [(0_u8, "Off"), (1, "A medium length option"), (2, "Daily")];
        // Room to the right: the left edges are equal and the list is no narrower.
        let s = scene(&options, None, 0, 500.0, Some(160.0), 0.0, 0.0, 260.0);
        assert!(
            (s.list.left() - s.trigger.left()).abs() < 0.5,
            "list {:?} trigger {:?}",
            s.list,
            s.trigger
        );
        assert!(s.list.width() >= s.trigger.width() - 0.5);
        assert!(s.host.contains_rect(s.list));
        // A control at the right end of a dialog whose list is wider: right edges are equal.
        let long = [(0_u8, "Off"), (1, "A considerably longer second option")];
        let s = scene(&long, None, 0, 400.0, Some(60.0), 340.0, 0.0, 200.0);
        assert!(s.list.width() > s.trigger.width());
        assert!(
            (s.list.right() - s.trigger.right()).abs() < 0.5,
            "list {:?} trigger {:?}",
            s.list,
            s.trigger
        );
        assert!(s.host.contains_rect(s.list), "{:?} in {:?}", s.list, s.host);
    }

    /// #255: a recommended label longer than the host still shows whole beside its badge (it wraps
    /// in the space the badge leaves), with the badge on the first line's row.
    #[test]
    fn a_long_recommended_label_wraps_beside_its_badge_in_a_narrow_host() {
        let options = [(0_u8, "Merge"), (1, LONG)];
        let s = scene(&options, Some(1), 0, 460.0, Some(120.0), 0.0, 0.0, 300.0);
        let listed = saying(&s.open, LONG, s.list);
        assert_eq!(listed.len(), 1, "the recommended label is in the list");
        assert!(!listed[0].elided, "wrapped, not cut before the badge");
        assert!(listed[0].rows.len() > 1);
        let badge = s
            .open
            .iter()
            .find(|(at, g)| g.text() == "Recommended" && s.list.contains_rect(*at))
            .expect("the badge");
        let label = s
            .open
            .iter()
            .find(|(at, g)| g.text() == LONG && s.list.contains_rect(*at))
            .unwrap();
        assert!(
            label.0.right() <= badge.0.left(),
            "the text stays left of the badge"
        );
        assert!(s.host.contains_rect(s.list), "{:?} in {:?}", s.list, s.host);
    }

    /// The height reserved for a row is exactly what the painter's layout of the label needs, at
    /// every width around the point where the label stops fitting (egui rounds the width up to a
    /// pixel and lets a line run 0.5 pt past its wrap width, so a label a little over still
    /// paints on one line, and a fixed tolerance here would reserve height it never uses).
    #[test]
    fn a_row_is_made_taller_exactly_when_its_label_wraps() {
        let label = "Rebase the local commits onto the remote branch";
        let options = [(0_u8, "Off"), (1, label)];
        let ctx = testkit::ctx();
        let mut value = 0_u8;
        let mut wraps_at = Vec::new();
        frame(&ctx, Vec::new(), |ui| {
            let m = theme::metrics();
            let select = Select::new(&mut value, &options);
            let font = FontId::new(m.body_size, theme::medium());
            let natural = ui
                .painter()
                .layout_no_wrap(label.to_owned(), font.clone(), Color32::WHITE)
                .size()
                .x;
            let whole = natural + 3.0 * ITEM_PAD + m.icon_sm + 2.0 * m.menu_pad;
            for step in 0..=80_usize {
                let host = whole - 2.0 + crate::count_f32(step) * 0.05;
                let room = host - 2.0 * m.menu_pad - 3.0 * ITEM_PAD - m.icon_sm;
                let painted =
                    ui.painter()
                        .layout(label.to_owned(), font.clone(), Color32::WHITE, room);
                let reserved = select.measure_list(ui, 50.0, host).1 > m.menu_item_height;
                assert_eq!(
                    reserved,
                    painted.rows.len() > 1,
                    "host {host}: room {room} for a label {natural} wide"
                );
                wraps_at.push((host - whole, reserved));
            }
        });
        assert!(
            wraps_at.iter().any(|&(_, r)| r) && wraps_at.iter().any(|&(_, r)| !r),
            "the sweep crosses the boundary"
        );
        assert!(
            wraps_at
                .iter()
                .find(|&&(d, _)| (d + 0.3).abs() < 0.01)
                .is_some_and(|&(_, r)| !r),
            "0.3 pt short still paints on one line (egui rounds the width and allows 0.5 more), so no taller row is reserved"
        );
        assert!(
            wraps_at
                .iter()
                .find(|&&(d, _)| (d + 2.0).abs() < 0.01)
                .is_some_and(|&(_, r)| r),
            "two points short wraps"
        );
    }

    /// The list's measured size is remembered between frames but follows the options and the
    /// widths when they change.
    #[test]
    fn the_remembered_list_size_follows_the_options_and_the_widths() {
        let ctx = testkit::ctx();
        let id = Id::new("remembered-size");
        let short = [(0_u8, "Off"), (1, "Daily")];
        let long = [
            (0_u8, "Off"),
            (1, "Every day, at the end of the working day"),
        ];
        let width_of = |options: &[(u8, &str)], trigger_w: f32, bounds_w: f32| {
            let mut value = 0_u8;
            let mut out = (0.0, 0.0);
            frame(&ctx, Vec::new(), |ui| {
                out = Select::new(&mut value, options).list_size(ui, id, trigger_w, bounds_w);
            });
            out
        };
        let a = width_of(&short, 60.0, 800.0);
        assert_eq!(
            a,
            width_of(&short, 60.0, 800.0),
            "the same inputs, the same size"
        );
        let b = width_of(&long, 60.0, 800.0);
        assert!(b.0 > a.0, "longer options widen the list: {a:?} then {b:?}");
        let c = width_of(&long, 60.0, 150.0);
        assert!(
            c.0 <= 150.0 && c.1 > b.1,
            "a narrower host wraps: {b:?} then {c:?}"
        );
        let d = width_of(&long, 400.0, 800.0);
        assert!(d.0 >= 400.0, "a wider trigger widens the list: {d:?}");
        assert_eq!(width_of(&short, 60.0, 800.0), a, "and back again");
    }

    /// What the list's minimum width used to be.
    const LIST_OLD_MIN_WIDTH: f32 = 200.0;

    /// #255 as reported: a recommended label cut short before its badge ("Don't reba…") when the
    /// list was only about as wide as the select. The list sized itself for label, badge, check and
    /// padding but left out its own inner padding, so a label that just filled a 200-point list lost
    /// its last letters. Here the label fills the list as nearly as it can without going over.
    #[test]
    fn a_recommended_label_that_just_fills_the_list_loses_no_letter() {
        let ctx = testkit::ctx();
        let m = theme::metrics();
        let badge = Badge::new("Recommended").tone(Tone::Accent);
        let mut badge_w = 0.0;
        let mut label = String::from("Don't ");
        frame(&ctx, Vec::new(), |ui| {
            badge_w = badge.measure(ui).x;
            let need = |text: &str| {
                ui.painter()
                    .layout_no_wrap(
                        text.to_owned(),
                        FontId::new(m.body_size, theme::medium()),
                        Color32::WHITE,
                    )
                    .size()
                    .x
                    + badge_w
                    + m.icon_sm
                    + 4.0 * ITEM_PAD
            };
            // The widest label whose row, with badge, check and padding but not the list's own
            // inner padding, still fits the 200 points a list used to be at least.
            for c in [
                "m", "m", "m", "m", "m", "m", "m", "m", "w", "n", "i", "i", "i", "i", ".",
            ] {
                let longer = format!("{label}{c}");
                if need(&longer) <= LIST_OLD_MIN_WIDTH - 0.5 {
                    label = longer;
                }
            }
        });
        let options = [(0_u8, "Merge"), (1, label.as_str())];
        let s = scene(&options, Some(1), 0, 600.0, Some(120.0), 0.0, 0.0, 200.0);
        let listed = saying(&s.open, &label, s.list);
        assert_eq!(
            listed.len(),
            1,
            "the recommended label {label:?} is in the list"
        );
        assert!(
            !listed[0].elided,
            "{label:?} is shown whole beside its badge"
        );
        assert_eq!(listed[0].rows.len(), 1);
    }

    /// What a select whose chosen value says `label` painted on its first frame: the closed
    /// control's rectangle and the text it painted.
    fn first_frame(
        label: &str,
        recommended: bool,
    ) -> (egui::Rect, Vec<std::sync::Arc<egui::Galley>>) {
        let options = [(0_u8, "Off"), (1, label)];
        let ctx = testkit::ctx();
        let draw = |ui: &mut Ui, value: &mut u8| {
            ui.set_width(400.0);
            let mut select = Select::new(value, &options).id_salt("hostile");
            if recommended {
                select = select.recommended(1);
            }
            select.show(ui).rect
        };
        let mut value = 0_u8;
        frame(&ctx, Vec::new(), |ui| {
            draw(ui, &mut value);
        });
        value = 1;
        let mut trigger = egui::Rect::NOTHING;
        let shapes = frame(&ctx, Vec::new(), |ui| trigger = draw(ui, &mut value));
        let said = galleys(&shapes).into_iter().map(|(_, g)| g).collect();
        (trigger, said)
    }

    /// The text of a label cut at the cap: its first `LABEL_MAX_CHARS` characters and an ellipsis.
    fn cut(label: &str) -> String {
        label
            .chars()
            .take(LABEL_MAX_CHARS)
            .chain(std::iter::once('\u{2026}'))
            .collect()
    }

    /// #187: a value from a hostile source (a crafted git config value) can be a megabyte of text.
    /// The closed control showed all of it, wrapped: 457,184 pt tall, and half a second to lay out.
    /// It shows the first characters and an ellipsis, so it is as tall as a label of the cap, and
    /// the whole value stays in what a screen reader hears.
    #[test]
    fn a_megabyte_value_is_cut_in_the_closed_control_which_stays_short() {
        let word = "refs/heads/feature-";
        let huge = word.repeat(1_000_000 / word.len());
        let (trigger, said) = first_frame(&huge, false);
        let shown = cut(&huge);
        assert!(
            said.iter().any(|g| g.text() == shown),
            "the control paints the first {LABEL_MAX_CHARS} characters and an ellipsis"
        );
        assert!(
            said.iter().all(|g| g.text().len() < 2 * LABEL_MAX_CHARS),
            "no galley holds the whole value"
        );
        let (cap, _) = first_frame(&cut(&huge), false);
        assert!(
            (trigger.height() - cap.height()).abs() < 0.5,
            "as tall as the cap's text: {} against {}",
            trigger.height(),
            cap.height()
        );
        assert!(trigger.height() < 20_000.0, "{}", trigger.height());

        let options = [(0_u8, "Off"), (1, huge.as_str())];
        let mut value = 1_u8;
        let nodes = spoken(&testkit::ctx(), |ui| {
            Select::new(&mut value, &options)
                .name("Pull strategy")
                .id_salt("hostile")
                .show(ui);
        });
        let combo = nodes
            .iter()
            .find(|n| n.role == Role::ComboBox)
            .expect("a combo box");
        assert_eq!(
            combo.value.as_deref(),
            Some(huge.as_str()),
            "a screen reader gets the whole value"
        );
    }

    /// The cut falls between characters, so a multibyte value is never split inside one, and the
    /// recommended mark still follows the ellipsis.
    #[test]
    fn a_long_multibyte_value_is_cut_between_characters_and_keeps_its_mark() {
        let huge = "\u{65e5}\u{672c}\u{8a9e}".repeat(LABEL_MAX_CHARS);
        let (_, said) = first_frame(&huge, true);
        let shown = format!("{}{RECOMMENDED_SUFFIX}", cut(&huge));
        assert!(
            said.iter().any(|g| g.text() == shown),
            "the cut value, then the mark"
        );
    }

    /// A label that fits its three rows is shown whole, with no ellipsis, however many rows it
    /// takes: a long recommendation wrapped in a narrow control is not a hostile value.
    #[test]
    fn a_label_that_fits_three_rows_is_shown_whole() {
        let label = "Rebase the local commits onto the remote branch, keeping merge commits \
                     and reapplying every one of them in turn, then push";
        let (trigger, said) = first_frame(label, false);
        let galley = said
            .iter()
            .find(|g| g.text() == label)
            .expect("the control says the label");
        assert!(!galley.elided, "no ellipsis");
        assert!(
            (2..=3).contains(&galley.rows.len()),
            "{} rows",
            galley.rows.len()
        );
        assert!(trigger.height() > Size::Lg.height(), "the control grew");
    }

    /// What the closed control cuts, its tooltip says (at the default Tooltips level), up to 600
    /// characters and then how many there are, in place of the caller's own tooltip.
    #[test]
    fn a_cut_value_is_in_the_tooltip_in_place_of_the_callers() {
        let hover = |label: &str, text: &str| {
            let options = [(0_u8, "Off"), (1, label)];
            testkit::painted_after_hover(pos2(20.0, 20.0), text, |ui| {
                let mut value = 1_u8;
                ui.set_width(400.0);
                Select::new(&mut value, &options)
                    .tooltip("Caller's tip")
                    .id_salt("tip")
                    .show(ui);
            })
        };
        // Cut by the character cap: the control paints the start, the tooltip the first 600.
        let huge = format!("{}tail", "x".repeat(LABEL_MAX_CHARS + 50));
        assert_eq!(
            hover(&huge, &cut(&huge)),
            1,
            "the control says the first characters"
        );
        assert_eq!(
            hover(
                &huge,
                &format!("{}\u{2026} 4,150 characters", "x".repeat(600))
            ),
            1,
            "the tooltip says the first 600 characters and the count"
        );
        assert_eq!(hover(&huge, "Caller's tip"), 0, "in place of the caller's");
        // Cut only by the rows: the control's text and the tooltip both say the whole label.
        let long = "word ".repeat(100);
        assert_eq!(hover(&long, &long), 2, "the control's text and the tooltip");
        assert_eq!(hover(&long, "Caller's tip"), 0);
    }

    /// A select fitted to its options does not lay out a megabyte to size itself.
    #[test]
    fn fitting_a_megabyte_label_measures_only_what_is_shown() {
        let huge = "w".repeat(1_000_000);
        let ctx = testkit::ctx();
        let mut fit = 0.0;
        let mut cap = 0.0;
        frame(&ctx, Vec::new(), |ui| {
            fit = fit_width(ui, [huge.as_str()]);
            cap = fit_width_of(ui, [cut(&huge).as_str()]);
        });
        assert!((fit - cap).abs() < 1.0, "{fit} against {cap}");
    }

    /// One frame at `time` seconds with `events`: every galley it painted.
    fn frame_at(
        ctx: &egui::Context,
        time: f64,
        events: Vec<Event>,
        mut draw: impl FnMut(&mut Ui),
    ) -> Vec<(egui::Rect, std::sync::Arc<egui::Galley>)> {
        let mut out = ctx.run_ui(
            egui::RawInput {
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| draw(ui),
        );
        out.textures_delta.clear();
        galleys(&out.shapes)
    }

    /// A million characters of ordinary words.
    fn megabyte() -> String {
        "abcdefghi ".repeat(100_000)
    }

    /// What a tooltip says of [`megabyte`]: its first 600 characters and how many there are.
    fn capped_tip(label: &str) -> String {
        let head: String = label.chars().take(TOOLTIP_MAX_CHARS).collect();
        format!("{head}\u{2026} 1,000,000 characters")
    }

    /// #187: the tooltip of a cut value laid out all of it, wrapped to the tooltip's width: a
    /// megabyte took 9.7 s to show and painted a galley 394,740 pt tall. It says the first 600
    /// characters and how many there are.
    #[test]
    fn a_megabyte_value_has_a_short_tooltip() {
        let huge = megabyte();
        let options = [(0_u8, "Off"), (1, huge.as_str())];
        let ctx = testkit::ctx();
        let mut value = 1_u8;
        let mut painted = Vec::new();
        for i in 0..12_u8 {
            let events = if i == 0 {
                vec![Event::PointerMoved(pos2(20.0, 20.0))]
            } else {
                Vec::new()
            };
            painted = frame_at(&ctx, f64::from(i) * 0.2, events, |ui| {
                ui.set_width(400.0);
                Select::new(&mut value, &options).id_salt("tip").show(ui);
            });
        }
        assert!(
            painted.iter().any(|(_, g)| g.text() == capped_tip(&huge)),
            "the tooltip says the first {TOOLTIP_MAX_CHARS} characters and the count"
        );
        let tallest = painted.iter().map(|(_, g)| g.size().y).fold(0.0, f32::max);
        assert!(tallest < 1000.0, "the tallest painted text is {tallest} pt");
    }

    /// The tooltip says a label whole up to 600 characters, then a head and the count, cut between
    /// characters.
    #[test]
    fn a_tooltip_is_cut_at_600_characters_between_characters() {
        let whole = "w".repeat(TOOLTIP_MAX_CHARS);
        assert_eq!(tooltip_text(&whole), whole);
        let one_more = "w".repeat(TOOLTIP_MAX_CHARS + 1);
        assert_eq!(
            tooltip_text(&one_more),
            format!("{whole}\u{2026} 601 characters")
        );
        let wide = "\u{65e5}".repeat(1234);
        let said = tooltip_text(&wide);
        assert_eq!(
            said,
            format!(
                "{}\u{2026} 1,234 characters",
                "\u{65e5}".repeat(TOOLTIP_MAX_CHARS)
            )
        );
    }

    /// Counts have a thousands separator.
    #[test]
    fn a_count_is_grouped_in_thousands() {
        for (n, said) in [
            (0, "0"),
            (999, "999"),
            (1000, "1,000"),
            (12_345, "12,345"),
            (1_000_000, "1,000,000"),
        ] {
            assert_eq!(grouped(n), said);
        }
    }

    /// The cap is exact: a label of the cap's length is whole, one character more is cut to the cap
    /// and an ellipsis (so the painted text is one character longer than the cap).
    #[test]
    fn a_label_is_cut_from_one_character_past_the_cap() {
        let at = "b".repeat(LABEL_MAX_CHARS);
        assert!(matches!(clipped(&at), Cow::Borrowed(_)));
        assert!(!is_cut(&at));
        let past = "b".repeat(LABEL_MAX_CHARS + 1);
        assert!(is_cut(&past));
        let shown = clipped(&past);
        assert_eq!(shown.chars().count(), LABEL_MAX_CHARS + 1);
        assert_eq!(shown, cut(&past));
        let wide = "\u{e9}".repeat(LABEL_MAX_CHARS + 1);
        assert_eq!(clipped(&wide), cut(&wide), "between characters");
    }

    /// A value too long for three rows paints three and an ellipsis, however long it is: a 4096
    /// character cap alone still made a control 1,328 pt tall in a narrow host.
    #[test]
    fn a_long_value_paints_three_rows_and_an_ellipsis() {
        let (one, _) = first_frame("Rebase", false);
        for label in [
            "Rebase the local commits and keep every merge commit. ".repeat(40),
            "x".repeat(LABEL_MAX_CHARS),
            megabyte(),
        ] {
            let (trigger, said) = first_frame(&label, false);
            let shown = clipped(&label);
            let galley = said
                .iter()
                .find(|g| g.text() == shown)
                .expect("the control says the label's start");
            assert!(galley.elided, "{} chars", label.chars().count());
            assert_eq!(galley.rows.len(), SHOWN_ROWS);
            let line = galley.rows[0].height();
            assert!(
                trigger.height() < one.height() + 3.0 * line,
                "{} pt tall against {} for one row",
                trigger.height(),
                one.height()
            );
        }
    }

    /// #257: an option label of a megabyte is cut in the open list as in the closed control: three
    /// rows, every row as tall as that, the list a few rows tall.
    #[test]
    fn a_megabyte_option_label_keeps_the_open_list_short() {
        let huge = megabyte();
        let options = [(0_u8, "Off"), (1, huge.as_str()), (2, "Daily")];
        let s = scene(&options, None, 0, 400.0, Some(120.0), 0.0, 0.0, 400.0);
        let shown = cut(&huge);
        let listed = saying(&s.open, &shown, s.list);
        assert_eq!(listed.len(), 1, "the list says the start of the label");
        assert!(listed[0].elided);
        assert_eq!(listed[0].rows.len(), SHOWN_ROWS);
        assert!(
            s.open
                .iter()
                .all(|(_, g)| g.text().len() < 2 * LABEL_MAX_CHARS),
            "no galley holds the whole label"
        );
        assert!(
            s.list.height() < 400.0,
            "the list is {} pt tall",
            s.list.height()
        );
        assert!(s.host.contains_rect(s.list));
    }

    /// #257: the row that cuts its label says it, capped, on hover.
    #[test]
    fn a_cut_row_says_its_label_capped_on_hover() {
        let huge = megabyte();
        let options = [(0_u8, "Off"), (1, huge.as_str()), (2, "Daily")];
        let ctx = testkit::ctx();
        let mut value = 0_u8;
        let trigger = std::cell::Cell::new(egui::Rect::NOTHING);
        let mut draw = |ui: &mut Ui| {
            ui.set_width(400.0);
            trigger.set(
                Select::new(&mut value, &options)
                    .width(120.0)
                    .id_salt("row-tip")
                    .show(ui)
                    .rect,
            );
        };
        let mut time = 0.0;
        let mut next = |ctx: &egui::Context, events: Vec<Event>, draw: &mut dyn FnMut(&mut Ui)| {
            time += 0.2;
            frame_at(ctx, time, events, draw)
        };
        for _ in 0..3 {
            next(&ctx, Vec::new(), &mut draw);
        }
        let at = trigger.get().center();
        next(&ctx, testkit::press(at), &mut draw);
        next(&ctx, testkit::release(at), &mut draw);
        let mut painted = next(&ctx, Vec::new(), &mut draw);
        let shown = cut(&huge);
        let row = painted
            .iter()
            .find(|(_, g)| g.text() == shown)
            .expect("the row")
            .0
            .center();
        next(&ctx, vec![Event::PointerMoved(row)], &mut draw);
        for _ in 0..12 {
            painted = next(&ctx, Vec::new(), &mut draw);
        }
        assert!(
            painted.iter().any(|(_, g)| g.text() == capped_tip(&huge)),
            "the tooltip says the first {TOOLTIP_MAX_CHARS} characters and the count"
        );
        let tallest = painted.iter().map(|(_, g)| g.size().y).fold(0.0, f32::max);
        assert!(tallest < 1000.0, "the tallest painted text is {tallest} pt");
    }

    /// A select fitted to its options takes at most the room its host has, and lays out only the
    /// cut text, whatever a label holds.
    #[test]
    fn a_fitted_select_with_a_megabyte_option_stays_inside_its_host() {
        let huge = megabyte();
        let options = [(0_u8, "Off"), (1, huge.as_str())];
        let ctx = testkit::ctx();
        let mut value = 0_u8;
        let mut trigger = egui::Rect::NOTHING;
        let shapes = frame(&ctx, Vec::new(), |ui| {
            ui.set_width(400.0);
            trigger = Select::new(&mut value, &options)
                .fit_to_options()
                .id_salt("fit-huge")
                .show(ui)
                .rect;
        });
        assert!(
            trigger.width() <= 400.5,
            "{} pt wide in a 400 pt host",
            trigger.width()
        );
        assert!(
            galleys(&shapes)
                .iter()
                .all(|(_, g)| g.text().len() < 2 * LABEL_MAX_CHARS)
        );
    }
}
