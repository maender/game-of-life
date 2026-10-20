// Grid overlay: a full-screen triangle drawing one-pixel lines on cell
// boundaries, blended over the background and under the cells.

struct Uniforms {
    // Line color, already in the target's color space.
    rgb: vec3<f32>,
    // Line opacity (0 = hidden).
    alpha: f32,
    // Screen position (px) of one vertical and one horizontal boundary.
    phase: vec2<f32>,
    px_per_cell: f32,
    _pad: f32,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // (0,0), (2,0), (0,2) in uv: one triangle covering the whole screen.
    let uv = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    // Distance (px) past the last boundary on each axis; the pixel whose
    // center lies within one pixel after a boundary carries the line.
    let d = fract((pos.xy - u.phase) / u.px_per_cell) * u.px_per_cell;
    if d.x >= 1.0 && d.y >= 1.0 { discard; }
    return vec4<f32>(u.rgb, u.alpha);
}
