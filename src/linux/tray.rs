//! System tray icon via StatusNotifierItem (KDE, Ubuntu GNOME, XFCE, waybar…).

use crate::app::{App, MenuCommand};
use crate::counters::{menu_adapters, AdapterInfo, Selection};
use crate::format::Units;
use crate::linux::{autostart, icon, sysfs};
use crate::settings::{load_from, log_error, settings_path};
use ksni::blocking::TrayMethods;
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const NO_TRAY_HINT: &str = "No system tray found. On GNOME install the \"AppIndicator and KStatusNotifierItem Support\" extension, or use: ferrimeter --bar";

/// Menu labels for the Adapter submenu ("Auto" first, then `menu_adapters`
/// names) and the index of the current selection (unknown selection → Auto).
/// On Linux an adapter's id is its name, so labels double as ids.
pub fn adapter_entries(adapters: &[AdapterInfo], sel: &Selection) -> (Vec<String>, usize) {
    let mut labels = vec!["Auto".to_string()];
    labels.extend(menu_adapters(adapters).into_iter().map(|a| a.guid.clone()));
    let selected = match sel {
        Selection::Auto => 0,
        Selection::Adapter(name) => labels.iter().skip(1).position(|l| l == name).map_or(0, |i| i + 1),
    };
    (labels, selected)
}

/// The two short lines drawn in the icon; "-" until the first rate exists.
pub fn icon_texts(app: &App) -> (String, String) {
    match app.rates {
        Some((down, up)) => (icon::compact(down, app.settings.units), icon::compact(up, app.settings.units)),
        None => ("-".into(), "-".into()),
    }
}

/// Lock file for "only one tray at a time": `$XDG_RUNTIME_DIR/ferrimeter.lock`,
/// or `/tmp/ferrimeter-<uid>.lock` when that isn't set.
pub fn lock_path(runtime_dir: Option<std::ffi::OsString>, uid: u32) -> PathBuf {
    match runtime_dir.filter(|d| !d.is_empty()) {
        Some(d) => PathBuf::from(d).join("ferrimeter.lock"),
        None => PathBuf::from(format!("/tmp/ferrimeter-{uid}.lock")),
    }
}

/// `O_NOFOLLOW` on x86-64 Linux (asm-generic/fcntl.h); avoids a libc dependency.
const O_NOFOLLOW: i32 = 0o400000;

/// Opens (creating if needed) a lock file safely in a shared directory such
/// as /tmp: never truncates, never follows a symlink, private to this user.
pub fn open_lock_file(path: &std::path::Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new().write(true).create(true).mode(0o600).custom_flags(O_NOFOLLOW).open(path)
}

/// The lock file, from the runtime dir when it's usable, else /tmp.
pub fn acquire_lock_file(runtime_dir: Option<std::ffi::OsString>, uid: u32) -> Result<(File, PathBuf), String> {
    let preferred = lock_path(runtime_dir, uid);
    let fallback = lock_path(None, uid);
    match open_lock_file(&preferred) {
        Ok(f) => Ok((f, preferred)),
        Err(_) if preferred != fallback => open_lock_file(&fallback)
            .map(|f| (f, fallback.clone()))
            .map_err(|e| format!("can't create {}: {e}", fallback.display())),
        Err(e) => Err(format!("can't create {}: {e}", preferred.display())),
    }
}

/// Real user id from /proc (avoids a libc dependency); 0 if unreadable.
fn current_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("Uid:"))?.split_whitespace().nth(1)?.parse().ok())
        .unwrap_or(0)
}

struct FerriTray {
    app: App,
    exe: Option<PathBuf>,
}

impl FerriTray {
    fn apply(&mut self, cmd: MenuCommand) {
        if self.app.apply(&cmd) {
            self.app.save();
            // Re-render now so a unit change shows before the next tick.
            (self.app.down, self.app.up) = crate::format::widget_lines(self.app.rates, self.app.settings.units);
        }
    }

    fn toggle_autostart(&mut self) {
        let (Some(dir), Some(exe)) = (autostart::autostart_dir(), self.exe.as_ref()) else { return };
        let enable = !autostart::is_enabled(&dir, exe);
        if let Err(e) = autostart::set(&dir, exe, enable) {
            log_error(&format!("autostart change failed: {e}"));
        }
    }
}

impl ksni::Tray for FerriTray {
    fn id(&self) -> String {
        "ferrimeter".into()
    }

    fn title(&self) -> String {
        "Ferrimeter".into()
    }

    fn category(&self) -> ksni::Category {
        ksni::Category::ApplicationStatus
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let (down, up) = icon_texts(&self.app);
        [22, 44]
            .into_iter()
            .map(|size| ksni::Icon { width: size, height: size, data: icon::render(&down, &up, size) })
            .collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip { title: "Ferrimeter".into(), description: self.app.tooltip.clone(), ..Default::default() }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let adapters = sysfs::read_adapters().unwrap_or_default();
        let (labels, selected) = adapter_entries(&adapters, &self.app.settings.selection);
        let names = labels.clone();
        let autostart_on = match (autostart::autostart_dir(), self.exe.as_ref()) {
            (Some(dir), Some(exe)) => autostart::is_enabled(&dir, exe),
            _ => false,
        };
        vec![
            SubMenu {
                label: "Adapter".into(),
                submenu: vec![RadioGroup {
                    selected,
                    select: Box::new(move |t: &mut Self, i| {
                        let cmd = match i {
                            0 => MenuCommand::SelectAuto,
                            i => MenuCommand::SelectAdapter(names[i].clone()),
                        };
                        t.apply(cmd);
                    }),
                    options: labels.into_iter().map(|label| RadioItem { label, ..Default::default() }).collect(),
                }
                .into()],
                ..Default::default()
            }
            .into(),
            SubMenu {
                label: "Units".into(),
                submenu: vec![RadioGroup {
                    selected: usize::from(self.app.settings.units == Units::Bits),
                    select: Box::new(|t: &mut Self, i| {
                        t.apply(MenuCommand::SetUnits(if i == 1 { Units::Bits } else { Units::Bytes }))
                    }),
                    options: ["Bytes/s", "Bits/s"].map(|l| RadioItem { label: l.into(), ..Default::default() }).into(),
                }
                .into()],
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Start at login".into(),
                checked: autostart_on,
                activate: Box::new(|t: &mut Self| t.toggle_autostart()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Exit".into(),
                // Settings are saved on every change, so exiting needs no cleanup.
                activate: Box::new(|_: &mut Self| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Runs the tray until Exit; returns the process exit code.
pub fn run() -> i32 {
    let lock_file = match acquire_lock_file(std::env::var_os("XDG_RUNTIME_DIR"), current_uid()) {
        Ok((f, _)) => f,
        Err(msg) => {
            eprintln!("ferrimeter: {msg}");
            return 1;
        }
    };
    if lock_file.try_lock().is_err() {
        println!("Ferrimeter is already running.");
        return 0;
    }

    let make = || {
        let path = settings_path();
        let (settings, path) = match path {
            Some(p) => {
                let l = load_from(&p);
                // Never save over a settings file we couldn't read.
                (l.settings, Some(p).filter(|_| l.can_save))
            }
            None => (Default::default(), None),
        };
        FerriTray { app: App::new(settings, path), exe: std::env::current_exe().ok() }
    };
    let handle = match make().spawn() {
        Ok(h) => h,
        Err(ksni::Error::Watcher(_)) => {
            // No tray host yet (or none at all): keep running so we appear if one starts.
            eprintln!("{NO_TRAY_HINT}");
            match make().assume_sni_available(true).spawn() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("ferrimeter: {e}");
                    return 1;
                }
            }
        }
        Err(e) => {
            eprintln!("ferrimeter: can't start the tray: {e}");
            return 1;
        }
    };

    let start = Instant::now();
    let mut n = 0u64;
    while !handle.is_closed() {
        let reading = sysfs::read_adapters().map_err(|e| log_error(&format!("reading counters failed: {e}")));
        handle.update(|t| t.app.tick(reading, Instant::now()));
        n += 1;
        std::thread::sleep((start + Duration::from_secs(n)).saturating_duration_since(Instant::now()));
    }
    drop(lock_file);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counters::AdapterInfo;
    use crate::settings::Settings;
    use std::time::{Duration, Instant};

    fn ad(name: &str, up: bool, hw: bool, lo: bool) -> AdapterInfo {
        AdapterInfo {
            guid: name.into(),
            alias: name.into(),
            description: String::new(),
            is_up: up,
            is_hardware: hw,
            is_loopback: lo,
            is_filter: false,
            in_octets: 0,
            out_octets: 0,
        }
    }

    #[test]
    fn adapter_entries_lists_auto_first() {
        // sysfs marks virtual Ethernet (bridges, veth) as filter rows; VPNs are not.
        let mut docker = ad("docker0", true, false, false);
        docker.is_filter = true;
        let list = [docker, ad("enp3s0", true, true, false), ad("lo", true, false, true), ad("wg0", true, false, false)];
        let (labels, selected) = adapter_entries(&list, &Selection::Auto);
        assert_eq!(labels, ["Auto", "enp3s0", "wg0"]);
        assert_eq!(selected, 0);
    }

    fn lock_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ferrimeter-lock-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn lock_file_is_private_and_not_truncated() {
        use std::os::unix::fs::PermissionsExt;
        let dir = lock_dir("private");
        let existing = dir.join("old.lock");
        std::fs::write(&existing, "keep").unwrap();
        drop(open_lock_file(&existing).unwrap());
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "keep");
        let fresh = dir.join("new.lock");
        drop(open_lock_file(&fresh).unwrap());
        assert_eq!(std::fs::metadata(&fresh).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_refuses_symlink() {
        let dir = lock_dir("symlink");
        let target = dir.join("target");
        std::fs::write(&target, "victim").unwrap();
        std::os::unix::fs::symlink(&target, dir.join("link.lock")).unwrap();
        assert!(open_lock_file(&dir.join("link.lock")).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "victim");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unusable_runtime_dir_falls_back_to_tmp() {
        let (file, path) = acquire_lock_file(Some("/nonexistent-ferrimeter-runtime".into()), 4_242_424).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/ferrimeter-4242424.lock"));
        drop(file);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn adapter_entries_selected_and_unknown() {
        let list = [ad("enp3s0", true, true, false)];
        assert_eq!(adapter_entries(&list, &Selection::Adapter("enp3s0".into())).1, 1);
        assert_eq!(adapter_entries(&list, &Selection::Adapter("gone".into())).1, 0);
    }

    #[test]
    fn icon_texts_follow_rates() {
        let mut app = App::new(Settings::default(), None);
        assert_eq!(icon_texts(&app), ("-".to_string(), "-".to_string()));
        let t = Instant::now();
        let mut a = ad("eth0", true, true, false);
        app.tick(Ok(vec![a.clone()]), t);
        a.in_octets = 1_258_291;
        a.out_octets = 81_920;
        app.tick(Ok(vec![a]), t + Duration::from_secs(1));
        assert_eq!(icon_texts(&app), ("1.2M".to_string(), "80K".to_string()));
    }

    #[test]
    fn lock_path_prefers_runtime_dir() {
        assert_eq!(lock_path(Some("/run/user/1000".into()), 1000), PathBuf::from("/run/user/1000/ferrimeter.lock"));
        assert_eq!(lock_path(None, 1000), PathBuf::from("/tmp/ferrimeter-1000.lock"));
        assert_eq!(lock_path(Some("".into()), 7), PathBuf::from("/tmp/ferrimeter-7.lock"));
    }
}
