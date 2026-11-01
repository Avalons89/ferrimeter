//! Network adapter byte counters: which adapters to watch and how fast they move.

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Readings closer together than this are too noisy to turn into a rate.
const MIN_ELAPSED: Duration = Duration::from_millis(50);

/// The fields of one `MIB_IF_ROW2` that the widget cares about.
#[derive(Clone, Debug)]
pub struct AdapterInfo {
    pub guid: String,
    pub alias: String,
    pub description: String,
    pub is_up: bool,
    pub is_hardware: bool,
    pub is_loopback: bool,
    pub is_filter: bool,
    pub in_octets: u64,
    pub out_octets: u64,
}

/// Which adapters to measure. `Adapter` holds the adapter's GUID, which survives
/// renames and reboots.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Selection {
    #[default]
    Auto,
    Adapter(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionState {
    Auto,
    Adapter,
    FellBackToAuto,
    NoActiveAdapter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdapterCounter {
    pub guid: String,
    pub rx: u64,
    pub tx: u64,
}

pub struct Selected {
    pub counters: Vec<AdapterCounter>,
    /// The UI shows `label`; `state` is the machine-readable form, checked by tests.
    #[allow(dead_code)]
    pub state: SelectionState,
    pub label: String,
}

pub struct Sample {
    pub counters: Vec<AdapterCounter>,
    pub at: Instant,
}

/// Real, connected hardware: excludes loopback, virtual and filter-driver rows.
pub fn qualifies(a: &AdapterInfo) -> bool {
    a.is_up && a.is_hardware && !a.is_loopback && !a.is_filter
}

/// Rows worth offering in the Adapter menu: real adapters (even while
/// disconnected) and connected virtual ones such as VPNs; never filter-driver
/// or loopback rows, which would only ever show 0. Connected hardware first.
pub fn menu_adapters(adapters: &[AdapterInfo]) -> Vec<&AdapterInfo> {
    let mut list: Vec<&AdapterInfo> = adapters
        .iter()
        .filter(|a| !a.is_filter && !a.is_loopback && (a.is_hardware || a.is_up))
        .collect();
    list.sort_by_key(|a| !qualifies(a)); // stable
    list
}

fn counter(a: &AdapterInfo) -> AdapterCounter {
    AdapterCounter { guid: a.guid.clone(), rx: a.in_octets, tx: a.out_octets }
}

pub fn select(adapters: &[AdapterInfo], sel: &Selection) -> Selected {
    if let Selection::Adapter(guid) = sel {
        if let Some(a) = adapters.iter().find(|a| &a.guid == guid) {
            return Selected {
                counters: vec![counter(a)],
                state: SelectionState::Adapter,
                label: a.alias.clone(),
            };
        }
    }
    let counters: Vec<_> = adapters.iter().filter(|a| qualifies(a)).map(counter).collect();
    let (state, label) = if counters.is_empty() {
        (SelectionState::NoActiveAdapter, "No active adapter")
    } else if matches!(sel, Selection::Adapter(_)) {
        (SelectionState::FellBackToAuto, "Auto (adapter missing)")
    } else {
        (SelectionState::Auto, "Auto")
    };
    Selected { counters, state, label: label.to_string() }
}

/// `IF_TYPE_SOFTWARE_LOOPBACK` from ipifcons.h.
const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
/// Bit positions in `MIB_IF_ROW2.InterfaceAndOperStatusFlags`.
const FLAG_HARDWARE_INTERFACE: u8 = 1 << 0;
const FLAG_FILTER_INTERFACE: u8 = 1 << 1;

fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// Snapshot of every interface Windows knows about, with 64-bit octet counters
/// (IPv4 and IPv6 combined).
pub fn read_adapters() -> windows::core::Result<Vec<AdapterInfo>> {
    use windows::Win32::Foundation::{NO_ERROR, WIN32_ERROR};
    use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
    use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;

    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: GetIfTable2 allocates the table and writes its pointer; we free it below.
    let status: WIN32_ERROR = unsafe { GetIfTable2(&mut table) };
    if status != NO_ERROR {
        return Err(status.into());
    }
    // SAFETY: on success `table` points to a header followed by NumEntries rows,
    // valid until FreeMibTable. Rows are copied out before freeing.
    let list = unsafe {
        let t = &*table;
        let rows = std::slice::from_raw_parts(t.Table.as_ptr(), t.NumEntries as usize);
        let list = rows
            .iter()
            .map(|row| {
                let flags = row.InterfaceAndOperStatusFlags._bitfield;
                AdapterInfo {
                    guid: format!("{:?}", row.InterfaceGuid),
                    alias: wide_to_string(&row.Alias),
                    description: wide_to_string(&row.Description),
                    is_up: row.OperStatus == IfOperStatusUp,
                    is_hardware: flags & FLAG_HARDWARE_INTERFACE != 0,
                    is_loopback: row.Type == IF_TYPE_SOFTWARE_LOOPBACK,
                    is_filter: flags & FLAG_FILTER_INTERFACE != 0,
                    in_octets: row.InOctets,
                    out_octets: row.OutOctets,
                }
            })
            .collect();
        FreeMibTable(table as *const _);
        list
    };
    Ok(list)
}

/// Bytes per second `(down, up)` between two samples.
///
/// Differences are taken per adapter and only for adapters present in both
/// samples, so an adapter appearing mid-run never shows its lifetime total as
/// a spike. A counter that went backwards (reset, reconnect) contributes zero.
pub fn compute_rates(prev: &Sample, cur: &Sample) -> Option<(f64, f64)> {
    let elapsed = cur.at.saturating_duration_since(prev.at);
    if elapsed < MIN_ELAPSED {
        return None;
    }
    let (mut rx, mut tx) = (0u64, 0u64);
    for c in &cur.counters {
        if let Some(p) = prev.counters.iter().find(|p| p.guid == c.guid) {
            rx = rx.saturating_add(c.rx.saturating_sub(p.rx));
            tx = tx.saturating_add(c.tx.saturating_sub(p.tx));
        }
    }
    let secs = elapsed.as_secs_f64();
    Some((rx as f64 / secs, tx as f64 / secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn ad(guid: &str, up: bool, hw: bool, lo: bool, filt: bool, rx: u64, tx: u64) -> AdapterInfo {
        AdapterInfo {
            guid: guid.to_string(),
            alias: format!("A-{guid}"),
            description: format!("Desc {guid}"),
            is_up: up,
            is_hardware: hw,
            is_loopback: lo,
            is_filter: filt,
            in_octets: rx,
            out_octets: tx,
        }
    }

    fn smp(ms: u64, base: Instant, list: &[(&str, u64, u64)]) -> Sample {
        Sample {
            counters: list
                .iter()
                .map(|(g, rx, tx)| AdapterCounter { guid: g.to_string(), rx: *rx, tx: *tx })
                .collect(),
            at: base + Duration::from_millis(ms),
        }
    }

    #[test]
    fn qualifies_truth_table() {
        assert!(qualifies(&ad("1", true, true, false, false, 0, 0)));
        assert!(!qualifies(&ad("1", false, true, false, false, 0, 0)));
        assert!(!qualifies(&ad("1", true, false, false, false, 0, 0)));
        assert!(!qualifies(&ad("1", true, true, true, false, 0, 0)));
        assert!(!qualifies(&ad("1", true, true, false, true, 0, 0)));
    }

    #[test]
    fn auto_takes_all_qualifying() {
        let s = select(
            &[
                ad("a", true, true, false, false, 10, 1),
                ad("b", true, true, false, false, 20, 2),
                ad("c", false, true, false, false, 99, 9),
            ],
            &Selection::Auto,
        );
        assert_eq!(s.counters.iter().map(|c| c.guid.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!((s.state, s.label.as_str()), (SelectionState::Auto, "Auto"));
    }

    #[test]
    fn specific_adapter_any_status() {
        let s = select(
            &[ad("a", true, true, false, false, 1, 1), ad("b", false, true, false, false, 5, 6)],
            &Selection::Adapter("b".into()),
        );
        assert_eq!(s.counters, vec![AdapterCounter { guid: "b".into(), rx: 5, tx: 6 }]);
        assert_eq!((s.state, s.label.as_str()), (SelectionState::Adapter, "A-b"));
    }

    #[test]
    fn missing_adapter_falls_back() {
        let s = select(&[ad("a", true, true, false, false, 1, 1)], &Selection::Adapter("zz".into()));
        assert_eq!(
            (s.state, s.label.as_str(), s.counters.len()),
            (SelectionState::FellBackToAuto, "Auto (adapter missing)", 1)
        );
    }

    #[test]
    fn none_active() {
        let s = select(&[ad("a", false, true, false, false, 1, 1)], &Selection::Auto);
        assert_eq!(
            (s.state, s.label.as_str(), s.counters.len()),
            (SelectionState::NoActiveAdapter, "No active adapter", 0)
        );
    }

    #[test]
    fn rate_uses_measured_elapsed() {
        let t = Instant::now();
        let r = compute_rates(&smp(0, t, &[("a", 1000, 500)]), &smp(1500, t, &[("a", 4000, 2000)])).unwrap();
        assert!((r.0 - 2000.0).abs() < 1e-6 && (r.1 - 1000.0).abs() < 1e-6, "{r:?}");
    }

    #[test]
    fn decreasing_counter_is_zero() {
        let t = Instant::now();
        let r = compute_rates(&smp(0, t, &[("a", 5000, 100)]), &smp(1000, t, &[("a", 10, 300)])).unwrap();
        assert_eq!(r, (0.0, 200.0));
    }

    #[test]
    fn new_adapter_does_not_spike() {
        let t = Instant::now();
        let r = compute_rates(
            &smp(0, t, &[("a", 0, 0)]),
            &smp(1000, t, &[("a", 100, 0), ("vpn", 9_000_000_000, 9_000_000_000)]),
        )
        .unwrap();
        assert_eq!(r, (100.0, 0.0));
    }

    #[test]
    fn menu_adapters_hides_noise() {
        let list = [
            ad("filter", true, true, false, true, 0, 0),
            ad("loop", true, false, true, false, 0, 0),
            ad("wan", false, false, false, false, 0, 0),
            ad("wifi", false, true, false, false, 0, 0),
            ad("eth", true, true, false, false, 0, 0),
            ad("vpn", true, false, false, false, 0, 0),
        ];
        let got: Vec<&str> = menu_adapters(&list).iter().map(|a| a.guid.as_str()).collect();
        assert_eq!(got, ["eth", "wifi", "vpn"]);
    }

    #[test]
    fn read_adapters_smoke() {
        let list = read_adapters().expect("GetIfTable2");
        assert!(!list.is_empty());
        assert!(list.iter().all(|a| !a.guid.is_empty()));
    }

    #[test]
    fn tiny_elapsed_skipped() {
        let t = Instant::now();
        assert!(compute_rates(&smp(0, t, &[("a", 0, 0)]), &smp(49, t, &[("a", 10, 10)])).is_none());
    }
}
