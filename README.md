# Simple Screen Grabber

Simple native Windows app for screenshots: press a shortcut, select an area, annotate it, then save it as PNG or copy it. It stays in the tray, uses almost no resources while idle and never uploads anything. The project is written in Rust and talks to Windows directly (Win32, Direct2D, WIC) through the `windows` crate, without a UI framework.

## Status

In planning. No code yet. [PLAN.md](PLAN.md) holds the design decisions, the performance targets and the milestones.

## Planned features (v1)

- **Ctrl+Alt+S** freezes every monitor and opens the selection overlay, in under 30 ms;
- select an area, move and resize it with handles, with its size in pixels;
- annotate with an arrow, rectangle, line, pen and highlighter, in 8 colors, with the stroke width on the mouse wheel and undo/redo;
- **Ctrl+S** saves a lossless PNG to the Windows Screenshots folder; **Ctrl+C**, **Enter** or a double-click copies it;
- pixel-exact output at native resolution on any display scaling, including selections across monitors with different scales;
- Windows 11 look: Acrylic toolbars, Segoe Fluent Icons, accent color, and light/dark theme following the system;
- tray icon, start with Windows, and a notification after saving;
- MSI installer and a portable executable.

## Requirements

- Windows 10 (1903 or later) or Windows 11, x64;
- to build: stable Rust with the MSVC toolchain (Visual Studio Build Tools).

## License

MIT. See [LICENSE](LICENSE).
