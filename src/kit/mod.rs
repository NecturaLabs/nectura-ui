//! Canopy's component library. See `docs/design.md`, "Component library".
//!
//! Every component is a struct built with `new(..)` (required inputs only), chained setters that
//! take and return `self`, and a terminal `show(self, ui) -> Response` (or [`KitResponse`] when it
//! returns a value too). Components also implement [`egui::Widget`], so `ui.add(Button::new(..))`
//! works. Common setters keep the same names everywhere: `size`, `icon`, `enabled`, `tooltip`,
//! `disabled_reason`, `shortcut`, `id_salt`, `full_width`, `min_width`.
//!
//! Components read their colours, geometry and options themselves ([`theme::palette`],
//! [`theme::surfaces`], [`theme::metrics`], [`theme::appearance`]); callers never pass colours or
//! sizes. Motion comes from [`anim`] and `ui::motion`, so reduced motion makes every transition
//! instant. Every interactive component reports its AccessKit role and label.
//!
//! The gallery (`--overlay gallery`) shows every component in every state: each module that has
//! components adds a `gallery(ui)` function and registers it in [`gallery::SECTIONS`].

pub mod a11y;
pub mod anim;
pub mod avatar;
pub mod button;
pub mod chip;
pub mod choice;
pub mod field;
pub mod gallery;
pub mod glass;
pub mod menu;
pub mod overlay;
pub mod paint;
pub mod progress;
pub mod range;
pub mod row;
pub mod select;
pub mod shortcut;
pub mod split;
pub mod surface;
pub mod tabs;
pub mod toast;
pub mod toggle;

#[cfg(any(test, feature = "testkit"))]
pub mod testkit;

pub use button::{Button, ButtonKind, HelpMark, IconButton, ToolButton};
pub use shortcut::Shortcut;

use egui::{Color32, Id, Response};

use super::theme::{self, Palette};

/// Control size. `Md` is the default; `Lg` is for welcome calls to action only; `Xs` exists for
/// icon buttons inside other controls (a field's clear button).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Size {
    Xs,
    Sm,
    #[default]
    Md,
    Lg,
}

impl Size {
    pub const ALL: [Self; 4] = [Self::Xs, Self::Sm, Self::Md, Self::Lg];

    /// The control height of this size in the active metrics.
    pub fn height(self) -> f32 {
        let m = theme::metrics();
        match self {
            Self::Xs => 20.0_f32.min(m.control_sm),
            Self::Sm => m.control_sm,
            Self::Md => m.control,
            Self::Lg => m.control_lg,
        }
    }
}

/// The meaning a coloured component carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tone {
    #[default]
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
}

impl Tone {
    pub const ALL: [Self; 5] = [
        Self::Neutral,
        Self::Accent,
        Self::Success,
        Self::Warning,
        Self::Danger,
    ];

    /// The text-grade colour of the tone.
    pub fn color(self, p: &Palette) -> Color32 {
        match self {
            Self::Neutral => p.text_dim,
            Self::Accent => p.accent,
            Self::Success => p.success,
            Self::Warning => p.warning,
            Self::Danger => p.danger,
        }
    }

    /// The tone's 16% tint (banners, callouts, dialog glyph discs).
    pub fn tint(self, p: &Palette) -> Color32 {
        match self {
            Self::Neutral => p.hover_strong(),
            Self::Accent => p.accent_tint(),
            Self::Success => p.success_tint(),
            Self::Warning => p.warning_tint(),
            Self::Danger => p.danger_tint(),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Neutral => "Neutral",
            Self::Accent => "Accent",
            Self::Success => "Success",
            Self::Warning => "Warning",
            Self::Danger => "Danger",
        }
    }
}

/// A component's response and the value it produced (a chosen option, an opened menu entry).
#[derive(Debug)]
pub struct KitResponse<T> {
    pub response: Response,
    pub value: T,
}

/// States forced on a component for the gallery and the kit's own tests, so a screenshot shows
/// hover, press, focus or an open popup without a pointer. Ordinary code never passes one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ForceState {
    pub hover: bool,
    pub pressed: bool,
    pub focus: bool,
    pub open: bool,
    /// A fixed transition progress (0 to 1) instead of the animated one.
    pub progress: Option<f32>,
}

impl ForceState {
    pub const HOVER: Self = Self {
        hover: true,
        pressed: false,
        focus: false,
        open: false,
        progress: None,
    };
    pub const PRESSED: Self = Self {
        hover: true,
        pressed: true,
        focus: false,
        open: false,
        progress: None,
    };
    pub const FOCUS: Self = Self {
        hover: false,
        pressed: false,
        focus: true,
        open: false,
        progress: None,
    };
    pub const OPEN: Self = Self {
        hover: false,
        pressed: false,
        focus: false,
        open: true,
        progress: None,
    };
}

/// The outline and indicator of a disabled selection control (a checkbox's rim and tick ground, a
/// switch's knob, a slider's fill and thumb ring): halfway between `disabled_edge` and
/// `disabled_text`, so the control keeps its shape and its value without reading as enabled.
pub(crate) fn disabled_mark(p: &Palette, s: &theme::Surfaces) -> Color32 {
    theme::mix(s.disabled_edge, p.disabled_text, 0.5)
}

/// The painter a control draws its labels and icons with: `ui`'s, with egui's own disabled fade
/// taken back off when the `Ui` itself is disabled, so a disabled label paints in
/// [`Palette::disabled_text`] at full strength either way.
pub(crate) fn content_painter(ui: &egui::Ui) -> egui::Painter {
    let mut painter = ui.painter().clone();
    if !ui.is_enabled() {
        let fade = ui.visuals().disabled_alpha();
        if fade > 0.0 {
            painter.set_opacity((painter.opacity() / fade).min(1.0));
        }
    }
    painter
}

/// Whether keyboard focus should be drawn: true after a key press, false again after a pointer
/// press (CSS `:focus-visible`; moving the pointer keeps it). Text fields show focus whatever this
/// says.
///
/// The answer follows every frame's input, not only the frames somebody asked in: a pointer press
/// in a frame where no focused widget looked (a right click that opens a context menu) still
/// counts.
pub fn focus_visible(ctx: &egui::Context) -> bool {
    let id = Id::new("canopy-kit-focus-visible");
    if ctx.data_mut(|d| d.get_temp::<bool>(id.with("hooked")).is_none()) {
        ctx.data_mut(|d| d.insert_temp(id.with("hooked"), true));
        ctx.on_begin_pass(
            "canopy-kit-focus-visible",
            std::sync::Arc::new(|ui: &mut egui::Ui| {
                focus_visible_now(ui.ctx());
            }),
        );
    }
    focus_visible_now(ctx)
}

/// Draws keyboard focus from now on, until the next pointer press: a dialog opening with its
/// default button focused shows which button Enter will press.
pub(crate) fn show_focus(ctx: &egui::Context) {
    focus_visible(ctx);
    ctx.data_mut(|d| d.insert_temp(Id::new("canopy-kit-focus-visible"), true));
}

/// [`focus_visible`] for this frame's input.
fn focus_visible_now(ctx: &egui::Context) -> bool {
    let id = Id::new("canopy-kit-focus-visible");
    let (key, pointer) = ctx.input(|i| {
        (
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key {
                        pressed: true,
                        key,
                        ..
                    } if matches!(key, egui::Key::Tab | egui::Key::ArrowUp | egui::Key::ArrowDown
                        | egui::Key::ArrowLeft | egui::Key::ArrowRight | egui::Key::Space
                        | egui::Key::Enter | egui::Key::Escape | egui::Key::Home | egui::Key::End)
                )
            }),
            i.pointer.any_pressed(),
        )
    });
    ctx.data_mut(|d| {
        let visible = d.get_temp_mut_or(id, false);
        if key {
            *visible = true;
        } else if pointer {
            *visible = false;
        }
        *visible
    })
}

/// What a component paints from: eased hover, press and focus, with the forced states applied.
#[derive(Debug, Clone, Copy)]
pub struct Interact {
    /// Hover, eased in over `FAST` and out over `FAST_OUT` (0 to 1).
    pub hover: f32,
    /// Press, on at once and eased off over `FAST` (0 to 1).
    pub press: f32,
    /// Whether the press is held now (for the press scale).
    pub held: bool,
    /// Keyboard focus that should be drawn, eased in over `FAST`, out at once.
    pub focus: f32,
}

/// Whether a press that ended is still shown ([`super::motion::PRESS_MIN`] after release), so a
/// quick click is seen. Always false with reduced motion. While it lingers, a repaint is asked
/// for at its end.
fn lingering_press(ctx: &egui::Context, id: Id, down: bool, clicked: bool) -> bool {
    let key = id.with("kit-press-since");
    let down_key = id.with("kit-press-down");
    let now = ctx.input(|i| i.time);
    let since = ctx.data_mut(|d| {
        if down {
            d.insert_temp(down_key, true);
            return None;
        }
        if d.get_temp::<bool>(down_key).unwrap_or(false) || clicked {
            d.remove::<bool>(down_key);
            d.insert_temp(key, now);
        }
        d.get_temp::<f64>(key)
    });
    let Some(since) = since else {
        return false;
    };
    let left = f64::from(super::motion::time(super::motion::PRESS_MIN)) - (now - since);
    if left > 0.0 {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(left));
        return true;
    }
    ctx.data_mut(|d| d.remove::<f64>(key));
    false
}

impl Interact {
    pub fn of(
        ctx: &egui::Context,
        response: &Response,
        enabled: bool,
        force: Option<ForceState>,
    ) -> Self {
        let f = force.unwrap_or_default();
        let id = response.id;
        let hovered = enabled && (f.hover || response.hovered());
        let down = f.pressed || (enabled && response.is_pointer_button_down_on());
        let lingering =
            force.is_none() && lingering_press(ctx, id, down, enabled && response.clicked());
        let held = (enabled && down) || lingering;
        let focused = f.focus || (response.has_focus() && focus_visible(ctx));
        if force.is_some() {
            // A forced progress shows the transition part-way (the gallery's motion strip).
            let k = f.progress.unwrap_or(1.0);
            let on = |b: bool| f32::from(u8::from(b)) * k;
            return Self {
                hover: on(hovered),
                press: on(held),
                held,
                focus: on(focused),
            };
        }
        Self {
            hover: anim::ease_bool(
                ctx,
                id.with("kit-hover"),
                hovered,
                super::motion::FAST,
                super::motion::FAST_OUT,
            ),
            press: anim::ease_bool(
                ctx,
                id.with("kit-press"),
                held,
                super::motion::PRESS_IN,
                super::motion::FAST,
            ),
            held,
            focus: anim::ease_bool(ctx, id.with("kit-focus"), focused, super::motion::FAST, 0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::Ui;
    use egui_phosphor::regular as icon;

    use super::testkit;
    use super::theme::{self, Density, Style, ThemeId};
    use super::{chip, choice, field, menu, range, row, select, toggle};

    /// usersim F19: a pointer press ends visible focus even in a frame where no focused widget
    /// asked (a right click that opens a context menu), so the menu does not show its first item
    /// as keyboard focused.
    /// A click that is over within a frame or two still shows as a press for `PRESS_MIN`.
    #[test]
    fn a_quick_press_stays_visible_for_a_moment() {
        let ctx = egui::Context::default();
        let id = egui::Id::new("quick");
        let at = |t: f64, down: bool| {
            let mut shown = false;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    time: Some(t),
                    ..Default::default()
                },
                |_| shown = super::lingering_press(&ctx, id, down, false),
            );
            out.textures_delta.clear();
            shown
        };
        crate::motion::set_reduced(false);
        assert!(
            !at(1.0, true),
            "while down, the press is shown by being down"
        );
        assert!(at(1.03, false), "just released");
        assert!(!at(1.5, false), "gone after the minimum");
        assert!(!at(1.6, false));
    }

    #[test]
    fn a_pointer_press_lingers_through_the_widget_interaction() {
        let ctx = egui::Context::default();
        crate::motion::set_reduced(false);
        let at = |time, events| {
            let mut held = false;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response =
                        ui.allocate_response(egui::vec2(100.0, 30.0), egui::Sense::click());
                    held = super::Interact::of(&ctx, &response, true, None).held;
                },
            );
            out.textures_delta.clear();
            held
        };
        let pos = egui::pos2(15.0, 15.0);
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        assert!(!at(0.9, vec![egui::Event::PointerMoved(pos)]));
        assert!(at(1.0, vec![button(true)]), "the pointer holds the widget");
        assert!(at(1.03, vec![button(false)]), "release remains visible");
        assert!(!at(1.5, vec![]), "the press expires");
    }

    #[test]
    fn quick_clicks_remain_visible_when_the_action_becomes_disabled() {
        for same_frame in [false, true] {
            let ctx = egui::Context::default();
            crate::motion::set_reduced(false);
            let pos = egui::pos2(15.0, 15.0);
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let at = |time, events, enabled| {
                let mut held = false;
                let mut clicked = false;
                let mut out = ctx.run_ui(
                    egui::RawInput {
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let sense = if enabled {
                            egui::Sense::click()
                        } else {
                            egui::Sense::hover()
                        };
                        let response = ui.allocate_response(egui::vec2(100.0, 30.0), sense);
                        held = super::Interact::of(&ctx, &response, enabled, None).held;
                        clicked = response.clicked();
                    },
                );
                out.textures_delta.clear();
                (held, clicked)
            };
            assert_eq!(
                at(0.9, vec![egui::Event::PointerMoved(pos)], true),
                (false, false)
            );
            let events = if same_frame {
                vec![button(true), button(false)]
            } else {
                vec![button(true)]
            };
            assert!(
                at(1.0, events, true).0,
                "the click reads as a press: same_frame={same_frame}"
            );
            let events = if same_frame {
                vec![]
            } else {
                vec![button(false)]
            };
            assert_eq!(
                at(1.03, events, false),
                (true, false),
                "the inert action keeps its feedback: same_frame={same_frame}"
            );
            let released_at = if same_frame { 1.0 } else { 1.03 };
            assert_eq!(
                at(released_at + 0.08, vec![], false),
                (true, false),
                "the minimum is measured from release"
            );
            assert_eq!(at(released_at + 0.12, vec![], false), (false, false));
        }
    }

    #[test]
    fn a_pointer_press_hides_focus_though_nobody_asked_that_frame() {
        let ctx = testkit::ctx();
        let key = egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let press = egui::Event::PointerButton {
            pos: egui::pos2(10.0, 10.0),
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        let mut asked = Vec::new();
        testkit::frame(&ctx, vec![key], |ui| {
            asked.push(super::focus_visible(ui.ctx()));
        });
        // The press frame: nothing asks.
        testkit::frame(&ctx, vec![press], |_| {});
        testkit::frame(&ctx, Vec::new(), |ui| {
            asked.push(super::focus_visible(ui.ctx()));
        });
        assert_eq!(
            asked,
            [true, false],
            "a key shows focus, the press hides it"
        );
    }

    #[test]
    fn disabled_text_reads_on_every_control_surface() {
        for id in ThemeId::ALL {
            let p = id.palette();
            for style in Style::ALL {
                let s = p.surfaces(style);
                let c = p.disabled_text;
                for g in [s.pane, s.panel, s.elevated, s.overlay, s.field] {
                    let dimmed = theme::over(s.raised.top.gamma_multiply(theme::DISABLED_ALPHA), g);
                    for ground in [g, dimmed] {
                        let k = theme::contrast(c, ground);
                        assert!(k >= 3.0, "{id:?} {style:?}: {k:.2}:1 on {ground:?}");
                    }
                }
                assert!(
                    theme::contrast(c, s.pane) < theme::contrast(p.text_dim, s.pane),
                    "{id:?} {style:?}: a disabled label must read dimmer than an enabled one"
                );
            }
        }
    }

    #[test]
    fn disabled_controls_keep_a_readable_label_in_every_theme() {
        type Show = fn(&mut Ui);
        let controls: [(&str, Show); 11] = [
            ("slider", |ui| {
                range::Slider::new(&mut 3.0_f32, 0.0..=10.0)
                    .label("Context lines")
                    .header(true)
                    .enabled(false)
                    .show(ui);
            }),
            ("underline tabs", |ui| {
                let _ = choice::UnderlineTabs::new(&mut 0_u8, &[(0, "Privacy"), (1, "Terms")])
                    .enabled(false)
                    .show(ui);
            }),
            ("segmented", |ui| {
                let mut v = 0_u8;
                choice::Segmented::new(&mut v, &[(0, "Unified"), (1, "Side by side")])
                    .enabled(false)
                    .show(ui);
            }),
            ("checkbox", |ui| {
                toggle::Checkbox::new(&mut true, "Amend last commit")
                    .enabled(false)
                    .show(ui);
            }),
            ("radio", |ui| {
                toggle::Radio::new(true, "Fast-forward")
                    .enabled(false)
                    .show(ui);
            }),
            ("switch", |ui| {
                toggle::Switch::new(&mut true, "Prune on fetch")
                    .enabled(false)
                    .show(ui);
            }),
            ("select", |ui| {
                let mut v = 0_u8;
                select::Select::new(&mut v, &[(0, "Every 5 minutes")])
                    .enabled(false)
                    .show(ui);
            }),
            ("list row", |ui| {
                row::ListRow::new("disabled-row")
                    .icon(icon::FILE_TEXT)
                    .label("Disabled")
                    .detail("src/ui")
                    .trailing("+4")
                    .enabled(false)
                    .show(ui);
            }),
            ("tree row", |ui| {
                let _ = row::TreeRow::new("disabled-tree", 0)
                    .icon(icon::GIT_BRANCH)
                    .label("main")
                    .enabled(false)
                    .show(ui);
            }),
            ("chip", |ui| {
                chip::Chip::new("Both modified").enabled(false).show(ui);
            }),
            ("text field", |ui| {
                field::TextField::new(&mut "ada@example.com".to_owned())
                    .label("Email")
                    .enabled(false)
                    .show(ui);
            }),
        ];
        for id in ThemeId::ALL {
            for style in Style::ALL {
                let ctx = testkit::ctx_with(id, style, Density::Comfortable);
                let s = id.palette().surfaces(style);
                let grounds = [s.pane, s.panel, s.overlay];
                for (name, show) in controls {
                    let w = testkit::weakest_label(&ctx, &grounds, show);
                    assert!(w >= 3.0, "{id:?} {style:?} disabled {name}: {w:.2}:1");
                }
            }
        }
    }

    /// Every colour `shapes` painted: fills, strokes and text, with the painter's opacity applied.
    fn every_color(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Color32> {
        fn walk(shape: &egui::Shape, out: &mut Vec<egui::Color32>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                egui::Shape::Rect(r) if r.blur_width == 0.0 => {
                    out.extend([r.fill, r.stroke.color]);
                }
                egui::Shape::Circle(c) => out.extend([c.fill, c.stroke.color]),
                egui::Shape::Path(p) => {
                    if let egui::epaint::ColorMode::Solid(c) = p.stroke.color {
                        out.push(c);
                    }
                    out.push(p.fill);
                }
                egui::Shape::LineSegment { stroke, .. } => out.push(stroke.color),
                egui::Shape::Text(t) => out.push(
                    t.override_text_color
                        .unwrap_or(t.fallback_color)
                        .gamma_multiply(t.opacity_factor),
                ),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for c in shapes {
            walk(&c.shape, &mut out);
        }
        out.retain(|c| c.a() > 0);
        out
    }

    /// H11 (theme-design "Disabled tokens"): a disabled control paints with `disabled_fill`,
    /// `disabled_edge` and `disabled_text` at full strength, whether it is disabled itself or sits
    /// in a disabled `Ui`: nothing it paints is faded by `DISABLED_ALPHA`, no accent or danger
    /// fill shows through, and its label is `disabled_text`.
    #[test]
    fn disabled_controls_paint_the_disabled_tokens_at_full_strength() {
        type Show = fn(&mut Ui, bool);
        let controls: [(&str, Show); 17] = [
            ("menu item", |ui, on| {
                menu::MenuItem::new(icon::GIT_BRANCH, "Check out")
                    .description("Switch to this branch")
                    .shortcut("f")
                    .enabled(on)
                    .show(ui);
            }),
            ("danger menu item", |ui, on| {
                menu::MenuItem::new(icon::TRASH, "Delete branch…")
                    .danger()
                    .enabled(on)
                    .show(ui);
            }),
            ("underline tabs", |ui, on| {
                let _ = choice::UnderlineTabs::new(&mut 0_u8, &[(0, "Privacy"), (1, "Terms")])
                    .enabled(on)
                    .show(ui);
            }),
            ("primary", |ui, on| {
                super::Button::new("Commit").primary().enabled(on).show(ui);
            }),
            ("danger", |ui, on| {
                super::Button::new("Delete").danger().enabled(on).show(ui);
            }),
            ("secondary", |ui, on| {
                super::Button::new("Fetch").enabled(on).show(ui);
            }),
            ("icon button", |ui, on| {
                super::IconButton::new(icon::GEAR, "Settings")
                    .enabled(on)
                    .show(ui);
            }),
            ("tool button", |ui, on| {
                super::ToolButton::new(icon::TAG, "Tag")
                    .enabled(on)
                    .show(ui);
            }),
            ("split button", |ui, on| {
                let mut v = 0_u8;
                super::button::SplitButton::new(&mut v, &[(0, "Commit"), (1, "Push")])
                    .enabled(on)
                    .show(ui);
            }),
            ("checkbox", |ui, on| {
                toggle::Checkbox::new(&mut true, "Amend")
                    .enabled(on)
                    .show(ui);
            }),
            ("radio", |ui, on| {
                toggle::Radio::new(true, "Fast-forward")
                    .enabled(on)
                    .show(ui);
            }),
            ("switch", |ui, on| {
                toggle::Switch::new(&mut true, "Prune").enabled(on).show(ui);
            }),
            ("slider", |ui, on| {
                range::Slider::new(&mut 6.0_f32, 0.0..=10.0)
                    .label("Context lines")
                    .header(true)
                    .enabled(on)
                    .show(ui);
            }),
            ("segmented", |ui, on| {
                choice::Segmented::new(&mut 0_u8, &[(0, "Unified"), (1, "Side by side")])
                    .enabled(on)
                    .show(ui);
            }),
            ("chip", |ui, on| {
                chip::Chip::new("main").enabled(on).show(ui);
            }),
            ("danger chip", |ui, on| {
                chip::Chip::new("3 failing")
                    .icon(icon::X_CIRCLE)
                    .tone(super::Tone::Danger)
                    .enabled(on)
                    .show(ui);
            }),
            ("card", |ui, on| {
                super::surface::Card::new()
                    .interactive(true)
                    .selected(true)
                    .ring(super::Tone::Accent)
                    .enabled(on)
                    .show(ui, |ui| super::Button::new("Open").primary().show(ui));
            }),
        ];
        for id in ThemeId::ALL {
            for style in Style::ALL {
                let ctx = testkit::ctx_with(id, style, Density::Comfortable);
                let p = id.palette();
                let s = p.surfaces(style);
                let accents = [
                    s.primary.top,
                    s.primary.bottom,
                    s.danger_solid.top,
                    s.danger_solid.bottom,
                ];
                for (name, show) in controls {
                    for in_disabled_ui in [false, true] {
                        let shapes = testkit::frame(&ctx, Vec::new(), |ui| {
                            if in_disabled_ui {
                                ui.add_enabled_ui(false, |ui| show(ui, true));
                            } else {
                                show(ui, false);
                            }
                        });
                        let colors = every_color(&shapes);
                        let at = format!("{id:?} {style:?} {name} (disabled Ui: {in_disabled_ui})");
                        assert!(
                            colors.contains(&p.disabled_text),
                            "{at}: label not in disabled_text: {colors:?}"
                        );
                        for c in &colors {
                            assert_eq!(c.a(), 255, "{at}: faded paint {c:?}");
                            assert!(
                                !accents.contains(c),
                                "{at}: an accent or danger fill shows through: {c:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Kit components show the kit tooltip (`overlay::TooltipExt`), never egui's stock one, so
    /// every tooltip looks and moves the same.
    #[test]
    fn the_kit_uses_its_own_tooltip() {
        const STOCK: [&str; 5] = [
            ".on_hover_text(",
            ".on_disabled_hover_text(",
            ".on_hover_text_at_pointer(",
            ".on_hover_ui(",
            ".on_hover_ui_at_pointer(",
        ];
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let files: Vec<_> = std::fs::read_dir(src.join("kit"))
            .expect("kit folder")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "rs"))
            .collect();
        let mut found = Vec::new();
        for path in files {
            let text = std::fs::read_to_string(&path).expect("kit source");
            let code = text.split("#[cfg(test)]").next().unwrap_or_default();
            for (i, line) in code.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                for stock in STOCK {
                    if line.contains(stock) {
                        found.push(format!("{}:{}: {stock}", path.display(), i + 1));
                    }
                }
            }
        }
        assert!(found.is_empty(), "stock tooltips:\n{}", found.join("\n"));
    }
}
