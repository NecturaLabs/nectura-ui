//! Helpers the kit's own tests share: a themed context, one frame with input events, keyboard and
//! pointer events, and the AccessKit nodes a frame produced.

#![expect(
    clippy::missing_panics_doc,
    reason = "test helpers panic on a failed expectation; their docs say what they do"
)]

use egui::accesskit::{Node, NodeId, Role, Toggled};
use egui::{Color32, Context, Event, Key, Modifiers, PointerButton, Pos2, RawInput, Shape, Ui};

use crate::theme::{self, Appearance, Density, Style, ThemeId};

/// A context with Canopy's fonts and `theme` in `style` and `density`, motion off.
pub fn ctx_with(theme_id: ThemeId, style: Style, density: Density) -> Context {
    let ctx = Context::default();
    crate::fonts::install(&ctx);
    theme::apply(
        &ctx,
        theme_id,
        Appearance {
            style,
            density,
            ..Appearance::STILL
        },
    );
    ctx
}

/// Midnight, Standard, Comfortable, motion off.
pub fn ctx() -> Context {
    ctx_with(ThemeId::Midnight, Style::Standard, Density::Comfortable)
}

/// Runs one frame with `events`; returns what it painted.
pub fn frame(
    ctx: &Context,
    events: Vec<Event>,
    mut f: impl FnMut(&mut Ui),
) -> Vec<egui::epaint::ClippedShape> {
    let mut out = ctx.run_ui(
        RawInput {
            events,
            ..Default::default()
        },
        |ui| f(ui),
    );
    out.textures_delta.clear();
    out.shapes
}

/// Painted colours and where they are.
pub type Found = Vec<(egui::Rect, Color32)>;

/// What `shapes` painted, flattened and in paint order: the flat fills of unblurred rects and the
/// text, each with where it is and its colour with the painter's opacity applied (premultiplied,
/// ready for `theme::over`). Gradient fills (a textured rect) are left out: their colours live in
/// the texture.
pub fn painted_colors(shapes: &[egui::epaint::ClippedShape]) -> (Found, Found) {
    fn walk(shape: &Shape, fills: &mut Found, inks: &mut Found) {
        match shape {
            Shape::Vec(v) => v.iter().for_each(|s| walk(s, fills, inks)),
            Shape::Rect(r) if r.blur_width == 0.0 && r.fill.a() > 0 && r.brush.is_none() => {
                fills.push((r.rect, r.fill));
            }
            Shape::Text(t) => inks.push((
                t.visual_bounding_rect(),
                t.override_text_color
                    .unwrap_or(t.fallback_color)
                    .gamma_multiply(t.opacity_factor),
            )),
            _ => {}
        }
    }
    let (mut fills, mut inks) = (Vec::new(), Vec::new());
    for c in shapes {
        walk(&c.shape, &mut fills, &mut inks);
    }
    (fills, inks)
}

/// The weakest contrast of any text `f` paints, over the flat fills painted under it, on each of
/// `grounds`.
pub fn weakest_label(ctx: &Context, grounds: &[Color32], f: impl FnMut(&mut Ui)) -> f64 {
    let shapes = frame(ctx, Vec::new(), f);
    let (fills, inks) = painted_colors(&shapes);
    assert!(!inks.is_empty(), "nothing was written");
    let mut weakest = f64::INFINITY;
    for g in grounds {
        for (at, ink) in &inks {
            let under = fills
                .iter()
                .filter(|(r, _)| r.contains(at.center()))
                .fold(*g, |acc, (_, fill)| theme::over(*fill, acc));
            weakest = weakest.min(theme::contrast(theme::over(*ink, under), under));
        }
    }
    weakest
}

/// A key press and its release.
pub fn key(k: Key) -> Vec<Event> {
    key_with(k, Modifiers::NONE)
}

pub fn key_with(k: Key, modifiers: Modifiers) -> Vec<Event> {
    vec![
        Event::Key {
            key: k,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        },
        Event::Key {
            key: k,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        },
    ]
}

/// The pointer moving to `pos` and pressing there.
pub fn press(pos: Pos2) -> Vec<Event> {
    vec![
        Event::PointerMoved(pos),
        Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        },
    ]
}

/// The pointer releasing at `pos`.
pub fn release(pos: Pos2) -> Vec<Event> {
    vec![Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]
}

/// The pointer moving to `pos` and pressing and releasing the secondary button there.
pub fn secondary_click(pos: Pos2) -> Vec<Event> {
    let button = |pressed| Event::PointerButton {
        pos,
        button: PointerButton::Secondary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    vec![Event::PointerMoved(pos), button(true), button(false)]
}

/// What the app copied to the clipboard in the frame of `f`.
pub fn copied(ctx: &Context, events: Vec<Event>, mut f: impl FnMut(&mut Ui)) -> Vec<String> {
    let out = ctx.run_ui(
        RawInput {
            events,
            ..Default::default()
        },
        |ui| f(ui),
    );
    out.platform_output
        .commands
        .into_iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text),
            _ => None,
        })
        .collect()
}

/// A full click at `pos` over two frames of `f`; the second frame's result is returned.
pub fn click(ctx: &Context, pos: Pos2, mut f: impl FnMut(&mut Ui)) {
    frame(ctx, press(pos), &mut f);
    frame(ctx, release(pos), &mut f);
}

/// One AccessKit node as the tests read it.
#[derive(Debug, Clone)]
pub struct Spoken {
    pub role: Role,
    pub label: Option<String>,
    pub toggled: Option<Toggled>,
    pub selected: Option<bool>,
    pub expanded: Option<bool>,
    pub value: Option<String>,
    pub numeric: Option<f64>,
    pub disabled: bool,
    pub description: Option<String>,
    /// Marked invalid (a field with an error).
    pub invalid: bool,
    /// Where it is, in points.
    pub bounds: Option<egui::Rect>,
}

impl Spoken {
    fn of(n: &Node) -> Self {
        Self {
            role: n.role(),
            label: n.label().map(str::to_owned),
            toggled: n.toggled(),
            selected: n.is_selected(),
            expanded: n.is_expanded(),
            value: n.value().map(str::to_owned),
            numeric: n.numeric_value(),
            disabled: n.is_disabled(),
            description: n.description().map(str::to_owned),
            invalid: n.invalid().is_some(),
            bounds: n.bounds().map(|b| {
                egui::Rect::from_min_max(
                    egui::pos2(crate::to_f32(b.x0), crate::to_f32(b.y0)),
                    egui::pos2(crate::to_f32(b.x1), crate::to_f32(b.y1)),
                )
            }),
        }
    }
}

/// The AccessKit nodes one frame of `f` produced.
pub fn spoken(ctx: &Context, f: impl FnMut(&mut Ui)) -> Vec<Spoken> {
    spoken_with(ctx, Vec::new(), f)
}

/// [`spoken`] for a frame that gets `events`.
pub fn spoken_with(ctx: &Context, events: Vec<Event>, mut f: impl FnMut(&mut Ui)) -> Vec<Spoken> {
    ctx.enable_accesskit();
    let mut out = ctx.run_ui(
        RawInput {
            events,
            ..Default::default()
        },
        |ui| f(ui),
    );
    out.textures_delta.clear();
    out.platform_output
        .accesskit_update
        .map(|u| u.nodes.iter().map(|(_, n)| Spoken::of(n)).collect())
        .unwrap_or_default()
}

/// The names of the parts of the node labelled `group` after one frame of `f`: its named
/// descendants, looking through unnamed containers (what a screen reader lists inside it).
pub fn parts(ctx: &Context, group: &str, mut f: impl FnMut(&mut Ui)) -> Vec<String> {
    ctx.enable_accesskit();
    let mut out = ctx.run_ui(RawInput::default(), |ui| f(ui));
    out.textures_delta.clear();
    let Some(update) = out.platform_output.accesskit_update else {
        return Vec::new();
    };
    let nodes: std::collections::HashMap<NodeId, &Node> =
        update.nodes.iter().map(|(id, n)| (*id, n)).collect();
    let mut names = Vec::new();
    // A select's trigger and its list share a name: the group is the one with parts.
    let mut todo: Vec<NodeId> = nodes
        .values()
        .find(|n| n.label() == Some(group) && !n.children().is_empty())
        .map(|g| g.children().iter().rev().copied().collect())
        .unwrap_or_default();
    while let Some(id) = todo.pop() {
        let Some(n) = nodes.get(&id) else { continue };
        // A text label's words are its value (egui), what a screen reader reads for it.
        let name = if n.role() == Role::Label {
            n.value()
        } else {
            n.label()
        };
        match name {
            Some(l) => names.push(l.to_owned()),
            None => todo.extend(n.children().iter().rev().copied()),
        }
    }
    names
}

/// The node labelled `label`, if one was spoken.
pub fn find<'a>(nodes: &'a [Spoken], label: &str) -> Option<&'a Spoken> {
    nodes.iter().find(|n| n.label.as_deref() == Some(label))
}

/// How many times `text` is painted once the pointer has rested on `show`'s widget (at
/// `at`) for longer than the tooltip delay.
pub fn painted_after_hover(at: Pos2, text: &str, mut show: impl FnMut(&mut Ui)) -> usize {
    fn count(shape: &Shape, text: &str) -> usize {
        match shape {
            Shape::Text(t) => usize::from(t.galley.text() == text),
            Shape::Vec(v) => v.iter().map(|s| count(s, text)).sum(),
            _ => 0,
        }
    }
    let ctx = ctx();
    let mut shown = 0;
    for i in 0..12 {
        let events = if i == 0 {
            vec![Event::PointerMoved(at)]
        } else {
            Vec::new()
        };
        let mut out = ctx.run_ui(
            RawInput {
                time: Some(f64::from(i) * 0.2),
                events,
                ..Default::default()
            },
            |ui| show(ui),
        );
        out.textures_delta.clear();
        shown = out.shapes.iter().map(|c| count(&c.shape, text)).sum();
    }
    shown
}
