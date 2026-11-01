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

/// Adapter names longer than this are cut so the speeds always fit the tooltip.
const LABEL_MAX_CHARS: usize = 40;

/// Steps `value` up through units of size `base` once it would print with
/// four digits (999.5 rounds to "1000"), keeping the text narrow.
fn scale(mut value: f64, base: f64, unit_count: usize) -> (f64, usize) {
    let mut unit = 0;
    while value >= 999.5 && unit + 1 < unit_count {
        value /= base;
        unit += 1;
    }
    (value, unit)
}

/// One decimal below 100, none from 100 up; anything rounding to zero is "0".
fn number(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded == 0.0 {
        "0".to_string()
    } else if rounded >= 100.0 {
        format!("{value:.0}")
    } else {
        format!("{rounded:.1}")
    }
}

pub fn format_rate(bytes_per_sec: f64, units: Units) -> String {
    let bytes = if bytes_per_sec.is_finite() && bytes_per_sec > 0.0 {
        bytes_per_sec
    } else {
        0.0
    };
    // Bytes use binary steps; bits use the decimal steps network speeds are quoted in.
    let (value, base, names): (f64, f64, [&str; 4]) = match units {
        Units::Bytes => (bytes, 1024.0, ["B/s", "KB/s", "MB/s", "GB/s"]),
        Units::Bits => (bytes * 8.0, 1000.0, ["bps", "Kbps", "Mbps", "Gbps"]),
    };
    let (value, unit) = scale(value, base, names.len());
    format!("{} {}", number(value), names[unit])
}

/// A data amount such as "1.2 GB". Always in bytes (base 1024): data volumes
/// are quoted in bytes even when speeds are shown in bits.
pub fn format_bytes(bytes: u64) -> String {
    const NAMES: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (value, unit) = scale(bytes as f64, 1024.0, NAMES.len());
    if unit == 0 {
        format!("{bytes} B") // whole bytes never have decimals
    } else {
        format!("{} {}", number(value), NAMES[unit])
    }
}

/// Cuts `text` to at most `max_chars` characters, ending in "…" when cut.
pub fn shorten(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
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
    fit_tooltip(&format!("{}\n{down}  {up}", shorten(label, LABEL_MAX_CHARS)))
}

/// Tooltip with the session's peak speeds and total data moved.
pub fn tooltip_with_session(
    label: &str,
    rates: Option<(f64, f64)>,
    peak: (f64, f64),
    totals: (u64, u64),
    units: Units,
) -> String {
    let (down, up) = widget_lines(rates, units);
    fit_tooltip(&format!(
        "{}\n{down}  {up}\nPeak ↓ {}  ↑ {}\nTotal ↓ {}  ↑ {}",
        shorten(label, LABEL_MAX_CHARS),
        format_rate(peak.0, units),
        format_rate(peak.1, units),
        format_bytes(totals.0),
        format_bytes(totals.1),
    ))
}

/// Keeps whole characters while the UTF-16 length fits the tray's limit.
fn fit_tooltip(full: &str) -> String {
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
    fn rounding_to_1000_steps_up() {
        assert_eq!(format_rate(999.7, Units::Bytes), "1.0 KB/s");
        assert_eq!(format_rate(999.7 * KB, Units::Bytes), "1.0 MB/s");
        assert_eq!(format_rate(124.96, Units::Bits), "1.0 Kbps"); // 999.68 bps
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
    fn byte_totals() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(80 * 1024 * 1024), "80.0 MB");
        assert_eq!(format_bytes(1288490189), "1.2 GB"); // 1.2 GiB
        assert_eq!(format_bytes(5 * 1024u64.pow(4)), "5.0 TB");
    }

    #[test]
    fn long_labels_are_shortened() {
        assert_eq!(shorten("Ethernet", 40), "Ethernet");
        let long = "Intel(R) Wi-Fi 6E AX211 160MHz Virtual Adapter #2 (renamed)";
        let short = shorten(long, 40);
        assert_eq!(short.chars().count(), 40);
        assert!(short.ends_with('…'));
    }

    #[test]
    fn session_tooltip() {
        assert_eq!(
            tooltip_with_session("Auto", Some((9.6 * MB, 512.0 * KB)), (12.4 * MB, KB), (1288490189, 80 * 1024 * 1024), Units::Bytes),
            "Auto
↓ 9.6 MB/s  ↑ 512 KB/s
Peak ↓ 12.4 MB/s  ↑ 1.0 KB/s
Total ↓ 1.2 GB  ↑ 80.0 MB"
        );
    }

    #[test]
    fn session_tooltip_keeps_speeds_with_huge_label() {
        let t = tooltip_with_session(&"X".repeat(300), Some((MB, MB)), (MB, MB), (1, 1), Units::Bytes);
        assert!(t.encode_utf16().count() <= 127);
        assert!(t.contains("↓ 1.0 MB/s  ↑ 1.0 MB/s"));
        assert!(t.contains("Total ↓ 1 B  ↑ 1 B"));
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
