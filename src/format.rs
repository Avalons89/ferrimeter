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

/// Adapter names longer than this are cut so the speeds and the data-usage
/// lines (cap included) always fit the 127-character tooltip.
const LABEL_MAX_CHARS: usize = 30;

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

/// Data usage shown in the tooltip: (download, upload) bytes today and this
/// billing period, and the monthly cap in bytes if one is set.
pub struct UsageSummary {
    pub today: (u64, u64),
    pub month: (u64, u64),
    pub cap: Option<u64>,
}

/// A data amount in 1000-based units (as providers bill): "34.5 GB".
pub fn format_bytes_si(bytes: u64) -> String {
    const NAMES: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (value, unit) = scale(bytes as f64, 1000.0, NAMES.len());
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{} {}", number(value), NAMES[unit])
    }
}

/// A cap preset as a whole number: "50 GB", "1 TB".
pub fn format_cap(bytes: u64) -> String {
    const TB: u64 = 1_000_000_000_000;
    if bytes >= TB && bytes % TB == 0 {
        format!("{} TB", bytes / TB)
    } else {
        format!("{} GB", bytes / 1_000_000_000)
    }
}

/// Tooltip lines in order: adapter, speeds, Today, Month (with cap use),
/// Peak. The first two always appear; later lines are added whole only while
/// they fit, so a long name drops Peak first and never cuts a line in half.
pub fn tooltip_full(
    label: &str,
    rates: Option<(f64, f64)>,
    units: Units,
    usage: Option<&UsageSummary>,
    peak: Option<(f64, f64)>,
) -> String {
    let (down, up) = widget_lines(rates, units);
    let mut extra = Vec::new();
    if let Some(u) = usage {
        extra.push(format!("Today ↓ {}  ↑ {}", format_bytes_si(u.today.0), format_bytes_si(u.today.1)));
        let mut month = format!("Month ↓ {}  ↑ {}", format_bytes_si(u.month.0), format_bytes_si(u.month.1));
        if let Some(cap) = u.cap.filter(|&c| c > 0) {
            let used = u.month.0.saturating_add(u.month.1) as f64;
            month.push_str(&format!(" · {:.0}% of {}", used / cap as f64 * 100.0, format_cap(cap)));
        }
        extra.push(month);
    }
    if let Some((pd, pu)) = peak {
        extra.push(format!("Peak ↓ {}  ↑ {}", format_rate(pd, units), format_rate(pu, units)));
    }
    let mut out = fit_tooltip(&format!("{}\n{down}  {up}", shorten(label, LABEL_MAX_CHARS)));
    for line in extra {
        let candidate = format!("{out}\n{line}");
        if candidate.encode_utf16().count() > TOOLTIP_MAX_UTF16 {
            break;
        }
        out = candidate;
    }
    out
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
    fn long_labels_are_shortened() {
        assert_eq!(shorten("Ethernet", 40), "Ethernet");
        let long = "Intel(R) Wi-Fi 6E AX211 160MHz Virtual Adapter #2 (renamed)";
        let short = shorten(long, 40);
        assert_eq!(short.chars().count(), 40);
        assert!(short.ends_with('…'));
    }

    const GB: u64 = 1_000_000_000;

    #[test]
    fn data_amounts_use_1000_based_units() {
        assert_eq!(format_bytes_si(0), "0 B");
        assert_eq!(format_bytes_si(999), "999 B");
        assert_eq!(format_bytes_si(1_000), "1.0 KB");
        assert_eq!(format_bytes_si(80_000_000), "80.0 MB");
        assert_eq!(format_bytes_si(1_234_000_000), "1.2 GB");
        assert_eq!(format_bytes_si(34_500_000_000), "34.5 GB");
        assert_eq!(format_bytes_si(5_000_000_000_000), "5.0 TB");
    }

    #[test]
    fn cap_labels_are_whole_units() {
        assert_eq!(format_cap(50 * GB), "50 GB");
        assert_eq!(format_cap(1_000 * GB), "1 TB");
    }

    fn summary(cap: Option<u64>) -> UsageSummary {
        UsageSummary { today: (1_200_000_000, 80_000_000), month: (34_500_000_000, 2_100_000_000), cap }
    }

    #[test]
    fn full_tooltip_order() {
        assert_eq!(
            tooltip_full("Auto", Some((12.4 * MB, MB)), Units::Bytes, Some(&summary(Some(50 * GB))), Some((45.1 * MB, 3.2 * MB))),
            "Auto\n↓ 12.4 MB/s  ↑ 1.0 MB/s\nToday ↓ 1.2 GB  ↑ 80.0 MB\nMonth ↓ 34.5 GB  ↑ 2.1 GB · 73% of 50 GB\nPeak ↓ 45.1 MB/s  ↑ 3.2 MB/s"
        );
    }

    #[test]
    fn month_line_without_cap() {
        let t = tooltip_full("Auto", None, Units::Bytes, Some(&summary(None)), None);
        assert!(t.ends_with("Month ↓ 34.5 GB  ↑ 2.1 GB"), "{t}");
    }

    #[test]
    fn long_label_drops_whole_trailing_lines() {
        let t = tooltip_full(&"X".repeat(300), Some((MB, MB)), Units::Bytes, Some(&summary(Some(50 * GB))), Some((MB, MB)));
        assert!(t.encode_utf16().count() <= 127, "{}", t.encode_utf16().count());
        assert!(t.contains("· 73% of 50 GB"), "cap info must survive: {t}");
        assert!(!t.contains("Peak"), "Peak goes first, whole: {t}");
        assert!(t.lines().all(|l| !l.ends_with('/')), "no half lines: {t}");
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
