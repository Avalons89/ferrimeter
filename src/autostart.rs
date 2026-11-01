//! "Start with Windows" via the per-user Run registry key.

use std::path::Path;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, NO_ERROR, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegGetValueW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ,
};

pub const VALUE_NAME: &str = "netspeed-rs";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

fn check(status: WIN32_ERROR) -> windows::core::Result<()> {
    if status == NO_ERROR {
        Ok(())
    } else {
        Err(status.into())
    }
}

fn read_value(value_name: &str) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut bytes = (buf.len() * 2) as u32;
    // SAFETY: buf is valid for `bytes` bytes; RegGetValueW NUL-terminates REG_SZ output.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(RUN_KEY),
            &HSTRING::from(value_name),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status != NO_ERROR {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

/// True only if the Run value points at `exe`, so a stale entry for a moved
/// exe reads as disabled.
pub fn is_enabled(value_name: &str, exe: &Path) -> bool {
    read_value(value_name).is_some_and(|v| {
        v.trim().trim_matches('"').to_lowercase() == exe.to_string_lossy().to_lowercase()
    })
}

pub fn set(value_name: &str, exe: &Path, enabled: bool) -> windows::core::Result<()> {
    let mut key = HKEY::default();
    // SAFETY: plain registry calls with owned wide strings; the key is closed below.
    unsafe {
        check(RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(RUN_KEY), None, KEY_SET_VALUE, &mut key))?;
        let name = HSTRING::from(value_name);
        let result = if enabled {
            let data: Vec<u16> = format!("\"{}\"", exe.display()).encode_utf16().chain([0]).collect();
            let bytes = std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), data.len() * 2);
            check(RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)))
        } else {
            match RegDeleteValueW(key, PCWSTR(name.as_ptr())) {
                ERROR_FILE_NOT_FOUND => Ok(()),
                status => check(status),
            }
        };
        let _ = RegCloseKey(key);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "netspeed-rs-test-a";
    const T2: &str = "netspeed-rs-test-b";

    #[test]
    fn enable_disable_round_trip() {
        let exe = Path::new(r"C:\Tools\netspeed.exe");
        set(T, exe, true).unwrap();
        assert!(is_enabled(T, exe));
        assert!(is_enabled(T, Path::new(r"c:\tools\NETSPEED.EXE")));
        set(T, exe, false).unwrap();
        assert!(!is_enabled(T, exe));
        set(T, exe, false).unwrap();
    }

    #[test]
    fn moved_exe_reads_disabled() {
        set(T2, Path::new(r"C:\Old\netspeed.exe"), true).unwrap();
        assert!(!is_enabled(T2, Path::new(r"C:\New\netspeed.exe")));
        set(T2, Path::new(r"C:\New\netspeed.exe"), true).unwrap();
        assert!(is_enabled(T2, Path::new(r"C:\New\netspeed.exe")));
        set(T2, Path::new(r"C:\New\netspeed.exe"), false).unwrap();
    }
}
