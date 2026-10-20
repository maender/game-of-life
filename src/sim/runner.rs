use crate::sim::{CellRect, MetaMap, TileMap, Universe};
use arc_swap::ArcSwap;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

pub const SPEEDS: [Speed; 10] = [
    Speed::Gps(1),
    Speed::Gps(2),
    Speed::Gps(5),
    Speed::Gps(10),
    Speed::Gps(20),
    Speed::Gps(30),
    Speed::Gps(60),
    Speed::Gps(120),
    Speed::Gps(240),
    Speed::Max,
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Speed {
    Gps(u32),
    Max,
}

impl Speed {
    pub fn faster(self) -> Speed {
        match self {
            Speed::Max => Speed::Max,
            Speed::Gps(_) => {
                // Find current speed in SPEEDS and return the next one
                let idx = SPEEDS.iter().position(|&s| s == self);
                if let Some(idx) = idx {
                    if idx + 1 < SPEEDS.len() {
                        SPEEDS[idx + 1]
                    } else {
                        Speed::Max
                    }
                } else {
                    self
                }
            }
        }
    }

    pub fn slower(self) -> Speed {
        match self {
            Speed::Gps(_) => {
                // Find current speed in SPEEDS and return the previous one
                let idx = SPEEDS.iter().position(|&s| s == self);
                if let Some(idx) = idx {
                    if idx > 0 {
                        SPEEDS[idx - 1]
                    } else {
                        SPEEDS[0]
                    }
                } else {
                    self
                }
            }
            Speed::Max => {
                // Max goes to the last Gps speed
                SPEEDS[SPEEDS.len() - 2]
            }
        }
    }
}

pub enum SimCommand {
    Play,
    Pause,
    Step,
    SetSpeed(Speed),
    Edit(Vec<(i64, i64, bool)>),
    Randomize { rect: CellRect, density: f32 },
    Clear,
    SetEffects(bool),
    Shutdown,
}

pub struct Snapshot {
    pub tiles: Arc<TileMap>,
    pub prev: Arc<TileMap>,
    pub meta: Option<Arc<MetaMap>>,
    pub generation: u64,
    pub population: u64,
    pub running: bool,
    pub gens_per_sec: f32,
    pub published_at: Instant,
}

pub struct SimHandle {
    tx: Sender<SimCommand>,
    snapshot: Arc<ArcSwap<Snapshot>>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl SimHandle {
    pub fn send(&self, cmd: SimCommand) {
        let _ = self.tx.send(cmd);
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.snapshot.load_full()
    }
}

impl Drop for SimHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(SimCommand::Shutdown);
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn spawn(universe: Universe) -> SimHandle {
    let (tx, rx) = channel();
    let tiles = universe.tiles().clone();
    let snapshot_swap = Arc::new(ArcSwap::from_pointee(Snapshot {
        tiles: tiles.clone(),
        prev: tiles,
        meta: universe.meta().cloned(),
        generation: universe.generation(),
        population: universe.population(),
        running: false,
        gens_per_sec: 0.0,
        published_at: Instant::now(),
    }));

    let snapshot_clone = snapshot_swap.clone();

    let join_handle = thread::spawn(move || {
        run_simulation_loop(universe, rx, snapshot_clone);
    });

    SimHandle {
        tx,
        snapshot: snapshot_swap,
        join_handle: Some(join_handle),
    }
}

fn run_simulation_loop(
    mut universe: Universe,
    rx: Receiver<SimCommand>,
    snapshot: Arc<ArcSwap<Snapshot>>,
) {
    let mut running = false;
    let mut speed = Speed::Gps(30);
    let mut randomize_seed_counter: u64 = 0;

    // For gens_per_sec tracking: keep a rolling window of recent generation times
    let mut gen_times: Vec<Instant> = Vec::new();
    let gen_window = Duration::from_secs(1);

    loop {
        if running {
            // Running mode: drain commands, step, publish, sleep
            loop {
                match rx.try_recv() {
                    Ok(cmd) => {
                        match cmd {
                            SimCommand::Shutdown => return,
                            SimCommand::Pause => {
                                running = false;
                                publish_snapshot(
                                    &snapshot,
                                    &universe,
                                    false,
                                    &gen_times,
                                    gen_window,
                                );
                                break;
                            }
                            SimCommand::Play => {
                                // Already running, ignore
                            }
                            SimCommand::Step => {
                                // Ignored while running
                            }
                            SimCommand::SetSpeed(s) => {
                                speed = s;
                            }
                            SimCommand::Edit(edits) => {
                                apply_edits(&mut universe, edits);
                                publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                            }
                            SimCommand::Randomize { rect, density } => {
                                universe.fill_random(rect, density, randomize_seed_counter);
                                randomize_seed_counter += 1;
                                publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                            }
                            SimCommand::Clear => {
                                universe.clear();
                                publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                            }
                            SimCommand::SetEffects(on) => {
                                universe.set_effects(on);
                                publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                            }
                        }
                    }
                    Err(TryRecvError::Empty) => {
                        // No more commands, proceed with step
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        // Channel closed, shutdown
                        return;
                    }
                }
            }
            if !running {
                // Paused during the drain: no further generation.
                continue;
            }

            // Do a step
            let prev = universe.step();
            gen_times.push(Instant::now());

            // Publish snapshot with animation
            let new_snapshot = Snapshot {
                tiles: universe.tiles().clone(),
                prev,
                meta: universe.meta().cloned(),
                generation: universe.generation(),
                population: universe.population(),
                running: true,
                gens_per_sec: calculate_gens_per_sec(&gen_times, gen_window),
                published_at: Instant::now(),
            };
            snapshot.store(Arc::new(new_snapshot));

            // Clean old gen_times outside the window
            let cutoff = Instant::now() - gen_window;
            gen_times.retain(|&t| t > cutoff);

            // Wait according to speed, processing commands as they arrive
            match speed {
                Speed::Gps(gps) => {
                    let target_duration = Duration::from_secs_f64(1.0 / gps as f64);
                    let deadline = Instant::now() + target_duration;

                    // Wait until deadline, processing commands as they arrive
                    loop {
                        let now = Instant::now();
                        if now >= deadline {
                            break;
                        }

                        let timeout = deadline - now;
                        match rx.recv_timeout(timeout) {
                            Ok(cmd) => {
                                match cmd {
                                    SimCommand::Shutdown => return,
                                    SimCommand::Pause => {
                                        running = false;
                                        publish_snapshot(
                                            &snapshot,
                                            &universe,
                                            false,
                                            &gen_times,
                                            gen_window,
                                        );
                                        break;
                                    }
                                    SimCommand::Play => {
                                        // Already running, ignore
                                    }
                                    SimCommand::Step => {
                                        // Ignored while running
                                    }
                                    SimCommand::SetSpeed(s) => {
                                        speed = s;
                                        // Speed change takes effect on next step, keep waiting
                                    }
                                    SimCommand::Edit(edits) => {
                                        apply_edits(&mut universe, edits);
                                        publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                                        // Continue waiting after edit
                                    }
                                    SimCommand::Randomize { rect, density } => {
                                        universe.fill_random(rect, density, randomize_seed_counter);
                                        randomize_seed_counter += 1;
                                        publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                                        // Continue waiting
                                    }
                                    SimCommand::Clear => {
                                        universe.clear();
                                        publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                                        // Continue waiting
                                    }
                                    SimCommand::SetEffects(on) => {
                                        universe.set_effects(on);
                                        publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                                        // Continue waiting
                                    }
                                }
                            }
                            Err(_) => {
                                // Timeout reached or channel disconnected
                                break;
                            }
                        }
                    }
                }
                Speed::Max => {
                    // No sleep, run as fast as possible
                }
            }
        } else {
            // Paused mode: blocking recv
            match rx.recv() {
                Ok(cmd) => match cmd {
                    SimCommand::Shutdown => return,
                    SimCommand::Play => {
                        running = true;
                        gen_times.clear();
                        publish_snapshot(&snapshot, &universe, true, &gen_times, gen_window);
                    }
                    SimCommand::Pause => {
                        // Already paused, ignore
                    }
                    SimCommand::Step => {
                        let prev = universe.step();
                        gen_times.push(Instant::now());

                        let new_snapshot = Snapshot {
                            tiles: universe.tiles().clone(),
                            prev,
                            meta: universe.meta().cloned(),
                            generation: universe.generation(),
                            population: universe.population(),
                            running: false,
                            gens_per_sec: calculate_gens_per_sec(&gen_times, gen_window),
                            published_at: Instant::now(),
                        };
                        snapshot.store(Arc::new(new_snapshot));

                        let cutoff = Instant::now() - gen_window;
                        gen_times.retain(|&t| t > cutoff);
                    }
                    SimCommand::SetSpeed(s) => {
                        speed = s;
                    }
                    SimCommand::Edit(edits) => {
                        apply_edits(&mut universe, edits);
                        publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                    }
                    SimCommand::Randomize { rect, density } => {
                        universe.fill_random(rect, density, randomize_seed_counter);
                        randomize_seed_counter += 1;
                        publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                    }
                    SimCommand::Clear => {
                        universe.clear();
                        publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                    }
                    SimCommand::SetEffects(on) => {
                        universe.set_effects(on);
                        publish_snapshot(&snapshot, &universe, running, &gen_times, gen_window);
                    }
                },
                Err(_) => {
                    // Channel closed, shutdown
                    return;
                }
            }
        }
    }
}

fn apply_edits(universe: &mut Universe, edits: Vec<(i64, i64, bool)>) {
    for (x, y, alive) in edits {
        universe.set(x, y, alive);
    }
}

/// Publishes the current state without animation (prev == tiles), keeping
/// the measured generation rate.
fn publish_snapshot(
    snapshot: &Arc<ArcSwap<Snapshot>>,
    universe: &Universe,
    running: bool,
    gen_times: &[Instant],
    gen_window: Duration,
) {
    let tiles = universe.tiles().clone();
    let new_snapshot = Snapshot {
        tiles: tiles.clone(),
        prev: tiles,
        meta: universe.meta().cloned(),
        generation: universe.generation(),
        population: universe.population(),
        running,
        gens_per_sec: calculate_gens_per_sec(gen_times, gen_window),
        published_at: Instant::now(),
    };
    snapshot.store(Arc::new(new_snapshot));
}

fn calculate_gens_per_sec(gen_times: &[Instant], window: Duration) -> f32 {
    if gen_times.is_empty() {
        return 0.0;
    }

    let cutoff = Instant::now() - window;
    let recent_count = gen_times.iter().filter(|&&t| t > cutoff).count();

    if recent_count == 0 {
        0.0
    } else {
        // Average over the window
        recent_count as f32 / window.as_secs_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;

    fn wait_for_generation(handle: &SimHandle, target_gen: u64, timeout: Duration) -> bool {
        let start = Instant::now();
        loop {
            if handle.snapshot().generation >= target_gen {
                return true;
            }
            if start.elapsed() > timeout {
                return false;
            }
            sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn step_in_pause_advances_one() {
        let universe = Universe::new();
        let handle = spawn(universe);

        handle.send(SimCommand::Step);
        assert!(
            wait_for_generation(&handle, 1, Duration::from_secs(2)),
            "Failed to reach generation 1"
        );

        handle.send(SimCommand::Step);
        assert!(
            wait_for_generation(&handle, 2, Duration::from_secs(2)),
            "Failed to reach generation 2"
        );

        handle.send(SimCommand::Step);
        assert!(
            wait_for_generation(&handle, 3, Duration::from_secs(2)),
            "Failed to reach generation 3"
        );

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.generation, 3);
        assert!(!snapshot.running);
    }

    #[test]
    fn step_ignored_while_running() {
        let universe = Universe::new();
        let handle = spawn(universe);

        handle.send(SimCommand::SetSpeed(Speed::Gps(1)));
        handle.send(SimCommand::Play);

        sleep(Duration::from_millis(100));

        // Send 5 Step commands while running
        for _ in 0..5 {
            handle.send(SimCommand::Step);
        }

        handle.send(SimCommand::Pause);

        // Wait a bit for pause to take effect
        sleep(Duration::from_millis(100));

        let snapshot = handle.snapshot();
        assert!(
            snapshot.generation <= 2,
            "Generation {} exceeds 2 (at 1 GPS, ~0.2s total elapsed time with timing slack)",
            snapshot.generation
        );
    }

    #[test]
    fn edits_while_running_are_kept() {
        let universe = Universe::new();
        let handle = spawn(universe);

        handle.send(SimCommand::SetSpeed(Speed::Max));
        handle.send(SimCommand::Play);

        // Send edit for a 2x2 block far from origin
        let edits = vec![
            (1000, 1000, true),
            (1001, 1000, true),
            (1000, 1001, true),
            (1001, 1001, true),
        ];
        handle.send(SimCommand::Edit(edits));

        sleep(Duration::from_millis(200));

        handle.send(SimCommand::Pause);
        sleep(Duration::from_millis(50));

        let snapshot = handle.snapshot();
        let tiles = &snapshot.tiles;

        // Check all 4 cells are alive
        let (tc, lx0, ly0) = crate::sim::split(1000, 1000);
        let tile = tiles.get(&tc).expect("Tile should exist");
        assert!(tile.get(lx0, ly0), "Cell (1000, 1000) should be alive");

        let (tc, lx1, ly1) = crate::sim::split(1001, 1000);
        let tile = tiles.get(&tc).expect("Tile should exist");
        assert!(tile.get(lx1, ly1), "Cell (1001, 1000) should be alive");

        let (tc, lx2, ly2) = crate::sim::split(1000, 1001);
        let tile = tiles.get(&tc).expect("Tile should exist");
        assert!(tile.get(lx2, ly2), "Cell (1000, 1001) should be alive");

        let (tc, lx3, ly3) = crate::sim::split(1001, 1001);
        let tile = tiles.get(&tc).expect("Tile should exist");
        assert!(tile.get(lx3, ly3), "Cell (1001, 1001) should be alive");
    }

    #[test]
    fn speed_bounds() {
        assert_eq!(Speed::Max.faster(), Speed::Max);
        assert_eq!(Speed::Gps(1).slower(), Speed::Gps(1));
        assert_eq!(Speed::Gps(30).faster(), Speed::Gps(60));
    }

    #[test]
    fn prev_is_previous_generation() {
        let mut universe = Universe::new();

        // Create a blinker (oscillator with period 2)
        // Horizontal: .O. -> Vertical: ...
        //             .O.              OOO
        //             .O.              ...
        universe.set(1, 0, true);
        universe.set(1, 1, true);
        universe.set(1, 2, true);

        let handle = spawn(universe);

        handle.send(SimCommand::Step);
        assert!(
            wait_for_generation(&handle, 1, Duration::from_secs(2)),
            "Failed to reach generation 1"
        );

        let snapshot = handle.snapshot();

        // prev should contain the horizontal state (1,0), (1,1), (1,2)
        let (tc, lx, ly0) = crate::sim::split(1, 0);
        let prev_tile = snapshot.prev.get(&tc).expect("Prev tile should exist");
        assert!(prev_tile.get(lx, ly0), "Prev should have cell (1,0)");

        let (_, _, ly1) = crate::sim::split(1, 1);
        assert!(prev_tile.get(lx, ly1), "Prev should have cell (1,1)");

        let (_, _, ly2) = crate::sim::split(1, 2);
        assert!(prev_tile.get(lx, ly2), "Prev should have cell (1,2)");

        // tiles should contain the vertical state (0,1), (1,1), (2,1)
        let tiles_tile = snapshot.tiles.get(&tc).expect("Tiles tile should exist");
        let (_, lx0, ly) = crate::sim::split(0, 1);
        assert!(tiles_tile.get(lx0, ly), "Tiles should have cell (0,1)");

        let (_, lx1, _) = crate::sim::split(1, 1);
        assert!(tiles_tile.get(lx1, ly), "Tiles should have cell (1,1)");

        let (_, lx2, _) = crate::sim::split(2, 1);
        assert!(tiles_tile.get(lx2, ly), "Tiles should have cell (2,1)");
    }

    #[test]
    fn edit_responsive_at_slow_speed() {
        let universe = Universe::new();
        let handle = spawn(universe);

        handle.send(SimCommand::SetSpeed(Speed::Gps(1)));
        handle.send(SimCommand::Play);

        sleep(Duration::from_millis(100));

        // Send edit for a 2x2 block
        let edits = vec![
            (500, 500, true),
            (501, 500, true),
            (500, 501, true),
            (501, 501, true),
        ];
        handle.send(SimCommand::Edit(edits));

        // Edit should be applied within 0.3s total (not waiting for full 1s sleep)
        let timeout = Duration::from_millis(300);
        let start = Instant::now();
        loop {
            let snapshot = handle.snapshot();
            let (tc, lx0, ly0) = crate::sim::split(500, 500);
            if let Some(tile) = snapshot.tiles.get(&tc) {
                if tile.get(lx0, ly0) {
                    // Edit applied successfully
                    let response_time = start.elapsed();
                    assert!(
                        response_time < timeout,
                        "Edit took {}ms to apply, should be < 300ms",
                        response_time.as_millis()
                    );
                    break;
                }
            }

            if start.elapsed() > timeout {
                panic!("Edit not applied within 300ms at Gps(1)");
            }

            sleep(Duration::from_millis(10));
        }

        handle.send(SimCommand::Pause);
    }

    /// At Max speed Pause lands in the command drain: it must not run one more
    /// generation nor leave a `running: true` snapshot behind.
    #[test]
    fn pause_at_max_speed_stops() {
        let handle = spawn(Universe::new());
        handle.send(SimCommand::SetSpeed(Speed::Max));
        handle.send(SimCommand::Play);
        assert!(wait_for_generation(&handle, 10, Duration::from_secs(2)));

        handle.send(SimCommand::Pause);
        let start = Instant::now();
        while handle.snapshot().running {
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "snapshot still running 2 s after Pause"
            );
            sleep(Duration::from_millis(10));
        }
        let paused_at = handle.snapshot().generation;
        sleep(Duration::from_millis(100));
        let snap = handle.snapshot();
        assert!(!snap.running);
        assert_eq!(snap.generation, paused_at, "generation moved after Pause");
    }

    /// An edit while running keeps the measured rate in its snapshot. A lone
    /// cell dies on the next step, so seeing it means seeing the edit snapshot.
    #[test]
    fn edit_snapshot_keeps_gens_per_sec() {
        let handle = spawn(Universe::new());
        // At 1 gen/s the edit lands ~0.7 s before the step that kills it.
        handle.send(SimCommand::SetSpeed(Speed::Gps(1)));
        handle.send(SimCommand::Play);
        sleep(Duration::from_millis(1300));

        handle.send(SimCommand::Edit(vec![(700, 700, true)]));
        let (tc, lx, ly) = crate::sim::split(700, 700);
        let start = Instant::now();
        let snap = loop {
            let snap = handle.snapshot();
            if snap.tiles.get(&tc).is_some_and(|t| t.get(lx, ly)) {
                break snap;
            }
            assert!(start.elapsed() < Duration::from_secs(2), "edit not seen");
            thread::yield_now();
        };
        assert!(snap.gens_per_sec > 0.0, "edit snapshot reset gens_per_sec");
    }

    #[test]
    fn pause_responsive_at_slow_speed() {
        let universe = Universe::new();
        let handle = spawn(universe);

        handle.send(SimCommand::SetSpeed(Speed::Gps(1)));
        handle.send(SimCommand::Play);

        sleep(Duration::from_millis(100));

        handle.send(SimCommand::Pause);

        // Pause should take effect within 0.3s (not waiting for full 1s sleep)
        let timeout = Duration::from_millis(300);
        let start = Instant::now();
        loop {
            let snapshot = handle.snapshot();
            if !snapshot.running {
                // Pause took effect
                let response_time = start.elapsed();
                assert!(
                    response_time < timeout,
                    "Pause took {}ms to take effect, should be < 300ms",
                    response_time.as_millis()
                );
                break;
            }

            if start.elapsed() > timeout {
                panic!("Pause did not take effect within 300ms at Gps(1)");
            }

            sleep(Duration::from_millis(10));
        }

        let snapshot = handle.snapshot();
        assert!(!snapshot.running);
    }
}
