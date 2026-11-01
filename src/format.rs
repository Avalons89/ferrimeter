//! Turns byte rates into the short strings shown on the widget and tray tooltip.

use serde::{Deserialize, Serialize};

/// Shown in place of a value when no reading is available.
pub const DASH: &str = "—";

/// Tray tooltips are capped at 128 UTF-16 units including the terminating NUL.
const TOOLTIP_MAX_UTF16: usize = 127;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Units {
    #[default]
    Bytes,
    Bits,
}

pub fn format_rate(bytes_per_sec: f64, units: Units) -> String {
    let bytes = if bytes_per_sec.is_finite() && bytes_per_sec > 0.0 {
        bytes_per_sec
    } else {
        0.0
    };
    // Bytes use binary steps; bits use the decimal steps network speeds are quoted in.
    let (mut value, base, names): (f64, f64, [&str; 4]) = match units {
        Units::Bytes => (bytes, 1024.0, ["B/s", "KB/s", "MB/s", "GB/s"]),
        Units::Bits => (bytes * 8.0, 1000.0, ["bps", "Kbps", "Mbps", "Gbps"]),
    };
    // Step up once a value reaches four digits, so the widget stays narrow.
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < names.len() {
        value /= base;
        unit += 1;
    }
    let rounded = (value * 10.0).round() / 10.0;
    let number = if rounded == 0.0 {
        "0".to_string()
    } else if rounded >= 100.0 {
        format!("{value:.0}")
    } else {
        format!("{rounded:.1}")
    };
    format!("{number} {}", names[unit])
}

pub fn widget_lines(rates: Option<(f64, f64)>, units: Units) -> (String, String) {
    match rates {
        Some((down, up)) => (
            format!("↓ {}", format_rate(down, units)),
            format!("↑ {}", format_rate(up, units)),
        ),
        None => (format!("↓ {DASH}"), format!("↑ {DASH}")),
    }
}

pub fn tooltip(label: &str, rates: Option<(f64, f64)>, units: Units) -> String {
    let (down, up) = widget_lines(rates, units);
    let full = format!("{label}\n{down}  {up}");
    let mut out = String::new();
    let mut used = 0;
    for ch in full.chars() {
        used += ch.len_utf16();
        if used > TOOLTIP_MAX_UTF16 {
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;

    #[test]
    fn zero() {
        assert_eq!(format_rate(0.0, Units::Bytes), "0 B/s");
        assert_eq!(format_rate(0.0, Units::Bits), "0 bps");
    }

    #[test]
    fn bytes_scale_and_decimals() {
        assert_eq!(format_rate(999.0, Units::Bytes), "999 B/s");
        assert_eq!(format_rate(1000.0, Units::Bytes), "1.0 KB/s");
        assert_eq!(format_rate(512.0 * KB, Units::Bytes), "512 KB/s");
        assert_eq!(format_rate(9.6 * MB, Units::Bytes), "9.6 MB/s");
        assert_eq!(format_rate(12.4 * MB, Units::Bytes), "12.4 MB/s");
        assert_eq!(format_rate(99.96 * KB, Units::Bytes), "100 KB/s");
    }

    #[test]
    fn bits_scale() {
        assert_eq!(format_rate(1_250_000.0, Units::Bits), "10.0 Mbps");
        assert_eq!(format_rate(12_500_000.0, Units::Bits), "100 Mbps");
        assert_eq!(format_rate(125.0, Units::Bits), "1.0 Kbps");
    }

    #[test]
    fn largest_unit_caps() {
        assert_eq!(format_rate(5000.0 * 1024.0 * MB, Units::Bytes), "5000 GB/s");
    }

    #[test]
    fn non_finite_and_negative_render_zero() {
        for v in [f64::NAN, f64::INFINITY, -5.0] {
            assert_eq!(format_rate(v, Units::Bytes), "0 B/s");
        }
    }

    #[test]
    fn lines() {
        assert_eq!(
            widget_lines(Some((9.6 * MB, 512.0 * KB)), Units::Bytes),
            ("↓ 9.6 MB/s".to_string(), "↑ 512 KB/s".to_string())
        );
        assert_eq!(
            widget_lines(None, Units::Bytes),
            ("↓ —".to_string(), "↑ —".to_string())
        );
    }

    #[test]
    fn tooltip_shape() {
        assert_eq!(
            tooltip("Ethernet", Some((9.6 * MB, 512.0 * KB)), Units::Bytes),
            "Ethernet\n↓ 9.6 MB/s  ↑ 512 KB/s"
        );
        assert_eq!(tooltip("Ethernet", None, Units::Bytes), "Ethernet\n↓ —  ↑ —");
    }

    #[test]
    fn tooltip_truncation_respects_surrogates() {
        let label = "😀".repeat(80); // 160 UTF-16 units
        let t = tooltip(&label, None, Units::Bytes);
        let units: Vec<u16> = t.encode_utf16().collect();
        assert!(units.len() <= 127);
        assert!(String::from_utf16(&units).is_ok());
    }
}
