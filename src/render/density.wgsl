// Far-zoom density view: a full-screen triangle reading the per-pixel live
// fraction from an R32Float texture; main color x sqrt(density).

struct Uniforms {
    // Main color (sRGB 0..1).
    base_rgb: vec3<f32>,
    // Bit 0: output target is sRGB (write linear values).
    flags: u32,
    // Screen size in physical pixels.
    screen: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var density: texture_2d<f32>;

const FLAG_SRGB_TARGET: u32 = 1u;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // (0,0), (2,0), (0,2) in uv: one triangle covering the whole screen.
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(density));
    let texel = min(floor(pos.xy / u.screen * dims), dims - 1.0);
    let d = textureLoad(density, vec2<u32>(texel), 0).r;
    if d <= 0.0 { discard; }
    var c = clamp(u.base_rgb * sqrt(min(d, 1.0)), vec3<f32>(0.0), vec3<f32>(1.0));
    if (u.flags & FLAG_SRGB_TARGET) != 0u {
        c = srgb_to_linear(c);
    }
    return vec4<f32>(c, 1.0);
}
