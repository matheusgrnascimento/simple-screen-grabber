//! The app: single instance, the hidden host window, the tray icon, the
//! global shortcut and the message loop.
//!
//! All state lives in one [`App`] on the UI thread. Window procedures reach
//! it through [`with_app`], which skips messages that arrive while the
//! state is already borrowed (messages Windows sends synchronously from
//! inside a call such as `ShowWindow`); those get the default handling.
//! Modal loops (menus, message boxes) run while nothing is borrowed.

use std::cell::RefCell;

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Shell::{NINF_KEY, NIN_BALLOONUSERCLICK, NIN_SELECT};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    FindWindowW, GetMessageW, GetWindowThreadProcessId, PostMessageW, PostQuitMessage,
    RegisterClassExW, RegisterWindowMessageW, TranslateMessage, MSG, WM_APP, WM_CONTEXTMENU,
    WM_DISPLAYCHANGE, WM_HOTKEY, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::fluent::Palette;
use crate::hotkey::{self, Hotkey};
use crate::log;
use crate::overlay::{self, Overlay};
use crate::tray::{self, MenuItem, Tray};
use crate::win::{self, Handle};

/// Tray icon events (`uCallbackMessage`).
const WM_APP_TRAY: u32 = WM_APP + 1;
/// Sent by a second launch of the app: start a capture.
const WM_APP_CAPTURE: u32 = WM_APP + 2;

const HOST_CLASS: PCWSTR = w!("SimpleScreenGrabber.Host");
const INSTANCE_MUTEX: PCWSTR = w!("Local\\SimpleScreenGrabber.Instance");
const APP_NAME: &str = "Simple Screen Grabber";

/// The keyboard variant of `NIN_SELECT` (Enter or Space on the icon).
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const CMD_CAPTURE: u32 = 1;
const CMD_EXIT: u32 = 99;

/// Command line options.
#[derive(Default)]
pub struct Options {
    /// Started by Windows at sign-in: no "running" notification.
    pub autostart: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app state, or returns `None` if there is no app yet or
/// the state is already borrowed further up the stack.
fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| {
        let mut app = cell.try_borrow_mut().ok()?;
        app.as_mut().map(f)
    })
}

struct App {
    host: HWND,
    tray: Tray,
    overlay: Overlay,
    hotkey: Hotkey,
    registration: Option<hotkey::Registration>,
    taskbar_created: u32,
}

/// Runs the app until the user exits it.
pub fn run(options: Options) -> Result<()> {
    let Some(_instance) = acquire_instance()? else {
        signal_running_instance();
        return Ok(());
    };
    // SAFETY: initializes COM for the UI thread, once, before any COM call.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };

    let host = create_host_window()?;
    // SAFETY: registers (or looks up) a system-wide message name.
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let hotkey = Hotkey::DEFAULT;
    let tray = Tray::new(host, WM_APP_TRAY, &tooltip(hotkey))?;
    let overlay = Overlay::new(Some(overlay_proc), Palette::load())?;
    let mut app = App {
        host,
        tray,
        overlay,
        hotkey,
        registration: None,
        taskbar_created,
    };
    app.register_hotkey();
    if !options.autostart {
        app.tray.notify(
            &format!("{APP_NAME} is running"),
            &format!("Press {hotkey} to take a screenshot."),
        );
    }
    APP.with(|cell| *cell.borrow_mut() = Some(app));

    let result = message_loop();

    // Drop the app (tray icon, shortcut) before its window.
    APP.with(|cell| cell.borrow_mut().take());
    // SAFETY: the host window belongs to this thread.
    unsafe {
        let _ = DestroyWindow(host);
    }
    result
}

/// Takes the single-instance mutex. `None` when another instance runs.
fn acquire_instance() -> Result<Option<Handle>> {
    // SAFETY: creates or opens a named mutex owned by the returned wrapper.
    let mutex = Handle(unsafe { CreateMutexW(None, false, INSTANCE_MUTEX)? });
    // SAFETY: reads the error of the CreateMutexW call above.
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return Ok(None);
    }
    Ok(Some(mutex))
}

/// Asks the running instance to start a capture.
fn signal_running_instance() {
    // SAFETY: finds and posts to the other instance's host window; lets it
    // take the foreground, which this newly started process may grant.
    unsafe {
        let Ok(hwnd) = FindWindowW(HOST_CLASS, PCWSTR::null()) else {
            return;
        };
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let _ = AllowSetForegroundWindow(pid);
        let _ = PostMessageW(Some(hwnd), WM_APP_CAPTURE, WPARAM(0), LPARAM(0));
    }
}

fn create_host_window() -> Result<HWND> {
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(host_proc),
        hInstance: win::instance(),
        lpszClassName: HOST_CLASS,
        ..Default::default()
    };
    // SAFETY: the class structure is valid for the call; the window is a
    // hidden top-level window (not message-only, so it receives broadcasts
    // such as TaskbarCreated and WM_SETTINGCHANGE).
    unsafe {
        RegisterClassExW(&class);
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            HOST_CLASS,
            w!("Simple Screen Grabber"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(win::instance()),
            None,
        )
    }
}

fn message_loop() -> Result<()> {
    let mut msg = MSG::default();
    loop {
        // SAFETY: standard message loop on the UI thread.
        let status = unsafe { GetMessageW(&mut msg, None, 0, 0) }.0;
        match status {
            0 => return Ok(()),
            -1 => return Err(windows::core::Error::from_thread()),
            _ => {
                // SAFETY: `msg` was filled by GetMessageW.
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }
    }
}

fn tooltip(hotkey: Hotkey) -> String {
    format!("{APP_NAME}\n{hotkey} to capture")
}

extern "system" fn host_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_APP_TRAY && win::low_word(lparam.0 as usize) == WM_CONTEXTMENU {
        // The menu runs a modal loop: build it, then show it with nothing
        // borrowed.
        if let Some(items) = with_app(|app| app.menu_items()) {
            let anchor = POINT {
                x: win::x_of(wparam.0 as isize),
                y: win::y_of(wparam.0 as isize),
            };
            if let Some(id) = tray::show_menu(hwnd, anchor, &items) {
                with_app(|app| app.command(id));
            }
        }
        return LRESULT(0);
    }
    match with_app(|app| app.host_message(msg, wparam, lparam)) {
        Some(Some(result)) => result,
        // SAFETY: default handling for this thread's window.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match with_app(|app| app.overlay_message(msg, wparam, lparam)) {
        Some(Some(result)) => result,
        // SAFETY: default handling for this thread's window.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl App {
    fn overlay_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        let reply = self.overlay.message(msg, wparam, lparam);
        match reply.event {
            Some(overlay::Event::Close) => self.overlay.close(),
            None => {}
        }
        reply.result
    }

    fn host_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_HOTKEY if wparam.0 == hotkey::CAPTURE_ID as usize => self.capture(),
            WM_APP_CAPTURE => self.capture(),
            WM_APP_TRAY => match win::low_word(lparam.0 as usize) {
                NIN_SELECT | NIN_KEYSELECT => self.capture(),
                NIN_BALLOONUSERCLICK => {}
                _ => {}
            },
            WM_DISPLAYCHANGE => self.tray.refresh_icon(),
            _ if msg == self.taskbar_created && msg != 0 => {
                self.tray.refresh_icon();
                if let Err(err) = self.tray.add() {
                    log!("tray icon not added: {err}");
                }
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }

    fn menu_items(&self) -> Vec<MenuItem> {
        vec![
            MenuItem::Command {
                id: CMD_CAPTURE,
                label: format!("Capture\t{}", self.hotkey),
                checked: false,
                enabled: true,
                default: true,
            },
            MenuItem::Separator,
            MenuItem::Command {
                id: CMD_EXIT,
                label: "Exit".into(),
                checked: false,
                enabled: true,
                default: false,
            },
        ]
    }

    fn command(&mut self, id: u32) {
        match id {
            CMD_CAPTURE => self.capture(),
            // SAFETY: ends the message loop of this thread.
            CMD_EXIT => unsafe { PostQuitMessage(0) },
            _ => {}
        }
    }

    fn register_hotkey(&mut self) {
        self.registration = None;
        match hotkey::register(self.host, self.hotkey) {
            Ok(registration) => self.registration = Some(registration),
            Err(err) => {
                log!("shortcut {} not registered: {err}", self.hotkey);
                self.tray.notify(
                    &format!("{} is in use", self.hotkey),
                    "Another app uses this shortcut. Use the tray icon to capture, or choose another shortcut in the settings.",
                );
            }
        }
    }

    fn capture(&mut self) {
        if let Err(err) = self.overlay.open() {
            log!("capture failed: {err}");
            self.tray
                .notify("Couldn't capture the screen", &err.message());
        }
    }
}
