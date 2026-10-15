//! Bundled fonts: Inter for the interface, JetBrains Mono NL (no ligatures) for code and ids,
//! Phosphor for icons (regular, and filled for themes that fill active icons), Atkinson
//! Hyperlegible Bold for headings at high contrast, and Canopy CJK, the fallback for Chinese,
//! Japanese and Korean text.
//! Licences: `assets/fonts/README.md`.

use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

use super::theme::HeadingFace;

/// Font data and family name of Phosphor's filled glyphs.
pub const FILL: &str = "phosphor-fill";

/// Font data name of the CJK fallback: Canopy CJK, everyday Han, kana and Hangul from Noto Sans
/// CJK JP (assets/fonts/README.md).
const CJK: &str = "canopy-cjk";

/// Installs the fonts on `ctx`.
pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let mut add = |name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add("inter", include_bytes!("../assets/fonts/Inter-Regular.ttf"));
    add(
        "inter-medium",
        include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    );
    add(
        "inter-semibold",
        include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    );
    add(
        "jetbrains-mono",
        include_bytes!("../assets/fonts/JetBrainsMonoNL-Regular.ttf"),
    );
    add(
        "atkinson-hyperlegible-bold",
        include_bytes!("../assets/fonts/AtkinsonHyperlegible-Bold.ttf"),
    );
    add(CJK, include_bytes!("../assets/fonts/CanopyCJK-Regular.otf"));

    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // The filled glyphs share the regular ones' code points, so they get a family of their own
    // (`theme::icon_fill`): themes with `IconWeight::FillAccents` draw active icons in it.
    fonts.font_data.insert(
        FILL.to_owned(),
        Arc::new(FontData::from_static(
            egui_phosphor::Variant::Fill.font_bytes(),
        )),
    );

    // Phosphor icons live in the Unicode private-use area. The bundled Inter files have their own
    // private-use mappings removed (see assets/fonts/README.md) and Phosphor comes straight after
    // Inter, ahead of egui's emoji font, which also uses that area. egui's defaults stay as
    // fallbacks for scripts and emoji Inter lacks, and Canopy CJK comes last of all, for the Han,
    // kana and Hangul in file names, branches and messages.
    let mut fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|f| f != "phosphor")
        .collect();
    fallbacks.push(CJK.to_owned());
    let with = |primary: &str| {
        let mut v = vec![primary.to_owned(), "phosphor".to_owned()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, with("inter"));
    fonts
        .families
        .insert(FontFamily::Name("medium".into()), with("inter-medium"));
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), with("inter-semibold"));
    // The heading face covers Latin only: anything else falls back to Inter SemiBold, then the
    // icons and egui's fonts.
    let mut chain = vec!["atkinson-hyperlegible-bold".to_owned()];
    chain.extend(with("inter-semibold"));
    fonts
        .families
        .insert(HeadingFace::Hyperlegible.family(), chain);
    // Inter first (Phosphor maps letters to the blank glyphs its ligatures use), then the filled
    // icons in place of the regular ones.
    let mut fill = vec!["inter".to_owned(), FILL.to_owned()];
    fill.extend(fallbacks.iter().cloned());
    fonts.families.insert(FontFamily::Name(FILL.into()), fill);
    if let Some(mono) = fonts.families.get_mut(&FontFamily::Monospace) {
        // Icons inside monospace text (branch names in pills) come from Phosphor, ahead of the
        // default monospace font's own private-use glyphs.
        mono.retain(|f| f != "phosphor");
        mono.insert(0, "jetbrains-mono".to_owned());
        mono.insert(1, "phosphor".to_owned());
        mono.push(CJK.to_owned());
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use egui::FontId;

    use super::*;

    /// The heading face and the filled icons are registered as families of their own, so titles
    /// at high contrast and a filled icon draw in their own faces, not Inter's.
    #[test]
    fn theme_faces_and_filled_icons_are_installed() {
        let ctx = egui::Context::default();
        install(&ctx);
        let fill = FontFamily::Name(FILL.into());
        let title = "Commit history";
        let mut widths = Vec::new();
        // Laying out text in a family that is not installed panics.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let width = |text: &str, font: FontId| {
                ui.painter()
                    .layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE)
                    .size()
                    .x
            };
            widths = vec![
                width(title, FontId::new(20.0, HeadingFace::Ui.family())),
                width(title, FontId::new(20.0, HeadingFace::Hyperlegible.family())),
                width(
                    egui_phosphor::regular::STAR,
                    FontId::new(20.0, fill.clone()),
                ),
                width("Fill", FontId::new(20.0, fill.clone())),
                width("Fill", FontId::proportional(20.0)),
            ];
        });
        out.textures_delta.clear();
        let [inter, hyper, star, fill_text, text] = widths[..] else {
            panic!("{widths:?}");
        };
        assert!(
            hyper > 0.0 && (hyper - inter).abs() > 1.0,
            "hyperlegible draws as Inter"
        );
        assert!(star > 0.0);
        // Text in the filled family stays Inter (Phosphor maps letters to blank ligature glyphs).
        assert!((fill_text - text).abs() < 0.01);
    }

    /// Chinese, Japanese and Korean text (file names, branch names, commit messages are user data)
    /// draws real glyphs in every family instead of the missing-glyph box.
    #[test]
    fn cjk_text_draws_real_glyphs() {
        let ctx = egui::Context::default();
        install(&ctx);
        let families = [
            FontFamily::Proportional,
            FontFamily::Monospace,
            FontFamily::Name("semibold".into()),
            HeadingFace::Hyperlegible.family(),
        ];
        let mut missing = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            for family in &families {
                let font = FontId::new(14.0, family.clone());
                let glyph = |c: char| {
                    let galley = ui.painter().layout_no_wrap(
                        c.to_string(),
                        font.clone(),
                        egui::Color32::WHITE,
                    );
                    let g = galley.rows[0].glyphs[0];
                    (g.uv_rect.offset, g.uv_rect.size)
                };
                // A private-use code point in plane 16 that no bundled font maps.
                let unknown = glyph('\u{10FFFD}');
                for c in [
                    '日', '本', '語', '中', '文', '體', '한', '국', 'ひ', 'カ', '「', '。',
                ] {
                    let g = glyph(c);
                    if g == unknown || g.1 == egui::emath::Vec2::ZERO {
                        missing.push(format!("{family:?} {c}"));
                    }
                }
            }
        });
        out.textures_delta.clear();
        assert!(missing.is_empty(), "missing glyphs: {missing:?}");
    }

    /// Code shows the characters typed: in monospace, `->`, `!=`, `<=` and `::` draw each character
    /// with the glyph it has on its own (no programming ligatures), so what a diff shows is what
    /// copying it gives.
    #[test]
    fn code_text_has_no_ligatures() {
        let ctx = egui::Context::default();
        install(&ctx);
        let font = FontId::monospace(13.0);
        let mut mismatches = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let glyphs = |text: &str| {
                let galley = ui.painter().layout_no_wrap(
                    text.to_owned(),
                    font.clone(),
                    egui::Color32::WHITE,
                );
                galley.rows[0]
                    .glyphs
                    .iter()
                    .map(|g| (g.chr, g.uv_rect.offset, g.uv_rect.size))
                    .collect::<Vec<_>>()
            };
            for pair in ["->", "!=", "<=", "::", "=>", "==", "&&", "/*"] {
                let joined = glyphs(&format!("x{pair}x"));
                // Each character alone at the same position (spaces join with nothing), so the
                // same subpixel offset picks the same atlas entry.
                let apart: Vec<_> = pair
                    .chars()
                    .enumerate()
                    .map(|(i, c)| glyphs(&format!("x{}{c} x", " ".repeat(i)))[1 + i])
                    .collect();
                if joined[1..=2] != apart[..] {
                    mismatches.push(pair);
                }
            }
        });
        out.textures_delta.clear();
        assert!(mismatches.is_empty(), "ligatures in code: {mismatches:?}");
    }
}
