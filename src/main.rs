#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod counters;
mod format;
mod settings;
mod usage;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    win::run();
    #[cfg(target_os = "linux")]
    std::process::exit(linux::run());
}
