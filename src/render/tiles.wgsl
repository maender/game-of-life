// Cell rendering: one instanced quad per 64x64 tile; the fragment shader
// looks up the cell bit and shapes it according to the selected style.

struct Uniforms {
    // Camera center in cells, relative to the origin of the tile containing it.
    cam_off: vec2<f32>,
    // Screen size in physical pixels.
    screen: vec2<f32>,
    // Main color (sRGB 0..1), used verbatim in degraded mode.
    base_rgb: vec3<f32>,
    px_per_cell: f32,
    // Main color as HSL (hue in degrees, saturation and lightness 0..1).
    base_hsl: vec3<f32>,
    variation: f32,
    // 0 = Candy, 1 = Rounded, 2 = Dots, 3 = Neon.
    style: u32,
    // Bit 0: degraded mode. Bit 1: output target is sRGB (write linear values).
    flags: u32,
    _pad: vec2<u32>,
}

struct GpuTile {
    // Tile coordinate minus the camera-center tile coordinate.
    rel: vec2<i32>,
    // Low 32 bits of the absolute tile coordinate (for the color hash).
    abs_lo: vec2<i32>,
    // 64 rows of 64 bits, as (low word = columns 0..31, high word = 32..63).
    rows: array<u32, 128>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> tiles: array<GpuTile>;

const TILE: f32 = 64.0;
const STYLE_CANDY: u32 = 0u;
const STYLE_ROUNDED: u32 = 1u;
const STYLE_DOTS: u32 = 2u;
const FLAG_DEGRADED: u32 = 1u;
const FLAG_SRGB_TARGET: u32 = 2u;

// Cell geometry, in cell units.
const MARGIN: f32 = 0.09375;      // 1.5 / 16
const RADIUS: f32 = 0.25;         // 4 / 16
const DOT_RADIUS: f32 = 0.375;    // 0.5 - 2 / 16
const NEON_HALF_WIDTH: f32 = 0.05; // 1.6 / 16 / 2

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Position inside the tile, in cells (0..64).
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) tile: u32,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let local = corners[vi] * TILE;
    let cell = vec2<f32>(tiles[ii].rel) * TILE + local - u.cam_off;
    let px = cell * u.px_per_cell + u.screen * 0.5;
    let ndc = vec2<f32>(px.x / u.screen.x * 2.0 - 1.0, 1.0 - px.y / u.screen.y * 2.0);
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.local = local;
    out.tile = ii;
    return out;
}

fn cell_alive(t: u32, x: u32, y: u32) -> bool {
    let word = tiles[t].rows[y * 2u + (x >> 5u)];
    return ((word >> (x & 31u)) & 1u) == 1u;
}

// Stable integer hash of an absolute cell (low bits of its coordinates).
fn hash2(x: u32, y: u32) -> u32 {
    var h = (x * 0x8da6b343u) ^ (y * 0xd8163841u);
    h = h ^ (h >> 16u);
    h = h * 0x7feb352du;
    h = h ^ (h >> 15u);
    h = h * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return h;
}

fn hue_to_rgb(p: f32, q: f32, t_in: f32) -> f32 {
    var t = fract(t_in);
    if t < 1.0 / 6.0 { return p + (q - p) * 6.0 * t; }
    if t < 0.5 { return q; }
    if t < 2.0 / 3.0 { return p + (q - p) * (2.0 / 3.0 - t) * 6.0; }
    return p;
}

fn hsl_to_rgb(hsl: vec3<f32>) -> vec3<f32> {
    let h = hsl.x / 360.0;
    let s = hsl.y;
    let l = hsl.z;
    if s <= 0.0 { return vec3<f32>(l); }
    let q = select(l + s - l * s, l * (1.0 + s), l < 0.5);
    let p = 2.0 * l - q;
    return vec3<f32>(
        hue_to_rgb(p, q, h + 1.0 / 3.0),
        hue_to_rgb(p, q, h),
        hue_to_rgb(p, q, h - 1.0 / 3.0),
    );
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

fn finish(rgb: vec3<f32>, a: f32) -> vec4<f32> {
    var c = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if (u.flags & FLAG_SRGB_TARGET) != 0u {
        c = srgb_to_linear(c);
    }
    return vec4<f32>(c, a);
}

fn sd_round_rect(p: vec2<f32>, half_size: f32, r: f32) -> f32 {
    let q = abs(p) - vec2<f32>(half_size - r);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

// Coverage of the inside (sd < 0) of a signed distance, antialiased over one pixel.
fn coverage(sd: f32, fw: f32) -> f32 {
    return clamp(0.5 - sd / max(fw, 1e-6), 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Shape distances and their derivatives are computed first, in uniform
    // control flow (before any data-dependent branch or discard).
    let uv = fract(in.local);
    let p = uv - vec2<f32>(0.5);
    let sd_rect = sd_round_rect(p, 0.5 - MARGIN, RADIUS);
    let fw_rect = fwidth(sd_rect);
    let sd_dot = length(p) - DOT_RADIUS;
    let fw_dot = fwidth(sd_dot);
    // Neon path: inset so the outer edge of the stroke sits on the margin.
    let sd_path = sd_round_rect(p, 0.5 - MARGIN - NEON_HALF_WIDTH, RADIUS);
    let sd_line = abs(sd_path) - NEON_HALF_WIDTH;
    let fw_line = fwidth(sd_line);

    let t = in.tile;
    let cx = u32(clamp(floor(in.local.x), 0.0, 63.0));
    let cy = u32(clamp(floor(in.local.y), 0.0, 63.0));
    let degraded = (u.flags & FLAG_DEGRADED) != 0u;
    let ppc = u.px_per_cell;

    // Sub-pixel cells: brightness from the density of the pixel's footprint.
    if ppc < 1.0 {
        let h = 0.5 / ppc;
        let x0 = u32(clamp(floor(in.local.x - h + 1e-3), 0.0, 63.0));
        let x1 = u32(clamp(floor(in.local.x + h - 1e-3), 0.0, 63.0));
        let y0 = u32(clamp(floor(in.local.y - h + 1e-3), 0.0, 63.0));
        let y1 = u32(clamp(floor(in.local.y + h - 1e-3), 0.0, 63.0));
        var alive = 0u;
        for (var y = y0; y <= y1; y++) {
            for (var x = x0; x <= x1; x++) {
                if cell_alive(t, x, y) { alive++; }
            }
        }
        if alive == 0u { discard; }
        let density = f32(alive) / f32((x1 - x0 + 1u) * (y1 - y0 + 1u));
        return finish(u.base_rgb * sqrt(density), 1.0);
    }

    if !cell_alive(t, cx, cy) { discard; }

    if degraded {
        return finish(u.base_rgb, 1.0);
    }

    // Per-cell color variation from a stable hash of the absolute cell.
    let ax = (u32(tiles[t].abs_lo.x) << 6u) | cx;
    let ay = (u32(tiles[t].abs_lo.y) << 6u) | cy;
    let hsh = hash2(ax, ay);
    let r1 = f32(hsh & 0xffffu) / 65535.0 * 2.0 - 1.0;
    let r2 = f32(hsh >> 16u) / 65535.0 * 2.0 - 1.0;
    let k = u.variation / 0.35;
    let hue = u.base_hsl.x + r1 * 14.0 * k;
    let light = clamp(u.base_hsl.z + r2 * 0.07 * k, 0.0, 1.0);
    let sat = u.base_hsl.y;

    // Too small for the shape to be visible: plain square.
    if ppc < 3.0 {
        return finish(hsl_to_rgb(vec3<f32>(hue, sat, light)), 1.0);
    }

    if u.style == STYLE_CANDY {
        let a = coverage(sd_rect, fw_rect);
        if a <= 0.0 { discard; }
        // Vertical gradient: +18% lightness at the top, -12% at the bottom.
        let l = clamp(light + mix(0.18, -0.12, uv.y), 0.05, 0.95);
        return finish(hsl_to_rgb(vec3<f32>(hue, sat, l)), a);
    }
    if u.style == STYLE_ROUNDED {
        let a = coverage(sd_rect, fw_rect);
        if a <= 0.0 { discard; }
        return finish(hsl_to_rgb(vec3<f32>(hue, sat, light)), a);
    }
    if u.style == STYLE_DOTS {
        let a_disk = coverage(sd_dot, fw_dot);
        // Radial halo fading to zero at the cell border.
        let fall = 1.0 - clamp(sd_dot / (0.5 - DOT_RADIUS), 0.0, 1.0);
        let halo = 0.5 * fall * fall;
        let a = a_disk + (1.0 - a_disk) * halo;
        if a <= 0.0 { discard; }
        return finish(hsl_to_rgb(vec3<f32>(hue, sat, light)), a);
    }
    // Neon: bright rounded outline with a soft glow, nearly empty inside.
    let a_stroke = coverage(sd_line, fw_line);
    let glow = 0.35 * exp(-max(sd_line, 0.0) / 0.05);
    let a_line = max(a_stroke, glow);
    let a_fill = 0.15 * coverage(sd_path, fw_rect);
    let a = a_line + (1.0 - a_line) * a_fill;
    if a <= 0.0 { discard; }
    let line_rgb = hsl_to_rgb(vec3<f32>(hue, sat, min(light + 0.15, 0.8)));
    let fill_rgb = hsl_to_rgb(vec3<f32>(hue, sat, light));
    let rgb = (line_rgb * a_line + fill_rgb * a_fill * (1.0 - a_line)) / max(a, 1e-6);
    return finish(rgb, a);
}
