//! GPU rendering of the universe (cells) and of the egui overlay.

use crate::sim::{Snapshot, TileCoord, TileMap, TILE};
use crate::ui::{CellStyle, Settings};
use crate::view::Camera;
use bytemuck::{Pod, Zeroable};
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
    _pad: [u32; 2],
}

const FLAG_DEGRADED: u32 = 1;
const FLAG_SRGB_TARGET: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuTile {
    rel: [i32; 2],
    abs_lo: [i32; 2],
    rows: [u32; 128],
}

/// What the uploaded tile buffer currently holds.
struct UploadKey {
    tiles: Arc<TileMap>,
    center_tile: TileCoord,
    range: (i64, i64, i64, i64),
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    tile_buf: wgpu::Buffer,
    tile_capacity: usize,
    bind_group: wgpu::BindGroup,
    tile_count: u32,
    upload_key: Option<UploadKey>,
    staging: Vec<GpuTile>,
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
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
        let tile_capacity = 1024;
        let tile_buf = create_tile_buffer(&device, tile_capacity);
        let bind_group = create_bind_group(&device, &bind_layout, &uniforms, &tile_buf);
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
            tile_capacity,
            bind_group,
            tile_count: 0,
            upload_key: None,
            staging: Vec::new(),
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
        _anim_t: f32,
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

        let center_tile = (
            (cam.center.0.floor() as i64).div_euclid(TILE),
            (cam.center.1.floor() as i64).div_euclid(TILE),
        );
        self.upload_tiles(cam, snap, center_tile);

        let u = Uniforms {
            cam_off: [
                (cam.center.0 - (center_tile.0 * TILE) as f64) as f32,
                (cam.center.1 - (center_tile.1 * TILE) as f64) as f32,
            ],
            screen: [self.config.width as f32, self.config.height as f32],
            base_rgb: s.color,
            px_per_cell: cam.px_per_cell as f32,
            base_hsl: rgb_to_hsl(s.color),
            variation: s.variation,
            style: style_index(s.style),
            flags: if s.degraded { FLAG_DEGRADED } else { 0 }
                | if srgb_target { FLAG_SRGB_TARGET } else { 0 },
            _pad: [0; 2],
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));

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
            if self.tile_count > 0 {
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

    /// Uploads the visible tiles (plus a one-tile margin) when the snapshot or
    /// the visible tile range changed.
    fn upload_tiles(&mut self, cam: &Camera, snap: &Snapshot, center_tile: TileCoord) {
        let r = cam.visible_rect();
        let range = (
            r.x0.div_euclid(TILE) - 1,
            r.y0.div_euclid(TILE) - 1,
            r.x1.div_euclid(TILE) + 1,
            r.y1.div_euclid(TILE) + 1,
        );
        if let Some(k) = &self.upload_key {
            if Arc::ptr_eq(&k.tiles, &snap.tiles)
                && k.center_tile == center_tile
                && k.range == range
            {
                return;
            }
        }

        self.staging.clear();
        let (tx0, ty0, tx1, ty1) = range;
        let mut push = |coord: TileCoord, rows: &[u64; 64]| {
            let t = GpuTile {
                rel: [
                    (coord.0 - center_tile.0) as i32,
                    (coord.1 - center_tile.1) as i32,
                ],
                abs_lo: [coord.0 as i32, coord.1 as i32],
                rows: pack_rows(rows),
            };
            self.staging.push(t);
        };
        let range_len = ((tx1 - tx0 + 1) as u128) * ((ty1 - ty0 + 1) as u128);
        if range_len <= snap.tiles.len() as u128 {
            for ty in ty0..=ty1 {
                for tx in tx0..=tx1 {
                    if let Some(tile) = snap.tiles.get(&(tx, ty)) {
                        push((tx, ty), &tile.rows);
                    }
                }
            }
        } else {
            for (&coord, tile) in snap.tiles.iter() {
                if (tx0..=tx1).contains(&coord.0) && (ty0..=ty1).contains(&coord.1) {
                    push(coord, &tile.rows);
                }
            }
        }

        // Never exceed what a single storage binding can address.
        let max_tiles = self.device.limits().max_storage_buffer_binding_size as usize
            / std::mem::size_of::<GpuTile>();
        self.staging.truncate(max_tiles);
        if self.staging.len() > self.tile_capacity {
            self.tile_capacity = self.staging.len().next_power_of_two().min(max_tiles);
            self.tile_buf = create_tile_buffer(&self.device, self.tile_capacity);
            self.bind_group = create_bind_group(
                &self.device,
                &self.bind_layout,
                &self.uniforms,
                &self.tile_buf,
            );
        }
        if !self.staging.is_empty() {
            self.queue
                .write_buffer(&self.tile_buf, 0, bytemuck::cast_slice(&self.staging));
        }
        self.tile_count = self.staging.len() as u32;
        self.upload_key = Some(UploadKey {
            tiles: snap.tiles.clone(),
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

fn create_tile_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tiles"),
        size: (capacity * std::mem::size_of::<GpuTile>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    tiles: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tiles"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: tiles.as_entire_binding(),
            },
        ],
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
