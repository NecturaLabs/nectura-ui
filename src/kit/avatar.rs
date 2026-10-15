//! [`Avatar`] and [`AvatarStack`] (design spec 4.19): a person's picture, or their initial on a
//! tint of their colour, as a circle.
//!
//! The picture comes from the avatar cache (allowed hosts, 12 hour cache) as a texture; until it
//! arrives the initial shows, and the picture crossfades in over `BASE` when it does.

use egui::emath::GuiRounding as _;
use egui::epaint::RectShape;
use egui::{
    Align2, Color32, CornerRadius, FontId, Id, Painter, Pos2, Rect, Response, Sense, TextureId, Ui,
    Vec2, Widget, WidgetInfo, WidgetType, pos2, vec2,
};

use super::anim;
use super::chip::Badge;
use super::overlay::TooltipExt;
use crate::motion;
use crate::text;
use crate::theme::{self, Palette};

/// The avatar sizes of the spec: rows and lists, the detail pane, the author card's header, and
/// the author card itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AvatarSize {
    /// History rows and lists (`avatar_row`, 18 / 16).
    #[default]
    Row,
    /// The commit detail pane (`avatar_detail`, 24).
    Detail,
    /// Large (`avatar_lg`, 30).
    Large,
    /// The author card (48).
    Card,
}

impl AvatarSize {
    pub const ALL: [Self; 4] = [Self::Row, Self::Detail, Self::Large, Self::Card];

    /// The diameter in the active metrics.
    pub fn diameter(self) -> f32 {
        let m = theme::metrics();
        match self {
            Self::Row => m.avatar_row,
            Self::Detail => m.avatar_detail,
            Self::Large => m.avatar_lg,
            Self::Card => 48.0,
        }
    }
}

/// Who an avatar shows: the name gives the initial and the colour (the email when the name is
/// blank), and the picture replaces both once it has loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
    pub picture: Option<TextureId>,
}

impl Person {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            email: email.into(),
            picture: None,
        }
    }

    #[must_use]
    pub fn picture(mut self, picture: Option<TextureId>) -> Self {
        self.picture = picture;
        self
    }

    /// The identity the colour comes from.
    fn identity(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.email
        } else {
            &self.name
        }
    }

    /// The name spoken by screen readers and listed in tooltips.
    pub fn display_name(&self) -> &str {
        let name = self.name.trim();
        if name.is_empty() {
            self.email.trim()
        } else {
            name
        }
    }
}

/// The single initial an avatar shows.
pub fn initial(person: &Person) -> String {
    let source = if person.name.trim().is_empty() {
        person.email.as_str()
    } else {
        person.name.as_str()
    };
    text::initials(source).chars().take(1).collect()
}

/// The fill and letter colours of an initials avatar: the person's colour at 24% over `pane`, and
/// the colour moved 22% towards `text`, then far enough to read on that fill.
pub fn colors(p: &Palette, person: &Person) -> (Color32, Color32) {
    let c = text::person_color(p, person.identity());
    let s = p.s();
    let fill = theme::over(Palette::avatar_fill(c), s.pane);
    let letter = p.ink(theme::mix(c, p.text, 0.22), Palette::avatar_fill(c));
    (fill, letter)
}

/// Paints an avatar of `diameter` centred on `center`. `ring` draws a 1.5 pt ring in the surface
/// behind (over the graph or in a stack). `id` keys the picture's crossfade; without one the
/// picture shows at once.
pub fn paint(
    painter: &Painter,
    center: Pos2,
    diameter: f32,
    person: &Person,
    ring: Option<Color32>,
    id: Option<Id>,
) {
    let ctx = painter.ctx();
    let p = theme::palette(ctx);
    let ppp = painter.pixels_per_point();
    let center = center.round_to_pixel_center(ppp);
    let r = diameter / 2.0;
    if let Some(ring) = ring {
        painter.circle_filled(center, r + RING, ring);
    }
    let shown = match id {
        Some(id) => anim::ease_bool(
            ctx,
            id.with("kit-avatar-picture"),
            person.picture.is_some(),
            motion::BASE,
            0.0,
        ),
        None => f32::from(u8::from(person.picture.is_some())),
    };
    if shown < 1.0 {
        let (fill, letter) = colors(p, person);
        painter.circle_filled(center, r, fill);
        painter.text(
            center,
            Align2::CENTER_CENTER,
            initial(person),
            initial_font(diameter),
            letter,
        );
    }
    if let Some(tex) = person.picture
        && shown > 0.0
    {
        let rect = Rect::from_center_size(center, Vec2::splat(diameter));
        let radius = CornerRadius::same(crate::saturate_u8(r.ceil().min(f32::from(u8::MAX))));
        painter.add(
            RectShape::filled(rect, radius, Color32::WHITE.gamma_multiply(shown))
                .with_texture(tex, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
        );
    }
}

/// Width of the ring an avatar wears over the graph or in a stack.
const RING: f32 = 1.5;
/// The share of an avatar the next one in a stack covers, at most.
const STACK_OVERLAP: f32 = 0.3;
/// The clear space between a stacked initial and the next avatar's ring.
const STACK_CLEAR: f32 = 1.0;

/// The initial's font in an avatar of `diameter`.
fn initial_font(diameter: f32) -> FontId {
    FontId::new((diameter * 0.55).round(), theme::semibold())
}

// ------------------------------------------------------------------------------------- Avatar

/// A person's avatar. `Avatar::new(Person::new(name, email)).size(AvatarSize::Detail).show(ui)`.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct Avatar {
    person: Person,
    size: AvatarSize,
    ring: Option<Color32>,
    tooltip: bool,
    id_salt: Option<Id>,
}

impl Avatar {
    pub fn new(person: Person) -> Self {
        Self {
            person,
            size: AvatarSize::Row,
            ring: None,
            tooltip: false,
            id_salt: None,
        }
    }

    pub fn size(mut self, size: AvatarSize) -> Self {
        self.size = size;
        self
    }

    /// A 1.5 pt ring in `ground`, the surface the avatar sits on (over the graph or in a stack).
    pub fn ring(mut self, ground: Color32) -> Self {
        self.ring = Some(ground);
        self
    }

    /// Shows the person's name on hover.
    pub fn tooltip(mut self, on: bool) -> Self {
        self.tooltip = on;
        self
    }

    pub fn id_salt(mut self, salt: impl std::hash::Hash + std::fmt::Debug) -> Self {
        self.id_salt = Some(Id::new(salt));
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let d = self.size.diameter();
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(d), Sense::hover());
        let id = self.id_salt.map_or(response.id, |s| ui.id().with(s));
        if ui.is_rect_visible(rect) {
            paint(
                ui.painter(),
                rect.center(),
                d,
                &self.person,
                self.ring,
                Some(id),
            );
        }
        let name = self.person.display_name().to_owned();
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Image, true, &name));
        if self.tooltip && !name.is_empty() {
            response.kit_tooltip(name).show()
        } else {
            response
        }
    }
}

impl Widget for Avatar {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// -------------------------------------------------------------------------------- AvatarStack

/// Overlapping avatars of several people (up to 30% overlap, less where an initial would be cut:
/// each initial keeps `STACK_CLEAR` from the next avatar's ring), at most three and a `+N` badge;
/// the tooltip lists everyone.
#[derive(Debug)]
#[must_use = "call .show(ui) or ui.add(..)"]
pub struct AvatarStack {
    people: Vec<Person>,
    size: AvatarSize,
    max: usize,
    ground: Option<Color32>,
}

impl AvatarStack {
    pub fn new(people: Vec<Person>) -> Self {
        Self {
            people,
            size: AvatarSize::Row,
            max: 3,
            ground: None,
        }
    }

    pub fn size(mut self, size: AvatarSize) -> Self {
        self.size = size;
        self
    }

    /// How many avatars show before the `+N` badge (3).
    pub fn max(mut self, max: usize) -> Self {
        self.max = max.max(1);
        self
    }

    /// The surface the stack sits on, for the rings between avatars (default `pane`).
    pub fn ground(mut self, ground: Color32) -> Self {
        self.ground = Some(ground);
        self
    }

    /// How many people show as avatars, and how many the badge counts.
    fn split(&self) -> (usize, usize) {
        let shown = self.people.len().min(self.max);
        (shown, self.people.len() - shown)
    }

    /// How far apart the avatars' centres are: `STACK_OVERLAP` of a diameter covered, or less, so
    /// the widest initial among the covered avatars ends `STACK_CLEAR` before the next ring.
    fn step(&self, ui: &Ui, shown: usize) -> f32 {
        let d = self.size.diameter();
        let font = initial_font(d);
        let widest = self.people[..shown.saturating_sub(1)]
            .iter()
            .filter(|p| p.picture.is_none())
            .map(|p| {
                ui.painter()
                    .layout_no_wrap(initial(p), font.clone(), Color32::PLACEHOLDER)
                    .rect
                    .width()
            })
            .fold(0.0, f32::max);
        let clear = if widest > 0.0 {
            d / 2.0 + widest / 2.0 + STACK_CLEAR + RING
        } else {
            0.0
        };
        (d * (1.0 - STACK_OVERLAP)).max(clear).min(d + RING)
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        let ground = self.ground.unwrap_or(theme::surfaces(ui.ctx()).pane);
        let d = self.size.diameter();
        let (shown, hidden) = self.split();
        let step = self.step(ui, shown);
        let avatars_w = if shown == 0 {
            0.0
        } else {
            d + step * crate::count_f32(shown - 1)
        };
        let badge = (hidden > 0).then(|| Badge::new(format!("+{hidden}")));
        let badge_w = badge.as_ref().map_or(0.0, |b| theme::XS + b.measure(ui).x);
        let height = d.max(theme::metrics().badge_height);
        let (rect, response) =
            ui.allocate_exact_size(vec2(avatars_w + badge_w, height), Sense::hover());
        if ui.is_rect_visible(rect) {
            // Each avatar sits over the one before it, its ring cutting a clean edge.
            for i in 0..shown {
                let c = pos2(
                    rect.left() + d / 2.0 + step * crate::count_f32(i),
                    rect.center().y,
                );
                paint(
                    ui.painter(),
                    c,
                    d,
                    &self.people[i],
                    Some(ground),
                    Some(response.id.with(i)),
                );
            }
            if let Some(b) = &badge {
                let size = b.measure(ui);
                let at = Rect::from_min_size(
                    pos2(
                        rect.left() + avatars_w + theme::XS,
                        rect.center().y - size.y / 2.0,
                    ),
                    size,
                );
                b.paint(ui, at);
            }
        }
        let names: Vec<&str> = self.people.iter().map(Person::display_name).collect();
        let label = names.join(", ");
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Image, true, &label));
        if label.is_empty() {
            response
        } else {
            response.kit_tooltip(names.join("\n")).show()
        }
    }
}

impl Widget for AvatarStack {
    fn ui(self, ui: &mut Ui) -> Response {
        self.show(ui)
    }
}

// ------------------------------------------------------------------------------------ gallery

/// The gallery's avatar rows: every size with initials and with a picture placeholder, a ring
/// over a coloured ground, and stacks.
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
    let people = [
        Person::new("Maya Chen", "maya@example.com"),
        Person::new("Priya Raman", "priya@example.com"),
        Person::new("Lucía Ortega", "lucia@example.com"),
        Person::new("Jonas Weber", "jonas@example.com"),
        Person::new("Sam Okafor", "sam@example.com"),
        Person::new("", "ci-bot@example.com"),
    ];
    small(ui, "Sizes: row, detail, large, card (initials)");
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SM;
        for (i, size) in AvatarSize::ALL.into_iter().enumerate() {
            Avatar::new(people[i].clone())
                .size(size)
                .tooltip(true)
                .id_salt(("gallery-avatar", i))
                .show(ui);
        }
        ui.add_space(theme::MD);
        for (i, person) in people.iter().enumerate() {
            Avatar::new(person.clone())
                .size(AvatarSize::Detail)
                .id_salt(("gallery-avatar-people", i))
                .show(ui);
        }
    });
    small(
        ui,
        "Picture (placeholder texture), ringed over a lane-coloured ground",
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::SM;
        let tex = placeholder_picture(ui.ctx());
        for (i, size) in AvatarSize::ALL.into_iter().enumerate() {
            Avatar::new(people[i].clone().picture(Some(tex)))
                .size(size)
                .id_salt(("gallery-avatar-pic", i))
                .show(ui);
        }
        ui.add_space(theme::MD);
        let ground = theme::over(p.selected(), theme::surfaces(ui.ctx()).pane);
        egui::Frame::new()
            .fill(ground)
            .corner_radius(CornerRadius::same(m.radius_sm))
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                Avatar::new(people[2].clone())
                    .ring(ground)
                    .id_salt("gallery-avatar-ring")
                    .show(ui);
            });
    });
    small(
        ui,
        "Stacks: two people, three, five (+2), at most two on the chrome (+3)",
    );
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme::LG;
        AvatarStack::new(people[..2].to_vec()).show(ui);
        AvatarStack::new(people[..5].to_vec())
            .max(2)
            .ground(theme::surfaces(ui.ctx()).panel)
            .show(ui);
        AvatarStack::new(people[..3].to_vec())
            .size(AvatarSize::Detail)
            .show(ui);
        AvatarStack::new(people[..5].to_vec())
            .size(AvatarSize::Detail)
            .show(ui);
    });
}

/// A small generated picture for the gallery (no network, no files): a diagonal two-tone square.
fn placeholder_picture(ctx: &egui::Context) -> TextureId {
    const N: usize = 32;

    let id = Id::new("kit-gallery-avatar-picture");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t.id();
    }
    let p = theme::palette(ctx);
    let (a, b) = (p.lanes[0], p.lanes[3]);
    let pixels = (0..N * N)
        .map(|i| {
            let (x, y) = (i % N, i / N);
            a.lerp_to_gamma(b, crate::count_f32(x + y) / crate::count_f32(2 * N - 2))
        })
        .collect();
    let image = egui::ColorImage::new([N, N], pixels);
    let handle = ctx.load_texture(
        "kit-gallery-avatar-picture",
        image,
        egui::TextureOptions::LINEAR,
    );
    let tex = handle.id();
    ctx.data_mut(|d| d.insert_temp(id, handle));
    tex
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

    #[test]
    fn the_initial_comes_from_the_name_then_the_email() {
        assert_eq!(initial(&Person::new("maya chen", "m@x")), "M");
        assert_eq!(initial(&Person::new("  ", "ci-bot@example.com")), "C");
        assert_eq!(initial(&Person::new("", "")), "?");
    }

    #[test]
    fn letters_read_on_their_fill_in_every_theme() {
        for id in theme::ThemeId::ALL {
            let p = id.palette();
            for name in [
                "Maya Chen",
                "Priya Raman",
                "Lucía Ortega",
                "Jonas Weber",
                "Sam",
            ] {
                let person = Person::new(name, "");
                let (fill, letter) = colors(p, &person);
                assert!(
                    theme::contrast(letter, fill) >= 4.5,
                    "{} {name}: {:.2}",
                    p.name,
                    theme::contrast(letter, fill)
                );
            }
        }
    }

    #[test]
    fn avatars_follow_the_metrics_and_announce_the_person() {
        let ctx = ctx();
        ctx.enable_accesskit();
        for density in Density::ALL {
            theme::apply(
                &ctx,
                theme::ThemeId::Midnight,
                theme::Appearance {
                    density,
                    ..theme::Appearance::STILL
                },
            );
            let mut sizes = Vec::new();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                for size in AvatarSize::ALL {
                    sizes.push(
                        Avatar::new(Person::new("Maya Chen", "maya@example.com"))
                            .size(size)
                            .show(ui)
                            .rect
                            .width(),
                    );
                }
            });
            out.textures_delta.clear();
            let m = theme::metrics();
            assert_eq!(
                sizes,
                vec![m.avatar_row, m.avatar_detail, m.avatar_lg, 48.0]
            );
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
            assert!(labels.iter().any(|l| l == "Maya Chen"), "{labels:?}");
        }
    }

    #[test]
    fn a_stack_shows_three_and_counts_the_rest() {
        let people: Vec<Person> = (0..5).map(|i| Person::new(format!("P{i}"), "")).collect();
        assert_eq!(AvatarStack::new(people.clone()).split(), (3, 2));
        assert_eq!(AvatarStack::new(people[..2].to_vec()).split(), (2, 0));
        assert_eq!(AvatarStack::new(people).max(1).split(), (1, 4));
    }

    #[test]
    fn a_stacked_initial_stays_clear_of_the_next_avatar() {
        use crate::kit::testkit;
        let people = vec![
            Person::new("Maya Chen", ""),
            Person::new("Wendy Moss", ""),
            Person::new("Mo", ""),
        ];
        for density in Density::ALL {
            let ctx = testkit::ctx_with(theme::ThemeId::Midnight, theme::Style::Standard, density);
            for size in AvatarSize::ALL {
                let shapes = testkit::frame(&ctx, Vec::new(), |ui| {
                    AvatarStack::new(people.clone()).size(size).show(ui);
                });
                let r = size.diameter() / 2.0;
                let rings: Vec<f32> = shapes
                    .iter()
                    .filter_map(|c| match &c.shape {
                        egui::Shape::Circle(k) if k.radius > r => Some(k.center.x - k.radius),
                        _ => None,
                    })
                    .collect();
                let (_, inks) = testkit::painted_colors(&shapes);
                assert_eq!((rings.len(), inks.len()), (3, 3), "{density:?} {size:?}");
                for i in 0..2 {
                    let right = inks[i].0.right();
                    assert!(
                        right <= rings[i + 1],
                        "{density:?} {size:?}: initial {i} ends at {right:.1}, \
                         the next ring starts at {:.1}",
                        rings[i + 1]
                    );
                }
            }
        }
    }

    #[test]
    fn pictures_appear_at_once_with_reduced_motion() {
        let ctx = ctx();
        let tex = placeholder_picture(&ctx);
        let person = Person::new("Maya", "").picture(Some(tex));
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            Avatar::new(person.clone()).id_salt("a").show(ui);
        });
        out.textures_delta.clear();
        // The picture is painted at full strength on the first frame, with no initials under it.
        let textured: Vec<Color32> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.brush.is_some() => Some(r.fill),
                _ => None,
            })
            .collect();
        assert_eq!(textured, vec![Color32::WHITE]);
        assert!(
            !out.shapes
                .iter()
                .any(|c| matches!(c.shape, egui::Shape::Circle(_)))
        );
    }
}
