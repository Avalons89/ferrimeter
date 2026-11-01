//! Tray icon: compact rate text drawn with a built-in 3×5 pixel font.

use crate::format::Units;

/// Icons are designed on a 22×22 grid and scaled up by whole multiples.
const BASE: i32 = 22;
const DOWN_RGB: [u8; 3] = [0x4C, 0xD9, 0x64];
const UP_RGB: [u8; 3] = [0xFF, 0x9F, 0x43];
/// Top row of each 5-row line on the base grid.
const LINE_Y: [i32; 2] = [4, 13];

/// At most 4 characters: `0`, `512`, `12K`, `1.2M`, `999G`.
pub fn compact(bytes_per_sec: f64, units: Units) -> String {
    let bytes = if bytes_per_sec.is_finite() && bytes_per_sec > 0.0 { bytes_per_sec } else { 0.0 };
    let (mut value, base) = match units {
        Units::Bytes => (bytes, 1024.0),
        Units::Bits => (bytes * 8.0, 1000.0),
    };
    const SUFFIX: [&str; 4] = ["", "K", "M", "G"];
    let mut unit = 0;
    while value >= 999.5 && unit + 1 < SUFFIX.len() {
        value /= base;
        unit += 1;
    }
    if value >= 999.5 {
        return "999G".into(); // beyond the largest unit: clamp instead of growing wider
    }
    let one_decimal = (value * 10.0).round() / 10.0;
    if unit == 0 || one_decimal >= 10.0 {
        format!("{}{}", value.round() as u64, SUFFIX[unit])
    } else if one_decimal == 0.0 {
        "0".into()
    } else {
        format!("{one_decimal:.1}{}", SUFFIX[unit])
    }
}

/// 3×5 glyphs, one row per entry, bit 2 = left column.
fn glyph(c: char) -> Option<[u8; 5]> {
    Some(match c {
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b001, 0b001],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '.' => [0b000, 0b000, 0b000, 0b000, 0b010],
        '-' => [0b000, 0b000, 0b111, 0b000, 0b000],
        'K' => [0b101, 0b101, 0b110, 0b101, 0b101],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'G' => [0b111, 0b100, 0b101, 0b101, 0b111],
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Cell {
    Empty,
    Outline,
    Ink([u8; 3]),
}

/// ARGB32 pixels in network byte order (A, R, G, B per pixel), as the
/// StatusNotifierItem `IconPixmap` property requires. `size` is 22 or 44.
pub fn render(down: &str, up: &str, size: i32) -> Vec<u8> {
    let mut grid = [[Cell::Empty; BASE as usize]; BASE as usize];
    for (text, rgb, top) in [(down, DOWN_RGB, LINE_Y[0]), (up, UP_RGB, LINE_Y[1])] {
        let glyphs: Vec<[u8; 5]> = text.chars().filter_map(glyph).collect();
        let width = glyphs.len() as i32 * 4 - 1;
        let left = (BASE - width) / 2;
        for (i, g) in glyphs.iter().enumerate() {
            for (row, bits) in g.iter().enumerate() {
                for col in 0..3 {
                    if bits & (0b100 >> col) != 0 {
                        let (x, y) = (left + i as i32 * 4 + col, top + row as i32);
                        if (0..BASE).contains(&x) && (0..BASE).contains(&y) {
                            grid[y as usize][x as usize] = Cell::Ink(rgb);
                        }
                    }
                }
            }
        }
    }
    // A dark outline keeps the text readable on light and dark panels.
    let snapshot = grid;
    for y in 0..BASE {
        for x in 0..BASE {
            let inked = |dx: i32, dy: i32| {
                let (nx, ny) = (x + dx, y + dy);
                (0..BASE).contains(&nx) && (0..BASE).contains(&ny) && matches!(snapshot[ny as usize][nx as usize], Cell::Ink(_))
            };
            if snapshot[y as usize][x as usize] == Cell::Empty && (inked(-1, 0) || inked(1, 0) || inked(0, -1) || inked(0, 1)) {
                grid[y as usize][x as usize] = Cell::Outline;
            }
        }
    }
    let scale = (size / BASE).max(1);
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let (gx, gy) = (x / scale, y / scale);
            if gx >= BASE || gy >= BASE {
                continue;
            }
            let argb = match grid[gy as usize][gx as usize] {
                Cell::Empty => continue,
                Cell::Outline => [0xFF, 0, 0, 0],
                Cell::Ink([r, g, b]) => [0xFF, r, g, b],
            };
            let i = ((y * size + x) * 4) as usize;
            out[i..i + 4].copy_from_slice(&argb);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;

    fn px(buf: &[u8], size: i32, x: i32, y: i32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    #[test]
    fn compact_values() {
        assert_eq!(compact(0.0, Units::Bytes), "0");
        assert_eq!(compact(512.0, Units::Bytes), "512");
        assert_eq!(compact(12.0 * KB, Units::Bytes), "12K");
        assert_eq!(compact(1.2 * MB, Units::Bytes), "1.2M");
        assert_eq!(compact(45.0 * MB, Units::Bytes), "45M");
        assert_eq!(compact(1.1 * GB, Units::Bytes), "1.1G");
        assert_eq!(compact(9.96 * KB, Units::Bytes), "10K");
        assert_eq!(compact(1_250_000.0, Units::Bits), "10M");
    }

    #[test]
    fn compact_never_exceeds_four_chars() {
        for v in [5000.0 * GB, f64::NAN, f64::INFINITY, -1.0, 999.6 * KB, 99.96 * MB, 1023.9, 999.4 * GB] {
            for u in [Units::Bytes, Units::Bits] {
                let s = compact(v, u);
                assert!(s.chars().count() <= 4, "{v} {u:?} -> {s}");
            }
        }
        assert_eq!(compact(5000.0 * GB, Units::Bytes), "999G");
        assert_eq!(compact(f64::NAN, Units::Bytes), "0");
        assert_eq!(compact(999.6 * KB, Units::Bytes), "1.0M");
    }

    #[test]
    fn render_size_and_transparency() {
        let buf = render("0", "0", 22);
        assert_eq!(buf.len(), 22 * 22 * 4);
        assert_eq!(px(&buf, 22, 0, 0)[0], 0, "corner must be transparent");
    }

    #[test]
    fn render_byte_order() {
        // A lone "0" is centered: x0 = (22 - 3) / 2 = 9; lines start at y 4 and 13.
        let buf = render("0", "0", 22);
        assert_eq!(px(&buf, 22, 9, 4), [0xFF, 0x4C, 0xD9, 0x64]); // down: green
        assert_eq!(px(&buf, 22, 9, 13), [0xFF, 0xFF, 0x9F, 0x43]); // up: orange
        assert_eq!(px(&buf, 22, 8, 4), [0xFF, 0x00, 0x00, 0x00]); // outline left of the glyph
    }

    #[test]
    fn render_scales_to_44() {
        let buf = render("1.2M", "80K", 44);
        assert_eq!(buf.len(), 44 * 44 * 4);
        assert!(buf.chunks(4).any(|p| p == [0xFF, 0x4C, 0xD9, 0x64]));
    }

    #[test]
    fn unknown_characters_are_skipped() {
        // The tray passes "-" while there's no reading yet; it has a glyph.
        assert!(render("-", "-", 22).chunks(4).any(|p| p[0] == 0xFF));
        assert_eq!(render("?", "?", 22).len(), 22 * 22 * 4);
    }
}
