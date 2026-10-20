//! Optional grid overlay: one-pixel light gray lines on cell boundaries, drawn
//! full screen under the cells and faded out when cells get too small.

use super::srgb_to_linear;
use crate::view::Camera;
use bytemuck::{Pod, Zeroable};

/// Line color `#c9d1d9` (sRGB).
const GRID_RGB: [f32; 3] = [
    0xc9 as f32 / 255.0,
    0xd1 as f32 / 255.0,
    0xd9 as f32 / 255.0,
];
/// Line opacity once fully faded in.
pub const GRID_ALPHA: f32 = 0.11;
/// The grid starts fading in at this zoom...
const FADE_START: f64 = 4.0;
/// ...and is fully visible from this one.
const FADE_END: f64 = 8.0;

/// Line opacity at a zoom level: 0 up to `FADE_START` px/cell, `GRID_ALPHA`
/// from `FADE_END`, linear in between.
pub fn grid_alpha(px_per_cell: f64) -> f32 {
    let t = ((px_per_cell - FADE_START) / (FADE_END - FADE_START)).clamp(0.0, 1.0);
    GRID_ALPHA * t as f32
}

/// Screen position (physical pixels from the top-left corner) of one vertical
/// and one horizontal cell boundary, each in `[0, px_per_cell)`. The fraction
/// is taken in f64 so lines stay exact far from the origin.
pub fn grid_phase(cam: &Camera) -> [f32; 2] {
    let ppc = cam.px_per_cell;
    let phase = |center: f64, screen: f64| {
        let edge = center - screen / 2.0 / ppc;
        ((edge.ceil() - edge) * ppc) as f32
    };
    [
        phase(cam.center.0, cam.screen.0),
        phase(cam.center.1, cam.screen.1),
    ]
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    rgb: [f32; 3],
    alpha: f32,
    phase: [f32; 2],
    px_per_cell: f32,
    _pad: f32,
}

pub(super) struct GridView {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    visible: bool,
}

impl GridView {
    pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid"),
            source: wgpu::ShaderSource::Wgsl(include_str!("grid.wgsl").into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid"),
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
            label: Some("grid uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        GridView {
            pipeline,
            uniforms,
            bind_group,
            visible: false,
        }
    }

    /// Updates the uniforms; the grid is drawn only if `enabled` and the zoom
    /// leaves it some opacity.
    pub(super) fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        cam: &Camera,
        enabled: bool,
        srgb_target: bool,
    ) {
        let alpha = grid_alpha(cam.px_per_cell);
        self.visible = enabled && alpha > 0.0;
        if !self.visible {
            return;
        }
        let rgb = if srgb_target {
            GRID_RGB.map(|c| srgb_to_linear(c as f64) as f32)
        } else {
            GRID_RGB
        };
        let u = Uniforms {
            rgb,
            alpha,
            phase: grid_phase(cam),
            px_per_cell: cam.px_per_cell as f32,
            _pad: 0.0,
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'static>) {
        if !self.visible {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(center: (f64, f64)) -> Camera {
        Camera {
            center,
            px_per_cell: 10.0,
            screen: (100.0, 100.0),
        }
    }

    #[test]
    fn grid_alpha_fades_in_between_4_and_8_px() {
        assert_eq!(grid_alpha(1.0), 0.0);
        assert_eq!(grid_alpha(4.0), 0.0);
        let mid = grid_alpha(6.0);
        assert!(mid > 0.0 && mid < GRID_ALPHA, "{mid}");
        assert_eq!(grid_alpha(8.0), GRID_ALPHA);
        assert_eq!(grid_alpha(128.0), GRID_ALPHA);
    }

    #[test]
    fn grid_phase_puts_lines_on_cell_boundaries() {
        // Left edge at cell x = 0.25 - 5 = -4.75: the boundary x = -4 is 0.75
        // cell = 7.5 px in. Top edge at y = -5 exactly: a boundary at 0 px.
        assert_eq!(grid_phase(&cam((0.25, 0.0))), [7.5, 0.0]);
    }

    #[test]
    fn grid_phase_is_exact_far_from_origin() {
        assert_eq!(grid_phase(&cam((1e12 + 0.25, -3e11))), [7.5, 0.0]);
    }
}
