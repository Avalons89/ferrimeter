//! The floating speed widget: a small borderless always-on-top window.
//!
//! All state lives in the boxed `App` stored in GWLP_USERDATA. Win32 calls can
//! re-enter the window procedure, so no borrow of `App` is held across them:
//! callers take a `Settings` snapshot or copy the strings out first.

use crate::app::{App, MenuCommand};
use crate::win::counters_win::read_adapters;
use crate::format;
use crate::settings::{log_error, Settings, TextSize, Theme};
use crate::win::{autostart, dock, tray};
use std::time::Instant;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const CLASS_NAME: PCWSTR = w!("ferrimeter-widget");
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
        Theme::Dark | Theme::System => Palette { bg: rgb(0x20, 0x20, 0x20), down: rgb(0x4C, 0xD9, 0x64), up: rgb(0xFF, 0x9F, 0x43) },
        Theme::Light => Palette { bg: rgb(0xF3, 0xF3, 0xF3), down: rgb(0x1E, 0x8E, 0x3E), up: rgb(0xC2, 0x5E, 0x00) },
        Theme::Transparent => Palette { bg: COLOR_KEY, down: rgb(0x4C, 0xD9, 0x64), up: rgb(0xFF, 0x9F, 0x43) },
    }
}

/// The taskbar's light/dark setting, which "Follow Windows" mirrors.
fn system_uses_light_theme() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let mut value: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: value/size describe a valid u32 buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok() && value != 0
}

/// Gap between a docked widget and the tray icons.
const DOCK_MARGIN: i32 = 4;

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

/// Converts a GDI-drawn 32-bit buffer for UpdateLayeredWindow: key-colored
/// background becomes alpha 1 (invisible but still clickable, so the whole
/// widget can be dragged), everything else becomes opaque.
fn keyed_to_premultiplied(px: &mut [u32], key: u32) {
    for p in px {
        *p = if *p & 0x00FF_FFFF == key { 0x0100_0000 } else { *p | 0xFF00_0000 };
    }
}

fn rects_overlap(a: &RECT, b: &RECT) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// Main and secondary-monitor taskbars. Only these are re-raised over: other
/// topmost windows (menus, tooltips, other overlays) are never fought.
fn is_taskbar_class(name: &str) -> bool {
    matches!(name, "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

/// True when a visible taskbar sits above us in z-order and overlaps us. The
/// taskbar rises over every always-on-top window when clicked or activated,
/// and our WS_EX_TOPMOST flag stays set, so the flag alone can't detect it.
unsafe fn covered_by_taskbar(hwnd: HWND) -> bool {
    let mut me = RECT::default();
    if GetWindowRect(hwnd, &mut me).is_err() {
        return false;
    }
    let mut next = GetWindow(hwnd, GW_HWNDPREV);
    // Only topmost windows can be above a topmost window, so this walk is short;
    // the cap just guards against a z-order that changes mid-walk.
    for _ in 0..1024 {
        let Ok(above) = next else { break };
        if above.is_invalid() {
            break;
        }
        let mut buf = [0u16; 64];
        let len = GetClassNameW(above, &mut buf).max(0) as usize;
        if IsWindowVisible(above).as_bool() && is_taskbar_class(&String::from_utf16_lossy(&buf[..len])) {
            let mut r = RECT::default();
            if GetWindowRect(above, &mut r).is_ok() && rects_overlap(&me, &r) {
                return true;
            }
        }
        next = GetWindow(above, GW_HWNDPREV);
    }
    false
}

thread_local! {
    /// Which layered mode the window is in: Some(true) = per-pixel alpha
    /// (Transparent theme), Some(false) = whole-window alpha.
    static PER_PIXEL: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
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
        let docked_at = if s.docked { dock::current_target(w, h, scale(DOCK_MARGIN, dpi)) } else { None };
        let (mut x, mut y) = docked_at.or(s.position).unwrap_or_else(|| default_position(w, h, dpi));
        let rect = RECT { left: x, top: y, right: x + w, bottom: y + h };
        if MonitorFromRect(&rect, MONITOR_DEFAULTTONULL).is_invalid() {
            (x, y) = default_position(w, h, dpi);
        }
        let per_pixel = s.theme == Theme::Transparent;
        if PER_PIXEL.get() != Some(per_pixel) {
            // A window can't go from SetLayeredWindowAttributes to
            // UpdateLayeredWindow until WS_EX_LAYERED is cleared and set again.
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_LAYERED.0 as isize));
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_LAYERED.0 as isize);
            PER_PIXEL.set(Some(per_pixel));
        }
        if !per_pixel {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
        }
        // A docked widget must stay on top, or the taskbar would hide it.
        let z = if s.always_on_top || s.docked { HWND_TOPMOST } else { HWND_NOTOPMOST };
        let _ = SetWindowPos(hwnd, Some(z), x, y, w, h, SWP_NOACTIVATE);
        let _ = ShowWindow(hwnd, if s.visible { SW_SHOWNOACTIVATE } else { SW_HIDE });
        redraw(hwnd);
    }
}

/// Repaints: through WM_PAINT normally, or directly for the per-pixel-alpha
/// Transparent theme, which Windows never sends WM_PAINT.
fn redraw(hwnd: HWND) {
    unsafe {
        if PER_PIXEL.get() == Some(true) {
            present_layered(hwnd);
        } else {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
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
        // NOACTIVATE: clicking or dragging the widget never takes focus away
        // from the window you were using (so Alt+F4 still closes that one).
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            CLASS_NAME,
            w!("Ferrimeter"),
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

/// Keeps a docked widget next to the tray as the taskbar moves, resizes or
/// gains tray icons.
fn follow_taskbar(hwnd: HWND) {
    unsafe {
        let Some(s) = snapshot(hwnd) else { return };
        let (w, h, _) = measure(hwnd, &s);
        let dpi = GetDpiForWindow(hwnd).max(96);
        let Some((x, y)) = dock::current_target(w, h, scale(DOCK_MARGIN, dpi)) else { return };
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_ok() && (r.left, r.top) != (x, y) {
            let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

fn tick(hwnd: HWND) {
    let adapters = read_adapters().map_err(|e| log_error(&format!("GetIfTable2 failed: {e}")));
    let result = with_app(hwnd, |a| {
        a.tick(adapters, Instant::now());
        let alert = a.account(crate::usage::local_today());
        (a.tooltip.clone(), alert)
    });
    if let Some((tip, alert)) = result {
        tray::set_tooltip(hwnd, &tip);
        if let Some(alert) = alert {
            tray::notify(hwnd, "Ferrimeter", &alert.message());
        }
    }
    unsafe {
        // Windows refuses topmost while a full-screen app has focus, and the
        // taskbar rises over us when clicked; re-assert so "always on top"
        // recovers within a second either way.
        let (want_top, docked) = with_app(hwnd, |a| (a.settings.always_on_top || a.settings.docked, a.settings.docked)).unwrap_or((false, false));
        if docked {
            follow_taskbar(hwnd);
        }
        let is_top = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0;
        if want_top && (!is_top || covered_by_taskbar(hwnd)) {
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
    redraw(hwnd);
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

/// What to draw, copied out of `App` so no borrow is held during GDI calls.
struct Frame {
    settings: Settings,
    down: String,
    up: String,
}

fn frame(hwnd: HWND) -> Option<Frame> {
    with_app(hwnd, |a| Frame { settings: a.settings.clone(), down: a.down.clone(), up: a.up.clone() })
}

/// Draws background and both lines into `dc`, which holds a w×h bitmap.
unsafe fn draw(hwnd: HWND, dc: HDC, w: i32, h: i32, f: &Frame) {
    let s = &f.settings;
    let dpi = GetDpiForWindow(hwnd).max(96);
    let (_, _, line) = measure(hwnd, s);
    let pal = palette(s.theme.resolve(system_uses_light_theme()));

    let brush = CreateSolidBrush(pal.bg);
    FillRect(dc, &RECT { left: 0, top: 0, right: w, bottom: h }, brush);
    let _ = DeleteObject(brush.into());

    let font = create_font(s.text_size, dpi);
    let old_font = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    let (x, y) = (scale(PAD_X, dpi), scale(PAD_Y, dpi));
    for (i, (text, color)) in [(&f.down, pal.down), (&f.up, pal.up)].into_iter().enumerate() {
        let text = wide(text);
        let ty = y + i as i32 * line;
        if s.theme == Theme::Transparent {
            // A 1px dark outline keeps the text readable on any wallpaper.
            SetTextColor(dc, rgb(0, 0, 0));
            for (dx, dy) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let _ = TextOutW(dc, x + dx, ty + dy, &text);
            }
        }
        SetTextColor(dc, color);
        let _ = TextOutW(dc, x, ty, &text);
    }
    SelectObject(dc, old_font);
    let _ = DeleteObject(font.into());
}

/// Opaque themes: draw off-screen, then copy once, so the text never flickers.
unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    if let Some(f) = frame(hwnd) {
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
        let mem = CreateCompatibleDC(Some(hdc));
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old_bmp = SelectObject(mem, bmp.into());
        draw(hwnd, mem, w, h, &f);
        let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
        SelectObject(mem, old_bmp);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
    }
    let _ = EndPaint(hwnd, &ps);
}

/// Transparent theme: per-pixel alpha via UpdateLayeredWindow.
unsafe fn present_layered(hwnd: HWND) {
    let Some(f) = frame(hwnd) else { return };
    let (w, h, _) = measure(hwnd, &f.settings);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let screen = GetDC(None);
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    if let Ok(bmp) = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
        let mem = CreateCompatibleDC(Some(screen));
        let old_bmp = SelectObject(mem, bmp.into());
        draw(hwnd, mem, w, h, &f);
        let _ = GdiFlush();
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
        keyed_to_premultiplied(px, COLOR_KEY.0);
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let _ = UpdateLayeredWindow(
            hwnd, Some(screen), None, Some(&SIZE { cx: w, cy: h }), Some(mem),
            Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA,
        );
        SelectObject(mem, old_bmp);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(mem);
    }
    ReleaseDC(None, screen);
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
            // Clicking the widget must not steal focus from the user's window.
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => {
                let locked = with_app(hwnd, |a| a.settings.locked || a.settings.docked).unwrap_or(false);
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
                tray::refresh_icon(hwnd);
                with_app(hwnd, |a| {
                    a.settings.position = Some((suggested.left, suggested.top));
                    a.save();
                });
                if let Some(s) = snapshot(hwnd) {
                    sync(hwnd, &s);
                }
                LRESULT(0)
            }
            // Windows light/dark switch: redraw so "Follow Windows" updates at once.
            WM_SETTINGCHANGE if lparam.0 != 0 && PCWSTR(lparam.0 as *const u16).to_string().is_ok_and(|v| v == "ImmersiveColorSet") => {
                redraw(hwnd);
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
                with_app(hwnd, |a| a.save_usage(crate::usage::local_today()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlap_requires_shared_area() {
        let widget = RECT { left: 1556, top: 1030, right: 1653, bottom: 1076 };
        let taskbar = RECT { left: 0, top: 1032, right: 1920, bottom: 1080 };
        assert!(rects_overlap(&widget, &taskbar));
        let above = RECT { left: 1556, top: 978, right: 1653, bottom: 1032 }; // touches the edge only
        assert!(!rects_overlap(&above, &taskbar));
    }

    #[test]
    fn only_taskbar_windows_count() {
        assert!(is_taskbar_class("Shell_TrayWnd"));
        assert!(is_taskbar_class("Shell_SecondaryTrayWnd")); // taskbar on other monitors
        assert!(!is_taskbar_class("#32768")); // menus must never be fought
        assert!(!is_taskbar_class("Chrome_WidgetWin_1"));
    }

    #[test]
    fn key_pixels_become_nearly_transparent_and_rest_opaque() {
        // 0x00BBGGRR as GDI writes it; output is premultiplied 0xAARRGGBB-order DIB words.
        let mut px = [0x0001_0101, 0x0064_D94C, 0x0000_0000];
        keyed_to_premultiplied(&mut px, 0x0001_0101);
        // Alpha 1 (not 0) keeps the background clickable for dragging.
        assert_eq!(px, [0x0100_0000, 0xFF64_D94C, 0xFF00_0000]);
    }
}
