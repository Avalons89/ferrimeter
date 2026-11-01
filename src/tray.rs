//! Notification-area icon, tooltip and the shared right-click menu.

use crate::app::MenuCommand;
use crate::autostart;
use crate::counters::{qualifies, read_adapters, Selection};
use crate::format::Units;
use crate::settings::{Settings, TextSize, Theme};
use crate::widget::WM_APP_TRAY;
use std::cell::Cell;
use std::sync::OnceLock;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const TRAY_ID: u32 = 1;
const ID_AUTO: u32 = 1;
const ID_ADAPTER_BASE: u32 = 1000;

thread_local! {
    static ICON: Cell<Option<HICON>> = const { Cell::new(None) };
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        ..Default::default()
    }
}

fn copy_tip(dst: &mut [u16; 128], text: &str) {
    let mut n = 0;
    for unit in text.encode_utf16().take(dst.len() - 1) {
        dst[n] = unit;
        n += 1;
    }
    dst[n] = 0;
}

/// Draws a ↓↑ glyph pair into a 32-bit icon at the tray's small-icon size.
unsafe fn draw_icon(hwnd: HWND) -> Option<HICON> {
    let dpi = GetDpiForWindow(hwnd).max(96);
    let size = GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(16);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let screen = GetDC(None);
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let color = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    let mem = CreateCompatibleDC(Some(screen));
    ReleaseDC(None, screen);
    let old_bmp = SelectObject(mem, color.into());

    let font = CreateFontW(
        -size, 0, 0, 0, FW_BOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS, NONANTIALIASED_QUALITY, (DEFAULT_PITCH.0 | FF_SWISS.0) as u32, w!("Segoe UI"),
    );
    let old_font = SelectObject(mem, font.into());
    SetBkMode(mem, TRANSPARENT);
    let halves = [(0, "↓", COLORREF(0x0064D94C)), (size / 2, "↑", COLORREF(0x00439FFF))];
    for (left, glyph, color) in halves {
        SetTextColor(mem, color);
        let mut rc = RECT { left, top: 0, right: left + size / 2, bottom: size };
        let mut text: Vec<u16> = glyph.encode_utf16().collect();
        DrawTextW(mem, &mut text, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOCLIP);
    }
    SelectObject(mem, old_font);
    let _ = DeleteObject(font.into());
    SelectObject(mem, old_bmp);
    let _ = DeleteDC(mem);

    // GDI leaves alpha at 0; make every drawn pixel opaque and the rest transparent.
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
    for p in px.iter_mut() {
        *p = if *p & 0x00FF_FFFF != 0 { *p | 0xFF00_0000 } else { 0 };
    }

    let mask = CreateBitmap(size, size, 1, 1, None);
    let info = ICONINFO { fIcon: true.into(), hbmMask: mask, hbmColor: color, ..Default::default() };
    let icon = CreateIconIndirect(&info).ok();
    let _ = DeleteObject(mask.into());
    let _ = DeleteObject(color.into());
    icon
}

/// Adds the icon. Safe to call again (e.g. after Explorer restarts).
pub fn add(hwnd: HWND) {
    unsafe {
        if let Some(old) = ICON.take() {
            let _ = DestroyIcon(old);
        }
        let icon = draw_icon(hwnd);
        ICON.set(icon);
        let mut nid = base_data(hwnd);
        nid.uFlags = NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP | if icon.is_some() { NIF_ICON } else { NOTIFY_ICON_DATA_FLAGS(0) };
        nid.uCallbackMessage = WM_APP_TRAY;
        nid.hIcon = icon.unwrap_or_default();
        copy_tip(&mut nid.szTip, "netspeed");
        let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }
}

pub fn set_tooltip(hwnd: HWND, text: &str) {
    let mut nid = base_data(hwnd);
    nid.uFlags = NIF_TIP | NIF_SHOWTIP;
    copy_tip(&mut nid.szTip, text);
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &base_data(hwnd));
        if let Some(icon) = ICON.take() {
            let _ = DestroyIcon(icon);
        }
    }
}

/// Explorer broadcasts this after it (re)starts; tray icons must be re-added.
pub fn is_taskbar_created(msg: u32) -> bool {
    static ID: OnceLock<u32> = OnceLock::new();
    let id = *ID.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });
    id != 0 && msg == id
}

pub fn command_for_id(id: u32, adapter_guids: &[String]) -> Option<MenuCommand> {
    use MenuCommand::*;
    Some(match id {
        ID_AUTO => SelectAuto,
        2 => SetUnits(Units::Bytes),
        3 => SetUnits(Units::Bits),
        4 => SetTheme(Theme::Dark),
        5 => SetTheme(Theme::Light),
        6 => SetTheme(Theme::Transparent),
        7 => SetTextSize(TextSize::Small),
        8 => SetTextSize(TextSize::Medium),
        9 => SetTextSize(TextSize::Large),
        10 => ToggleAlwaysOnTop,
        11 => ToggleLocked,
        12 => ToggleVisible,
        13 => ToggleAutostart,
        14 => ResetPosition,
        15 => Exit,
        n if n >= ID_ADAPTER_BASE => SelectAdapter(adapter_guids.get((n - ID_ADAPTER_BASE) as usize)?.clone()),
        _ => return None,
    })
}

fn flags(checked: bool) -> MENU_ITEM_FLAGS {
    if checked { MF_STRING | MF_CHECKED } else { MF_STRING }
}

unsafe fn item(menu: HMENU, id: u32, label: &str, checked: bool) {
    // A single '&' would turn the next letter into a keyboard shortcut.
    let _ = AppendMenuW(menu, flags(checked), id as usize, &HSTRING::from(label.replace('&', "&&")));
}

unsafe fn submenu(parent: HMENU, label: &str, items: &[(u32, &str, bool)]) {
    let Ok(sub) = CreatePopupMenu() else { return };
    for &(id, text, checked) in items {
        item(sub, id, text, checked);
    }
    let _ = AppendMenuW(parent, MF_POPUP, sub.0 as usize, &HSTRING::from(label));
}

/// Shows the menu at the cursor and returns the chosen command, if any.
/// Takes a settings snapshot: the menu's modal loop keeps dispatching timer
/// messages, which mutate the live `App`.
pub fn show_menu(hwnd: HWND, s: &Settings) -> Option<MenuCommand> {
    let mut adapters = read_adapters().unwrap_or_default();
    adapters.sort_by_key(|a| !qualifies(a)); // stable: connected hardware first
    let guids: Vec<String> = adapters.iter().map(|a| a.guid.clone()).collect();
    let autostart_on = std::env::current_exe()
        .map(|exe| autostart::is_enabled(autostart::VALUE_NAME, &exe))
        .unwrap_or(false);

    unsafe {
        let menu = CreatePopupMenu().ok()?;

        let adapter_menu = CreatePopupMenu().ok()?;
        item(adapter_menu, ID_AUTO, "Auto", s.selection == Selection::Auto);
        let _ = AppendMenuW(adapter_menu, MF_SEPARATOR, 0, None);
        for (i, a) in adapters.iter().enumerate() {
            let selected = s.selection == Selection::Adapter(a.guid.clone());
            item(adapter_menu, ID_ADAPTER_BASE + i as u32, &format!("{} — {}", a.alias, a.description), selected);
        }
        let _ = AppendMenuW(menu, MF_POPUP, adapter_menu.0 as usize, w!("Adapter"));

        submenu(menu, "Units", &[(2, "Bytes/s", s.units == Units::Bytes), (3, "Bits/s", s.units == Units::Bits)]);
        submenu(menu, "Theme", &[
            (4, "Dark", s.theme == Theme::Dark),
            (5, "Light", s.theme == Theme::Light),
            (6, "Transparent", s.theme == Theme::Transparent),
        ]);
        submenu(menu, "Text size", &[
            (7, "Small", s.text_size == TextSize::Small),
            (8, "Medium", s.text_size == TextSize::Medium),
            (9, "Large", s.text_size == TextSize::Large),
        ]);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        item(menu, 10, "Always on top", s.always_on_top);
        item(menu, 11, "Lock position", s.locked);
        item(menu, 12, "Show widget", s.visible);
        item(menu, 13, "Start with Windows", autostart_on);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        item(menu, 14, "Reset position", false);
        item(menu, 15, "Exit", false);

        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        // Documented tray-menu requirement: foreground first, WM_NULL after,
        // or the menu won't close when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let id = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, None, hwnd, None).0 as u32;
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu); // also destroys the submenus
        command_for_id(id, &guids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::MenuCommand;
    use crate::format::Units;
    use crate::settings::Theme;

    #[test]
    fn ids_map() {
        let g = vec!["A".to_string(), "B".to_string()];
        assert_eq!(command_for_id(1, &g), Some(MenuCommand::SelectAuto));
        assert_eq!(command_for_id(1001, &g), Some(MenuCommand::SelectAdapter("B".into())));
        assert_eq!(command_for_id(1002, &g), None);
        assert_eq!(command_for_id(3, &g), Some(MenuCommand::SetUnits(Units::Bits)));
        assert_eq!(command_for_id(6, &g), Some(MenuCommand::SetTheme(Theme::Transparent)));
        assert_eq!(command_for_id(15, &g), Some(MenuCommand::Exit));
        assert_eq!(command_for_id(0, &g), None);
    }
}
