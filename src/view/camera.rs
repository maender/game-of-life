use crate::sim::CellRect;

pub const MIN_PX_PER_CELL: f64 = 1.0 / 4096.0;
pub const MAX_PX_PER_CELL: f64 = 128.0;
pub const MAX_RANDOMIZE_CELLS: u128 = 1 << 28;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub center: (f64, f64), // in cells
    pub px_per_cell: f64,
    pub screen: (f64, f64), // physical pixels
}

impl Camera {
    pub fn screen_to_cell(&self, sx: f64, sy: f64) -> (f64, f64) {
        let screen_center_x = self.screen.0 / 2.0;
        let screen_center_y = self.screen.1 / 2.0;
        let pixel_offset_x = sx - screen_center_x;
        let pixel_offset_y = sy - screen_center_y;
        let cell_offset_x = pixel_offset_x / self.px_per_cell;
        let cell_offset_y = pixel_offset_y / self.px_per_cell;
        (self.center.0 + cell_offset_x, self.center.1 + cell_offset_y)
    }

    pub fn cell_at(&self, sx: f64, sy: f64) -> (i64, i64) {
        let (cx, cy) = self.screen_to_cell(sx, sy);
        (cx.floor() as i64, cy.floor() as i64)
    }

    pub fn zoom_at(&mut self, sx: f64, sy: f64, factor: f64) {
        // Guard against non-finite inputs
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }

        // Get the cell coordinate at the cursor before zoom
        let cell_before = self.screen_to_cell(sx, sy);

        // Apply zoom with clamping
        let new_px_per_cell = (self.px_per_cell * factor).clamp(MIN_PX_PER_CELL, MAX_PX_PER_CELL);

        // Update px_per_cell
        self.px_per_cell = new_px_per_cell;

        // Get the cell coordinate at the cursor after zoom (with new px_per_cell)
        let cell_after = self.screen_to_cell(sx, sy);

        // Adjust center to keep the point fixed
        self.center.0 += cell_before.0 - cell_after.0;
        self.center.1 += cell_before.1 - cell_after.1;
    }

    pub fn pan_px(&mut self, dx: f64, dy: f64) {
        let cell_dx = -dx / self.px_per_cell;
        let cell_dy = -dy / self.px_per_cell;
        self.center.0 += cell_dx;
        self.center.1 += cell_dy;
    }

    pub fn visible_rect(&self) -> CellRect {
        // Top-left corner of screen
        let (tl_x, tl_y) = self.screen_to_cell(0.0, 0.0);
        // Bottom-right corner of screen
        let (br_x, br_y) = self.screen_to_cell(self.screen.0, self.screen.1);

        CellRect {
            x0: tl_x.floor() as i64,
            y0: tl_y.floor() as i64,
            x1: br_x.ceil() as i64,
            y1: br_y.ceil() as i64,
        }
    }

    pub fn fit(&mut self, r: Option<CellRect>) {
        if let Some(rect) = r {
            // Center on the rect
            let center_x = (rect.x0 as f64 + rect.x1 as f64) / 2.0;
            let center_y = (rect.y0 as f64 + rect.y1 as f64) / 2.0;
            self.center = (center_x, center_y);

            // Compute dimensions
            let width = (rect.x1 - rect.x0) as f64;
            let height = (rect.y1 - rect.y0) as f64;

            // Choose px_per_cell to fit ~80% of screen
            let px_per_cell_x = (self.screen.0 * 0.8) / width;
            let px_per_cell_y = (self.screen.1 * 0.8) / height;
            let px_per_cell = px_per_cell_x.min(px_per_cell_y);

            // Clamp to bounds
            self.px_per_cell = px_per_cell.clamp(MIN_PX_PER_CELL, MAX_PX_PER_CELL);
        }
    }
}

pub fn randomize_rect(cam: &Camera) -> (CellRect, bool) {
    let visible = cam.visible_rect();
    let visible_area = visible.area();

    if visible_area <= MAX_RANDOMIZE_CELLS {
        return (visible, false);
    }

    // Need to cap: compute a centered rect with area <= MAX_RANDOMIZE_CELLS
    // Keep screen aspect ratio
    let aspect_ratio = cam.screen.0 / cam.screen.1;

    // Compute dimensions: width * height <= MAX_RANDOMIZE_CELLS, width / height = aspect_ratio
    // => width = aspect_ratio * height
    // => aspect_ratio * height^2 = MAX_RANDOMIZE_CELLS
    // => height = sqrt(MAX_RANDOMIZE_CELLS / aspect_ratio)
    let height_f64 = ((MAX_RANDOMIZE_CELLS as f64) / aspect_ratio).sqrt();
    let width_f64 = height_f64 * aspect_ratio;

    // Convert to i64, using floor to ensure area doesn't exceed limit
    let width = width_f64.floor() as i64;
    let height = height_f64.floor() as i64;

    let half_width = width / 2;
    let half_height = height / 2;

    let center_x = cam.center.0.round() as i64;
    let center_y = cam.center.1.round() as i64;

    let rect = CellRect {
        x0: center_x - half_width,
        y0: center_y - half_height,
        x1: center_x - half_width + width,
        y1: center_y - half_height + height,
    };

    (rect, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_keeps_point_under_cursor() {
        let mut cam = Camera {
            center: (100.0, 100.0),
            px_per_cell: 10.0,
            screen: (800.0, 600.0),
        };

        let p = cam.screen_to_cell(300.0, 200.0);
        cam.zoom_at(300.0, 200.0, 1.7);
        let p_after = cam.screen_to_cell(300.0, 200.0);

        assert!(
            (p.0 - p_after.0).abs() < 1e-9,
            "x mismatch: {} vs {}",
            p.0,
            p_after.0
        );
        assert!(
            (p.1 - p_after.1).abs() < 1e-9,
            "y mismatch: {} vs {}",
            p.1,
            p_after.1
        );
    }

    #[test]
    fn zoom_is_clamped() {
        let mut cam = Camera {
            center: (0.0, 0.0),
            px_per_cell: 10.0,
            screen: (800.0, 600.0),
        };

        // Zoom in repeatedly with huge factor
        for _ in 0..10 {
            cam.zoom_at(400.0, 300.0, 1e9);
        }
        assert_eq!(cam.px_per_cell, MAX_PX_PER_CELL);
        assert!(!cam.px_per_cell.is_nan());

        // Reset and zoom out repeatedly with tiny factor
        cam.px_per_cell = 10.0;
        for _ in 0..10 {
            cam.zoom_at(400.0, 300.0, 1e-9);
        }
        assert_eq!(cam.px_per_cell, MIN_PX_PER_CELL);
        assert!(!cam.px_per_cell.is_nan());
    }

    #[test]
    fn cell_at_floors_negatives() {
        let cam = Camera {
            center: (0.0, 0.0),
            px_per_cell: 10.0,
            screen: (100.0, 100.0),
        };

        assert_eq!(cam.cell_at(49.0, 49.0), (-1, -1));
        assert_eq!(cam.cell_at(50.0, 50.0), (0, 0));
    }

    #[test]
    fn fit_empty_is_noop() {
        let cam = Camera {
            center: (100.0, 200.0),
            px_per_cell: 15.0,
            screen: (1024.0, 768.0),
        };

        let mut cam_copy = cam;
        cam_copy.fit(None);

        assert_eq!(cam.center.0, cam_copy.center.0);
        assert_eq!(cam.center.1, cam_copy.center.1);
        assert_eq!(cam.px_per_cell, cam_copy.px_per_cell);
    }

    #[test]
    fn randomize_rect_is_capped() {
        // Small px_per_cell (very zoomed out) - should be capped
        let cam_small = Camera {
            center: (0.0, 0.0),
            px_per_cell: MIN_PX_PER_CELL,
            screen: (3456.0, 2234.0),
        };

        let (rect, capped) = randomize_rect(&cam_small);
        assert!(rect.area() <= MAX_RANDOMIZE_CELLS);
        assert!(capped, "Should be capped when zoomed out far");

        // Larger px_per_cell - should not be capped
        let cam_large = Camera {
            center: (0.0, 0.0),
            px_per_cell: 8.0,
            screen: (3456.0, 2234.0),
        };

        let (rect, capped) = randomize_rect(&cam_large);
        assert_eq!(rect, cam_large.visible_rect());
        assert!(!capped, "Should not be capped when visible rect is small");
    }

    #[test]
    fn line_cells_is_8_connected() {
        use crate::view::line::line_cells;

        let cells = line_cells((-5, -3), (37, -80));

        // Check first and last
        assert_eq!(cells[0], (-5, -3));
        assert_eq!(cells[cells.len() - 1], (37, -80));

        // Check 8-connectivity (Chebyshev distance <= 1)
        for i in 0..cells.len() - 1 {
            let (x0, y0) = cells[i];
            let (x1, y1) = cells[i + 1];
            let dx = (x1 - x0).abs();
            let dy = (y1 - y0).abs();
            let chebyshev = dx.max(dy);
            assert_eq!(
                chebyshev,
                1,
                "Non-8-connected at index {}: {:?} -> {:?}",
                i,
                cells[i],
                cells[i + 1]
            );
        }
    }
}
