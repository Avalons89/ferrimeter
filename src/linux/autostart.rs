//! "Start at login" via an XDG autostart .desktop file.

use std::fs;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "ferrimeter.desktop";

/// `$XDG_CONFIG_HOME/autostart` (or `~/.config/autostart`).
pub fn autostart_dir() -> Option<PathBuf> {
    crate::settings::config_dir_from(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))
        .map(|d| d.join("autostart"))
}

/// Quotes a path for `Exec=` per the Desktop Entry spec, which applies two
/// layers: inside double quotes `"`, `` ` ``, `$` and `\` get a backslash, and
/// then the whole value is a string where each backslash is itself doubled.
/// So `"` is written `\\"` and `\` is written `\\\\`. `%` starts a field code,
/// so a literal one is `%%`.
fn quote_exec(exe: &Path) -> String {
    let mut out = String::from("\"");
    for ch in exe.to_string_lossy().chars() {
        match ch {
            '"' | '`' | '$' => {
                out.push_str("\\\\");
                out.push(ch);
            }
            '\\' => out.push_str("\\\\\\\\"),
            '%' => out.push_str("%%"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// True when the autostart file exists and launches exactly `exe`, so a stale
/// entry for a moved binary reads as disabled.
pub fn is_enabled(dir: &Path, exe: &Path) -> bool {
    let Ok(text) = fs::read_to_string(dir.join(FILE_NAME)) else { return false };
    let wanted = format!("Exec={}", quote_exec(exe));
    text.lines().any(|line| line == wanted)
}

pub fn set(dir: &Path, exe: &Path, enabled: bool) -> std::io::Result<()> {
    let file = dir.join(FILE_NAME);
    if !enabled {
        return match fs::remove_file(&file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        };
    }
    fs::create_dir_all(dir)?;
    fs::write(
        file,
        format!(
            "[Desktop Entry]\nType=Application\nName=Ferrimeter\nComment=Network speed in the system tray\nExec={}\nX-GNOME-Autostart-enabled=true\n",
            quote_exec(exe)
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ferrimeter-autostart-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn enable_disable_round_trip() {
        let dir = temp_dir("roundtrip");
        let exe = Path::new("/opt/ferrimeter/ferrimeter");
        set(&dir, exe, true).unwrap();
        assert!(is_enabled(&dir, exe));
        let text = fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains("Type=Application\n") && text.contains("Name=Ferrimeter\n"));
        assert!(text.contains("X-GNOME-Autostart-enabled=true\n"));
        set(&dir, exe, false).unwrap();
        assert!(!is_enabled(&dir, exe));
        assert!(!dir.join(FILE_NAME).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn moved_exe_reads_disabled() {
        let dir = temp_dir("moved");
        set(&dir, Path::new("/old/ferrimeter"), true).unwrap();
        assert!(!is_enabled(&dir, Path::new("/new/ferrimeter")));
        set(&dir, Path::new("/new/ferrimeter"), true).unwrap();
        assert!(is_enabled(&dir, Path::new("/new/ferrimeter")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exec_path_with_spaces_and_quotes_round_trips() {
        let dir = temp_dir("quoting");
        let exe = Path::new("/opt/my apps/fer\"ri$meter");
        set(&dir, exe, true).unwrap();
        let text = fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        // Desktop Entry spec: quote-level escape (\" \$) and then string-level
        // escape of that backslash, so the file holds \\" and \\$.
        assert!(text.contains("Exec=\"/opt/my apps/fer\\\\\"ri\\\\$meter\"\n"), "{text}");
        assert!(is_enabled(&dir, exe));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn exec_escapes_backslash_and_percent() {
        let dir = temp_dir("escapes");
        set(&dir, Path::new("/opt/100%/a\\b"), true).unwrap();
        let text = fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        // `\` → `\\` (quote level) → `\\\\` (string level); `%` → `%%` (field codes).
        assert!(text.contains("Exec=\"/opt/100%%/a\\\\\\\\b\"\n"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabling_missing_file_is_ok() {
        let dir = temp_dir("missing");
        assert!(set(&dir, Path::new("/x"), false).is_ok());
    }
}
