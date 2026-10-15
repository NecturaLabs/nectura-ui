//! Small text and egui helpers the components share.

use egui::{Color32, FontId, Ui};

use crate::theme::Palette;

/// A painted one-line label that ends in an ellipsis instead of cutting through a glyph.
pub fn single_line_galley(
    ui: &Ui,
    text: String,
    font: FontId,
    color: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text, font, color, width.max(0.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    layout_tidy(ui.painter(), job)
}

/// `job` laid out; when it is cut, the ellipsis follows the last word, not a space.
/// egui cuts at a word boundary and keeps the space before its ellipsis ("Make the …").
pub fn layout_tidy(
    painter: &egui::Painter,
    job: egui::text::LayoutJob,
) -> std::sync::Arc<egui::Galley> {
    let galley = painter.layout_job(job.clone());
    let Some(ellipsis) = job.wrap.overflow_character.filter(|_| galley.elided) else {
        return galley;
    };
    let shown: String = galley.rows.iter().map(|row| row.text()).collect();
    let before = shown.strip_suffix(ellipsis).unwrap_or(&shown);
    if !before.ends_with(char::is_whitespace) {
        return galley;
    }
    let kept_chars = before.trim_end().chars().count();
    let kept = tidy_end(&job.text, kept_chars);
    let mut tidy = job;
    tidy.text.truncate(kept);
    tidy.sections.retain_mut(|section| {
        section.byte_range.end.0 = section.byte_range.end.0.min(kept);
        section.byte_range.start.0 < section.byte_range.end.0
    });
    let Some(last) = tidy.sections.last().map(|s| s.format.clone()) else {
        return galley;
    };
    tidy.append(&ellipsis.to_string(), 0.0, last);
    tidy.wrap.max_rows = usize::MAX;
    let mut galley = painter.layout_job(tidy);
    // The text is cut all the same: callers ask the galley.
    std::sync::Arc::make_mut(&mut galley).elided = true;
    galley
}

/// The byte offset after the first `chars` characters of `text`.
fn tidy_end(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(i, _)| i)
}

/// Removes tooltips from content below a newly drawn blocking overlay.
/// egui can keep an existing tooltip open while the pointer stays over its old widget.
pub fn dismiss_underlying_tooltips(ctx: &egui::Context) {
    let layers = ctx.memory(|memory| {
        memory
            .layer_ids()
            .filter(|layer| layer.order == egui::Order::Tooltip)
            .collect::<Vec<_>>()
    });
    ctx.graphics_mut(|graphics| {
        for layer in layers {
            if let Some(list) = graphics.get_mut(layer) {
                *list = egui::layers::PaintList::default();
            }
        }
    });
}

/// A stable colour for a displayed identity, shared by names and matching host handles.
pub fn person_color(p: &Palette, identity: &str) -> Color32 {
    let mut h: u32 = 2_166_136_261;
    let key: String = identity
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    for b in key.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(16_777_619);
    }
    p.lanes[(h as usize) % p.lanes.len()]
}

/// Up to two initials of a name.
pub fn initials(name: &str) -> String {
    let mut out: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect();
    if out.is_empty() {
        out.push('?');
    }
    out.to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit;
    use crate::theme::{self, palette};
    use egui_phosphor::regular as icon;

    #[test]
    fn a_cut_label_ends_its_last_word_with_the_ellipsis_and_no_space() {
        let ctx = kit::testkit::ctx();
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        for step in 0..52_u16 {
            let width = 40.0 + f32::from(step) * 5.0;
            let mut cut = String::new();
            kit::testkit::frame(&ctx, Vec::new(), |ui| {
                let mut job = egui::text::LayoutJob::simple(
                    text.to_owned(),
                    FontId::proportional(13.0),
                    Color32::WHITE,
                    width,
                );
                job.wrap.max_rows = 1;
                let g = layout_tidy(ui.painter(), job);
                cut = g.rows.iter().map(|r| r.text()).collect();
                assert_eq!(g.rows.len(), 1, "one row at {width}");
            });
            assert!(!cut.contains(" …"), "{width}: {cut:?}");
            assert!(cut == text || cut.ends_with('…'), "{width}: {cut:?}");
        }
    }

    #[test]
    fn controls_keep_keyboard_activation_and_disabled_guards() {
        for kind in 0..4 {
            for enabled in [false, true] {
                let ctx = egui::Context::default();
                crate::fonts::install(&ctx);
                theme::apply(&ctx, theme::ThemeId::Midnight, theme::Appearance::STILL);
                let mut checked = false;
                let mut activated = false;
                let mut focused = false;
                for frame in 0..5 {
                    let events = if frame == 2 {
                        vec![egui::Event::Key {
                            key: egui::Key::Space,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::default(),
                        }]
                    } else {
                        Vec::new()
                    };
                    ctx.run_ui(
                        egui::RawInput {
                            events,
                            ..Default::default()
                        },
                        |ui| {
                            ui.add_enabled_ui(enabled, |ui| {
                                let response = match kind {
                                    0 => kit::toggle::Checkbox::new(&mut checked, "Checkbox")
                                        .show(ui),
                                    1 => kit::Button::new("Primary")
                                        .primary()
                                        .min_width(120.0)
                                        .show(ui),
                                    2 => kit::ToolButton::new(icon::CHECK, "Tool")
                                        .tooltip("Tool action")
                                        .show(ui),
                                    _ => kit::IconButton::new(icon::CHECK, "Icon action").show(ui),
                                };
                                if frame == 0 && enabled {
                                    response.request_focus();
                                }
                                activated |= response.clicked();
                                focused |= response.has_focus();
                            });
                        },
                    )
                    .drop_without_applying_deltas();
                }
                assert_eq!(activated, enabled, "control {kind}, enabled {enabled}");
                assert_eq!(focused, enabled, "control {kind}, enabled {enabled}");
                if kind == 0 {
                    assert_eq!(checked, enabled);
                }
            }
        }
    }

    #[test]
    fn blocking_overlay_hides_underlying_tooltips_and_keeps_its_own() {
        let ctx = egui::Context::default();
        for frame in 0..3 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                egui::Tooltip::always_open(
                    ctx.clone(),
                    ui.layer_id(),
                    egui::Id::new("underlying"),
                    egui::pos2(20.0, 20.0),
                )
                .show(|ui| {
                    ui.label("Underlying tooltip");
                });
                dismiss_underlying_tooltips(&ctx);
                egui::Modal::new(egui::Id::new("blocking-overlay")).show(&ctx, |ui| {
                    ui.label("Overlay content");
                    egui::Tooltip::always_open(
                        ctx.clone(),
                        ui.layer_id(),
                        egui::Id::new("overlay-tooltip"),
                        egui::pos2(100.0, 100.0),
                    )
                    .show(|ui| {
                        ui.label("Overlay tooltip");
                    });
                });
            });
            output.textures_delta.clear();
            if frame < 2 {
                continue;
            }
            let text: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some(text.galley.text())
                    } else {
                        None
                    }
                })
                .collect();
            assert!(text.contains(&"Overlay content"));
            assert!(text.contains(&"Overlay tooltip"));
            assert!(
                !text.contains(&"Underlying tooltip"),
                "background tooltip painted above overlay"
            );
        }
    }

    #[test]
    fn segmented_dims_the_options_that_are_not_selected() {
        let ctx = egui::Context::default();
        crate::fonts::install(&ctx);
        let mut value = 0;
        for frame in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                kit::choice::Segmented::new(&mut value, &[(0, "One"), (1, "Two")]).show(ui);
            });
            output.textures_delta.clear();
            if frame == 0 {
                continue;
            }
            let ink = |label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(t) if t.galley.text() == label => {
                            let own = t.galley.job.sections[0].format.color;
                            Some(if own == Color32::PLACEHOLDER {
                                t.fallback_color
                            } else {
                                own
                            })
                        }
                        _ => None,
                    })
                    .expect(label)
            };
            let p = palette(&ctx);
            assert_eq!(ink("One"), p.text);
            assert_eq!(ink("Two"), p.text_dim);
        }
    }

    #[test]
    fn initials_of_a_name() {
        assert_eq!(initials("ada lovelace"), "AL");
        assert_eq!(initials("Linus"), "L");
        assert_eq!(initials(""), "?");
    }
}
