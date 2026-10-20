//! GPU rendering of the universe (cells) and of the egui overlay.

mod density;
mod grid;

pub use density::density_grid;

use crate::sim::{MetaMap, Snapshot, TileCoord, TileMap, TILE};
use crate::ui::{CellStyle, Settings};
use crate::view::Camera;
use bytemuck::{Pod, Zeroable};
use density::DensityView;
use grid::GridView;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use winit::window::Window;

/// Background color `#0d1117` (sRGB).
const BACKGROUND: [f64; 3] = [
    0x0d as f64 / 255.0,
    0x11 as f64 / 255.0,
    0x17 as f64 / 255.0,
];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    cam_off: [f32; 2],
    screen: [f32; 2],
    base_rgb: [f32; 3],
    px_per_cell: f32,
    base_hsl: [f32; 3],
    variation: f32,
    style: u32,
    flags: u32,
    anim_t: f32,
    _pad: u32,
}

const FLAG_DEGRADED: u32 = 1;
const FLAG_SRGB_TARGET: u32 = 2;
const FLAG_AGE: u32 = 4;
const FLAG_TRAILS: u32 = 8;
const FLAG_ANIM: u32 = 16;

/// Below this zoom the density view replaces the cell path.
const DENSITY_PX_PER_CELL: f64 = 0.5;

/// Bytes per tile of the age and trail buffers (one u8 per cell).
const META_BYTES: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuTile {
    rel: [i32; 2],
    abs_lo: [i32; 2],
    rows: [u32; 128],
}

/// Inclusive range of tile coordinates.
#[derive(Clone, Copy, PartialEq, Eq)]
struct TileRange {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

/// Tiles of `map` inside `range`, by probing the range or scanning the map,
/// whichever is smaller.
fn visible_tiles<V>(map: &FxHashMap<TileCoord, V>, range: TileRange) -> Vec<(TileCoord, &V)> {
    let range_len = ((range.x1 - range.x0 + 1) as u128) * ((range.y1 - range.y0 + 1) as u128);
    if range_len <= map.len() as u128 {
        (range.y0..=range.y1)
            .flat_map(|ty| (range.x0..=range.x1).map(move |tx| (tx, ty)))
            .filter_map(|c| map.get(&c).map(|v| (c, v)))
            .collect()
    } else {
        map.iter()
            .filter(|(c, _)| {
                (range.x0..=range.x1).contains(&c.0) && (range.y0..=range.y1).contains(&c.1)
            })
            .map(|(&c, v)| (c, v))
            .collect()
    }
}

/// Effect buffers uploaded alongside the tiles.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Effects {
    age: bool,
    trails: bool,
    anim: bool,
}

/// What the uploaded tile buffers currently hold.
struct UploadKey {
    tiles: Arc<TileMap>,
    prev: Option<Arc<TileMap>>,
    meta: Option<Arc<MetaMap>>,
    effects: Effects,
    center_tile: TileCoord,
    range: TileRange,
}

fn same_arc<T>(a: &Option<Arc<T>>, b: &Option<Arc<T>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => true,
        _ => false,
    }
}

/// A storage buffer that grows (to a power of two) on demand.
struct StorageBuf {
    buf: wgpu::Buffer,
    size: u64,
    label: &'static str,
}

impl StorageBuf {
    fn new(device: &wgpu::Device, label: &'static str, size: u64) -> Self {
        StorageBuf {
            buf: create_storage_buffer(device, label, size),
            size,
            label,
        }
    }

    /// Makes room for `bytes`; returns whether the buffer was recreated.
    fn reserve(&mut self, device: &wgpu::Device, bytes: u64) -> bool {
        if bytes <= self.size {
            return false;
        }
        let max = device.limits().max_storage_buffer_binding_size as u64;
        self.size = bytes.next_power_of_two().min(max);
        self.buf = create_storage_buffer(device, self.label, self.size);
        true
    }
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    tile_buf: StorageBuf,
    age_buf: StorageBuf,
    trail_buf: StorageBuf,
    prev_buf: StorageBuf,
    bind_group: wgpu::BindGroup,
    tile_count: u32,
    upload_key: Option<UploadKey>,
    staging: Vec<GpuTile>,
    staging_age: Vec<u8>,
    staging_trail: Vec<u8>,
    staging_prev: Vec<[u32; 128]>,
    density: DensityView,
    grid: GridView,
    egui: egui_wgpu::Renderer,
    window: Arc<Window>,
}

impl Renderer {
    pub fn new(window: Arc<Window>) -> Self {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = instance
            .create_surface(window.clone())
            .expect("failed to create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("no suitable GPU adapter");
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
                .expect("failed to create device");

        let caps = surface.get_capabilities(&adapter);
        // Prefer a non-sRGB target: colors are blended in sRGB space like the mockups.
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
        };
        surface.configure(&device, &config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tiles"),
            source: wgpu::ShaderSource::Wgsl(include_str!("tiles.wgsl").into()),
        });
        let storage = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tiles"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1, wgpu::ShaderStages::VERTEX_FRAGMENT),
                storage(2, wgpu::ShaderStages::FRAGMENT),
                storage(3, wgpu::ShaderStages::FRAGMENT),
                storage(4, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tiles"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tiles"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tile uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tile_buf = StorageBuf::new(
            &device,
            "tiles",
            (1024 * std::mem::size_of::<GpuTile>()) as u64,
        );
        // Effect buffers start tiny; they only grow once an effect is used.
        let age_buf = StorageBuf::new(&device, "ages", 16);
        let trail_buf = StorageBuf::new(&device, "trails", 16);
        let prev_buf = StorageBuf::new(&device, "prev rows", 16);
        let bind_group = create_bind_group(
            &device,
            &bind_layout,
            &uniforms,
            [&tile_buf, &age_buf, &trail_buf, &prev_buf],
        );
        let density = DensityView::new(&device, format);
        let grid = GridView::new(&device, format);
        let egui = egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::default());

        Renderer {
            surface,
            device,
            queue,
            config,
            pipeline,
            bind_layout,
            uniforms,
            tile_buf,
            age_buf,
            trail_buf,
            prev_buf,
            bind_group,
            tile_count: 0,
            upload_key: None,
            staging: Vec::new(),
            staging_age: Vec::new(),
            staging_trail: Vec::new(),
            staging_prev: Vec::new(),
            density,
            grid,
            egui,
            window,
        }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn render(
        &mut self,
        cam: &Camera,
        snap: &Snapshot,
        s: &Settings,
        anim_t: f32,
        ui: Option<(&egui::Context, egui::FullOutput)>,
    ) {
        // egui sends each texture delta only once: apply uploads before anything
        // can fail, and keep the frees so they happen whatever the outcome.
        let mut ui_frame = None;
        let mut free = Vec::new();
        if let Some((ctx, output)) = ui {
            for (id, delta) in &output.textures_delta.set {
                self.egui
                    .update_texture(&self.device, &self.queue, *id, delta);
            }
            free = output.textures_delta.free;
            ui_frame = Some((ctx, output.shapes, output.pixels_per_point));
        }

        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(e) => {
                if matches!(e, wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) {
                    self.surface.configure(&self.device, &self.config);
                }
                self.free_egui_textures(&free);
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let srgb_target = self.config.format.is_srgb();
        let screen = [self.config.width as f32, self.config.height as f32];

        self.grid.prepare(&self.queue, cam, s.grid, srgb_target);
        let density_view = cam.px_per_cell < DENSITY_PX_PER_CELL;
        if density_view {
            self.density.prepare(
                &self.device,
                &self.queue,
                cam,
                &snap.tiles,
                s.color,
                srgb_target,
                screen,
            );
        } else {
            let center_tile = (
                (cam.center.0.floor() as i64).div_euclid(TILE),
                (cam.center.1.floor() as i64).div_euclid(TILE),
            );
            let has_meta = snap.meta.is_some();
            let fx = Effects {
                age: !s.degraded && s.effect_age && has_meta,
                trails: !s.degraded && s.effect_trails && has_meta,
                anim: !s.degraded && s.effect_anim,
            };
            self.upload_tiles(cam, snap, center_tile, fx);

            let u = Uniforms {
                cam_off: [
                    (cam.center.0 - (center_tile.0 * TILE) as f64) as f32,
                    (cam.center.1 - (center_tile.1 * TILE) as f64) as f32,
                ],
                screen,
                base_rgb: s.color,
                px_per_cell: cam.px_per_cell as f32,
                base_hsl: rgb_to_hsl(s.color),
                variation: s.variation,
                style: style_index(s.style),
                flags: if s.degraded { FLAG_DEGRADED } else { 0 }
                    | if srgb_target { FLAG_SRGB_TARGET } else { 0 }
                    | if fx.age { FLAG_AGE } else { 0 }
                    | if fx.trails { FLAG_TRAILS } else { 0 }
                    | if fx.anim { FLAG_ANIM } else { 0 },
                anim_t: anim_t.clamp(0.0, 1.0),
                _pad: 0,
            };
            self.queue
                .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        // Prepare egui buffers and textures before the pass.
        let mut egui_frame = None;
        let mut extra_cmds = Vec::new();
        if let Some((ctx, shapes, pixels_per_point)) = ui_frame {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.config.width, self.config.height],
                pixels_per_point,
            };
            let jobs = ctx.tessellate(shapes, pixels_per_point);
            extra_cmds =
                self.egui
                    .update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
            egui_frame = Some((jobs, screen));
        }

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("main"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(background(srgb_target)),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            // Under the cells, so they stay crisp on top.
            self.grid.draw(&mut pass);
            if density_view {
                self.density.draw(&mut pass);
            } else if self.tile_count > 0 {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..6, 0..self.tile_count);
            }
            if let Some((jobs, screen)) = &egui_frame {
                self.egui.render(&mut pass, jobs, screen);
            }
        }

        self.queue.submit(
            extra_cmds
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        self.window.pre_present_notify();
        frame.present();

        self.free_egui_textures(&free);
    }

    fn free_egui_textures(&mut self, ids: &[egui::TextureId]) {
        for id in ids {
            self.egui.free_texture(id);
        }
    }

    /// Uploads the visible tiles (plus a one-tile margin) and the buffers of
    /// the active effects when the snapshot, the effects or the visible tile
    /// range changed. Uploaded tiles are the visible live tiles, plus the
    /// visible meta tiles with trails (trails on dead tiles) and the visible
    /// previous-generation tiles with animation (cells dying in a dead tile).
    fn upload_tiles(&mut self, cam: &Camera, snap: &Snapshot, center_tile: TileCoord, fx: Effects) {
        let r = cam.visible_rect();
        let range = TileRange {
            x0: r.x0.div_euclid(TILE) - 1,
            y0: r.y0.div_euclid(TILE) - 1,
            x1: r.x1.div_euclid(TILE) + 1,
            y1: r.y1.div_euclid(TILE) + 1,
        };
        let prev = fx.anim.then(|| snap.prev.clone());
        let meta = if fx.age || fx.trails {
            snap.meta.clone()
        } else {
            None
        };
        if let Some(k) = &self.upload_key {
            if Arc::ptr_eq(&k.tiles, &snap.tiles)
                && same_arc(&k.prev, &prev)
                && same_arc(&k.meta, &meta)
                && k.effects == fx
                && k.center_tile == center_tile
                && k.range == range
            {
                return;
            }
        }

        let mut coords: Vec<TileCoord> = visible_tiles(&snap.tiles, range)
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        if let Some(prev) = &prev {
            coords.extend(
                visible_tiles(prev, range)
                    .into_iter()
                    .map(|(c, _)| c)
                    .filter(|c| !snap.tiles.contains_key(c)),
            );
        }
        if let (Some(meta), true) = (&meta, fx.trails) {
            coords.extend(
                visible_tiles(meta, range)
                    .into_iter()
                    .map(|(c, _)| c)
                    .filter(|c| {
                        !snap.tiles.contains_key(c)
                            && !prev.as_ref().is_some_and(|p| p.contains_key(c))
                    }),
            );
        }

        // Never exceed what a single storage binding can address.
        let max_binding = self.device.limits().max_storage_buffer_binding_size as usize;
        let mut stride = std::mem::size_of::<GpuTile>();
        if fx.age || fx.trails {
            stride = stride.max(META_BYTES);
        }
        coords.truncate(max_binding / stride);

        self.staging.clear();
        self.staging_age.clear();
        self.staging_trail.clear();
        self.staging_prev.clear();
        let zero_meta = [0u8; META_BYTES];
        for &coord in &coords {
            self.staging.push(GpuTile {
                rel: [
                    (coord.0 - center_tile.0) as i32,
                    (coord.1 - center_tile.1) as i32,
                ],
                abs_lo: [coord.0 as i32, coord.1 as i32],
                rows: snap
                    .tiles
                    .get(&coord)
                    .map_or([0; 128], |t| pack_rows(&t.rows)),
            });
            let m = meta.as_ref().and_then(|m| m.get(&coord));
            // Bytes are laid out so that the shader reads cell `i` as byte
            // `i % 4` (little-endian) of u32 `i / 4`.
            if fx.age {
                self.staging_age
                    .extend_from_slice(m.map_or(&zero_meta, |m| &m.age));
            }
            if fx.trails {
                self.staging_trail
                    .extend_from_slice(m.map_or(&zero_meta, |m| &m.trail));
            }
            if let Some(prev) = &prev {
                self.staging_prev
                    .push(prev.get(&coord).map_or([0; 128], |t| pack_rows(&t.rows)));
            }
        }

        let tiles_bytes: &[u8] = bytemuck::cast_slice(&self.staging);
        let prev_bytes: &[u8] = bytemuck::cast_slice(&self.staging_prev);
        let mut regrown = false;
        for (buf, bytes) in [
            (&mut self.tile_buf, tiles_bytes),
            (&mut self.age_buf, &self.staging_age[..]),
            (&mut self.trail_buf, &self.staging_trail[..]),
            (&mut self.prev_buf, prev_bytes),
        ] {
            regrown |= buf.reserve(&self.device, bytes.len() as u64);
            if !bytes.is_empty() {
                self.queue.write_buffer(&buf.buf, 0, bytes);
            }
        }
        if regrown {
            self.bind_group = create_bind_group(
                &self.device,
                &self.bind_layout,
                &self.uniforms,
                [
                    &self.tile_buf,
                    &self.age_buf,
                    &self.trail_buf,
                    &self.prev_buf,
                ],
            );
        }
        self.tile_count = self.staging.len() as u32;
        self.upload_key = Some(UploadKey {
            tiles: snap.tiles.clone(),
            prev,
            meta,
            effects: fx,
            center_tile,
            range,
        });
    }
}

/// Splits 64-bit rows into u32 pairs, low word first (columns 0..31, then 32..63).
fn pack_rows(rows: &[u64; 64]) -> [u32; 128] {
    let mut out = [0u32; 128];
    for (i, row) in rows.iter().enumerate() {
        out[2 * i] = *row as u32;
        out[2 * i + 1] = (*row >> 32) as u32;
    }
    out
}

fn create_storage_buffer(device: &wgpu::Device, label: &str, size: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    storage: [&StorageBuf; 4],
) -> wgpu::BindGroup {
    let mut entries = vec![wgpu::BindGroupEntry {
        binding: 0,
        resource: uniforms.as_entire_binding(),
    }];
    for (binding, buf) in (1..).zip(storage) {
        entries.push(wgpu::BindGroupEntry {
            binding,
            resource: buf.buf.as_entire_binding(),
        });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tiles"),
        layout,
        entries: &entries,
    })
}

fn style_index(style: CellStyle) -> u32 {
    match style {
        CellStyle::Candy => 0,
        CellStyle::Rounded => 1,
        CellStyle::Dots => 2,
        CellStyle::Neon => 3,
    }
}

fn background(srgb_target: bool) -> wgpu::Color {
    let c = if srgb_target {
        BACKGROUND.map(srgb_to_linear)
    } else {
        BACKGROUND
    };
    wgpu::Color {
        r: c[0],
        g: c[1],
        b: c[2],
        a: 1.0,
    }
}

fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB (0..1) to HSL: hue in degrees, saturation and lightness in 0..1.
fn rgb_to_hsl([r, g, b]: [f32; 3]) -> [f32; 3] {
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return [0.0, 0.0, l];
    }
    let d = mx - mn;
    let s = if l > 0.5 {
        d / (2.0 - mx - mn)
    } else {
        d / (mx + mn)
    };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [h * 60.0, s, l]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_rows_low_word_first() {
        let mut rows = [0u64; 64];
        rows[5] = (1 << 0) | (1 << 31) | (1 << 32) | (1 << 63);
        let packed = pack_rows(&rows);
        assert_eq!(packed[10], (1 << 0) | (1 << 31));
        assert_eq!(packed[11], (1 << 0) | (1 << 31));
        assert!(packed
            .iter()
            .enumerate()
            .all(|(i, w)| i == 10 || i == 11 || *w == 0));
    }
}
