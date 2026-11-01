//! Linux app: tray icon and --bar output.

pub mod sysfs;

/// Entry point; returns the process exit code.
pub fn run() -> i32 {
    eprintln!("ferrimeter: Linux support in progress");
    1
}
