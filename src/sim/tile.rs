pub const TILE: i64 = 64;

pub type TileCoord = (i64, i64);

/// Splits global coordinates into tile coordinates and local coordinates within that tile.
/// Returns (tile_coord, local_x, local_y)
pub fn split(x: i64, y: i64) -> (TileCoord, usize, usize) {
    let tile_x = x.div_euclid(TILE);
    let tile_y = y.div_euclid(TILE);
    let local_x = x.rem_euclid(TILE) as usize;
    let local_y = y.rem_euclid(TILE) as usize;
    ((tile_x, tile_y), local_x, local_y)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Tile {
    pub rows: [u64; 64],
}

impl Default for Tile {
    fn default() -> Self {
        Tile { rows: [0; 64] }
    }
}

impl Tile {
    pub fn get(&self, lx: usize, ly: usize) -> bool {
        (self.rows[ly] & (1u64 << lx)) != 0
    }

    pub fn set(&mut self, lx: usize, ly: usize, alive: bool) {
        if alive {
            self.rows[ly] |= 1u64 << lx;
        } else {
            self.rows[ly] &= !(1u64 << lx);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rows.iter().all(|&row| row == 0)
    }

    pub fn population(&self) -> u32 {
        self.rows.iter().map(|row| row.count_ones()).sum()
    }

    pub fn border(&self) -> Border {
        let n = self.rows[0] != 0;
        let s = self.rows[63] != 0;
        let w = self.rows.iter().any(|&row| (row & 1) != 0);
        let e = self.rows.iter().any(|&row| (row & (1u64 << 63)) != 0);
        Border { n, s, w, e }
    }
}

pub struct Border {
    pub n: bool,
    pub s: bool,
    pub w: bool,
    pub e: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_handles_negatives() {
        assert_eq!(split(-1, -1), ((-1, -1), 63, 63));
        assert_eq!(split(64, 0), ((1, 0), 0, 0));
        assert_eq!(split(63, 127), ((0, 1), 63, 63));
    }

    #[test]
    fn set_get_population() {
        let mut tile = Tile::default();

        // Set three cells
        tile.set(0, 0, true);
        tile.set(63, 0, true);
        tile.set(5, 63, true);

        assert_eq!(tile.population(), 3);
        assert!(tile.get(63, 0));
        assert!(!tile.get(1, 0));

        // Clear one cell
        tile.set(0, 0, false);
        assert_eq!(tile.population(), 2);
    }

    #[test]
    fn border_flags() {
        let mut tile = Tile::default();

        // Tile with only (0, 10) - should have west border only
        tile.set(0, 10, true);
        let border = tile.border();
        assert!(border.w);
        assert!(!border.e);
        assert!(!border.n);
        assert!(!border.s);

        // Tile with only (63, 63) - should have east and south borders
        let mut tile2 = Tile::default();
        tile2.set(63, 63, true);
        let border2 = tile2.border();
        assert!(border2.e);
        assert!(border2.s);
    }
}
