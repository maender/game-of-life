//! Far-zoom density view: fraction of live cells per output pixel, computed on
//! the CPU and drawn as a full-screen `R32Float` texture.

use super::{visible_tiles, TileRange};
use crate::sim::{TileMap, TILE};
use crate::view::Camera;
use bytemuck::{Pod, Zeroable};
use rayon::prelude::*;
use std::sync::Arc;
use std::time::Instant;

/// Output rows per parallel work item.
const BAND: usize = 16;

/// Fraction of live cells (0..1) of each of the `out_w` x `out_h` output pixels,
/// which together cover the camera's screen (row-major, top row first).
pub fn density_grid(tiles: &TileMap, cam: &Camera, out_w: u32, out_h: u32) -> Vec<f32> {
    let (w, h) = (out_w as usize, out_h as usize);
    let mut out = vec![0f32; w * h];
    if w == 0 || h == 0 {
        return out;
    }
    // Cells per output pixel, and the cell coordinate of the screen's top-left corner.
    let cw = cam.screen.0 / cam.px_per_cell / w as f64;
    let ch = cam.screen.1 / cam.px_per_cell / h as f64;
    let left = cam.center.0 - cam.screen.0 / 2.0 / cam.px_per_cell;
    let top = cam.center.1 - cam.screen.1 / 2.0 / cam.px_per_cell;
    let px_of = |x: i64| ((x as f64 + 0.5 - left) / cw).floor();
    let py_of = |y: i64| ((y as f64 + 0.5 - top) / ch).floor();
    // A tile no larger than a pixel adds its whole population to one pixel.
    let small = cw >= TILE as f64 && ch >= TILE as f64;

    let r = cam.visible_rect();
    let range = TileRange {
        x0: r.x0.div_euclid(TILE),
        y0: r.y0.div_euclid(TILE),
        x1: r.x1.div_euclid(TILE),
        y1: r.y1.div_euclid(TILE),
    };
    let visible = visible_tiles(tiles, range);

    // Bucket tiles by the bands of output rows they touch.
    let bands = h.div_ceil(BAND);
    let mut lists: Vec<Vec<usize>> = vec![Vec::new(); bands];
    for (i, ((_, ty), _)) in visible.iter().enumerate() {
        let (r0, r1) = if small {
            let py = py_of(ty * TILE + TILE / 2);
            (py, py)
        } else {
            (py_of(ty * TILE), py_of(ty * TILE + TILE - 1))
        };
        let (r0, r1) = (r0.max(0.0), r1.min(h as f64 - 1.0));
        if r0 > r1 {
            continue;
        }
        for list in &mut lists[r0 as usize / BAND..=r1 as usize / BAND] {
            list.push(i);
        }
    }

    let area = (cw * ch) as f32;
    out.par_chunks_mut(BAND * w)
        .zip(lists.par_iter())
        .enumerate()
        .for_each(|(b, (chunk, list))| {
            let y_lo = (b * BAND) as f64;
            let rows = chunk.len() / w;
            let in_band = |py: f64| py >= y_lo && py < y_lo + rows as f64;
            let in_width = |px: f64| px >= 0.0 && px < w as f64;
            for &i in list {
                let ((tx, ty), tile) = visible[i];
                if small {
                    let px = px_of(tx * TILE + TILE / 2);
                    let py = py_of(ty * TILE + TILE / 2);
                    if in_width(px) && in_band(py) {
                        chunk[(py - y_lo) as usize * w + px as usize] += tile.population() as f32;
                    }
                    continue;
                }
                // Column groups of the tile: (output column, mask of its cells).
                let mut groups: Vec<(usize, u64)> = Vec::with_capacity(TILE as usize);
                for lx in 0..TILE {
                    let px = px_of(tx * TILE + lx);
                    if !in_width(px) {
                        continue;
                    }
                    match groups.last_mut() {
                        Some((last, mask)) if *last == px as usize => *mask |= 1 << lx,
                        _ => groups.push((px as usize, 1 << lx)),
                    }
                }
                for (ly, &row) in tile.rows.iter().enumerate() {
                    if row == 0 {
                        continue;
                    }
                    let py = py_of(ty * TILE + ly as i64);
                    if !in_band(py) {
                        continue;
                    }
                    let line = &mut chunk[(py - y_lo) as usize * w..][..w];
                    for &(px, mask) in &groups {
                        line[px] += (row & mask).count_ones() as f32;
                    }
                }
            }
            for v in chunk.iter_mut() {
                *v = (*v / area).min(1.0);
            }
        });
    out
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    base_rgb: [f32; 3],
    flags: u32,
    screen: [f32; 2],
    _pad: [f32; 2],
}

/// What the density texture currently holds.
struct GridKey {
    tiles: Arc<TileMap>,
    center: (f64, f64),
    px_per_cell: f64,
    screen: (f64, f64),
    /// Texture size: the surface size (the shader's `screen`) / `div`.
    out: (u32, u32),
}

impl GridKey {
    fn same_as(&self, other: &GridKey) -> bool {
        Arc::ptr_eq(&self.tiles, &other.tiles)
            && self.center == other.center
            && self.px_per_cell == other.px_per_cell
            && self.screen == other.screen
            && self.out == other.out
    }
}

/// Grid computations slower than this (half a 60 FPS frame) switch to half resolution.
const SLOW_GRID_SECS: f64 = 0.008;

pub(super) struct DensityView {
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    texture: Option<(wgpu::Texture, wgpu::BindGroup)>,
    key: Option<GridKey>,
    /// Output resolution divisor (1 = screen resolution, 2 = half).
    div: u32,
}

impl DensityView {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("density"),
            source: wgpu::ShaderSource::Wgsl(include_str!("density.wgsl").into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("density"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("density"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("density"),
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
                    blend: None,
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
            label: Some("density uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        DensityView {
            pipeline,
            bind_layout,
            uniforms,
            texture: None,
            key: None,
            div: 1,
        }
    }

    /// Recomputes and uploads the density texture if the snapshot or the
    /// camera changed, and updates the uniforms.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cam: &Camera,
        tiles: &Arc<TileMap>,
        color: [f32; 3],
        srgb_target: bool,
        screen: [f32; 2],
    ) {
        let u = Uniforms {
            base_rgb: color,
            flags: srgb_target as u32,
            screen,
            _pad: [0.0; 2],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));

        // Sized from the same `screen` as the shader, so texels map 1:1 (or
        // 2:1) onto the surface.
        let out_w = ((screen[0] as u32) / self.div).max(1);
        let out_h = ((screen[1] as u32) / self.div).max(1);
        let key = GridKey {
            tiles: tiles.clone(),
            center: cam.center,
            px_per_cell: cam.px_per_cell,
            screen: cam.screen,
            out: (out_w, out_h),
        };
        if self.key.as_ref().is_some_and(|k| k.same_as(&key)) {
            return;
        }
        let start = Instant::now();
        let grid = density_grid(tiles, cam, out_w, out_h);
        let secs = start.elapsed().as_secs_f64();
        // Switch back only with a wide margin, so the resolution doesn't flip
        // on every recomputation.
        if self.div == 1 && secs > SLOW_GRID_SECS {
            self.div = 2;
        } else if self.div == 2 && secs * 4.0 < SLOW_GRID_SECS {
            self.div = 1;
        }

        let size = wgpu::Extent3d {
            width: out_w,
            height: out_h,
            depth_or_array_layers: 1,
        };
        if self.texture.as_ref().map(|(t, _)| t.size()) != Some(size) {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("density"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R32Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("density"),
                layout: &self.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniforms.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                ],
            });
            self.texture = Some((texture, bind_group));
        }
        let (texture, _) = self.texture.as_ref().expect("texture just created");
        queue.write_texture(
            texture.as_image_copy(),
            bytemuck::cast_slice(&grid),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(out_w * 4),
                rows_per_image: Some(out_h),
            },
            size,
        );
        self.key = Some(key);
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'static>) {
        if let Some((_, bind_group)) = &self.texture {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Tile;

    fn far_cam() -> Camera {
        Camera {
            center: (32.0, 32.0),
            px_per_cell: 1.0 / 64.0,
            screen: (3.0, 3.0),
        }
    }

    fn assert_center_only(grid: &[f32], center: f32) {
        assert_eq!(grid.len(), 9);
        for (i, v) in grid.iter().enumerate() {
            let want = if i == 4 { center } else { 0.0 };
            assert!((v - want).abs() <= 1e-6, "pixel {i}: {v} != {want}");
        }
    }

    #[test]
    fn density_full_tile() {
        let mut tiles = TileMap::default();
        tiles.insert(
            (0, 0),
            Tile {
                rows: [u64::MAX; 64],
            },
        );
        assert_center_only(&density_grid(&tiles, &far_cam(), 3, 3), 1.0);
    }

    #[test]
    fn density_half() {
        let mut rows = [0u64; 64];
        rows[..32].fill(u64::MAX);
        let mut tiles = TileMap::default();
        tiles.insert((0, 0), Tile { rows });
        assert_center_only(&density_grid(&tiles, &far_cam(), 3, 3), 0.5);
    }

    #[test]
    fn density_sub_block_negative_coords() {
        // Tile (-1, 0) covers cells x -64..-1, y 0..63; at 1/32 px per cell
        // each pixel holds 32x32 cells. Left half of the tile is full, right
        // half (columns 32..63) has only its top 16 rows alive.
        let mut rows = [u32::MAX as u64; 64];
        for (ly, row) in rows.iter_mut().enumerate() {
            if ly < 16 {
                *row = u64::MAX;
            }
        }
        let mut tiles = TileMap::default();
        tiles.insert((-1, 0), Tile { rows });
        let cam = Camera {
            center: (0.0, 32.0),
            px_per_cell: 1.0 / 32.0,
            screen: (4.0, 2.0),
        };
        let grid = density_grid(&tiles, &cam, 4, 2);
        // Pixel columns: x -64..-32, -32..0, 0..32, 32..64; rows: y 0..32, 32..64.
        let expected = [1.0, 0.5, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        for (i, (v, e)) in grid.iter().zip(expected).enumerate() {
            assert!((v - e).abs() <= 1e-6, "pixel {i}: {v} != {e}");
        }
    }
}
