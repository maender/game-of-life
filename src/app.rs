//! The application: window, event handling, simulation thread and panel.
//! Env: `GOL_FRAMES=n` quits after n frames (non-interactive startup check).

use crate::render::Renderer;
use crate::sim::{bounding_box, spawn, SimCommand, SimHandle, Snapshot, Speed, Universe};
use crate::ui::{action_for_key, panel, Action, Settings};
use crate::view::{line_cells, randomize_rect, Camera};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

const START_PX_PER_CELL: f64 = 8.0;
const START_SPEED: Speed = Speed::Gps(30);
const NOTICE_CAPPED: &str = "Zone limitée à 268 M cellules";
const NOTICE_DURATION: Duration = Duration::from_secs(3);
/// Below one pixel per cell a stroke would cover huge areas: drawing is off.
const MIN_DRAW_PX_PER_CELL: f64 = 1.0;

/// Window-dependent state, created on `resumed`.
struct Gfx {
    window: Arc<Window>,
    renderer: Renderer,
    egui: egui_winit::State,
}

struct App {
    gfx: Option<Gfx>,
    sim: Option<SimHandle>,
    snap: Option<Arc<Snapshot>>,
    cam: Camera,
    settings: Settings,
    effects_sent: bool,
    running: bool,
    speed: Speed,
    cursor: (f64, f64),
    /// Last cell of the stroke in progress (left button held).
    stroke: Option<(i64, i64)>,
    panning: bool,
    notice_until: Option<Instant>,
    fps: f32,
    fps_frames: u32,
    fps_since: Instant,
    frames_left: Option<u64>,
}

pub fn run() {
    let mut app = App {
        gfx: None,
        sim: None,
        snap: None,
        cam: Camera {
            center: (0.0, 0.0),
            px_per_cell: START_PX_PER_CELL,
            screen: (1.0, 1.0),
        },
        settings: Settings::default(),
        effects_sent: false,
        running: false,
        speed: START_SPEED,
        cursor: (0.0, 0.0),
        stroke: None,
        panning: false,
        notice_until: None,
        fps: 0.0,
        fps_frames: 0,
        fps_since: Instant::now(),
        frames_left: std::env::var("GOL_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok()),
    };
    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(&mut app).expect("event loop failed");
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Game of Life")
            .with_fullscreen(Some(Fullscreen::Borderless(None)));
        let window = Arc::new(el.create_window(attrs).expect("failed to create window"));

        // The fullscreen transition may not be finished yet: size the starting
        // soup from the monitor, the camera from the actual surface.
        let size = window.inner_size();
        self.cam.screen = (size.width as f64, size.height as f64);
        let screen = window
            .current_monitor()
            .map(|m| m.size())
            .filter(|s| s.width > 0 && s.height > 0)
            .unwrap_or(size);
        let soup_cam = Camera {
            screen: (screen.width as f64, screen.height as f64),
            ..self.cam
        };
        let seed = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        let mut universe = Universe::new();
        universe.fill_random(soup_cam.visible_rect(), self.settings.density, seed);
        let sim = spawn(universe);
        sim.send(SimCommand::SetSpeed(self.speed));
        self.snap = Some(sim.snapshot());
        self.sim = Some(sim);

        let ctx = egui::Context::default();
        let egui = egui_winit::State::new(
            ctx,
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let renderer = Renderer::new(window.clone());
        window.request_redraw();
        self.gfx = Some(Gfx {
            window,
            renderer,
            egui,
        });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(gfx) = &mut self.gfx else { return };

        // Shortcuts go to the app unless egui is editing text; they are not
        // forwarded to egui (Tab would move its focus, Space press a button).
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            if !gfx.egui.egui_ctx().wants_keyboard_input() {
                if let Some(action) = action_for_key(&key.logical_key, key.physical_key) {
                    let repeatable =
                        matches!(action, Action::Step | Action::Faster | Action::Slower);
                    if key.state == ElementState::Pressed && (!key.repeat || repeatable) {
                        self.apply(action, el);
                    }
                    return;
                }
            }
        }

        let consumed = gfx.egui.on_window_event(&gfx.window, &event).consumed;
        let ui_pointer = consumed || gfx.egui.egui_ctx().wants_pointer_input();

        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                let size = gfx.window.inner_size();
                self.cam.screen = (size.width as f64, size.height as f64);
                gfx.renderer.resize(size.width, size.height);
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = (position.x, position.y);
                if self.panning {
                    self.cam.pan_px(x - self.cursor.0, y - self.cursor.1);
                }
                self.cursor = (x, y);
                if let Some(last) = self.stroke {
                    let cell = self.cam.cell_at(x, y);
                    if cell != last {
                        self.paint(line_cells(last, cell));
                        self.stroke = Some(cell);
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left if pressed && !ui_pointer => {
                        let cell = self.cam.cell_at(self.cursor.0, self.cursor.1);
                        self.stroke = Some(cell);
                        self.paint(vec![cell]);
                    }
                    MouseButton::Left if !pressed => self.stroke = None,
                    MouseButton::Right => self.panning = pressed && !ui_pointer,
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } if !ui_pointer => match delta {
                MouseScrollDelta::LineDelta(_, lines) => {
                    self.cam
                        .zoom_at(self.cursor.0, self.cursor.1, 1.1f64.powf(lines as f64));
                }
                MouseScrollDelta::PixelDelta(p) => self.cam.pan_px(p.x, p.y),
            },
            WindowEvent::PinchGesture { delta, .. } if !ui_pointer => {
                self.cam.zoom_at(self.cursor.0, self.cursor.1, 1.0 + delta);
            }
            WindowEvent::RedrawRequested => self.frame(el),
            _ => {}
        }
    }
}

impl App {
    fn sim(&self) -> &SimHandle {
        self.sim.as_ref().expect("simulation not started")
    }

    /// Sets the given cells to the current tool (draw or erase).
    fn paint(&self, cells: Vec<(i64, i64)>) {
        if self.cam.px_per_cell < MIN_DRAW_PX_PER_CELL {
            return;
        }
        let alive = !self.settings.tool_erase;
        let edits = cells.into_iter().map(|(x, y)| (x, y, alive)).collect();
        self.sim().send(SimCommand::Edit(edits));
    }

    fn apply(&mut self, action: Action, el: &ActiveEventLoop) {
        let s = &mut self.settings;
        match action {
            Action::TogglePlay => {
                self.running = !self.running;
                let cmd = if self.running {
                    SimCommand::Play
                } else {
                    SimCommand::Pause
                };
                self.sim().send(cmd);
            }
            Action::Step => {
                if !self.running {
                    self.sim().send(SimCommand::Step);
                }
            }
            Action::Faster | Action::Slower => {
                self.speed = if action == Action::Faster {
                    self.speed.faster()
                } else {
                    self.speed.slower()
                };
                self.sim().send(SimCommand::SetSpeed(self.speed));
            }
            Action::Randomize => {
                let (rect, capped) = randomize_rect(&self.cam);
                let density = s.density;
                self.sim().send(SimCommand::Randomize { rect, density });
                if capped {
                    self.notice_until = Some(Instant::now() + NOTICE_DURATION);
                }
            }
            Action::Clear => self.sim().send(SimCommand::Clear),
            Action::ToolDraw => s.tool_erase = false,
            Action::ToolErase => s.tool_erase = true,
            Action::Recenter => {
                if let Some(snap) = &self.snap {
                    self.cam.fit(bounding_box(&snap.tiles));
                }
            }
            Action::NextStyle => s.style = s.style.next(),
            Action::ToggleAge => s.effect_age = !s.effect_age,
            Action::ToggleTrails => s.effect_trails = !s.effect_trails,
            Action::ToggleAnim => s.effect_anim = !s.effect_anim,
            Action::ToggleDegraded => s.degraded = !s.degraded,
            Action::TogglePanel => s.panel_open = !s.panel_open,
            Action::ToggleFullscreen => {
                if let Some(gfx) = &self.gfx {
                    let fs = match gfx.window.fullscreen() {
                        Some(_) => None,
                        None => Some(Fullscreen::Borderless(None)),
                    };
                    gfx.window.set_fullscreen(fs);
                }
            }
            Action::Quit => el.exit(),
        }
        let effects = self.settings.effects_needed();
        if effects != self.effects_sent {
            self.effects_sent = effects;
            self.sim().send(SimCommand::SetEffects(effects));
        }
    }

    fn frame(&mut self, el: &ActiveEventLoop) {
        let snap = self.sim().snapshot();
        self.snap = Some(snap.clone());

        self.fps_frames += 1;
        let elapsed = self.fps_since.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            self.fps = self.fps_frames as f32 / elapsed;
            self.fps_frames = 0;
            self.fps_since = Instant::now();
        }

        let notice = self
            .notice_until
            .filter(|&t| Instant::now() < t)
            .map(|_| NOTICE_CAPPED);
        let Some(gfx) = &mut self.gfx else { return };
        let input = gfx.egui.take_egui_input(&gfx.window);
        let ctx = gfx.egui.egui_ctx().clone();
        let mut actions = Vec::new();
        let mut output = ctx.run(input, |ctx| {
            actions = panel::draw(
                ctx,
                &mut self.settings,
                &snap,
                self.speed,
                self.fps,
                &self.cam,
                notice,
            );
        });
        let platform = std::mem::take(&mut output.platform_output);
        gfx.egui.handle_platform_output(&gfx.window, platform);
        for action in actions {
            self.apply(action, el);
        }

        let anim_t = match self.speed {
            Speed::Gps(gps) if snap.running => {
                let period = 1.0 / gps as f32;
                (snap.published_at.elapsed().as_secs_f32() / (0.6 * period)).min(1.0)
            }
            _ => 1.0,
        };
        let Some(gfx) = &mut self.gfx else { return };
        gfx.renderer.render(
            &self.cam,
            &snap,
            &self.settings,
            anim_t,
            Some((&ctx, output)),
        );

        if let Some(n) = &mut self.frames_left {
            *n = n.saturating_sub(1);
            if *n == 0 {
                el.exit();
            }
        }
        gfx.window.request_redraw();
    }
}
