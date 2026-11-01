//! The floating speed widget: a small borderless always-on-top window.
//!
//! All state lives in the boxed `App` stored in GWLP_USERDATA. Win32 calls can
//! re-enter the window procedure, so no borrow of `App` is held across them:
//! callers take a `Settings` snapshot or copy the strings out first.

use crate::app::{App, MenuCommand};
use crate::counters::read_adapters;
use crate::format;
use crate::settings::{log_error, Settings, TextSize, Theme};
use crate::{autostart, tray};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const CLASS_NAME: PCWSTR = w!("netspeed-rs-widget");
pub const WM_APP_TRAY: u32 = WM_APP + 1;
pub const WM_APP_SHOW: u32 = WM_APP + 2;
const TIMER_ID: usize = 1;
const TICK_MS: u32 = 1000;
/// Widest text either unit system produces; sizes the window so it never resizes per tick.
const WIDEST_SAMPLES: [&str; 2] = ["↓ 999.9 MB/s", "↓ 999.9 Mbps"];
const PAD_X: i32 = 6;
const PAD_Y: i32 = 4;
const EDGE_MARGIN: i32 = 8;
/// Background color made fully transparent in the Transparent theme.
const COLOR_KEY: COLORREF = rgb(0x01, 0x01, 0x01);

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

struct Palette {
    bg: COLORREF,
    down: COLORREF,
    up: COLORREF,
}

fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => Palette { bg: rgb(0x20, 0x20, 0x20), down: rgb(0x4C, 0xD9, 0x64), up: rgb(0xFF, 0x9F, 0x43) },
        Theme::Light => Palette { bg: rgb(0xF3, 0xF3, 0xF3), down: rgb(0x1E, 0x8E, 0x3E), up: rgb(0xC2, 0x5E, 0x00) },
        Theme::Transparent => Palette { bg: COLOR_KEY, down: rgb(0x4C, 0xD9, 0x64), up: rgb(0xFF, 0x9F, 0x43) },
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn scale(px: i32, dpi: u32) -> i32 {
    px * dpi as i32 / 96
}

fn point_size(size: TextSize) -> i32 {
    match size {
        TextSize::Small => 9,
        TextSize::Medium => 11,
        TextSize::Large => 13,
    }
}

/// Caller must delete the returned font.
unsafe fn create_font(size: TextSize, dpi: u32) -> HFONT {
    let height = -(point_size(size) * dpi as i32 / 72);
    CreateFontW(
        height, 0, 0, 0, FW_NORMAL.0 as i32, 0, 0, 0,
        DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY,
        (DEFAULT_PITCH.0 | FF_SWISS.0) as u32, w!("Segoe UI"),
    )
}

/// Window size and line height for the current text size and DPI.
unsafe fn measure(hwnd: HWND, s: &Settings) -> (i32, i32, i32) {
    let dpi = GetDpiForWindow(hwnd).max(96);
    let hdc = GetDC(Some(hwnd));
    let font = create_font(s.text_size, dpi);
    let old = SelectObject(hdc, font.into());
    let (mut width, mut line) = (0, 0);
    for sample in WIDEST_SAMPLES {
        let mut size = SIZE::default();
        let _ = GetTextExtentPoint32W(hdc, &wide(sample), &mut size);
        width = width.max(size.cx);
        line = line.max(size.cy);
    }
    SelectObject(hdc, old);
    let _ = DeleteObject(font.into());
    ReleaseDC(Some(hwnd), hdc);
    (width + 2 * scale(PAD_X, dpi), 2 * line + 2 * scale(PAD_Y, dpi), line)
}

unsafe fn default_position(w: i32, h: i32, dpi: u32) -> (i32, i32) {
    let mut work = RECT::default();
    let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut work as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    let m = scale(EDGE_MARGIN, dpi);
    (work.right - w - m, work.bottom - h - m)
}

fn snapshot(hwnd: HWND) -> Option<Settings> {
    with_app(hwnd, |a| a.settings.clone())
}

/// Gives temporary mutable access to the window's `App`. Do not call Win32
/// functions that send messages to this window from inside `f`.
pub fn with_app<R>(hwnd: HWND, f: impl FnOnce(&mut App) -> R) -> Option<R> {
    // SAFETY: GWLP_USERDATA holds either 0 or the Box<App> pointer set in `create`,
    // and every access goes through this single-threaded, non-reentrant helper.
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        ptr.as_mut().map(f)
    }
}

/// Applies size, position (clamped on-screen), z-order, theme and visibility.
pub fn sync(hwnd: HWND, s: &Settings) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd).max(96);
        let (w, h, _) = measure(hwnd, s);
        let (mut x, mut y) = s.position.unwrap_or_else(|| default_position(w, h, dpi));
        let rect = RECT { left: x, top: y, right: x + w, bottom: y + h };
        if MonitorFromRect(&rect, MONITOR_DEFAULTTONULL).is_invalid() {
            (x, y) = default_position(w, h, dpi);
        }
        let z = if s.always_on_top { HWND_TOPMOST } else { HWND_NOTOPMOST };
        let _ = SetWindowPos(hwnd, Some(z), x, y, w, h, SWP_NOACTIVATE);
        let _ = if s.theme == Theme::Transparent {
            SetLayeredWindowAttributes(hwnd, COLOR_KEY, 0, LWA_COLORKEY)
        } else {
            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)
        };
        let _ = ShowWindow(hwnd, if s.visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

pub fn create(app: App) -> windows::core::Result<HWND> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS_NAME,
            ..Default::default()
        };
        if RegisterClassExW(&class) == 0 {
            return Err(windows::core::Error::from_win32());
        }
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            CLASS_NAME,
            w!("netspeed"),
            WS_POPUP,
            0, 0, 1, 1,
            None, None, Some(instance.into()), None,
        )?;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(Box::new(app)) as isize);
        tray::add(hwnd);
        tick(hwnd);
        if let Some(s) = snapshot(hwnd) {
            sync(hwnd, &s);
        }
        SetTimer(Some(hwnd), TIMER_ID, TICK_MS, None);
        Ok(hwnd)
    }
}

fn tick(hwnd: HWND) {
    let adapters = read_adapters().map_err(|e| log_error(&format!("GetIfTable2 failed: {e}")));
    let tip = with_app(hwnd, |a| {
        a.tick(adapters, Instant::now());
        a.tooltip.clone()
    });
    if let Some(tip) = tip {
        tray::set_tooltip(hwnd, &tip);
    }
    unsafe {
        // Windows refuses topmost while a full-screen app has focus, and other
        // apps can push us down; re-assert so "always on top" recovers.
        let want_top = with_app(hwnd, |a| a.settings.always_on_top).unwrap_or(false);
        let is_top = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
        if want_top && !is_top {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

pub fn handle_command(hwnd: HWND, cmd: MenuCommand) {
    match cmd {
        MenuCommand::ToggleAutostart => match std::env::current_exe() {
            Ok(exe) => {
                let enable = !autostart::is_enabled(autostart::VALUE_NAME, &exe);
                if let Err(e) = autostart::set(autostart::VALUE_NAME, &exe, enable) {
                    log_error(&format!("autostart change failed: {e}"));
                }
            }
            Err(e) => log_error(&format!("current_exe failed: {e}")),
        },
        MenuCommand::Exit => unsafe {
            let _ = DestroyWindow(hwnd);
        },
        cmd => {
            let changed = with_app(hwnd, |a| {
                let changed = a.apply(&cmd);
                if changed {
                    a.save();
                    // Re-render immediately so a unit change doesn't wait for the next tick.
                    (a.down, a.up) = format::widget_lines(a.rates, a.settings.units);
                }
                changed
            });
            if changed == Some(true) {
                if let Some(s) = snapshot(hwnd) {
                    sync(hwnd, &s);
                }
            }
        }
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let Some((s, down, up)) = with_app(hwnd, |a| (a.settings.clone(), a.down.clone(), a.up.clone())) else {
        let _ = EndPaint(hwnd, &ps);
        return;
    };
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
    let dpi = GetDpiForWindow(hwnd).max(96);
    let (_, _, line) = measure(hwnd, &s);
    let pal = palette(s.theme);

    // Draw into an off-screen bitmap, then copy once, so the text never flickers.
    let mem = CreateCompatibleDC(Some(hdc));
    let bmp = CreateCompatibleBitmap(hdc, w, h);
    let old_bmp = SelectObject(mem, bmp.into());
    let brush = CreateSolidBrush(pal.bg);
    FillRect(mem, &rc, brush);
    let _ = DeleteObject(brush.into());

    let font = create_font(s.text_size, dpi);
    let old_font = SelectObject(mem, font.into());
    SetBkMode(mem, TRANSPARENT);
    let (x, y) = (scale(PAD_X, dpi), scale(PAD_Y, dpi));
    for (i, (text, color)) in [(down, pal.down), (up, pal.up)].into_iter().enumerate() {
        let text = wide(&text);
        let ty = y + i as i32 * line;
        if s.theme == Theme::Transparent {
            // A 1px dark outline keeps the text readable on any wallpaper.
            SetTextColor(mem, rgb(0, 0, 0));
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let _ = TextOutW(mem, x + dx, ty + dy, &text);
            }
        }
        SetTextColor(mem, color);
        let _ = TextOutW(mem, x, ty, &text);
    }
    let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);

    SelectObject(mem, old_font);
    let _ = DeleteObject(font.into());
    SelectObject(mem, old_bmp);
    let _ = DeleteObject(bmp.into());
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}

fn open_menu(hwnd: HWND, anchor: Option<(i32, i32)>) {
    if let Some(s) = snapshot(hwnd) {
        if let Some(cmd) = tray::show_menu(hwnd, &s, anchor) {
            handle_command(hwnd, cmd);
        }
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCHITTEST => {
                let locked = with_app(hwnd, |a| a.settings.locked).unwrap_or(false);
                LRESULT(if locked { HTCLIENT } else { HTCAPTION } as isize)
            }
            // Caption double-click would try to maximize; a caption right-click
            // would open the system menu. Neither belongs on this widget.
            WM_NCLBUTTONDBLCLK => LRESULT(0),
            WM_NCRBUTTONUP | WM_RBUTTONUP => {
                open_menu(hwnd, None);
                LRESULT(0)
            }
            WM_EXITSIZEMOVE => {
                let mut r = RECT::default();
                if GetWindowRect(hwnd, &mut r).is_ok() {
                    with_app(hwnd, |a| {
                        a.settings.position = Some((r.left, r.top));
                        a.save();
                    });
                }
                LRESULT(0)
            }
            WM_TIMER if wparam.0 == TIMER_ID => {
                tick(hwnd);
                LRESULT(0)
            }
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_DPICHANGED => {
                // Sent mid-drag when crossing onto a monitor with different
                // scaling: follow Windows' suggested spot rather than snapping
                // back to the last saved position.
                let suggested = *(lparam.0 as *const RECT);
                with_app(hwnd, |a| {
                    a.settings.position = Some((suggested.left, suggested.top));
                    a.save();
                });
                if let Some(s) = snapshot(hwnd) {
                    sync(hwnd, &s);
                }
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                if let Some(s) = snapshot(hwnd) {
                    sync(hwnd, &s);
                }
                LRESULT(0)
            }
            WM_APP_SHOW => {
                with_app(hwnd, |a| {
                    a.settings.visible = true;
                    a.save();
                });
                if let Some(s) = snapshot(hwnd) {
                    sync(hwnd, &s);
                }
                let _ = SetForegroundWindow(hwnd);
                LRESULT(0)
            }
            WM_APP_TRAY => {
                // NOTIFYICON_VERSION_4: the event is in LOWORD(lParam).
                let event = (lparam.0 & 0xFFFF) as u32;
                if event == WM_LBUTTONDBLCLK {
                    handle_command(hwnd, MenuCommand::ToggleVisible);
                } else if tray::opens_menu(event) {
                    open_menu(hwnd, Some(tray::anchor_from_wparam(wparam.0)));
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER_ID);
                tray::remove(hwnd);
                let ptr = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut App;
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ if tray::is_taskbar_created(msg) => {
                tray::add(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}
