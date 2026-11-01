# Ferrimeter

A tiny network speed meter for Windows 10/11 and Linux, written in Rust.

```
↓ 12.4 MB/s
↑ 512 KB/s
```

It shows your download and upload speed once per second: on Windows in a small
always-on-top box you can drag anywhere or dock on the taskbar, on Linux in the
system tray or your status bar. It reads the operating system's own network byte
counters and **never connects to the network itself**: no telemetry, no update
checks, no ads, no web pages opened. One small file with no runtime to install,
using about 1 MB of RAM and effectively no CPU.

## Features (Windows)

- Live download/upload speed, updated every second
- Floating, borderless, always-on-top widget; drag it anywhere, or lock it in place
- **Dock on taskbar**: sits right next to the tray icons and follows the taskbar
- Tray icon whose tooltip shows current speeds, the session's peak, and total data moved
- **Auto** mode adds up all connected network adapters, or pick one specific adapter
- Bytes per second (KB/s, MB/s) or bits per second (Kbps, Mbps)
- Follow Windows (matches the taskbar's light/dark setting), Dark, Light and Transparent themes; three text sizes
- Sharp text at any display scaling, including mixed-DPI multi-monitor setups
- Optional start with Windows
- Settings and position remembered between runs

## Windows

### Install

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

### Use

- **Move it:** drag the widget. Its position is saved.
- **Settings:** right-click the widget or the tray icon:

  | Menu | What it does |
  |---|---|
  | Adapter | Auto, or one specific adapter |
  | Units | Bytes/s or Bits/s |
  | Theme | Follow Windows, Dark, Light, Transparent |
  | Text size | Small, Medium, Large |
  | Always on top | Keep the widget above other windows |
  | Dock on taskbar | Sit on the taskbar next to the tray icons (can't be dragged while docked) |
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

## Linux

Works with KDE Plasma, GNOME (Ubuntu), Cinnamon and XFCE, and with status bars
such as waybar, polybar and i3blocks. One static x86-64 binary that runs on any
distribution, with no dependencies.

### Install

Download `ferrimeter-linux-x86_64.tar.gz` from the
[latest release](https://github.com/Avalons89/ferrimeter/releases/latest), then:

```
sha256sum -c ferrimeter-linux-x86_64.tar.gz.sha256
tar -xzf ferrimeter-linux-x86_64.tar.gz
mkdir -p ~/.local/bin ~/.local/share/applications
cp ferrimeter-linux-x86_64/ferrimeter ~/.local/bin/
# optional: an app-menu entry that points at the installed binary
sed "s|^Exec=.*|Exec=\"$HOME/.local/bin/ferrimeter\"|" ferrimeter-linux-x86_64/ferrimeter.desktop > ~/.local/share/applications/ferrimeter.desktop
```

If `~/.local/bin` isn't on your `PATH` (some distributions don't add it, and others
only after you log in again), run it as `~/.local/bin/ferrimeter` and use that full
path in status-bar configs.

### Tray icon

Run `ferrimeter`. The icon shows the download speed on top (green) and the upload
speed below (orange), for example `1.2M` over `80K`. Hover for exact speeds, the
session's peak and total data moved. Right-click for:

| Menu | What it does |
|---|---|
| Adapter | Auto, or one specific network interface |
| Units | Bytes/s or Bits/s |
| Start at login | Adds or removes `~/.config/autostart/ferrimeter.desktop` |
| Exit | Quit |

- **KDE Plasma, Cinnamon, XFCE:** works out of the box.
- **GNOME:** needs the "AppIndicator and KStatusNotifierItem Support" extension,
  which Ubuntu ships enabled. Without a tray, Ferrimeter says so and waits for one.
- Only one tray icon runs at a time.

### Status bars

`ferrimeter --bar` prints one line per second, such as `↓ 1.2 MB/s ↑ 80.0 KB/s`.
Add `--json` for waybar, `--units bits` for Mbps, or `--adapter wlan0` for one
interface (`ferrimeter --help` lists everything).

waybar (`config`):

```
"custom/ferrimeter": {
    "exec": "ferrimeter --bar --json",
    "return-type": "json"
}
```

polybar:

```
[module/ferrimeter]
type = custom/script
exec = ferrimeter --bar
tail = true
```

i3blocks:

```
[ferrimeter]
command=ferrimeter --bar
interval=persist
```

The Linux version is tested on a desktop-less test setup, not yet on every desktop.
If something looks wrong on yours, please
[open an issue](https://github.com/Avalons89/ferrimeter/issues).

## How it works

Every second the app reads the 64-bit byte counters Windows keeps for each network
adapter (`GetIfTable2`, which counts IPv4 and IPv6 traffic), subtracts the previous
reading, and divides by the time that actually elapsed. Differences are taken per
adapter, so an adapter that connects mid-run (a VPN, Wi-Fi reconnecting) never
shows up as a fake spike, and a counter that resets counts as zero.

In Auto mode, adapters count when they're connected, are real hardware, and aren't
loopback or filter-driver entries. The numbers match Task Manager's network graph.

On Linux the counters come from `/sys/class/net/<interface>/statistics`. Auto counts
physical interfaces that are up; Docker bridges, `veth` pairs, VPN tunnels and `lo`
are left out, though VPNs can be picked from the Adapter menu.

Unit steps: bytes use 1024 (KB/s, MB/s); bits use 1000 (Kbps, Mbps), the way network
speeds are usually quoted.

## Privacy

Ferrimeter makes no network connections. On Windows the executable doesn't even
import the networking libraries (`ws2_32`, `winhttp`, `wininet`); the only
network-related call it makes is reading the adapter counters. On Linux it only
reads files under `/sys` and talks to the desktop's tray over the local D-Bus.
It stores two files, in `%APPDATA%\ferrimeter` on Windows or `~/.config/ferrimeter`
on Linux:

- `settings.json`: your choices and the widget position
- `error.log`: rare errors, if any

"Start with Windows" adds one value, `ferrimeter`, under
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; turning it off removes it.
On Linux, "Start at login" writes `~/.config/autostart/ferrimeter.desktop`.

## Build

### Windows

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

### Linux

Requirements: [Rust](https://rustup.rs) (stable) and, for the static build, the
musl target and tools:

```
rustup target add x86_64-unknown-linux-musl
sudo apt install musl-tools          # Debian/Ubuntu; other distros: musl-gcc
cargo build --release --target x86_64-unknown-linux-musl
cargo test
```

The result is `target/x86_64-unknown-linux-musl/release/ferrimeter`, fully static.

## Project layout

| File | Purpose |
|---|---|
| `src/format.rs` | Turns byte rates into text like `12.4 MB/s` |
| `src/counters.rs` | Picks adapters and computes rates (shared) |
| `src/settings.rs` | Loads and saves `settings.json` (shared) |
| `src/app.rs` | App state: what each tick shows, what each menu item changes (shared) |
| `src/main.rs` | Starts the Windows or Linux app |
| `src/win/` | Windows: counters (`GetIfTable2`), floating widget, tray, docking, Run key |
| `src/linux/` | Linux: counters (`/sys`), tray icon and its pixel font, `--bar`, autostart |

The shared files and most of `src/linux/` are plain logic with unit tests; the
Windows widget and tray are checked by hand with the list below.

## Manual test checklist

Window behavior isn't covered by unit tests. Before a release, check on Windows:

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
13. Docked: the widget sits just left of the tray icons, comes back above the taskbar
    within a second after clicking the taskbar, and follows when tray icons are added.
14. Follow Windows: switching Windows between light and dark mode recolors the widget.

On Linux:

1. `ldd ferrimeter` reports a static executable.
2. During a large download, `ferrimeter --bar` agrees within about 10% with the
   change in `/proc/net/dev` over the same seconds.
3. `ferrimeter --bar | head -3` prints three lines and exits with status 0.
4. On KDE and on Ubuntu GNOME: the tray icon appears, its menu works, and Start at
   login survives a re-login.

## License

[MIT](LICENSE) © 2026 [Avalons89](https://github.com/Avalons89)
