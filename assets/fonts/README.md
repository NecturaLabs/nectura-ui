# Bundled fonts

| File | Font | Licence | Source |
|---|---|---|---|
| `Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf` | Inter 4.1 by Rasmus Andersson | SIL Open Font License 1.1 (`OFL-Inter.txt`) | npm `inter-ui@4.1.1`, converted from WOFF2 with fontTools; private-use code points dropped from the cmap so Phosphor icons win |
| `JetBrainsMonoNL-Regular.ttf` | JetBrains Mono NL 2.242 by JetBrains: the build without programming ligatures, so code shows the characters typed | SIL Open Font License 1.1 (`OFL-JetBrainsMono.txt`, the project's own `OFL.txt` at v2.242) | npm `jetbrains-mono@1.0.6` (`fonts/webfonts/JetBrainsMonoNL-Regular.woff2`), converted from WOFF2 with fontTools |
| `CanopyCJK-Regular.otf` | Canopy CJK: a modified version of Noto Sans CJK JP Regular 2.004 by Adobe for Google | SIL Open Font License 1.1 (`OFL-NotoSansCJK.txt`, the project's `Sans/LICENSE`) | `notofonts/noto-cjk` repository `Sans/OTF/Japanese/NotoSansCJKjp-Regular.otf`; subset with fontTools to everyday CJK text (5.7 MB): CJK punctuation, kana, Bopomofo, Hangul jamo and all 11,172 Hangul syllables, enclosed and compatibility CJK, fullwidth and halfwidth forms, and the Han characters of GB 2312, JIS X 0208 level 1 and Big5 level 1 (22,167 code points); hinting removed, cmap as one format 12 table, and renamed ("Noto" is Google's trademark) |
| `AtkinsonHyperlegible-Bold.ttf` | Atkinson Hyperlegible Bold by the Braille Institute of America | SIL Open Font License 1.1 (`OFL-AtkinsonHyperlegible.txt`) | Google Fonts repository `ofl/atkinsonhyperlegible/AtkinsonHyperlegible-Bold.ttf`; subset with fontTools to Latin (U+0000-024F), spacing modifiers, punctuation, currency, letterlike symbols and arrows, hinting removed |

The OFL allows bundling with software. The fonts are embedded in the binary with `include_bytes!`
(`src/fonts.rs`). Atkinson Hyperlegible is a theme heading face
(`theme::HeadingFace`: titles at high contrast). Canopy CJK is the last fallback of
every family, so Chinese, Japanese and Korean file names, branches and messages draw real glyphs; a
rarer Han character outside its sets still shows the missing-glyph box. Any new font must have a row here and
its licence text alongside.
