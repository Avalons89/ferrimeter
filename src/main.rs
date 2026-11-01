#![windows_subsystem = "windows"]

mod app;
mod autostart;
mod counters;
mod format;
mod settings;
mod tray;
mod widget;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, FindWindowW, GetMessageW, PostMessageW, TranslateMessage, MSG,
};

fn main() {
    std::panic::set_hook(Box::new(|info| settings::log_error(&format!("panic: {info}"))));

    // Held for the life of the process; a second launch sees it and hands off.
    let _instance = unsafe { CreateMutexW(None, true, w!("Local\\netspeed-rs-singleton")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if let Ok(existing) = unsafe { FindWindowW(widget::CLASS_NAME, PCWSTR::null()) } {
            let _ = unsafe { PostMessageW(Some(existing), widget::WM_APP_SHOW, WPARAM(0), LPARAM(0)) };
        }
        return;
    }

    let path = settings::settings_path();
    let loaded = path.as_deref().map(settings::load_from).unwrap_or_default();
    if let Err(e) = widget::create(app::App::new(loaded, path)) {
        settings::log_error(&format!("creating widget failed: {e}"));
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
