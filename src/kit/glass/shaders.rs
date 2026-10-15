//! GLSL for the backdrop blur: a full-viewport triangle, the dual-Kawase down and up passes and
//! the composite (saturation, luminance clamp, dither and rounded-rect mask). Plain source text,
//! compiled by `gl.rs`; the header picks the GLSL dialect the context speaks.

/// The `#version` line and precision for an OpenGL `major.minor` context, or `None` when the
/// context is older than desktop OpenGL 3.0 or OpenGL ES 3.0 (no framebuffer blits, no
/// `gl_VertexID`): the blur is then unavailable.
pub(super) fn header(major: u32, minor: u32, embedded: bool) -> Option<&'static str> {
    match (embedded, major, minor) {
        (true, 3.., _) => Some("#version 300 es\nprecision highp float;\n#define GLASS_ES 1\n"),
        // OpenGL 3.0 guarantees GLSL 1.30; 3.1 and later guarantee 1.40, which is also the
        // oldest dialect macOS core profiles accept.
        (false, 3, 0) => Some("#version 130\n"),
        (false, 3.., _) => Some("#version 140\n"),
        _ => None,
    }
}

/// One triangle that covers the whole viewport; no vertex buffer, the corners come from
/// `gl_VertexID`.
pub(super) const VERTEX: &str = "
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

/// Shared by the two blur passes: `tap` reads the source level at a position in its pixels,
/// clamped to the part of the texture this frame filled (textures are sized for the largest
/// region seen, so the rest holds older content).
const TAP: &str = "
#ifdef GLASS_ES
layout(location = 0) out vec4 o_color;
#else
out vec4 o_color;
#endif
uniform sampler2D u_src;
uniform vec2 u_texel;
uniform vec2 u_src_size;
uniform vec2 u_dst_size;
uniform float u_offset;

vec4 tap(vec2 p) {
    return texture(u_src, clamp(p, vec2(0.5), u_src_size - vec2(0.5)) * u_texel);
}
";

/// Dual-Kawase downsample: the centre four times and four diagonal neighbours, from a level into
/// the next one at half its size.
pub(super) fn down() -> String {
    format!(
        "{TAP}{}",
        "
void main() {
    vec2 p = gl_FragCoord.xy * (u_src_size / u_dst_size);
    float d = u_offset;
    vec4 s = tap(p) * 4.0;
    s += tap(p + vec2(-d, -d));
    s += tap(p + vec2(d, d));
    s += tap(p + vec2(d, -d));
    s += tap(p + vec2(-d, d));
    o_color = s / 8.0;
}
"
    )
}

/// Dual-Kawase upsample: four axis neighbours at twice the offset and four diagonal ones at double
/// weight, from a level into the one above it.
pub(super) fn up() -> String {
    format!(
        "{TAP}{}",
        "
void main() {
    vec2 p = gl_FragCoord.xy * (u_src_size / u_dst_size);
    float u = u_offset;
    vec4 s = tap(p + vec2(-2.0 * u, 0.0)) + tap(p + vec2(2.0 * u, 0.0));
    s += tap(p + vec2(0.0, -2.0 * u)) + tap(p + vec2(0.0, 2.0 * u));
    s += 2.0 * (tap(p + vec2(-u, -u)) + tap(p + vec2(u, u)));
    s += 2.0 * (tap(p + vec2(u, -u)) + tap(p + vec2(-u, u)));
    o_color = s / 12.0;
}
"
    )
}

/// The composite, drawn into egui's framebuffer over the layer: the blurred backdrop with its
/// saturation raised and its luminance kept in the band the theme's contrast checks assume, in
/// linear light: in dark themes scaled down to at most the cap where it is brighter (as
/// `theme::clamp_backdrop` does); in light themes lifted towards white as a whole, every channel
/// moved the cap's share of the way to white, so the floor holds and what lies behind keeps its
/// shapes instead of flattening to one tone. Then dithered by half a step, masked to the layer's
/// rounded rectangle with one pixel of anti-aliasing, premultiplied and faded with the layer's
/// opacity.
pub(super) const COMPOSITE: &str = "
#ifdef GLASS_ES
layout(location = 0) out vec4 o_color;
#else
out vec4 o_color;
#endif
uniform sampler2D u_src;
uniform vec2 u_texel;
uniform vec2 u_src_size;
uniform vec4 u_region;
uniform vec4 u_rect;
uniform vec4 u_radii;
uniform vec4 u_look;

vec3 to_linear(vec3 c) {
    vec3 lo = c / 12.92;
    vec3 hi = pow((c + 0.055) / 1.055, vec3(2.4));
    return mix(lo, hi, step(vec3(0.04045), c));
}

vec3 to_srgb(vec3 l) {
    vec3 lo = l * 12.92;
    vec3 hi = 1.055 * pow(l, vec3(1.0 / 2.4)) - 0.055;
    return mix(lo, hi, step(vec3(0.0031308), l));
}

float luma(vec3 l) {
    return dot(l, vec3(0.2126, 0.7152, 0.0722));
}

// Jimenez 2014, interleaved gradient noise.
float noise(vec2 p) {
    return fract(52.9829189 * fract(0.06711056 * p.x + 0.00583715 * p.y));
}

void main() {
    vec2 frag = gl_FragCoord.xy;
    // u_region: the copied region's origin and size in window pixels.
    vec2 n = (frag - u_region.xy) / u_region.zw;
    vec2 px = clamp(n * u_src_size, vec2(0.5), u_src_size - vec2(0.5));
    vec3 lin = to_linear(clamp(texture(u_src, px * u_texel).rgb, 0.0, 1.0));
    // u_look: x = saturation, y = luminance cap, z = 1 in dark themes, w = layer opacity.
    lin = clamp(mix(vec3(luma(lin)), lin, u_look.x), 0.0, 1.0);
    float y = luma(lin);
    if (u_look.z > 0.5) {
        if (y > u_look.y) {
            lin *= u_look.y / y;
        }
    } else {
        lin = 1.0 - (1.0 - lin) * (1.0 - u_look.y);
    }
    vec3 g = clamp(to_srgb(clamp(lin, 0.0, 1.0)) + (noise(frag) - 0.5) / 255.0, 0.0, 1.0);
    // u_rect: centre and half size in window pixels; u_radii: corner radii, top-left, top-right,
    // bottom-left, bottom-right (window y points up).
    vec2 p = frag - u_rect.xy;
    float r = p.y > 0.0 ? (p.x < 0.0 ? u_radii.x : u_radii.y) : (p.x < 0.0 ? u_radii.z : u_radii.w);
    vec2 q = abs(p) - u_rect.zw + vec2(r);
    float dist = length(max(q, vec2(0.0))) + min(max(q.x, q.y), 0.0) - r;
    float cover = clamp(0.5 - dist, 0.0, 1.0) * u_look.w;
    o_color = vec4(g * cover, cover);
}
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_gl3_and_es3_contexts_get_a_dialect() {
        assert_eq!(header(2, 1, false), None);
        assert_eq!(header(2, 0, true), None);
        assert_eq!(header(1, 1, false), None);
        assert!(header(3, 0, false).is_some_and(|h| h.starts_with("#version 130")));
        assert!(header(3, 3, false).is_some_and(|h| h.starts_with("#version 140")));
        assert!(header(4, 5, false).is_some_and(|h| h.starts_with("#version 140")));
        let es = header(3, 0, true).unwrap_or_default();
        assert!(es.starts_with("#version 300 es") && es.contains("precision highp float"));
    }

    #[test]
    fn every_program_writes_one_output() {
        for source in [down(), up(), COMPOSITE.to_owned()] {
            assert_eq!(source.matches("out vec4 o_color;").count(), 2, "{source}");
            assert!(source.contains("void main()"));
        }
        assert!(VERTEX.contains("gl_VertexID"));
    }
}
