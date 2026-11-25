//! Simple Screen Grabber: press a shortcut, select an area, annotate it, then
//! save or copy it.

#![windows_subsystem = "windows"]

mod annotate;
mod app;
mod bench;
mod capture;
mod config;
mod export;
mod fluent;
mod geometry;
mod gfx;
mod hotkey;
mod log;
mod overlay;
mod paths;
mod startup;
mod tray;
mod win;

use windows::core::w;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

fn main() {
    let mut options = app::Options::default();
    let mut bench = false;
    let mut set_autostart = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--autostart" => options.autostart = true,
            "--bench" => bench = true,
            // Used by the installer, and by scripts.
            "--enable-autostart" => set_autostart = Some(true),
            "--disable-autostart" => set_autostart = Some(false),
            _ => {}
        }
    }
    let result = if let Some(enabled) = set_autostart {
        startup::set_enabled(enabled)
    } else if bench {
        bench::run()
    } else {
        app::run(options)
    };
    if let Err(err) = result {
        log!("fatal: {err}");
        let text = win::wide(&format!("Simple Screen Grabber could not start.\n\n{err}"));
        // SAFETY: shows a modal message box with a null-terminated string
        // that outlives the call.
        unsafe {
            MessageBoxW(
                None,
                win::pcwstr(&text),
                w!("Simple Screen Grabber"),
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
