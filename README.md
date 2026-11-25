# Simple Screen Grabber

Simple native Windows app for screenshots: press a shortcut, select an area, annotate it, then save it as PNG or copy it. It stays in the tray, uses almost no resources while idle and never uploads anything. The project is written in Rust and talks to Windows directly (Win32, Direct2D, DirectWrite, WIC) through the `windows` crate, without a UI framework.

## Features

- **Ctrl+Alt+S** (configurable) dims the screen at once and freezes every monitor in a selection overlay;
- drag to select an area: both corner pixels are included, and the size shows in pixels. Move the selection by dragging inside it or with the arrow keys (Shift: 10 px), resize it with Windows 11 style handles;
- pixel-exact output: the app is per-monitor DPI aware (PerMonitorV2), so nothing is ever scaled, also for selections across monitors with different scales. Without annotations the PNG holds the captured pixels unchanged;
- annotations: arrow, rectangle, line, pen and highlighter, in 8 colors, with the stroke width on the mouse wheel (the brush outline follows the mouse), Shift for 45 degree lines and squares, and undo/redo. They are vectors, drawn again at native resolution for the output with the same code as the overlay;
- **Ctrl+S** saves a lossless PNG to the Windows Screenshots folder (`Pictures\Screenshots`, or a folder of your choice) as `2026-11-25_14-32-10.png`, without ever replacing a file, and a notification opens it in File Explorer;
- **Ctrl+C**, **Enter** or a double-click copies it: the clipboard announces a bitmap and a PNG, and makes them only when an app pastes;
- with nothing selected, saving or copying takes the monitor under the mouse;
- Windows 11 look: toolbars, color flyout, size label and tooltips in Acrylic made from the frozen screen, elevation shadows, Segoe UI Variable and Segoe Fluent Icons (Segoe MDL2 Assets on Windows 10), the accent color, and the light/dark theme, high contrast, transparency and animation settings of Windows, followed live;
- tray icon with Capture, Open screenshots folder, Settings, Start with Windows and Exit; the tray menu follows the dark theme;
- settings in a `config.toml` file, applied as soon as it is saved;
- a portable mode, an MSI installer, and a `--bench` mode that measures the app on your machine.

## Shortcuts

In the overlay:

| Key | Action |
|---|---|
| drag | select an area; inside it, move it; on a handle, resize it |
| `A` / `R` / `L` / `P` / `H` | arrow, rectangle, line, pen, highlighter (again: back to selecting) |
| `1` to `8` | color |
| mouse wheel, `[` / `]` | thinner / thicker stroke |
| Shift while drawing | lines and arrows at 45 degree steps, squares |
| `Ctrl+Z` / `Ctrl+Y` (or `Ctrl+Shift+Z`) | undo / redo |
| `←` `→` `↑` `↓` | move the selection by 1 px (Shift: 10 px) |
| `Ctrl+C`, `Enter`, double-click | copy and close |
| `Ctrl+S` | save and close |
| `Esc` | close the color flyout, or cancel |

Every toolbar button has a tooltip with its shortcut. Launching the app again while it runs, or clicking the tray icon, also starts a capture.

## Settings

**Settings...** in the tray menu opens `config.toml`, writing it from a commented template the first time. It lives in `%APPDATA%\simple-screen-grabber`, or next to the executable in portable mode. Changes apply when the file is saved; invalid values keep the defaults and a notification names the line.

| Setting | Default | |
|---|---|---|
| `hotkey` | `"Ctrl+Alt+S"` | Modifiers `Ctrl`, `Alt`, `Shift`, `Win` and a letter, digit, `F1`-`F24`, `PrintScreen`, `Insert`, `Delete`, `Home`, `End`, `PageUp`, `PageDown`, `Pause` or `ScrollLock`. Letters and digits need Ctrl, Alt or Win |
| `save_folder` | `''` | Empty: the Windows Screenshots folder. Use single quotes for paths (`'D:\Shots'`); `%VARIABLES%` expand and relative paths start at the settings folder |
| `color` | `"red"` | `red`, `orange`, `yellow`, `green`, `blue`, `purple`, `black` or `white` |
| `stroke_width` | `3` | 1 to 20 pixels at 100% display scale |

**Portable mode**: with a file named `portable` next to `simple-screen-grabber.exe`, the settings stay in that folder and nothing is written anywhere else (Start with Windows is then not available).

`SIMPLE_SCREEN_GRABBER_THEME` set to `light` or `dark` forces a theme, to check both without changing Windows.

## Requirements

- Windows 10 (1903 or later; 2004 or later for the instant dimming) or Windows 11, x64;
- to build: stable Rust with the MSVC toolchain (Visual Studio Build Tools);
- for the installer: [WiX Toolset 5](https://wixtoolset.org) with the UI and Util extensions.

## Build

```powershell
cargo build --release
```

`target\release\simple-screen-grabber.exe` is the whole app (about 470 KB): the C runtime is linked statically (`.cargo\config.toml`) and `build.rs` embeds the manifest (PerMonitorV2, segment heap), the icon and the version information without extra tools.

### Icon

The icon is a blue rounded square (the gradient of Simple RTSP Viewer and Simple Clip Trimmer) with a white dashed selection and red corner handles. `tools\genicon` draws every size of `assets\icon.ico` from code with 8x8 supersampling, with a solid, pixel-aligned border at 24 px and below:

```powershell
cargo run -p genicon
```

### Installer and portable package

```powershell
dotnet tool install --global wix --version 5.0.2
wix extension add -g WixToolset.UI.wixext/5.0.2
wix extension add -g WixToolset.Util.wixext/5.0.2
powershell -ExecutionPolicy Bypass -File .\installer\build-installer.ps1
```

This builds in `dist\`:

- `SimpleScreenGrabber-<version>-x64.msi`: installs to `Program Files\Simple Screen Grabber` with a Start menu shortcut, upgrades in place, closes the running app before replacing it, and offers to open it at the end. Start with Windows is on by default (`START_WITH_WINDOWS=0` turns it off in a silent install), stays off on upgrade if you turned it off, and is removed on uninstall;
- `SimpleScreenGrabber-<version>-portable.zip`: the executable, the license and the `portable` marker.

## How it works

- The capture overlay, its Direct2D device and swap chain are created at startup and kept hidden and cloaked. The shortcut first shows a translucent window that dims the screen at once (excluded from capture with `WDA_EXCLUDEFROMCAPTURE`), copies the virtual screen with GDI, uploads it once, draws the first frame while the overlay is still cloaked, then shows it over the dimming.
- Direct2D runs on WARP, the Direct3D software rasterizer of Windows: a hardware device took 240 ms and 52 MB that the GPU driver keeps even after release, for frames that WARP draws in about 2 ms.
- Frames are drawn only on input. Closing the overlay frees the capture, the swap chain buffers and the Direct2D caches, and trims the working set.
- The message loop sleeps in the kernel until a message arrives or the settings folder changes: nothing polls.

## Benchmark

```powershell
.\target\release\simple-screen-grabber.exe --bench | Out-Host
```

It opens the real overlay several times (the screen dims briefly), draws frames, exports and encodes a PNG in memory (nothing is written to disk) and prints the timings and the memory. On a 3440x1440 desktop:

| | |
|---|---|
| Shortcut to the screen dimming | 7.6 ms |
| Shortcut to the frozen overlay | 55 ms (60 ms right after the working set was trimmed); the GDI copy of the screen is 30-40 ms of it |
| Drawing a frame | 2.0 ms |
| Copying a 2136x936 selection / drawing it with annotations | 5 ms / 4 ms |
| PNG encoding of it (on a worker thread) | 41 ms |
| Idle memory (working set / private) | 0.2 MB / 3.3 MB (27.7 MB private after the first capture: caches of the software rasterizer) |
| CPU while idle | 0 |

## Tests

```powershell
cargo test
```

The tests cover the selection geometry, the toolbar layout and hit testing, the annotation model and undo, the hotkey and settings parsers, file naming, the clipboard bitmap, saving (lossless round trip, no overwriting, no leftover temporary file), the export (pixels away from annotations unchanged) and the overlay driven by real mouse and keyboard messages. A preview test draws the overlay over a synthetic screen into PNG files, to check the look:

```powershell
$env:SIMPLE_SCREEN_GRABBER_PREVIEW = "C:\some\folder"
cargo test render_previews -- --ignored
```

## License

MIT. See [LICENSE](LICENSE).
