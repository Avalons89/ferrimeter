//! User settings persisted as JSON under %APPDATA%\ferrimeter, plus the error log.

use crate::counters::Selection;
use crate::format::Units;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Transparent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub selection: Selection,
    pub units: Units,
    pub theme: Theme,
    pub text_size: TextSize,
    pub always_on_top: bool,
    pub locked: bool,
    pub visible: bool,
    /// Widget top-left in screen pixels; `None` means the default spot.
    pub position: Option<(i32, i32)>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            version: 1,
            selection: Selection::default(),
            units: Units::default(),
            theme: Theme::default(),
            text_size: TextSize::default(),
            always_on_top: true,
            locked: false,
            visible: true,
            position: None,
        }
    }
}

pub fn app_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|d| PathBuf::from(d).join("ferrimeter"))
}

pub fn settings_path() -> Option<PathBuf> {
    app_dir().map(|d| d.join("settings.json"))
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Missing file → defaults. Unreadable or invalid file → kept as `<name>.bak`, defaults.
pub fn load_from(path: &Path) -> Settings {
    let Ok(bytes) = fs::read(path) else {
        return Settings::default();
    };
    match serde_json::from_slice(&bytes) {
        Ok(s) => s,
        Err(e) => {
            log_error(&format!("settings file invalid ({e}); using defaults"));
            let _ = fs::rename(path, sibling(path, ".bak"));
            Settings::default()
        }
    }
}

/// Writes to a temp file then renames over the target, so a crash mid-write
/// can't leave a half-written settings file (`fs::rename` replaces atomically on Windows).
pub fn save_to(path: &Path, s: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = sibling(path, ".tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(s)?)?;
    fs::rename(&tmp, path)
}

/// Best-effort append to error.log; never fails or panics.
pub fn log_error(msg: &str) {
    let Some(dir) = app_dir() else { return };
    let _ = fs::create_dir_all(&dir);
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(dir.join("error.log")) {
        let _ = writeln!(f, "{secs} {msg}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::counters::Selection;
    use crate::format::Units;
    use std::fs;

    /// A fresh, empty directory unique to one test.
    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ferrimeter-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn round_trip() {
        let dir = temp_dir("round_trip");
        let p = dir.join("settings.json");
        let mut s = Settings::default();
        s.units = Units::Bits;
        s.position = Some((10, -20));
        s.selection = Selection::Adapter("G".into());
        save_to(&p, &s).unwrap();
        assert_eq!(load_from(&p), s);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_defaults() {
        let dir = temp_dir("missing");
        assert_eq!(load_from(&dir.join("nope.json")), Settings::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_file_backed_up() {
        let dir = temp_dir("corrupt");
        let p = dir.join("settings.json");
        fs::write(&p, "{not json").unwrap();
        assert_eq!(load_from(&p), Settings::default());
        assert!(dir.join("settings.json.bak").exists());
        assert!(!p.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_enum_value_is_corrupt() {
        let dir = temp_dir("bad_enum");
        let p = dir.join("settings.json");
        fs::write(&p, r#"{"units":"Kilo"}"#).unwrap();
        assert_eq!(load_from(&p), Settings::default());
        assert!(dir.join("settings.json.bak").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn partial_and_unknown_fields() {
        let dir = temp_dir("partial");
        let p = dir.join("settings.json");
        fs::write(&p, r#"{"locked":true,"future":42}"#).unwrap();
        let s = load_from(&p);
        assert!(s.locked);
        assert!(s.always_on_top);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_creates_parent_and_leaves_no_tmp() {
        let dir = temp_dir("parent");
        let q = dir.join("sub").join("settings.json");
        save_to(&q, &Settings::default()).unwrap();
        assert!(q.exists());
        assert!(!q.with_extension("json.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
