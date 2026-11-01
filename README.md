# Ferrimeter

A tiny always-on-top network speed widget for Windows 10 and 11, written in Rust.

```
↓ 12.4 MB/s
↑ 512 KB/s
```

It shows your download and upload speed once per second in a small box you can
drag anywhere, with a tray icon for settings. It reads Windows' own adapter byte
counters and **never connects to the network itself**: no telemetry, no update
checks, no ads, no web pages opened. One file of about 350 KB with no runtime to
install, using about 1 MB of RAM and effectively no CPU.

## Features

- Live download/upload speed, updated every second
- Floating, borderless, always-on-top widget; drag it anywhere, or lock it in place
- Tray icon with the current speeds in its tooltip
- **Auto** mode adds up all connected network adapters, or pick one specific adapter
- Bytes per second (KB/s, MB/s) or bits per second (Kbps, Mbps)
- Dark, Light and Transparent themes; three text sizes
- Sharp text at any display scaling, including mixed-DPI multi-monitor setups
- Optional start with Windows
- Settings and position remembered between runs

## Install

1. Download `ferrimeter.exe` from the [latest release](https://github.com/Avalons89/ferrimeter/releases/latest).
2. Put it somewhere permanent, for example `%LOCALAPPDATA%\Programs\Ferrimeter\`.
3. Run it. To have it start automatically, right-click the widget and turn on
   **Start with Windows**.

Nothing else needs to be installed. To update, exit Ferrimeter and replace the file.

The exe isn't code-signed yet, so Windows SmartScreen may say "Windows protected
your PC" the first time. Click **More info → Run anyway**. You can check the file
against the published checksum first:

```
Get-FileHash ferrimeter.exe -Algorithm SHA256
```

The result should match `ferrimeter.exe.sha256` on the release page. Releases are
built from the tagged source in this repository.

You can also [build it yourself](#build).

## Use

- **Move it:** drag the widget. Its position is saved.
- **Settings:** right-click the widget or the tray icon:

  | Menu | What it does |
  |---|---|
  | Adapter | Auto, or one specific adapter |
  | Units | Bytes/s or Bits/s |
  | Theme | Dark, Light, Transparent |
  | Text size | Small, Medium, Large |
  | Always on top | Keep the widget above other windows |
  | Lock position | Stop the widget from being dragged |
  | Show widget | Hide or show the widget (the tray icon stays) |
  | Start with Windows | Launch automatically when you sign in |
  | Reset position | Move the widget back to the bottom-right corner |
  | Exit | Quit |

- **Hide or show:** double-click the tray icon.
- Starting it again while it's already running just brings the widget back.

Windows doesn't let any window stay above a full-screen app (such as a game) while
that app has focus. The widget re-applies "always on top" every second, so it comes
back as soon as you switch away.

## How it works

Every second the app reads the 64-bit byte counters Windows keeps for each network
adapter (`GetIfTable2`, which counts IPv4 and IPv6 traffic), subtracts the previous
reading, and divides by the time that actually elapsed. Differences are taken per
adapter, so an adapter that connects mid-run (a VPN, Wi-Fi reconnecting) never
shows up as a fake spike, and a counter that resets counts as zero.

In Auto mode, adapters count when they're connected, are real hardware, and aren't
loopback or filter-driver entries. The numbers match Task Manager's network graph.

Unit steps: bytes use 1024 (KB/s, MB/s); bits use 1000 (Kbps, Mbps), the way network
speeds are usually quoted.

## Privacy

Ferrimeter makes no network connections. The executable doesn't even import
Windows' networking libraries (`ws2_32`, `winhttp`, `wininet`); the only
network-related call it makes is reading the adapter counters. It stores two files,
both in `%APPDATA%\ferrimeter`:

- `settings.json`: your choices and the widget position
- `error.log`: rare errors, if any

"Start with Windows" adds one value, `ferrimeter`, under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; turning it off removes it.

## Build

Requirements:

- Windows 10 1809 or later
- [Rust](https://rustup.rs) (stable, MSVC toolchain)
- Visual Studio Build Tools with the "Desktop development with C++" workload (for the linker)

```
git clone https://github.com/Avalons89/ferrimeter.git
cd ferrimeter
cargo build --release
```

The result is `target\release\ferrimeter.exe`. The C runtime is linked statically, so
the exe runs without the Visual C++ redistributable.

Run the tests with:

```
cargo test
```

Note: two tests briefly write and remove test values under the Run registry key
above, and the settings tests may append a line to `error.log`.

## Project layout

| File | Purpose |
|---|---|
| `src/format.rs` | Turns byte rates into text like `12.4 MB/s` |
| `src/counters.rs` | Reads adapter counters; picks adapters; computes rates |
| `src/settings.rs` | Loads and saves `settings.json` |
| `src/autostart.rs` | Start with Windows (Run registry key) |
| `src/app.rs` | App state: what each tick shows, what each menu item changes |
| `src/widget.rs` | The floating window: drawing, dragging, themes, DPI |
| `src/tray.rs` | Tray icon, tooltip and menu |
| `src/main.rs` | Startup, single-instance check, message loop |

Everything outside `widget.rs`, `tray.rs` and `main.rs` is plain logic with unit tests.

## Manual test checklist

Window behavior isn't covered by unit tests. Before a release, check:

1. During a large download, the widget's ↓ is within about 10% of Task Manager's figure.
2. Drag, exit, relaunch: same position. Lock position prevents dragging.
3. Each theme is readable on light and dark wallpaper.
4. At 150% display scaling, text stays sharp and fits.
5. After restarting Explorer, the tray icon comes back.
6. Launching twice leaves one process, with the widget shown.
7. Start with Windows on: the registry value exists. Off: it's removed.
8. Unplugging or disabling the selected adapter makes the tooltip show the fallback.
9. Task Manager: about 0% CPU, under 5 MB memory.
10. Resource Monitor shows no network connections for ferrimeter.exe.
11. With two monitors at different scaling, dragging across works and the widget stays where dropped.
12. Right-clicking the tray icon opens exactly one menu, at the icon.

## License

[MIT](LICENSE) © 2026 Michael Florentio
