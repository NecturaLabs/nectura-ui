//! Design tokens. This is the only place colours, geometry and type sizes are defined; see
//! `docs/design.md`.
//!
//! Canopy ships three themes, Midnight and Daylight (the default look) and Observatory, plus the
//! custom theme recoloured from either of the first two (`ui/custom_theme.rs`). Each is a
//! [`Palette`]: the colours both styles share, plus one
//! [`Surfaces`] set per [`Style`] (Standard, the default, and Deeper). Geometry and type come from
//! [`Metrics`], one table per ([`Density`], [`Style`]) pair. The active theme, style, density and
//! translucency are globals set by [`apply`], so views ask [`palette`], [`surfaces`], [`metrics`]
//! and [`appearance`] without knowing which is active.
#![expect(
    clippy::disallowed_methods,
    reason = "the palette: the one module that makes colours"
)]
#![expect(
    clippy::unreadable_literal,
    reason = "colours are written 0xRRGGBB, as design tools show them"
)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{LazyLock, Mutex, PoisonError, RwLock};

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Theme, Visuals,
    vec2,
};
use serde::{Deserialize, Serialize};

pub use super::custom_theme::CustomTheme;

pub const XS: f32 = 4.0;
pub const SM: f32 = 8.0;
pub const MD: f32 = 12.0;
pub const LG: f32 = 16.0;
pub const XL: f32 = 24.0;
pub const XXL: f32 = 32.0;
/// The smallest side of anything a pointer must hit (WCAG 2.5.8), taking a point as a CSS pixel.
pub const MIN_TARGET: f32 = 24.0;
pub const COMPACT_WIDTH: f32 = 1000.0;
pub const SETTINGS_MAX_WIDTH: f32 = 1120.0;
pub const SETTINGS_MAX_HEIGHT: f32 = 900.0;
/// The widest a column of prose and its fields grow, so lines stay readable on wide windows.
pub const MEASURE: f32 = 920.0;
/// A disabled control's opacity (the whole control, label included).
pub const DISABLED_ALPHA: f32 = 0.42;
/// Opacity of the status washes over their [`Tones`] (16%).
pub(super) const TONE_ALPHA: u8 = 41;

pub const MIN_WINDOW_SIZE: egui::Vec2 = vec2(900.0, 560.0);
/// Shared control geometry, in logical points before interface zoom.
pub const CONTROL_INSET: f32 = 2.0;

/// Two ends of a vertical fill, top first. Standard surfaces are flat (`top == bottom`) and paint
/// as one rect; Deeper's are painted with `kit::paint::fill` (a ramp texture brush).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fill {
    pub top: Color32,
    pub bottom: Color32,
}

impl Fill {
    pub const fn flat(c: Color32) -> Self {
        Self { top: c, bottom: c }
    }

    pub fn is_flat(self) -> bool {
        self.top == self.bottom
    }

    /// Both ends as a pair, for checks that run on each.
    pub fn ends(self) -> [Color32; 2] {
        [self.top, self.bottom]
    }
}

/// The surfaces and material of one [`Style`] (Standard = round-2 concept 1, Deeper = concept 4).
#[derive(Debug, Clone, Copy)]
pub struct Surfaces {
    /// Window ground. Standard: also the content ground. Deeper: the gutter colour around cards.
    pub bg: Color32,
    /// Chrome: sidebar, status bar, pane headers, settings navigation. Deeper: equal to `bg`.
    pub panel: Color32,
    /// Content ground: History table, file lists, diff, welcome. Standard `== bg`; Deeper: the card.
    pub pane: Color32,
    /// Raised flat surface: inset boxes, the HEAD pill, toast detail blocks.
    pub elevated: Color32,
    /// Floating layers: menus, select lists, popovers, toasts, dialogs, palette, cheatsheet.
    pub overlay: Color32,
    /// Recessed input fill (text fields, search).
    pub field: Color32,
    /// Recessed tracks: the segmented track.
    pub track: Color32,
    /// Switch off track, slider rail, progress track.
    pub switch_off: Color32,
    /// Neutral count-badge fill.
    pub badge: Color32,
    /// Seam: 1 pt lines between regions and under headers.
    pub border: Color32,
    /// Edge: rings of fields, keycaps and cards' hover.
    pub edge: Color32,
    /// Ring of secondary buttons and select triggers.
    pub raised_edge: Color32,
    /// Ring of floating layers and tooltips.
    pub overlay_edge: Color32,
    /// Ring of the active document tab.
    pub tab_edge: Color32,
    /// Ring of the segmented track.
    pub track_edge: Color32,
    /// Ring of floating panes and cards; `None` in Standard (flush panes).
    pub pane_edge: Option<Color32>,
    /// 1 px inner bottom line of keycaps.
    pub keycap_edge: Color32,
    /// Secondary button, select trigger and their states.
    pub raised: Fill,
    pub raised_hover: Fill,
    pub raised_pressed: Fill,
    /// Segmented thumb.
    pub thumb: Fill,
    /// Keycap.
    pub keycap: Fill,
    /// Active document tab.
    pub tab: Fill,
    /// Tooltip (light themes use a dark tooltip).
    pub tooltip: Fill,
    /// A keycap inside a tooltip (its shortcut): fill, 1 pt ring, inner bottom line (`None` for
    /// a flat key) and label. Dark themes use their ordinary keycap; light themes, whose tooltip
    /// is dark, a key a step lighter than the tooltip.
    pub tooltip_key: Color32,
    pub tooltip_key_edge: Color32,
    pub tooltip_key_base: Option<Color32>,
    pub on_tooltip_key: Color32,
    /// Switch thumb, on and off.
    pub knob: Fill,
    /// Primary button, checked checkbox and radio, switch on, slider range, accent count badge.
    pub primary: Fill,
    pub primary_hover: Fill,
    pub primary_pressed: Fill,
    /// Danger button (confirmations and destructive primary actions) and its states.
    pub danger_solid: Fill,
    pub danger_solid_hover: Fill,
    pub danger_solid_pressed: Fill,
    /// Diff lines and gutters, composited over `pane`.
    pub added_bg: Color32,
    pub removed_bg: Color32,
    pub added_gutter: Color32,
    pub removed_gutter: Color32,
    /// The top bars ([`crate::kit::surface::Deck`]). Standard: the flat `panel` chrome of
    /// the repository tab strip and the action bar (concept 1's two-tone chrome). Deeper: the
    /// action bar's floating card, a lifted gradient with its own highlight and shadow; the tab
    /// strip stays on the window's ground.
    pub deck: Fill,
    /// The deck's bottom edge: Standard's seam under the tab strip and the action bar (none over
    /// the content where the theme parts panes by tone); Deeper's outer line under the action
    /// bar's card.
    pub deck_edge: Color32,
    /// Header strip of a table pane (History's column header). Standard: the pane itself.
    /// Deeper: a lifted tone.
    pub pane_header: Color32,
    /// The seam under a table pane's header. Standard: `border`. Deeper: tinted with `focus`, so
    /// it never reads as one of the neutral pane edges.
    pub pane_header_seam: Color32,
    /// 1 pt ring around the switch thumb (light themes, where white on the off track is 1.5:1):
    /// 3:1 against the thumb. `None` in dark themes.
    pub knob_edge: Option<Color32>,
    /// Fill of a disabled primary, danger or secondary button: a flat, neutral, dimmed step (no
    /// gradient, highlight, glow or shadow), labelled in [`Palette::disabled_text`].
    pub disabled_fill: Color32,
    /// 1 pt ring around `disabled_fill`, so a disabled button keeps its shape on any surface.
    pub disabled_edge: Color32,
}

/// The base colours of the status washes ([`Palette::success_tint`] and the rest). Dark themes
/// use their text-grade status colours; light themes a brighter, more chromatic version of the
/// same hue, so a 16% wash over a pale ground reads as a clean colour rather than a muddy grey.
#[derive(Debug, Clone, Copy)]
pub struct Tones {
    pub accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

/// Colours of one theme. Opaque unless stated; translucent values are made with
/// [`Palette::tint`] (premultiplied).
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Display name.
    pub name: &'static str,
    /// One-line concept, shown with the theme's card.
    pub concept: &'static str,
    /// Dark theme (egui's dark visuals as the base).
    pub dark: bool,
    /// Stricter contrast targets (high contrast): text 10:1, outlines and focus 4.5:1.
    pub strict: bool,
    pub standard: Surfaces,
    pub deeper: Surfaces,
    /// Primary text: 7:1 on every surface, row state and diff background (10:1 at high contrast).
    pub text: Color32,
    /// Secondary text: authors, dates, paths, inactive labels. 4.5:1 (7:1 at high contrast).
    pub text_dim: Color32,
    /// Tertiary text: placeholders, line numbers, shortcuts, section labels. 4.5:1 (5.5:1).
    pub text_faint: Color32,
    /// Label and icon of a disabled control: 3:1 (4.5:1 at high contrast) on `disabled_fill`, every
    /// `raised*` fill, the grounds and their hover tints, dimmer than `text_faint`. Controls drawn
    /// with it are not also faded by [`DISABLED_ALPHA`].
    pub disabled_text: Color32,
    /// Text-grade accent: selected SHA, active icons, checkmarks, matched characters, counts.
    pub accent: Color32,
    /// Links and remote-ref accents.
    pub accent2: Color32,
    /// Keyboard focus ring, focused field border, active underline-tab indicator, Deeper's
    /// focused-pane edge and table header seam.
    pub focus: Color32,
    /// Hue of the selection wash ([`Palette::selected`]).
    pub selection: Color32,
    pub selection_alpha: u8,
    /// Label on `primary*`: 4.5:1 on every end of every state, both styles.
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    /// Text-grade danger: danger menu items, error text, deletions.
    pub danger: Color32,
    /// Concept 1's danger red (`--danger`): error field border and halo, the danger button's focus
    /// ring, the danger badge fill, the danger menu highlight and the destructive dialog disc.
    pub danger_base: Color32,
    /// Label on `danger_solid*`: 4.5:1 on every end of every state, both styles.
    pub on_danger: Color32,
    /// Bases of the status washes (banners, callouts, dialog discs, diff lines).
    pub tones: Tones,
    /// Ink of added and removed lines: the diff's `+` and `−` marks and the counts and blocks of
    /// every diffstat (file lists, commit cards, request summaries). Richer than `success` and
    /// `danger`, which also mean a passing or failing check; 4.5:1 on every ground, row state and
    /// diff row and gutter, and apart in lightness from each other, so the pair reads without hue.
    pub diff_added: Color32,
    pub diff_removed: Color32,
    /// Outlines that identify an input: text fields, checkbox and radio rims. 3:1 on every surface
    /// (4.5:1 at high contrast).
    pub control_border: Color32,
    /// Tooltip text and its second line.
    pub on_tooltip: Color32,
    pub on_tooltip_dim: Color32,
    /// Base of the hover, strong-hover, pressed and zebra overlays.
    pub hover_ink: Color32,
    /// Shadow colours: `shadow` for neutral layers, `primary_shadow` and `danger_shadow` for the
    /// tinted contact shadows of light themes (black in dark themes).
    pub shadow: Color32,
    pub primary_shadow: Color32,
    pub danger_shadow: Color32,
    /// Scrim behind dialogs, settings, palette and cheatsheet (premultiplied).
    pub scrim: Color32,
    /// Translucent surfaces: alpha of `overlay` painted over the blurred backdrop; 0 = never glass.
    pub glass_alpha: u8,
    /// Translucent surfaces: the blurred backdrop's relative luminance is clamped to at most this
    /// (dark) or at least this (light) so the contrast pairings hold over any content.
    pub glass_luminance: f32,
    /// Graph lanes, ref pills, avatar fills. 3:1 on pane, chrome, hovered and selected rows.
    pub lanes: [Color32; 10],
    /// Code colours in diffs. 4.5:1 on `pane`, `added_bg`, `removed_bg`.
    pub syntax: Syntax,
    /// Colours of a request's fields in the list and the details header.
    pub fields: Fields,
}

/// One colour for each field of a request (pull or merge request) row and details header, so the
/// eye finds the number, the author or the age by colour as well as by place. The title is
/// `text` and the state its tone's colour. Each is text-grade: 4.5:1 on the plain grounds, hovers
/// and selected rows, and distinct from the others and from `text`, `text_dim` and `accent`
/// (`every_theme_keeps_the_request_fields_apart`). A field is never told by colour alone: each also
/// has its place, its icon or its wording.
#[derive(Debug, Clone, Copy)]
pub struct Fields {
    /// The request's number (`#428`, `!428`).
    pub number: Color32,
    /// Who opened it.
    pub author: Color32,
    /// When it was last updated.
    pub time: Color32,
    /// Its labels: the ink, and the base of the tint behind each label.
    pub label: Color32,
    /// How many comments it has.
    pub comments: Color32,
}

/// Syntax highlighting colours. Plain code uses `text`.
#[derive(Debug, Clone, Copy)]
pub struct Syntax {
    pub keyword: Color32,
    pub string: Color32,
    pub comment: Color32,
    pub number: Color32,
    pub function: Color32,
    pub type_: Color32,
    pub constant: Color32,
}

const fn hex(rgb: u32) -> Color32 {
    let [_, r, g, b] = rgb.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

// Each theme is an OKLCH surface ladder in its own neutral hue (Standard: flat chrome one clear
// step off the ground; Deeper: a receding ground under lifted cards), its signature colours, status
// tones and ten lanes at least dE 25 apart, contrast-fitted against the tests at the end of this file.
/// "Midnight" (concept 1, the default): neutral blue-black, two-tone chrome, an indigo signature.
pub const MIDNIGHT: Palette = Palette {
    name: "Midnight",
    concept: "Neutral blue-black with two-tone chrome and an indigo signature.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x0F1012),
        panel: hex(0x151619),
        pane: hex(0x0F1012),
        elevated: hex(0x1B1C20),
        overlay: hex(0x1B1C20),
        field: hex(0x111214),
        track: hex(0x0C0D0F),
        switch_off: hex(0x30323A),
        badge: hex(0x2B2D33),
        border: hex(0x24262B),
        edge: hex(0x2B2D33),
        raised_edge: hex(0x2B2D33),
        overlay_edge: hex(0x2B2D33),
        tab_edge: hex(0x2B2D33),
        track_edge: hex(0x24262B),
        pane_edge: None,
        keycap_edge: hex(0x0B0B0D),
        raised: Fill::flat(hex(0x1B1C20)),
        raised_hover: Fill::flat(hex(0x222329)),
        raised_pressed: Fill::flat(hex(0x17181B)),
        thumb: Fill::flat(hex(0x26282E)),
        keycap: Fill::flat(hex(0x24262B)),
        tab: Fill::flat(hex(0x1B1C20)),
        tooltip: Fill::flat(hex(0x2A2C32)),
        tooltip_key: hex(0x24262B),
        tooltip_key_edge: hex(0x2B2D33),
        tooltip_key_base: Some(hex(0x0B0B0D)),
        on_tooltip_key: hex(0xA4A6AD),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x5E6AD2)),
        primary_hover: Fill::flat(hex(0x5964CC)),
        primary_pressed: Fill::flat(hex(0x5560C4)),
        danger_solid: Fill::flat(hex(0xD63C42)),
        danger_solid_hover: Fill::flat(hex(0xD0353D)),
        danger_solid_pressed: Fill::flat(hex(0xC2353B)),
        added_bg: hex(0x032917),
        removed_bg: hex(0x411418),
        added_gutter: hex(0x013820),
        removed_gutter: hex(0x5A1820),
        deck: Fill::flat(hex(0x151619)),
        deck_edge: hex(0x24262B),
        pane_header: hex(0x0F1012),
        pane_header_seam: hex(0x24262B),
        knob_edge: None,
        disabled_fill: hex(0x17181C),
        disabled_edge: hex(0x25272C),
    },
    deeper: Surfaces {
        bg: hex(0x0A0B0D),
        panel: hex(0x0A0B0D),
        pane: hex(0x151619),
        elevated: hex(0x1D1E23),
        overlay: hex(0x1D1E23),
        field: hex(0x0F1013),
        track: hex(0x0B0C0E),
        switch_off: hex(0x30323A),
        badge: hex(0x2A2C32),
        border: hex(0x22242A),
        edge: hex(0x2D2F36),
        raised_edge: hex(0x2F3138),
        overlay_edge: hex(0x2E3037),
        tab_edge: hex(0x2A2C32),
        track_edge: hex(0x1F2126),
        pane_edge: Some(hex(0x1F2126)),
        keycap_edge: hex(0x08080A),
        raised: Fill {
            top: hex(0x26282E),
            bottom: hex(0x1D1E23),
        },
        raised_hover: Fill {
            top: hex(0x2C2E35),
            bottom: hex(0x222329),
        },
        raised_pressed: Fill {
            top: hex(0x1A1B1F),
            bottom: hex(0x1F2025),
        },
        thumb: Fill {
            top: hex(0x2D2F36),
            bottom: hex(0x25272D),
        },
        keycap: Fill {
            top: hex(0x2A2C32),
            bottom: hex(0x202126),
        },
        tab: Fill {
            top: hex(0x202227),
            bottom: hex(0x1A1B20),
        },
        tooltip: Fill {
            top: hex(0x2C2E35),
            bottom: hex(0x24262C),
        },
        tooltip_key: hex(0x2A2C32),
        tooltip_key_edge: hex(0x2D2F36),
        tooltip_key_base: Some(hex(0x08080A)),
        on_tooltip_key: hex(0xA4A6AD),
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xE6E7EC),
        },
        primary: Fill {
            top: hex(0x616CD6),
            bottom: hex(0x4753BE),
        },
        primary_hover: Fill {
            top: hex(0x5C66D0),
            bottom: hex(0x434DB8),
        },
        primary_pressed: Fill {
            top: hex(0x5260C8),
            bottom: hex(0x4F5AC2),
        },
        danger_solid: Fill {
            top: hex(0xD63E44),
            bottom: hex(0xBB282E),
        },
        danger_solid_hover: Fill {
            top: hex(0xD0373F),
            bottom: hex(0xB52029),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC2353B),
            bottom: hex(0xC9363C),
        },
        added_bg: hex(0x032917),
        removed_bg: hex(0x411418),
        added_gutter: hex(0x013820),
        removed_gutter: hex(0x5A1820),
        deck: Fill {
            top: hex(0x1C1D20),
            bottom: hex(0x17181B),
        },
        deck_edge: hex(0x060608),
        pane_header: hex(0x191A1D),
        pane_header_seam: hex(0x323556),
        knob_edge: None,
        disabled_fill: hex(0x1A1B1F),
        disabled_edge: hex(0x26282E),
    },
    text: hex(0xECECEF),
    text_dim: hex(0xA4A6AD),
    text_faint: hex(0x9698A0),
    disabled_text: hex(0x8A8C94),
    accent: hex(0x9AA2FF),
    accent2: hex(0x7EC8F2),
    focus: hex(0x7C86FF),
    selection: hex(0x7C86FF),
    selection_alpha: 33,
    on_accent: hex(0xFFFFFF),
    success: hex(0x4CC38A),
    warning: hex(0xD9A54A),
    danger: hex(0xFF7D81),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    tones: Tones {
        accent: hex(0x7C86FF),
        success: hex(0x4CC38A),
        warning: hex(0xD9A54A),
        danger: hex(0xE5484D),
    },
    diff_added: hex(0x34DE8A),
    diff_removed: hex(0xFF7076),
    control_border: hex(0x787A81),
    on_tooltip: hex(0xECECEF),
    on_tooltip_dim: hex(0xB6B6BA),
    hover_ink: hex(0xFFFFFF),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(4, 4, 5, 158),
    glass_alpha: 204,
    glass_luminance: 0.03,
    lanes: [
        hex(0x7C86FF),
        hex(0x4CC38A),
        hex(0xE5739F),
        hex(0xD9A54A),
        hex(0x4EB8E9),
        hex(0xEE8266),
        hex(0xCB81E4),
        hex(0x43C3C3),
        hex(0x9EBF5F),
        hex(0xDDAA88),
    ],
    syntax: Syntax {
        keyword: hex(0xADB8FF),
        string: hex(0x7BD4A4),
        comment: hex(0x9698A0),
        number: hex(0xF3B17A),
        function: hex(0x82CBF6),
        type_: hex(0x7BDAD9),
        constant: hex(0xF49CBB),
    },
    fields: Fields {
        number: hex(0xF2A26A),
        author: hex(0x5ECAD6),
        time: hex(0xBDBA8E),
        label: hex(0xF197C2),
        comments: hex(0xD1A1EF),
    },
};

/// "Daylight" (concept 1 by day): white canvas, light-grey chrome and the same indigo.
pub const DAYLIGHT: Palette = Palette {
    name: "Daylight",
    concept: "White canvas, light-grey chrome and the same indigo: Midnight by day.",
    dark: false,
    strict: false,
    standard: Surfaces {
        bg: hex(0xFFFFFF),
        panel: hex(0xF7F7F8),
        pane: hex(0xFFFFFF),
        elevated: hex(0xFFFFFF),
        overlay: hex(0xFFFFFF),
        field: hex(0xFFFFFF),
        track: hex(0xEFEFF1),
        switch_off: hex(0xD4D4DA),
        badge: hex(0xECECEF),
        border: hex(0xE6E6EA),
        edge: hex(0xD9D9DF),
        raised_edge: hex(0xD9D9DF),
        overlay_edge: hex(0xE1E1E6),
        tab_edge: hex(0xE1E1E6),
        track_edge: hex(0xE3E3E8),
        pane_edge: None,
        keycap_edge: hex(0xCDCED5),
        raised: Fill::flat(hex(0xFFFFFF)),
        raised_hover: Fill::flat(hex(0xF6F6F8)),
        raised_pressed: Fill::flat(hex(0xEFEFF2)),
        thumb: Fill::flat(hex(0xFFFFFF)),
        keycap: Fill::flat(hex(0xF3F3F5)),
        tab: Fill::flat(hex(0xFFFFFF)),
        tooltip: Fill::flat(hex(0x18181B)),
        tooltip_key: hex(0x2E2E33),
        tooltip_key_edge: hex(0x3C3C42),
        tooltip_key_base: None,
        on_tooltip_key: hex(0xD4D4D8),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x5E6AD2)),
        primary_hover: Fill::flat(hex(0x5560C8)),
        primary_pressed: Fill::flat(hex(0x4B56B8)),
        danger_solid: Fill::flat(hex(0xD83C41)),
        danger_solid_hover: Fill::flat(hex(0xCC3238)),
        danger_solid_pressed: Fill::flat(hex(0xB92B31)),
        added_bg: hex(0xDDF6E7),
        removed_bg: hex(0xFFE9E7),
        added_gutter: hex(0xBDEDD2),
        removed_gutter: hex(0xFFD1CE),
        deck: Fill::flat(hex(0xF7F7F8)),
        deck_edge: hex(0xE6E6EA),
        pane_header: hex(0xFFFFFF),
        pane_header_seam: hex(0xE6E6EA),
        knob_edge: Some(hex(0x94949A)),
        disabled_fill: hex(0xF0F0F2),
        disabled_edge: hex(0xE1E1E6),
    },
    deeper: Surfaces {
        bg: hex(0xECEEF1),
        panel: hex(0xECEEF1),
        pane: hex(0xFFFFFF),
        elevated: hex(0xFFFFFF),
        overlay: hex(0xFFFFFF),
        field: hex(0xFFFFFF),
        track: hex(0xE2E4E8),
        switch_off: hex(0xD4D4DA),
        badge: hex(0xECECEF),
        border: hex(0xE3E5EA),
        edge: hex(0xD3D6DC),
        raised_edge: hex(0xD3D6DC),
        overlay_edge: hex(0xDFE1E6),
        tab_edge: hex(0xD9DCE1),
        track_edge: hex(0xD9DCE1),
        pane_edge: Some(hex(0xE3E5EA)),
        keycap_edge: hex(0xC9CCD3),
        raised: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF4F5F7),
        },
        raised_hover: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xEEF0F3),
        },
        raised_pressed: Fill {
            top: hex(0xECEEF1),
            bottom: hex(0xF2F3F5),
        },
        thumb: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF6F7F9),
        },
        keycap: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF0F1F4),
        },
        tab: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF6F7F9),
        },
        tooltip: Fill::flat(hex(0x18181B)),
        tooltip_key: hex(0x2E2E33),
        tooltip_key_edge: hex(0x3C3C42),
        tooltip_key_base: None,
        on_tooltip_key: hex(0xD4D4D8),
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xF1F2F5),
        },
        primary: Fill {
            top: hex(0x616CD6),
            bottom: hex(0x4753BE),
        },
        primary_hover: Fill {
            top: hex(0x5C66D0),
            bottom: hex(0x434DB8),
        },
        primary_pressed: Fill {
            top: hex(0x4F5BC4),
            bottom: hex(0x5561CC),
        },
        danger_solid: Fill {
            top: hex(0xD63E44),
            bottom: hex(0xBB282E),
        },
        danger_solid_hover: Fill {
            top: hex(0xD0373F),
            bottom: hex(0xB52029),
        },
        danger_solid_pressed: Fill {
            top: hex(0xBF3036),
            bottom: hex(0xC9363C),
        },
        added_bg: hex(0xDDF6E7),
        removed_bg: hex(0xFFE9E7),
        added_gutter: hex(0xBDEDD2),
        removed_gutter: hex(0xFFD1CE),
        deck: Fill {
            top: hex(0xF8FAFD),
            bottom: hex(0xF3F5F8),
        },
        deck_edge: hex(0xD3D5D8),
        pane_header: hex(0xFAFAFA),
        pane_header_seam: hex(0xCED1EE),
        knob_edge: Some(hex(0x898C92)),
        disabled_fill: hex(0xF1F2F4),
        disabled_edge: hex(0xDCDEE3),
    },
    text: hex(0x18181B),
    text_dim: hex(0x575962),
    text_faint: hex(0x5D5F69),
    disabled_text: hex(0x6A6B73),
    accent: hex(0x4651C0),
    accent2: hex(0x006790),
    focus: hex(0x5E6AD2),
    selection: hex(0x5E6AD2),
    selection_alpha: 26,
    on_accent: hex(0xFFFFFF),
    success: hex(0x006A4B),
    warning: hex(0x805500),
    danger: hex(0xB2191F),
    danger_base: hex(0xD93D42),
    on_danger: hex(0xFFFFFF),
    tones: Tones {
        accent: hex(0x6E78E0),
        success: hex(0x23BA7D),
        warning: hex(0xF8AC3D),
        danger: hex(0xF05656),
    },
    diff_added: hex(0x016B3B),
    diff_removed: hex(0xAF001A),
    control_border: hex(0x7C7D85),
    on_tooltip: hex(0xF4F4F5),
    on_tooltip_dim: hex(0xB6B6B8),
    hover_ink: hex(0x14141E),
    shadow: hex(0x101018),
    primary_shadow: hex(0x1E1E50),
    danger_shadow: hex(0x781414),
    scrim: Color32::from_rgba_premultiplied(6, 6, 9, 82),
    glass_alpha: 214,
    glass_luminance: 0.8,
    lanes: [
        hex(0x5E6AD2),
        hex(0x1D9A5F),
        hex(0xCC477C),
        hex(0xB07A1C),
        hex(0x017EAF),
        hex(0xC64E31),
        hex(0x822E9E),
        hex(0x02888E),
        hex(0x648828),
        hex(0x8A4F28),
    ],
    syntax: Syntax {
        keyword: hex(0x4B50B0),
        string: hex(0x116F42),
        comment: hex(0x696B75),
        number: hex(0x974C01),
        function: hex(0x006495),
        type_: hex(0x006B70),
        constant: hex(0xA22C5E),
    },
    fields: Fields {
        number: hex(0x874300),
        author: hex(0x02646D),
        time: hex(0x5D5A31),
        label: hex(0x873761),
        comments: hex(0x6E4188),
    },
};

/// "Observatory" (Midnight until build 690): navy-ink night, a periwinkle signature and starlight
/// links.
pub const OBSERVATORY: Palette = Palette {
    name: "Observatory",
    concept: "Observatory: navy-ink night, a periwinkle signature and starlight links, precise as an instrument.",
    dark: true,
    strict: false,
    standard: Surfaces {
        bg: hex(0x0B0F19),
        panel: hex(0x111723),
        pane: hex(0x0B0F19),
        elevated: hex(0x161D2B),
        overlay: hex(0x181E2D),
        field: hex(0x0E131E),
        track: hex(0x080D17),
        switch_off: hex(0x2A3346),
        badge: hex(0x232B3E),
        border: hex(0x1B2233),
        edge: hex(0x21293B),
        raised_edge: hex(0x21293B),
        overlay_edge: hex(0x20283B),
        tab_edge: hex(0x20283A),
        track_edge: hex(0x181F2E),
        pane_edge: None,
        keycap_edge: hex(0x050914),
        raised: Fill::flat(hex(0x161D2B)),
        raised_hover: Fill::flat(hex(0x1A2131)),
        raised_pressed: Fill::flat(hex(0x141927)),
        thumb: Fill::flat(hex(0x1D2435)),
        keycap: Fill::flat(hex(0x1A2131)),
        tab: Fill::flat(hex(0x161D2B)),
        tooltip: Fill::flat(hex(0x232B3E)),
        tooltip_key: hex(0x1A2131),
        tooltip_key_edge: hex(0x21293B),
        tooltip_key_base: Some(hex(0x050914)),
        on_tooltip_key: hex(0xB2B9C6),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x646CD6)),
        primary_hover: Fill::flat(hex(0x6067D1)),
        primary_pressed: Fill::flat(hex(0x5D63CD)),
        danger_solid: Fill::flat(hex(0xD73E41)),
        danger_solid_hover: Fill::flat(hex(0xD2383D)),
        danger_solid_pressed: Fill::flat(hex(0xCA3037)),
        added_bg: hex(0x022C1D),
        removed_bg: hex(0x41141C),
        added_gutter: hex(0x003D2A),
        removed_gutter: hex(0x5A1A27),
        deck: Fill::flat(hex(0x111723)),
        deck_edge: hex(0x1B2233),
        pane_header: hex(0x0B0F19),
        pane_header_seam: hex(0x1B2233),
        knob_edge: None,
        disabled_fill: hex(0x141A28),
        disabled_edge: hex(0x1C2334),
    },
    deeper: Surfaces {
        bg: hex(0x040813),
        panel: hex(0x040813),
        pane: hex(0x111622),
        elevated: hex(0x1A2131),
        overlay: hex(0x1B2233),
        field: hex(0x0D111C),
        track: hex(0x030612),
        switch_off: hex(0x2A3346),
        badge: hex(0x232B3E),
        border: hex(0x1B2233),
        edge: hex(0x232B3E),
        raised_edge: hex(0x252D40),
        overlay_edge: hex(0x232B3E),
        tab_edge: hex(0x222A3D),
        track_edge: hex(0x171D2C),
        pane_edge: Some(hex(0x1E2637)),
        keycap_edge: hex(0x02040E),
        raised: Fill {
            top: hex(0x21293B),
            bottom: hex(0x1A2131),
        },
        raised_hover: Fill {
            top: hex(0x262E41),
            bottom: hex(0x1E2638),
        },
        raised_pressed: Fill {
            top: hex(0x181E2D),
            bottom: hex(0x1A2030),
        },
        thumb: Fill {
            top: hex(0x252D40),
            bottom: hex(0x1E2638),
        },
        keycap: Fill {
            top: hex(0x222A3D),
            bottom: hex(0x1B2233),
        },
        tab: Fill {
            top: hex(0x1E2638),
            bottom: hex(0x1A2030),
        },
        tooltip: Fill {
            top: hex(0x272F42),
            bottom: hex(0x21293C),
        },
        tooltip_key: hex(0x222A3D),
        tooltip_key_edge: hex(0x232B3E),
        tooltip_key_base: Some(hex(0x02040E)),
        on_tooltip_key: hex(0xB2B9C6),
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xEDEEF1),
        },
        primary: Fill {
            top: hex(0x626BD7),
            bottom: hex(0x5157C1),
        },
        primary_hover: Fill {
            top: hex(0x5D65D1),
            bottom: hex(0x4C51BB),
        },
        primary_pressed: Fill {
            top: hex(0x5960C8),
            bottom: hex(0x5E65CE),
        },
        danger_solid: Fill {
            top: hex(0xD73E40),
            bottom: hex(0xC2282F),
        },
        danger_solid_hover: Fill {
            top: hex(0xD1373B),
            bottom: hex(0xBC202A),
        },
        danger_solid_pressed: Fill {
            top: hex(0xC72C34),
            bottom: hex(0xCC3239),
        },
        added_bg: hex(0x022C1D),
        removed_bg: hex(0x41141C),
        added_gutter: hex(0x003D2A),
        removed_gutter: hex(0x5A1A27),
        deck: Fill {
            top: hex(0x1D2435),
            bottom: hex(0x171D2C),
        },
        deck_edge: hex(0x02040E),
        pane_header: hex(0x1C2234),
        pane_header_seam: hex(0x454C8A),
        knob_edge: None,
        disabled_fill: hex(0x171D2C),
        disabled_edge: hex(0x1E2638),
    },
    text: hex(0xEAEDF2),
    text_dim: hex(0xB2B9C6),
    text_faint: hex(0x9FA6B4),
    disabled_text: hex(0x8E94A0),
    accent: hex(0xA3ABFF),
    accent2: hex(0x90D9F2),
    focus: hex(0x7F89FF),
    selection: hex(0x7F89FF),
    selection_alpha: 34,
    on_accent: hex(0xFFFFFF),
    success: hex(0x55D69F),
    warning: hex(0xEDB65C),
    danger: hex(0xFD8284),
    danger_base: hex(0xE5484D),
    on_danger: hex(0xFFFFFF),
    tones: Tones {
        accent: hex(0x7F89FF),
        success: hex(0x4CC38A),
        warning: hex(0xD9A54A),
        danger: hex(0xE5484D),
    },
    diff_added: hex(0x3FE0A2),
    diff_removed: hex(0xFF787C),
    control_border: hex(0x798090),
    on_tooltip: hex(0xEAEDF2),
    on_tooltip_dim: hex(0xAEB3BC),
    hover_ink: hex(0xFFFFFF),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(2, 2, 3, 158),
    glass_alpha: 153,
    glass_luminance: 0.04,
    lanes: [
        hex(0xA9B4FF),
        hex(0x25CEFA),
        hex(0x55D69F),
        hex(0xF6A6F8),
        hex(0xBFB83E),
        hex(0x12E3E0),
        hex(0xFD8BA1),
        hex(0x97CE6D),
        hex(0xFF9B78),
        hex(0xEBA540),
    ],
    syntax: Syntax {
        keyword: hex(0xAEB6FF),
        string: hex(0x83D9AE),
        comment: hex(0x8D93A8),
        number: hex(0xF2B582),
        function: hex(0x8FD3F7),
        type_: hex(0xCDB8FF),
        constant: hex(0xF5A1C0),
    },
    fields: Fields {
        number: hex(0xF6A56D),
        author: hex(0x61CDD9),
        time: hex(0xC0BD91),
        label: hex(0xF59AC5),
        comments: hex(0xD4A4F2),
    },
};

/// High contrast's reference: Midnight at full contrast. Every theme's high contrast
/// ([`super::contrast::derive`]) keeps the ratios these colours keep against this ground.
pub const CONTRAST_REFERENCE: Palette = Palette {
    name: "Contrast",
    concept: "Midnight at full contrast: white type, strong outlines and a bright indigo focus ring, for low vision and glare.",
    dark: true,
    strict: true,
    standard: Surfaces {
        bg: hex(0x0F1012),
        panel: hex(0x151619),
        pane: hex(0x0F1012),
        elevated: hex(0x1B1C20),
        overlay: hex(0x1B1C20),
        field: hex(0x111214),
        track: hex(0x0C0D0F),
        switch_off: hex(0x3F4149),
        badge: hex(0x2B2D33),
        border: hex(0x7B7D83),
        edge: hex(0xA0A3AA),
        raised_edge: hex(0xA0A3AA),
        overlay_edge: hex(0xD1D4DB),
        tab_edge: hex(0xA0A3AA),
        track_edge: hex(0x7B7D83),
        pane_edge: None,
        keycap_edge: hex(0x7D7D80),
        raised: Fill::flat(hex(0x1B1C20)),
        raised_hover: Fill::flat(hex(0x222329)),
        raised_pressed: Fill::flat(hex(0x17181B)),
        thumb: Fill::flat(hex(0x26282E)),
        keycap: Fill::flat(hex(0x24262B)),
        tab: Fill::flat(hex(0x1B1C20)),
        tooltip: Fill::flat(hex(0x2A2C32)),
        tooltip_key: hex(0x24262B),
        tooltip_key_edge: hex(0xA0A3AA),
        tooltip_key_base: Some(hex(0x7D7D80)),
        on_tooltip_key: hex(0xF2F4FC),
        knob: Fill::flat(hex(0xFFFFFF)),
        primary: Fill::flat(hex(0x94A4FF)),
        primary_hover: Fill::flat(hex(0xA5B3FF)),
        primary_pressed: Fill::flat(hex(0x8999FF)),
        danger_solid: Fill::flat(hex(0xFF9892)),
        danger_solid_hover: Fill::flat(hex(0xFFACA7)),
        danger_solid_pressed: Fill::flat(hex(0xFF817D)),
        added_bg: hex(0x032917),
        removed_bg: hex(0x411418),
        added_gutter: hex(0x013820),
        removed_gutter: hex(0x5A1820),
        deck: Fill::flat(hex(0x151619)),
        deck_edge: hex(0x7B7D83),
        pane_header: hex(0x0F1012),
        pane_header_seam: hex(0x7B7D83),
        knob_edge: None,
        disabled_fill: hex(0x17181C),
        disabled_edge: hex(0x606368),
    },
    deeper: Surfaces {
        bg: hex(0x0A0B0D),
        panel: hex(0x0A0B0D),
        pane: hex(0x151619),
        elevated: hex(0x1D1E23),
        overlay: hex(0x1D1E23),
        field: hex(0x0F1013),
        track: hex(0x0B0C0E),
        switch_off: hex(0x3C3E46),
        badge: hex(0x2A2C32),
        border: hex(0x787B82),
        edge: hex(0x9EA0A9),
        raised_edge: hex(0x9EA0A9),
        overlay_edge: hex(0xCED1DA),
        tab_edge: hex(0x9EA0A8),
        track_edge: hex(0x787B81),
        pane_edge: Some(hex(0x8D8F95)),
        keycap_edge: hex(0x7B7B7E),
        raised: Fill {
            top: hex(0x26282E),
            bottom: hex(0x1D1E23),
        },
        raised_hover: Fill {
            top: hex(0x2C2E35),
            bottom: hex(0x222329),
        },
        raised_pressed: Fill {
            top: hex(0x1A1B1F),
            bottom: hex(0x1F2025),
        },
        thumb: Fill {
            top: hex(0x2D2F36),
            bottom: hex(0x25272D),
        },
        keycap: Fill {
            top: hex(0x2A2C32),
            bottom: hex(0x202126),
        },
        tab: Fill {
            top: hex(0x202227),
            bottom: hex(0x1A1B20),
        },
        tooltip: Fill {
            top: hex(0x2C2E35),
            bottom: hex(0x24262C),
        },
        tooltip_key: hex(0x2A2C32),
        tooltip_key_edge: hex(0x9EA0A9),
        tooltip_key_base: Some(hex(0x7B7B7E)),
        on_tooltip_key: hex(0xEFF1F8),
        knob: Fill {
            top: hex(0xFFFFFF),
            bottom: hex(0xEEEFF4),
        },
        primary: Fill {
            top: hex(0xA3B1FF),
            bottom: hex(0x8DA0FF),
        },
        primary_hover: Fill {
            top: hex(0xB2BEFF),
            bottom: hex(0x9BADFF),
        },
        primary_pressed: Fill {
            top: hex(0x869AFF),
            bottom: hex(0x8EA0FF),
        },
        danger_solid: Fill {
            top: hex(0xFFACA7),
            bottom: hex(0xFF847D),
        },
        danger_solid_hover: Fill {
            top: hex(0xFFBBB7),
            bottom: hex(0xFF928B),
        },
        danger_solid_pressed: Fill {
            top: hex(0xFF7C78),
            bottom: hex(0xFF8580),
        },
        added_bg: hex(0x032917),
        removed_bg: hex(0x411418),
        added_gutter: hex(0x013820),
        removed_gutter: hex(0x5A1820),
        deck: Fill {
            top: hex(0x1C1D20),
            bottom: hex(0x17181B),
        },
        deck_edge: hex(0x7B7B7F),
        pane_header: hex(0x191A1D),
        pane_header_seam: hex(0xA3A8D0),
        knob_edge: None,
        disabled_fill: hex(0x1A1B1F),
        disabled_edge: hex(0x5F6168),
    },
    text: hex(0xFFFFFF),
    text_dim: hex(0xF2F4FC),
    text_faint: hex(0xDEE0E9),
    disabled_text: hex(0xA8AAB2),
    accent: hex(0xC7CDFF),
    accent2: hex(0x88D2FD),
    focus: hex(0xC3CCFF),
    selection: hex(0xADB8FF),
    selection_alpha: 40,
    on_accent: hex(0x000000),
    success: hex(0x5BD197),
    warning: hex(0xE8B459),
    danger: hex(0xFF9797),
    danger_base: hex(0xFD5E60),
    on_danger: hex(0x000000),
    tones: Tones {
        accent: hex(0xC4CDFF),
        success: hex(0x5BD197),
        warning: hex(0xE8B459),
        danger: hex(0xFF9691),
    },
    diff_added: hex(0x4FE59C),
    diff_removed: hex(0xFF8E91),
    control_border: hex(0xD1D4DB),
    on_tooltip: hex(0xFFFFFF),
    on_tooltip_dim: hex(0xCECED2),
    hover_ink: hex(0xFFFFFF),
    shadow: hex(0x000000),
    primary_shadow: hex(0x000000),
    danger_shadow: hex(0x000000),
    scrim: Color32::from_rgba_premultiplied(0, 0, 0, 200),
    glass_alpha: 0,
    glass_luminance: 0.03,
    lanes: [
        hex(0x9CA8FF),
        hex(0x4CC38A),
        hex(0xFD89B4),
        hex(0xD9A54A),
        hex(0x4FB9EA),
        hex(0xFD9073),
        hex(0xDC91F5),
        hex(0x43C3C3),
        hex(0x9EBF5F),
        hex(0xDDAA88),
    ],
    syntax: Syntax {
        keyword: hex(0xADB8FF),
        string: hex(0x7BD4A4),
        comment: hex(0xB0B3BB),
        number: hex(0xF3B17A),
        function: hex(0x82CBF6),
        type_: hex(0x7BDAD9),
        constant: hex(0xFAA2C1),
    },
    fields: Fields {
        number: hex(0xFED4B9),
        author: hex(0x87F2FE),
        time: hex(0xE3E1B3),
        label: hex(0xFECEE4),
        comments: hex(0xEDD2FE),
    },
};

/// The built-in themes at high contrast, in [`ThemeId::BUILT_IN`] order.
static HIGH_CONTRAST: LazyLock<[Palette; 3]> = LazyLock::new(|| {
    ThemeId::BUILT_IN.map(|t| super::contrast::derive(t.palette(), &CONTRAST_REFERENCE))
});

/// A theme the user can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum ThemeId {
    #[default]
    Midnight,
    Daylight,
    Observatory,
    /// The user's own colours on Midnight or Daylight ([`CustomTheme`], [`set_custom`]).
    Custom,
}

impl ThemeId {
    /// Every theme, in menu order.
    pub const ALL: [Self; 4] = [
        Self::Midnight,
        Self::Daylight,
        Self::Observatory,
        Self::Custom,
    ];

    /// The themes Canopy ships, whose colours never change.
    pub const BUILT_IN: [Self; 3] = [Self::Midnight, Self::Daylight, Self::Observatory];

    /// The theme's colours (the custom theme's as last set by [`set_custom`]).
    pub fn palette(self) -> &'static Palette {
        match self {
            Self::Midnight => &MIDNIGHT,
            Self::Daylight => &DAYLIGHT,
            Self::Observatory => &OBSERVATORY,
            Self::Custom => custom_built().palette,
        }
    }

    /// The theme's structure: the custom theme takes its base's.
    pub fn design(self) -> &'static Design {
        match self.built_in() {
            Self::Daylight => &DAYLIGHT_DESIGN,
            Self::Observatory => &OBSERVATORY_DESIGN,
            Self::Midnight | Self::Custom => &MIDNIGHT_DESIGN,
        }
    }

    /// The built-in theme whose structure and corners this one shows: itself, or the custom
    /// theme's base.
    fn built_in(self) -> Self {
        match self {
            Self::Custom if custom().light => Self::Daylight,
            Self::Custom => Self::Midnight,
            other => other,
        }
    }

    /// The colours shown: the theme's own, or at high contrast.
    pub fn shown_palette(self, high_contrast: bool) -> &'static Palette {
        match (self, high_contrast) {
            (_, false) => self.palette(),
            (Self::Custom, true) => custom_built().high_contrast,
            // `HIGH_CONTRAST` follows `BUILT_IN`, which holds every theme but Custom.
            (_, true) => {
                let i = Self::BUILT_IN.iter().position(|t| *t == self).unwrap_or(0);
                &HIGH_CONTRAST[i]
            }
        }
    }

    /// The structure shown: the theme's own, or high contrast's.
    pub fn shown_design(self, high_contrast: bool) -> &'static Design {
        if high_contrast {
            &HIGH_CONTRAST_DESIGN
        } else {
            self.design()
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        self.palette().name
    }

    /// The theme for a `--theme` argument: the display name or the variant name, any case.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| {
            t.name().eq_ignore_ascii_case(name) || format!("{t:?}").eq_ignore_ascii_case(name)
        })
    }

    /// The theme after this one, wrapping around.
    #[must_use]
    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    fn index(self) -> u8 {
        Self::ALL
            .iter()
            .position(|t| *t == self)
            .and_then(|i| u8::try_from(i).ok())
            .unwrap_or(0)
    }
}

/// Surfaces and material: Standard (concept 1, flat two-tone chrome, flush panes) or Deeper
/// (concept 4, floating card panes, gradient controls and layered shadows).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Style {
    #[default]
    Standard,
    Deeper,
}

impl Style {
    pub const ALL: [Self; 2] = [Self::Standard, Self::Deeper];

    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Deeper => "Deeper",
        }
    }

    /// The style for a `--style` argument.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|s| s.label().eq_ignore_ascii_case(name))
    }
}

/// Row heights, radii, toolbar layout and type size: Comfortable (concept 1) or Dense (concept 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Density {
    #[default]
    Comfortable,
    Dense,
}

impl Density {
    pub const ALL: [Self; 2] = [Self::Comfortable, Self::Dense];

    pub fn label(self) -> &'static str {
        match self {
            Self::Comfortable => "Comfortable",
            Self::Dense => "Dense",
        }
    }

    /// The density for a `--density` argument.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|d| d.label().eq_ignore_ascii_case(name))
    }
}

/// Shape of a commit node in the history graph (Settings, Appearance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum NodeShape {
    /// A filled circle on a ring of the row's ground.
    #[default]
    Dot,
    /// A hollow circle: a ring in the lane colour around the row's ground.
    Ring,
    /// A filled square with `Metrics::radius_sm` scaled corners.
    Square,
    /// A filled diamond (a square turned 45 degrees).
    Diamond,
}

impl NodeShape {
    pub const ALL: [Self; 4] = [Self::Dot, Self::Ring, Self::Square, Self::Diamond];

    pub fn label(self) -> &'static str {
        match self {
            Self::Dot => "Dots",
            Self::Ring => "Rings",
            Self::Square => "Squares",
            Self::Diamond => "Diamonds",
        }
    }
}

/// How the history graph draws an edge between lanes (Settings, Appearance).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Curve {
    /// A cubic curve leaving and entering vertically.
    #[default]
    Smooth,
    /// Vertical, then a rounded corner, horizontal, a rounded corner, vertical (a metro map).
    Rounded,
    /// Vertical, then a straight diagonal across one row, then vertical (a circuit trace).
    Angular,
}

impl Curve {
    pub const ALL: [Self; 3] = [Self::Smooth, Self::Rounded, Self::Angular];

    pub fn label(self) -> &'static str {
        match self {
            Self::Smooth => "Curves",
            Self::Rounded => "Metro",
            Self::Angular => "Circuit",
        }
    }
}

/// What renders, resolved from the settings, the theme and the graphics backend each time any of
/// them changes ([`Appearance::resolve`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Appearance {
    pub style: Style,
    pub density: Density,
    /// Blurred translucent floating layers.
    pub glass: bool,
    pub reduce_motion: bool,
}

/// Why Translucent surfaces cannot be on right now, for the setting's disabled state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlassBlocked {
    /// High contrast never uses glass.
    Theme,
    /// The operating system asks for reduced transparency.
    System,
    /// The graphics backend cannot blur (or this build has no blur yet).
    Backend,
}

impl GlassBlocked {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Theme => "Not available with high contrast",
            Self::System => "Your system asks for reduced transparency",
            Self::Backend => "Not supported by this graphics driver",
        }
    }
}

impl Appearance {
    /// The default options with motion off: screenshots and tests, where nothing may animate.
    pub const STILL: Self = Self {
        style: Style::Standard,
        density: Density::Comfortable,
        glass: false,
        reduce_motion: true,
    };

    /// What renders for these choices. `translucent` is the user's switch; glass also needs a
    /// theme that allows it, a backend that can blur and no system request for reduced
    /// transparency (`None`: the system did not say).
    pub fn resolve(
        theme: ThemeId,
        style: Style,
        density: Density,
        translucent: bool,
        reduce_motion: bool,
        glass_available: bool,
        reduced_transparency: Option<bool>,
    ) -> Self {
        let blocked = glass_blocked(theme, glass_available, reduced_transparency);
        Self {
            style,
            density,
            glass: translucent && blocked.is_none(),
            reduce_motion,
        }
    }
}

/// Why glass is off whatever the switch says, if it is.
pub fn glass_blocked(
    theme: ThemeId,
    glass_available: bool,
    reduced_transparency: Option<bool>,
) -> Option<GlassBlocked> {
    if high_contrast() || theme.palette().glass_alpha == 0 {
        Some(GlassBlocked::Theme)
    } else if reduced_transparency == Some(true) {
        Some(GlassBlocked::System)
    } else if !glass_available {
        Some(GlassBlocked::Backend)
    } else {
        None
    }
}

/// The active theme. One window, one look: globals keep [`palette`] and [`metrics`] cheap.
struct Look {
    theme: AtomicU8,
    style: AtomicU8,
    density: AtomicU8,
    glass: AtomicBool,
    node: AtomicU8,
    curve: AtomicU8,
    contrast: AtomicBool,
    /// The custom theme's choices and colours; `None` until [`set_custom`] (Midnight's colours).
    custom: RwLock<Option<CustomBuilt>>,
}

impl Look {
    const fn new() -> Self {
        Self {
            theme: AtomicU8::new(0),
            style: AtomicU8::new(0),
            density: AtomicU8::new(0),
            glass: AtomicBool::new(false),
            node: AtomicU8::new(0),
            curve: AtomicU8::new(0),
            contrast: AtomicBool::new(false),
            custom: RwLock::new(None),
        }
    }
}

#[cfg(not(any(test, feature = "testkit")))]
static LOOK: Look = Look::new();

#[cfg(not(any(test, feature = "testkit")))]
fn look<R>(f: impl FnOnce(&Look) -> R) -> R {
    f(&LOOK)
}

// Tests run in parallel threads, each with its own look, so one test's theme or density never
// leaks into another's.
#[cfg(any(test, feature = "testkit"))]
thread_local! {
    static LOOK: Look = const { Look::new() };
}

#[cfg(any(test, feature = "testkit"))]
fn look<R>(f: impl FnOnce(&Look) -> R) -> R {
    LOOK.with(f)
}

/// The theme currently shown.
pub fn active() -> ThemeId {
    ThemeId::ALL[usize::from(look(|l| l.theme.load(Ordering::Relaxed))) % ThemeId::ALL.len()]
}

/// The style currently shown.
pub fn style() -> Style {
    if look(|l| l.style.load(Ordering::Relaxed)) == 1 {
        Style::Deeper
    } else {
        Style::Standard
    }
}

/// The density currently shown.
pub fn density() -> Density {
    if look(|l| l.density.load(Ordering::Relaxed)) == 1 {
        Density::Dense
    } else {
        Density::Comfortable
    }
}

/// Turns high contrast on or off over every theme. Call before [`apply`], which installs egui's
/// visuals from the colours shown.
pub fn set_high_contrast(on: bool) {
    look(|l| l.contrast.store(on, Ordering::Relaxed));
}

/// Whether high contrast is on.
pub fn high_contrast() -> bool {
    look(|l| l.contrast.load(Ordering::Relaxed))
}

/// The custom theme's colours for one set of choices, as and at high contrast.
#[derive(Clone, Copy)]
struct CustomBuilt {
    choice: CustomTheme,
    palette: &'static Palette,
    high_contrast: &'static Palette,
}

/// Every custom palette built in this run, by choice. The palettes are borrowed for the life of
/// the program like the built-in ones, so each choice is built and kept once: dragging a slider
/// back and forth reuses them, and the choices are whole degrees and percents, so they stay few.
fn build_custom(choice: CustomTheme) -> CustomBuilt {
    static BUILT: LazyLock<Mutex<HashMap<CustomTheme, CustomBuilt>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut built = BUILT.lock().unwrap_or_else(PoisonError::into_inner);
    *built.entry(choice).or_insert_with(|| {
        let palette: &'static Palette = Box::leak(Box::new(choice.palette()));
        CustomBuilt {
            choice,
            palette,
            high_contrast: Box::leak(Box::new(super::contrast::derive(
                palette,
                &CONTRAST_REFERENCE,
            ))),
        }
    })
}

/// Sets the custom theme's colours. Call before [`apply`], like [`set_high_contrast`].
pub fn set_custom(choice: CustomTheme) {
    let choice = choice.clamped();
    if custom_slot() == Some(choice) {
        return;
    }
    let built = build_custom(choice);
    look(|l| *l.custom.write().unwrap_or_else(PoisonError::into_inner) = Some(built));
}

fn custom_slot() -> Option<CustomTheme> {
    look(|l| {
        l.custom
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .map(|b| b.choice)
    })
}

/// The custom theme's current choices.
pub fn custom() -> CustomTheme {
    custom_slot().unwrap_or_default()
}

fn custom_built() -> CustomBuilt {
    look(|l| *l.custom.read().unwrap_or_else(PoisonError::into_inner))
        .unwrap_or_else(|| build_custom(CustomTheme::default()))
}

/// The colours currently shown (the active theme, at high contrast when it is on).
pub fn active_palette() -> &'static Palette {
    active().shown_palette(high_contrast())
}

/// The structure currently shown.
pub fn active_design() -> &'static Design {
    active().shown_design(high_contrast())
}

/// Sets the history graph's node shape and edge curve (the user's choice in Settings).
pub fn set_graph(node: NodeShape, curve: Curve) {
    let index = |i: Option<usize>| i.and_then(|i| u8::try_from(i).ok()).unwrap_or(0);
    let n = index(NodeShape::ALL.iter().position(|v| *v == node));
    let c = index(Curve::ALL.iter().position(|v| *v == curve));
    look(|l| {
        l.node.store(n, Ordering::Relaxed);
        l.curve.store(c, Ordering::Relaxed);
    });
}

/// The history graph's node shape and edge curve currently shown.
pub fn graph_shape() -> (NodeShape, Curve) {
    let (n, c) = look(|l| {
        (
            l.node.load(Ordering::Relaxed),
            l.curve.load(Ordering::Relaxed),
        )
    });
    (
        NodeShape::ALL[usize::from(n) % NodeShape::ALL.len()],
        Curve::ALL[usize::from(c) % Curve::ALL.len()],
    )
}

/// The options currently rendering.
pub fn appearance() -> Appearance {
    Appearance {
        style: style(),
        density: density(),
        glass: look(|l| l.glass.load(Ordering::Relaxed)),
        reduce_motion: super::motion::reduced(),
    }
}

impl Palette {
    /// The surfaces of `style`.
    pub fn surfaces(&self, style: Style) -> &Surfaces {
        match style {
            Style::Standard => &self.standard,
            Style::Deeper => &self.deeper,
        }
    }

    /// The surfaces of the active style.
    pub fn s(&self) -> &Surfaces {
        self.surfaces(style())
    }

    /// Lane colour for a graph colour index.
    pub fn lane(&self, index: u16) -> Color32 {
        self.lanes[usize::from(index) % self.lanes.len()]
    }

    fn overlay_alpha(&self, dark: u8, light: u8) -> Color32 {
        Self::tint(self.hover_ink, if self.dark { dark } else { light })
    }

    /// Hovered rows and quiet controls: a translucent tint drawn over any surface.
    pub fn hover(&self) -> Color32 {
        self.overlay_alpha(9, 9)
    }

    /// Hovered tool and icon buttons, menu highlight, ghost buttons.
    pub fn hover_strong(&self) -> Color32 {
        self.overlay_alpha(15, 14)
    }

    /// Pressed rows and quiet controls.
    pub fn pressed(&self) -> Color32 {
        self.overlay_alpha(22, 20)
    }

    /// Even history rows in Dense.
    pub fn zebra(&self) -> Color32 {
        self.overlay_alpha(5, 6)
    }

    /// Fill of the selected row in lists, the graph, the sidebar, the palette and diffs, and of
    /// egui's selected values. Every text colour stays readable on it (see the contrast test).
    pub fn selected(&self) -> Color32 {
        Self::tint(self.selection, self.selection_alpha)
    }

    /// Status tints (16% of the theme's [`Tones`]): dialog glyph discs, banners, callouts.
    pub fn success_tint(&self) -> Color32 {
        Self::tint(self.tones.success, TONE_ALPHA)
    }

    pub fn warning_tint(&self) -> Color32 {
        Self::tint(self.tones.warning, TONE_ALPHA)
    }

    /// The danger wash: the destructive dialog's glyph disc, danger banners and callouts.
    pub fn danger_tint(&self) -> Color32 {
        Self::tint(self.tones.danger, TONE_ALPHA)
    }

    pub fn accent_tint(&self) -> Color32 {
        Self::tint(self.tones.accent, TONE_ALPHA)
    }

    /// Highlight of a danger menu item (15% of `danger_base`).
    pub fn danger_highlight(&self) -> Color32 {
        Self::tint(self.danger_base, 38)
    }

    /// The surfaces a coloured component (pill, badge, avatar) can sit on in `s`: plain surfaces,
    /// hovered rows and selected rows.
    pub fn component_surfaces(&self, s: &Surfaces) -> [Color32; 8] {
        let sel = self.selected();
        let hover = self.hover();
        [
            s.pane,
            s.panel,
            s.elevated,
            s.overlay,
            over(hover, s.pane),
            over(hover, s.panel),
            over(sel, s.pane),
            over(sel, s.panel),
        ]
    }

    /// A tonal control in colour `c` (the accent for a primary button, `danger` for a destructive
    /// one): the raised control ground tinted with `c`, more on hover and press, a hairline of
    /// `c` and the label in `c` lifted to 4.5:1 on it. Returns fill, ring and label.
    pub fn tonal(&self, c: Color32, hover: f32, press: f32) -> (Fill, Color32, Color32) {
        let s = self.s();
        let t =
            TONAL_REST + (TONAL_HOVER - TONAL_REST) * hover + (TONAL_PRESSED - TONAL_HOVER) * press;
        let fill = Fill {
            top: mix(s.raised.top, c, t),
            bottom: mix(s.raised.bottom, c, t),
        };
        let ring = mix(s.raised_edge, c, TONAL_RING);
        let label = self.ink(self.ink(c, fill.top), fill.bottom);
        (fill, ring, label)
    }

    /// A ref pill in colour `c` (its lane's): the badge surface tinted with `c` (more for the
    /// checked-out branch, `strong`) and a hairline of `c`, the tonal look of [`Palette::tonal`]
    /// at rest. The label is white where white reads at 4.5:1 on the pill over every component
    /// surface, and otherwise `c` lifted that far ([`Palette::ink`]). Returns fill, ring and label.
    pub fn pill(&self, c: Color32, strong: bool) -> (Color32, Color32, Color32) {
        let s = self.s();
        let fill = mix(s.badge, c, if strong { TONAL_PRESSED } else { TONAL_REST });
        let ring = mix(s.badge, c, TONAL_RING);
        let white_reads = self
            .component_surfaces(s)
            .iter()
            .all(|b| contrast(Color32::WHITE, over(fill, *b)) >= 4.5);
        let label = if white_reads {
            Color32::WHITE
        } else {
            self.ink(c, fill)
        };
        (fill, ring, label)
    }

    /// Text or an icon in colour `fg` drawn on the translucent `fill` in the active style: `fg`
    /// moved towards the text colour just enough to read at 4.5:1 on that fill over every
    /// component surface, so it keeps its hue wherever the component is drawn.
    pub fn ink(&self, fg: Color32, fill: Color32) -> Color32 {
        let style = style();
        let key = (std::ptr::from_ref(self).addr(), style, fg, fill);
        if let Some(c) = INKS.with(|m| m.borrow().get(&key).copied()) {
            return c;
        }
        let ink = self.ink_on(self.surfaces(style), fg, fill);
        INKS.with(|m| m.borrow_mut().insert(key, ink));
        ink
    }

    /// [`Palette::ink`] for the surfaces `s`, uncached.
    pub fn ink_on(&self, s: &Surfaces, fg: Color32, fill: Color32) -> Color32 {
        let behind = self.component_surfaces(s).map(|b| over(fill, b));
        let readable = |c: Color32| behind.iter().all(|b| contrast(c, *b) >= 4.5);
        (0..=20)
            .map(|step| mix(fg, self.text, crate::int_f32(step) / 20.0))
            .find(|c| readable(*c))
            .unwrap_or(self.text)
    }

    /// Fill of an initials avatar in colour `c` (24%).
    pub const fn avatar_fill(c: Color32) -> Color32 {
        Self::tint(c, 61)
    }

    /// Fill of a count badge in colour `c` (24%).
    pub const fn badge_fill(c: Color32) -> Color32 {
        Self::tint(c, 61)
    }

    /// Fill of a danger count badge (22% of `danger_base`), labelled in `ink(danger, ..)`.
    pub fn danger_badge_fill(&self) -> Color32 {
        Self::tint(self.danger_base, 56)
    }

    /// Fill of a file status letter in colour `c` (18%).
    pub const fn change_fill(c: Color32) -> Color32 {
        Self::tint(c, 46)
    }

    /// A translucent version of `c` for fills behind text (premultiplied).
    pub const fn tint(c: Color32, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied_const(c.r(), c.g(), c.b(), alpha)
    }

    /// The material of `level` in `style` for this theme: its shadow layers in the theme's
    /// [`ShadowCharacter`] (`Design::shadows`); high contrast has no highlights.
    pub fn material(&self, style: Style, level: Elevation) -> Material {
        let m = *material(style, self.dark, level);
        let m = Material {
            layers: shaped_layers(self.design().shadows, style, self.dark, level, m.layers),
            ..m
        };
        if self.strict {
            Material {
                highlight: 0,
                bottom_line: 0,
                ..m
            }
        } else {
            m
        }
    }

    /// The structure of the theme this palette belongs to: high contrast's for a strict palette,
    /// Midnight's for a palette no theme owns.
    pub fn design(&self) -> &'static Design {
        if self.strict {
            return &HIGH_CONTRAST_DESIGN;
        }
        ThemeId::ALL
            .into_iter()
            .find(|t| t.palette().name == self.name)
            .map_or(&MIDNIGHT_DESIGN, ThemeId::design)
    }

    /// The colour a shadow layer's tint resolves to in `style`.
    pub fn shadow_color(&self, tint: ShadowTint) -> Color32 {
        match tint {
            ShadowTint::Shadow => self.shadow,
            ShadowTint::Black => Color32::BLACK,
            ShadowTint::Primary => self.primary_shadow,
            ShadowTint::Danger => self.danger_shadow,
            ShadowTint::PrimaryGlow => self.standard.primary.top,
            ShadowTint::DangerGlow => self.standard.danger_solid.top,
        }
    }

    /// The fill of a translucent floating layer over `s`: the overlay leaning to the theme's light
    /// (`Design::glass_tint` over `overlay`) at `glass_alpha`.
    pub fn glass(&self, s: &Surfaces) -> Color32 {
        Self::tint(over(self.design().glass_tint, s.overlay), self.glass_alpha)
    }

    /// Ring of floating layers: `overlay_edge`, or `control_border` at high contrast.
    pub fn floating_ring(&self, s: &Surfaces) -> Color32 {
        if self.strict {
            self.control_border
        } else {
            s.overlay_edge
        }
    }
}

/// Relative luminance (WCAG 2.2) of an opaque colour.
pub fn luminance(c: Color32) -> f64 {
    let ch = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * ch(c.r()) + 0.7152 * ch(c.g()) + 0.0722 * ch(c.b())
}

/// WCAG contrast ratio between two opaque colours, from 1 to 21.
pub fn contrast(a: Color32, b: Color32) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Translucent `fg` drawn over opaque `bg`, as the GPU blends it (`Color32` is premultiplied).
pub fn over(fg: Color32, bg: Color32) -> Color32 {
    let a = f32::from(fg.a()) / 255.0;
    let mix = |f: u8, b: u8| crate::saturate_u8((f32::from(f) + f32::from(b) * (1.0 - a)).round());
    Color32::from_rgb(
        mix(fg.r(), bg.r()),
        mix(fg.g(), bg.g()),
        mix(fg.b(), bg.b()),
    )
}

/// How much of its colour a tonal control's ground takes at rest, hovered and pressed, and its
/// ring ([`Palette::tonal`]).
const TONAL_REST: f32 = 0.16;
const TONAL_HOVER: f32 = 0.22;
const TONAL_PRESSED: f32 = 0.28;
const TONAL_RING: f32 = 0.45;

/// `a` moved towards `b` by `t` (0 to 1).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| {
        crate::saturate_u8((f32::from(x) + (f32::from(y) - f32::from(x)) * t).round())
    };
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// CPU reference for the glass shader's backdrop luminance clamp, used by contrast tests.
/// Dark themes scale a sample down to at most `cap` relative luminance; light themes lift it
/// towards white to at least `cap`, in linear light. The GPU performs its own clamp.
pub fn clamp_backdrop(c: Color32, dark: bool, cap: f32) -> Color32 {
    let to_lin = |v: u8| {
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
        reason = "v is clamped to 0..=1 first, so the channel is 0..=255"
    )]
    let to_srgb = |v: f64| {
        let v = v.clamp(0.0, 1.0);
        let s = if v <= 0.003_130_8 {
            12.92 * v
        } else {
            1.055 * v.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0 + 0.5).floor() as u8
    };
    let mut lin = [to_lin(c.r()), to_lin(c.g()), to_lin(c.b())];
    let y = 0.2126 * lin[0] + 0.7152 * lin[1] + 0.0722 * lin[2];
    let cap = f64::from(cap);
    if dark && y > cap {
        lin = lin.map(|x| x * cap / y);
    } else if !dark && y < cap {
        let t = (cap - y) / (1.0 - y);
        lin = lin.map(|x| x + (1.0 - x) * t);
    }
    Color32::from_rgb(to_srgb(lin[0]), to_srgb(lin[1]), to_srgb(lin[2]))
}

/// Theme, style, colour and fill of a worked-out ink.
/// The palette is told apart by its address: a theme and its high-contrast version share a name,
/// and every custom palette is named Custom.
type InkKey = (usize, Style, Color32, Color32);

thread_local! {
    /// Inks already worked out, per theme, style, colour and fill: views ask for the same few
    /// every frame.
    static INKS: std::cell::RefCell<HashMap<InkKey, Color32>> =
        std::cell::RefCell::new(HashMap::new());
}

/// The palette of the active theme.
pub fn palette(_ctx: &egui::Context) -> &'static Palette {
    active_palette()
}

/// The surfaces of the active theme and style.
pub fn surfaces(_ctx: &egui::Context) -> &'static Surfaces {
    active_palette().surfaces(style())
}

/// The structure of the active theme: how it draws corners, edges, shadows, selection, focus,
/// labels, headings, the graph and its art.
pub fn design(_ctx: &egui::Context) -> &'static Design {
    active_design()
}

// -------------------------------------------------------------------------------------- design

pub use design::*;

/// The per-theme design record (wave 3): the structure half of each theme.
mod design {
    use egui::{Color32, FontFamily};

    use super::{Elevation, FULL, Palette, ShadowLayer, hex};

    /// A theme's structure, the half of its identity colour cannot carry: corners, edges, shadows,
    /// how panes separate, the ground, the interaction voice, type, the graph, icons, art and motion.
    /// Every field changes what the kit and the views paint (`contracts/theme-2.md` says how); the
    /// colours stay in the [`Palette`].
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Design {
        /// Factor on every radius of the active [`Metrics`] ([`Design::radius`]): below 1 sharper and
        /// more technical, above 1 softer and more rounded. [`FULL`] stays full.
        pub radius_scale: f32,
        /// Width in points of seams, card edges and control rings.
        pub edge_width: f32,
        pub edges: EdgeMode,
        pub shadows: ShadowCharacter,
        /// How Standard's flush regions part (Deeper always floats cards).
        pub panes: PaneSeparation,
        /// Treatment of the window ground where no content sits: Deeper's gutter, the welcome screen
        /// and empty states. Content grounds stay flat.
        pub ground: Ground,
        pub selection: SelectionStyle,
        pub focus: FocusStyle,
        pub hover: HoverStyle,
        /// The active repository tab and the active view tab.
        pub active_tab: TabStyle,
        /// Links are underlined at rest, not only when hovered or focused: where the link colour
        /// alone does not set a link apart from body text, or for low vision (high contrast).
        pub link_underline: bool,
        /// Section labels (sidebar sections, settings groups, table headers, menu headings).
        pub labels: LabelStyle,
        /// Face of titles and headlines; body text stays Inter.
        pub heading: HeadingFace,
        pub graph: GraphStyle,
        pub icons: IconWeight,
        /// The motif of the welcome backdrop and the empty-state illustrations (`ui/art.rs`).
        pub art: ArtMotif,
        /// Easing and pace within the user's Motion setting (Reduced still turns motion off).
        pub motion: MotionCharacter,
        /// Translucent surfaces: a premultiplied tint laid over `Surfaces::overlay` before the glass
        /// opacity, so the glass leans to the theme's light. Transparent: plain overlay.
        pub glass_tint: Color32,
    }

    /// How region edges and control rings are drawn.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum EdgeMode {
        /// 1 px seams in `Surfaces::border` between regions and under headers; controls ringed.
        Hairline,
        /// Strong lines: seams and card edges in `Palette::control_border` at `edge_width`.
        Outline,
    }

    /// Character of shadows, applied to every [`Material`] layer by [`Palette::material`]
    /// ([`ShadowCharacter::shape`]).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ShadowCharacter {
        /// The material tables as they are: long, soft, low-alpha layers.
        Soft,
        /// No shadows below floating layers; floating layers keep a faint, short one so they lift.
        Flat,
    }

    /// How Standard's flush regions (sidebar, content, details) part.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PaneSeparation {
        /// A 1 px seam in `Surfaces::border`.
        Seams,
        /// A strong rule in `Palette::control_border` at `Design::edge_width`.
        Outline,
    }

    /// The window ground's treatment where no content sits. Every variant is a translucent `ink`
    /// laid over the style's ground (`Surfaces::bg`, and `pane` on the welcome screen in Standard),
    /// so text and controls painted over it keep their contrast (tested).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Ground {
        Flat,
        /// `ink` at `top` alpha at the top edge, fading linearly to `bottom` alpha at the bottom.
        Vertical {
            ink: Color32,
            top: u8,
            bottom: u8,
        },
    }

    /// How a selected row, item or chip is marked. Every style keeps the [`Palette::selected`] wash
    /// under it, so the contrast tests hold for all of them.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum SelectionStyle {
        /// The wash alone.
        Wash,
        /// The wash plus a leading bar of `width` points in `Palette::focus`, full row height,
        /// inset by the row's radius.
        Bar { width: f32 },
        /// The wash plus a ring of `width` points in `Palette::focus` at 70%.
        Outline { width: f32 },
    }

    /// The keyboard focus ring.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum FocusStyle {
        /// A ring of `width` in `Palette::focus`, `offset` points outside the control.
        Ring { width: f32, offset: f32 },
        /// A ring of `width` in `Palette::focus` with a 1 pt ring of the ground inside it, so it
        /// reads on any fill.
        Double { width: f32 },
    }

    /// What a hovered row or quiet control shows.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HoverStyle {
        /// The translucent [`Palette::hover`] wash.
        Tint,
        /// The wash plus a 1 px ring in `Surfaces::edge` (`control_border` at high contrast).
        Edge,
    }

    /// The active repository tab and view tab.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum TabStyle {
        /// A filled, ringed tab lifted off the strip (`Surfaces::tab`, `tab_edge`).
        Lifted,
        /// No fill: an underline of `width` points in `Palette::focus` under the label.
        Underline { width: f32 },
    }

    /// Section labels, set in capitals at `Metrics::micro_size` in Inter Semibold: their
    /// letter-spacing (em, times the label's size).
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct LabelStyle {
        pub tracking: f32,
    }

    /// Face of titles and headlines.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HeadingFace {
        /// Inter Semibold, like the rest of the interface.
        Ui,
        /// Atkinson Hyperlegible Bold: designed for low vision, for titles at high contrast.
        Hyperlegible,
    }

    impl HeadingFace {
        /// The egui family name the face is registered under (`ui/fonts.rs`).
        pub const fn family_name(self) -> &'static str {
            match self {
                Self::Ui => "semibold",
                Self::Hyperlegible => "heading-hyperlegible",
            }
        }

        pub fn family(self) -> FontFamily {
            FontFamily::Name(self.family_name().into())
        }

        /// Size factor that gives the face Inter's apparent size (x-height), applied to heading sizes.
        pub const fn size_scale(self) -> f32 {
            match self {
                Self::Ui => 1.0,
                Self::Hyperlegible => 1.04,
            }
        }
    }

    /// The history graph's drawing: round nodes on smooth curves, scaled per theme.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct GraphStyle {
        /// Factor on `Metrics::graph_node_r` and `graph_head_halo`.
        pub node_scale: f32,
        /// Factor on `Metrics::graph_edge` and `graph_merge_stroke`.
        pub line_scale: f32,
    }

    /// Phosphor weight for icons.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum IconWeight {
        /// Regular everywhere.
        Regular,
        /// Regular, with the `fill` glyph for active and selected icons (the active view tab, a
        /// toggled tool, the selected sidebar row, the checked menu item).
        FillAccents,
    }

    /// The motif `ui/art.rs` draws the welcome backdrop and the empty states from.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum ArtMotif {
        /// Midnight and Daylight (concept 1): the illustrations as drawn, over a plain ground.
        Plain,
        /// Observatory: stars joined into constellations, the commits as stars.
        Constellation,
        /// High contrast: bold line art, no glows or gradients.
        Signal,
    }

    /// Easing and pace of the theme's motion.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum MotionCharacter {
        /// Quick and exact: 0.8 times the durations.
        Crisp,
        Standard,
    }

    impl MotionCharacter {
        /// Factor on every `ui::motion` duration.
        pub const fn duration_scale(self) -> f32 {
            match self {
                Self::Crisp => 0.8,
                Self::Standard => 1.0,
            }
        }
    }

    impl Design {
        /// `r` (a [`Metrics`] radius) in this theme: scaled and rounded, [`FULL`] kept full, and a
        /// rounded corner never squared off entirely.
        pub fn radius(&self, r: u8) -> u8 {
            if r == FULL || r == 0 {
                return r;
            }
            crate::saturate_u8((f32::from(r) * self.radius_scale).round()).max(1)
        }
    }

    impl ShadowCharacter {
        pub const ALL: [Self; 2] = [Self::Soft, Self::Flat];

        /// Shadow `layer` of `level` in this character, or `None` when it draws no shadow there.
        pub fn shape(self, level: Elevation, layer: ShadowLayer) -> Option<ShadowLayer> {
            let scale = |v: u8, f: f32| crate::saturate_u8((f32::from(v) * f).round());
            match self {
                Self::Soft => Some(layer),
                Self::Flat => (level == Elevation::Floating).then_some(ShadowLayer {
                    dy: layer.dy.min(4),
                    blur: scale(layer.blur, 0.4),
                    alpha: scale(layer.alpha, 0.6),
                    ..layer
                }),
            }
        }
    }

    const fn labels(tracking: f32) -> LabelStyle {
        LabelStyle { tracking }
    }

    const fn graph(node_scale: f32, line_scale: f32) -> GraphStyle {
        GraphStyle {
            node_scale,
            line_scale,
        }
    }

    /// Round-2 concept 1, the design the owner locked for the default themes: flat two-tone chrome
    /// parted by 1 pt seams, soft shadows only under floating layers, the selection a wash with no
    /// bar, the active tab lifted, a 2 pt focus ring 2 pt outside the control, circle nodes on
    /// smooth curves, a flat ground and the illustrations as drawn.
    const CONCEPT_ONE: Design = Design {
        radius_scale: 1.0,
        edge_width: 1.0,
        edges: EdgeMode::Hairline,
        shadows: ShadowCharacter::Soft,
        panes: PaneSeparation::Seams,
        ground: Ground::Flat,
        selection: SelectionStyle::Wash,
        focus: FocusStyle::Ring {
            width: 2.0,
            offset: 2.0,
        },
        hover: HoverStyle::Tint,
        active_tab: TabStyle::Lifted,
        link_underline: false,
        labels: labels(0.07),
        heading: HeadingFace::Ui,
        graph: graph(1.0, 1.0),
        icons: IconWeight::Regular,
        art: ArtMotif::Plain,
        motion: MotionCharacter::Standard,
        glass_tint: Color32::TRANSPARENT,
    };

    /// Midnight: concept 1 in the dark.
    pub const MIDNIGHT_DESIGN: Design = CONCEPT_ONE;

    /// Daylight: concept 1 by day, the same design as Midnight.
    pub const DAYLIGHT_DESIGN: Design = CONCEPT_ONE;

    /// Observatory: precise as an instrument. Hairlines, soft long shadows, a selection
    /// bar like an indicator, a faint zenith-to-horizon lift, constellations.
    pub const OBSERVATORY_DESIGN: Design = Design {
        radius_scale: 1.0,
        edge_width: 1.0,
        edges: EdgeMode::Hairline,
        shadows: ShadowCharacter::Soft,
        panes: PaneSeparation::Seams,
        ground: Ground::Vertical {
            ink: hex(0x7F89FF),
            top: 0,
            bottom: 12,
        },
        selection: SelectionStyle::Bar { width: 2.0 },
        focus: FocusStyle::Ring {
            width: 2.0,
            offset: 1.0,
        },
        hover: HoverStyle::Tint,
        active_tab: TabStyle::Lifted,
        link_underline: false,
        labels: labels(0.07),
        heading: HeadingFace::Ui,
        graph: graph(1.0, 1.0),
        icons: IconWeight::Regular,
        art: ArtMotif::Constellation,
        motion: MotionCharacter::Standard,
        glass_tint: Palette::tint(hex(0x1A1F4A), 28),
    };

    /// High contrast, over any theme: built for legibility. Strong outlines instead of shadows,
    /// an outlined selection, a double focus ring, a thick underline, bold labels, Atkinson
    /// Hyperlegible titles, Midnight's round nodes on thicker lines, line art without glows.
    pub const HIGH_CONTRAST_DESIGN: Design = Design {
        radius_scale: 0.6,
        edge_width: 1.5,
        edges: EdgeMode::Outline,
        shadows: ShadowCharacter::Flat,
        panes: PaneSeparation::Outline,
        ground: Ground::Flat,
        selection: SelectionStyle::Outline { width: 2.0 },
        focus: FocusStyle::Double { width: 2.5 },
        hover: HoverStyle::Edge,
        active_tab: TabStyle::Underline { width: 3.0 },
        link_underline: true,
        labels: labels(0.06),
        heading: HeadingFace::Hyperlegible,
        graph: graph(1.15, 1.25),
        icons: IconWeight::FillAccents,
        art: ArtMotif::Signal,
        motion: MotionCharacter::Crisp,
        glass_tint: Color32::TRANSPARENT,
    };
}

// ------------------------------------------------------------------------------------ material

/// One soft shadow layer in egui units, painted bottom layer first, before the surface. `spread`
/// may be negative (CSS `0 10px 28px -8px`): `kit::paint::shadow` shrinks the rect by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowLayer {
    pub dy: i8,
    pub blur: u8,
    pub spread: i8,
    pub alpha: u8,
    pub tint: ShadowTint,
}

/// The colour a shadow layer takes ([`Palette::shadow_color`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowTint {
    /// `Palette::shadow`.
    Shadow,
    Black,
    /// `Palette::primary_shadow`.
    Primary,
    /// `Palette::danger_shadow`.
    Danger,
    /// The theme's Standard primary rest colour (Deeper's coloured glow).
    PrimaryGlow,
    /// The theme's Standard danger rest colour.
    DangerGlow,
}

/// What a level paints besides its fill: a 1 px inner top highlight (white at `highlight` alpha),
/// a 1 px inner bottom line (`bottom_line`, light Deeper controls), an inner recess shadow
/// (fields in Deeper) and the outer layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Material {
    pub highlight: u8,
    pub bottom_line: u8,
    pub recess: u8,
    pub layers: &'static [ShadowLayer],
}

/// The colour of a material's top highlight line: white at `alpha` (`Material::highlight`).
pub fn highlight_line(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
}

/// Elevation levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Elevation {
    /// Secondary button, select trigger.
    Control,
    /// Segmented thumb.
    Thumb,
    /// Switch thumb.
    Knob,
    /// Active document tab.
    Tab,
    /// Primary button.
    Primary,
    /// Danger button.
    Danger,
    /// Menus, select list, popover, tooltip, toast, dialog, palette, cheatsheet.
    Floating,
    /// Floating panes and cards (Deeper).
    Pane,
    /// Text fields (Deeper's recess).
    Field,
    /// The top deck (Deeper).
    Deck,
}

impl Elevation {
    pub const ALL: [Self; 10] = [
        Self::Control,
        Self::Thumb,
        Self::Knob,
        Self::Tab,
        Self::Primary,
        Self::Danger,
        Self::Floating,
        Self::Pane,
        Self::Field,
        Self::Deck,
    ];
}

const fn layer(dy: i8, blur: u8, spread: i8, alpha: u8) -> ShadowLayer {
    ShadowLayer {
        dy,
        blur,
        spread,
        alpha,
        tint: ShadowTint::Shadow,
    }
}

const fn tinted(dy: i8, blur: u8, spread: i8, alpha: u8, tint: ShadowTint) -> ShadowLayer {
    ShadowLayer {
        dy,
        blur,
        spread,
        alpha,
        tint,
    }
}

const fn mat(highlight: u8, layers: &'static [ShadowLayer]) -> Material {
    Material {
        highlight,
        bottom_line: 0,
        recess: 0,
        layers,
    }
}

const NONE: Material = mat(0, &[]);

/// The material of `level` in `style`, for dark or light themes (concept 1 and concept 4).
pub fn material(style: Style, dark: bool, level: Elevation) -> &'static Material {
    use Elevation as E;
    use ShadowTint as T;
    static STD_DARK: [Material; 10] = [
        mat(11, &[layer(1, 1, 0, 77)]),
        mat(13, &[layer(1, 2, 0, 115)]),
        mat(0, &[layer(1, 2, 0, 115)]),
        NONE,
        mat(46, &[layer(1, 2, 0, 102)]),
        mat(41, &[layer(1, 2, 0, 102)]),
        mat(0, &[layer(10, 28, -8, 166), layer(2, 6, 0, 77)]),
        NONE,
        NONE,
        NONE,
    ];
    static STD_LIGHT: [Material; 10] = [
        mat(0, &[layer(1, 2, 0, 15)]),
        mat(0, &[layer(1, 2, 0, 26), layer(0, 0, 1, 13)]),
        // Concept 1's contact shadow plus a soft halo (owner's amendment, with the knob ring).
        mat(
            0,
            &[tinted(1, 2, 0, 56, T::Black), tinted(2, 6, 0, 20, T::Black)],
        ),
        NONE,
        mat(46, &[tinted(1, 2, 0, 51, T::Primary)]),
        mat(0, &[tinted(1, 2, 0, 51, T::Danger)]),
        mat(0, &[layer(14, 36, -10, 64), layer(2, 6, 0, 20)]),
        NONE,
        NONE,
        NONE,
    ];
    static DEEP_DARK: [Material; 10] = [
        mat(19, &[layer(1, 1, 0, 102), layer(3, 8, -2, 115)]),
        mat(23, &[layer(1, 2, 0, 128), layer(2, 6, 0, 89)]),
        mat(0, &[layer(1, 2, 0, 128), layer(2, 6, 0, 77)]),
        mat(15, &[layer(2, 6, -2, 128)]),
        mat(
            66,
            &[layer(1, 1, 0, 102), tinted(5, 14, -5, 166, T::PrimaryGlow)],
        ),
        mat(
            56,
            &[layer(1, 1, 0, 102), tinted(5, 14, -6, 153, T::DangerGlow)],
        ),
        mat(13, &[layer(20, 44, -14, 217), layer(4, 12, 0, 102)]),
        mat(16, &[layer(1, 2, 0, 140), layer(10, 26, -10, 205)]),
        Material {
            highlight: 0,
            bottom_line: 0,
            recess: 71,
            layers: &[],
        },
        mat(20, &[layer(1, 2, 0, 128), layer(8, 22, -10, 179)]),
    ];
    static DEEP_LIGHT: [Material; 10] = [
        Material {
            highlight: 0,
            bottom_line: 10,
            recess: 0,
            layers: &[layer(1, 1, 0, 15), layer(2, 6, -2, 31)],
        },
        mat(0, &[layer(1, 1, 0, 20), layer(2, 6, -2, 38)]),
        mat(
            0,
            &[tinted(1, 2, 0, 51, T::Black), tinted(2, 5, 0, 26, T::Black)],
        ),
        mat(0, &[layer(1, 2, 0, 20)]),
        mat(
            64,
            &[
                tinted(1, 1, 0, 51, T::Primary),
                tinted(5, 12, -5, 128, T::PrimaryGlow),
            ],
        ),
        mat(
            56,
            &[
                tinted(1, 1, 0, 51, T::Danger),
                tinted(5, 12, -6, 115, T::DangerGlow),
            ],
        ),
        mat(0, &[layer(20, 44, -16, 71), layer(3, 10, 0, 20)]),
        mat(0, &[layer(1, 2, 0, 20), layer(8, 24, -12, 66)]),
        Material {
            highlight: 0,
            bottom_line: 0,
            recess: 18,
            layers: &[],
        },
        mat(0, &[layer(1, 2, 0, 20), layer(6, 18, -10, 56)]),
    ];
    let table = match (style, dark) {
        (Style::Standard, true) => &STD_DARK,
        (Style::Standard, false) => &STD_LIGHT,
        (Style::Deeper, true) => &DEEP_DARK,
        (Style::Deeper, false) => &DEEP_LIGHT,
    };
    let i = match level {
        E::Control => 0,
        E::Thumb => 1,
        E::Knob => 2,
        E::Tab => 3,
        E::Primary => 4,
        E::Danger => 5,
        E::Floating => 6,
        E::Pane => 7,
        E::Field => 8,
        E::Deck => 9,
    };
    &table[i]
}

/// `layers` (the material of `level` in `style`, dark or light) in shadow character `shadows`,
/// worked out once for every combination: `Soft` is the tables as they are.
fn shaped_layers(
    shadows: ShadowCharacter,
    style: Style,
    dark: bool,
    level: Elevation,
    layers: &'static [ShadowLayer],
) -> &'static [ShadowLayer] {
    type Key = (ShadowCharacter, Style, bool, Elevation);
    static SHAPED: LazyLock<HashMap<Key, Vec<ShadowLayer>>> = LazyLock::new(|| {
        let mut out = HashMap::new();
        for shadows in ShadowCharacter::ALL {
            for style in [Style::Standard, Style::Deeper] {
                for dark in [true, false] {
                    for level in Elevation::ALL {
                        let shaped = material(style, dark, level)
                            .layers
                            .iter()
                            .filter_map(|l| shadows.shape(level, *l))
                            .collect();
                        out.insert((shadows, style, dark, level), shaped);
                    }
                }
            }
        }
        out
    });
    if shadows == ShadowCharacter::Soft {
        return layers;
    }
    SHAPED
        .get(&(shadows, style, dark, level))
        .map_or(layers, Vec::as_slice)
}

/// Deeper's focused-pane edge: `focus` at these alphas at the top, at 38% of the height, and gone
/// from 70% (concept 4's `.c-diff::before`).
pub const FOCUS_EDGE: [(f32, u8); 3] = [(0.0, 191), (0.38, 46), (0.70, 0)];

// ------------------------------------------------------------------------------------- metrics

/// How tool buttons lay out their icon and caption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLayout {
    /// Icon over caption (Comfortable).
    Stacked,
    /// Icon beside the label (Dense).
    Inline,
}

/// Radius meaning "fully rounded" (half the shorter side).
pub const FULL: u8 = u8::MAX;

/// Geometry and type for one ([`Density`], [`Style`]) pair, read with [`metrics`] and installed
/// into egui by [`apply`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub hero_size: f32,
    pub display_size: f32,
    /// Detail-pane commit subject.
    pub headline_size: f32,
    /// Dialog titles.
    pub title_size: f32,
    /// Card titles.
    pub subtitle_size: f32,
    /// Body, row text, buttons.
    pub body_size: f32,
    pub lead_size: f32,
    /// Metadata, toast body, field labels.
    pub meta_size: f32,
    /// Ref pills, hints.
    pub small_size: f32,
    /// Uppercase section labels.
    pub micro_size: f32,
    /// Diff text.
    pub mono_size: f32,
    pub sha_size: f32,
    /// Keycaps.
    pub chord_size: f32,
    pub icon_xs: f32,
    pub icon_sm: f32,
    pub icon_md: f32,
    pub icon_lg: f32,
    pub icon_tool: f32,
    pub icon_toast: f32,
    pub icon_xl: f32,
    pub icon_hero: f32,
    /// Rows, chips, checkboxes.
    pub radius_sm: u8,
    /// Controls.
    pub radius: u8,
    /// Floating layers and cards.
    pub radius_lg: u8,
    pub pill_radius: u8,
    pub doc_tab_radius: u8,
    pub card_radius: u8,
    /// Deeper's floating panes (0: flush).
    pub pane_radius: u8,
    /// Deeper's gap between panes and around them (0: flush).
    pub pane_gutter: f32,
    pub control_sm: f32,
    pub control: f32,
    pub control_lg: f32,
    pub button_pad_x: f32,
    pub button_pad_x_sm: f32,
    pub button_pad_y: f32,
    pub button_gap: f32,
    pub item_spacing: egui::Vec2,
    pub field_pad_x: f32,
    pub field_label_gap: f32,
    pub tab_strip_height: f32,
    pub doc_tab_height: f32,
    pub search_trigger: egui::Vec2,
    pub action_bar_height: f32,
    pub tool_height: f32,
    pub tool_layout: ToolLayout,
    pub tool_min_width: f32,
    pub tool_pad: f32,
    pub tool_gap: f32,
    pub tool_caption: f32,
    pub toolbar_gap: f32,
    pub tool_divider_height: f32,
    pub tool_divider_margin: f32,
    pub status_bar_height: f32,
    pub pane_header_height: f32,
    pub file_header_height: f32,
    pub sidebar_width: f32,
    pub sidebar_pad: f32,
    pub sidebar_row_height: f32,
    pub section_header_height: f32,
    pub section_gap: f32,
    pub tree_indent: f32,
    pub history_row_height: f32,
    pub table_header_height: f32,
    /// Graph, author, date and SHA columns; the message takes the rest.
    pub history_columns: [f32; 4],
    pub row_height: f32,
    pub detail_info_width: f32,
    pub detail_pad: egui::Vec2,
    pub detail_gap: f32,
    pub menu_item_height: f32,
    pub menu_pad: f32,
    pub palette_row_height: f32,
    pub palette_input_height: f32,
    pub diff_line_height: f32,
    pub hunk_header_height: f32,
    pub request_row_height: f32,
    pub card_pad: f32,
    pub card_gap: f32,
    pub dialog_pad: f32,
    pub dialog_gap: f32,
    pub checkbox: f32,
    pub radio: f32,
    pub switch: egui::Vec2,
    pub switch_knob: f32,
    pub switch_travel: f32,
    pub slider_rail: f32,
    pub slider_thumb: f32,
    pub slider_hit: f32,
    pub progress_height: f32,
    pub spinner: f32,
    pub avatar_row: f32,
    pub avatar_detail: f32,
    pub avatar_lg: f32,
    pub ref_pill_height: f32,
    /// Ref pill padding on the icon side, and on the other side.
    pub ref_pill_pad_icon: f32,
    pub ref_pill_pad: f32,
    pub badge_height: f32,
    pub keycap_height: f32,
    pub keycap_radius: u8,
    /// A status dot's diameter (spec 4.18: 8, 6 in Dense).
    pub status_dot: f32,
    pub segment_pad: f32,
    pub segment_track_pad: f32,
    pub underline_tab_height: f32,
    pub tooltip_pad: Margin,
    pub tooltip_size: f32,
    pub toast_pad: Margin,
    pub toast_width: f32,
    pub dialog_width: f32,
    pub dialog_width_form: f32,
    pub dialog_width_wide: f32,
    pub graph_lane: f32,
    pub graph_x0: f32,
    pub graph_node_r: f32,
    pub graph_ring: f32,
    pub graph_edge: f32,
    pub graph_merge_stroke: f32,
    pub graph_head_halo: f32,
    pub graph_head_halo_alpha: u8,
    /// Faint stripes on even history rows.
    pub zebra: bool,
}

const fn margin(top: i8, right: i8, bottom: i8, left: i8) -> Margin {
    Margin {
        left,
        right,
        top,
        bottom,
    }
}

/// Concept 1's metrics.
pub const COMFORTABLE: Metrics = Metrics {
    hero_size: 38.0,
    display_size: 24.0,
    headline_size: 15.5,
    title_size: 15.0,
    subtitle_size: 14.0,
    body_size: 13.0,
    lead_size: 13.0,
    meta_size: 12.5,
    small_size: 11.5,
    micro_size: 10.5,
    mono_size: 12.5,
    sha_size: 12.0,
    chord_size: 11.0,
    icon_xs: 12.0,
    icon_sm: 13.0,
    icon_md: 15.0,
    icon_lg: 16.0,
    icon_tool: 17.0,
    icon_toast: 18.0,
    icon_xl: 24.0,
    icon_hero: 32.0,
    radius_sm: 5,
    radius: 7,
    radius_lg: 12,
    pill_radius: FULL,
    doc_tab_radius: 7,
    card_radius: 12,
    pane_radius: 0,
    pane_gutter: 0.0,
    control_sm: 24.0,
    control: 28.0,
    control_lg: 32.0,
    button_pad_x: 12.0,
    button_pad_x_sm: 10.0,
    button_pad_y: 6.0,
    button_gap: 6.0,
    item_spacing: vec2(8.0, 6.0),
    field_pad_x: 10.0,
    field_label_gap: 6.0,
    tab_strip_height: 42.0,
    doc_tab_height: 30.0,
    search_trigger: vec2(280.0, 30.0),
    action_bar_height: 58.0,
    tool_height: 46.0,
    tool_layout: ToolLayout::Stacked,
    tool_min_width: 54.0,
    tool_pad: 9.0,
    tool_gap: 4.0,
    tool_caption: 12.5,
    toolbar_gap: 4.0,
    tool_divider_height: 30.0,
    tool_divider_margin: 8.0,
    status_bar_height: 28.0,
    pane_header_height: 40.0,
    file_header_height: 36.0,
    sidebar_width: 236.0,
    sidebar_pad: 6.0,
    sidebar_row_height: 26.0,
    section_header_height: 28.0,
    section_gap: 4.0,
    tree_indent: 20.0,
    history_row_height: 28.0,
    table_header_height: 30.0,
    history_columns: [76.0, 162.0, 116.0, 82.0],
    row_height: 30.0,
    detail_info_width: 300.0,
    detail_pad: vec2(14.0, 14.0),
    detail_gap: 10.0,
    menu_item_height: 30.0,
    menu_pad: 4.0,
    palette_row_height: 40.0,
    palette_input_height: 48.0,
    diff_line_height: 20.0,
    hunk_header_height: 24.0,
    request_row_height: 64.0,
    card_pad: 16.0,
    card_gap: 14.0,
    dialog_pad: 18.0,
    dialog_gap: 12.0,
    checkbox: 16.0,
    radio: 16.0,
    switch: vec2(32.0, 18.0),
    switch_knob: 14.0,
    switch_travel: 14.0,
    slider_rail: 4.0,
    slider_thumb: 16.0,
    slider_hit: 18.0,
    progress_height: 6.0,
    spinner: 16.0,
    avatar_row: 18.0,
    avatar_detail: 24.0,
    avatar_lg: 30.0,
    ref_pill_height: 19.0,
    ref_pill_pad_icon: 5.0,
    ref_pill_pad: 7.0,
    badge_height: 17.0,
    keycap_height: 20.0,
    keycap_radius: 5,
    status_dot: 8.0,
    segment_pad: 12.0,
    segment_track_pad: 2.0,
    underline_tab_height: 34.0,
    tooltip_pad: margin(5, 6, 5, 9),
    tooltip_size: 12.0,
    toast_pad: margin(11, 8, 11, 12),
    toast_width: 380.0,
    dialog_width: 420.0,
    dialog_width_form: 520.0,
    dialog_width_wide: 720.0,
    graph_lane: 14.0,
    graph_x0: 18.0,
    graph_node_r: 4.2,
    graph_ring: 2.0,
    graph_edge: 2.0,
    graph_merge_stroke: 2.0,
    graph_head_halo: 5.2,
    graph_head_halo_alpha: 61,
    zebra: false,
};

/// Concept 5's metrics.
pub const DENSE: Metrics = Metrics {
    headline_size: 14.0,
    subtitle_size: 13.5,
    body_size: 12.0,
    lead_size: 12.0,
    meta_size: 12.0,
    small_size: 11.0,
    mono_size: 12.0,
    chord_size: 10.5,
    icon_lg: 15.0,
    icon_tool: 15.0,
    icon_xl: 22.0,
    icon_hero: 28.0,
    radius_sm: 3,
    radius: 5,
    radius_lg: 6,
    pill_radius: 3,
    doc_tab_radius: 5,
    card_radius: 6,
    control_sm: 20.0,
    control: 24.0,
    control_lg: 28.0,
    button_pad_y: 4.0,
    item_spacing: vec2(6.0, 4.0),
    tab_strip_height: 34.0,
    doc_tab_height: 24.0,
    search_trigger: vec2(240.0, 24.0),
    action_bar_height: 36.0,
    tool_height: 26.0,
    tool_layout: ToolLayout::Inline,
    tool_min_width: 0.0,
    tool_pad: 8.0,
    tool_gap: 6.0,
    tool_caption: 12.0,
    toolbar_gap: 2.0,
    tool_divider_height: 18.0,
    tool_divider_margin: 6.0,
    status_bar_height: 24.0,
    pane_header_height: 32.0,
    file_header_height: 30.0,
    sidebar_width: 210.0,
    sidebar_row_height: 22.0,
    section_header_height: 24.0,
    section_gap: 2.0,
    history_row_height: 24.0,
    table_header_height: 26.0,
    history_columns: [66.0, 150.0, 104.0, 74.0],
    row_height: 24.0,
    detail_info_width: 270.0,
    detail_pad: vec2(12.0, 10.0),
    detail_gap: 8.0,
    menu_item_height: 24.0,
    palette_row_height: 32.0,
    palette_input_height: 40.0,
    diff_line_height: 18.0,
    request_row_height: 52.0,
    card_pad: 12.0,
    card_gap: 10.0,
    checkbox: 14.0,
    radio: 14.0,
    switch: vec2(28.0, 16.0),
    switch_knob: 12.0,
    switch_travel: 12.0,
    avatar_row: 16.0,
    ref_pill_height: 17.0,
    ref_pill_pad_icon: 4.0,
    ref_pill_pad: 5.0,
    keycap_height: 18.0,
    keycap_radius: 4,
    status_dot: 6.0,
    segment_pad: 9.0,
    icon_toast: 15.0,
    toast_pad: margin(8, 6, 8, 9),
    toast_width: 360.0,
    dialog_width_form: 500.0,
    dialog_width_wide: 680.0,
    graph_lane: 12.0,
    graph_x0: 15.0,
    graph_node_r: 3.6,
    graph_edge: 1.8,
    zebra: true,
    ..COMFORTABLE
};

/// Concept 4's geometry over Comfortable.
pub const DEEPER_COMFORTABLE: Metrics = Metrics {
    radius_sm: 6,
    radius: 10,
    radius_lg: 14,
    pill_radius: 6,
    control: 30.0,
    doc_tab_radius: 10,
    card_radius: 14,
    pane_radius: 10,
    pane_gutter: 6.0,
    action_bar_height: 56.0,
    ..COMFORTABLE
};

/// Concept 4's step scaled by concept 5's, over Dense (no concept drew this pair).
pub const DEEPER_DENSE: Metrics = Metrics {
    radius_sm: 4,
    radius: 7,
    radius_lg: 8,
    pill_radius: 4,
    control: 26.0,
    doc_tab_radius: 7,
    card_radius: 8,
    pane_radius: 8,
    pane_gutter: 4.0,
    action_bar_height: 36.0,
    ..DENSE
};

/// The metrics of a (density, style) pair.
pub fn metrics_for(density: Density, style: Style) -> &'static Metrics {
    match (density, style) {
        (Density::Comfortable, Style::Standard) => &COMFORTABLE,
        (Density::Dense, Style::Standard) => &DENSE,
        (Density::Comfortable, Style::Deeper) => &DEEPER_COMFORTABLE,
        (Density::Dense, Style::Deeper) => &DEEPER_DENSE,
    }
}

/// The metrics of a (density, style) pair in `theme`: every corner through the theme's radius
/// scale ([`Design::radius`]), so the kit and the views draw the theme's corners by reading
/// [`metrics`] as they always have.
pub fn metrics_in(theme: ThemeId, density: Density, style: Style) -> &'static Metrics {
    const PAIRS: [(Density, Style); 4] = [
        (Density::Comfortable, Style::Standard),
        (Density::Dense, Style::Standard),
        (Density::Comfortable, Style::Deeper),
        (Density::Dense, Style::Deeper),
    ];
    static SHAPED: LazyLock<Vec<Metrics>> = LazyLock::new(|| {
        ThemeId::BUILT_IN
            .into_iter()
            .flat_map(|t| PAIRS.map(|(d, s)| metrics_for(d, s).shaped(t.design())))
            .collect()
    });
    let pair = PAIRS
        .iter()
        .position(|&pair| pair == (density, style))
        .unwrap_or(0);
    SHAPED
        .get(usize::from(theme.built_in().index()) * PAIRS.len() + pair)
        .unwrap_or_else(|| metrics_for(density, style))
}

/// The active metrics, in the active theme's corners.
pub fn metrics() -> &'static Metrics {
    if high_contrast() {
        static SHAPED: LazyLock<[Metrics; 4]> = LazyLock::new(|| {
            [
                (Density::Comfortable, Style::Standard),
                (Density::Dense, Style::Standard),
                (Density::Comfortable, Style::Deeper),
                (Density::Dense, Style::Deeper),
            ]
            .map(|(d, s)| metrics_for(d, s).shaped(&HIGH_CONTRAST_DESIGN))
        });
        let i =
            usize::from(density() == Density::Dense) + 2 * usize::from(style() == Style::Deeper);
        return &SHAPED[i];
    }
    metrics_in(active(), density(), style())
}

impl Metrics {
    /// These metrics with every radius in `design`'s scale.
    #[must_use]
    pub fn shaped(&self, design: &Design) -> Self {
        Self {
            radius_sm: design.radius(self.radius_sm),
            radius: design.radius(self.radius),
            radius_lg: design.radius(self.radius_lg),
            pill_radius: design.radius(self.pill_radius),
            doc_tab_radius: design.radius(self.doc_tab_radius),
            card_radius: design.radius(self.card_radius),
            pane_radius: design.radius(self.pane_radius),
            keycap_radius: design.radius(self.keycap_radius),
            ..*self
        }
    }

    /// Every token as (name, value), for the gallery's token sheet.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        let f = |v: f32| format!("{v}");
        let r = |v: u8| {
            if v == FULL {
                "full".to_owned()
            } else {
                v.to_string()
            }
        };
        let v2 = |v: egui::Vec2| format!("{} x {}", v.x, v.y);
        let m = |v: Margin| format!("{} {} {} {}", v.top, v.right, v.bottom, v.left);
        vec![
            (
                "hero / display",
                format!("{} / {}", self.hero_size, self.display_size),
            ),
            (
                "headline / title / subtitle",
                format!(
                    "{} / {} / {}",
                    self.headline_size, self.title_size, self.subtitle_size
                ),
            ),
            (
                "body / lead / meta",
                format!(
                    "{} / {} / {}",
                    self.body_size, self.lead_size, self.meta_size
                ),
            ),
            (
                "small / micro",
                format!("{} / {}", self.small_size, self.micro_size),
            ),
            (
                "mono / sha / chord",
                format!(
                    "{} / {} / {}",
                    self.mono_size, self.sha_size, self.chord_size
                ),
            ),
            (
                "icon xs / sm / md / lg",
                format!(
                    "{} / {} / {} / {}",
                    self.icon_xs, self.icon_sm, self.icon_md, self.icon_lg
                ),
            ),
            (
                "icon tool / toast / xl / hero",
                format!(
                    "{} / {} / {} / {}",
                    self.icon_tool, self.icon_toast, self.icon_xl, self.icon_hero
                ),
            ),
            (
                "radius sm / md / lg / pill",
                format!(
                    "{} / {} / {} / {}",
                    r(self.radius_sm),
                    r(self.radius),
                    r(self.radius_lg),
                    r(self.pill_radius)
                ),
            ),
            (
                "doc tab / card / pane radius",
                format!(
                    "{} / {} / {}",
                    r(self.doc_tab_radius),
                    r(self.card_radius),
                    r(self.pane_radius)
                ),
            ),
            ("pane gutter", f(self.pane_gutter)),
            (
                "control sm / md / lg",
                format!(
                    "{} / {} / {}",
                    self.control_sm, self.control, self.control_lg
                ),
            ),
            (
                "button pad x / x sm / y / gap",
                format!(
                    "{} / {} / {} / {}",
                    self.button_pad_x, self.button_pad_x_sm, self.button_pad_y, self.button_gap
                ),
            ),
            ("item spacing", v2(self.item_spacing)),
            (
                "field pad x / label gap",
                format!("{} / {}", self.field_pad_x, self.field_label_gap),
            ),
            (
                "tab strip / doc tab",
                format!("{} / {}", self.tab_strip_height, self.doc_tab_height),
            ),
            ("search trigger", v2(self.search_trigger)),
            (
                "action bar / tool",
                format!(
                    "{} / {} ({:?})",
                    self.action_bar_height, self.tool_height, self.tool_layout
                ),
            ),
            (
                "tool min w / pad / gap / caption",
                format!(
                    "{} / {} / {} / {}",
                    self.tool_min_width, self.tool_pad, self.tool_gap, self.tool_caption
                ),
            ),
            (
                "toolbar gap / divider h / margin",
                format!(
                    "{} / {} / {}",
                    self.toolbar_gap, self.tool_divider_height, self.tool_divider_margin
                ),
            ),
            ("status bar", f(self.status_bar_height)),
            (
                "pane header / file header",
                format!("{} / {}", self.pane_header_height, self.file_header_height),
            ),
            (
                "sidebar width / pad",
                format!("{} / {}", self.sidebar_width, self.sidebar_pad),
            ),
            (
                "sidebar row / section / gap",
                format!(
                    "{} / {} / {}",
                    self.sidebar_row_height, self.section_header_height, self.section_gap
                ),
            ),
            ("tree indent", f(self.tree_indent)),
            (
                "history row / table header",
                format!("{} / {}", self.history_row_height, self.table_header_height),
            ),
            ("history columns", format!("{:?}", self.history_columns)),
            ("row", f(self.row_height)),
            (
                "detail info / pad / gap",
                format!(
                    "{} / {} / {}",
                    self.detail_info_width,
                    v2(self.detail_pad),
                    self.detail_gap
                ),
            ),
            (
                "menu item / pad",
                format!("{} / {}", self.menu_item_height, self.menu_pad),
            ),
            (
                "palette row / input",
                format!(
                    "{} / {}",
                    self.palette_row_height, self.palette_input_height
                ),
            ),
            (
                "diff line / hunk header",
                format!("{} / {}", self.diff_line_height, self.hunk_header_height),
            ),
            ("request row", f(self.request_row_height)),
            (
                "card pad / gap",
                format!("{} / {}", self.card_pad, self.card_gap),
            ),
            (
                "dialog pad / gap",
                format!("{} / {}", self.dialog_pad, self.dialog_gap),
            ),
            (
                "checkbox / radio",
                format!("{} / {}", self.checkbox, self.radio),
            ),
            (
                "switch / knob / travel",
                format!(
                    "{} / {} / {}",
                    v2(self.switch),
                    self.switch_knob,
                    self.switch_travel
                ),
            ),
            (
                "slider rail / thumb / hit",
                format!(
                    "{} / {} / {}",
                    self.slider_rail, self.slider_thumb, self.slider_hit
                ),
            ),
            (
                "progress / spinner",
                format!("{} / {}", self.progress_height, self.spinner),
            ),
            (
                "avatar row / detail / lg",
                format!(
                    "{} / {} / {}",
                    self.avatar_row, self.avatar_detail, self.avatar_lg
                ),
            ),
            (
                "ref pill h / pad icon / pad",
                format!(
                    "{} / {} / {}",
                    self.ref_pill_height, self.ref_pill_pad_icon, self.ref_pill_pad
                ),
            ),
            (
                "badge / keycap / keycap radius",
                format!(
                    "{} / {} / {}",
                    self.badge_height,
                    self.keycap_height,
                    r(self.keycap_radius)
                ),
            ),
            (
                "segment pad / track pad",
                format!("{} / {}", self.segment_pad, self.segment_track_pad),
            ),
            ("underline tab", f(self.underline_tab_height)),
            (
                "tooltip pad / size",
                format!("{} / {}", m(self.tooltip_pad), self.tooltip_size),
            ),
            (
                "toast pad / width",
                format!("{} / {}", m(self.toast_pad), self.toast_width),
            ),
            (
                "dialog widths",
                format!(
                    "{} / {} / {}",
                    self.dialog_width, self.dialog_width_form, self.dialog_width_wide
                ),
            ),
            (
                "graph lane / x0",
                format!("{} / {}", self.graph_lane, self.graph_x0),
            ),
            (
                "graph node / ring / edge / merge",
                format!(
                    "{} / {} / {} / {}",
                    self.graph_node_r, self.graph_ring, self.graph_edge, self.graph_merge_stroke
                ),
            ),
            (
                "graph head halo",
                format!(
                    "+{} at {}",
                    self.graph_head_halo, self.graph_head_halo_alpha
                ),
            ),
            ("zebra", self.zebra.to_string()),
        ]
    }
}

// ------------------------------------------------------------------------------------------ type

/// Family of the semibold UI face.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

/// Family of the medium UI face.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

/// A section label (sidebar sections, settings groups, table column headers, menu headings): in
/// capitals at `micro_size`, semibold, with the active theme's tracking ([`LabelStyle`]).
pub fn section_label(text: &str, color: Color32) -> egui::text::LayoutJob {
    let style = active_design().labels;
    let size = metrics().micro_size;
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &text.to_uppercase(),
        0.0,
        egui::TextFormat {
            font_id: FontId::new(size, semibold()),
            color,
            extra_letter_spacing: style.tracking * size,
            ..Default::default()
        },
    );
    job
}

/// A title or headline at `size` (`Metrics::title_size`, `headline_size`, `subtitle_size` for card
/// titles, `display_size`, `hero_size`) in the active theme's heading face ([`HeadingFace`]).
pub fn heading(size: f32) -> FontId {
    let face = active_design().heading;
    FontId::new(size * face.size_scale(), face.family())
}

/// An icon glyph's font at `size`: the filled Phosphor glyph when `lit` (active, selected or
/// toggled) and the theme fills its active icons ([`IconWeight::FillAccents`]), else the regular
/// one.
pub fn icon_font(size: f32, lit: bool) -> FontId {
    if lit && active_design().icons == IconWeight::FillAccents {
        FontId::new(size, FontFamily::Name(super::fonts::FILL.into()))
    } else {
        FontId::proportional(size)
    }
}

// ------------------------------------------------------------------------------- egui mapping

/// The largest `Floating` layer of `style` as one egui shadow (egui's own windows and popups).
fn floating_shadow(p: &Palette, style: Style) -> Shadow {
    let m = p.material(style, Elevation::Floating);
    m.layers
        .iter()
        .max_by_key(|l| l.blur)
        .map_or(Shadow::NONE, |l| Shadow {
            offset: [0, l.dy],
            blur: l.blur,
            spread: 0,
            color: Palette::tint(p.shadow_color(l.tint), l.alpha),
        })
}

/// egui's visuals for `p` in `style` with metrics `m`, so any stock widget left over (and egui's
/// own submenus, tooltips, scrollbars and resize handles) matches the kit.
fn visuals(p: &Palette, style: Style, m: &Metrics) -> Visuals {
    let s = p.surfaces(style);
    let mut v = if p.dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.dark_mode = p.dark;
    v.text_options.color_transfer_function = if p.dark {
        egui::epaint::FontColorTransferFunction::TwoCoverageMinusCoverageSq
    } else {
        egui::epaint::FontColorTransferFunction::Off
    };
    v.panel_fill = s.panel;
    v.window_fill = s.overlay;
    v.extreme_bg_color = s.field;
    v.text_edit_bg_color = Some(s.field);
    v.faint_bg_color = over(p.hover(), s.pane);
    v.code_bg_color = s.field;
    v.hyperlink_color = p.accent2;
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;
    v.window_stroke = Stroke::new(1.0, p.floating_ring(s));
    v.window_corner_radius = CornerRadius::same(m.radius_lg);
    v.menu_corner_radius = CornerRadius::same(m.radius_lg);
    v.window_shadow = floating_shadow(p, style);
    v.popup_shadow = floating_shadow(p, style);
    // Selected list rows and egui's selected values and text share one fill.
    v.selection.bg_fill = p.selected();
    // Also egui's keyboard focus ring: fully opaque so focus is always visible.
    v.selection.stroke = Stroke::new(2.0, p.focus);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    v.text_cursor.blink = true;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;
    v.indent_has_left_vline = false;
    v.collapsing_header_frame = false;
    v.button_frame = true;
    v.interact_cursor = None;
    v.image_loading_spinners = false;
    v.disabled_alpha = DISABLED_ALPHA;

    let w = &mut v.widgets;
    let r = CornerRadius::same(m.radius);
    w.noninteractive.bg_fill = s.panel;
    w.noninteractive.weak_bg_fill = s.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, s.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;
    w.inactive.bg_fill = s.raised.bottom;
    w.inactive.weak_bg_fill = s.raised.bottom;
    w.inactive.bg_stroke = Stroke::new(1.0, p.control_border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;
    w.inactive.expansion = 0.0;
    w.hovered.bg_fill = s.raised_hover.bottom;
    w.hovered.weak_bg_fill = s.raised_hover.bottom;
    w.hovered.bg_stroke = Stroke::new(1.0, p.control_border);
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;
    w.active.bg_fill = s.raised_pressed.bottom;
    w.active.weak_bg_fill = s.raised_pressed.bottom;
    w.active.bg_stroke = Stroke::new(1.0, p.focus);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;
    w.open = w.hovered;
    v
}

/// Floating scroll bars (section 4.21): thin until hovered, no gutter, no built-in fade.
fn floating_scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        floating: true,
        floating_width: 4.0,
        bar_width: 8.0,
        floating_allocated_width: 0.0,
        handle_min_length: SCROLL_HANDLE_MIN,
        bar_inner_margin: 2.0,
        bar_outer_margin: 0.0,
        dormant_handle_opacity: 0.0,
        active_handle_opacity: 0.45,
        interact_handle_opacity: 0.75,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.4,
        foreground_color: true,
        fade: egui::style::ScrollFadeStyle {
            strength: 0.0,
            ..Default::default()
        },
        ..egui::style::ScrollStyle::floating()
    }
}

/// The shortest a scrollbar's handle gets, so a long document's handle is still easy to see and
/// to grab (egui's default is 12).
pub const SCROLL_HANDLE_MIN: f32 = 32.0;

/// Always-visible scroll bars for code, diff and detail boxes, for position awareness (section
/// 4.21): the floating bars' thin handle in the text colour at 45% (75% while hovered or dragged),
/// with its room kept beside the content so the handle never covers text.
pub fn solid_scroll() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        floating_allocated_width: SM,
        dormant_handle_opacity: 0.45,
        active_handle_opacity: 0.45,
        ..floating_scroll()
    }
}

fn look_set(id: ThemeId, style: Style, density: Density, glass: bool) {
    look(|l| {
        l.theme.store(id.index(), Ordering::Relaxed);
        l.style
            .store(u8::from(style == Style::Deeper), Ordering::Relaxed);
        l.density
            .store(u8::from(density == Density::Dense), Ordering::Relaxed);
        l.glass.store(glass, Ordering::Relaxed);
    });
}

/// Runs `f` with `style` and `density` active, then restores the installed ones: the gallery's
/// side-by-side strips paint both styles in one frame. egui's own visuals stay as installed.
pub fn with_look<R>(style: Style, density: Density, f: impl FnOnce() -> R) -> R {
    let (id, before) = (active(), appearance());
    look_set(id, style, density, before.glass);
    let r = f();
    look_set(id, before.style, before.density, before.glass);
    r
}

/// Makes `id` the active theme with `look`'s style, density, glass and motion, and installs the
/// matching egui style on `ctx`.
pub fn apply(ctx: &egui::Context, id: ThemeId, look: Appearance) {
    look_set(id, look.style, look.density, look.glass);
    super::motion::set_reduced(look.reduce_motion);
    let p = active_palette();
    let m = metrics();
    let theme = if p.dark { Theme::Dark } else { Theme::Light };
    let mut style = (*ctx.style_of(theme)).clone();
    style.visuals = visuals(p, look.style, m);
    let face = active_design().heading;
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(m.title_size * face.size_scale(), face.family()),
        ),
        (
            TextStyle::Body,
            FontId::new(m.body_size, FontFamily::Proportional),
        ),
        (TextStyle::Button, FontId::new(m.body_size, medium())),
        (
            TextStyle::Small,
            FontId::new(m.small_size, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(m.mono_size, FontFamily::Monospace),
        ),
    ]
    .into();
    let sp = &mut style.spacing;
    sp.item_spacing = m.item_spacing;
    sp.button_padding = vec2(m.button_pad_x, m.button_pad_y);
    sp.interact_size.y = m.control;
    sp.window_margin = Margin::same(crate::points_i8(m.dialog_pad));
    sp.menu_margin = Margin::same(crate::points_i8(m.menu_pad));
    sp.menu_spacing = 1.0;
    sp.indent = m.tree_indent;
    sp.icon_width = m.checkbox;
    sp.icon_width_inner = m.checkbox / 2.0;
    sp.icon_spacing = 9.0;
    sp.tooltip_width = 260.0;
    sp.menu_width = 360.0;
    sp.scroll = floating_scroll();
    style.interaction.selectable_labels = false;
    style.interaction.tooltip_delay = super::motion::TOOLTIP_DELAY;
    style.interaction.tooltip_grace_time = super::motion::TOOLTIP_SKIP;
    style.animation_time = super::motion::time(super::motion::BASE);
    style.scroll_animation = super::motion::scroll_animation();
    ctx.set_style_of(theme, style);
    ctx.set_theme(theme);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every palette shown: each built-in theme and a sweep of custom ones (both bases, every
    /// accent swatch at the weakest and strongest intensity, no tint and full tints around the
    /// wheel), each also at high contrast.
    fn every_palette() -> Vec<&'static Palette> {
        let mut all: Vec<&'static Palette> = ThemeId::BUILT_IN
            .iter()
            .flat_map(|t| [t.palette(), t.shown_palette(true)])
            .collect();
        for choice in custom_sweep() {
            let built = build_custom(choice);
            all.extend([built.palette, built.high_contrast]);
        }
        all
    }

    /// The ink cache tells palettes apart: a theme and its high-contrast version share a name,
    /// and every custom palette is named Custom, yet each gets the ink worked out for itself.
    #[test]
    fn each_palette_keeps_its_own_inks() {
        let red = build_custom(CustomTheme {
            accent_hue: 28,
            ..CustomTheme::default()
        });
        let green = build_custom(CustomTheme {
            accent_hue: 150,
            ..CustomTheme::default()
        });
        let palettes = [
            ThemeId::Midnight.palette(),
            ThemeId::Midnight.shown_palette(true),
            red.palette,
            green.palette,
        ];
        let base = ThemeId::Midnight.palette();
        let (fg, fill) = (base.lane(3), Palette::tint(base.text_faint, 87));
        let fresh: Vec<Color32> = palettes
            .iter()
            .map(|p| p.ink_on(p.surfaces(style()), fg, fill))
            .collect();
        assert!(
            fresh.windows(2).any(|w| w[0] != w[1]),
            "the palettes must disagree for the test to tell them apart"
        );
        for (p, want) in palettes.iter().zip(fresh) {
            assert_eq!(p.ink(fg, fill), want, "{}", p.name);
        }
    }

    /// Custom choices the contrast tests run over: every swatch hue and hues between them, the
    /// extremes of intensity, no tint and full tints around the wheel, plus choices that once
    /// missed a gate by a hair (a vivid red selection on Daylight, near-black tinted grounds).
    /// The fix was checked over 2,880 choices (hue every 10 degrees, four intensities, ten tints).
    fn custom_sweep() -> Vec<CustomTheme> {
        let hues = super::super::custom_theme::HUES
            .iter()
            .map(|(h, _)| *h)
            .chain((7..360).step_by(30));
        let mut out = Vec::new();
        for light in [false, true] {
            for accent_hue in hues.clone() {
                for accent_intensity in [20, 140] {
                    for (tint, tint_hue) in [(0, 0), (100, 28), (100, 150), (100, 250), (100, 335)]
                    {
                        out.push(CustomTheme {
                            light,
                            accent_hue,
                            accent_intensity,
                            tint_hue,
                            tint,
                        });
                    }
                }
            }
        }
        for (light, accent_hue, accent_intensity, tint_hue, tint) in [
            (true, 21, 140, 240, 100),
            (true, 21, 140, 0, 0),
            (true, 10, 140, 190, 100),
            (false, 0, 20, 240, 100),
            (false, 234, 52, 335, 100),
        ] {
            out.push(CustomTheme {
                light,
                accent_hue,
                accent_intensity,
                tint_hue,
                tint,
            });
        }
        out
    }

    /// The custom theme starts as its base theme: the default choices change no colour.
    #[test]
    fn the_default_custom_theme_is_midnight_and_its_light_one_daylight() {
        for (light, base) in [(false, &MIDNIGHT), (true, &DAYLIGHT)] {
            let custom = CustomTheme {
                light,
                ..CustomTheme::default()
            }
            .palette();
            assert_eq!(custom.name, "Custom");
            assert_eq!(
                format!("{:?}", custom.standard),
                format!("{:?}", base.standard)
            );
            assert_eq!(format!("{:?}", custom.deeper), format!("{:?}", base.deeper));
            for (got, want) in [
                (custom.text, base.text),
                (custom.text_dim, base.text_dim),
                (custom.accent, base.accent),
                (custom.focus, base.focus),
                (custom.selection, base.selection),
                (custom.on_accent, base.on_accent),
                (custom.control_border, base.control_border),
            ] {
                assert_eq!(got, want, "{}", base.name);
            }
        }
    }

    /// The accent recolours the indigo family and the tint the neutrals, at the base's luminance;
    /// status colours, lanes and syntax stay the base's.
    #[test]
    fn custom_choices_recolour_the_accent_and_the_neutrals_only() {
        let green = CustomTheme {
            accent_hue: 150,
            tint: 100,
            tint_hue: 150,
            ..CustomTheme::default()
        }
        .palette();
        let hue = |c: Color32| {
            let [_, a, b] = super::super::contrast::oklab(c);
            b.atan2(a).to_degrees().rem_euclid(360.0)
        };
        for c in [green.standard.primary.top, green.accent, green.focus] {
            assert!((hue(c) - 150.0).abs() < 15.0, "{c:?} is not green");
        }
        assert!(
            (hue(green.standard.bg) - 150.0).abs() < 15.0,
            "the ground takes the tint"
        );
        let delta = (luminance(green.standard.bg) - luminance(MIDNIGHT.standard.bg)).abs();
        assert!(delta < 0.002, "the ground keeps its luminance ({delta})");
        // Status colours, lanes and syntax keep their hues; contrast may move them a little.
        let pairs = [
            (green.success, MIDNIGHT.success),
            (green.danger, MIDNIGHT.danger),
            (green.syntax.keyword, MIDNIGHT.syntax.keyword),
        ];
        for (got, want) in pairs
            .into_iter()
            .chain(green.lanes.into_iter().zip(MIDNIGHT.lanes))
        {
            let d = (hue(got) - hue(want)).abs();
            assert!(d.min(360.0 - d) < 4.0, "{got:?} left {want:?}'s hue");
        }
    }

    /// Choosing the custom theme shows the colours last set, at high contrast too, and the inks
    /// worked out for one palette are never reused for another of the same name.
    #[test]
    fn the_custom_theme_shows_the_colours_last_set() {
        let ctx = egui::Context::default();
        let teal = CustomTheme {
            accent_hue: 205,
            ..CustomTheme::default()
        };
        set_custom(teal);
        apply(&ctx, ThemeId::Custom, Appearance::STILL);
        assert_eq!(custom(), teal);
        let shown = palette(&ctx);
        assert_eq!(shown.accent, teal.palette().accent);
        let ink = shown.ink(shown.accent, Color32::TRANSPARENT);
        set_custom(CustomTheme::default());
        apply(&ctx, ThemeId::Custom, Appearance::STILL);
        let midnight_ink = palette(&ctx).ink(palette(&ctx).accent, Color32::TRANSPARENT);
        assert_ne!(ink, midnight_ink, "a stale ink from the teal palette");
        set_high_contrast(true);
        assert!(ThemeId::Custom.shown_palette(true).strict);
        set_high_contrast(false);
        assert_eq!(
            ThemeId::Custom.design().heading,
            MIDNIGHT_DESIGN.heading,
            "the custom theme takes its base's structure"
        );
    }

    /// Out-of-range choices from a hand-edited settings file are brought into range.
    #[test]
    fn custom_choices_are_clamped() {
        let c = CustomTheme {
            light: true,
            accent_hue: 725,
            accent_intensity: 250,
            tint_hue: 360,
            tint: 180,
        }
        .clamped();
        assert_eq!(
            (c.accent_hue, c.accent_intensity, c.tint_hue, c.tint),
            (5, 140, 0, 100)
        );
    }

    /// How far apart two colours look: their `OKLab` distance, times 100.
    fn oklab_distance(a: Color32, b: Color32) -> f32 {
        let (a, b) = (
            super::super::contrast::oklab(a),
            super::super::contrast::oklab(b),
        );
        100.0 * ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    /// Collects failed pairings so one run lists all of them.
    struct Gate {
        failures: Vec<String>,
        name: &'static str,
        style: Style,
    }

    impl Gate {
        fn new(p: &Palette, style: Style) -> Self {
            Self {
                failures: Vec::new(),
                name: p.name,
                style,
            }
        }

        fn need(&mut self, what: &str, fg: Color32, on: &str, bg: Color32, min: f64) {
            let r = contrast(fg, bg);
            if r < min - 1e-9 {
                self.failures.push(format!(
                    "{}/{:?}: {what} on {on} is {r:.2}:1, needs {min}:1",
                    self.name, self.style
                ));
            }
        }
    }

    /// (`text`, `text_dim`, `text_faint`, `outline` and `focus`) minimums: high contrast is
    /// stricter.
    fn targets(p: &Palette) -> (f64, f64, f64, f64) {
        if p.strict {
            (10.0, 7.0, 5.5, 4.5)
        } else {
            (7.0, 4.5, 4.5, 3.0)
        }
    }

    /// The surfaces text sits on in `style`: the four grounds (and Deeper's window ground) plus
    /// field, track, a Dense zebra row and every hover state over each ground.
    fn grounds(s: &Surfaces, style: Style) -> Vec<(String, Color32)> {
        let mut g = vec![
            ("pane".to_owned(), s.pane),
            ("panel".to_owned(), s.panel),
            ("elevated".to_owned(), s.elevated),
            ("overlay".to_owned(), s.overlay),
        ];
        if style == Style::Deeper {
            g.push(("deeper ground".to_owned(), s.bg));
        }
        g
    }

    fn with_hovers(p: &Palette, base: &[(String, Color32)]) -> Vec<(String, Color32)> {
        let mut out = base.to_vec();
        for (on, s) in base {
            for (k, ink) in [
                ("hover", p.hover()),
                ("hover_strong", p.hover_strong()),
                ("pressed", p.pressed()),
            ] {
                out.push((format!("{k} over {on}"), over(ink, *s)));
            }
        }
        out
    }

    /// WCAG 2.2 for every pairing the views and the kit draw, in every theme and both styles
    /// (design spec 3.3 items 1, 3 and 5; `spec/themes.py` runs the same checks).
    #[test]
    fn every_theme_meets_contrast_targets() {
        let mut failures = Vec::new();
        for p in every_palette() {
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, dim_min, faint_min, rim_min) = targets(p);
                let sel = p.selected();
                let surfaces = grounds(s, style);
                let mut plain = surfaces.clone();
                plain.extend([
                    ("field".to_owned(), s.field),
                    ("track".to_owned(), s.track),
                    ("zebra row".to_owned(), over(p.zebra(), s.pane)),
                ]);
                for (on, bg) in &surfaces {
                    for (k, ink) in [
                        ("hover", p.hover()),
                        ("hover_strong", p.hover_strong()),
                        ("pressed", p.pressed()),
                    ] {
                        plain.push((format!("{k} over {on}"), over(ink, *bg)));
                    }
                }
                let mut selrows: Vec<(String, Color32)> = surfaces
                    .iter()
                    .map(|(on, bg)| (format!("selected row on {on}"), over(sel, *bg)))
                    .collect();
                selrows.extend(surfaces[..2].iter().map(|(on, bg)| {
                    (
                        format!("hovered selected row on {on}"),
                        over(p.hover(), over(sel, *bg)),
                    )
                }));
                let warning_tint = over(p.warning_tint(), s.pane);
                let diff = [
                    ("added_bg".to_owned(), s.added_bg),
                    ("removed_bg".to_owned(), s.removed_bg),
                    ("added_gutter".to_owned(), s.added_gutter),
                    ("removed_gutter".to_owned(), s.removed_gutter),
                    ("selected added line".to_owned(), over(sel, s.added_bg)),
                    ("selected removed line".to_owned(), over(sel, s.removed_bg)),
                    ("hovered added line".to_owned(), over(p.hover(), s.added_bg)),
                    (
                        "hovered removed line".to_owned(),
                        over(p.hover(), s.removed_bg),
                    ),
                    ("warning tint".to_owned(), warning_tint),
                ];
                for (on, bg) in plain.iter().chain(&selrows).chain(&diff) {
                    gate.need("text", p.text, on, *bg, text_min);
                    gate.need("text_dim", p.text_dim, on, *bg, dim_min);
                    gate.need("text_faint", p.text_faint, on, *bg, faint_min);
                }
                for (on, bg) in &plain {
                    gate.need("accent", p.accent, on, *bg, 4.5);
                    gate.need("control_border", p.control_border, on, *bg, rim_min);
                }
                for (on, bg) in plain.iter().chain(&selrows) {
                    gate.need("accent2", p.accent2, on, *bg, 4.5);
                    gate.need("success", p.success, on, *bg, 4.5);
                    gate.need("warning", p.warning, on, *bg, 4.5);
                    gate.need("danger", p.danger, on, *bg, 4.5);
                    gate.need("diff_added", p.diff_added, on, *bg, 4.5);
                    gate.need("diff_removed", p.diff_removed, on, *bg, 4.5);
                }
                gate.need("diff_added", p.diff_added, "added_bg", s.added_bg, 4.5);
                gate.need(
                    "diff_removed",
                    p.diff_removed,
                    "removed_bg",
                    s.removed_bg,
                    4.5,
                );
                gate.need(
                    "diff_added",
                    p.diff_added,
                    "added_gutter",
                    s.added_gutter,
                    4.5,
                );
                gate.need(
                    "diff_removed",
                    p.diff_removed,
                    "removed_gutter",
                    s.removed_gutter,
                    4.5,
                );
                gate.need("warning", p.warning, "warning tint", warning_tint, 4.5);
                let lane_bgs = [
                    ("pane", s.pane),
                    ("panel", s.panel),
                    ("hover over pane", over(p.hover(), s.pane)),
                    ("selected row on pane", over(sel, s.pane)),
                ];
                for (i, lane) in p.lanes.iter().enumerate() {
                    for (on, bg) in lane_bgs {
                        gate.need(&format!("lane {i}"), *lane, on, bg, 3.0);
                    }
                }
                let sx = p.syntax;
                for (what, fg) in [
                    ("keyword", sx.keyword),
                    ("string", sx.string),
                    ("comment", sx.comment),
                    ("number", sx.number),
                    ("function", sx.function),
                    ("type", sx.type_),
                    ("constant", sx.constant),
                ] {
                    for (on, bg) in [
                        ("pane", s.pane),
                        ("added_bg", s.added_bg),
                        ("removed_bg", s.removed_bg),
                    ] {
                        gate.need(&format!("syntax {what}"), fg, on, bg, 4.5);
                    }
                }

                let fields = p.fields;
                for (what, fg) in [
                    ("number", fields.number),
                    ("author", fields.author),
                    ("time", fields.time),
                    ("label", fields.label),
                    ("comments", fields.comments),
                ] {
                    for (on, bg) in plain.iter().chain(&selrows) {
                        gate.need(&format!("fields {what}"), fg, on, *bg, 4.5);
                    }
                }

                // Kit pairings.
                for (name, fill) in [
                    ("primary", s.primary),
                    ("primary_hover", s.primary_hover),
                    ("primary_pressed", s.primary_pressed),
                ] {
                    for c in fill.ends() {
                        gate.need("on_accent", p.on_accent, name, c, 4.5);
                    }
                }
                for (name, fill) in [
                    ("danger_solid", s.danger_solid),
                    ("danger_solid_hover", s.danger_solid_hover),
                    ("danger_solid_pressed", s.danger_solid_pressed),
                ] {
                    for c in fill.ends() {
                        gate.need("on_danger", p.on_danger, name, c, 4.5);
                    }
                }
                for (name, fill) in [
                    ("raised", s.raised),
                    ("raised_hover", s.raised_hover),
                    ("raised_pressed", s.raised_pressed),
                    ("thumb", s.thumb),
                    ("tab", s.tab),
                ] {
                    for c in fill.ends() {
                        gate.need("text", p.text, name, c, text_min);
                        gate.need("text_dim", p.text_dim, name, c, dim_min);
                    }
                }
                for c in s.keycap.ends() {
                    gate.need("text_dim", p.text_dim, "keycap", c, dim_min);
                }
                gate.need("text", p.text, "badge", s.badge, text_min);
                for c in s.tooltip.ends() {
                    gate.need("on_tooltip", p.on_tooltip, "tooltip", c, text_min);
                    gate.need("on_tooltip_dim", p.on_tooltip_dim, "tooltip", c, dim_min);
                }
                gate.need(
                    "on_tooltip_key",
                    s.on_tooltip_key,
                    "tooltip_key",
                    s.tooltip_key,
                    dim_min,
                );
                let mut focus_bgs: Vec<(String, Color32)> =
                    plain.iter().chain(&selrows).cloned().collect();
                focus_bgs.extend([
                    ("added_bg".to_owned(), s.added_bg),
                    ("removed_bg".to_owned(), s.removed_bg),
                    ("raised.top".to_owned(), s.raised.top),
                    ("raised.bottom".to_owned(), s.raised.bottom),
                ]);
                for (on, bg) in &focus_bgs {
                    gate.need("focus", p.focus, on, *bg, rim_min);
                }
                if p.dark {
                    for c in s.knob.ends() {
                        gate.need("switch knob", c, "switch_off", s.switch_off, 3.0);
                    }
                }
                for (name, c) in [
                    ("accent", p.accent),
                    ("success", p.success),
                    ("warning", p.warning),
                    ("danger", p.danger),
                ] {
                    let tint = Palette::tint(c, TONE_ALPHA);
                    for (on, bg) in &surfaces[..4] {
                        let b = over(tint, *bg);
                        gate.need(
                            "text",
                            p.text,
                            &format!("{name} tint over {on}"),
                            b,
                            text_min,
                        );
                        gate.need(name, c, &format!("its tint over {on}"), b, 4.5);
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The tokens added after the owner's preview review: concept 1's danger red, the Deeper top
    /// deck, the table header strip and its seam, and the light switch knob's ring.
    #[test]
    fn amendment_tokens_meet_their_targets() {
        let mut failures = Vec::new();
        for p in every_palette() {
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, _, _, rim_min) = targets(p);
                // `danger_base` draws error rings and the danger button's focus ring.
                let mut rims = vec![
                    ("pane", s.pane),
                    ("panel", s.panel),
                    ("elevated", s.elevated),
                    ("overlay", s.overlay),
                    ("field", s.field),
                ];
                rims.extend([
                    ("raised.top", s.raised.top),
                    ("raised.bottom", s.raised.bottom),
                ]);
                for (on, bg) in rims {
                    gate.need("danger_base", p.danger_base, on, bg, rim_min);
                }
                for (on, bg) in [
                    ("overlay", s.overlay),
                    ("elevated", s.elevated),
                    ("pane", s.pane),
                    ("panel", s.panel),
                ] {
                    for (what, fill) in [
                        ("danger highlight", p.danger_highlight()),
                        ("danger tint", p.danger_tint()),
                    ] {
                        let b = over(fill, bg);
                        gate.need("danger", p.danger, &format!("{what} over {on}"), b, 4.5);
                        gate.need("text", p.text, &format!("{what} over {on}"), b, text_min);
                    }
                }
                // Text and controls sit on the deck and the header strip as on any chrome.
                let deck = [
                    ("deck.top".to_owned(), s.deck.top),
                    ("deck.bottom".to_owned(), s.deck.bottom),
                    ("pane_header".to_owned(), s.pane_header),
                ];
                let (_, dim_min, faint_min, _) = targets(p);
                for (on, bg) in with_hovers(p, &deck) {
                    gate.need("text", p.text, &on, bg, text_min);
                    gate.need("text_dim", p.text_dim, &on, bg, dim_min);
                    gate.need("text_faint", p.text_faint, &on, bg, faint_min);
                    gate.need("accent", p.accent, &on, bg, 4.5);
                    gate.need("focus", p.focus, &on, bg, rim_min);
                    gate.need("control_border", p.control_border, &on, bg, rim_min);
                }
                match s.knob_edge {
                    Some(ring) => {
                        for c in s.knob.ends() {
                            gate.need("knob_edge", ring, "knob", c, 3.0);
                        }
                    }
                    None => assert!(p.dark, "{}: light themes ring the switch knob", p.name),
                }
                failures.extend(gate.failures);
            }
            // Owner: Deeper's table header seam is not one more neutral pane edge, and Standard
            // keeps concept 1's flat two-tone chrome.
            let d = &p.deeper;
            if !p.strict {
                assert_ne!(Some(d.pane_header_seam), d.pane_edge, "{}", p.name);
                assert_ne!(d.pane_header_seam, d.border, "{}", p.name);
                assert_ne!(d.pane_header, d.pane, "{}", p.name);
                assert!(
                    !d.deck.is_flat(),
                    "{}: the Deeper deck is a gradient",
                    p.name
                );
            }
            let st = &p.standard;
            assert_eq!(st.deck, Fill::flat(st.panel), "{}", p.name);
            assert_eq!(st.pane_header_seam, st.border, "{}", p.name);
            assert_eq!(st.pane_header, st.pane, "{}", p.name);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The fields of a request each have a colour of their own: apart from each other and from the
    /// neutral text and the accent, in every theme. High contrast pulls every colour toward the
    /// ends of the lightness scale, so there the hue alone has to carry the difference. A custom
    /// theme lets the person choose the accent and tint every neutral with a hue of their choosing,
    /// so there the fields only have to differ from each other.
    #[test]
    fn every_theme_keeps_the_request_fields_apart() {
        let mut failures = Vec::new();
        for p in every_palette() {
            let f = p.fields;
            let fields = [
                ("number", f.number),
                ("author", f.author),
                ("time", f.time),
                ("label", f.label),
                ("comments", f.comments),
            ];
            let others = [
                ("text", p.text),
                ("text_dim", p.text_dim),
                ("text_faint", p.text_faint),
                ("accent", p.accent),
            ];
            let (among, beside) = if p.strict { (3.0, 2.5) } else { (7.0, 6.0) };
            let others: &[(&str, Color32)] = if p.name == "Custom" { &[] } else { &others };
            for (i, (a, ca)) in fields.iter().enumerate() {
                for (b, cb) in &fields[i + 1..] {
                    let d = oklab_distance(*ca, *cb);
                    if d < among {
                        failures.push(format!("{}: {a} and {b} are {d:.1} apart", p.name));
                    }
                }
                for (b, cb) in others {
                    let d = oklab_distance(*ca, *cb);
                    if d < beside {
                        failures.push(format!("{}: {a} and {b} are {d:.1} apart", p.name));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Pills, avatars and badges keep their hue and still read at 4.5:1 wherever they are drawn,
    /// in both styles.
    #[test]
    fn coloured_components_are_readable() {
        let mut failures = Vec::new();
        for p in every_palette() {
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut colours: Vec<Color32> = p.lanes.to_vec();
                colours.extend([
                    p.accent, p.accent2, p.success, p.warning, p.danger, p.text_dim,
                ]);
                colours.extend([p.fields.label, p.fields.comments]);
                for c in colours {
                    let mut fills = vec![
                        s.badge,
                        Palette::avatar_fill(c),
                        Palette::badge_fill(c),
                        Palette::change_fill(c),
                    ];
                    if c == p.danger {
                        fills.extend([Palette::tint(c, 56), p.danger_badge_fill()]);
                    }
                    for fill in fills {
                        let ink = p.ink_on(s, c, fill);
                        for bg in p.component_surfaces(s) {
                            let r = contrast(ink, over(fill, bg));
                            if r < 4.5 {
                                failures.push(format!(
                                    "{}/{style:?}: {c:?} on {fill:?} is {r:.2}:1",
                                    p.name
                                ));
                            }
                        }
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// What can sit behind a glass layer: the extremes and the primaries, each clamped by
    /// `clamp_backdrop` as the blur is.
    const GLASS_PROBES: [(&str, Color32); 9] = [
        ("white", Color32::WHITE),
        ("black", Color32::BLACK),
        ("red", Color32::from_rgb(255, 0, 0)),
        ("green", Color32::from_rgb(0, 255, 0)),
        ("blue", Color32::from_rgb(0, 0, 255)),
        ("yellow", Color32::from_rgb(255, 255, 0)),
        ("cyan", Color32::from_rgb(0, 255, 255)),
        ("magenta", Color32::from_rgb(255, 0, 255)),
        ("grey", Color32::from_gray(128)),
    ];

    /// Translucent surfaces keep the contrast targets over any content: the overlay at its glass
    /// opacity over nine worst-case backdrops after the shader's luminance clamp.
    #[test]
    fn glass_keeps_contrast_over_any_backdrop() {
        let probes = GLASS_PROBES;
        let mut failures = Vec::new();
        for p in every_palette() {
            if p.glass_alpha == 0 {
                assert!(p.strict, "{}: only high contrast never uses glass", p.name);
                continue;
            }
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, dim_min, faint_min, _) = targets(p);
                for (name, probe) in probes {
                    let backdrop = clamp_backdrop(probe, p.dark, p.glass_luminance);
                    let glass = over(Palette::tint(s.overlay, p.glass_alpha), backdrop);
                    for (on, bg) in [
                        (format!("glass over {name}"), glass),
                        (
                            format!("hover_strong on glass over {name}"),
                            over(p.hover_strong(), glass),
                        ),
                    ] {
                        gate.need("text", p.text, &on, bg, text_min);
                        gate.need("text_dim", p.text_dim, &on, bg, dim_min);
                        gate.need("text_faint", p.text_faint, &on, bg, faint_min);
                        gate.need("accent", p.accent, &on, bg, 4.5);
                        gate.need("danger", p.danger, &on, bg, 4.5);
                        gate.need("control_border", p.control_border, &on, bg, 3.0);
                        gate.need("focus", p.focus, &on, bg, 3.0);
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// #281: an unavailable option in an open list names a real program ("Not installed"), so its
    /// `disabled_text` reads at 4.5:1 on the list's surface in every built-in theme and style: the
    /// opaque `overlay`, and the glass fill over the app's own pane or anything else the blur
    /// can show behind it.
    #[test]
    fn unavailable_options_read_in_an_open_list() {
        let mut failures = Vec::new();
        for p in ThemeId::BUILT_IN
            .iter()
            .flat_map(|t| [t.palette(), t.shown_palette(true)])
        {
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut grounds = vec![
                    ("overlay".to_owned(), s.overlay),
                    ("glass over pane".to_owned(), over(p.glass(s), s.pane)),
                ];
                if p.glass_alpha > 0 {
                    for (name, probe) in GLASS_PROBES {
                        let backdrop = clamp_backdrop(probe, p.dark, p.glass_luminance);
                        grounds.push((format!("glass over {name}"), over(p.glass(s), backdrop)));
                    }
                }
                for (name, bg) in grounds {
                    let ratio = contrast(p.disabled_text, bg);
                    if ratio < 4.5 {
                        failures.push(format!("{} {style:?} on {name}: {ratio:.2}", p.name));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// A muted badge (the select's "Not installed" tag) reads in `text_dim` on the neutral badge
    /// fill: 4.5:1 (7:1 at high contrast), in every style.
    #[test]
    fn muted_badge_text_reads_on_the_badge_fill() {
        let mut failures = Vec::new();
        for p in every_palette() {
            let min = targets(p).1;
            for style in Style::ALL {
                let s = p.surfaces(style);
                let ratio = contrast(over(p.text_dim, s.badge), s.badge);
                if ratio < min {
                    failures.push(format!("{} {style:?}: {ratio:.2} < {min}", p.name));
                }
            }
        }
        assert!(failures.is_empty(), "{failures:#?}");
    }

    /// A disabled control stays readable (QA wave 1: labels at 1.3 to 2.3:1): `disabled_text` at
    /// 3:1 (4.5:1 at high contrast) on every surface a disabled control is painted on, dimmer than
    /// `text_faint` so it still reads as disabled, on a neutral fill with a ring of its own.
    #[test]
    fn disabled_tokens_are_readable() {
        let mut failures = Vec::new();
        for p in every_palette() {
            let min = if p.strict { 4.5 } else { 3.0 };
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let mut bgs = vec![
                    ("disabled_fill".to_owned(), s.disabled_fill),
                    ("pane".to_owned(), s.pane),
                    ("panel".to_owned(), s.panel),
                    ("elevated".to_owned(), s.elevated),
                    ("overlay".to_owned(), s.overlay),
                    ("field".to_owned(), s.field),
                ];
                for (name, fill) in [
                    ("raised", s.raised),
                    ("raised_hover", s.raised_hover),
                    ("raised_pressed", s.raised_pressed),
                ] {
                    for (end, c) in ["top", "bottom"].into_iter().zip(fill.ends()) {
                        bgs.push((format!("{name}.{end}"), c));
                    }
                }
                for (on, bg) in with_hovers(p, &bgs[..6]) {
                    gate.need("disabled_text", p.disabled_text, &on, bg, min);
                }
                for (on, bg) in &bgs[6..] {
                    gate.need("disabled_text", p.disabled_text, on, *bg, min);
                }
                failures.extend(gate.failures);
                assert!(
                    contrast(p.disabled_text, s.pane) < contrast(p.text_faint, s.pane),
                    "{}/{style:?}: disabled_text reads dimmer than text_faint",
                    p.name
                );
                assert_ne!(s.disabled_edge, s.disabled_fill, "{}", p.name);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// CIELAB (D65) of an opaque colour.
    fn lab(c: Color32) -> [f64; 3] {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (lin(c.r()), lin(c.g()), lin(c.b()));
        let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.950_47;
        let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.088_83;
        let f = |t: f64| {
            if t > 216.0 / 24389.0 {
                t.cbrt()
            } else {
                (24389.0 / 27.0 * t + 16.0) / 116.0
            }
        };
        let (fx, fy, fz) = (f(x), f(y), f(z));
        [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
    }

    /// CIE76 colour difference.
    fn delta_e(a: Color32, b: Color32) -> f64 {
        let (a, b) = (lab(a), lab(b));
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    }

    /// Two branches never draw in look-alike colours: every pair of a theme's lanes at least
    /// dE 25 apart (QA wave 1 found pairs down to dE 6).
    #[test]
    fn graph_lanes_are_distinct() {
        let mut failures = Vec::new();
        for p in every_palette() {
            for (i, a) in p.lanes.iter().enumerate() {
                for (j, b) in p.lanes.iter().enumerate().skip(i + 1) {
                    let d = delta_e(*a, *b);
                    if d < 25.0 {
                        failures.push(format!("{}: lanes {i} and {j} are dE {d:.1}", p.name));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The status washes the kit paints (banners, callouts, discs) keep text and their own status
    /// colour readable, whatever tone a theme gives them.
    #[test]
    fn status_washes_keep_text_readable() {
        let mut failures = Vec::new();
        for p in every_palette() {
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, ..) = targets(p);
                for (name, wash, fg) in [
                    ("accent", p.accent_tint(), p.accent),
                    ("success", p.success_tint(), p.success),
                    ("warning", p.warning_tint(), p.warning),
                    ("danger", p.danger_tint(), p.danger),
                ] {
                    for (on, bg) in grounds(s, Style::Standard) {
                        let b = over(wash, bg);
                        gate.need(
                            "text",
                            p.text,
                            &format!("{name} wash over {on}"),
                            b,
                            text_min,
                        );
                        gate.need(name, fg, &format!("its wash over {on}"), b, 4.5);
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// High contrast serves low vision, so there a link carries a cue beyond colour: it is
    /// underlined at rest (QV2-3, WCAG 1.4.1). Elsewhere the underline appears on hover.
    #[test]
    fn links_are_underlined_at_rest_at_high_contrast() {
        use crate::kit::button::LinkButton;
        for (id, hc) in ThemeId::ALL
            .into_iter()
            .flat_map(|t| [(t, false), (t, true)])
        {
            let ctx = egui::Context::default();
            crate::fonts::install(&ctx);
            set_high_contrast(hc);
            apply(&ctx, id, Appearance::STILL);
            let mut text = egui::Rect::NOTHING;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                text = LinkButton::new("Create a token").show(ui).rect;
            });
            out.textures_delta.clear();
            let p = id.shown_palette(hc);
            let underlined = out.shapes.iter().any(|c| match &c.shape {
                egui::Shape::LineSegment { points, stroke } => {
                    stroke.color == p.accent2
                        && (points[0].y - points[1].y).abs() < 0.01
                        && points[0].y > text.center().y
                        && points[0].y <= text.bottom() + 2.0
                }
                _ => false,
            });
            assert_eq!(underlined, hc, "{id:?} high contrast {hc}");
            assert_eq!(id.shown_design(hc).link_underline, hc, "{id:?}");
        }
    }

    /// A banner's or callout's action can be disabled ("Continue" while conflicts remain): disabled
    /// text keeps the 3:1 floor on every status wash over every ground, and a clear margin, 3.5:1,
    /// on the grounds banners sit on (pane, panel, Deeper's window ground), so it still reads as
    /// the step that will come (QV2-9: Ember was 3.16:1; washes over dialogs were down to 2.71:1).
    /// High contrast keeps 4.5:1 everywhere.
    #[test]
    fn disabled_actions_read_on_banners() {
        let mut failures = Vec::new();
        for p in every_palette() {
            let floor = if p.strict { 4.5 } else { 3.0 };
            let banner = if p.strict { 4.5 } else { 3.5 };
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                for (name, wash) in [
                    ("accent", p.accent_tint()),
                    ("success", p.success_tint()),
                    ("warning", p.warning_tint()),
                    ("danger", p.danger_tint()),
                ] {
                    for (on, bg) in grounds(s, style) {
                        let min = if matches!(on.as_str(), "pane" | "panel" | "deeper ground") {
                            banner
                        } else {
                            floor
                        };
                        gate.need(
                            "disabled_text",
                            p.disabled_text,
                            &format!("{name} wash over {on}"),
                            over(wash, bg),
                            min,
                        );
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Structure you can see (QA wave 1: every ladder was one uniform 1 to 2 L* step): Standard's
    /// chrome sits at least 2.5 L* off the content ground, and Deeper's cards at least 4 L* off
    /// their ground (high contrast outlines its cards instead).
    #[test]
    fn solid_scroll_bars_stay_thin_and_quiet() {
        let solid = solid_scroll();
        let floating = floating_scroll();
        assert!(solid.floating, "a solid bar is the thin floating handle");
        assert_eq!(solid.floating_width, floating.floating_width);
        assert!(
            solid.floating_allocated_width > 0.0,
            "the bar keeps its room beside the content"
        );
        assert!(
            solid.dormant_handle_opacity > 0.0 && solid.dormant_handle_opacity < 1.0,
            "the handle is always visible but never a solid slab"
        );
        assert_eq!(solid.dormant_handle_opacity, solid.active_handle_opacity);
        assert!(solid.interact_handle_opacity > solid.active_handle_opacity);
    }

    #[test]
    fn surface_steps_are_visible() {
        for p in every_palette() {
            let l = |c: Color32| lab(c)[0];
            let st = &p.standard;
            let chrome = (l(st.panel) - l(st.pane)).abs();
            assert!(chrome >= 2.5, "{}: chrome step {chrome:.1} L*", p.name);
            if !p.strict {
                let d = &p.deeper;
                let card = (l(d.pane) - l(d.bg)).abs();
                assert!(card >= 4.0, "{}: Deeper card step {card:.1} L*", p.name);
            }
        }
    }

    #[test]
    fn backdrop_clamp_caps_dark_and_lifts_light() {
        let lum = |c: Color32| luminance(c);
        let dark = clamp_backdrop(Color32::WHITE, true, 0.03);
        assert!(lum(dark) <= 0.031, "{dark:?}");
        assert_eq!(clamp_backdrop(Color32::BLACK, true, 0.03), Color32::BLACK);
        let light = clamp_backdrop(Color32::BLACK, false, 0.80);
        assert!(lum(light) >= 0.79, "{light:?}");
        assert_eq!(clamp_backdrop(Color32::WHITE, false, 0.80), Color32::WHITE);
    }

    /// OKLCH hue of an opaque colour, in degrees.
    fn oklch_hue(c: Color32) -> f64 {
        let lin = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.040_45 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let (r, g, b) = (lin(c.r()), lin(c.g()), lin(c.b()));
        let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
        let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
        let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
        let a = 1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s;
        let bb = 0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s;
        bb.atan2(a).to_degrees().rem_euclid(360.0)
    }

    /// The accent never reads as a status: at least 40 degrees of OKLCH hue from success and
    /// danger in every built-in theme. A custom theme's accent is the user's own choice.
    #[test]
    fn accent_clears_status_hues() {
        let dist = |a: f64, b: f64| {
            let d = (a - b).abs() % 360.0;
            d.min(360.0 - d)
        };
        for p in every_palette().into_iter().filter(|p| p.name != "Custom") {
            let accent = oklch_hue(p.accent);
            for (what, c) in [("success", p.success), ("danger", p.danger)] {
                let d = dist(accent, oklch_hue(c));
                assert!(d >= 40.0, "{}: accent is {d:.0} deg from {what}", p.name);
            }
        }
    }

    /// Standard is concept 1: flat fills, flush panes, the content ground is the window ground.
    #[test]
    fn standard_is_flat_and_flush() {
        for p in every_palette() {
            let s = &p.standard;
            for (name, fill) in [
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
                ("deck", s.deck),
            ] {
                assert!(fill.is_flat(), "{}: Standard {name} is flat", p.name);
            }
            assert_eq!(s.pane, s.bg, "{}", p.name);
            assert_eq!(s.pane_edge, None, "{}", p.name);
            assert!(p.deeper.pane_edge.is_some(), "{}", p.name);
            for level in Elevation::ALL {
                assert_eq!(
                    material(Style::Standard, p.dark, level).recess,
                    0,
                    "Standard has no field recess"
                );
            }
        }
    }

    #[test]
    fn high_contrast_has_no_highlights() {
        for p in ThemeId::ALL.map(|t| t.shown_palette(true)) {
            for style in Style::ALL {
                for level in Elevation::ALL {
                    assert_eq!(p.material(style, level).highlight, 0);
                }
                assert_eq!(p.floating_ring(p.surfaces(style)), p.control_border);
            }
        }
    }

    #[test]
    fn interact_size_follows_the_metrics() {
        let ctx = egui::Context::default();
        for density in Density::ALL {
            for style in Style::ALL {
                let look = Appearance {
                    style,
                    density,
                    glass: false,
                    reduce_motion: true,
                };
                apply(&ctx, ThemeId::Midnight, look);
                let m = metrics_for(density, style);
                let s = ctx.style_of(Theme::Dark);
                assert_eq!(
                    s.spacing.interact_size.y, m.control,
                    "{density:?} {style:?}"
                );
                assert_eq!(metrics(), m);
                assert_eq!(appearance(), look);
            }
        }
    }

    #[test]
    fn metrics_match_the_concepts() {
        assert_eq!(COMFORTABLE.control, 28.0);
        assert_eq!(COMFORTABLE.history_row_height, 28.0);
        assert_eq!(COMFORTABLE.radius, 7);
        assert_eq!(DENSE.control, 24.0);
        assert_eq!(DENSE.history_row_height, 24.0);
        assert_eq!(DENSE.radius_lg, 6);
        const { assert!(DENSE.zebra && !COMFORTABLE.zebra) };
        assert_eq!(DEEPER_COMFORTABLE.control, 30.0);
        assert_eq!(DEEPER_COMFORTABLE.radius, 10);
        assert_eq!(DEEPER_COMFORTABLE.pane_gutter, 6.0);
        assert_eq!(DEEPER_DENSE.control, 26.0);
        // Style overrides geometry only; type and rows come from the density.
        assert_eq!(DEEPER_DENSE.body_size, DENSE.body_size);
        assert_eq!(DEEPER_DENSE.history_row_height, DENSE.history_row_height);
        assert_eq!(COMFORTABLE.pane_gutter, 0.0);
        assert_eq!(DENSE.pane_radius, 0);
    }

    #[test]
    fn theme_names_accept_display_and_variant_names() {
        assert_eq!(ThemeId::from_name("midnight"), Some(ThemeId::Midnight));
        assert_eq!(
            ThemeId::from_name("OBSERVATORY"),
            Some(ThemeId::Observatory)
        );
        assert_eq!(ThemeId::from_name("nope"), None);
        assert_eq!(Style::from_name("deeper"), Some(Style::Deeper));
        assert_eq!(Density::from_name("Dense"), Some(Density::Dense));
    }

    #[test]
    fn glass_needs_theme_backend_and_system_consent() {
        let on = |theme, available, system| {
            Appearance::resolve(
                theme,
                Style::Standard,
                Density::Comfortable,
                true,
                false,
                available,
                system,
            )
            .glass
        };
        assert!(on(ThemeId::Midnight, true, None));
        assert!(on(ThemeId::Midnight, true, Some(false)));
        assert!(!on(ThemeId::Midnight, true, Some(true)));
        assert!(!on(ThemeId::Midnight, false, None));
        let off = Appearance::resolve(
            ThemeId::Midnight,
            Style::Standard,
            Density::Comfortable,
            false,
            false,
            true,
            None,
        );
        assert!(!off.glass);
        set_high_contrast(true);
        assert!(!on(ThemeId::Midnight, true, None));
        assert_eq!(
            glass_blocked(ThemeId::Midnight, true, None),
            Some(GlassBlocked::Theme)
        );
        set_high_contrast(false);
    }

    /// Owner (6 Oct 2026): three themes, Midnight and Daylight (concept 1) and Observatory (the
    /// Midnight that build 690 shipped, kept as a theme of its own), plus a custom theme the user
    /// colours, starting from Midnight; high contrast is a switch over any of them.
    #[test]
    fn the_themes_are_midnight_daylight_observatory_and_custom() {
        assert_eq!(
            ThemeId::BUILT_IN,
            [ThemeId::Midnight, ThemeId::Daylight, ThemeId::Observatory]
        );
        assert_eq!(
            ThemeId::ALL,
            [
                ThemeId::Midnight,
                ThemeId::Daylight,
                ThemeId::Observatory,
                ThemeId::Custom
            ]
        );
        assert_eq!(ThemeId::default(), ThemeId::Midnight);
        let mut seen = vec![ThemeId::Midnight];
        while seen.len() < ThemeId::ALL.len() {
            let next = seen[seen.len() - 1].next();
            assert!(!seen.contains(&next), "{next:?} came round early");
            seen.push(next);
        }
        assert_eq!(seen[seen.len() - 1].next(), ThemeId::Midnight);
    }

    /// Observatory keeps build 690's Midnight: its navy ground, periwinkle primary and indigo
    /// focus, and its structure (the selection bar and the zenith lift).
    #[test]
    fn observatory_is_the_released_midnight() {
        let s = OBSERVATORY.standard;
        assert_eq!(
            [s.bg, s.panel, s.border, s.primary.top],
            [hex(0x0B0F19), hex(0x111723), hex(0x1B2233), hex(0x646CD6)]
        );
        assert_eq!(OBSERVATORY.focus, hex(0x7F89FF));
        assert_eq!(
            OBSERVATORY_DESIGN.selection,
            SelectionStyle::Bar { width: 2.0 }
        );
        assert_eq!(OBSERVATORY_DESIGN.art, ArtMotif::Constellation);
    }

    /// Owner (6 Oct 2026): high contrast applies over the chosen theme. Each theme keeps its
    /// grounds and its hues (a colour's hue moves by little), and turns strict.
    #[test]
    fn high_contrast_keeps_each_themes_grounds_and_hues() {
        let hue = |c: Color32| {
            let [r, g, b, _] = c.to_array();
            let (r, g, b) = (f32::from(r), f32::from(g), f32::from(b));
            (3.0_f32.sqrt() * (g - b)).atan2(2.0 * r - g - b)
        };
        for t in ThemeId::ALL {
            let (base, hc) = (t.palette(), t.shown_palette(true));
            assert!(hc.strict && !base.strict, "{t:?}");
            assert_eq!(hc.dark, base.dark);
            for style in Style::ALL {
                let (c, m) = (hc.surfaces(style), base.surfaces(style));
                assert_eq!(
                    [c.bg, c.panel, c.pane, c.elevated, c.field],
                    [m.bg, m.panel, m.pane, m.elevated, m.field],
                    "{t:?} {style:?}"
                );
            }
            for (a, b) in [
                (hc.standard.primary.top, base.standard.primary.top),
                (hc.focus, base.focus),
                (hc.accent, base.accent),
            ] {
                let d = (hue(a) - hue(b)).abs();
                let d = d.min(std::f32::consts::TAU - d);
                assert!(d < 0.35, "{t:?}: {a:?} left {b:?}'s hue");
            }
        }
    }

    /// Owner's lock (5 Oct 2026, round 2): the default look is concept 1 exactly. Midnight carries
    /// its dark palette and Daylight its light one, both with the indigo primary, and both use
    /// concept 1's structure: flat ground, a wash with no bar, a lifted tab and a 2 pt focus ring
    /// 2 pt outside the control.
    #[test]
    fn the_default_themes_are_concept_one() {
        let s = MIDNIGHT.standard;
        assert_eq!(
            [s.bg, s.panel, s.elevated, s.border],
            [hex(0x0F1012), hex(0x151619), hex(0x1B1C20), hex(0x24262B)]
        );
        assert_eq!(
            [MIDNIGHT.text, MIDNIGHT.text_dim, MIDNIGHT.focus],
            [hex(0xECECEF), hex(0xA4A6AD), hex(0x7C86FF)]
        );
        let d = DAYLIGHT.standard;
        assert_eq!(
            [d.bg, d.panel, d.border],
            [hex(0xFFFFFF), hex(0xF7F7F8), hex(0xE6E6EA)]
        );
        assert_eq!(DAYLIGHT.text, hex(0x18181B));
        for p in [&MIDNIGHT, &DAYLIGHT] {
            assert_eq!(p.standard.primary.top, hex(0x5E6AD2), "{}", p.name);
            assert_eq!(p.standard.primary.bottom, hex(0x5E6AD2), "{}", p.name);
            assert_eq!(p.danger_base == hex(0xE5484D), p.dark, "{}", p.name);
        }
        for id in [ThemeId::Midnight, ThemeId::Daylight] {
            let d = id.design();
            assert_eq!(d.ground, Ground::Flat);
            assert_eq!(d.selection, SelectionStyle::Wash);
            assert_eq!(d.active_tab, TabStyle::Lifted);
            assert_eq!(
                d.focus,
                FocusStyle::Ring {
                    width: 2.0,
                    offset: 2.0
                }
            );
            assert!((d.radius_scale - 1.0).abs() < f32::EPSILON);
            assert_eq!(d.shadows, ShadowCharacter::Soft);
            assert_eq!(d.hover, HoverStyle::Tint);
            assert_eq!(d.heading, HeadingFace::Ui);
            assert_eq!(d.art, ArtMotif::Plain);
            assert_eq!(d.glass_tint, Color32::TRANSPARENT);
        }
    }

    #[test]
    fn open_menu_items_keep_the_same_surface_as_hovered_items() {
        for id in ThemeId::ALL {
            for style in Style::ALL {
                let v = visuals(
                    id.palette(),
                    style,
                    metrics_for(Density::Comfortable, style),
                );
                assert_eq!(
                    v.widgets.open.weak_bg_fill,
                    v.widgets.hovered.weak_bg_fill,
                    "{} submenu must keep its hover fill when the pointer enters a child",
                    id.palette().name
                );
                assert_eq!(
                    v.widgets.open.fg_stroke.color,
                    v.widgets.hovered.fg_stroke.color
                );
            }
        }
    }

    /// Owner (6 Oct 2026): each theme is a design of its own, not a recolour. Every pair of themes
    /// differs in at least four structural choices, and no two share an art motif. The Midnight
    /// family is the exception: Midnight and Daylight are the locked concept 1 in dark and light,
    /// and Observatory is build 690's Midnight, kept as it shipped (its own palette, ground,
    /// selection bar and motif).
    #[test]
    fn every_theme_is_a_design_of_its_own() {
        use std::mem::discriminant as kind;
        let traits = |d: &Design| {
            [
                format!("{:.1}", (d.radius_scale * 4.0).round() / 4.0),
                format!("{:?}", d.edges),
                format!("{:?}", d.shadows),
                format!("{:?}", d.panes),
                format!("{:?}", kind(&d.ground)),
                format!("{:?}", kind(&d.selection)),
                format!("{:?}", kind(&d.focus)),
                format!("{:?}", d.hover),
                format!("{:?}", kind(&d.active_tab)),
                format!("{:.2}", d.labels.tracking),
                format!("{:?}", d.heading),
                format!("{:.2}{:.2}", d.graph.node_scale, d.graph.line_scale),
                format!("{:?}", d.icons),
                format!("{:?}", d.motion),
            ]
        };
        let pair = [ThemeId::Midnight, ThemeId::Daylight];
        let family = [ThemeId::Midnight, ThemeId::Daylight, ThemeId::Observatory];
        assert_eq!(
            traits(ThemeId::Midnight.design()),
            traits(ThemeId::Daylight.design())
        );
        let mut motifs = std::collections::HashSet::new();
        for (i, a) in ThemeId::BUILT_IN.iter().enumerate() {
            if *a != ThemeId::Daylight {
                assert!(
                    motifs.insert(a.design().art),
                    "{}: art motif shared",
                    a.name()
                );
            }
            for b in ThemeId::BUILT_IN.iter().skip(i + 1) {
                if pair.contains(a) && pair.contains(b) {
                    continue;
                }
                let (x, y) = (traits(a.design()), traits(b.design()));
                let differ = x.iter().zip(&y).filter(|(p, q)| p != q).count();
                let least = if family.contains(a) && family.contains(b) {
                    2
                } else {
                    4
                };
                assert!(
                    differ >= least,
                    "{} and {} differ in only {differ} structural choices",
                    a.name(),
                    b.name()
                );
            }
        }
    }

    /// A theme's radius scale reshapes every corner but keeps pills full, square corners square and
    /// a rounded corner from collapsing; high contrast's tighter corners stay tighter than
    /// Midnight's.
    #[test]
    fn design_radii_scale_and_keep_their_shape() {
        for id in ThemeId::ALL {
            let d = id.design();
            assert!((0.3..=1.5).contains(&d.radius_scale), "{}", id.name());
            assert_eq!(d.radius(FULL), FULL);
            assert_eq!(d.radius(0), 0);
            let mut last = 0;
            for r in 1..=40 {
                let v = d.radius(r);
                assert!(v >= 1 && v >= last, "{}: radius {r} -> {v}", id.name());
                last = v;
            }
        }
        let r = |id: ThemeId| id.design().radius(COMFORTABLE.radius);
        assert!(HIGH_CONTRAST_DESIGN.radius(COMFORTABLE.radius) < r(ThemeId::Midnight));
    }

    /// Lines, rings and markers the record sizes stay visible and never heavy.
    #[test]
    fn design_strokes_are_sized_sensibly() {
        for id in ThemeId::ALL {
            let d = id.design();
            let mut widths = vec![("edge", d.edge_width)];
            match d.selection {
                SelectionStyle::Bar { width } | SelectionStyle::Outline { width } => {
                    widths.push(("selection", width));
                }
                SelectionStyle::Wash => {}
            }
            match d.focus {
                FocusStyle::Ring { width, offset } => {
                    widths.push(("focus", width));
                    assert!((0.0..=2.0).contains(&offset), "{}", id.name());
                }
                FocusStyle::Double { width } => {
                    widths.push(("focus", width));
                }
            }
            match d.active_tab {
                TabStyle::Underline { width } => {
                    widths.push(("tab", width));
                }
                TabStyle::Lifted => {}
            }
            for (what, w) in widths {
                assert!((1.0..=6.0).contains(&w), "{}: {what} width {w}", id.name());
            }
            let g = d.graph;
            assert!((0.8..=1.4).contains(&g.node_scale), "{}", id.name());
            assert!((0.8..=1.4).contains(&g.line_scale), "{}", id.name());
            assert!((0.0..=0.2).contains(&d.labels.tracking), "{}", id.name());
            assert!((0.7..=1.3).contains(&d.motion.duration_scale()));
            assert!((1.0..=1.1).contains(&d.heading.size_scale()));
        }
        // High contrast keeps the strongest structure: outlines, a double ring, no shadows.
        let c = &HIGH_CONTRAST_DESIGN;
        assert_eq!(c.edges, EdgeMode::Outline);
        assert!(matches!(c.focus, FocusStyle::Double { .. }));
        assert_eq!(c.shadows, ShadowCharacter::Flat);
        assert_eq!(c.ground, Ground::Flat);
    }

    /// Ground treatments are washes laid over the ground: text, links, the focus ring and input
    /// outlines keep their targets on the strongest point of the wash, on both styles' grounds.
    #[test]
    fn ground_treatments_keep_text_readable() {
        let mut failures = Vec::new();
        for p in every_palette() {
            let wash = match p.design().ground {
                Ground::Flat => continue,
                Ground::Vertical { ink, top, bottom } => Palette::tint(ink, top.max(bottom)),
            };
            for style in Style::ALL {
                let s = p.surfaces(style);
                let mut gate = Gate::new(p, style);
                let (text_min, dim_min, faint_min, rim_min) = targets(p);
                for (on, g) in [("ground", s.bg), ("pane", s.pane)] {
                    let b = over(wash, g);
                    let on = format!("ground treatment over {on}");
                    gate.need("text", p.text, &on, b, text_min);
                    gate.need("text_dim", p.text_dim, &on, b, dim_min);
                    gate.need("text_faint", p.text_faint, &on, b, faint_min);
                    gate.need("accent", p.accent, &on, b, 4.5);
                    gate.need("focus", p.focus, &on, b, rim_min);
                    gate.need("control_border", p.control_border, &on, b, rim_min);
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// The theme's glass tint keeps every pairing `glass_keeps_contrast_over_any_backdrop` checks;
    /// High contrast never tints (it never uses glass).
    #[test]
    fn glass_tint_keeps_contrast_over_any_backdrop() {
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
        ];
        let mut failures = Vec::new();
        for p in every_palette() {
            let tint = p.design().glass_tint;
            if p.glass_alpha == 0 {
                assert_eq!(tint, Color32::TRANSPARENT, "{}", p.name);
                continue;
            }
            assert!(
                tint.a() <= 40,
                "{}: the tint leans, it does not repaint",
                p.name
            );
            for style in Style::ALL {
                let s = p.surfaces(style);
                let fill = over(tint, s.overlay);
                let mut gate = Gate::new(p, style);
                let (text_min, dim_min, faint_min, _) = targets(p);
                for probe in probes {
                    let backdrop = clamp_backdrop(probe, p.dark, p.glass_luminance);
                    let glass = over(Palette::tint(fill, p.glass_alpha), backdrop);
                    for bg in [glass, over(p.hover_strong(), glass)] {
                        gate.need("text", p.text, "tinted glass", bg, text_min);
                        gate.need("text_dim", p.text_dim, "tinted glass", bg, dim_min);
                        gate.need("text_faint", p.text_faint, "tinted glass", bg, faint_min);
                        gate.need("accent", p.accent, "tinted glass", bg, 4.5);
                        gate.need("danger", p.danger, "tinted glass", bg, 4.5);
                        gate.need("control_border", p.control_border, "tinted glass", bg, 3.0);
                        gate.need("focus", p.focus, "tinted glass", bg, 3.0);
                    }
                }
                failures.extend(gate.failures);
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    /// Shadow characters: Soft keeps the tables, Flat keeps only a faint floating lift.
    #[test]
    fn shadow_characters_reshape_the_layers() {
        use ShadowCharacter as C;
        let drop = ShadowLayer {
            dy: 10,
            blur: 28,
            spread: -8,
            alpha: 166,
            tint: ShadowTint::Shadow,
        };
        assert_eq!(C::Soft.shape(Elevation::Pane, drop), Some(drop));
        assert_eq!(C::Flat.shape(Elevation::Control, drop), None);
        assert_eq!(C::Flat.shape(Elevation::Pane, drop), None);
        let lift = C::Flat.shape(Elevation::Floating, drop).unwrap();
        assert!(lift.alpha < drop.alpha && lift.blur < drop.blur);
    }

    /// Every material a theme hands the kit is already in its shadow character: high contrast's
    /// controls cast nothing, and the soft themes hand over the material tables as they are.
    #[test]
    fn materials_come_in_the_themes_shadow_character() {
        for style in [Style::Standard, Style::Deeper] {
            for level in Elevation::ALL {
                for t in ThemeId::ALL {
                    let contrast = t.shown_palette(true).material(style, level);
                    if level != Elevation::Floating {
                        assert!(contrast.layers.is_empty(), "{t:?} {style:?} {level:?}");
                    }
                }
                assert_eq!(
                    ThemeId::Daylight.palette().material(style, level).layers,
                    material(style, false, level).layers
                );
                assert_eq!(
                    ThemeId::Midnight.palette().material(style, level).layers,
                    material(style, true, level).layers
                );
            }
        }
    }

    /// Heading faces resolve to the families `ui/fonts.rs` registers, and every theme's design is
    /// the one `apply` installs.
    #[test]
    fn type_tokens_resolve() {
        assert_eq!(HeadingFace::Ui.family(), semibold());
        assert_ne!(HeadingFace::Ui.family(), HeadingFace::Hyperlegible.family());
        let ctx = egui::Context::default();
        for id in ThemeId::ALL {
            apply(&ctx, id, Appearance::STILL);
            assert_eq!(design(&ctx), id.design());
        }
        assert_ne!(
            IconWeight::Regular,
            HIGH_CONTRAST_DESIGN.icons,
            "high contrast's active icons are filled"
        );
    }

    /// The active metrics carry the active theme's corners: every radius through its scale,
    /// everything else (sizes, gutters, type) as the density and style set it.
    #[test]
    fn metrics_come_in_the_themes_corners() {
        for id in ThemeId::ALL {
            for density in Density::ALL {
                for style in [Style::Standard, Style::Deeper] {
                    let raw = metrics_for(density, style);
                    let m = metrics_in(id, density, style);
                    let d = id.design();
                    assert_eq!(m.radius, d.radius(raw.radius), "{id:?}");
                    assert_eq!(m.radius_sm, d.radius(raw.radius_sm));
                    assert_eq!(m.pane_radius, d.radius(raw.pane_radius));
                    assert_eq!(m.keycap_radius, d.radius(raw.keycap_radius));
                    assert_eq!(m.pill_radius, d.radius(raw.pill_radius));
                    assert_eq!(
                        (m.control, m.pane_gutter, m.body_size),
                        (raw.control, raw.pane_gutter, raw.body_size)
                    );
                }
            }
        }
        set_high_contrast(true);
        look_set(
            ThemeId::Daylight,
            Style::Deeper,
            Density::Comfortable,
            false,
        );
        assert_eq!(
            metrics().radius,
            HIGH_CONTRAST_DESIGN.radius(DEEPER_COMFORTABLE.radius)
        );
        assert!(
            metrics().radius < DEEPER_COMFORTABLE.radius,
            "high contrast is sharper"
        );
        set_high_contrast(false);
    }

    /// Section labels are capitals at `micro_size` in the theme's tracking; titles take its heading
    /// face; active icons its icon weight.
    #[test]
    fn labels_titles_and_icons_follow_the_theme() {
        let job_of = |id: ThemeId| {
            look_set(id, Style::Standard, Density::Comfortable, false);
            section_label("Branches", Color32::WHITE)
        };
        set_high_contrast(true);
        let contrast = job_of(ThemeId::Midnight);
        set_high_contrast(false);
        assert_eq!(contrast.text, "BRANCHES");
        let format = &contrast.sections[0].format;
        assert_eq!(
            format.font_id,
            FontId::new(COMFORTABLE.micro_size, semibold())
        );
        assert!((format.extra_letter_spacing - 0.06 * COMFORTABLE.micro_size).abs() < 1e-4);
        look_set(
            ThemeId::Observatory,
            Style::Standard,
            Density::Comfortable,
            false,
        );
        assert_eq!(icon_font(14.0, true), FontId::proportional(14.0));
        set_high_contrast(true);
        look_set(
            ThemeId::Midnight,
            Style::Standard,
            Density::Comfortable,
            false,
        );
        assert_eq!(heading(20.0).family, HeadingFace::Hyperlegible.family());
        assert_ne!(icon_font(14.0, true), FontId::proportional(14.0));
        assert_eq!(icon_font(14.0, false), FontId::proportional(14.0));
        set_high_contrast(false);
        look_set(
            ThemeId::Midnight,
            Style::Standard,
            Density::Comfortable,
            false,
        );
        assert_eq!(heading(20.0), FontId::new(20.0, semibold()));
    }

    /// The glass fill leans to the theme's light; high contrast has none.
    #[test]
    fn glass_leans_to_the_theme() {
        for p in every_palette() {
            let s = p.surfaces(Style::Standard);
            let glass = p.glass(s);
            assert_eq!(glass.a(), p.glass_alpha, "{}", p.name);
            if p.design().glass_tint == Color32::TRANSPARENT {
                assert_eq!(glass, Palette::tint(s.overlay, p.glass_alpha));
            } else {
                assert_ne!(glass, Palette::tint(s.overlay, p.glass_alpha), "{}", p.name);
            }
        }
    }
}
