pub fn line_cells(a: (i64, i64), b: (i64, i64)) -> Vec<(i64, i64)> {
    let (x0, y0) = a;
    let (x1, y1) = b;

    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();

    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };

    let mut cells = Vec::new();
    let mut x = x0;
    let mut y = y0;
    let mut err = dx - dy;

    loop {
        cells.push((x, y));

        if x == x1 && y == y1 {
            break;
        }

        let e2 = 2 * err;

        if e2 > -dy {
            err -= dy;
            x += sx;
        }

        if e2 < dx {
            err += dx;
            y += sy;
        }
    }

    cells
}
