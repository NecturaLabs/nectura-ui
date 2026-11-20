// Glass rendering and color-compositing regression tests.
#![expect(
    clippy::disallowed_methods,
    reason = "test backdrops are arbitrary colours, not the theme's"
)]
use egui::epaint::{ClippedShape, RectShape};

use super::*;
use crate::theme::{self, Appearance, Density, Style, ThemeId};

fn at(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect::from_min_max(pos2(x0, y0), pos2(x1, y1))
}

fn screen(w: f32, h: f32) -> Rect {
    at(0.0, 0.0, w, h)
}

/// Installs `theme` with Translucent surfaces as given and the blur as `backend`, on this test's
/// thread.
fn setup(ctx: &egui::Context, theme: ThemeId, glass: bool, backend: Backend) {
    theme::apply(
        ctx,
        theme,
        Appearance {
            glass,
            ..Appearance::STILL
        },
    );
    status(|s| {
        s.backend.store(backend.code(), Ordering::Relaxed);
        s.reason.store(0, Ordering::Relaxed);
        s.max_side.store(MAX_SIDE_CAP, Ordering::Relaxed);
    });
}

fn frame(ctx: &egui::Context, f: impl FnMut(&mut Ui)) -> Vec<ClippedShape> {
    let mut out = ctx.run_ui(egui::RawInput::default(), f);
    out.textures_delta.clear();
    out.shapes
}

fn callbacks(shapes: &[ClippedShape]) -> usize {
    shapes
        .iter()
        .filter(|c| matches!(c.shape, Shape::Callback(_)))
        .count()
}

/// Glass layers among `shapes`: each paints two callbacks, its capture and its composite.
fn glass_layers(shapes: &[ClippedShape]) -> usize {
    let n = callbacks(shapes);
    assert_eq!(n % 2, 0, "a capture without its composite: {n} callbacks");
    n / 2
}

#[test]
fn plan_flips_to_window_pixels_and_pads_the_region() {
    let plan = plan(
        at(100.0, 50.0, 300.0, 150.0),
        CornerRadius::same(12),
        1.0,
        [800, 600],
        screen(800.0, 600.0),
        4096,
    );
    let Some(plan) = plan else {
        panic!("a visible layer has a plan");
    };
    let bounds = PxRect {
        x: 100,
        y: 450,
        w: 200,
        h: 100,
    };
    assert_eq!(plan.bounds, bounds);
    assert_eq!(plan.scissor, bounds);
    assert_eq!(plan.rect, [200.0, 500.0, 100.0, 50.0]);
    assert_eq!(plan.radii, [12.0; 4]);
    let blur = plan.blur.unwrap_or_else(|| panic!("blurred"));
    assert_eq!(
        blur.region,
        PxRect {
            x: 64,
            y: 414,
            w: 272,
            h: 172
        }
    );
    assert_eq!(blur.passes, 3);
}

#[test]
fn plan_clamps_at_the_window_edges() {
    let Some(plan) = plan(
        at(-50.0, -20.0, 100.0, 80.0),
        CornerRadius::same(6),
        1.0,
        [800, 600],
        screen(800.0, 600.0),
        4096,
    ) else {
        panic!("partly visible");
    };
    assert_eq!(
        plan.bounds,
        PxRect {
            x: 0,
            y: 520,
            w: 100,
            h: 80
        }
    );
    let region = plan.blur.map(|b| b.region);
    assert_eq!(
        region,
        Some(PxRect {
            x: 0,
            y: 484,
            w: 136,
            h: 116
        })
    );
    // The bottom-right corner: the region stops at the window.
    let Some(plan) = plan_at(at(700.0, 550.0, 900.0, 700.0)) else {
        panic!("partly visible");
    };
    assert_eq!(
        plan.blur.map(|b| b.region),
        Some(PxRect {
            x: 664,
            y: 0,
            w: 136,
            h: 86
        })
    );
}

fn plan_at(layer: Rect) -> Option<Plan> {
    plan(
        layer,
        CornerRadius::same(8),
        1.0,
        [800, 600],
        screen(800.0, 600.0),
        4096,
    )
}

#[test]
fn plan_refuses_what_it_cannot_paint() {
    let nan = f32::NAN;
    for layer in [
        at(nan, 0.0, 10.0, 10.0),
        at(0.0, 0.0, f32::INFINITY, 10.0),
        at(10.0, 10.0, 10.0, 40.0),
        at(40.0, 10.0, 10.0, 40.0),
        at(900.0, 10.0, 950.0, 40.0),
        at(10.0, -90.0, 50.0, -10.0),
        at(-1e30, -1e30, 1e30, 1e30),
    ] {
        assert_eq!(plan_at(layer), None, "{layer:?}");
    }
    let layer = at(10.0, 10.0, 60.0, 60.0);
    for ppp in [0.0, -1.0, nan, f32::INFINITY, 100.0] {
        let p = plan(
            layer,
            CornerRadius::ZERO,
            ppp,
            [800, 600],
            screen(800.0, 600.0),
            4096,
        );
        assert_eq!(p, None, "ppp {ppp}");
    }
    let clipped_away = plan(
        layer,
        CornerRadius::ZERO,
        1.0,
        [800, 600],
        at(300.0, 300.0, 400.0, 400.0),
        4096,
    );
    assert_eq!(clipped_away, None);
    let no_window = plan(
        layer,
        CornerRadius::ZERO,
        1.0,
        [0, 600],
        screen(800.0, 600.0),
        4096,
    );
    assert_eq!(no_window, None);
    let huge_window = plan(
        layer,
        CornerRadius::ZERO,
        1.0,
        [u32::MAX, 600],
        screen(800.0, 600.0),
        4096,
    );
    assert_eq!(huge_window, None);
}

#[test]
fn plan_scales_with_the_pixel_density() {
    let Some(plan) = plan(
        at(100.0, 50.0, 300.0, 70.0),
        CornerRadius {
            nw: 14,
            ne: 4,
            sw: 0,
            se: 40,
        },
        2.0,
        [1600, 1200],
        screen(800.0, 600.0),
        4096,
    ) else {
        panic!("visible");
    };
    assert_eq!(
        plan.bounds,
        PxRect {
            x: 200,
            y: 1060,
            w: 400,
            h: 40
        }
    );
    // Radii in pixels, never past half the layer's height.
    assert_eq!(plan.radii, [20.0, 8.0, 0.0, 20.0]);
    let blur = plan.blur.unwrap_or_else(|| panic!("blurred"));
    assert_eq!(blur.region.x, 200 - 72);
    assert_eq!(blur.passes, 4);
}

#[test]
fn the_clip_limits_the_composite() {
    let Some(plan) = plan(
        at(100.0, 100.0, 300.0, 200.0),
        CornerRadius::ZERO,
        1.0,
        [800, 600],
        at(0.0, 150.0, 800.0, 600.0),
        4096,
    ) else {
        panic!("visible");
    };
    assert_eq!(
        plan.scissor,
        PxRect {
            x: 100,
            y: 400,
            w: 200,
            h: 50
        }
    );
}

#[test]
fn regions_too_small_or_too_large_are_covered_not_blurred() {
    let tiny = plan(
        at(0.0, 0.0, 4.0, 4.0),
        CornerRadius::ZERO,
        1.0,
        [6, 6],
        screen(6.0, 6.0),
        4096,
    );
    assert!(tiny.is_some_and(|p| p.blur.is_none()));
    let large = plan(
        at(0.0, 0.0, 3000.0, 100.0),
        CornerRadius::ZERO,
        1.0,
        [5000, 1000],
        screen(5000.0, 1000.0),
        2048,
    );
    assert!(large.is_some_and(|p| p.blur.is_none()));
}

#[test]
fn the_fallback_leaves_the_rounded_corners_alone() {
    let Some(plan) = plan_at(at(100.0, 100.0, 300.0, 200.0)) else {
        panic!("visible");
    };
    let [wide, tall] = plan.cross();
    assert_eq!(
        wide,
        Some(PxRect {
            x: 108,
            y: 400,
            w: 184,
            h: 100
        })
    );
    assert_eq!(
        tall,
        Some(PxRect {
            x: 100,
            y: 408,
            w: 200,
            h: 84
        })
    );
}

#[test]
fn textures_grow_in_steps_and_never_shrink() {
    assert_eq!(grow([300, 200], [0, 0], 4096), Some([512, 256]));
    assert_eq!(grow([100, 100], [512, 256], 4096), Some([512, 256]));
    assert_eq!(grow([600, 100], [512, 256], 4096), Some([768, 256]));
    assert_eq!(grow([4000, 10], [0, 0], 4096), Some([4096, 256]));
    assert_eq!(grow([900, 10], [0, 0], 1000), Some([1000, 256]));
    assert_eq!(grow([4097, 10], [0, 0], 4096), None);
    assert_eq!(grow([0, 10], [0, 0], 4096), None);
    assert_eq!(grow([-5, 10], [0, 0], 4096), None);
    assert_eq!(
        grow([i32::MAX, 10], [0, 0], i32::MAX),
        Some([i32::MAX, 256])
    );
}

#[test]
fn levels_halve_rounding_up() {
    assert_eq!(
        level_sizes([272, 172]),
        [[136, 86], [68, 43], [34, 22], [17, 11], [9, 6]]
    );
    assert_eq!(level_sizes([1, 1]), [[1, 1]; LEVELS]);
    assert_eq!(
        level_sizes([4096, 3]),
        [[2048, 2], [1024, 1], [512, 1], [256, 1], [128, 1]]
    );
}

#[test]
fn the_blur_is_about_twelve_points_at_any_density() {
    // Offsets from simulating the passes on an impulse (sigma 12 pt): 0.429 texels for three
    // passes at 1 pixel per point, the same for four passes at 2.
    let (passes, offset) = kawase(1.0);
    assert_eq!(passes, 3);
    assert!((offset - 0.429).abs() < 0.01, "{offset}");
    let (passes, offset) = kawase(2.0);
    assert_eq!(passes, 4);
    assert!((offset - 0.429).abs() < 0.01, "{offset}");
    let (_, offset) = kawase(1.5);
    assert!((offset - 0.715).abs() < 0.02, "{offset}");
    let mut last = 0.0;
    for ppp in [0.8, 1.0, 1.25, 1.5, 1.75, 1.99] {
        let (passes, offset) = kawase(ppp);
        assert_eq!(passes, 3);
        assert!(offset > last, "grows with density");
        last = offset;
    }
    for ppp in [f32::NAN, -1.0, 0.0, 100.0] {
        let (passes, offset) = kawase(ppp);
        assert!((3..=MAX_PASSES).contains(&passes));
        assert!((0.25..=1.2).contains(&offset));
    }
}

#[test]
fn glass_needs_the_switch_the_theme_the_driver_and_the_system() {
    let resolve = |theme, translucent, available, reduced| {
        Appearance::resolve(
            theme,
            Style::Standard,
            Density::Comfortable,
            translucent,
            false,
            available,
            reduced,
        )
        .glass
    };
    assert!(resolve(ThemeId::Midnight, true, true, None));
    assert!(resolve(ThemeId::Daylight, true, true, Some(false)));
    assert!(!resolve(ThemeId::Midnight, false, true, None));
    assert!(!resolve(ThemeId::Midnight, true, false, None));
    assert!(!resolve(ThemeId::Midnight, true, true, Some(true)));
    theme::set_high_contrast(true);
    assert!(!resolve(ThemeId::Midnight, true, true, None));
    theme::set_high_contrast(false);
    // Reduced motion leaves glass alone.
    let moving = Appearance::resolve(
        ThemeId::Midnight,
        Style::Deeper,
        Density::Dense,
        true,
        true,
        true,
        None,
    );
    assert!(moving.glass && moving.reduce_motion);
}

#[test]
fn without_the_blur_layers_are_opaque() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Midnight, true, Backend::Unprobed);
    let shapes = frame(&ctx, |ui| {
        let fill = layer_fill(
            ui.painter(),
            at(10.0, 10.0, 200.0, 100.0),
            CornerRadius::ZERO,
        );
        assert_eq!(fill, theme::surfaces(ui.ctx()).overlay);
        assert_eq!(fill.a(), 255);
    });
    assert_eq!(callbacks(&shapes), 0);
    assert!(!available());
}

#[test]
fn with_translucent_surfaces_off_layers_are_opaque() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Midnight, false, Backend::Supported);
    let shapes = frame(&ctx, |ui| {
        let fill = layer_fill(
            ui.painter(),
            at(10.0, 10.0, 200.0, 100.0),
            CornerRadius::ZERO,
        );
        assert_eq!(fill, theme::surfaces(ui.ctx()).overlay);
    });
    assert_eq!(callbacks(&shapes), 0);
}

#[test]
fn a_glass_layer_blurs_its_backdrop_under_a_translucent_fill() {
    let ctx = egui::Context::default();
    for theme in [ThemeId::Midnight, ThemeId::Daylight, ThemeId::Observatory] {
        setup(&ctx, theme, true, Backend::Supported);
        let shapes = frame(&ctx, |ui| {
            let fill = layer_fill(
                ui.painter(),
                at(10.0, 10.0, 200.0, 100.0),
                CornerRadius::ZERO,
            );
            let p = theme::palette(ui.ctx());
            assert_eq!(fill, p.glass(theme::surfaces(ui.ctx())));
            assert!(fill.a() < 255 && fill.a() >= 128, "{fill:?}");
        });
        assert_eq!(glass_layers(&shapes), 1, "{}", theme.name());
        let areas: Vec<Rect> = shapes
            .iter()
            .filter_map(|c| match &c.shape {
                Shape::Callback(cb) => Some(cb.rect),
                _ => None,
            })
            .collect();
        let padded = at(10.0, 10.0, 200.0, 100.0)
            .expand(PAD)
            .intersect(ctx.viewport_rect());
        assert_eq!(areas, [padded, padded]);
    }
}

#[test]
fn high_contrast_never_uses_glass() {
    let ctx = egui::Context::default();
    theme::set_high_contrast(true);
    setup(&ctx, ThemeId::Midnight, true, Backend::Supported);
    let shapes = frame(&ctx, |ui| {
        let fill = layer_fill_with(
            ui.painter(),
            at(10.0, 10.0, 200.0, 100.0),
            CornerRadius::ZERO,
            true,
        );
        assert_eq!(fill.a(), 255);
    });
    assert_eq!(callbacks(&shapes), 0);
}

#[test]
fn the_backdrop_moves_between_the_shadow_and_the_fill() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Daylight, true, Backend::Supported);
    let layer = at(40.0, 40.0, 240.0, 160.0);
    let shapes = frame(&ctx, |ui| {
        let p = theme::palette(ui.ctx());
        let s = theme::surfaces(ui.ctx());
        let radius = CornerRadius::same(10);
        let fill = layer_fill(ui.painter(), layer, radius);
        crate::kit::paint::surface(
            ui.painter(),
            layer,
            radius,
            p,
            Style::Standard,
            Elevation::Floating,
            Fill::flat(fill),
            Some(p.floating_ring(s)),
        );
    });
    let kinds: Vec<&str> = shapes
        .iter()
        .filter_map(|c| match &c.shape {
            Shape::Noop => Some("noop"),
            Shape::Callback(_) => Some("backdrop"),
            Shape::Rect(RectShape { blur_width, .. }) if *blur_width > 0.0 => Some("shadow"),
            Shape::Rect(r) if r.rect == layer && r.fill.a() > 0 => Some("fill"),
            _ => None,
        })
        .collect();
    // The capture stays before the shadow, so the shadow is not blurred into the glass; the
    // composite moves between the shadow and the fill.
    assert_eq!(
        kinds,
        ["backdrop", "noop", "shadow", "shadow", "backdrop", "fill"],
        "{kinds:?}"
    );
}

/// H18 / G3: menus, popovers, dialogs and toasts (`overlay::surface_shapes`) copy the backdrop
/// before their own shadow, so the glass never blurs the shadow under the card into it.
#[test]
fn floating_layers_capture_the_backdrop_before_their_shadow() {
    let ctx = egui::Context::default();
    for id in [ThemeId::Midnight, ThemeId::Daylight] {
        setup(&ctx, id, true, Backend::Supported);
        let layer = at(40.0, 40.0, 240.0, 160.0);
        for kind in [
            crate::kit::overlay::Layer::Menu,
            crate::kit::overlay::Layer::Popover,
            crate::kit::overlay::Layer::Dialog,
            crate::kit::overlay::Layer::Toast,
        ] {
            let mut kinds = Vec::new();
            let _ = frame(&ctx, |ui| {
                let shapes = crate::kit::overlay::surface_shapes(ui.painter(), layer, kind);
                kinds = shapes
                    .iter()
                    .flat_map(|s| match s {
                        Shape::Vec(v) => v.clone(),
                        other => vec![other.clone()],
                    })
                    .filter_map(|s| match &s {
                        Shape::Callback(_) => Some("backdrop"),
                        Shape::Rect(RectShape { blur_width, .. }) if *blur_width > 0.0 => {
                            Some("shadow")
                        }
                        Shape::Rect(r) if r.rect == layer && r.fill.a() > 0 => Some("fill"),
                        _ => None,
                    })
                    .collect();
            });
            let first_shadow = kinds.iter().position(|k| *k == "shadow");
            let fill = kinds.iter().position(|k| *k == "fill");
            assert_eq!(
                kinds.first(),
                Some(&"backdrop"),
                "{id:?} {kind:?}: {kinds:?}"
            );
            assert!(first_shadow.is_some() && fill.is_some(), "{kinds:?}");
            assert_eq!(
                kinds.iter().filter(|k| **k == "backdrop").count(),
                2,
                "{kinds:?}"
            );
            let composite = kinds.iter().rposition(|k| *k == "backdrop");
            assert!(
                composite > kinds.iter().rposition(|k| *k == "shadow") && composite < fill,
                "{id:?} {kind:?}: the composite sits between the shadow and the fill: {kinds:?}"
            );
        }
    }
}

#[test]
fn a_caller_placing_the_backdrop_gets_the_capture_and_the_composite() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Midnight, true, Backend::Supported);
    let layer = at(40.0, 40.0, 240.0, 160.0);
    let shapes = frame(&ctx, |ui| {
        let b = backdrop_shapes(ui.painter(), layer, CornerRadius::same(8));
        assert!(b.as_ref().is_some_and(|b| {
            matches!(b.capture, Shape::Callback(_)) && matches!(b.composite, Shape::Callback(_))
        }));
        // Each took one of the frame's glass layers.
        for _ in 1..MAX_REGIONS {
            assert!(backdrop_shapes(ui.painter(), layer, CornerRadius::ZERO).is_some());
        }
        assert!(backdrop_shapes(ui.painter(), layer, CornerRadius::ZERO).is_none());
    });
    // Nothing was painted: the caller places them.
    assert_eq!(callbacks(&shapes), 0);
}

#[test]
fn a_surface_at_another_rect_leaves_the_backdrop_where_it_was() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Midnight, true, Backend::Supported);
    let shapes = frame(&ctx, |ui| {
        let layer = at(40.0, 40.0, 240.0, 160.0);
        let _ = layer_fill(ui.painter(), layer, CornerRadius::ZERO);
        assert!(lift(ui.painter(), layer.translate(vec2(1.0, 0.0))).is_none());
    });
    assert_eq!(glass_layers(&shapes), 1);
}

#[test]
fn at_most_four_glass_layers_a_frame() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Midnight, true, Backend::Supported);
    for _ in 0..2 {
        let shapes = frame(&ctx, |ui| {
            let fills: Vec<u8> = (0..6)
                .map(|i| {
                    let r = at(10.0 + crate::int_f32(i) * 20.0, 10.0, 200.0, 100.0);
                    layer_fill(ui.painter(), r, CornerRadius::ZERO).a()
                })
                .collect();
            assert!(fills[..4].iter().all(|&a| a < 255), "{fills:?}");
            assert!(fills[4..].iter().all(|&a| a == 255), "{fills:?}");
        });
        // Each frame starts with the full allowance.
        assert_eq!(glass_layers(&shapes), usize::from(MAX_REGIONS));
    }
}

#[test]
fn layers_nobody_sees_take_no_allowance() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Midnight, true, Backend::Supported);
    let shapes = frame(&ctx, |ui| {
        let mut faded = ui.painter().clone();
        faded.set_opacity(0.0);
        assert_eq!(
            layer_fill(&faded, at(10.0, 10.0, 90.0, 90.0), CornerRadius::ZERO).a(),
            255
        );
        let clipped = ui.painter().with_clip_rect(at(0.0, 0.0, 5.0, 5.0));
        assert_eq!(
            layer_fill(&clipped, at(10.0, 10.0, 90.0, 90.0), CornerRadius::ZERO).a(),
            255
        );
        for i in 0..4 {
            let r = at(10.0 + crate::int_f32(i) * 10.0, 10.0, 90.0, 90.0);
            assert!(layer_fill(ui.painter(), r, CornerRadius::ZERO).a() < 255);
        }
    });
    assert_eq!(glass_layers(&shapes), 4);
}

#[test]
fn a_fading_layer_hands_its_opacity_to_the_blur() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Midnight, true, Backend::Supported);
    frame(&ctx, |ui| {
        let mut half = ui.painter().clone();
        half.set_opacity(0.5);
        assert!(backdrop_shapes(&half, at(10.0, 10.0, 90.0, 90.0), CornerRadius::ZERO).is_some());
    });
    let look = Look {
        dark: true,
        cap: 0.03,
        opacity: 0.5,
    };
    assert_eq!(look.uniform(), [SATURATION, 0.03, 1.0, 0.5]);
    let light = Look {
        dark: false,
        cap: 0.8,
        opacity: 1.0,
    };
    assert_eq!(light.uniform(), [SATURATION, 0.8, 0.0, 1.0]);
}

#[test]
fn a_failure_turns_glass_off_once() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Midnight, true, Backend::Ready);
    assert!(available());
    assert!(latch(Reason::Compile));
    assert!(!available());
    assert!(!latch(Reason::Panic), "only the first failure counts");
    assert!(
        describe().contains("rejected a blur shader"),
        "{}",
        describe()
    );
    let shapes = frame(&ctx, |ui| {
        assert_eq!(
            layer_fill(ui.painter(), at(10.0, 10.0, 90.0, 90.0), CornerRadius::ZERO).a(),
            255
        );
    });
    assert_eq!(callbacks(&shapes), 0);
    // Probing cannot bring it back.
    assert!(!advance(Backend::Unprobed, Backend::Supported));
}

#[test]
fn reasons_survive_their_code() {
    for reason in [
        Reason::NoContext,
        Reason::OldGl,
        Reason::Multisampled,
        Reason::SmallTextures,
        Reason::Compile,
        Reason::Link,
        Reason::Uniform,
        Reason::Framebuffer,
        Reason::Gl(0),
        Reason::ContextLost,
        Reason::Panic,
        Reason::Poisoned,
        Reason::Exited,
        Reason::Internal,
    ] {
        assert_eq!(Reason::from_code(reason.code()), Some(reason));
        assert!(
            !reason.describe().is_empty(),
            "{reason:?} has no description"
        );
    }
    assert_eq!(Reason::from_code(0), None);
    assert_eq!(Reason::Gl(0x0505).describe(), "OpenGL error 0x0505");
}

#[test]
fn resident_memory_is_read_from_proc_status() {
    let status = "Name:\tcanopy\nVmPeak:\t  900 kB\nVmRSS:\t   123456 kB\nThreads:\t9\n";
    assert_eq!(parse_vm_rss(status), Some(123_456));
    assert_eq!(parse_vm_rss("Name:\tcanopy\n"), None);
    assert_eq!(parse_vm_rss("VmRSS:\tlots kB\n"), None);
}

/// Relative luminance (WCAG) of an opaque colour.
fn luminance(c: Color32) -> f64 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// The composite shader's colour maths in Rust: saturation from Rec. 709 luma, then the luminance
/// band (the dark cap or the light lift), in linear light.
fn shade(c: Color32, dark: bool, cap: f32) -> Color32 {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.040_45 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped to 0..=1 first, so the channel is 0..=255"
    )]
    let srgb = |v: f64| {
        let s = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    let luma = |l: [f64; 3]| 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2];
    let mut l = [lin(c.r()), lin(c.g()), lin(c.b())];
    let y = luma(l);
    l = l.map(|x| (y + (x - y) * f64::from(SATURATION)).clamp(0.0, 1.0));
    let y = luma(l);
    let cap = f64::from(cap);
    if dark && y > cap {
        l = l.map(|x| x * cap / y);
    } else if !dark {
        l = l.map(|x| 1.0 - (1.0 - x) * (1.0 - cap));
    }
    Color32::from_rgb(srgb(l[0]), srgb(l[1]), srgb(l[2]))
}

#[test]
fn the_shader_clamp_keeps_the_backdrop_in_the_band_the_contrast_test_assumes() {
    let probes = [
        Color32::WHITE,
        Color32::BLACK,
        Color32::from_rgb(255, 0, 0),
        Color32::from_rgb(0, 255, 0),
        Color32::from_rgb(0, 0, 255),
        Color32::from_rgb(255, 255, 0),
        Color32::from_rgb(0, 255, 255),
        Color32::from_rgb(255, 0, 255),
        Color32::from_gray(128),
        Color32::from_rgb(200, 120, 40),
    ];
    for id in ThemeId::ALL {
        let p = id.palette();
        if p.glass_alpha == 0 {
            continue;
        }
        for probe in probes {
            let out = shade(probe, p.dark, p.glass_luminance);
            let y = luminance(out);
            let cap = f64::from(p.glass_luminance);
            if p.dark {
                assert!(y <= cap + 0.004, "{}: {probe:?} -> {out:?} ({y})", p.name);
            } else {
                assert!(y >= cap - 0.01, "{}: {probe:?} -> {out:?} ({y})", p.name);
            }
        }
        if p.dark {
            // Greys have nothing to saturate: the shader's clamp is `theme::clamp_backdrop`.
            for grey in [0, 60, 128, 200, 255] {
                let c = Color32::from_gray(grey);
                let ours = shade(c, p.dark, p.glass_luminance);
                let theirs = theme::clamp_backdrop(c, p.dark, p.glass_luminance);
                for (a, b) in ours.to_array().into_iter().zip(theirs.to_array()) {
                    assert!(
                        a.abs_diff(b) <= 1,
                        "{}: grey {grey}: {ours:?} vs {theirs:?}",
                        p.name
                    );
                }
            }
        } else {
            // Every channel of every backdrop ends at least as light as the darkest case the
            // theme's contrast test checks (black lifted to the floor), and the theme's inks are
            // all darker than glass over it: whatever lies behind, the glass is no darker than
            // that case, so its contrast holds.
            let floor = theme::clamp_backdrop(Color32::BLACK, false, p.glass_luminance);
            for r in (0..=255).step_by(15) {
                for g in (0..=255).step_by(15) {
                    for b in (0..=255).step_by(15) {
                        let c = Color32::from_rgb(r, g, b);
                        let out = shade(c, false, p.glass_luminance);
                        for (a, f) in out.to_array().into_iter().zip(floor.to_array()) {
                            assert!(
                                a.saturating_add(1) >= f,
                                "{}: {c:?} -> {out:?} under {floor:?}",
                                p.name
                            );
                        }
                    }
                }
            }
            let darkest = theme::over(p.glass(p.surfaces(Style::Standard)), floor);
            for ink in [
                p.text,
                p.text_dim,
                p.text_faint,
                p.accent,
                p.danger,
                p.focus,
                p.control_border,
            ] {
                assert!(
                    luminance(ink) < luminance(darkest),
                    "{}: {ink:?} is lighter than the glass",
                    p.name
                );
            }
        }
    }
}

#[test]
fn light_glass_keeps_the_shapes_behind_it() {
    // Text and lines dimmed under a scrim sit below a light theme's luminance floor; lifting them
    // as a whole keeps them apart, where raising each to the floor flattened them to one grey.
    let (line, ground) = (Color32::from_gray(40), Color32::from_gray(150));
    for floor in [0.45, 0.5, 0.8] {
        let (l, g) = (shade(line, false, floor), shade(ground, false, floor));
        assert!(g.g() > l.g(), "floor {floor}: {l:?} vs {g:?}");
        let flat = |c| theme::clamp_backdrop(c, false, floor);
        assert_eq!(flat(line), flat(ground), "floor {floor}");
    }
    let (l, g) = (shade(line, false, 0.5), shade(ground, false, 0.5));
    assert!(g.g() >= l.g() + 16, "{l:?} vs {g:?}");
}

#[test]
fn the_gallery_section_draws_both_cards() {
    let ctx = egui::Context::default();
    let section = |ui: &mut Ui| {
        ui.set_max_width(640.0);
        gallery(ui);
    };
    setup(&ctx, ThemeId::Midnight, false, Backend::Supported);
    let shapes = frame(&ctx, section);
    // The forced card is glass even with the setting off; the other stays opaque.
    assert_eq!(glass_layers(&shapes), 1);
    setup(&ctx, ThemeId::Midnight, false, Backend::Unavailable);
    let shapes = frame(&ctx, section);
    assert_eq!(callbacks(&shapes), 0);
    // Wider than the driver allows: opaque, never a translucent fill over raw content.
    setup(&ctx, ThemeId::Midnight, false, Backend::Supported);
    set_max_side(256);
    let shapes = frame(&ctx, section);
    assert_eq!(callbacks(&shapes), 0);
}

#[test]
fn the_soak_ends_after_its_frames() {
    for glass in [true, false] {
        let ctx = egui::Context::default();
        setup(&ctx, ThemeId::Midnight, glass, Backend::Supported);
        let mut soak = Soak::new(12, 6);
        let mut done = false;
        let mut most = 0;
        for _ in 0..18 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                done = soak.frame(ui.ctx());
            });
            out.textures_delta.clear();
            let n = glass_layers(&out.shapes);
            assert!(n <= usize::from(MAX_REGIONS));
            most = most.max(n);
        }
        assert!(done);
        // With the setting off the soak paints the same layers opaque: the baseline run.
        assert_eq!(most, if glass { usize::from(MAX_REGIONS) } else { 0 });
        // A frame after the last one changes nothing.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            done = soak.frame(ui.ctx());
        });
        out.textures_delta.clear();
        assert!(done);
        assert!(
            soak.report().starts_with("glass-soak: frames=12 warmup=6 "),
            "{}",
            soak.report()
        );
    }
}

#[test]
fn objects_are_deleted_only_in_their_context_on_their_thread() {
    let ours = std::thread::current().id();
    let other = std::thread::spawn(|| std::thread::current().id())
        .join()
        .unwrap_or(ours);
    assert_ne!(ours, other);
    // Addresses only: never dereferenced.
    let made_in = std::ptr::dangling::<glow::Context>();
    let elsewhere = made_in.wrapping_byte_add(64);
    assert!(gl::owns((made_in, ours), (made_in, ours)));
    assert!(!gl::owns((made_in, ours), (made_in, other)));
    assert!(!gl::owns((made_in, ours), (elsewhere, ours)));
}

#[test]
fn destroying_twice_is_harmless_and_turns_glass_off() {
    let ctx = egui::Context::default();
    setup(&ctx, ThemeId::Midnight, true, Backend::Supported);
    destroy(ExitContext::from_on_exit(None));
    destroy(ExitContext::from_on_exit(None));
    assert!(!available());
    assert!(describe().contains("window closed"), "{}", describe());
}
