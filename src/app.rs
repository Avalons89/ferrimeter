//! Window-free application state: what each tick displays and what each menu command changes.

use crate::counters::{compute_rates, select, AdapterInfo, Sample, Selection};
use crate::format::{self, Units};
use crate::settings::{log_error, save_to, Settings, TextSize, Theme};
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
}

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
        }
    }

    /// One timer tick. A failed read clears the baseline so the next good
    /// reading starts fresh instead of producing a spike.
    pub fn tick(&mut self, adapters: Result<Vec<AdapterInfo>, ()>, now: Instant) {
        let label = match adapters {
            Err(()) => {
                self.prev = None;
                self.rates = None;
                "Counters unavailable".to_string()
            }
            Ok(list) => {
                let selected = select(&list, &self.settings.selection);
                let cur = Sample { counters: selected.counters, at: now };
                if let Some(prev) = &self.prev {
                    match compute_rates(prev, &cur) {
                        Some(r) => self.rates = Some(r),
                        None => return, // too soon after the last reading; keep it
                    }
                }
                self.prev = Some(cur);
                selected.label
            }
        };
        let units = self.settings.units;
        (self.down, self.up) = format::widget_lines(self.rates, units);
        self.tooltip = format::tooltip(&label, self.rates, units);
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
        assert_eq!(a.tooltip, "Auto\n↓ 2.0 KB/s  ↑ 0 B/s");
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
        assert!(!a.apply(&MenuCommand::ToggleAutostart));
        assert!(!a.apply(&MenuCommand::Exit));
    }
}
