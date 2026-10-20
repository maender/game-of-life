//! Soup benchmark: 4096x4096 random soup at density 0.35, 10 warm-up
//! generations then 200 timed ones, with effects off and then on. Then the
//! far-zoom density grid of a fresh soup (the renderer's CPU work per new
//! snapshot below 0.5 px per cell), at full and half resolution.
//!
//! Run with `cargo run --release --example bench_soup`.

use gol::render::density_grid;
use gol::sim::{CellRect, Universe};
use gol::view::Camera;
use std::time::Instant;

const WARMUP: u32 = 10;
const TIMED: u32 = 200;

const SOUP: CellRect = CellRect {
    x0: 0,
    y0: 0,
    x1: 4096,
    y1: 4096,
};
const DENSITY_RUNS: u32 = 20;

fn soup(effects: bool) -> Universe {
    let mut universe = Universe::new();
    universe.set_effects(effects);
    universe.fill_random(SOUP, 0.35, 1);
    universe
}

fn run(effects: bool) {
    let mut universe = soup(effects);
    let initial = universe.population();
    for _ in 0..WARMUP {
        universe.step();
    }
    let start = Instant::now();
    for _ in 0..TIMED {
        universe.step();
    }
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "effects {}: initial population {}, {:.1} gen/s ({:.2} ms/gen), final population {}, tiles {}",
        if effects { "on " } else { "off" },
        initial,
        TIMED as f64 / elapsed,
        elapsed * 1000.0 / TIMED as f64,
        universe.population(),
        universe.tile_count(),
    );
}

/// Density grid of the whole soup on a 3024x1964 (MacBook Pro 14") screen.
fn density() {
    let universe = soup(false);
    let cam = Camera {
        center: (2048.0, 2048.0),
        px_per_cell: 0.45,
        screen: (3024.0, 1964.0),
    };
    for div in [1, 2] {
        let (w, h) = (cam.screen.0 as u32 / div, cam.screen.1 as u32 / div);
        let start = Instant::now();
        for _ in 0..DENSITY_RUNS {
            std::hint::black_box(density_grid(universe.tiles(), &cam, w, h));
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / DENSITY_RUNS as f64;
        println!(
            "density grid {}x{} of {} live cells: {:.2} ms",
            w,
            h,
            universe.population(),
            ms
        );
    }
}

fn main() {
    run(false);
    run(true);
    density();
}
