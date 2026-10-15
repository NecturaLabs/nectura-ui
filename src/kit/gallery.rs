//! The component gallery (`--overlay gallery`): every kit component in every state on the active
//! theme, style, density and translucency, for screenshot QA. Not reachable from the interface.
//!
//! Each kit module with components adds `pub(crate) fn gallery(ui: &mut Ui)` and one row in
//! [`SECTIONS`]. A section is drawn twice side by side, on the content ground (`pane`) and on the
//! chrome (`panel`), so every component is seen on both. Force states with `show_forced`.
//! `--gallery-page N` scrolls down N pages (the window less the strip at its bottom) so the
//! screenshot script can capture a page at a time; a page past the end shows the last one, and
//! the strip says which page it is ("Page 4 of 16" for `--gallery-page 3`), below the content.

use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Order, Rect, RichText, Sense, Stroke, StrokeKind,
    Ui, Vec2, pos2, vec2,
};

use super::{ButtonKind, ForceState, button, menu, overlay, paint, progress};
use crate::theme::{self, Elevation, Fill, Palette, Style};

/// A gallery section: its title and the function that draws its components.
pub type Section = (&'static str, fn(&mut Ui));

/// The sections, in the spec's order (design spec 9). Component modules append theirs here.
pub const SECTIONS: &[Section] = &[
    ("Buttons", button::gallery),
    ("Fields", super::field::gallery),
    ("Select", super::select::gallery),
    ("Toggles", super::toggle::gallery),
    ("Segmented and tabs", super::choice::gallery),
    ("Document tabs", super::tabs::gallery),
    ("Slider", super::range::gallery),
    ("Progress", progress::gallery),
    ("Menus", menu::gallery),
    ("Overlays", overlay::gallery),
    ("Lists", super::row::gallery),
    ("Chips", super::chip::gallery),
    ("People", super::avatar::gallery),
    ("Surfaces", super::surface::gallery),
    ("Split handles", super::split::gallery),
    ("Translucent surfaces", super::glass::gallery),
];

const MAX_WIDTH: f32 = 1360.0;

/// The strip along the window's bottom that holds the page label, clear of the content.
fn strip_height() -> f32 {
    theme::metrics().small_size + 2.0 * theme::XS + 2.0 * theme::SM
}

/// Paints the gallery over the whole window, scrolled down `page` window heights (no further than
/// its end). `extra` are the host application's own sections, drawn after the library's.
pub fn show(ctx: &egui::Context, page: usize, extra: &[Section]) {
    let screen = ctx.content_rect();
    // The content's and the viewport's height last frame: a page past the end is the last page.
    let size_key = Id::new("nectura-gallery-size");
    let size: Option<(f32, f32)> = ctx.data(|d| d.get_temp(size_key));
    let viewport = (screen.height() - strip_height()).max(0.0);
    let offset = page_offset(page, size, viewport);
    egui::Area::new(Id::new("nectura-gallery"))
        .order(Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let s = theme::surfaces(ctx);
            ui.painter().rect_filled(screen, CornerRadius::ZERO, s.bg);
            ui.set_min_size(screen.size());
            ui.set_max_size(screen.size());
            let out = egui::ScrollArea::vertical()
                .id_salt("nectura-gallery-scroll")
                .max_height(viewport)
                .auto_shrink([false, false])
                .vertical_scroll_offset(offset)
                .show(ui, |ui| {
                    let width = ui.available_width().min(MAX_WIDTH);
                    let side = (ui.available_width() - width) / 2.0;
                    ui.horizontal(|ui| {
                        ui.add_space(side);
                        ui.vertical(|ui| {
                            ui.set_width(width);
                            page_content(ui, extra);
                        });
                    });
                });
            let now = (out.content_size.y, out.inner_rect.height());
            if size != Some(now) {
                ctx.data_mut(|d| d.insert_temp(size_key, now));
                ctx.request_repaint();
            }
            page_label(ui, screen, offset, now);
        });
}

/// Where page `page` (from 0) starts: `page` viewports down, but no further than the last full
/// viewport of the content (`size`: its height and the viewport's, once known).
fn page_offset(page: usize, size: Option<(f32, f32)>, viewport: f32) -> f32 {
    let wanted = crate::count_f32(page) * viewport;
    match size {
        Some((content, viewport)) => wanted.min((content - viewport).max(0.0)),
        None => wanted,
    }
}

/// "Page 4 of 16" in the strip at the bottom right, under the content, so a screenshot says where
/// in the gallery it is (pages count from 1).
fn page_label(ui: &Ui, screen: Rect, offset: f32, (content, viewport): (f32, f32)) {
    if viewport <= 0.0 {
        return;
    }
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let (at, pages) = page_of(offset, content, viewport);
    let text = format!("Page {at} of {pages}");
    let g = ui
        .painter()
        .layout_no_wrap(text, FontId::proportional(m.small_size), p.text_dim);
    let pad = vec2(theme::SM, theme::XS);
    let rect = Rect::from_min_size(
        screen.right_bottom() - g.size() - pad * 2.0 - vec2(theme::LG, theme::SM),
        g.size() + pad * 2.0,
    );
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(m.radius_sm), s.overlay);
    painter.rect_stroke(
        rect,
        CornerRadius::same(m.radius_sm),
        Stroke::new(paint::SEAM, s.border),
        StrokeKind::Inside,
    );
    painter.galley(rect.min + pad, g, p.text_dim);
}

/// The page (from 1) the gallery shows at `offset`, and how many there are.
fn page_of(offset: f32, content: f32, viewport: f32) -> (usize, usize) {
    let rest = ((content - viewport).max(0.0) / viewport).ceil();
    let at = (offset / viewport).ceil().min(rest);
    (
        crate::saturate_usize(at) + 1,
        crate::saturate_usize(rest) + 1,
    )
}

fn page_content(ui: &mut Ui, extra: &[Section]) {
    let p = theme::palette(ui.ctx());
    ui.add_space(theme::LG);
    header(ui, p);
    for (title, draw) in SECTIONS.iter().chain(extra) {
        band(ui, title, *draw);
    }
    band(ui, "Elevation", elevation);
    band(ui, "Style strip", style_strip);
    band(ui, "Motion strip", motion_strip);
    ui.add_space(theme::XXL);
}

fn title(ui: &mut Ui, text: &str) {
    let m = theme::metrics();
    let p = theme::palette(ui.ctx());
    ui.label(
        RichText::new(text)
            .font(theme::heading(m.title_size))
            .color(p.text),
    );
}

fn caption(ui: &mut Ui, text: &str) {
    let p = theme::palette(ui.ctx());
    ui.label(theme::section_label(text, p.text_faint));
}

/// A titled band: `draw` on the content ground (left) and on the chrome (right).
pub fn band(ui: &mut Ui, name: &str, draw: fn(&mut Ui)) {
    let s = theme::surfaces(ui.ctx());
    ui.add_space(theme::XL);
    title(ui, name);
    ui.add_space(theme::SM);
    let width = ui.available_width();
    let half = (width - theme::SM) / 2.0;
    ui.horizontal_top(|ui| {
        for (i, ground) in [s.pane, s.panel].into_iter().enumerate() {
            ui.push_id((name, i), |ui| {
                ui.allocate_ui_with_layout(
                    vec2(half, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        egui::Frame::new()
                            .fill(ground)
                            .stroke(Stroke::new(paint::SEAM, s.border))
                            .corner_radius(CornerRadius::same(theme::metrics().radius_lg))
                            .inner_margin(egui::Margin::same(crate::points_i8(theme::LG)))
                            .show(ui, |ui| {
                                ui.set_width(half - 2.0 * theme::LG - 2.0);
                                caption(ui, if i == 0 { "On content" } else { "On chrome" });
                                ui.add_space(theme::SM);
                                draw(ui);
                            });
                    },
                );
            });
        }
    });
}

/// The options that render, then every palette and surface token with its hex.
fn header(ui: &mut Ui, p: &Palette) {
    let look = theme::appearance();
    let ppp = ui.ctx().pixels_per_point();
    title(ui, &format!("Canopy components: {}", p.name));
    ui.add_space(theme::XS);
    let m = theme::metrics();
    ui.label(
        RichText::new(format!(
            "Style {} · Density {} · Translucent surfaces {} (glass {} at clamp {}) · Motion {} · {:.2} px per point",
            look.style.label(),
            look.density.label(),
            if look.glass { "on" } else { "off" },
            p.glass_alpha,
            p.glass_luminance,
            if look.reduce_motion { "reduced" } else { "full" },
            ppp
        ))
        .size(m.meta_size)
        .color(p.text_dim),
    );
    ui.add_space(theme::MD);
    let s = p.surfaces(look.style);
    let shared: [(&str, Color32); 29] = [
        ("text", p.text),
        ("text_dim", p.text_dim),
        ("text_faint", p.text_faint),
        ("accent", p.accent),
        ("accent2", p.accent2),
        ("focus", p.focus),
        ("on_accent", p.on_accent),
        ("success", p.success),
        ("warning", p.warning),
        ("danger", p.danger),
        ("diff_added", p.diff_added),
        ("diff_removed", p.diff_removed),
        ("danger_base", p.danger_base),
        ("on_danger", p.on_danger),
        ("control_border", p.control_border),
        ("on_tooltip", p.on_tooltip),
        ("on_tooltip_dim", p.on_tooltip_dim),
        ("hover", theme::over(p.hover(), s.pane)),
        ("hover_strong", theme::over(p.hover_strong(), s.pane)),
        ("pressed", theme::over(p.pressed(), s.pane)),
        ("selected", theme::over(p.selected(), s.pane)),
        ("zebra", theme::over(p.zebra(), s.pane)),
        ("shadow", p.shadow),
        ("success_tint", theme::over(p.success_tint(), s.pane)),
        ("danger_tint", theme::over(p.danger_tint(), s.pane)),
        ("warning_tint", theme::over(p.warning_tint(), s.pane)),
        (
            "danger_highlight",
            theme::over(p.danger_highlight(), s.overlay),
        ),
        ("danger_badge", theme::over(p.danger_badge_fill(), s.pane)),
        (
            "change_fill",
            theme::over(Palette::change_fill(p.success), s.pane),
        ),
    ];
    let surfaces: Vec<(&str, Fill)> = vec![
        ("bg", Fill::flat(s.bg)),
        ("panel", Fill::flat(s.panel)),
        ("pane", Fill::flat(s.pane)),
        ("elevated", Fill::flat(s.elevated)),
        ("overlay", Fill::flat(s.overlay)),
        ("field", Fill::flat(s.field)),
        ("track", Fill::flat(s.track)),
        ("switch_off", Fill::flat(s.switch_off)),
        ("badge", Fill::flat(s.badge)),
        ("border", Fill::flat(s.border)),
        ("edge", Fill::flat(s.edge)),
        ("raised_edge", Fill::flat(s.raised_edge)),
        ("overlay_edge", Fill::flat(s.overlay_edge)),
        ("tab_edge", Fill::flat(s.tab_edge)),
        ("track_edge", Fill::flat(s.track_edge)),
        ("keycap_edge", Fill::flat(s.keycap_edge)),
        ("raised", s.raised),
        ("raised_hover", s.raised_hover),
        ("raised_pressed", s.raised_pressed),
        ("thumb", s.thumb),
        ("keycap", s.keycap),
        ("tab", s.tab),
        ("tooltip", s.tooltip),
        ("knob", s.knob),
        ("primary", s.primary),
        ("primary_hover", s.primary_hover),
        ("primary_pressed", s.primary_pressed),
        ("danger_solid", s.danger_solid),
        ("danger_solid_hover", s.danger_solid_hover),
        ("danger_solid_pressed", s.danger_solid_pressed),
        ("added_bg", Fill::flat(s.added_bg)),
        ("removed_bg", Fill::flat(s.removed_bg)),
        ("added_gutter", Fill::flat(s.added_gutter)),
        ("removed_gutter", Fill::flat(s.removed_gutter)),
        ("deck", s.deck),
        ("deck_edge", Fill::flat(s.deck_edge)),
        ("pane_header", Fill::flat(s.pane_header)),
        ("pane_header_seam", Fill::flat(s.pane_header_seam)),
    ];
    let mut surfaces = surfaces;
    if let Some(ring) = s.knob_edge {
        surfaces.push(("knob_edge", Fill::flat(ring)));
    }
    caption(ui, "Theme colours");
    swatches(ui, shared.iter().map(|(n, c)| (*n, Fill::flat(*c))));
    ui.add_space(theme::SM);
    caption(ui, "Surfaces");
    swatches(ui, surfaces.into_iter());
    ui.add_space(theme::SM);
    caption(ui, "Lanes");
    swatches(
        ui,
        p.lanes.iter().enumerate().map(|(i, c)| {
            (
                ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"][i],
                Fill::flat(*c),
            )
        }),
    );
    ui.add_space(theme::SM);
    caption(ui, "Tones");
    swatches(
        ui,
        super::Tone::ALL
            .iter()
            .map(|t| (t.label(), Fill::flat(theme::over(t.tint(p), s.pane)))),
    );
    ui.add_space(theme::SM);
    caption(ui, "Metrics");
    let rows = m.rows();
    let cols = 3;
    ui.spacing_mut().interact_size.y = m.small_size + theme::XS;
    egui::Grid::new("gallery-metrics")
        .num_columns(cols * 2)
        .spacing(vec2(theme::MD, 2.0))
        .show(ui, |ui| {
            for chunk in rows.chunks(cols) {
                for (name, value) in chunk {
                    ui.label(RichText::new(*name).size(m.small_size).color(p.text_faint));
                    ui.label(RichText::new(value).size(m.small_size).color(p.text_dim));
                }
                ui.end_row();
            }
        });
}

fn swatches<'a>(ui: &mut Ui, items: impl Iterator<Item = (&'a str, Fill)>) {
    let p = theme::palette(ui.ctx());
    let s = theme::surfaces(ui.ctx());
    let m = theme::metrics();
    let items: Vec<(&str, Fill)> = items.collect();
    // Every swatch as wide as the longest name (or value) needs, so none runs into the next.
    let widest = items
        .iter()
        .map(|(name, fill)| {
            let width = |text: String, font: FontId| {
                ui.painter().layout_no_wrap(text, font, p.text_dim).size().x
            };
            let value = if fill.is_flat() { 6 } else { 13 };
            width((*name).to_owned(), FontId::proportional(m.micro_size))
                .max(width("0".repeat(value), FontId::monospace(m.micro_size)))
        })
        .fold(0.0_f32, f32::max);
    let swatch_w = (28.0 + 5.0 + widest + theme::SM).max(118.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(theme::XS, theme::XS);
        for (name, fill) in items {
            let (rect, _) = ui.allocate_exact_size(vec2(swatch_w, 40.0), Sense::hover());
            let chip = Rect::from_min_size(rect.min, vec2(28.0, rect.height()));
            paint::fill(ui.painter(), chip, CornerRadius::same(m.radius_sm), fill);
            ui.painter().rect_stroke(
                chip,
                CornerRadius::same(m.radius_sm),
                Stroke::new(paint::SEAM, s.border),
                StrokeKind::Inside,
            );
            let hex = |c: Color32| format!("{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
            let value = if fill.is_flat() {
                hex(fill.top)
            } else {
                format!("{}→{}", hex(fill.top), hex(fill.bottom))
            };
            ui.painter().text(
                pos2(chip.right() + 5.0, rect.top() + 11.0),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(m.micro_size),
                p.text_dim,
            );
            ui.painter().text(
                pos2(chip.right() + 5.0, rect.top() + 27.0),
                Align2::LEFT_CENTER,
                value,
                FontId::monospace(m.micro_size),
                p.text_faint,
            );
        }
    });
}

/// Every elevation level of the active style on a sample of the surface that uses it.
fn elevation(ui: &mut Ui) {
    let p = theme::palette(ui.ctx());
    let style = theme::style();
    let s = p.surfaces(style);
    let m = theme::metrics();
    for row in Elevation::ALL.chunks(5) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(theme::SM, theme::SM);
            for &level in row {
                let (body, ring, size) = match level {
                    Elevation::Control => (s.raised, Some(s.raised_edge), vec2(84.0, m.control)),
                    Elevation::Thumb => (s.thumb, None, vec2(84.0, m.control - 4.0)),
                    Elevation::Knob => (s.knob, s.knob_edge, Vec2::splat(m.switch_knob)),
                    Elevation::Tab => (s.tab, Some(s.tab_edge), vec2(84.0, m.doc_tab_height)),
                    Elevation::Primary => (s.primary, None, vec2(84.0, m.control)),
                    Elevation::Danger => (s.danger_solid, None, vec2(84.0, m.control)),
                    Elevation::Floating => (
                        Fill::flat(s.overlay),
                        Some(p.floating_ring(s)),
                        vec2(84.0, 56.0),
                    ),
                    Elevation::Pane => (Fill::flat(s.pane), s.pane_edge, vec2(84.0, 56.0)),
                    Elevation::Field => (
                        Fill::flat(s.field),
                        Some(p.control_border),
                        vec2(84.0, m.control),
                    ),
                    Elevation::Deck => (s.deck, None, vec2(84.0, 40.0)),
                };
                ui.vertical(|ui| {
                    let (slot, _) = ui.allocate_exact_size(vec2(96.0, 64.0), Sense::hover());
                    let rect = Rect::from_center_size(slot.center(), size);
                    let radius = if level == Elevation::Knob {
                        CornerRadius::same(u8::MAX)
                    } else {
                        CornerRadius::same(m.radius)
                    };
                    paint::surface(ui.painter(), rect, radius, p, style, level, body, ring);
                    caption(ui, &format!("{level:?}"));
                });
            }
        });
    }
}

/// The same slice (chrome bar, a row, a primary and a secondary button, a floating surface) in
/// Standard and in Deeper side by side, whatever style is active.
fn style_strip(ui: &mut Ui) {
    let density = theme::density();
    ui.horizontal_top(|ui| {
        for style in Style::ALL {
            ui.push_id(style, |ui| {
                theme::with_look(style, density, || slice(ui, style));
            });
            ui.add_space(theme::LG);
        }
    });
}

fn slice(ui: &mut Ui, style: Style) {
    let p = theme::palette(ui.ctx());
    let s = p.surfaces(style);
    let m = theme::metrics();
    ui.vertical(|ui| {
        caption(ui, style.label());
        let (rect, _) = ui.allocate_exact_size(vec2(260.0, 150.0), Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, s.bg);
        let bar = Rect::from_min_size(rect.min, vec2(rect.width(), 30.0));
        paint::deck(painter, bar, p, s);
        let pane =
            Rect::from_min_max(pos2(rect.left(), bar.bottom()), rect.max).shrink(m.pane_gutter);
        let pane = Rect::from_min_max(pos2(pane.left(), bar.bottom() + m.pane_gutter), pane.max);
        paint::pane(painter, pane, p, s.pane, 1.0);
        let row = Rect::from_min_size(
            pane.min + vec2(0.0, 8.0),
            vec2(pane.width(), m.history_row_height),
        );
        painter.rect_filled(row, CornerRadius::ZERO, theme::over(p.selected(), s.pane));
        painter.text(
            row.left_center() + vec2(12.0, 0.0),
            Align2::LEFT_CENTER,
            "Selected row",
            FontId::proportional(m.body_size),
            p.text,
        );
        let menu = Rect::from_min_size(pane.min + vec2(130.0, 46.0), vec2(110.0, 60.0));
        let radius = CornerRadius::same(m.radius_lg);
        let fill = super::glass::layer_fill(painter, menu, radius);
        paint::surface(
            painter,
            menu,
            radius,
            p,
            style,
            Elevation::Floating,
            Fill::flat(fill),
            Some(p.floating_ring(s)),
        );
        painter.text(
            menu.left_top() + vec2(10.0, 16.0),
            Align2::LEFT_CENTER,
            "Menu item",
            FontId::proportional(m.body_size),
            p.text,
        );
        let hi = Rect::from_min_size(menu.min + vec2(4.0, 28.0), vec2(menu.width() - 8.0, 24.0));
        painter.rect_filled(hi, CornerRadius::same(m.radius_sm), p.hover_strong());
        painter.text(
            hi.left_center() + vec2(6.0, 0.0),
            Align2::LEFT_CENTER,
            "Highlighted",
            FontId::proportional(m.body_size),
            p.text,
        );
        ui.add_space(theme::SM);
        ui.horizontal(|ui| {
            super::Button::new("Commit")
                .primary()
                .id_salt(("strip", style, 0))
                .show(ui);
            super::Button::new("Cancel")
                .id_salt(("strip", style, 1))
                .show(ui);
        });
    });
}

/// Each animated component at 0, 0.5 and 1 of its transition, painted with a fixed progress.
fn motion_strip(ui: &mut Ui) {
    for kind in [
        ButtonKind::Primary,
        ButtonKind::Secondary,
        ButtonKind::Ghost,
    ] {
        ui.horizontal(|ui| {
            for (i, t) in [0.0, 0.5, 1.0].into_iter().enumerate() {
                let force = ForceState {
                    hover: true,
                    progress: Some(t),
                    ..ForceState::default()
                };
                super::Button::new(format!("Hover {t:.1}"))
                    .kind(kind)
                    .id_salt(("motion", kind, i))
                    .show_forced(ui, force);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_past_the_end_show_the_last_page() {
        // Unknown size: as asked.
        assert_eq!(page_offset(3, None, 900.0), 2700.0);
        // 10000 tall in a 900 viewport: page 3 is 2700 down, page 16 stops at the end.
        assert_eq!(page_offset(3, Some((10000.0, 900.0)), 900.0), 2700.0);
        assert_eq!(page_offset(16, Some((10000.0, 900.0)), 900.0), 9100.0);
        assert_eq!(page_offset(16, Some((500.0, 900.0)), 900.0), 0.0);
    }

    /// QVIS-3, QV2-8: pages count from 1 and the last page is the count, however far past the end
    /// the screenshot script asks.
    #[test]
    fn pages_are_labelled_from_one() {
        // 10000 tall in a 900 viewport: twelve pages (the last stops at 9100).
        assert_eq!(page_of(0.0, 10000.0, 900.0), (1, 12));
        assert_eq!(page_of(2700.0, 10000.0, 900.0), (4, 12));
        assert_eq!(
            page_of(
                page_offset(40, Some((10000.0, 900.0)), 900.0),
                10000.0,
                900.0
            ),
            (12, 12)
        );
        assert_eq!(page_of(0.0, 500.0, 900.0), (1, 1), "all on one page");
    }
}
