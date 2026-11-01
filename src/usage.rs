//! Data usage: daily totals, billing periods and the monthly cap.

pub use chrono::NaiveDate;
use chrono::{Datelike, Months};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Data cap choices in the menus, in GB; 0 = no cap.
pub const CAP_PRESETS_GB: [u32; 9] = [0, 5, 10, 25, 50, 100, 200, 500, 1000];

/// Menu label for a cap preset.
pub fn cap_label(gb: u32) -> String {
    match gb {
        0 => "Off".into(),
        gb if gb % 1000 == 0 => format!("{} TB", gb / 1000),
        gb => format!("{gb} GB"),
    }
}

/// Loads the usage file for a process: (store, path to save/re-read, or
/// None if the file exists but couldn't be read and must not be overwritten).
pub fn load() -> (UsageStore, Option<PathBuf>) {
    match usage_path() {
        Some(p) => {
            let (store, can_save) = crate::settings::load_json(&p);
            (store, Some(p).filter(|_| can_save))
        }
        None => (UsageStore::default(), None),
    }
}

/// Days of history kept; older days are dropped.
const KEEP_DAYS: i64 = 400;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayTotals {
    pub rx: u64,
    pub tx: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapLevel {
    Warn80,
    Over100,
}

/// A cap level just reached, with what's needed to tell the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapAlert {
    pub level: CapLevel,
    /// Bytes used this period (download + upload).
    pub used: u64,
    pub cap: u64,
    /// First day of the next billing period.
    pub resets: NaiveDate,
}

impl CapAlert {
    pub fn message(&self) -> String {
        use crate::format::{format_bytes_si, format_cap};
        let resets = self.resets.format("%b %-d");
        match self.level {
            CapLevel::Warn80 => format!(
                "80% of your {} data cap used ({}). Resets on {resets}.",
                format_cap(self.cap),
                format_bytes_si(self.used)
            ),
            CapLevel::Over100 => format!(
                "Data cap reached: {} of {} used. Resets on {resets}.",
                format_bytes_si(self.used),
                format_cap(self.cap)
            ),
        }
    }
}

/// Highest cap level already announced, and for which billing period.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Notified {
    period_start: NaiveDate,
    level: CapLevel,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UsageStore {
    days: BTreeMap<NaiveDate, DayTotals>,
    notified: Option<Notified>,
}

/// `usage.json` next to the settings file.
pub fn usage_path() -> Option<PathBuf> {
    crate::settings::app_dir().map(|d| d.join("usage.json"))
}

/// Today in the computer's local time zone.
pub fn local_today() -> NaiveDate {
    chrono::Local::now().date_naive()
}

fn clamp_day(billing_day: u32) -> u32 {
    billing_day.clamp(1, 28)
}

/// First day of the billing period that contains `today`.
pub fn period_start(today: NaiveDate, billing_day: u32) -> NaiveDate {
    let day = clamp_day(billing_day);
    let this_month = today.with_day(day).expect("days 1-28 exist in every month");
    if today.day() >= day {
        this_month
    } else {
        this_month - Months::new(1)
    }
}

/// The day the current billing period ends and the count starts over.
pub fn next_reset(today: NaiveDate, billing_day: u32) -> NaiveDate {
    period_start(today, billing_day) + Months::new(1)
}

impl UsageStore {
    pub fn add(&mut self, date: NaiveDate, rx: u64, tx: u64) {
        let day = self.days.entry(date).or_default();
        day.rx = day.rx.saturating_add(rx);
        day.tx = day.tx.saturating_add(tx);
    }

    /// (download, upload) bytes from `from` through `to`, both inclusive.
    pub fn total(&self, from: NaiveDate, to: NaiveDate) -> (u64, u64) {
        self.days.range(from..=to).fold((0, 0), |(rx, tx), (_, d)| (rx.saturating_add(d.rx), tx.saturating_add(d.tx)))
    }

    pub fn prune(&mut self, today: NaiveDate) {
        let oldest = today - chrono::Duration::days(KEEP_DAYS);
        self.days.retain(|date, _| *date >= oldest);
    }

    /// The cap level newly reached this billing period (download + upload),
    /// at most once per level per period; records it so restarts don't repeat it.
    pub fn cap_alert(&mut self, today: NaiveDate, billing_day: u32, cap_bytes: Option<u64>) -> Option<CapLevel> {
        let cap = cap_bytes.filter(|&c| c > 0)?;
        let start = period_start(today, billing_day);
        let (rx, tx) = self.total(start, today);
        let used = rx.saturating_add(tx) as f64;
        let reached = if used >= cap as f64 {
            CapLevel::Over100
        } else if used >= cap as f64 * 0.8 {
            CapLevel::Warn80
        } else {
            return None;
        };
        let already = match &self.notified {
            Some(n) if n.period_start == start => Some(n.level),
            _ => None,
        };
        let new = match (already, reached) {
            (None, level) => Some(level),
            (Some(CapLevel::Warn80), CapLevel::Over100) => Some(CapLevel::Over100),
            _ => None,
        }?;
        self.notified = Some(Notified { period_start: start, level: new });
        Some(new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn period_starts_on_billing_day() {
        assert_eq!(period_start(d(2026, 11, 20), 15), d(2026, 11, 15));
        assert_eq!(period_start(d(2026, 11, 15), 15), d(2026, 11, 15));
        assert_eq!(period_start(d(2026, 11, 14), 15), d(2026, 10, 15));
        assert_eq!(period_start(d(2026, 1, 3), 15), d(2025, 12, 15)); // across the year
        assert_eq!(period_start(d(2026, 3, 1), 1), d(2026, 3, 1));
    }

    #[test]
    fn billing_day_is_clamped_to_1_through_28() {
        assert_eq!(period_start(d(2026, 2, 28), 31), d(2026, 2, 28));
        assert_eq!(period_start(d(2026, 2, 10), 0), d(2026, 2, 1));
    }

    #[test]
    fn next_reset_is_one_month_after_period_start() {
        assert_eq!(next_reset(d(2026, 11, 20), 15), d(2026, 12, 15));
        assert_eq!(next_reset(d(2026, 12, 20), 15), d(2027, 1, 15));
        assert_eq!(next_reset(d(2026, 11, 14), 15), d(2026, 11, 15));
    }

    #[test]
    fn totals_sum_days_in_range() {
        let mut u = UsageStore::default();
        u.add(d(2026, 11, 1), 100, 10);
        u.add(d(2026, 11, 1), 50, 5);
        u.add(d(2026, 11, 2), 1000, 1);
        u.add(d(2026, 10, 31), 7, 7);
        assert_eq!(u.total(d(2026, 11, 1), d(2026, 11, 1)), (150, 15));
        assert_eq!(u.total(d(2026, 11, 1), d(2026, 11, 30)), (1150, 16));
        assert_eq!(u.total(d(2026, 10, 1), d(2026, 11, 30)), (1157, 23));
    }

    #[test]
    fn prune_keeps_400_days() {
        let mut u = UsageStore::default();
        u.add(d(2025, 1, 1), 1, 1);
        u.add(d(2026, 10, 1), 2, 2);
        u.prune(d(2026, 11, 1));
        assert_eq!(u.total(d(2000, 1, 1), d(2100, 1, 1)), (2, 2));
    }

    #[test]
    fn cap_warns_once_per_level_per_period() {
        let mut u = UsageStore::default();
        let cap = Some(1_000u64);
        u.add(d(2026, 11, 2), 500, 0);
        assert_eq!(u.cap_alert(d(2026, 11, 2), 1, cap), None);
        u.add(d(2026, 11, 3), 300, 50); // 850 = 85%
        assert_eq!(u.cap_alert(d(2026, 11, 3), 1, cap), Some(CapLevel::Warn80));
        assert_eq!(u.cap_alert(d(2026, 11, 3), 1, cap), None); // already told
        u.add(d(2026, 11, 4), 200, 0); // 1050 = 105%
        assert_eq!(u.cap_alert(d(2026, 11, 4), 1, cap), Some(CapLevel::Over100));
        assert_eq!(u.cap_alert(d(2026, 11, 5), 1, cap), None);
        // A new billing period starts over.
        u.add(d(2026, 12, 1), 900, 0);
        assert_eq!(u.cap_alert(d(2026, 12, 1), 1, cap), Some(CapLevel::Warn80));
    }

    #[test]
    fn crossing_both_levels_at_once_reports_only_100() {
        let mut u = UsageStore::default();
        u.add(d(2026, 11, 2), 2_000, 0);
        assert_eq!(u.cap_alert(d(2026, 11, 2), 1, Some(1_000)), Some(CapLevel::Over100));
        assert_eq!(u.cap_alert(d(2026, 11, 2), 1, Some(1_000)), None);
    }

    #[test]
    fn no_cap_never_alerts() {
        let mut u = UsageStore::default();
        u.add(d(2026, 11, 2), u64::MAX / 2, 0);
        assert_eq!(u.cap_alert(d(2026, 11, 2), 1, None), None);
    }

    #[test]
    fn alert_messages() {
        let warn = CapAlert { level: CapLevel::Warn80, used: 40_200_000_000, cap: 50_000_000_000, resets: d(2026, 11, 15) };
        assert_eq!(warn.message(), "80% of your 50 GB data cap used (40.2 GB). Resets on Nov 15.");
        let over = CapAlert { level: CapLevel::Over100, used: 51_300_000_000, cap: 50_000_000_000, resets: d(2026, 12, 1) };
        assert_eq!(over.message(), "Data cap reached: 51.3 GB of 50 GB used. Resets on Dec 1.");
    }

    #[test]
    fn store_round_trips_and_survives_corruption() {
        let dir = std::env::temp_dir().join(format!("ferrimeter-usage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let p = dir.join("usage.json");
        let mut u = UsageStore::default();
        u.add(d(2026, 11, 1), 123, 45);
        u.cap_alert(d(2026, 11, 1), 1, Some(100));
        crate::settings::save_json(&p, &u).unwrap();
        let (mut back, can_save) = crate::settings::load_json::<UsageStore>(&p);
        assert!(can_save);
        assert_eq!(back.total(d(2026, 11, 1), d(2026, 11, 1)), (123, 45));
        assert_eq!(back.cap_alert(d(2026, 11, 1), 1, Some(100)), None, "notified level persists");
        fs::write(&p, "{broken").unwrap();
        let (fresh, can_save) = crate::settings::load_json::<UsageStore>(&p);
        assert!(can_save);
        assert_eq!(fresh.total(d(2000, 1, 1), d(2100, 1, 1)), (0, 0));
        assert!(dir.join("usage.json.bak").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
