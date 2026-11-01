//! Desktop notifications via the freedesktop Notifications service (local D-Bus).

use std::collections::HashMap;
use zbus::zvariant::Value;

/// Shows a desktop notification. Failures (no notification daemon, no session
/// bus) are logged and otherwise ignored.
pub fn notify(summary: &str, body: &str) {
    let result = zbus::blocking::Connection::session().and_then(|conn| {
        conn.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Ferrimeter",
                0u32,                      // replaces_id: new notification
                "network-transmit-receive", // icon name from the icon theme
                summary,
                body,
                Vec::<&str>::new(),                  // actions
                HashMap::<&str, Value<'_>>::new(),   // hints
                -1i32,                               // default timeout
            ),
        )
    });
    if let Err(e) = result {
        crate::settings::log_error(&format!("desktop notification failed: {e}"));
    }
}
