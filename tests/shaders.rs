use wgpu::naga;
use wgpu::naga::valid::{Capabilities, ValidationFlags, Validator};

fn validate(src: &str) {
    let module =
        naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
}

#[test]
fn tiles_wgsl_is_valid() {
    validate(include_str!("../src/render/tiles.wgsl"));
}

#[test]
fn density_wgsl_is_valid() {
    validate(include_str!("../src/render/density.wgsl"));
}

#[test]
fn grid_wgsl_is_valid() {
    validate(include_str!("../src/render/grid.wgsl"));
}
