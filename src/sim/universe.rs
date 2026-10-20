use crate::sim::{split, step_tile, Neighborhood, Tile, TileCoord, TILE};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::sync::Arc;

pub type TileMap = FxHashMap<TileCoord, Tile>;

struct TileRng {
    state: u64,
}

impl TileRng {
    fn new(seed: u64, tx: i64, ty: i64) -> Self {
        // Combine seed with tile coordinates
        let mut state = seed;
        state ^= (tx as u64).wrapping_mul(0x517cc1b727220a95);
        state ^= (ty as u64).wrapping_mul(0x6b51d35be0a3db87);
        state = state.wrapping_add(0x9e3779b97f4a7c15);
        TileRng { state }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellRect {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
}

impl CellRect {
    pub fn area(&self) -> u128 {
        let width = (self.x1 - self.x0) as u128;
        let height = (self.y1 - self.y0) as u128;
        width * height
    }
}

pub struct Universe {
    tiles: Arc<TileMap>,
    generation: u64,
    population: u64,
}

impl Universe {
    pub fn new() -> Self {
        Universe {
            tiles: Arc::new(TileMap::default()),
            generation: 0,
            population: 0,
        }
    }

    pub fn get(&self, x: i64, y: i64) -> bool {
        let (tile_coord, lx, ly) = split(x, y);
        self.tiles
            .get(&tile_coord)
            .map_or(false, |tile| tile.get(lx, ly))
    }

    pub fn set(&mut self, x: i64, y: i64, alive: bool) {
        let (tile_coord, lx, ly) = split(x, y);
        let tiles = Arc::make_mut(&mut self.tiles);

        let was_alive = tiles
            .get(&tile_coord)
            .map_or(false, |tile| tile.get(lx, ly));

        if alive == was_alive {
            return;
        }

        if alive {
            let tile = tiles.entry(tile_coord).or_insert_with(Tile::default);
            tile.set(lx, ly, true);
            self.population += 1;
        } else {
            if let Some(tile) = tiles.get_mut(&tile_coord) {
                tile.set(lx, ly, false);
                self.population -= 1;
                if tile.is_empty() {
                    tiles.remove(&tile_coord);
                }
            }
        }
    }

    pub fn step(&mut self) -> Arc<TileMap> {
        let old_tiles = self.tiles.clone();

        // Find candidate tiles: existing tiles + neighbors on borders with live cells
        let mut candidates = std::collections::HashSet::new();

        for (&(tx, ty), tile) in old_tiles.iter() {
            candidates.insert((tx, ty));

            let border = tile.border();
            if border.n {
                candidates.insert((tx, ty - 1));
            }
            if border.s {
                candidates.insert((tx, ty + 1));
            }
            if border.w {
                candidates.insert((tx - 1, ty));
            }
            if border.e {
                candidates.insert((tx + 1, ty));
            }
            // Corners: conservative approach using adjacent borders
            if border.n && border.w {
                candidates.insert((tx - 1, ty - 1));
            }
            if border.n && border.e {
                candidates.insert((tx + 1, ty - 1));
            }
            if border.s && border.w {
                candidates.insert((tx - 1, ty + 1));
            }
            if border.s && border.e {
                candidates.insert((tx + 1, ty + 1));
            }
        }

        // Compute next generation in parallel
        let empty_tile = Tile::default();
        let new_tiles: TileMap = candidates
            .par_iter()
            .filter_map(|&(tx, ty)| {
                let c = old_tiles.get(&(tx, ty)).unwrap_or(&empty_tile);
                let n = old_tiles.get(&(tx, ty - 1)).unwrap_or(&empty_tile);
                let s = old_tiles.get(&(tx, ty + 1)).unwrap_or(&empty_tile);
                let w = old_tiles.get(&(tx - 1, ty)).unwrap_or(&empty_tile);
                let e = old_tiles.get(&(tx + 1, ty)).unwrap_or(&empty_tile);
                let nw = old_tiles.get(&(tx - 1, ty - 1)).unwrap_or(&empty_tile);
                let ne = old_tiles.get(&(tx + 1, ty - 1)).unwrap_or(&empty_tile);
                let sw = old_tiles.get(&(tx - 1, ty + 1)).unwrap_or(&empty_tile);
                let se = old_tiles.get(&(tx + 1, ty + 1)).unwrap_or(&empty_tile);

                let nb = Neighborhood {
                    c,
                    n: Some(n),
                    s: Some(s),
                    w: Some(w),
                    e: Some(e),
                    nw: Some(nw),
                    ne: Some(ne),
                    sw: Some(sw),
                    se: Some(se),
                };

                let next_tile = step_tile(&nb);
                if !next_tile.is_empty() {
                    Some(((tx, ty), next_tile))
                } else {
                    None
                }
            })
            .collect();

        // Update population
        self.population = new_tiles.values().map(|tile| tile.population() as u64).sum();
        self.generation += 1;
        self.tiles = Arc::new(new_tiles);

        old_tiles
    }

    pub fn tiles(&self) -> &Arc<TileMap> {
        &self.tiles
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn population(&self) -> u64 {
        self.population
    }

    pub fn tile_count(&self) -> usize {
        self.tiles.len()
    }

    pub fn clear(&mut self) {
        self.tiles = Arc::new(TileMap::default());
        self.generation = 0;
        self.population = 0;
    }

    pub fn fill_random(&mut self, rect: CellRect, density: f32, seed: u64) {
        // Clear the rect first
        let tx0 = rect.x0.div_euclid(TILE);
        let ty0 = rect.y0.div_euclid(TILE);
        let tx1 = (rect.x1 - 1).div_euclid(TILE) + 1;
        let ty1 = (rect.y1 - 1).div_euclid(TILE) + 1;

        // Remove cells in the rect
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                if self.get(x, y) {
                    self.set(x, y, false);
                }
            }
        }

        // Fill in parallel by tile
        let density_byte = (density * 256.0).round() as u32;

        let new_tiles: Vec<(TileCoord, Tile)> = (ty0..ty1)
            .into_par_iter()
            .flat_map(|ty| {
                (tx0..tx1).into_par_iter().filter_map(move |tx| {
                    let mut tile = Tile::default();
                    let mut rng = TileRng::new(seed, tx, ty);

                    for ly in 0..64 {
                        let y = ty * TILE + ly as i64;
                        if y < rect.y0 || y >= rect.y1 {
                            continue;
                        }

                        // Generate mask for this row using density byte and 8 random words
                        // The approach: generate 8 random u64 words, and combine them
                        // based on the bits of density_byte to approximate the density
                        let words = [
                            rng.next(),
                            rng.next(),
                            rng.next(),
                            rng.next(),
                            rng.next(),
                            rng.next(),
                            rng.next(),
                            rng.next(),
                        ];

                        let mut row_mask = 0u64;
                        for lx in 0..64 {
                            // Generate a random byte and compare with density_byte
                            let word_idx = lx / 8;
                            let bit_in_word = (lx % 8) * 8;
                            let random_byte = ((words[word_idx] >> bit_in_word) & 0xFF) as u32;

                            if random_byte < density_byte {
                                row_mask |= 1u64 << lx;
                            }
                        }

                        // Mask out cells outside the x range
                        let x0_in_tile = if tx * TILE >= rect.x0 {
                            0
                        } else {
                            (rect.x0 - tx * TILE) as usize
                        };
                        let x1_in_tile = if (tx + 1) * TILE <= rect.x1 {
                            64
                        } else {
                            (rect.x1 - tx * TILE) as usize
                        };

                        let mut mask = 0u64;
                        for lx in x0_in_tile..x1_in_tile {
                            mask |= 1u64 << lx;
                        }

                        tile.rows[ly] = row_mask & mask;
                    }

                    if !tile.is_empty() {
                        Some(((tx, ty), tile))
                    } else {
                        None
                    }
                })
            })
            .collect();

        // Update tiles and population
        let tiles = Arc::make_mut(&mut self.tiles);
        for (coord, tile) in new_tiles {
            let pop = tile.population() as u64;
            tiles.insert(coord, tile);
            self.population += pop;
        }
    }

    pub fn bounding_box(&self) -> Option<CellRect> {
        bounding_box(&self.tiles)
    }
}

pub fn bounding_box(tiles: &TileMap) -> Option<CellRect> {
    if tiles.is_empty() {
        return None;
    }

    let mut min_tx = i64::MAX;
    let mut max_tx = i64::MIN;
    let mut min_ty = i64::MAX;
    let mut max_ty = i64::MIN;

    for &(tx, ty) in tiles.keys() {
        min_tx = min_tx.min(tx);
        max_tx = max_tx.max(tx);
        min_ty = min_ty.min(ty);
        max_ty = max_ty.max(ty);
    }

    Some(CellRect {
        x0: min_tx * TILE,
        y0: min_ty * TILE,
        x1: (max_tx + 1) * TILE,
        y1: (max_ty + 1) * TILE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::step::step_naive;
    use std::collections::HashSet;

    #[test]
    fn set_get_negative_and_across_tiles() {
        let mut u = Universe::new();

        // Set cells at various positions including negative and large coordinates
        u.set(-1, -1, true);
        u.set(-64, 0, true);
        u.set(63, 64, true);
        u.set(i64::MAX / 2, -5, true);

        // All should be read as alive
        assert!(u.get(-1, -1));
        assert!(u.get(-64, 0));
        assert!(u.get(63, 64));
        assert!(u.get(i64::MAX / 2, -5));
        assert_eq!(u.population(), 4);

        // Clear one cell
        u.set(-1, -1, false);
        assert!(!u.get(-1, -1));
        assert_eq!(u.population(), 3);

        // The tile at (-1, -1) should be removed if it's now empty
        assert_eq!(u.tile_count(), 3);
    }

    #[test]
    fn glider_crosses_tiles() {
        let mut u = Universe::new();

        // Standard glider at (60, 60)
        //  .O.
        //  ..O
        //  OOO
        u.set(61, 60, true);
        u.set(62, 61, true);
        u.set(60, 62, true);
        u.set(61, 62, true);
        u.set(62, 62, true);

        assert_eq!(u.population(), 5);

        // After 4*k generations, glider is translated by (k, k)
        for k in 1..=10 {
            for _ in 0..4 {
                u.step();
            }
            assert_eq!(u.population(), 5, "Population should remain 5 at k={}", k);
            // Check that the glider has moved to (60+k, 60+k)
            // The glider should be centered around (60+k, 60+k)
        }

        assert_eq!(u.generation(), 40);
    }

    #[test]
    fn r_pentomino_1103() {
        let mut u = Universe::new();

        // R-pentomino pattern:
        //  .OO
        //  OO.
        //  .O.
        u.set(1, 0, true);
        u.set(2, 0, true);
        u.set(0, 1, true);
        u.set(1, 1, true);
        u.set(1, 2, true);

        for _ in 0..1103 {
            u.step();
        }

        assert_eq!(u.population(), 116);
        assert_eq!(u.generation(), 1103);
    }

    #[test]
    fn matches_naive_negative_region() {
        let mut u = Universe::new();

        // 120x120 soup with density 0.35 centered on (0,0)
        // This means from (-60, -60) to (60, 60)
        let rect = CellRect {
            x0: -60,
            y0: -60,
            x1: 60,
            y1: 60,
        };
        u.fill_random(rect, 0.35, 42);

        // Convert to HashSet for naive implementation
        let mut naive_cells = HashSet::new();
        for y in -60..60 {
            for x in -60..60 {
                if u.get(x, y) {
                    naive_cells.insert((x, y));
                }
            }
        }

        // Step 60 generations
        for gen in 0..60 {
            u.step();
            naive_cells = step_naive(&naive_cells);

            // Compare all cells in the region
            for y in -100..100 {
                for x in -100..100 {
                    let u_alive = u.get(x, y);
                    let naive_alive = naive_cells.contains(&(x, y));
                    assert_eq!(
                        u_alive, naive_alive,
                        "Mismatch at generation {} position ({}, {})",
                        gen + 1,
                        x,
                        y
                    );
                }
            }
        }
    }

    #[test]
    fn glider_keeps_tile_count_bounded() {
        let mut u = Universe::new();

        // Standard glider
        u.set(1, 0, true);
        u.set(2, 1, true);
        u.set(0, 2, true);
        u.set(1, 2, true);
        u.set(2, 2, true);

        for gen in 1..=1000 {
            u.step();
            assert!(
                u.tile_count() <= 4,
                "Generation {}: tile_count = {} exceeds 4",
                gen,
                u.tile_count()
            );
        }
    }

    #[test]
    fn fill_random_replaces_and_respects_density() {
        let mut u = Universe::new();

        // Set a cell outside the rect
        u.set(1000, 1000, true);

        // Fill a 512x512 region with density 0.35
        let rect = CellRect {
            x0: 0,
            y0: 0,
            x1: 512,
            y1: 512,
        };
        u.fill_random(rect, 0.35, 7);

        // Check density is roughly 0.35 (within 0.33 to 0.37)
        let expected = 512 * 512;
        let pop = u.population() - 1; // Exclude the (1000, 1000) cell
        let density = pop as f64 / expected as f64;
        assert!(
            density >= 0.33 && density <= 0.37,
            "Density {} not in range [0.33, 0.37]",
            density
        );

        // Check that (1000, 1000) is still alive
        assert!(u.get(1000, 1000));

        // Fill again with density 0 - should clear the rect but keep (1000, 1000)
        u.fill_random(rect, 0.0, 7);
        assert_eq!(u.population(), 1);
        assert!(u.get(1000, 1000));
    }

    #[test]
    fn bounding_box_empty_and_some() {
        let u = Universe::new();
        assert_eq!(u.bounding_box(), None);

        let mut u = Universe::new();
        u.set(-70, 5, true);

        let bb = u.bounding_box();
        assert!(bb.is_some());
        let bb = bb.unwrap();

        // Should contain the cell (-70, 5)
        assert!(bb.x0 <= -70 && bb.x1 > -70);
        assert!(bb.y0 <= 5 && bb.y1 > 5);
    }
}
