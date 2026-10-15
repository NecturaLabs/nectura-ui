//! Painting primitives the kit's components share: layered shadows, gradient fills clipped to a
//! rounded rect, the inner top highlight and bottom line, the field recess, focus rings, 1 pt
//! seams snapped to the pixel grid, Deeper's floating panes with their focus edge, the top deck's
//! bars, edge and scroll fades, and soft radial glows. Colours come in from the caller, which reads
//! them from the theme.

use egui::epaint::tessellator::path::rounded_rectangle;
use egui::epaint::{CornerRadiusF32, PathShape, PathStroke, RectShape};
use egui::{
    Color32, Context, CornerRadius, Id, Painter, Pos2, Rangef, Rect, Shape, Stroke, StrokeKind,
    TextureHandle, TextureOptions, pos2, vec2,
};

use crate::theme::{
    self, Design, EdgeMode, Elevation, Fill, FocusStyle, Ground, HoverStyle, Material, Palette,
    PaneSeparation, SelectionStyle, Style, Surfaces,
};

/// Width of a seam, edge or ring (1 pt, as concept 1's 1 px borders). Painted seams, rims and
/// control rings use [`edge`], the active theme's width (heavier at high contrast).
pub const SEAM: f32 = 1.0;
/// Inset focus ring of rows, tabs, segments and menu items.
pub const INSET_FOCUS_WIDTH: f32 = 1.5;
/// Stroke width of a spinner's arc.
pub const SPINNER_WIDTH: f32 = 1.6;

/// The active theme's structure.
fn design() -> &'static Design {
    theme::active_design()
}

/// Width of seams, card edges and control rings in the active theme (`Design::edge_width`).
pub fn edge() -> f32 {
    design().edge_width
}

/// The seam between Standard's flush panes (sidebar, content, details) in the active theme
/// (`Design::panes`): a `hairline` (`Surfaces::border`) or a `control_border` rule (`Outline`).
pub fn pane_seam(p: &Palette, hairline: Color32) -> Stroke {
    let d = design();
    match d.panes {
        PaneSeparation::Outline => Stroke::new(d.edge_width, p.control_border),
        PaneSeparation::Seams => Stroke::new(SEAM, hairline),
    }
}

/// The seam under a header or between two bars in the active theme (`Design::edges`): a
/// `hairline` in its colour or a `control_border` rule (`Outline`).
pub fn edge_seam(p: &Palette, hairline: Color32) -> Stroke {
    let d = design();
    match d.edges {
        EdgeMode::Outline => Stroke::new(d.edge_width, p.control_border),
        EdgeMode::Hairline => Stroke::new(SEAM, hairline),
    }
}

/// Paints `seam` along `y` across `x`, snapped to the pixel grid.
pub fn seam_line_h(painter: &Painter, x: Rangef, y: f32, seam: Stroke) {
    let mut y = y;
    seam.round_center_to_pixel(painter.pixels_per_point(), &mut y);
    painter.hline(x, y, seam);
}

/// Paints `seam` along `x` down `y`, snapped to the pixel grid.
pub fn seam_line_v(painter: &Painter, x: f32, y: Rangef, seam: Stroke) {
    let mut x = x;
    seam.round_center_to_pixel(painter.pixels_per_point(), &mut x);
    painter.vline(x, y, seam);
}

/// A row's selection mark over its wash, in the active theme's [`SelectionStyle`]: the leading
/// bar, the ring or the inner glow (`Wash` adds nothing).
pub fn selection_mark(painter: &Painter, rect: Rect, radius: CornerRadius, p: &Palette) {
    match design().selection {
        SelectionStyle::Wash => {}
        SelectionStyle::Bar { width } => {
            let inset = (f32::from(radius.nw).max(2.0)).min(rect.height() / 4.0);
            let bar = Rect::from_min_max(
                pos2(rect.left(), rect.top() + inset),
                pos2(rect.left() + width, rect.bottom() - inset),
            );
            painter.rect_filled(
                bar,
                CornerRadius::same(crate::saturate_u8((width / 2.0).round())),
                p.focus,
            );
        }
        SelectionStyle::Outline { width } => {
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(width, Palette::tint(p.focus, 179)),
                StrokeKind::Inside,
            );
        }
    }
}

/// What a hovered row or quiet control adds over its hover wash in the active theme's
/// [`HoverStyle`], at strength `t` (the hover's ease): `Edge` a 1 px ring. `Tint` adds nothing.
pub fn hover_mark(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    p: &Palette,
    s: &Surfaces,
    t: f32,
) {
    if t <= 0.0 {
        return;
    }
    match design().hover {
        HoverStyle::Tint => {}
        HoverStyle::Edge => {
            let c = if p.strict { p.control_border } else { s.edge };
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, c.gamma_multiply(t)),
                StrokeKind::Inside,
            );
        }
    }
}

/// The layers of `material` under a surface at `rect`, largest first. Paint before the surface.
/// A negative spread shrinks the rect (egui's `Shadow::spread` is unsigned); a layer whose rect is
/// too small to hold its blur grows to hold it and fades by as much, so it stays soft.
pub fn shadow(painter: &Painter, rect: Rect, radius: CornerRadius, p: &Palette, m: &Material) {
    painter.extend(shadow_shapes(rect, radius, p, m));
}

/// The shapes [`shadow`] paints, for surfaces painted into a reserved slot once their size is
/// known (floating layers).
pub fn shadow_shapes(rect: Rect, radius: CornerRadius, p: &Palette, m: &Material) -> Vec<Shape> {
    let mut shapes = Vec::with_capacity(m.layers.len());
    for l in m.layers {
        let mut r = rect
            .translate(vec2(0.0, f32::from(l.dy)))
            .expand(f32::from(l.spread));
        if r.width() <= 0.0 || r.height() <= 0.0 {
            continue;
        }
        // egui cuts a blur wider than the rect's shorter side, which leaves a crisp dark stub
        // under a small surface: hold the whole blur instead, as faint as the rect grew.
        let blur = f32::from(l.blur);
        let need = blur + 1.0;
        let mut alpha = f32::from(l.alpha);
        if r.width() < need {
            alpha *= r.width() / need;
            r = Rect::from_center_size(r.center(), vec2(need, r.height()));
        }
        if r.height() < need {
            alpha *= r.height() / need;
            r = Rect::from_center_size(r.center(), vec2(r.width(), need));
        }
        let spread = f32::from(l.spread);
        let corner = if spread >= 0.0 {
            radius + CornerRadius::same(crate::saturate_u8(spread))
        } else {
            shrink_radius(radius, -spread)
        };
        let color = Palette::tint(p.shadow_color(l.tint), crate::saturate_u8(alpha));
        shapes.push(
            RectShape::filled(r, corner, color)
                .with_blur_width(blur)
                .into(),
        );
    }
    shapes
}

fn shrink_radius(r: CornerRadius, by: f32) -> CornerRadius {
    let by = crate::saturate_u8(by.round().clamp(0.0, 255.0));
    CornerRadius {
        nw: r.nw.saturating_sub(by),
        ne: r.ne.saturating_sub(by),
        sw: r.sw.saturating_sub(by),
        se: r.se.saturating_sub(by),
    }
}

/// A 1 x 32 ramp from `top` to `bottom`, uploaded once per pair of colours and reused.
fn ramp(ctx: &Context, top: Color32, bottom: Color32) -> TextureHandle {
    const N: usize = 32;

    let id = Id::new(("canopy-kit-ramp", top, bottom));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let pixels = (0..N)
        .map(|i| top.lerp_to_gamma(bottom, crate::count_f32(i) / crate::count_f32(N - 1)))
        .collect();
    let image = egui::ColorImage::new([1, N], pixels);
    let handle = ctx.load_texture("canopy-kit-ramp", image, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
    handle
}

/// The shape of a vertical fill: one flat rect, or a ramp-textured rect (Deeper's gradients) with
/// true rounded corners and the tessellator's anti-aliasing.
pub fn fill_shape(ctx: &Context, rect: Rect, radius: CornerRadius, fill: Fill) -> Shape {
    if fill.is_flat() {
        return RectShape::filled(rect, radius, fill.top).into();
    }
    let tex = ramp(ctx, fill.top, fill.bottom);
    RectShape::filled(rect, radius, Color32::WHITE)
        .with_texture(tex.id(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)))
        .into()
}

/// Paints `fill` in `rect`.
pub fn fill(painter: &Painter, rect: Rect, radius: CornerRadius, fill: Fill) {
    painter.add(fill_shape(painter.ctx(), rect, radius, fill));
}

/// A 1 px light line just inside the top edge, inset by the corner radius (white at `alpha`).
pub fn top_highlight(painter: &Painter, rect: Rect, radius: CornerRadius, alpha: u8) {
    painter.extend(top_highlight_shape(
        painter.pixels_per_point(),
        rect,
        radius,
        alpha,
    ));
}

fn top_highlight_shape(ppp: f32, rect: Rect, radius: CornerRadius, alpha: u8) -> Option<Shape> {
    if alpha == 0 {
        return None;
    }
    let inset = f32::from(radius.nw.max(radius.ne)).max(1.0);
    let w = 1.0 / ppp;
    let y = rect.top() + w * 0.5;
    Some(Shape::hline(
        Rangef::new(rect.left() + inset, rect.right() - inset),
        y,
        Stroke::new(w, theme::highlight_line(alpha)),
    ))
}

/// A 1 px dark line just inside the bottom edge (light Deeper controls).
pub fn bottom_line(painter: &Painter, rect: Rect, radius: CornerRadius, color: Color32, alpha: u8) {
    painter.extend(bottom_line_shape(
        painter.pixels_per_point(),
        rect,
        radius,
        color,
        alpha,
    ));
}

fn bottom_line_shape(
    ppp: f32,
    rect: Rect,
    radius: CornerRadius,
    color: Color32,
    alpha: u8,
) -> Option<Shape> {
    if alpha == 0 {
        return None;
    }
    let inset = f32::from(radius.sw.max(radius.se)).max(1.0);
    let w = 1.0 / ppp;
    Some(Shape::hline(
        Rangef::new(rect.left() + inset, rect.right() - inset),
        rect.bottom() - w * 0.5,
        Stroke::new(w, Palette::tint(color, alpha)),
    ))
}

/// An inner shadow along the top edge (Deeper's field recess): three 1 pt rows fading down, clipped
/// to the rounded rect.
pub fn recess(painter: &Painter, rect: Rect, radius: CornerRadius, color: Color32, alpha: u8) {
    painter.extend(recess_shape(rect, radius, color, alpha));
}

fn recess_shape(rect: Rect, radius: CornerRadius, color: Color32, alpha: u8) -> Option<Shape> {
    if alpha == 0 {
        return None;
    }
    let mut pts = Vec::new();
    rounded_rectangle(&mut pts, rect.shrink(0.5), CornerRadiusF32::from(radius));
    let top = rect.top();
    let stroke = PathStroke::new_uv(1.5, move |_bbox: Rect, p: Pos2| {
        let k = (1.0 - (p.y - top) / 4.0).clamp(0.0, 1.0);
        Palette::tint(color, crate::saturate_u8(f32::from(alpha) * k))
    })
    .inside();
    Some(Shape::Path(PathShape {
        points: pts,
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke,
    }))
}

/// Everything a surface of `level` paints in `style`: its shadow layers, the fill, the top
/// highlight, the bottom line, the recess and a 1 pt ring inside its edge (`ring`, if any).
#[expect(clippy::too_many_arguments, reason = "one call paints a whole surface")]
pub fn surface(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    p: &Palette,
    style: Style,
    level: Elevation,
    body: Fill,
    ring: Option<Color32>,
) {
    let m = p.material(style, level);
    // A translucent floating layer's blurred backdrop goes between its shadow and its fill.
    let backdrop = if level == Elevation::Floating {
        super::glass::lift(painter, rect)
    } else {
        None
    };
    shadow(painter, rect, radius, p, &m);
    if let Some(backdrop) = backdrop {
        painter.add(backdrop);
    }
    fill(painter, rect, radius, body);
    top_highlight(painter, rect, radius, m.highlight);
    bottom_line(painter, rect, radius, p.shadow, m.bottom_line);
    recess(painter, rect, radius, p.shadow, m.recess);
    if let Some(c) = ring {
        rim(painter, rect, radius, c);
    }
}

/// The shapes [`surface`] paints, for a surface laid out after its content (reserve a slot with
/// `Painter::add(Shape::Noop)` and fill it with `Painter::set(slot, Shape::Vec(..))`).
#[expect(clippy::too_many_arguments, reason = "one call builds a whole surface")]
pub fn surface_shapes(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    p: &Palette,
    style: Style,
    level: Elevation,
    body: Fill,
    ring: Option<Color32>,
) -> Vec<Shape> {
    let m = p.material(style, level);
    let ppp = painter.pixels_per_point();
    let mut out = shadow_shapes(rect, radius, p, &m);
    out.push(fill_shape(painter.ctx(), rect, radius, body));
    out.extend(top_highlight_shape(ppp, rect, radius, m.highlight));
    out.extend(bottom_line_shape(
        ppp,
        rect,
        radius,
        p.shadow,
        m.bottom_line,
    ));
    out.extend(recess_shape(rect, radius, p.shadow, m.recess));
    if let Some(c) = ring {
        out.push(
            RectShape::stroke(rect, radius, Stroke::new(edge(), c), StrokeKind::Inside).into(),
        );
    }
    out
}

/// A ring inside `rect`'s edge in the active theme's edge width.
pub fn rim(painter: &Painter, rect: Rect, radius: CornerRadius, color: Color32) {
    painter.rect_stroke(rect, radius, Stroke::new(edge(), color), StrokeKind::Inside);
}

/// The outside focus ring in the active theme's [`FocusStyle`], `color` around the control with the
/// gap filled by `moat` (the surface behind): `Ring` a ring of its width at its offset, `Double` a
/// ring with a 1 pt ring of the ground inside it so it reads on any fill. The painter must not clip
/// outside `rect`.
pub fn focus_ring(
    painter: &Painter,
    rect: Rect,
    radius: CornerRadius,
    color: Color32,
    moat: Color32,
) {
    painter.extend(focus_ring_shapes(design().focus, rect, radius, color, moat));
}

fn focus_ring_shapes(
    style: FocusStyle,
    rect: Rect,
    radius: CornerRadius,
    color: Color32,
    moat: Color32,
) -> Vec<Shape> {
    let grow = |by: f32| radius + CornerRadius::same(crate::saturate_u8(by.round()));
    let (offset, width) = match style {
        FocusStyle::Ring { width, offset } => (offset, width),
        FocusStyle::Double { width } => (1.0, width),
    };
    let mut out = Vec::new();
    if moat.a() > 0 {
        out.push(
            RectShape::stroke(
                rect.expand(offset * 0.5),
                grow(offset * 0.5),
                Stroke::new(offset, moat),
                StrokeKind::Middle,
            )
            .into(),
        );
    }
    let ring = rect.expand(offset + width * 0.5);
    out.push(
        RectShape::stroke(
            ring,
            grow(offset),
            Stroke::new(width, color),
            StrokeKind::Middle,
        )
        .into(),
    );
    out
}

/// The inset focus ring of rows, tabs, segments and menu items, in the active theme's
/// [`FocusStyle`]: `INSET_FOCUS_WIDTH`, or a `Double` ring's width.
pub fn inset_ring(painter: &Painter, rect: Rect, radius: CornerRadius, color: Color32) {
    let width = match design().focus {
        FocusStyle::Double { width } => width.min(rect.height() / 4.0),
        FocusStyle::Ring { .. } => INSET_FOCUS_WIDTH,
    };
    painter.rect_stroke(rect, radius, Stroke::new(width, color), StrokeKind::Inside);
}

/// A horizontal seam at `y` in the active theme's edge width, snapped to the pixel grid.
pub fn seam_h(painter: &Painter, x: Rangef, y: f32, color: Color32) {
    let ppp = painter.pixels_per_point();
    let stroke = Stroke::new(edge(), color);
    let mut y = y;
    stroke.round_center_to_pixel(ppp, &mut y);
    painter.hline(x, y, stroke);
}

/// A vertical seam at `x` in the active theme's edge width, snapped to the pixel grid.
pub fn seam_v(painter: &Painter, x: f32, y: Rangef, color: Color32) {
    let ppp = painter.pixels_per_point();
    let stroke = Stroke::new(edge(), color);
    let mut x = x;
    stroke.round_center_to_pixel(ppp, &mut x);
    painter.vline(x, y, stroke);
}

/// Deeper's focused-pane edge: a 1 pt ring inside the pane, `focus` at `theme::FOCUS_EDGE`'s alphas
/// from the top down, scaled by `t` (its fade-in).
pub fn focus_edge(painter: &Painter, rect: Rect, radius: CornerRadius, focus: Color32, t: f32) {
    if t <= 0.0 {
        return;
    }
    let mut pts = Vec::new();
    rounded_rectangle(&mut pts, rect.shrink(0.5), CornerRadiusF32::from(radius));
    let (top, h) = (rect.top(), rect.height().max(1.0));
    let stroke = PathStroke::new_uv(SEAM, move |_bbox: Rect, p: Pos2| {
        let y = ((p.y - top) / h).clamp(0.0, 1.0);
        Palette::tint(focus, crate::saturate_u8(focus_edge_alpha(y) * t))
    })
    .inside();
    painter.add(Shape::Path(PathShape {
        points: pts,
        closed: true,
        fill: Color32::TRANSPARENT,
        stroke,
    }));
}

/// The focused-pane edge's alpha at `y` (0 at the top of the pane, 1 at the bottom).
pub fn focus_edge_alpha(y: f32) -> f32 {
    let stops = theme::FOCUS_EDGE;
    for w in stops.windows(2) {
        let ((y0, a0), (y1, a1)) = (w[0], w[1]);
        if y <= y1 {
            let k = ((y - y0) / (y1 - y0)).clamp(0.0, 1.0);
            return f32::from(a0) + (f32::from(a1) - f32::from(a0)) * k;
        }
    }
    f32::from(stops[stops.len() - 1].1)
}

/// Deeper's focused-pane edge on `card` at strength `t` (0 to 1): the [`focus_edge`] with its top
/// edge lit, so the pane that holds the keyboard (History's table) reads as the one in front.
pub fn pane_focus(painter: &Painter, card: Rect, radius: CornerRadius, focus: Color32, t: f32) {
    if t <= 0.0 {
        return;
    }
    focus_edge(painter, card, radius, focus, t);
    let r = f32::from(radius.nw);
    let width = 1.5;
    painter.hline(
        (card.left() + r)..=(card.right() - r),
        card.top() + width / 2.0,
        Stroke::new(width, Palette::tint(focus, crate::saturate_u8(210.0 * t))),
    );
}

/// A content pane in the active style. Standard: a flush `fill` with nothing else. Deeper: a
/// floating card (`Elevation::Pane`, `pane_edge` ring, `pane_radius`) and, while it holds focus
/// (`focus` 0 to 1), the focused-pane edge ([`pane_focus`]). `rect` is the card itself (the
/// caller leaves the gutters). Returns the radius it painted with, for content clipping.
pub fn pane(
    painter: &Painter,
    rect: Rect,
    p: &Palette,
    fill_color: Color32,
    focus: f32,
) -> CornerRadius {
    let style = theme::style();
    let s = p.surfaces(style);
    let m = theme::metrics();
    let radius = CornerRadius::same(m.pane_radius);
    if style == Style::Standard {
        painter.rect_filled(rect, CornerRadius::ZERO, fill_color);
        return CornerRadius::ZERO;
    }
    surface(
        painter,
        rect,
        radius,
        p,
        style,
        Elevation::Pane,
        Fill::flat(fill_color),
        s.pane_edge,
    );
    pane_focus(painter, rect, radius, p.focus, focus);
    radius
}

/// One bar of the top deck (the repository tab strip or the action bar) across `rect`, in the
/// active style. Standard: the flat chrome with a seam at the bottom. Deeper: the deck's lifted
/// gradient with its own highlight and a shadow onto the ground, edged below by `deck_edge`.
/// Deeper's window paints no full-width bar: its tab strip stays on the ground and its action bar
/// is a card ([`super::surface::Deck`] paints both styles' top bars).
pub fn deck(painter: &Painter, rect: Rect, p: &Palette, s: &Surfaces) {
    deck_in(painter, rect, p, s, theme::style());
}

/// [`deck`] in `style`, whatever the active style (the gallery's side-by-side strip).
pub fn deck_in(painter: &Painter, rect: Rect, p: &Palette, s: &Surfaces, style: Style) {
    if style == Style::Standard {
        painter.rect_filled(rect, CornerRadius::ZERO, s.deck.top);
        let seam = edge_seam(p, s.deck_edge);
        let w = seam.width;
        seam_line_h(painter, rect.x_range(), rect.bottom() - w * 0.5, seam);
        return;
    }
    let m = p.material(style, Elevation::Deck);
    shadow(painter, rect, CornerRadius::ZERO, p, &m);
    fill(painter, rect, CornerRadius::ZERO, s.deck);
    let w = 1.0 / painter.pixels_per_point();
    if m.highlight > 0 {
        painter.hline(
            rect.x_range(),
            rect.top() + w * 0.5,
            Stroke::new(w, theme::highlight_line(m.highlight)),
        );
    }
    seam_h(
        painter,
        rect.x_range(),
        rect.bottom() - SEAM * 0.5,
        s.deck_edge,
    );
}

/// How deep a scroll fade reaches into a vertical scroll area, and how strongly it covers the
/// content at the edge (spec 4.21: 12 pt at 85%).
pub const SCROLL_FADE: f32 = theme::MD;
pub const SCROLL_FADE_ALPHA: f32 = 0.85;
/// How wide a sideways scroll fade is (the tab strip): its items are wide, and a fade as narrow
/// as the vertical one reads as a clipped label rather than "more this way".
pub const SIDE_FADE: f32 = theme::LG;

/// The edge of a viewport a fade starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeEdge {
    Top,
    Bottom,
    Left,
    Right,
}

/// A fade `band` deep inside `rect` from `edge`: the ground behind the viewport (`ground`, top to
/// bottom across `rect`) at `alpha` along the edge, clear `band` inwards.
pub fn edge_fade(rect: Rect, edge: FadeEdge, band: f32, ground: Fill, alpha: f32) -> Shape {
    let band = band.min(match edge {
        FadeEdge::Top | FadeEdge::Bottom => rect.height(),
        FadeEdge::Left | FadeEdge::Right => rect.width(),
    });
    let at = |y: f32| {
        let t = ((y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        theme::mix(ground.top, ground.bottom, t).gamma_multiply(alpha)
    };
    let clear = Color32::TRANSPARENT;
    // The four corners clockwise from the top left, each with its colour.
    let corners = match edge {
        FadeEdge::Top => [
            (rect.left_top(), at(rect.top())),
            (rect.right_top(), at(rect.top())),
            (pos2(rect.right(), rect.top() + band), clear),
            (pos2(rect.left(), rect.top() + band), clear),
        ],
        FadeEdge::Bottom => [
            (pos2(rect.left(), rect.bottom() - band), clear),
            (pos2(rect.right(), rect.bottom() - band), clear),
            (rect.right_bottom(), at(rect.bottom())),
            (rect.left_bottom(), at(rect.bottom())),
        ],
        FadeEdge::Left => [
            (rect.left_top(), at(rect.top())),
            (pos2(rect.left() + band, rect.top()), clear),
            (pos2(rect.left() + band, rect.bottom()), clear),
            (rect.left_bottom(), at(rect.bottom())),
        ],
        FadeEdge::Right => [
            (pos2(rect.right() - band, rect.top()), clear),
            (rect.right_top(), at(rect.top())),
            (rect.right_bottom(), at(rect.bottom())),
            (pos2(rect.right() - band, rect.bottom()), clear),
        ],
    };
    let mut mesh = egui::Mesh::default();
    for (pos, color) in corners {
        mesh.colored_vertex(pos, color);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Shape::mesh(mesh)
}

/// Fades the top and bottom edges of a vertical scroll area into `surface` where more content lies
/// beyond them ([`SCROLL_FADE`] at [`SCROLL_FADE_ALPHA`]), so a row cut by the viewport reads as
/// "scroll for more".
pub fn scroll_fade<R>(
    ui: &egui::Ui,
    output: &egui::scroll_area::ScrollAreaOutput<R>,
    surface: Color32,
) {
    let rect = output.inner_rect;
    let hidden_above = output.state.offset.y;
    let hidden_below = output.content_size.y - rect.height() - output.state.offset.y;
    let painter = ui.painter_at(rect);
    let band = SCROLL_FADE.min(rect.height() / 4.0);
    let ground = Fill::flat(surface);
    if hidden_above > 1.0 {
        painter.add(edge_fade(
            rect,
            FadeEdge::Top,
            band,
            ground,
            SCROLL_FADE_ALPHA,
        ));
    }
    if hidden_below > 1.0 {
        painter.add(edge_fade(
            rect,
            FadeEdge::Bottom,
            band,
            ground,
            SCROLL_FADE_ALPHA,
        ));
    }
}

/// Fades the left and right edges of a sideways scroll area into `ground` (its colour from the
/// top of the viewport to the bottom) where more content lies beyond them ([`SIDE_FADE`] at
/// [`SCROLL_FADE_ALPHA`]).
pub fn scroll_fade_x<R>(
    ui: &egui::Ui,
    output: &egui::scroll_area::ScrollAreaOutput<R>,
    ground: Fill,
) {
    let rect = output.inner_rect;
    let hidden_left = output.state.offset.x;
    let hidden_right = output.content_size.x - rect.width() - output.state.offset.x;
    let painter = ui.painter_at(rect);
    let band = SIDE_FADE.min(rect.width() / 4.0);
    if hidden_left > 0.5 {
        painter.add(edge_fade(
            rect,
            FadeEdge::Left,
            band,
            ground,
            SCROLL_FADE_ALPHA,
        ));
    }
    if hidden_right > 0.5 {
        painter.add(edge_fade(
            rect,
            FadeEdge::Right,
            band,
            ground,
            SCROLL_FADE_ALPHA,
        ));
    }
}

// ---------------------------------------------------------------------------------------- glow

/// Texels across the glow texture. It is magnified with linear filtering, so even a glow hundreds
/// of points wide follows its curve in steps of a few pixels, far below any visible crease.
const GLOW_TEXELS: usize = 256;

/// A glow's strength at `t` (0 at the centre, 1 at the rim): `(1 - t²)²`. Flat at the centre and
/// meeting the rim with zero slope, so a glow has no hot spot and no visible edge.
pub fn glow_falloff(t: f32) -> f32 {
    let u = (1.0 - t * t).max(0.0);
    u * u
}

/// The glow's falloff as a white texture at full strength, uploaded once per context. Its alpha
/// spans all 255 steps, so a faint glow's colour is never rounded in the texture: the tint carries
/// the strength in floating point, and the renderer's output dither (egui's interleaved-gradient
/// noise, on in eframe by default) turns the final 8-bit rounding into noise instead of rings.
fn glow_texture(ctx: &Context) -> TextureHandle {
    let id = Id::new("canopy-kit-glow");
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return t;
    }
    let handle = ctx.load_texture("canopy-kit-glow", glow_image(), TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, handle.clone()));
    handle
}

fn glow_image() -> egui::ColorImage {
    let half = crate::count_f32(GLOW_TEXELS) / 2.0;
    let pixels = (0..GLOW_TEXELS * GLOW_TEXELS)
        .map(|i| {
            let x = crate::count_f32(i % GLOW_TEXELS) + 0.5 - half;
            let y = crate::count_f32(i / GLOW_TEXELS) + 0.5 - half;
            let t = x.hypot(y) / half;
            Palette::tint(Color32::WHITE, crate::saturate_u8(glow_falloff(t) * 255.0))
        })
        .collect();
    egui::ColorImage::new([GLOW_TEXELS, GLOW_TEXELS], pixels)
}

/// The shape of a soft radial glow: `color` (premultiplied; its alpha is the glow's strength) at
/// `center`, falling off along [`glow_falloff`] to nothing at `radius`.
pub fn glow_shape(ctx: &Context, center: Pos2, radius: f32, color: Color32) -> Shape {
    if radius <= 0.0 || color == Color32::TRANSPARENT {
        return Shape::Noop;
    }
    let mut mesh = egui::Mesh::with_texture(glow_texture(ctx).id());
    mesh.add_rect_with_uv(
        Rect::from_center_size(center, egui::Vec2::splat(radius * 2.0)),
        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        color,
    );
    Shape::mesh(mesh)
}

/// Paints a soft radial glow (see [`glow_shape`]): the welcome backdrop and the illustrations'
/// halos.
pub fn glow(painter: &Painter, center: Pos2, radius: f32, color: Color32) {
    painter.add(glow_shape(painter.ctx(), center, radius, color));
}

/// The active theme's ground treatment (`Design::ground`) over `rect`, a window ground where no
/// content sits (Deeper's gutter, the welcome screen): a vertical wash, a translucent ink over
/// the ground already painted. `t` (0 to 1) scales it (a fade-in). Nothing for a flat ground.
pub fn ground(painter: &Painter, rect: Rect, t: f32) {
    if t <= 0.0 || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let scale = |a: u8| crate::saturate_u8(f32::from(a) * t);
    match design().ground {
        Ground::Flat => {}
        Ground::Vertical { ink, top, bottom } => fill(
            &painter,
            rect,
            CornerRadius::ZERO,
            Fill {
                top: Palette::tint(ink, scale(top)),
                bottom: Palette::tint(ink, scale(bottom)),
            },
        ),
    }
}

/// How strongly a spinner's full circle shows under its turning arc.
pub const SPINNER_TRACK: f32 = 0.25;

/// A spinner: a 270 degree arc turning once a second around `center`, `color` at full strength
/// over a faint full circle. Repaints itself while painted; still (one fixed arc) with reduced
/// motion.
pub fn spinner(painter: &Painter, center: Pos2, radius: f32, width: f32, color: Color32) {
    let ctx = painter.ctx();
    let still = crate::motion::reduced();
    let turn = if still {
        0.0
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        crate::to_f32(ctx.input(|i| i.time) % 1.0) * std::f32::consts::TAU
    };
    painter.circle_stroke(
        center,
        radius,
        Stroke::new(width, color.gamma_multiply(SPINNER_TRACK)),
    );
    let points: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = turn + crate::int_f32(i) / 24.0 * std::f32::consts::TAU * 0.75;
            center + vec2(a.cos(), a.sin()) * radius
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(width, color)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_edge_fades_from_the_top() {
        assert_eq!(focus_edge_alpha(0.0), 191.0);
        assert!((focus_edge_alpha(0.38) - 46.0).abs() < 0.01);
        assert_eq!(focus_edge_alpha(0.7), 0.0);
        assert_eq!(focus_edge_alpha(1.0), 0.0);
        assert!(focus_edge_alpha(0.2) < 191.0 && focus_edge_alpha(0.2) > 46.0);
    }

    #[test]
    fn the_glow_falls_off_without_a_hot_spot_or_an_edge() {
        assert!((glow_falloff(0.0) - 1.0).abs() < f32::EPSILON);
        assert!(glow_falloff(1.0).abs() < f32::EPSILON);
        assert!(glow_falloff(1.5).abs() < f32::EPSILON);
        // Zero slope at the centre and at the rim.
        let h = 1e-3;
        assert!((glow_falloff(h) - glow_falloff(0.0)).abs() / h < 0.01);
        assert!((glow_falloff(1.0 - h) - glow_falloff(1.0)).abs() / h < 0.01);
        let mut last = 1.0;
        for i in 1..=100 {
            let v = glow_falloff(crate::int_f32(i) / 100.0);
            assert!(v <= last, "rises at {i}");
            last = v;
        }
    }

    /// The texture behind every glow: full strength at the centre, nothing at its edge, and no
    /// step between neighbouring texels large enough to show as a ring when magnified.
    #[test]
    fn the_glow_texture_is_one_smooth_curve() {
        let image = glow_image();
        let n = GLOW_TEXELS;
        let alpha = |x: usize, y: usize| i32::from(image.pixels[y * n + x].a());
        assert!(alpha(n / 2, n / 2) >= 254);
        for i in 0..n {
            for (x, y) in [(i, 0), (0, i), (i, n - 1), (n - 1, i)] {
                assert!(alpha(x, y) <= 1, "edge texel ({x}, {y}) is {}", alpha(x, y));
            }
        }
        let row = n / 2;
        for x in n / 2..n - 1 {
            let step = alpha(x, row) - alpha(x + 1, row);
            assert!((0..=4).contains(&step), "step {step} at {x}");
        }
        // White, premultiplied: every channel equals the alpha.
        assert!(
            image
                .pixels
                .iter()
                .all(|c| c.r() == c.a() && c.b() == c.a())
        );
    }

    #[test]
    fn a_glow_is_one_tinted_quad_on_the_glow_texture() {
        let ctx = Context::default();
        let color = Palette::tint(theme::ThemeId::Midnight.palette().accent, 40);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            glow(ui.painter(), pos2(300.0, 200.0), 250.0, color);
            glow(ui.painter(), pos2(10.0, 10.0), 0.0, color);
        });
        out.textures_delta.clear();
        let meshes: Vec<&egui::Mesh> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Mesh(m) => Some(&**m),
                _ => None,
            })
            .collect();
        assert_eq!(meshes.len(), 1, "an empty glow paints nothing");
        let mesh = meshes[0];
        assert_eq!(mesh.vertices.len(), 4);
        assert_ne!(mesh.texture_id, egui::TextureId::default());
        assert!(mesh.vertices.iter().all(|v| v.color == color));
        let bounds = mesh.calc_bounds();
        assert_eq!(
            bounds,
            Rect::from_center_size(pos2(300.0, 200.0), egui::Vec2::splat(500.0))
        );
    }

    #[test]
    fn edge_fades_are_opaque_at_their_edge_and_clear_inwards() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 30.0));
        let ground = Fill {
            top: theme::ThemeId::Midnight.palette().text,
            bottom: theme::ThemeId::Midnight.palette().s().pane,
        };
        for edge in [
            FadeEdge::Top,
            FadeEdge::Bottom,
            FadeEdge::Left,
            FadeEdge::Right,
        ] {
            let Shape::Mesh(mesh) = edge_fade(rect, edge, 16.0, ground, 1.0) else {
                panic!("not a mesh");
            };
            for v in &mesh.vertices {
                let on_edge = match edge {
                    FadeEdge::Top => v.pos.y <= rect.top(),
                    FadeEdge::Bottom => v.pos.y >= rect.bottom(),
                    FadeEdge::Left => v.pos.x <= rect.left(),
                    FadeEdge::Right => v.pos.x >= rect.right(),
                };
                assert_eq!(v.color.a() == 255, on_edge, "{edge:?} {v:?}");
                assert!(rect.contains(v.pos));
            }
        }
        // A sideways fade follows the ground down its height.
        let Shape::Mesh(mesh) = edge_fade(rect, FadeEdge::Left, 16.0, ground, 1.0) else {
            panic!("not a mesh");
        };
        assert_eq!(mesh.vertices[0].color, ground.top);
        assert_eq!(mesh.vertices[3].color, ground.bottom);
    }

    fn stroke_widths(shapes: &[Shape]) -> Vec<f32> {
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Rect(r) => Some(r.stroke.width),
                _ => None,
            })
            .collect()
    }

    /// Focus rings take the theme's focus style: a ring at its offset, a thin ring with a glow
    /// beyond it, or a heavy ring with a ground-coloured ring inside it.
    #[test]
    fn focus_rings_take_the_themes_style() {
        let rect = Rect::from_min_size(pos2(20.0, 20.0), vec2(80.0, 24.0));
        let r = CornerRadius::same(4);
        let (c, moat) = (Color32::RED, Color32::BLUE);
        let ring = focus_ring_shapes(
            FocusStyle::Ring {
                width: 2.0,
                offset: 1.0,
            },
            rect,
            r,
            c,
            moat,
        );
        assert_eq!(stroke_widths(&ring), [1.0, 2.0]);
        let double = focus_ring_shapes(FocusStyle::Double { width: 2.5 }, rect, r, c, moat);
        assert_eq!(stroke_widths(&double), [1.0, 2.5]);
        let Shape::Rect(inner) = &double[0] else {
            panic!("the ground ring")
        };
        assert_eq!(inner.stroke.color, moat);
    }

    /// The shapes `f` paints in `theme`.
    fn painted(theme: theme::ThemeId, f: impl Fn(&Painter)) -> Vec<Shape> {
        let ctx = Context::default();
        theme::apply(&ctx, theme, theme::Appearance::STILL);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| f(ui.painter()));
        out.textures_delta.clear();
        out.shapes.into_iter().map(|c| c.shape).collect()
    }

    /// A selected row carries its theme's mark: Observatory's leading bar, high contrast's ring;
    /// concept 1's wash (Midnight, Daylight) needs none.
    #[test]
    fn selection_takes_the_themes_mark() {
        use theme::ThemeId as T;
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 28.0));
        let mark = |t: T| {
            painted(t, |painter| {
                selection_mark(painter, rect, CornerRadius::same(4), t.palette());
            })
        };
        let bar = mark(T::Observatory);
        let Some(Shape::Rect(bar)) = bar.first() else {
            panic!("{bar:?}")
        };
        assert_eq!(bar.rect.width(), 2.0);
        assert_eq!(bar.rect.left(), rect.left());
        assert_eq!(bar.fill, T::Observatory.palette().focus);
        theme::set_high_contrast(true);
        let ring = mark(T::Midnight);
        theme::set_high_contrast(false);
        let Some(Shape::Rect(ring)) = ring.first() else {
            panic!("{ring:?}")
        };
        assert_eq!(ring.rect, rect);
        assert!(ring.stroke.width > 0.0 && ring.fill == Color32::TRANSPARENT);
        assert_eq!(mark(T::Daylight).len(), 0);
        assert_eq!(mark(T::Midnight).len(), 0);
    }

    /// The ground treatment: nothing for a flat ground, a ramp for a vertical one (Observatory).
    #[test]
    fn grounds_take_the_themes_treatment() {
        use theme::ThemeId as T;
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 500.0));
        let ground_of = |t: T| painted(t, |painter| ground(painter, rect, 1.0));
        assert_eq!(ground_of(T::Daylight).len(), 0);
        assert_eq!(ground_of(T::Midnight).len(), 0);
        theme::set_high_contrast(true);
        assert_eq!(ground_of(T::Observatory).len(), 0, "high contrast is flat");
        theme::set_high_contrast(false);
        assert!(matches!(ground_of(T::Observatory)[..], [Shape::Rect(_)]));
    }

    /// A shadow under a surface smaller than its blur (a one-line tooltip) keeps its whole blur:
    /// egui would cut the blur to the rect, leaving a crisp dark stub under the surface, so the
    /// layer's rect grows to hold the blur and fades by as much as it grew.
    #[test]
    fn a_small_surface_keeps_its_shadow_soft() {
        let rect = Rect::from_min_size(pos2(40.0, 40.0), vec2(64.0, 22.0));
        for id in theme::ThemeId::ALL {
            let p = id.palette();
            for style in [Style::Standard, Style::Deeper] {
                let m = p.material(style, Elevation::Floating);
                let shapes = shadow_shapes(rect, CornerRadius::same(6), p, &m);
                let mut painted = shapes.iter().filter_map(|s| match s {
                    Shape::Rect(r) => Some(r),
                    _ => None,
                });
                for l in m.layers {
                    let r = rect
                        .translate(vec2(0.0, f32::from(l.dy)))
                        .expand(f32::from(l.spread));
                    if r.width() <= 0.0 || r.height() <= 0.0 {
                        continue;
                    }
                    let shape = painted.next().expect("a shape per visible layer");
                    let what = format!("{}/{style:?} {l:?}", p.name);
                    assert!(
                        (shape.blur_width - f32::from(l.blur)).abs() < 0.01,
                        "{what}: blur cut to {}",
                        shape.blur_width
                    );
                    assert!(
                        shape.rect.size().min_elem() >= shape.blur_width + 1.0 - 0.01,
                        "{what}: {:?} cannot hold the blur",
                        shape.rect
                    );
                    assert!(shape.fill.a() <= l.alpha, "{what}: darker than designed");
                }
            }
        }
    }

    #[test]
    fn negative_spread_shrinks_the_shadow() {
        let ctx = Context::default();
        let p = theme::ThemeId::Midnight.palette();
        let rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(100.0, 60.0));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let m = p.material(Style::Standard, Elevation::Floating);
            shadow(ui.painter(), rect, CornerRadius::same(12), p, &m);
        });
        out.textures_delta.clear();
        let shapes: Vec<Rect> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Rect(r) => Some(r.rect),
                _ => None,
            })
            .collect();
        // (10, 28, -8): moved down 10 and 8 smaller on every side (tall enough to hold its blur).
        assert!(
            shapes.contains(&Rect::from_min_max(pos2(18.0, 28.0), pos2(102.0, 72.0))),
            "{shapes:?}"
        );
    }

    /// The pane that holds the keyboard reads as the one in front, the same wherever a pane is
    /// painted (the window's content pane, the gallery): Deeper lights its top edge.
    #[test]
    fn a_focused_pane_lights_its_top_edge() {
        let card = Rect::from_min_size(pos2(10.0, 10.0), vec2(200.0, 120.0));
        let paint = |id: theme::ThemeId, focus: f32| {
            let ctx = crate::kit::testkit::ctx_with(id, Style::Deeper, theme::Density::Comfortable);
            let p = theme::palette(&ctx);
            let shapes = crate::kit::testkit::frame(&ctx, Vec::new(), |ui| {
                pane(ui.painter(), card, p, p.surfaces(Style::Deeper).pane, focus);
            });
            (p.focus, shapes)
        };
        let top_lines = |focus_color: Color32, shapes: &[egui::epaint::ClippedShape]| {
            shapes
                .iter()
                .filter(|c| match &c.shape {
                    Shape::LineSegment { points, stroke } => {
                        (points[0].y - points[1].y).abs() < 0.01
                            && points[0].y < card.top() + 2.0
                            && stroke.color == Palette::tint(focus_color, 210)
                    }
                    _ => false,
                })
                .count()
        };
        let (focus, shapes) = paint(theme::ThemeId::Midnight, 1.0);
        assert_eq!(top_lines(focus, &shapes), 1, "a lit top edge: {shapes:?}");
        let (focus, shapes) = paint(theme::ThemeId::Midnight, 0.0);
        assert_eq!(top_lines(focus, &shapes), 0, "no edge without focus");
    }
}
