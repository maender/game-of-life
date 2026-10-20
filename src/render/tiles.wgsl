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
    // Bits 2, 3, 4: age, trail and animation effects (their buffers are bound).
    flags: u32,
    // Progress of the birth/death animation, 0..1.
    anim_t: f32,
    _pad: u32,
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
// Parallel to `tiles`, only meaningful when the matching effect flag is set.
// Ages and trails: one byte per cell (index y * 64 + x), 4 per u32, 1024 u32 per tile.
@group(0) @binding(2) var<storage, read> ages: array<u32>;
@group(0) @binding(3) var<storage, read> trails: array<u32>;
// Previous generation rows, laid out like `GpuTile::rows` (128 u32 per tile).
@group(0) @binding(4) var<storage, read> prev_rows: array<u32>;

const TILE: f32 = 64.0;
const STYLE_CANDY: u32 = 0u;
const STYLE_ROUNDED: u32 = 1u;
const STYLE_DOTS: u32 = 2u;
const FLAG_DEGRADED: u32 = 1u;
const FLAG_SRGB_TARGET: u32 = 2u;
const FLAG_AGE: u32 = 4u;
const FLAG_TRAILS: u32 = 8u;
const FLAG_ANIM: u32 = 16u;
const AGE_MAX: f32 = 12.0;
const TRAIL_LEN: f32 = 4.0;

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

fn prev_alive(t: u32, x: u32, y: u32) -> bool {
    let word = prev_rows[t * 128u + y * 2u + (x >> 5u)];
    return ((word >> (x & 31u)) & 1u) == 1u;
}

fn byte_at(word: u32, i: u32) -> u32 {
    return (word >> ((i & 3u) * 8u)) & 0xffu;
}

fn cell_age(t: u32, x: u32, y: u32) -> u32 {
    let i = y * 64u + x;
    return byte_at(ages[t * 1024u + (i >> 2u)], i);
}

fn cell_trail(t: u32, x: u32, y: u32) -> u32 {
    let i = y * 64u + x;
    return byte_at(trails[t * 1024u + (i >> 2u)], i);
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
    // Shape distance derivatives are computed first, in uniform
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

    let alive = cell_alive(t, cx, cy);
    var was = alive;
    if (u.flags & FLAG_ANIM) != 0u {
        was = prev_alive(t, cx, cy);
    }
    // Opacity of the fading trail left by a dead cell.
    var trail_a = 0.0;
    if !alive && (u.flags & FLAG_TRAILS) != 0u {
        trail_a = 0.7 * f32(cell_trail(t, cx, cy)) / TRAIL_LEN;
    }
    // Size of the live shape: grows when born, shrinks when dying.
    var scale = 0.0;
    if alive {
        scale = select(1.0, u.anim_t, !was);
    } else if was {
        scale = 1.0 - u.anim_t;
    }
    if scale <= 0.02 { scale = 0.0; }
    if scale == 0.0 && trail_a <= 0.0 { discard; }

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
    var light = clamp(u.base_hsl.z + r2 * 0.07 * k, 0.0, 1.0);
    let sat = u.base_hsl.y;
    if alive && (u.flags & FLAG_AGE) != 0u {
        // +25% lightness when born (age 1) down to -10% at AGE_MAX.
        let a = (clamp(f32(cell_age(t, cx, cy)), 1.0, AGE_MAX) - 1.0) / (AGE_MAX - 1.0);
        light = clamp(u.base_hsl.z + 0.25 - a * 0.35 + r2 * 0.04 * k, 0.12, 0.92);
    }
    let col = vec3<f32>(hue, sat, light);
    let fw = vec3<f32>(fw_rect, fw_dot, fw_line);

    var out = vec4<f32>(0.0);
    if scale > 0.0 {
        out = shape(p, scale, col, fw, ppc);
    }
    if trail_a > 0.0 {
        let tr = shape(p, 1.0, col, fw, ppc);
        if tr.a * trail_a > out.a {
            out = vec4<f32>(tr.rgb, tr.a * trail_a);
        }
    }
    if out.a <= 0.0 { discard; }
    return finish(out.rgb, out.a);
}

// One cell of the current style, scaled by `s` around the cell center.
// `p`: position relative to the cell center (cells); `col`: HSL color;
// `fw`: screen-space derivatives of the rect, dot and neon line distances
// (unchanged by scaling, since a scaled distance keeps a unit gradient).
// Returns sRGB color and alpha.
fn shape(p: vec2<f32>, s: f32, col: vec3<f32>, fw: vec3<f32>, ppc: f32) -> vec4<f32> {
    let hue = col.x;
    let sat = col.y;
    let light = col.z;
    // Too small for the shape to be visible: plain square, faded by its scale.
    if ppc < 3.0 {
        return vec4<f32>(hsl_to_rgb(col), s);
    }
    let q = p / s;
    if u.style == STYLE_CANDY {
        let a = coverage(sd_round_rect(q, 0.5 - MARGIN, RADIUS) * s, fw.x);
        // Vertical gradient: +18% lightness at the top, -12% at the bottom.
        let l = clamp(light + mix(0.18, -0.12, q.y + 0.5), 0.05, 0.95);
        return vec4<f32>(hsl_to_rgb(vec3<f32>(hue, sat, l)), a);
    }
    if u.style == STYLE_ROUNDED {
        let a = coverage(sd_round_rect(q, 0.5 - MARGIN, RADIUS) * s, fw.x);
        return vec4<f32>(hsl_to_rgb(col), a);
    }
    if u.style == STYLE_DOTS {
        let sd_dot = length(q) - DOT_RADIUS;
        let a_disk = coverage(sd_dot * s, fw.y);
        // Radial halo fading to zero at the cell border.
        let fall = 1.0 - clamp(sd_dot / (0.5 - DOT_RADIUS), 0.0, 1.0);
        let halo = 0.5 * fall * fall;
        return vec4<f32>(hsl_to_rgb(col), a_disk + (1.0 - a_disk) * halo);
    }
    // Neon: bright rounded outline with a soft glow, nearly empty inside.
    // Path inset so the outer edge of the stroke sits on the margin.
    let sd_path = sd_round_rect(q, 0.5 - MARGIN - NEON_HALF_WIDTH, RADIUS);
    let sd_line = abs(sd_path) - NEON_HALF_WIDTH;
    let a_stroke = coverage(sd_line * s, fw.z);
    let glow = 0.35 * exp(-max(sd_line, 0.0) / 0.05);
    let a_line = max(a_stroke, glow);
    let a_fill = 0.15 * coverage(sd_path * s, fw.x);
    let a = a_line + (1.0 - a_line) * a_fill;
    let line_rgb = hsl_to_rgb(vec3<f32>(hue, sat, min(light + 0.15, 0.8)));
    let fill_rgb = hsl_to_rgb(col);
    let rgb = (line_rgb * a_line + fill_rgb * a_fill * (1.0 - a_line)) / max(a, 1e-6);
    return vec4<f32>(rgb, a);
}
