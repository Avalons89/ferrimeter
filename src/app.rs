//! Window-free application state: what each tick displays and what each menu command changes.

use crate::counters::{compute_rates, select, AdapterInfo, Sample, Selection};
use crate::format::{self, Units};
use crate::settings::{log_error, save_json, save_to, Settings, TextSize, Theme};
use crate::usage::{self, CapAlert, NaiveDate, UsageStore};
use std::path::PathBuf;
use std::time::Instant;

#[derive(Clone, Debug, PartialEq)]
pub enum MenuCommand {
    SelectAuto,
    SelectAdapter(String),
    SetUnits(Units),
    SetTheme(Theme),
    SetTextSize(TextSize),
    ToggleAlwaysOnTop,
    ToggleLocked,
    ToggleDock,
    /// Monthly cap in GB; 0 turns it off.
    SetCap(u32),
    /// Billing period start day, 1-28.
    SetBillingDay(u32),
    ToggleVisible,
    ToggleAutostart,
    ResetPosition,
    Exit,
}

pub struct App {
    pub settings: Settings,
    pub settings_path: Option<PathBuf>,
    pub prev: Option<Sample>,
    pub rates: Option<(f64, f64)>,
    pub down: String,
    pub up: String,
    pub tooltip: String,
    /// Highest (down, up) bytes/s seen since start.
    pub peak: (f64, f64),
    /// Bytes (down, up) moved since start.
    pub totals: (u64, u64),
    /// The last reading failed (counters unavailable).
    pub read_failed: bool,
    /// Daily data usage; see `account`.
    pub usage: UsageStore,
    /// Where `usage` is saved (counting) or re-read from (reader); None = memory only.
    pub usage_path: Option<PathBuf>,
    /// True for the one process per PC that adds traffic to `usage`; a second
    /// process (e.g. `--bar` while the tray runs) only re-reads the file.
    pub counting: bool,
    /// Adapter label from the last tick, for rebuilding the tooltip.
    label: String,
    /// `totals` already added to `usage`.
    accounted: (u64, u64),
    account_calls: u64,
}

/// Counting processes save usage this often (in `account` calls, ~1 per second).
const USAGE_SAVE_EVERY: u64 = 60;
/// Reader processes re-read the usage file this often.
const USAGE_RELOAD_EVERY: u64 = 30;
const BYTES_PER_GB: u64 = 1_000_000_000;

impl App {
    pub fn new(settings: Settings, settings_path: Option<PathBuf>) -> App {
        let units = settings.units;
        let (down, up) = format::widget_lines(None, units);
        App {
            settings,
            settings_path,
            prev: None,
            rates: None,
            down,
            up,
            tooltip: format::tooltip("Ferrimeter", None, units),
            peak: (0.0, 0.0),
            totals: (0, 0),
            read_failed: false,
            usage: UsageStore::default(),
            usage_path: None,
            counting: true,
            label: "Ferrimeter".into(),
            accounted: (0, 0),
            account_calls: 0,
        }
    }

    /// One timer tick. A failed read clears the baseline so the next good
    /// reading starts fresh instead of producing a spike.
    pub fn tick(&mut self, adapters: Result<Vec<AdapterInfo>, ()>, now: Instant) {
        let label = match adapters {
            Err(()) => {
                self.read_failed = true;
                self.prev = None;
                self.rates = None;
                "Counters unavailable".to_string()
            }
            Ok(list) => {
                self.read_failed = false;
                let selected = select(&list, &self.settings.selection);
                let cur = Sample { counters: selected.counters, at: now };
                if let Some(prev) = &self.prev {
                    match compute_rates(prev, &cur) {
                        Some(r) => {
                            let secs = now.saturating_duration_since(prev.at).as_secs_f64();
                            self.rates = Some(r);
                            self.peak = (self.peak.0.max(r.0), self.peak.1.max(r.1));
                            self.totals.0 += (r.0 * secs).round() as u64;
                            self.totals.1 += (r.1 * secs).round() as u64;
                        }
                        None => return, // too soon after the last reading; keep it
                    }
                }
                self.prev = Some(cur);
                selected.label
            }
        };
        let units = self.settings.units;
        (self.down, self.up) = format::widget_lines(self.rates, units);
        self.label = label;
        let peak = Some(self.peak).filter(|p| *p != (0.0, 0.0));
        self.tooltip = format::tooltip_full(&self.label, self.rates, units, None, peak);
    }

    fn cap_bytes(&self) -> Option<u64> {
        Some(u64::from(self.settings.cap_gb) * BYTES_PER_GB).filter(|&c| c > 0)
    }

    /// Call once per tick with the local date: adds the bytes moved since the
    /// last call to today's usage (counting process only), saves periodically,
    /// checks the cap, and rebuilds the tooltip with Today/Month lines.
    /// Returns a cap level reached for the first time this billing period.
    pub fn account(&mut self, today: NaiveDate) -> Option<CapAlert> {
        self.account_calls += 1;
        if self.counting {
            let delta = (self.totals.0 - self.accounted.0, self.totals.1 - self.accounted.1);
            if delta != (0, 0) {
                self.usage.add(today, delta.0, delta.1);
            }
            self.accounted = self.totals;
            if self.account_calls % USAGE_SAVE_EVERY == 0 {
                self.save_usage(today);
            }
        } else if self.account_calls % USAGE_RELOAD_EVERY == 1 {
            if let Some(path) = &self.usage_path {
                self.usage = crate::settings::load_json(path).0;
            }
        }
        let billing_day = self.settings.billing_day;
        let cap = self.cap_bytes();
        let alert = if self.counting { self.usage.cap_alert(today, billing_day, cap) } else { None };
        let start = usage::period_start(today, billing_day);
        let month = self.usage.total(start, today);
        if alert.is_some() {
            self.save_usage(today); // remember the announcement across restarts
        }
        let summary = format::UsageSummary { today: self.usage.total(today, today), month, cap };
        let peak = Some(self.peak).filter(|p| *p != (0.0, 0.0));
        self.tooltip = format::tooltip_full(&self.label, self.rates, self.settings.units, Some(&summary), peak);
        alert.map(|level| CapAlert {
            level,
            used: month.0.saturating_add(month.1),
            cap: cap.unwrap_or(0),
            resets: usage::next_reset(today, billing_day),
        })
    }

    /// Saves usage (counting process only), dropping days older than kept.
    pub fn save_usage(&mut self, today: NaiveDate) {
        if !self.counting {
            return;
        }
        self.usage.prune(today);
        if let Some(path) = &self.usage_path {
            if let Err(e) = save_json(path, &self.usage) {
                log_error(&format!("saving usage failed: {e}"));
            }
        }
    }

    /// Applies a settings change. Returns `true` when settings changed (the
    /// caller saves and re-syncs the window); `false` for commands the caller
    /// handles itself (autostart, exit).
    pub fn apply(&mut self, cmd: &MenuCommand) -> bool {
        let s = &mut self.settings;
        match cmd {
            MenuCommand::SelectAuto => s.selection = Selection::Auto,
            MenuCommand::SelectAdapter(guid) => s.selection = Selection::Adapter(guid.clone()),
            MenuCommand::SetUnits(u) => s.units = *u,
            MenuCommand::SetTheme(t) => s.theme = *t,
            MenuCommand::SetTextSize(t) => s.text_size = *t,
            MenuCommand::ToggleAlwaysOnTop => s.always_on_top = !s.always_on_top,
            MenuCommand::ToggleLocked => s.locked = !s.locked,
            MenuCommand::ToggleDock => s.docked = !s.docked,
            MenuCommand::SetCap(gb) => s.cap_gb = *gb,
            MenuCommand::SetBillingDay(day) => s.billing_day = (*day).clamp(1, 28),
            MenuCommand::ToggleVisible => s.visible = !s.visible,
            MenuCommand::ResetPosition => s.position = None,
            MenuCommand::ToggleAutostart | MenuCommand::Exit => return false,
        }
        true
    }

    pub fn save(&self) {
        if let Some(path) = &self.settings_path {
            if let Err(e) = save_to(path, &self.settings) {
                log_error(&format!("saving settings failed: {e}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counters::{AdapterInfo, Selection};
    use crate::format::Units;
    use std::time::{Duration, Instant};

    fn ad(guid: &str, rx: u64, tx: u64) -> AdapterInfo {
        AdapterInfo {
            guid: guid.into(),
            alias: format!("A-{guid}"),
            description: String::new(),
            is_up: true,
            is_hardware: true,
            is_loopback: false,
            is_filter: false,
            in_octets: rx,
            out_octets: tx,
        }
    }

    #[test]
    fn first_tick_shows_dash_then_rate() {
        let mut a = App::new(Settings::default(), None);
        let t = Instant::now();
        a.tick(Ok(vec![ad("a", 1000, 0)]), t);
        assert_eq!(a.down, "↓ —");
        a.tick(Ok(vec![ad("a", 1000 + 2048, 0)]), t + Duration::from_secs(1));
        assert_eq!(a.down, "↓ 2.0 KB/s");
        assert_eq!(
            a.tooltip,
            "Auto\n↓ 2.0 KB/s  ↑ 0 B/s\nPeak ↓ 2.0 KB/s  ↑ 0 B/s"
        );
    }

    #[test]
    fn session_peak_and_totals_accumulate() {
        let mut a = App::new(Settings::default(), None);
        let t = Instant::now();
        a.tick(Ok(vec![ad("a", 0, 0)]), t);
        a.tick(Ok(vec![ad("a", 4096, 100)]), t + Duration::from_secs(1)); // 4 KB/s down
        a.tick(Ok(vec![ad("a", 5120, 300)]), t + Duration::from_secs(2)); // 1 KB/s down
        assert_eq!(a.peak, (4096.0, 200.0));
        assert_eq!(a.totals, (5120, 300));
        // A failed read keeps the session numbers.
        a.tick(Err(()), t + Duration::from_secs(3));
        assert_eq!(a.totals, (5120, 300));
    }

    #[test]
    fn error_clears_baseline() {
        let mut a = App::new(Settings::default(), None);
        let t = Instant::now();
        a.tick(Ok(vec![ad("a", 0, 0)]), t);
        a.tick(Err(()), t + Duration::from_secs(1));
        assert_eq!(a.down, "↓ —");
        assert!(a.tooltip.starts_with("Counters unavailable"));
        a.tick(Ok(vec![ad("a", 9_000_000, 0)]), t + Duration::from_secs(2));
        assert_eq!(a.down, "↓ —");
    }

    fn day(n: u32) -> crate::usage::NaiveDate {
        crate::usage::NaiveDate::from_ymd_opt(2026, 11, n).unwrap()
    }

    #[test]
    fn account_adds_bytes_moved_since_last_call() {
        let mut a = App::new(Settings::default(), None);
        let t = Instant::now();
        a.tick(Ok(vec![ad("a", 0, 0)]), t);
        a.tick(Ok(vec![ad("a", 3_000, 500)]), t + Duration::from_secs(1));
        a.account(day(2));
        a.tick(Ok(vec![ad("a", 5_000, 700)]), t + Duration::from_secs(2));
        a.account(day(2));
        assert_eq!(a.usage.total(day(2), day(2)), (5_000, 700));
        assert!(a.tooltip.contains("Today ↓ 5.0 KB  ↑ 700 B"), "{}", a.tooltip);
        assert!(a.tooltip.contains("Month ↓ 5.0 KB  ↑ 700 B"), "{}", a.tooltip);
    }

    #[test]
    fn reader_mode_never_counts() {
        let mut a = App::new(Settings::default(), None);
        a.counting = false;
        let t = Instant::now();
        a.tick(Ok(vec![ad("a", 0, 0)]), t);
        a.tick(Ok(vec![ad("a", 9_000, 9_000)]), t + Duration::from_secs(1));
        a.account(day(2));
        assert_eq!(a.usage.total(day(1), day(30)), (0, 0));
    }

    #[test]
    fn account_reports_cap_crossing_once() {
        let mut s = Settings::default();
        s.cap_gb = 5;
        let mut a = App::new(s, None);
        a.usage.add(day(2), 4_100_000_000, 0); // 82% of 5 GB
        let alert = a.account(day(2)).expect("80% alert");
        assert_eq!(alert.level, crate::usage::CapLevel::Warn80);
        assert_eq!(alert.used, 4_100_000_000);
        assert_eq!(alert.cap, 5_000_000_000);
        assert!(a.account(day(2)).is_none());
        assert!(a.tooltip.contains("· 82% of 5 GB"), "{}", a.tooltip);
    }

    #[test]
    fn apply_commands() {
        let mut a = App::new(Settings::default(), None);
        assert!(a.apply(&MenuCommand::SetUnits(Units::Bits)));
        assert_eq!(a.settings.units, Units::Bits);
        assert!(a.apply(&MenuCommand::ToggleLocked));
        assert!(a.settings.locked);
        assert!(a.apply(&MenuCommand::SelectAdapter("G".into())));
        assert_eq!(a.settings.selection, Selection::Adapter("G".into()));
        a.settings.position = Some((1, 2));
        assert!(a.apply(&MenuCommand::ResetPosition));
        assert_eq!(a.settings.position, None);
        assert!(!a.settings.docked);
        assert!(a.apply(&MenuCommand::ToggleDock));
        assert!(a.settings.docked);
        assert!(a.apply(&MenuCommand::SetCap(50)));
        assert_eq!(a.settings.cap_gb, 50);
        assert!(a.apply(&MenuCommand::SetBillingDay(15)));
        assert_eq!(a.settings.billing_day, 15);
        assert!(!a.apply(&MenuCommand::ToggleAutostart));
        assert!(!a.apply(&MenuCommand::Exit));
    }
}
