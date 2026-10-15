//! NecturaLabs' egui component library: design tokens and themes ([`theme`]), fonts ([`fonts`]),
//! motion ([`motion`]), in-code illustrations ([`art`]) and the component kit ([`kit`]).
//!
//! The library knows nothing about the application using it. What it needs from the host (the
//! operating system's accessibility settings, the key that opens a context menu) comes through
//! [`host::Host`], and shortcut hints through [`kit::Shortcut`].

pub mod art;
pub mod contrast;
pub mod custom_theme;
pub mod fonts;
pub mod host;
pub mod kit;
pub mod motion;
pub mod text;
pub mod theme;

// Conversions between the integer and float types egui mixes. Each one is a plain `as` cast,
// named for what it does with a value out of range, so a view never casts by hand.

/// A length in points as egui's `Margin` stores it: whole points in an `i8`, the fraction dropped
/// and anything past ±127 clamped.
#[expect(
    clippy::cast_possible_truncation,
    reason = "egui's Margin is i8; the spacing tokens are far below 127 points, and the cast saturates"
)]
pub fn points_i8(points: f32) -> i8 {
    points as i8
}

/// A count or index of rows, items, columns or steps as an `f32` for layout.
#[expect(
    clippy::cast_precision_loss,
    reason = "every integer below 2^24 is exact in f32, and no list Canopy lays out comes near it"
)]
pub fn count_f32(n: usize) -> f32 {
    n as f32
}

/// A signed pixel size, offset or step index as an `f32`.
#[expect(
    clippy::cast_precision_loss,
    reason = "every integer below 2^24 is exact in f32; pixel sizes and indices stay far below it"
)]
pub fn int_f32(n: i32) -> f32 {
    n as f32
}

/// egui's `f64` clock (or another `f64`) as the `f32` egui draws and animates with.
#[expect(
    clippy::cast_possible_truncation,
    reason = "egui paints in f32; times, fractions and coordinates lose nothing visible"
)]
pub fn to_f32(v: f64) -> f32 {
    v as f32
}

/// A colour channel, alpha or egui `u8` size (corner radius, shadow spread): the fraction
/// dropped, negatives as 0 and anything past 255 as 255.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "saturating at 0 and 255 is the clamp every caller wants"
)]
pub fn saturate_u8(v: f32) -> u8 {
    v as u8
}

/// A row or column index from a float position: the fraction dropped and negatives as 0.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a position above the first row means row 0, and the cast saturates there"
)]
pub fn saturate_usize(v: f32) -> usize {
    v as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_drop_the_fraction_and_saturate() {
        assert_eq!(points_i8(12.9), 12);
        assert_eq!(points_i8(500.0), i8::MAX);
        assert_eq!(saturate_u8(-3.0), 0);
        assert_eq!(saturate_u8(254.9), 254);
        assert_eq!(saturate_u8(300.0), u8::MAX);
        assert_eq!(saturate_usize(-0.5), 0);
        assert_eq!(saturate_usize(3.99), 3);
        assert!((count_f32(7) - 7.0).abs() < f32::EPSILON);
        assert!((int_f32(-4) + 4.0).abs() < f32::EPSILON);
        assert!((to_f32(0.25) - 0.25).abs() < f32::EPSILON);
    }
}
