//! Where the widget sits when docked on the taskbar.

use windows::core::w;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, FindWindowW, GetWindowRect};

/// A plain rectangle, so the placement maths can be tested without Windows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }
}

/// Top-left for a `w`×`h` widget on the taskbar: just before the tray icons
/// (left of them on a horizontal taskbar, above them on a vertical one),
/// centered across the taskbar's thickness. `None` when the taskbar isn't
/// fully on its monitor (auto-hidden), since there's nothing to sit on.
pub fn dock_position(taskbar: &Rect, tray: Option<&Rect>, monitor: &Rect, w: i32, h: i32, margin: i32) -> Option<(i32, i32)> {
    let on_screen = taskbar.left >= monitor.left
        && taskbar.top >= monitor.top
        && taskbar.right <= monitor.right
        && taskbar.bottom <= monitor.bottom;
    if !on_screen {
        return None;
    }
    let (tw, th) = (taskbar.right - taskbar.left, taskbar.bottom - taskbar.top);
    if tw >= th {
        let anchor = tray.map_or(taskbar.right, |t| t.left);
        Some((anchor - margin - w, taskbar.top + (th - h) / 2))
    } else {
        let anchor = tray.map_or(taskbar.bottom, |t| t.top);
        Some((taskbar.left + (tw - w) / 2, anchor - margin - h))
    }
}

/// Docked position on the primary taskbar right now, if there is one.
pub fn current_target(w: i32, h: i32, margin: i32) -> Option<(i32, i32)> {
    unsafe {
        let taskbar = FindWindowW(w!("Shell_TrayWnd"), None).ok()?;
        let mut tb = RECT::default();
        GetWindowRect(taskbar, &mut tb).ok()?;
        // Still present on Windows 11 and covers the tray icons and clock.
        let tray = FindWindowExW(Some(taskbar), None, w!("TrayNotifyWnd"), None)
            .ok()
            .and_then(|t| {
                let mut r = RECT::default();
                GetWindowRect(t, &mut r).ok().map(|_| Rect::from(r))
            })
            .filter(|r| r.right > r.left && r.bottom > r.top);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(taskbar, MONITOR_DEFAULTTOPRIMARY), &mut info).as_bool() {
            return None;
        }
        dock_position(&tb.into(), tray.as_ref(), &info.rcMonitor.into(), w, h, margin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    const MONITOR: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };

    #[test]
    fn bottom_taskbar_sits_left_of_tray_centered_vertically() {
        let taskbar = r(0, 1032, 1920, 1080);
        let tray = r(1621, 1032, 1920, 1080);
        // 97x46 widget, 4 px gap before the tray icons
        assert_eq!(dock_position(&taskbar, Some(&tray), &MONITOR, 97, 46, 4), Some((1520, 1033)));
    }

    #[test]
    fn without_tray_info_uses_taskbar_end() {
        let taskbar = r(0, 1032, 1920, 1080);
        assert_eq!(dock_position(&taskbar, None, &MONITOR, 97, 46, 4), Some((1819, 1033)));
    }

    #[test]
    fn vertical_taskbar_sits_above_tray_centered_horizontally() {
        let taskbar = r(0, 0, 62, 1080); // left edge, Windows 10
        let tray = r(0, 800, 62, 1080);
        assert_eq!(dock_position(&taskbar, Some(&tray), &MONITOR, 50, 40, 4), Some((6, 756)));
    }

    #[test]
    fn auto_hidden_taskbar_is_not_docked_to() {
        let hidden = r(0, 1078, 1920, 1126); // slid mostly off-screen
        assert_eq!(dock_position(&hidden, None, &MONITOR, 97, 46, 4), None);
    }
}
