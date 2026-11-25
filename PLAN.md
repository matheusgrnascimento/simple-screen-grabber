# Simple Screen Grabber: plan

A native Windows screenshot tool: press a shortcut, select an area, annotate it, then save or copy it. Inspired by the capture-and-annotate flow of Lightshot, without any upload or sharing.

This document records the decisions made before implementation. v1 is implemented (milestones M0 to M8); what was measured and what changed along the way is in [Implementation notes](#implementation-notes) at the end.

## Priorities

In this order, every design choice is checked against them:

1. **Low resource use.** Zero CPU and a few MB of RAM while idle.
2. **Low latency.** The overlay appears almost instantly after the shortcut.
3. **Efficiency.** Small binary, minimal dependencies, no work that is not needed.
4. **Image quality.** Pixel-exact, lossless output at native resolution.

And everything follows the **Windows 11 (Fluent) design**.

## Scope

### v1

- Global shortcut **Ctrl+Alt+S** freezes the screen (all monitors).
- Select an area, move and resize it with handles, see its size in pixels. Esc cancels.
- Annotation tools: arrow, rectangle, line, pen, highlighter.
- Fixed palette of 8 colors, stroke width set with the mouse wheel.
- Undo and redo.
- Save as PNG to the default folder, or copy to the clipboard.
- Tray icon with a small menu.
- Notification after saving.
- Settings in a `config.toml` file (no settings window).
- MSI installer and portable `.exe`.

### Later

| Version | Feature |
|---|---|
| v1.1 | Text tool (inline editing, caret, IME) |
| v1.1 | Blur / pixelate tool to hide sensitive data |
| v2 | HDR capture with tone mapping to SDR |
| v2 | Configurable output formats |
| Future | Publish to winget |
| Future | Code signing (removes the SmartScreen "unknown publisher" warning) |

### Out of scope

- Upload, cloud hosting, share links, social networks, reverse image search, printing.

## User flow and shortcuts

| Where | Input | Action |
|---|---|---|
| Anywhere | Ctrl+Alt+S | Capture: freeze the screen and open the overlay |
| Overlay | Drag | Create the selection (dragging outside an existing selection replaces it) |
| Overlay | Drag inside the selection / on a handle | Move / resize (when no drawing tool is active) |
| Overlay | Ctrl+S | Save to the default folder and close |
| Overlay | Ctrl+C | Copy and close |
| Overlay | Enter / double-click inside the selection | Copy and close (double-click only when no drawing tool is active) |
| Overlay | Esc | Cancel and close |
| Overlay | Ctrl+Z / Ctrl+Y | Undo / redo |
| Overlay | Mouse wheel | Stroke width |
| Overlay | A, R, L, P, H | Arrow, rectangle, line, pen, highlighter (proposal, to confirm) |

The global shortcut is configurable. If it cannot be registered (another app owns it), the app shows a notification instead of failing silently. There is no low-level keyboard hook: it would route every keystroke on the system through this process.

## Saving and copying

- **Folder:** the Windows Screenshots known folder (`FOLDERID_Screenshots`, "Pictures\Screenshots"), so it follows the user if the folder was moved to OneDrive or another drive. Configurable.
- **File name:** `YYYY-MM-DD_HH-MM-SS.png` (for example `2026-11-25_14-32-10.png`). On a collision a suffix is added: `_2`, `_3`, ...
- **Writing:** encode to a temporary file in the same folder, then rename, so a partial file never appears.
- **Clipboard:** `CF_DIBV5` placed immediately, plus the registered `PNG` format with delayed rendering, encoded only if the pasting app asks for it. The cropped image is kept in memory only while the app owns the clipboard and released on `WM_DESTROYCLIPBOARD`.
- **Notification:** after saving, a native tray notification ("Screenshot saved"); clicking it opens the folder. No notification on copy.

## Editor

- **Tools (v1):** arrow, rectangle, line, pen, highlighter (semi-transparent).
- **Colors:** fixed palette of 8, default red. Initial values: red `#FF4B4B`, orange `#FF8C00`, yellow `#FFC83D`, green `#16C60C`, blue `#0078D4`, purple `#8E44AD`, black `#000000`, white `#FFFFFF`.
- **Stroke width:** mouse wheel, default 3 px multiplied by the scale of the monitor, so strokes look the same on every display.
- **Model:** annotations are a list of vector shapes in physical pixel coordinates. Undo and redo move shapes between two stacks; no image copies.
- **Cursor:** not included in the capture.

## Windows 11 design

- **Toolbars:** a tools bar and an actions bar next to the selection, moved inside it when there is no room on screen.
- **Acrylic:** the toolbars are drawn over a blurred copy of the frozen screenshot (Direct2D Gaussian blur, plus the Fluent tint and noise). It looks like system Acrylic without extra windows or latency.
- **Shape:** 8 px corners, 1 px stroke, flyout elevation shadow.
- **Icons:** Segoe Fluent Icons; Segoe MDL2 Assets on Windows 10. The font is not embedded (its license does not allow it).
- **Text:** Segoe UI Variable through DirectWrite; Segoe UI on Windows 10.
- **Accent color:** the system accent marks the active tool, the selection border and the handles. The handles follow the Windows 11 style, like the crop handles of the Snipping Tool.
- **Size label:** `1920 × 1080` in a small Acrylic pill, in physical pixels.
- **Tooltips** on every button, with the shortcut (for example "Arrow (A)").
- **Theme:** light or dark following Windows, switching live.
- **Motion:** when "Animation effects" is off, nothing animates. When it is on, only the toolbars fade in briefly. The capture never waits for an animation.
- **High contrast:** uses the system colors.
- **Tray menu:** the native Win32 menu (rounded and Fluent on Windows 11), with dark mode enabled the same way Explorer does it.
- **UI language:** English, like the other projects.

## Tray and settings

- **Tray icon:** left click captures. Menu: Capture, Open folder, Start with Windows (check), Exit.
- **Single instance:** a named mutex. A second launch does not start another process; it asks the running one to capture.
- **Settings:** `%APPDATA%\simple-screen-grabber\config.toml`, flat `key = value` entries: shortcut, save folder, default color, default stroke width. Parsed by a small, tested parser for that flat subset instead of a TOML crate, to keep the dependency list short. Missing or invalid values fall back to defaults.
- **Portable mode:** if a file named `portable` sits next to the `.exe`, the config lives in that folder and nothing is written to the system.
- **Start with Windows:** a value under `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.

## Architecture

- **Language:** Rust (edition 2021), stable MSVC toolchain.
- **Dependencies:** only the `windows` crate (Win32, Direct2D, DirectWrite, WIC, DXGI). No async runtime, no UI framework, no image crate.
- **Threads:** the UI thread runs a blocking `GetMessage` loop (no timers, no polling). One worker thread encodes PNG files.

### Capture

- DPI awareness **PerMonitorV2** in the manifest, so Windows never scales the capture.
- `BitBlt` of the whole virtual screen into a top-down BGRA DIB section, in physical pixels. Whether to use `CAPTUREBLT` is decided by measuring (it includes some layered windows but can be slower).
- Selections across monitors with different scales work because every coordinate is a physical pixel of the virtual screen.
- Desktop Duplication / Windows.Graphics.Capture is kept for HDR (v2): it would hold GPU resources while idle, against priority 1.

### Overlay

- One borderless, topmost tool window covering the virtual screen, created at startup and kept hidden.
- Direct2D device context on a flip-model DXGI swap chain, created at startup.
- On capture: the screenshot is uploaded to the GPU once as a bitmap; each redraw only composes the bitmap, the dimming, the selection, the annotations and the toolbars.
- Redraws happen only on input. No render loop.
- The window has one DPI, so the UI (toolbars, handles, label) is scaled by the DPI of the monitor it sits on.

### Export

- The final image is rendered by the **same drawing code** used on screen, into an offscreen bitmap at native resolution, cropped to the selection, then read back.
- PNG encoding with WIC (built into Windows), on the worker thread.
- Output colors: 8-bit sRGB, no conversion. Pixels outside annotations are bit-identical to the screen.

### Memory

- The capture buffer (about 33 MB per 4K monitor) and the GPU bitmap are released as soon as the overlay closes.

### Proposed modules

| Module | Responsibility |
|---|---|
| `main.rs` | Entry point, single instance, message loop |
| `hotkey.rs` | Global shortcut registration |
| `tray.rs` | Tray icon, menu, notifications |
| `capture.rs` | Virtual screen capture |
| `overlay/` | Overlay window, rendering, selection, toolbars |
| `annotate/` | Shapes, tools, undo/redo history |
| `export/` | PNG encoding (WIC), clipboard |
| `fluent/` | Theme, accent color, palette, Acrylic, icons, fonts |
| `config.rs` | Settings file and portable mode |
| `paths.rs` | Known folders, file naming |
| `bench.rs` | `--bench` measurements |

## Performance targets

| Metric | Target |
|---|---|
| Shortcut to overlay visible (one 4K monitor) | < 30 ms |
| Input to updated frame | < 1 frame |
| Overlay closes after Ctrl+S | immediate (encoding continues in the background) |
| CPU while idle | 0% |
| RAM while idle (working set) | ≤ 5 MB |
| Executable size | < 1 MB |

A `--bench` mode measures shortcut-to-overlay, capture time, encoding time and memory before and after a capture, so each target is a number, not an impression.

Release profile:

```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

## Image quality rules

- Physical pixels from capture to file, with no resampling at any step.
- Lossless PNG; outside the annotations, the output equals the screen bit for bit.
- Annotations anti-aliased and rasterized at native resolution, with width proportional to DPI.
- What is shown in the overlay is exactly what is exported (same drawing code).

## Icon

Same family as Simple RTSP Viewer and Simple Clip Trimmer: blue rounded square with the vertical gradient `#2B88D8` → `#005AA6`, a white main symbol and one accent color.

- **Symbol:** a white dashed selection rectangle with red (`#FF4B4B`) handles at the four corners, like a screen selection.
- **Small sizes (24 px and below):** the border becomes a solid line and the handles get slightly larger, so the tray icon stays sharp.
- **Generated by code:** `tools/genicon` (Rust) renders every size of the `.ico` (16 to 256) with supersampling and writes `assets/icon.ico` and `assets/icon.png`. The icon is changed in the generator, never by editing the image files.

Geometry in unit coordinates (0..1), from the approved sketch: background `0.03..0.97` with radius `0.22`; selection rectangle `x 0.24..0.76`, `y 0.28..0.72`, stroke `0.05`, dash `0.09` / gap `0.06`; handles `0.12` squares centered on the rectangle corners.

## Distribution

Two outputs from the same build.

### MSI installer

WiX Toolset 5, as in the other projects (`installer/Package.wxs`, `installer/build-installer.ps1`).

- Installs to `Program Files\Simple Screen Grabber`, adds a Start menu shortcut, removable from Settings > Apps.
- `MajorUpgrade` installs over older versions and refuses downgrades.
- C runtime linked statically (no Visual C++ Redistributable needed).
- Closes the running app before upgrading or uninstalling (`WixToolset.Util`), since it stays in the tray.
- Final dialog: "Start with Windows" (checked) and "Open now". The stock WixUI exit dialog has one optional checkbox, so this may need a custom dialog.
- Uninstalling removes the "Start with Windows" value, so no broken entry remains in Startup apps.
- `config.toml` survives upgrades and uninstalls.

### Portable

`simple-screen-grabber.exe` alone, with the optional `portable` marker file described above.

## Platform

- Windows 10 (1903 or later) and Windows 11, x64.

## Conventions

Same as the other projects:

- All content in English: code, comments, UI text, docs, commits.
- `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, `cargo test` before every commit.
- No `unwrap()` / `expect()` in non-test code unless the invariant is documented.
- Never block the UI thread.
- Atomic commits with Conventional Commits.
- The full rules are in [AGENTS.md](AGENTS.md).

## Milestones

Each milestone builds, passes clippy and tests, and is usable on its own.

| # | Milestone | Done when |
|---|---|---|
| M0 | Scaffold | Cargo project, manifest (PerMonitorV2), release profile, icon generator and icon |
| M1 | Tray and shortcut | Tray icon and menu, single instance, Ctrl+Alt+S registered, notification when the shortcut is taken |
| M2 | Capture and selection | Frozen overlay on all monitors, create / move / resize selection, size label, Esc |
| M3 | Export | Ctrl+S saves to the Screenshots folder with the naming rules; Ctrl+C / Enter / double-click copy; save notification |
| M4 | Annotation | Arrow, rectangle, line, pen, highlighter; palette; stroke width; undo / redo |
| M5 | Fluent polish | Acrylic toolbars, Fluent icons, accent color, live light / dark theme, tooltips, motion and high contrast settings |
| M6 | Settings | `config.toml`, portable mode, Start with Windows |
| M7 | Performance | `--bench` mode; every target in the table met and measured |
| M8 | Distribution | MSI installer and portable build |

## Open points

- Single-letter tool shortcuts (A, R, L, P, H): implemented as proposed.
- Exact palette values: implemented as listed.

## Implementation notes

What changed from the plan during implementation, and why.

### Measured (3440x1440 desktop, `--bench`)

| Target | Result |
|---|---|
| Shortcut to overlay visible < 30 ms | The screen dims in 7.6 ms; the frozen, interactive overlay follows at 55 ms (60 ms right after a working set trim). Not met for the frozen overlay: see below |
| Input to updated frame < 1 frame | 2.0 ms per frame |
| Overlay closes after Ctrl+S immediately | Yes: encoding (about 40 ms for 2136x936) runs on a worker thread |
| CPU while idle 0% | 0 |
| RAM while idle (working set) ≤ 5 MB | 0.2 MB (trimmed); 15.6 MB untrimmed, mostly shared system DLLs. Private memory: 3.3 MB, 27.7 MB after the first capture |
| Executable < 1 MB | About 470 KB |

### Decisions that changed

- **Direct2D on WARP, not the GPU.** A hardware Direct3D device took 240 ms and 52 MB of private memory that the GPU driver keeps even after release (the idle app went from 3.5 MB to 52.9 MB). WARP, the software rasterizer of Windows, is created in 16 ms with 2.6 MB and draws a full frame in about 2 ms, with the same Direct2D output.
- **Instant dimming before the copy.** The GDI copy of the screen takes 30 to 40 ms at this size, with or without `CAPTUREBLT` (kept: it includes layered windows); copying in parallel stripes or to a DDB did not help, because the compositor reads the frame back from the GPU. GPU capture (Desktop Duplication) would need the hardware device above. So the shortcut first shows a translucent black window excluded from capture (`WDA_EXCLUDEFROMCAPTURE`, Windows 10 2004+): the screen reacts in under 10 ms, and the frozen overlay with the same dimming covers it when ready. A test confirmed that captures do not include it and take the same time.
- **Caches released after a capture.** Direct2D and WARP kept 132 MB after a capture; `ID2D1Device::ClearResources` and `IDXGIDevice3::Trim` bring it to 28 MB, which WARP keeps process-wide (it is not freed even when the device is destroyed) and which does not grow. The working set is trimmed after startup and after each capture, at about 5 ms on the next capture.
- **Clipboard formats.** Both `CF_DIB` (32-bit, opaque) and `PNG` use delayed rendering; Windows synthesizes `CF_DIBV5` and `CF_BITMAP` from `CF_DIB`.
- **Highlighter.** Each stroke is drawn opaque into a layer composited at 45% opacity, so the stroke never darkens where it crosses itself.
- **Extra keyboard paths.** `1`-`8` for colors, `[` and `]` for the stroke width, Shift for 45 degree lines and squares, the arrow keys to move the selection. With nothing selected, copy and save take the monitor under the mouse.
- **Tray menu.** Adds Settings..., which opens `config.toml` (written from a commented template on first use). Settings apply when the file is saved: the message loop waits on a change notification for the settings folder.
- **Installer.** The stock WixUI exit dialog has one checkbox, used for "Open Simple Screen Grabber". Start with Windows is a separate feature, on by default (`START_WITH_WINDOWS=0` turns it off), kept off on upgrade when the user turned it off in the tray, and never re-enabled by the advertised shortcut's repair check. Uninstall removes it even when it was turned on later from the tray.
- **Second launch.** Starts a capture in the running instance.

### Later

- Capture into a GDI-compatible texture to skip one copy (about 3 ms and 20 MB less during a capture).
- Smoother freehand strokes from the high-resolution mouse positions (`GetMouseMovePointsEx`).
