# AGENTS.md

Guidelines for AI agents and contributors working in this repository.

## Project overview

Simple Screen Grabber is a Windows desktop app for screenshots: press a shortcut, select an area, annotate it, then save it as PNG or copy it. It stays in the tray and never uploads anything.

- Language: Rust (edition 2021), stable MSVC toolchain.
- Platform APIs: the `windows` crate only: Win32, Direct2D, DirectWrite, WIC and DXGI. No UI framework, no async runtime, no image crate.
- UI: one borderless overlay window drawn with Direct2D in the Windows 11 (Fluent) style, a tray icon and native menus.
- Priorities, in order: low resource use, low latency, efficiency, image quality.

### Layout

The code does not exist yet. [PLAN.md](PLAN.md) holds the design, the performance targets and the milestones. The planned modules:

- `src/main.rs`: entry point, single instance, message loop.
- `src/hotkey.rs`: global shortcut registration.
- `src/tray.rs`: tray icon, menu and notifications.
- `src/capture.rs`: virtual screen capture.
- `src/overlay/`: overlay window, rendering, selection and toolbars.
- `src/annotate/`: shapes, tools and undo/redo history.
- `src/export/`: PNG encoding (WIC) and clipboard.
- `src/fluent/`: theme, accent color, palette, Acrylic, icons and fonts.
- `src/config.rs`: settings file and portable mode.
- `src/paths.rs`: known folders and file naming.
- `src/bench.rs`: the `--bench` measurements.
- `tools/genicon/` and `assets/`: the app icon, generated from code. Change the icon in the generator, never by editing the image files.
- `installer/`: the WiX MSI installer.

Keep this section in sync with the code as modules land.

## Language

All content must be written in English: code, identifiers, comments, doc comments, log and error messages, UI text, commit messages, documentation and scripts.

## Build and verify

```powershell
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Run formatting, clippy and tests before every commit. Fix warnings instead of silencing them.

## Coding conventions

- Follow idiomatic Rust and the existing module structure. Keep modules small and single-purpose.
- No `unwrap()` / `expect()` in non-test code unless the invariant is documented in a comment.
- Keep `unsafe` in the thin modules that call Win32 and COM. Every `unsafe` block gets a `// SAFETY:` comment.
- Wrap handles and COM objects in types that release them on `Drop` (GDI objects, DCs, windows, icons, global memory). No manual cleanup paths.
- Never block the UI thread. PNG encoding and file writing run on the worker thread.
- Do not add dependencies. The `windows` crate covers the platform; a new crate needs a clear need that it cannot meet, and default features disabled.
- Use a small internal logging helper (`OutputDebugStringW`), not a logging crate and not `println!`.
- Add tests for new logic. Keep pure logic (selection geometry, file naming, config parsing, hit testing) free of Win32 calls so it is testable.
- Keep changes scoped. No unrelated refactors in a feature or fix commit.

### Performance

- Nothing runs while idle: no timers, no polling, no background threads waiting on anything but work. The UI thread blocks in `GetMessage`.
- Create the overlay window and the Direct2D resources at startup; a capture only grabs the screen and shows them.
- Redraw only on input or state changes. No render loop.
- Release the capture buffer and the GPU bitmap as soon as the overlay closes.
- No allocations per frame in the drawing path.
- Measure with `--bench` before and after any change that can affect the targets in [PLAN.md](PLAN.md), and give the numbers in the commit body.

### Windows 11 design

- Draw the UI with the `fluent` module: take colors only from its palette (never hard-coded), and give sizes in DIPs scaled by the DPI of the monitor the element is on.
- Keep both light and dark themes working, and high contrast. `SIMPLE_SCREEN_GRABBER_THEME` (`light` or `dark`) forces one to check it.
- Animate nothing when the Windows "Animation effects" setting is off, and never make the capture wait for an animation.
- Every action needs a keyboard path, and every toolbar button a tooltip with its shortcut.

## Commits

Make atomic commits: each commit contains one logical change, builds, and passes tests on its own.

- Do not mix features, fixes, refactors, formatting and docs in one commit. Split them.
- Commit only the files that belong to the change. Stage deliberately (`git add -p`), not with `git add -A`.
- Use Conventional Commits: `type(scope): summary`.
  - Types: `feat`, `fix`, `refactor`, `perf`, `test`, `docs`, `build`, `chore`, `style`.
  - Summary in imperative mood, lowercase, no trailing period, 72 characters or fewer.
  - Add a body when the reason for the change is not obvious.
- Do not commit build output (`target/`, `dist/`) or local settings (`config.toml`, the `portable` marker).
- Do not rewrite history that has been pushed. Do not force push.

## Image handling

- Never resample captured pixels. Work in physical pixels from capture to file; outside the annotations the output must equal the screen bit for bit.
- Export with the same drawing code the overlay uses, so the file matches what was shown.
- Never overwrite an existing file. Write to a temporary file in the target folder and rename it, so a partial file never appears.

## Security and privacy

Screenshots can contain anything on screen: passwords, documents, messages.

- The app makes no network connections.
- Never write a capture anywhere the user did not ask for: no cache, no temporary copies left behind, no captures in logs or crash output.
- Release clipboard data the app holds as soon as another app takes the clipboard.
