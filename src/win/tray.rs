//! Notification-area icon, tooltip and the shared right-click menu.

use crate::app::MenuCommand;
use crate::win::autostart;
use crate::counters::{menu_adapters, Selection};
use crate::win::counters_win::read_adapters;
use crate::format::Units;
use crate::settings::{Settings, TextSize, Theme};
use crate::usage::{cap_label, CAP_PRESETS_GB};
use crate::win::widget::WM_APP_TRAY;
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
/// Data cap presets use ids 30.., billing days 41..=68 (day = id - 40).
const ID_CAP_BASE: u32 = 30;
const ID_DAY_BASE: u32 = 40;

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

fn copy_tip<const N: usize>(dst: &mut [u16; N], text: &str) {
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
    let color = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
    let mem = CreateCompatibleDC(Some(screen));
    ReleaseDC(None, screen);
    let Ok(color) = color else {
        let _ = DeleteDC(mem);
        return None;
    };
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

    // The alpha channel decides visibility; the mask just has to be defined (all zero).
    let mask_bits = vec![0u8; (((size + 15) / 16) * 2 * size) as usize]; // WORD-aligned rows
    let mask = CreateBitmap(size, size, 1, 1, Some(mask_bits.as_ptr().cast()));
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
        copy_tip(&mut nid.szTip, "Ferrimeter");
        // If Explorer still has our icon (TaskbarCreated without a real
        // restart), ADD fails; MODIFY then applies the fresh icon instead.
        if !Shell_NotifyIconW(NIM_ADD, &nid).as_bool() {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
    }
}

/// Redraws the icon at the current DPI (it would blur after a scaling change).
pub fn refresh_icon(hwnd: HWND) {
    unsafe {
        let Some(icon) = draw_icon(hwnd) else { return };
        let mut nid = base_data(hwnd);
        nid.uFlags = NIF_ICON;
        nid.hIcon = icon;
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        if let Some(old) = ICON.replace(Some(icon)) {
            let _ = DestroyIcon(old);
        }
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

/// Shows a tray notification (Windows toast / balloon) with a warning icon.
pub fn notify(hwnd: HWND, title: &str, text: &str) {
    let mut nid = base_data(hwnd);
    nid.uFlags = NIF_INFO;
    copy_tip(&mut nid.szInfoTitle, title);
    copy_tip(&mut nid.szInfo, text);
    nid.dwInfoFlags = NIIF_WARNING;
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

/// With NOTIFYICON_VERSION_4 one right-click produces WM_RBUTTONUP and then
/// WM_CONTEXTMENU; only the latter (also sent for Shift+F10 / the menu key)
/// opens the menu, so it never opens twice.
pub fn opens_menu(event: u32) -> bool {
    event == WM_CONTEXTMENU
}

/// NOTIFYICON_VERSION_4 passes the anchor point in wParam as two signed
/// 16-bit screen coordinates (negative on monitors left of/above the primary).
pub fn anchor_from_wparam(wparam: usize) -> (i32, i32) {
    let x = (wparam & 0xFFFF) as u16 as i16 as i32;
    let y = ((wparam >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
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
        16 => ToggleDock,
        17 => SetTheme(Theme::System),
        12 => ToggleVisible,
        13 => ToggleAutostart,
        14 => ResetPosition,
        15 => Exit,
        n if (ID_CAP_BASE..ID_CAP_BASE + CAP_PRESETS_GB.len() as u32).contains(&n) => {
            SetCap(CAP_PRESETS_GB[(n - ID_CAP_BASE) as usize])
        }
        n if (ID_DAY_BASE + 1..=ID_DAY_BASE + 28).contains(&n) => SetBillingDay(n - ID_DAY_BASE),
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

/// Shows the menu at `anchor` (or the cursor) and returns the chosen command.
/// Takes a settings snapshot: the menu's modal loop keeps dispatching timer
/// messages, which mutate the live `App`.
pub fn show_menu(hwnd: HWND, s: &Settings, anchor: Option<(i32, i32)>) -> Option<MenuCommand> {
    let all = read_adapters().unwrap_or_default();
    let adapters = menu_adapters(&all);
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
        let caps: Vec<(u32, String, bool)> = CAP_PRESETS_GB
            .iter()
            .enumerate()
            .map(|(i, &gb)| (ID_CAP_BASE + i as u32, cap_label(gb), s.cap_gb == gb))
            .collect();
        let days: Vec<(u32, String, bool)> = (1..=28u32).map(|d| (ID_DAY_BASE + d, d.to_string(), s.billing_day == d)).collect();
        for (label, entries) in [("Data cap", &caps), ("Billing day", &days)] {
            let items: Vec<(u32, &str, bool)> = entries.iter().map(|(id, l, c)| (*id, l.as_str(), *c)).collect();
            submenu(menu, label, &items);
        }
        submenu(menu, "Theme", &[
            (17, "Follow Windows", s.theme == Theme::System),
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
        item(menu, 16, "Dock on taskbar", s.docked);
        item(menu, 11, "Lock position", s.locked);
        item(menu, 12, "Show widget", s.visible);
        item(menu, 13, "Start with Windows", autostart_on);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        item(menu, 14, "Reset position", false);
        item(menu, 15, "Exit", false);

        let mut pt = POINT::default();
        match anchor {
            Some((x, y)) => pt = POINT { x, y },
            None => {
                let _ = GetCursorPos(&mut pt);
            }
        }
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
    fn only_context_menu_event_opens_menu() {
        // NOTIFYICON_VERSION_4 sends WM_RBUTTONUP *and* WM_CONTEXTMENU per right-click.
        assert!(opens_menu(WM_CONTEXTMENU));
        assert!(!opens_menu(WM_RBUTTONUP));
    }

    #[test]
    fn anchor_decodes_signed_coords() {
        // x = -100 (monitor left of primary), y = 50
        let wparam = ((50u32 << 16) | 0xFF9C) as usize;
        assert_eq!(anchor_from_wparam(wparam), (-100, 50));
    }

    #[test]
    fn ids_map() {
        let g = vec!["A".to_string(), "B".to_string()];
        assert_eq!(command_for_id(1, &g), Some(MenuCommand::SelectAuto));
        assert_eq!(command_for_id(1001, &g), Some(MenuCommand::SelectAdapter("B".into())));
        assert_eq!(command_for_id(1002, &g), None);
        assert_eq!(command_for_id(3, &g), Some(MenuCommand::SetUnits(Units::Bits)));
        assert_eq!(command_for_id(6, &g), Some(MenuCommand::SetTheme(Theme::Transparent)));
        assert_eq!(command_for_id(15, &g), Some(MenuCommand::Exit));
        assert_eq!(command_for_id(16, &g), Some(MenuCommand::ToggleDock));
        assert_eq!(command_for_id(30, &g), Some(MenuCommand::SetCap(0)));
        assert_eq!(command_for_id(34, &g), Some(MenuCommand::SetCap(50)));
        assert_eq!(command_for_id(38, &g), Some(MenuCommand::SetCap(1000)));
        assert_eq!(command_for_id(39, &g), None);
        assert_eq!(command_for_id(41, &g), Some(MenuCommand::SetBillingDay(1)));
        assert_eq!(command_for_id(68, &g), Some(MenuCommand::SetBillingDay(28)));
        assert_eq!(command_for_id(69, &g), None);
        assert_eq!(command_for_id(17, &g), Some(MenuCommand::SetTheme(Theme::System)));
        assert_eq!(command_for_id(0, &g), None);
    }
}
