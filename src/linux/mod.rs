//! Linux app: tray icon and --bar output.

pub mod autostart;
pub mod bar;
pub mod icon;
pub mod sysfs;
pub mod tray;

use crate::app::App;
use crate::counters::Selection;
use crate::settings::{load_from, settings_path, Settings};
use bar::Command;

/// Entry point; returns the process exit code.
pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match bar::parse_args(&args) {
        Ok(Command::Help) => {
            print!("{}", bar::USAGE);
            0
        }
        Err(msg) => {
            eprintln!("ferrimeter: {msg}\n\n{}", bar::USAGE);
            2
        }
        Ok(Command::Bar(a)) => {
            // Flags override saved settings for this run only (never saved).
            let mut settings: Settings = settings_path().map(|p| load_from(&p).settings).unwrap_or_default();
            if let Some(u) = a.units {
                settings.units = u;
            }
            if let Some(name) = a.adapter {
                settings.selection = Selection::Adapter(name);
            }
            let mut app = App::new(settings, None);
            let mut out = std::io::stdout().lock();
            bar::run_loop(&mut out, &mut app, a.json, || sysfs::read_adapters().map_err(|_| ()), None)
        }
        Ok(Command::Tray) => tray::run(),
    }
}
