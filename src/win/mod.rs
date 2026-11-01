//! Windows app: floating widget, tray icon and message loop.

pub mod autostart;
pub mod counters_win;
pub mod dock;
pub mod tray;
pub mod widget;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, FindWindowW, GetMessageW, PostMessageW, TranslateMessage, MSG,
};

pub fn run() {
    std::panic::set_hook(Box::new(|info| crate::settings::log_error(&format!("panic: {info}"))));

    // Held for the life of the process; a second launch sees it and hands off.
    let _instance = unsafe { CreateMutexW(None, true, w!("Local\\ferrimeter-singleton")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if let Ok(existing) = unsafe { FindWindowW(widget::CLASS_NAME, PCWSTR::null()) } {
            let _ = unsafe { PostMessageW(Some(existing), widget::WM_APP_SHOW, WPARAM(0), LPARAM(0)) };
        }
        return;
    }

    let path = crate::settings::settings_path();
    let (loaded, path) = match path {
        Some(p) => {
            let l = crate::settings::load_from(&p);
            // Never save over a settings file we couldn't read.
            (l.settings, Some(p).filter(|_| l.can_save))
        }
        None => (crate::settings::Settings::default(), None),
    };
    let mut app = crate::app::App::new(loaded, path);
    // One instance per PC (mutex above), so this process does the counting.
    (app.usage, app.usage_path) = crate::usage::load();
    if let Err(e) = widget::create(app) {
        crate::settings::log_error(&format!("creating widget failed: {e}"));
        return;
    }

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
