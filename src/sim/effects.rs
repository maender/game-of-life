use crate::sim::{Tile, TileCoord, TileMap};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

pub const AGE_MAX: u8 = 12;
pub const TRAIL_LEN: u8 = 4;

#[derive(Clone)]
pub struct TileMeta {
    pub age: [u8; 4096],
    pub trail: [u8; 4096],
}

pub type MetaMap = FxHashMap<TileCoord, TileMeta>;

pub fn update_meta(old: &MetaMap, new_tiles: &TileMap) -> MetaMap {
    // Tiles alive now, then tiles only in the old meta (fading trails): the
    // two parts are disjoint, so no set union is needed.
    let empty = Tile::default();
    let live = new_tiles
        .par_iter()
        .filter_map(|(&coord, tile)| next_tile_meta(old.get(&coord), tile).map(|m| (coord, m)));
    let fading = old
        .par_iter()
        .filter(|(coord, _)| !new_tiles.contains_key(coord))
        .filter_map(|(&coord, meta)| next_tile_meta(Some(meta), &empty).map(|m| (coord, m)));
    live.chain(fading).collect()
}

/// Next meta of one tile, or `None` if it holds no live cell and no trail.
fn next_tile_meta(old: Option<&TileMeta>, tile: &Tile) -> Option<TileMeta> {
    let mut meta = TileMeta {
        age: [0; 4096],
        trail: [0; 4096],
    };
    for (ly, &row) in tile.rows.iter().enumerate() {
        // 0xff for live cells, 0 for dead ones (branchless, vectorizable).
        let alive: [u8; 64] = std::array::from_fn(|lx| 0u8.wrapping_sub(((row >> lx) & 1) as u8));
        let range = ly * 64..ly * 64 + 64;
        let (age, trail) = (&mut meta.age[range.clone()], &mut meta.trail[range.clone()]);
        match old {
            Some(o) => {
                let (old_age, old_trail) = (&o.age[range.clone()], &o.trail[range]);
                for lx in 0..64 {
                    let m = alive[lx];
                    age[lx] = old_age[lx].saturating_add(1).min(AGE_MAX) & m;
                    trail[lx] = (TRAIL_LEN & m) | (old_trail[lx].saturating_sub(1) & !m);
                }
            }
            None => {
                for lx in 0..64 {
                    age[lx] = 1 & alive[lx];
                    trail[lx] = TRAIL_LEN & alive[lx];
                }
            }
        }
    }
    let has_data = !tile.is_empty() || meta.trail.iter().any(|&t| t > 0);
    has_data.then_some(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::Universe;

    /// Per-cell reference of the age/trail rules, over the union of both maps.
    fn update_meta_reference(old: &MetaMap, new_tiles: &TileMap) -> MetaMap {
        let mut coords: Vec<TileCoord> = old.keys().chain(new_tiles.keys()).copied().collect();
        coords.sort();
        coords.dedup();
        let mut out = MetaMap::default();
        for coord in coords {
            let mut m = TileMeta {
                age: [0; 4096],
                trail: [0; 4096],
            };
            for ly in 0..64 {
                for lx in 0..64 {
                    let idx = ly * 64 + lx;
                    let alive = new_tiles.get(&coord).is_some_and(|t| t.get(lx, ly));
                    let (old_age, old_trail) = old
                        .get(&coord)
                        .map_or((0, 0), |o| (o.age[idx], o.trail[idx]));
                    if alive {
                        m.age[idx] = old_age.saturating_add(1).min(AGE_MAX);
                        m.trail[idx] = TRAIL_LEN;
                    } else {
                        m.trail[idx] = old_trail.saturating_sub(1);
                    }
                }
            }
            if m.age.iter().any(|&a| a > 0) || m.trail.iter().any(|&t| t > 0) {
                out.insert(coord, m);
            }
        }
        out
    }

    fn assert_same_meta(a: &MetaMap, b: &MetaMap) {
        assert_eq!(a.len(), b.len(), "tile counts differ");
        for (coord, ma) in a {
            let mb = b
                .get(coord)
                .unwrap_or_else(|| panic!("tile {:?} missing", coord));
            assert!(ma.age == mb.age, "age differs in tile {:?}", coord);
            assert!(ma.trail == mb.trail, "trail differs in tile {:?}", coord);
        }
    }

    #[test]
    fn update_meta_matches_reference_on_soup() {
        use crate::sim::CellRect;
        let mut u = Universe::new();
        u.fill_random(
            CellRect {
                x0: -200,
                y0: -130,
                x1: 190,
                y1: 140,
            },
            0.35,
            7,
        );
        u.set_effects(true);
        for _ in 0..40 {
            let old = u.meta().expect("meta").clone();
            u.step();
            let expected = update_meta_reference(&old, u.tiles());
            assert_same_meta(u.meta().expect("meta"), &expected);
        }
        // Also compare once all cells are gone: only fading trails remain.
        let old = u.meta().expect("meta").clone();
        let empty = TileMap::default();
        assert_same_meta(
            &update_meta(&old, &empty),
            &update_meta_reference(&old, &empty),
        );
    }

    #[test]
    fn block_ages_to_cap() {
        let mut u = Universe::new();

        // Create 2x2 block
        u.set(0, 0, true);
        u.set(1, 0, true);
        u.set(0, 1, true);
        u.set(1, 1, true);

        u.set_effects(true);

        // Run 20 generations
        for _ in 0..20 {
            u.step();
        }

        // Check that all cells have aged to AGE_MAX
        let meta = u.meta().expect("Meta should exist");
        let tile_meta = meta.get(&(0, 0)).expect("Meta tile should exist");

        // Check each cell in the 2x2 block (ly*64+lx)
        for ly in 0..2 {
            for lx in 0..2 {
                let idx = ly * 64 + lx;
                assert_eq!(
                    tile_meta.age[idx], AGE_MAX,
                    "Cell ({}, {}) should have age {}",
                    lx, ly, AGE_MAX
                );
            }
        }
    }

    #[test]
    fn dead_cell_trail_fades() {
        let mut u = Universe::new();

        // Single isolated cell (will die after 1 generation)
        u.set(10, 10, true);

        u.set_effects(true);

        // Step once - cell dies
        u.step();

        // Check trail is TRAIL_LEN - 1
        let meta = u.meta().expect("Meta should exist");
        let tile_meta = meta.get(&(0, 0)).expect("Meta tile should exist");
        let idx = 10 * 64 + 10;
        assert_eq!(
            tile_meta.trail[idx],
            TRAIL_LEN - 1,
            "Trail should be {} after cell dies",
            TRAIL_LEN - 1
        );

        // Step TRAIL_LEN more times
        for _ in 0..TRAIL_LEN {
            u.step();
        }

        // Meta tile should not exist (or if it exists, trail should be 0)
        let meta = u.meta().expect("Meta should exist");
        if let Some(tile_meta) = meta.get(&(0, 0)) {
            assert_eq!(tile_meta.trail[idx], 0, "Trail should have faded to 0");
        }
        // Actually, the tile should be removed
        assert!(
            meta.get(&(0, 0)).is_none(),
            "Meta tile should be removed when no cells alive and no trail"
        );
    }

    #[test]
    fn effects_off_has_no_meta() {
        let mut u = Universe::new();

        // Check meta is None by default
        assert!(u.meta().is_none(), "Meta should be None by default");

        // Add some cells
        u.set(0, 0, true);
        u.set(1, 1, true);

        // Turn effects on then off
        u.set_effects(true);
        assert!(
            u.meta().is_some(),
            "Meta should exist after enabling effects"
        );

        u.set_effects(false);
        assert!(
            u.meta().is_none(),
            "Meta should be None after disabling effects"
        );
    }

    #[test]
    fn newborn_age_is_one() {
        let mut u = Universe::new();

        // Create blinker (oscillator with period 2)
        // Generation 0:  .O.    Generation 1:  ...
        //                .O.                   OOO
        //                .O.                   ...
        u.set(1, 0, true);
        u.set(1, 1, true);
        u.set(1, 2, true);

        u.set_effects(true);

        // Step once - blinker flips
        u.step();

        // The cells at (0,1), (1,1), (2,1) should exist
        // (1,1) was alive before, so age > 1
        // (0,1) and (2,1) are newborn, so age == 1
        let meta = u.meta().expect("Meta should exist");
        let tile_meta = meta.get(&(0, 0)).expect("Meta tile should exist");

        let idx_left = 1 * 64 + 0; // (0, 1)
        let idx_right = 1 * 64 + 2; // (2, 1)

        assert_eq!(tile_meta.age[idx_left], 1, "Newborn cell should have age 1");
        assert_eq!(
            tile_meta.age[idx_right], 1,
            "Newborn cell should have age 1"
        );
    }
}
