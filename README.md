# netspeed-rs

A tiny always-on-top network speed widget for Windows 10/11, written in Rust.

It shows download (↓) and upload (↑) speed once per second, from Windows' own
per-adapter byte counters. **It makes no network connections of any kind**: no
telemetry, no update checks, no ads, no web pages opened.

## Build

Requires Rust (stable, MSVC) and the Visual Studio C++ build tools.

```
cargo build --release
```

Output: `target\release\netspeed.exe` (single file, no runtime needed). Run `cargo test` for the unit tests.

## Use

- Drag the widget anywhere (unless **Lock position** is on). Its position is remembered.
- Right-click the widget or the tray icon for the menu:
  - **Adapter**: Auto (sum of all connected hardware adapters) or one specific adapter
  - **Units**: Bytes/s or Bits/s
  - **Theme**: Dark, Light, Transparent
  - **Text size**: Small, Medium, Large
  - **Always on top**, **Lock position**, **Show widget**, **Start with Windows**
  - **Reset position**, **Exit**
- Double-click the tray icon to show or hide the widget.
- Launching it again while it runs just brings the widget back.

Settings: `%APPDATA%\netspeed-rs\settings.json`. Errors: `%APPDATA%\netspeed-rs\error.log`.

Windows does not let any window stay above a full-screen app while that app has
focus. The widget re-asserts "always on top" every second, so it returns as soon
as Windows allows (for example after you Alt-Tab).

## Manual test checklist

1. During a large download, widget ↓ is within ~10% of Task Manager's
   Ethernet/Wi-Fi throughput.
2. Drag, exit, relaunch → same position. Lock prevents drag.
3. Each theme renders legibly on light and dark wallpaper.
4. Change display scaling 100% → 150%: text stays sharp and fits.
5. Kill/restart Explorer: tray icon returns.
6. Launch twice: one process, widget shown.
7. Start with Windows on → registry value present; off → removed.
8. Unplug/disable selected adapter → tooltip shows fallback.
9. Task Manager: CPU ~0%, memory < 5 MB.
10. Resource Monitor / `netstat -b`: netspeed.exe has no network connections.
