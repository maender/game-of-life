use crate::sim::Tile;

#[cfg(test)]
use std::collections::HashSet;

pub struct Neighborhood<'a> {
    pub c: &'a Tile,
    pub n: Option<&'a Tile>,
    pub s: Option<&'a Tile>,
    pub w: Option<&'a Tile>,
    pub e: Option<&'a Tile>,
    pub nw: Option<&'a Tile>,
    pub ne: Option<&'a Tile>,
    pub sw: Option<&'a Tile>,
    pub se: Option<&'a Tile>,
}

pub fn step_tile(nb: &Neighborhood) -> Tile {
    let empty_tile = Tile::default();
    let mut result = Tile::default();

    for y in 0..64 {
        // Get the three rows: above, current, below
        let (row_above, row_current, row_below) = if y == 0 {
            // Top row: need north tile
            let above = nb.n.unwrap_or(&empty_tile).rows[63];
            let current = nb.c.rows[0];
            let below = nb.c.rows[1];
            (above, current, below)
        } else if y == 63 {
            // Bottom row: need south tile
            let above = nb.c.rows[62];
            let current = nb.c.rows[63];
            let below = nb.s.unwrap_or(&empty_tile).rows[0];
            (above, current, below)
        } else {
            // Middle rows
            (nb.c.rows[y - 1], nb.c.rows[y], nb.c.rows[y + 1])
        };

        // Get west and east rows for current, above, and below
        let (w_above, w_current, w_below) = if y == 0 {
            let w_above = nb.nw.unwrap_or(&empty_tile).rows[63];
            let w_current = nb.w.unwrap_or(&empty_tile).rows[0];
            let w_below = nb.w.unwrap_or(&empty_tile).rows[1];
            (w_above, w_current, w_below)
        } else if y == 63 {
            let w_above = nb.w.unwrap_or(&empty_tile).rows[62];
            let w_current = nb.w.unwrap_or(&empty_tile).rows[63];
            let w_below = nb.sw.unwrap_or(&empty_tile).rows[0];
            (w_above, w_current, w_below)
        } else {
            let w = nb.w.unwrap_or(&empty_tile);
            (w.rows[y - 1], w.rows[y], w.rows[y + 1])
        };

        let (e_above, e_current, e_below) = if y == 0 {
            let e_above = nb.ne.unwrap_or(&empty_tile).rows[63];
            let e_current = nb.e.unwrap_or(&empty_tile).rows[0];
            let e_below = nb.e.unwrap_or(&empty_tile).rows[1];
            (e_above, e_current, e_below)
        } else if y == 63 {
            let e_above = nb.e.unwrap_or(&empty_tile).rows[62];
            let e_current = nb.e.unwrap_or(&empty_tile).rows[63];
            let e_below = nb.se.unwrap_or(&empty_tile).rows[0];
            (e_above, e_current, e_below)
        } else {
            let e = nb.e.unwrap_or(&empty_tile);
            (e.rows[y - 1], e.rows[y], e.rows[y + 1])
        };

        // Compute neighbor masks for all 8 neighbors
        // Row above: NW, N, NE
        let nw = (row_above << 1) | (w_above >> 63);
        let n = row_above;
        let ne = (row_above >> 1) | (e_above << 63);

        // Current row: W, E
        let w = (row_current << 1) | (w_current >> 63);
        let e = (row_current >> 1) | (e_current << 63);

        // Row below: SW, S, SE
        let sw = (row_below << 1) | (w_below >> 63);
        let s = row_below;
        let se = (row_below >> 1) | (e_below << 63);

        // Add all 8 neighbors using bit-parallel binary addition
        // We need to count 0-8 neighbors, which requires 4 bits, but we can saturate at 4
        // Use half/full adders to build s0, s1, s2 (bits of the count)

        // Add first two
        let sum0 = nw ^ n;
        let carry0 = nw & n;

        // Add third
        let sum1 = sum0 ^ ne;
        let carry1a = sum0 & ne;
        let carry1 = carry0 | carry1a;

        // Add fourth
        let sum2 = sum1 ^ w;
        let carry2a = sum1 & w;
        let carry2 = carry1 ^ carry2a;
        let carry2b = carry1 & carry2a;

        // Add fifth
        let sum3 = sum2 ^ e;
        let carry3a = sum2 & e;
        let carry3 = carry2 ^ carry3a;
        let carry3b = carry2 & carry3a;
        let carry3c = carry2b;

        // Add sixth
        let sum4 = sum3 ^ sw;
        let carry4a = sum3 & sw;
        let carry4 = carry3 ^ carry4a;
        let carry4b = carry3 & carry4a;
        let carry4c = carry3b | carry3c;

        // Add seventh
        let sum5 = sum4 ^ s;
        let carry5a = sum4 & s;
        let carry5 = carry4 ^ carry5a;
        let carry5b = carry4 & carry5a;
        let carry5c = carry4b | carry4c;

        // Add eighth
        let s0 = sum5 ^ se;
        let carry6a = sum5 & se;
        let s1 = carry5 ^ carry6a;
        let carry6b = carry5 & carry6a;
        let s2 = carry5b | carry5c | carry6b;

        // Saturate at 4 (s2 & s0 means count >= 4)
        // Actually, if s2 is set, count >= 4, so we can ignore higher bits

        // next = (count == 3) | (alive & count == 2)
        // count == 3: s2=0, s1=1, s0=1
        // count == 2: s2=0, s1=1, s0=0

        let count_eq_3 = !s2 & s1 & s0;
        let count_eq_2 = !s2 & s1 & !s0;

        result.rows[y] = count_eq_3 | (row_current & count_eq_2);
    }

    result
}

#[cfg(test)]
pub(crate) fn step_naive(cells: &HashSet<(i64, i64)>) -> HashSet<(i64, i64)> {
    let mut neighbor_counts: std::collections::HashMap<(i64, i64), u8> = std::collections::HashMap::new();

    // Count neighbors for all cells and their neighbors
    for &(x, y) in cells {
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let neighbor = (x + dx, y + dy);
                *neighbor_counts.entry(neighbor).or_insert(0) += 1;
            }
        }
    }

    let mut next = HashSet::new();

    // Apply Game of Life rules
    for (&pos, &count) in &neighbor_counts {
        let alive = cells.contains(&pos);
        let survives = if alive {
            count == 2 || count == 3
        } else {
            count == 3
        };

        if survives {
            next.insert(pos);
        }
    }

    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::TILE;

    fn make_tile(coords: &[(usize, usize)]) -> Tile {
        let mut tile = Tile::default();
        for &(x, y) in coords {
            tile.set(x, y, true);
        }
        tile
    }

    #[test]
    fn blinker_in_tile() {
        // Horizontal blinker: (10,10), (11,10), (12,10)
        // Should become vertical: (11,9), (11,10), (11,11)
        let c = make_tile(&[(10, 10), (11, 10), (12, 10)]);
        let nb = Neighborhood {
            c: &c,
            n: None,
            s: None,
            w: None,
            e: None,
            nw: None,
            ne: None,
            sw: None,
            se: None,
        };

        let next = step_tile(&nb);

        assert!(!next.get(10, 10), "Cell (10,10) should be dead");
        assert!(next.get(11, 9), "Cell (11,9) should be alive");
        assert!(next.get(11, 10), "Cell (11,10) should be alive");
        assert!(next.get(11, 11), "Cell (11,11) should be alive");
        assert!(!next.get(12, 10), "Cell (12,10) should be dead");
        assert_eq!(next.population(), 3, "Should have exactly 3 cells");
    }

    #[test]
    fn blinker_across_east_border() {
        // Horizontal blinker at east border: (62,5), (63,5) in c, (0,5) in e
        // Should become vertical: (63,4), (63,5), (63,6) in c only
        let c = make_tile(&[(62, 5), (63, 5)]);
        let e = make_tile(&[(0, 5)]);
        let nb = Neighborhood {
            c: &c,
            n: None,
            s: None,
            w: None,
            e: Some(&e),
            nw: None,
            ne: None,
            sw: None,
            se: None,
        };

        let next = step_tile(&nb);

        assert!(!next.get(62, 5), "Cell (62,5) should be dead");
        assert!(next.get(63, 4), "Cell (63,4) should be alive");
        assert!(next.get(63, 5), "Cell (63,5) should be alive");
        assert!(next.get(63, 6), "Cell (63,6) should be alive");
        assert_eq!(next.population(), 3, "Should have exactly 3 cells in c");
    }

    #[test]
    fn matches_naive_random() {
        // 200 random tests with fixed seed xorshift, density 0.4
        // step_tile on center tile should match cells from step_naive restricted to center tile

        struct XorShift {
            state: u64,
        }

        impl XorShift {
            fn new(seed: u64) -> Self {
                XorShift { state: seed }
            }

            fn next(&mut self) -> u64 {
                let mut x = self.state;
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                self.state = x;
                x
            }

            fn next_f64(&mut self) -> f64 {
                (self.next() & 0x1FFFFFFFFFFFFF) as f64 / (1u64 << 53) as f64
            }
        }

        let mut rng = XorShift::new(0x123456789ABCDEF0);
        const DENSITY: f64 = 0.4;

        for test_num in 0..200 {
            // Generate 9 tiles (3x3 neighborhood)
            let mut tiles = [[Tile::default(); 3]; 3];
            let mut all_cells = HashSet::new();

            for ty in 0..3 {
                for tx in 0..3 {
                    for ly in 0..64 {
                        for lx in 0..64 {
                            if rng.next_f64() < DENSITY {
                                tiles[ty][tx].set(lx, ly, true);
                                // Global coordinates: center tile is at (0, 0)
                                let gx = (tx as i64 - 1) * TILE + lx as i64;
                                let gy = (ty as i64 - 1) * TILE + ly as i64;
                                all_cells.insert((gx, gy));
                            }
                        }
                    }
                }
            }

            let nb = Neighborhood {
                c: &tiles[1][1],  // center
                n: Some(&tiles[0][1]),  // north
                s: Some(&tiles[2][1]),  // south
                w: Some(&tiles[1][0]),  // west
                e: Some(&tiles[1][2]),  // east
                nw: Some(&tiles[0][0]),
                ne: Some(&tiles[0][2]),
                sw: Some(&tiles[2][0]),
                se: Some(&tiles[2][2]),
            };

            let next_tile = step_tile(&nb);
            let next_naive = step_naive(&all_cells);

            // Check every cell in the center tile
            for ly in 0..64 {
                for lx in 0..64 {
                    let alive_in_tile = next_tile.get(lx, ly);
                    let alive_in_naive = next_naive.contains(&(lx as i64, ly as i64));

                    assert_eq!(
                        alive_in_tile, alive_in_naive,
                        "Test {}: cell ({},{}) mismatch: tile={} naive={}",
                        test_num, lx, ly, alive_in_tile, alive_in_naive
                    );
                }
            }
        }
    }
}
