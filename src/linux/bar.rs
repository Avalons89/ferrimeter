//! `--bar`: one line per second for status bars (waybar, polybar, i3blocks).

use crate::app::App;
use crate::counters::AdapterInfo;
use crate::format::Units;
use std::io::Write;
use std::time::{Duration, Instant};

pub const USAGE: &str = "Usage: ferrimeter [--bar [--json] [--units bytes|bits] [--adapter NAME]]

  (no options)   show the speed in the system tray
  --bar          print the speed once per second, for status bars
  --json         with --bar: waybar JSON ({\"text\",\"tooltip\",\"class\"})
  --units        bytes (MB/s) or bits (Mbps); default from settings
  --adapter      network interface name (e.g. wlan0); default Auto
  --help         show this help
";

#[derive(Debug, Default)]
pub struct BarArgs {
    pub json: bool,
    pub units: Option<Units>,
    pub adapter: Option<String>,
}

#[derive(Debug)]
pub enum Command {
    Tray,
    Bar(BarArgs),
    Help,
}

/// Parses arguments (without the program name).
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    let mut bar = false;
    let mut a = BarArgs::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok(Command::Help),
            "--bar" => bar = true,
            "--json" => a.json = true,
            "--units" => {
                a.units = Some(match it.next().map(String::as_str) {
                    Some("bytes") => Units::Bytes,
                    Some("bits") => Units::Bits,
                    other => return Err(format!("--units needs bytes or bits, got {}", other.unwrap_or("nothing"))),
                })
            }
            "--adapter" => a.adapter = Some(it.next().ok_or("--adapter needs an interface name")?.clone()),
            other => return Err(format!("unknown option: {other}")),
        }
    }
    if !bar && (a.json || a.units.is_some() || a.adapter.is_some()) {
        return Err("--json, --units and --adapter only work with --bar".into());
    }
    Ok(if bar { Command::Bar(a) } else { Command::Tray })
}

/// One output line: plain `↓ 1.2 MB/s ↑ 80.0 KB/s`, or waybar JSON.
pub fn bar_line(app: &App, json: bool) -> String {
    let text = format!("{} {}", app.down, app.up);
    if !json {
        return text;
    }
    serde_json::json!({
        "text": text,
        "tooltip": app.tooltip,
        "class": if app.read_failed { "error" } else { "ok" },
    })
    .to_string()
}

/// Writes one line per second until the output closes (returns 0) or
/// `ticks` lines have been written (tests).
pub fn run_loop<W: Write>(
    out: &mut W,
    app: &mut App,
    json: bool,
    mut read: impl FnMut() -> Result<Vec<AdapterInfo>, ()>,
    ticks: Option<usize>,
) -> i32 {
    let start = Instant::now();
    let mut n = 0usize;
    loop {
        app.tick(read(), Instant::now());
        if let Some(alert) = app.account(crate::usage::local_today()) {
            crate::linux::notify::notify("Ferrimeter", &alert.message());
        }
        // A status bar that exits closes our pipe: that's a normal end, not an error.
        if writeln!(out, "{}", bar_line(app, json)).and_then(|_| out.flush()).is_err() {
            return 0;
        }
        n += 1;
        if ticks.is_some_and(|t| n >= t) {
            return 0;
        }
        // Sleep to the next whole second since start, so ticks don't drift.
        let next = start + Duration::from_secs(n as u64);
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counters::AdapterInfo;
    use crate::settings::Settings;
    use std::time::{Duration, Instant};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn ad(rx: u64, tx: u64) -> AdapterInfo {
        AdapterInfo {
            guid: "eth0".into(),
            alias: "eth0".into(),
            description: String::new(),
            is_up: true,
            is_hardware: true,
            is_loopback: false,
            is_filter: false,
            in_octets: rx,
            out_octets: tx,
        }
    }

    /// App after two readings one second apart: 1.2 MB/s down, 80 KB/s up.
    fn busy_app() -> App {
        let mut app = App::new(Settings::default(), None);
        let t = Instant::now();
        app.tick(Ok(vec![ad(0, 0)]), t);
        app.tick(Ok(vec![ad(1_258_291, 81_920)]), t + Duration::from_secs(1));
        app
    }

    #[test]
    fn parse_variants() {
        assert!(matches!(parse_args(&args(&[])), Ok(Command::Tray)));
        assert!(matches!(parse_args(&args(&["--help"])), Ok(Command::Help)));
        match parse_args(&args(&["--bar"])) {
            Ok(Command::Bar(a)) => assert!(!a.json && a.units.is_none() && a.adapter.is_none()),
            _ => panic!("expected Bar"),
        }
        match parse_args(&args(&["--bar", "--json", "--units", "bits", "--adapter", "wlan0"])) {
            Ok(Command::Bar(a)) => {
                assert!(a.json);
                assert_eq!(a.units, Some(Units::Bits));
                assert_eq!(a.adapter.as_deref(), Some("wlan0"));
            }
            _ => panic!("expected Bar"),
        }
    }

    #[test]
    fn parse_errors() {
        for bad in [&["--json"][..], &["--bar", "--units", "kilo"], &["--bar", "--adapter"], &["--nope"]] {
            assert!(parse_args(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn plain_line() {
        assert_eq!(bar_line(&busy_app(), false), "↓ 1.2 MB/s ↑ 80.0 KB/s");
        assert_eq!(bar_line(&App::new(Settings::default(), None), false), "↓ — ↑ —");
    }

    #[test]
    fn json_line() {
        let v: serde_json::Value = serde_json::from_str(&bar_line(&busy_app(), true)).unwrap();
        assert_eq!(v["text"], "↓ 1.2 MB/s ↑ 80.0 KB/s");
        assert!(v["tooltip"].as_str().unwrap().contains('\n'));
        assert_eq!(v["class"], "ok");
        let mut failed = busy_app();
        failed.tick(Err(()), Instant::now() + Duration::from_secs(2));
        let v: serde_json::Value = serde_json::from_str(&bar_line(&failed, true)).unwrap();
        assert_eq!(v["class"], "error");
    }

    struct BrokenPipe;
    impl std::io::Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn run_loop_stops_cleanly_on_write_error() {
        let mut app = App::new(Settings::default(), None);
        assert_eq!(run_loop(&mut BrokenPipe, &mut app, false, || Ok(vec![ad(0, 0)]), None), 0);
    }

    #[test]
    fn run_loop_writes_one_line_per_tick() {
        let mut app = App::new(Settings::default(), None);
        let mut out = Vec::new();
        assert_eq!(run_loop(&mut out, &mut app, false, || Ok(vec![ad(0, 0)]), Some(2)), 0);
        assert_eq!(String::from_utf8(out).unwrap().lines().count(), 2);
    }

    #[test]
    fn run_loop_counts_usage_when_counting() {
        let mut app = App::new(Settings::default(), None);
        let mut rx = 0;
        let mut out = Vec::new();
        run_loop(&mut out, &mut app, false, || { rx += 4_000; Ok(vec![ad(rx, 0)]) }, Some(3));
        let today = crate::usage::local_today();
        assert_eq!(app.usage.total(today, today).0, 8_000); // two intervals of 4 KB
    }
}
