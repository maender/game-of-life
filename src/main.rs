//! Provisional viewer (replaced by `app::run` in Task 9): a 512x512 soup stepped
//! on the main thread every 350 ms, Escape quits, Space pauses, wheel zooms,
//! left drag pans, digits 1-4 pick the style, 0 toggles degraded mode,
//! 5 / 6 / 7 toggle the age / trail / animation effects.
//! Env: `GOL_FAR=1` places soup and camera around (1e12, 1e12);
//! `GOL_FRAMES=n` quits after n frames; `GOL_EFFECTS=1` starts with all
//! effects on; `GOL_PPC=x` sets the initial pixels per cell.

use gol::render::Renderer;
use gol::sim::{CellRect, Snapshot, Universe};
use gol::ui::{CellStyle, Settings};
use gol::view::Camera;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

const PERIOD: Duration = Duration::from_millis(350);

struct Viewer {
    universe: Universe,
    running: bool,
    snap: Snapshot,
    cam: Camera,
    settings: Settings,
    renderer: Option<Renderer>,
    window: Option<Arc<Window>>,
    cursor: (f64, f64),
    dragging: bool,
    frames_left: Option<u64>,
}

impl ApplicationHandler for Viewer {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Game of Life")
            .with_fullscreen(Some(Fullscreen::Borderless(None)));
        let window = Arc::new(el.create_window(attrs).expect("failed to create window"));
        let size = window.inner_size();
        self.cam.screen = (size.width as f64, size.height as f64);
        self.renderer = Some(Renderer::new(window.clone()));
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                self.cam.screen = (size.width as f64, size.height as f64);
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => el.exit(),
                    PhysicalKey::Code(KeyCode::Digit1) => self.settings.style = CellStyle::ALL[0],
                    PhysicalKey::Code(KeyCode::Digit2) => self.settings.style = CellStyle::ALL[1],
                    PhysicalKey::Code(KeyCode::Digit3) => self.settings.style = CellStyle::ALL[2],
                    PhysicalKey::Code(KeyCode::Digit4) => self.settings.style = CellStyle::ALL[3],
                    PhysicalKey::Code(KeyCode::Digit0) => {
                        self.settings.degraded = !self.settings.degraded
                    }
                    PhysicalKey::Code(KeyCode::Digit5) => {
                        self.settings.effect_age = !self.settings.effect_age
                    }
                    PhysicalKey::Code(KeyCode::Digit6) => {
                        self.settings.effect_trails = !self.settings.effect_trails
                    }
                    PhysicalKey::Code(KeyCode::Digit7) => {
                        self.settings.effect_anim = !self.settings.effect_anim
                    }
                    PhysicalKey::Code(KeyCode::Space) => self.running = !self.running,
                    _ => {}
                }
                self.universe.set_effects(self.settings.effects_needed());
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.dragging {
                    self.cam
                        .pan_px(position.x - self.cursor.0, position.y - self.cursor.1);
                }
                self.cursor = (position.x, position.y);
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging = state == ElementState::Pressed;
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64,
                    MouseScrollDelta::PixelDelta(p) => p.y / 40.0,
                };
                self.cam
                    .zoom_at(self.cursor.0, self.cursor.1, 1.1f64.powf(lines));
            }
            WindowEvent::RedrawRequested => {
                if self.running && self.snap.published_at.elapsed() >= PERIOD {
                    let prev = self.universe.step();
                    self.snap = snapshot(&self.universe, prev);
                }
                let anim_t = if self.running {
                    (self.snap.published_at.elapsed().as_secs_f32() / (0.6 * PERIOD.as_secs_f32()))
                        .min(1.0)
                } else {
                    1.0
                };
                if let Some(r) = &mut self.renderer {
                    r.render(&self.cam, &self.snap, &self.settings, anim_t, None);
                }
                if let Some(n) = &mut self.frames_left {
                    *n = n.saturating_sub(1);
                    if *n == 0 {
                        el.exit();
                    }
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let origin: i64 = if std::env::var_os("GOL_FAR").is_some() {
        1_000_000_000_000
    } else {
        0
    };
    let mut universe = Universe::new();
    universe.fill_random(
        CellRect {
            x0: origin - 256,
            y0: origin - 256,
            x1: origin + 256,
            y1: origin + 256,
        },
        0.35,
        1,
    );
    let mut settings = Settings::default();
    if std::env::var_os("GOL_EFFECTS").is_some() {
        settings.effect_age = true;
        settings.effect_trails = true;
        settings.effect_anim = true;
    }
    universe.set_effects(settings.effects_needed());
    let prev = universe.tiles().clone();
    let snap = snapshot(&universe, prev);
    let mut viewer = Viewer {
        universe,
        running: true,
        snap,
        cam: Camera {
            center: (origin as f64, origin as f64),
            px_per_cell: std::env::var("GOL_PPC")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(8.0),
            screen: (1.0, 1.0),
        },
        settings,
        renderer: None,
        window: None,
        cursor: (0.0, 0.0),
        dragging: false,
        frames_left: std::env::var("GOL_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok()),
    };
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut viewer).expect("event loop failed");
}

fn snapshot(universe: &Universe, prev: Arc<gol::sim::TileMap>) -> Snapshot {
    Snapshot {
        tiles: universe.tiles().clone(),
        prev,
        meta: universe.meta().cloned(),
        generation: universe.generation(),
        population: universe.population(),
        running: true,
        gens_per_sec: 0.0,
        published_at: Instant::now(),
    }
}
