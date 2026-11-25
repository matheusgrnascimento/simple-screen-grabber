//! The app: single instance, the hidden host window, the tray icon, the
//! global shortcut, the settings and the message loop.
//!
//! All state lives in one [`App`] on the UI thread. Window procedures reach
//! it through [`with_app`], which skips messages that arrive while the
//! state is already borrowed (messages Windows sends synchronously from
//! inside a call such as `ShowWindow`); those get the default handling.
//! Modal loops (menus, message boxes) run while nothing is borrowed.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{
    GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, LRESULT, POINT, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
};
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_MULTITHREADED,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::System::Threading::INFINITE;
use windows::Win32::UI::Shell::{
    ILCreateFromPathW, ILFree, SHOpenFolderAndSelectItems, ShellExecuteW, NINF_KEY,
    NIN_BALLOONUSERCLICK, NIN_SELECT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    FindWindowW, GetWindowThreadProcessId, KillTimer, MsgWaitForMultipleObjectsEx, PeekMessageW,
    PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SetTimer,
    TranslateMessage, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT, SW_SHOWNORMAL, WM_APP,
    WM_CLOSE, WM_CONTEXTMENU, WM_DESTROYCLIPBOARD, WM_DISPLAYCHANGE, WM_ENDSESSION, WM_HOTKEY,
    WM_QUERYENDSESSION, WM_QUIT, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WM_SETTINGCHANGE,
    WM_SYSCOLORCHANGE, WM_THEMECHANGED, WM_TIMER, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};

use crate::capture::Timestamp;
use crate::config::{Change, Config, Location};
use crate::export::{self, clipboard::Clipboard, Image};
use crate::fluent::{menus, Theme};
use crate::hotkey::{self, Hotkey};
use crate::log;
use crate::overlay::{self, Overlay};
use crate::paths;
use crate::settings::{self, Action, Settings, Values};
use crate::startup;
use crate::tray::{self, MenuItem, Tray};
use crate::win::{self, Handle};

/// Tray icon events (`uCallbackMessage`).
const WM_APP_TRAY: u32 = WM_APP + 1;
/// Sent by a second launch of the app: start a capture.
const WM_APP_CAPTURE: u32 = WM_APP + 2;
/// Sent by the save thread; `lparam` is a `Box<Result<PathBuf>>`.
const WM_APP_SAVED: u32 = WM_APP + 3;
/// Sent by a launch with `--settings`: open the settings window.
const WM_APP_SETTINGS: u32 = WM_APP + 4;

const HOST_CLASS: PCWSTR = w!("SimpleScreenGrabber.Host");
const INSTANCE_MUTEX: PCWSTR = w!("Local\\SimpleScreenGrabber.Instance");
const APP_NAME: &str = "Simple Screen Grabber";

/// `WM_ENDSESSION` flag: the app is closed by the Restart Manager (an
/// installer), not by a sign-out.
const ENDSESSION_CLOSEAPP: u32 = 0x0000_0001;

/// The keyboard variant of `NIN_SELECT` (Enter or Space on the icon).
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;

const CMD_CAPTURE: u32 = 1;
const CMD_OPEN_FOLDER: u32 = 2;
const CMD_SETTINGS: u32 = 3;
const CMD_AUTOSTART: u32 = 4;
const CMD_EXIT: u32 = 99;

/// Reloads the settings shortly after their folder changes, once the
/// editor has finished saving.
const CONFIG_TIMER: usize = 1;
const CONFIG_DELAY_MS: u32 = 250;

/// Command line options.
#[derive(Default)]
pub struct Options {
    /// Started by Windows at sign-in: no "running" notification.
    pub autostart: bool,
    /// `--settings`: open the settings window.
    pub settings: bool,
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
    theme: Theme,
    clipboard: Clipboard,
    location: Location,
    config: Config,
    /// The settings file's modification time and size when last read.
    config_stamp: Option<(std::time::SystemTime, u64)>,
    /// Problems in the settings file, as last reported.
    config_problems: Vec<String>,
    /// Watches the settings folder, when it exists.
    watch: Option<FolderWatch>,
    registration: Option<hotkey::Registration>,
    taskbar_created: u32,
    /// What a click on the last notification does.
    notification: NotificationAction,
    /// The settings window, while open.
    settings: Option<Settings>,
}

/// What a click on a notification does.
enum NotificationAction {
    None,
    /// Show a saved screenshot in File Explorer.
    ShowFile(PathBuf),
}

/// A change notification on a folder, closed on drop.
struct FolderWatch(HANDLE);

impl FolderWatch {
    /// Signals when a file in `folder` is written, created or renamed.
    fn start(folder: &Path) -> Option<FolderWatch> {
        let path = win::wide(&folder.to_string_lossy());
        // SAFETY: the path is null-terminated and outlives the call.
        let handle = unsafe {
            FindFirstChangeNotificationW(
                win::pcwstr(&path),
                false,
                FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME,
            )
        };
        handle.ok().map(FolderWatch)
    }
}

impl Drop for FolderWatch {
    fn drop(&mut self) {
        // SAFETY: the handle came from FindFirstChangeNotificationW.
        unsafe {
            let _ = FindCloseChangeNotification(self.0);
        }
    }
}

/// A window handle that the save thread may post to.
struct PostTarget(HWND);

// SAFETY: PostMessageW accepts a window of another thread.
unsafe impl Send for PostTarget {}

/// Runs the app until the user exits it.
pub fn run(options: Options) -> Result<()> {
    let Some(_instance) = acquire_instance()? else {
        signal_running_instance(if options.settings {
            WM_APP_SETTINGS
        } else {
            WM_APP_CAPTURE
        });
        return Ok(());
    };
    // SAFETY: initializes COM for the UI thread, once, before any COM call.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };

    let host = create_host_window()?;
    // SAFETY: registers (or looks up) a system-wide message name.
    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    let location = Location::find();
    let config_stamp = location.stamp();
    let (config, config_problems) = location.load();
    let tray = Tray::new(host, WM_APP_TRAY, &tooltip(config.hotkey))?;
    let theme = Theme::load();
    menus::set_dark(theme.palette.dark);
    let mut overlay = Overlay::new(Some(overlay_proc), theme)?;
    overlay.set_defaults(config.color, config.stroke_width);
    let watch = FolderWatch::start(&location.folder);
    let mut app = App {
        host,
        tray,
        overlay,
        theme,
        clipboard: Clipboard::new(),
        location,
        config,
        config_stamp,
        config_problems: Vec::new(),
        watch,
        registration: None,
        taskbar_created,
        notification: NotificationAction::None,
        settings: None,
    };
    app.register_hotkey();
    app.report_problems(config_problems);
    if options.settings {
        app.open_settings();
    } else if !options.autostart {
        app.tray.notify(
            &format!("{APP_NAME} is running"),
            &format!("Press {} to take a screenshot.", app.config.hotkey),
        );
    }
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    // Idle from here on: give the start-up pages back to Windows.
    win::trim_working_set();

    let result = message_loop();

    // Leave a copied image on the clipboard after exit.
    with_app(|app| app.clipboard.render_all(app.host));
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

/// Asks the running instance to start a capture or open the settings.
fn signal_running_instance(message: u32) {
    // SAFETY: finds and posts to the other instance's host window; lets it
    // take the foreground, which this newly started process may grant.
    unsafe {
        let Ok(hwnd) = FindWindowW(HOST_CLASS, PCWSTR::null()) else {
            return;
        };
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let _ = AllowSetForegroundWindow(pid);
        let _ = PostMessageW(Some(hwnd), message, WPARAM(0), LPARAM(0));
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

/// The message loop. It sleeps in the kernel until a message arrives or
/// the settings folder changes: no polling, no timers.
fn message_loop() -> Result<()> {
    let mut msg = MSG::default();
    loop {
        let watch = with_app(|app| app.watch.as_ref().map(|w| w.0)).flatten();
        let handles: &[HANDLE] = match &watch {
            Some(handle) => std::slice::from_ref(handle),
            None => &[],
        };
        // SAFETY: waits on a handle owned by the app, which outlives the
        // wait (it is only replaced from this thread).
        let wait = unsafe {
            MsgWaitForMultipleObjectsEx(Some(handles), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
        if watch.is_some() && wait == WAIT_OBJECT_0 {
            with_app(|app| app.config_folder_changed());
            continue;
        }
        // SAFETY: standard message pump on the UI thread.
        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
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

extern "system" fn settings_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let (result, browse) =
        with_app(|app| app.settings_message(hwnd, msg, wparam, lparam)).unwrap_or((None, false));
    if browse {
        // The folder dialog runs a modal loop: show it with nothing borrowed.
        let start = with_app(|app| app.save_folder()).flatten();
        if let Some(folder) = settings::pick_folder(hwnd, start.as_deref()) {
            with_app(|app| app.change_setting(Change::SaveFolder(Some(folder))));
        }
    }
    match result {
        Some(result) => result,
        // SAFETY: default handling for this thread's window.
        None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
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
            Some(overlay::Event::Copy) => self.copy(),
            Some(overlay::Event::Save) => self.save(),
            None => {}
        }
        reply.result
    }

    fn host_message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_HOTKEY if wparam.0 == hotkey::CAPTURE_ID as usize => self.capture(),
            WM_APP_CAPTURE => self.capture(),
            WM_APP_SETTINGS => self.open_settings(),
            WM_APP_TRAY => match win::low_word(lparam.0 as usize) {
                NIN_SELECT | NIN_KEYSELECT => self.capture(),
                NIN_BALLOONUSERCLICK => self.notification_clicked(),
                _ => {}
            },
            WM_APP_SAVED => {
                // SAFETY: the save thread posted a pointer from Box::into_raw,
                // and this is its only consumer.
                let result = unsafe { Box::from_raw(lparam.0 as *mut Result<PathBuf>) };
                self.saved(*result);
            }
            WM_RENDERFORMAT => self.clipboard.render(wparam.0 as u32),
            WM_RENDERALLFORMATS => self.clipboard.render_all(self.host),
            WM_DESTROYCLIPBOARD => self.clipboard.release(),
            // The installer (and the Restart Manager) close the app.
            // SAFETY: ends the message loop of this thread.
            WM_CLOSE => unsafe { PostQuitMessage(0) },
            WM_QUERYENDSESSION => return Some(LRESULT(1)),
            WM_ENDSESSION if wparam.0 != 0 => {
                self.clipboard.render_all(self.host);
                if lparam.0 as u32 & ENDSESSION_CLOSEAPP != 0 {
                    // SAFETY: as above.
                    unsafe { PostQuitMessage(0) };
                }
            }
            WM_DISPLAYCHANGE => self.tray.refresh_icon(),
            WM_TIMER if wparam.0 == CONFIG_TIMER => {
                // SAFETY: stops this thread's window timer.
                unsafe {
                    let _ = KillTimer(Some(self.host), CONFIG_TIMER);
                }
                self.reload_config();
            }
            // Theme, accent, high contrast, transparency or animation changes.
            WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
                self.reload_theme();
                return None;
            }
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
                label: format!("Capture\t{}", self.config.hotkey),
                checked: false,
                enabled: true,
                default: true,
            },
            MenuItem::Command {
                id: CMD_OPEN_FOLDER,
                label: "Open screenshots folder".into(),
                checked: false,
                enabled: true,
                default: false,
            },
            MenuItem::Command {
                id: CMD_SETTINGS,
                label: "Settings\u{2026}".into(),
                checked: false,
                enabled: true,
                default: false,
            },
            MenuItem::Separator,
            MenuItem::Command {
                id: CMD_AUTOSTART,
                label: if self.location.portable {
                    "Start with Windows (not in portable mode)".into()
                } else {
                    "Start with Windows".into()
                },
                checked: !self.location.portable && startup::is_enabled(),
                enabled: !self.location.portable,
                default: false,
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
            CMD_OPEN_FOLDER => match self.save_folder() {
                Some(folder) => {
                    let _ = std::fs::create_dir_all(&folder);
                    open_folder(&folder);
                }
                None => self.tray.notify("Couldn't find the Screenshots folder", ""),
            },
            CMD_SETTINGS => self.open_settings(),
            CMD_AUTOSTART => {
                if let Err(err) = startup::set_enabled(!startup::is_enabled()) {
                    log!("start with Windows not changed: {err}");
                    self.tray
                        .notify("Couldn't change Start with Windows", &err.message());
                }
                self.refresh_settings();
            }
            // SAFETY: ends the message loop of this thread.
            CMD_EXIT => unsafe { PostQuitMessage(0) },
            _ => {}
        }
    }

    fn register_hotkey(&mut self) {
        self.registration = None;
        let hotkey = self.config.hotkey;
        match hotkey::register(self.host, hotkey) {
            Ok(registration) => self.registration = Some(registration),
            Err(err) => {
                log!("shortcut {hotkey} not registered: {err}");
                self.tray.notify(
                    &format!("{hotkey} is in use"),
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

    fn reload_theme(&mut self) {
        let theme = Theme::load();
        if theme != self.theme {
            self.theme = theme;
            menus::set_dark(theme.palette.dark);
            self.overlay.set_theme(theme);
            if let Some(settings) = &mut self.settings {
                settings.set_theme(&theme);
            }
        }
    }

    /// The folder for saved screenshots.
    fn save_folder(&self) -> Option<PathBuf> {
        self.config
            .save_folder
            .clone()
            .or_else(paths::screenshots_folder)
    }

    /// Opens the settings window, or brings it to the front.
    fn open_settings(&mut self) {
        if let Some(settings) = &self.settings {
            settings.show();
            return;
        }
        match Settings::open(Some(settings_proc), self.settings_values(), self.theme) {
            Ok(settings) => self.settings = Some(settings),
            Err(err) => {
                log!("settings window not opened: {err}");
                self.tray
                    .notify("Couldn't open the settings", &err.message());
            }
        }
    }

    /// The values the settings window shows.
    fn settings_values(&self) -> Values {
        Values {
            hotkey: self.config.hotkey,
            folder: self.config.save_folder.clone(),
            default_folder: paths::screenshots_folder(),
            color: self.config.color,
            width: self.config.stroke_width.round() as i64,
            autostart: !self.location.portable && startup::is_enabled(),
            portable: self.location.portable,
        }
    }

    fn refresh_settings(&mut self) {
        if self.settings.is_some() {
            let values = self.settings_values();
            if let Some(settings) = &mut self.settings {
                settings.set_values(values);
            }
        }
    }

    /// Handles a message of the settings window. Also returns whether to
    /// show the folder dialog, which must run with nothing borrowed.
    fn settings_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> (Option<LRESULT>, bool) {
        let Some(settings) = self.settings.as_mut().filter(|s| s.hwnd() == hwnd) else {
            return (None, false);
        };
        let reply = settings.message(msg, wparam, lparam);
        let browse = match reply.action {
            Some(Action::Browse) => true,
            Some(action) => {
                self.settings_action(action);
                false
            }
            None => false,
        };
        (reply.result, browse)
    }

    fn settings_action(&mut self, action: Action) {
        match action {
            Action::Change(change) => self.change_setting(change),
            // While the shortcut box listens, the shortcut must reach it.
            Action::Recording(true) => self.registration = None,
            Action::Recording(false) => self.register_hotkey(),
            Action::ToggleAutostart => self.command(CMD_AUTOSTART),
            Action::OpenFile => self.open_settings_file(),
            Action::Close => {
                self.settings = None;
                if self.registration.is_none() {
                    self.register_hotkey();
                }
                win::trim_working_set();
            }
            Action::Browse => {}
        }
    }

    /// Applies a change made in the settings window and writes it to the
    /// file. A new shortcut is registered first: when another app owns it,
    /// nothing changes and the window says why.
    fn change_setting(&mut self, change: Change) {
        if let Change::Hotkey(hotkey) = change {
            self.registration = None;
            match hotkey::register(self.host, hotkey) {
                Ok(registration) => self.registration = Some(registration),
                Err(err) => {
                    log!("shortcut {hotkey} not registered: {err}");
                    self.register_hotkey();
                    if let Some(settings) = &mut self.settings {
                        settings.hotkey_failed(format!("{hotkey} is used by another app"));
                    }
                    return;
                }
            }
        }
        if let Err(err) = self.location.save(&change) {
            log!("setting not saved: {err}");
            self.tray
                .notify("Couldn't save the settings", &err.to_string());
            if matches!(change, Change::Hotkey(_)) {
                self.register_hotkey();
            }
            self.refresh_settings();
            return;
        }
        if self.watch.is_none() {
            self.watch = FolderWatch::start(&self.location.folder);
        }
        // Read the file back, as after an edit by hand.
        self.config_stamp = None;
        self.reload_config();
        if let Change::Hotkey(hotkey) = change {
            if self.config.hotkey != hotkey {
                self.register_hotkey();
            }
        }
    }

    /// Opens the settings file, writing it from the template first if
    /// needed, in the app associated with .toml files or in Notepad.
    fn open_settings_file(&mut self) {
        let file = match self.location.ensure_file() {
            Ok(file) => file,
            Err(err) => {
                log!("settings file not created: {err}");
                self.tray
                    .notify("Couldn't create the settings file", &err.to_string());
                return;
            }
        };
        if self.watch.is_none() {
            self.watch = FolderWatch::start(&self.location.folder);
        }
        self.config_stamp = self.location.stamp();
        open_file(&file);
    }

    /// The settings folder changed: reload shortly, once the editor has
    /// finished writing.
    fn config_folder_changed(&mut self) {
        if let Some(watch) = &self.watch {
            // SAFETY: re-arms the notification owned by `watch`.
            if unsafe { FindNextChangeNotification(watch.0) }.is_err() {
                self.watch = None;
            }
        }
        // SAFETY: (re)starts a one-shot timer for this thread's window.
        unsafe { SetTimer(Some(self.host), CONFIG_TIMER, CONFIG_DELAY_MS, None) };
    }

    /// Applies the settings file if it changed.
    fn reload_config(&mut self) {
        let stamp = self.location.stamp();
        if stamp == self.config_stamp {
            return;
        }
        self.config_stamp = stamp;
        let (config, problems) = self.location.load();
        let hotkey_changed = config.hotkey != self.config.hotkey;
        self.config = config;
        self.overlay
            .set_defaults(self.config.color, self.config.stroke_width);
        if hotkey_changed {
            self.tray.set_tooltip(&tooltip(self.config.hotkey));
            self.register_hotkey();
        }
        self.report_problems(problems);
        self.refresh_settings();
    }

    /// Notifies about problems in the settings file, once per change.
    fn report_problems(&mut self, problems: Vec<String>) {
        if problems == self.config_problems {
            return;
        }
        if let Some(first) = problems.first() {
            let more = match problems.len() {
                1 => String::new(),
                n => format!(" (and {} more)", n - 1),
            };
            self.tray.notify(
                "Some settings were not applied",
                &format!("config.toml, {first}{more}"),
            );
        }
        self.config_problems = problems;
    }

    /// Copies the selection and closes the overlay.
    fn copy(&mut self) {
        let image = self.overlay.export();
        self.overlay.close();
        let copied = match image {
            Some(Ok((image, _))) => self.clipboard.copy(self.host, image),
            Some(Err(err)) => Err(err),
            None => Ok(()),
        };
        if let Err(err) = copied {
            log!("copy failed: {err}");
            self.tray
                .notify("Couldn't copy the screenshot", &err.message());
        }
    }

    /// Closes the overlay at once and saves the selection on another
    /// thread; a notification follows.
    fn save(&mut self) {
        let image = self.overlay.export();
        self.overlay.close();
        let (image, time) = match image {
            Some(Ok(image)) => image,
            Some(Err(err)) => {
                log!("export failed: {err}");
                self.tray
                    .notify("Couldn't save the screenshot", &err.message());
                return;
            }
            None => return,
        };
        let Some(folder) = self.save_folder() else {
            self.tray.notify(
                "Couldn't save the screenshot",
                "The Screenshots folder was not found.",
            );
            return;
        };
        spawn_save(PostTarget(self.host), image, folder, time);
    }

    fn saved(&mut self, result: Result<PathBuf>) {
        match result {
            Ok(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.tray.notify("Screenshot saved", &name);
                self.notification = NotificationAction::ShowFile(path);
            }
            Err(err) => {
                log!("save failed: {err}");
                self.tray
                    .notify("Couldn't save the screenshot", &err.message());
                self.notification = NotificationAction::None;
            }
        }
    }

    fn notification_clicked(&mut self) {
        match std::mem::replace(&mut self.notification, NotificationAction::None) {
            NotificationAction::ShowFile(path) => show_in_folder(&path),
            NotificationAction::None => {}
        }
    }
}

/// Encodes and writes the PNG on its own thread, then posts the result to
/// the host window.
fn spawn_save(target: PostTarget, image: Image, folder: PathBuf, time: Timestamp) {
    std::thread::spawn(move || {
        let target = target;
        // SAFETY: COM for this thread, balanced by CoUninitialize below.
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let result = export::save(&image, &folder, time);
        drop(image);
        if com {
            // SAFETY: balances the CoInitializeEx above.
            unsafe { CoUninitialize() };
        }
        let raw = Box::into_raw(Box::new(result));
        // SAFETY: the host window takes ownership of the box when it
        // handles the message; if posting fails, it is reclaimed here.
        unsafe {
            if PostMessageW(
                Some(target.0),
                WM_APP_SAVED,
                WPARAM(0),
                LPARAM(raw as isize),
            )
            .is_err()
            {
                drop(Box::from_raw(raw));
            }
        }
    });
}

/// Opens a folder in File Explorer.
fn open_folder(folder: &Path) {
    let path = win::wide(&folder.to_string_lossy());
    // SAFETY: the path is null-terminated and outlives the call.
    unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            win::pcwstr(&path),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Opens a file in its associated app, or in Notepad when it has none.
fn open_file(file: &Path) {
    let path = win::wide(&file.to_string_lossy());
    // SAFETY: the strings are null-terminated and outlive the calls.
    unsafe {
        let result = ShellExecuteW(
            None,
            w!("open"),
            win::pcwstr(&path),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        // Values up to 32 are errors, such as no app for the file type.
        if result.0 as usize <= 32 {
            let quoted = win::wide(&format!("\"{}\"", file.display()));
            ShellExecuteW(
                None,
                w!("open"),
                w!("notepad.exe"),
                win::pcwstr(&quoted),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            );
        }
    }
}

/// Opens File Explorer on the file's folder with the file selected.
fn show_in_folder(file: &Path) {
    let path = win::wide(&file.to_string_lossy());
    // SAFETY: the item list is created from a null-terminated path and
    // freed after use.
    unsafe {
        let item = ILCreateFromPathW(win::pcwstr(&path));
        if item.is_null() {
            return;
        }
        let _ = SHOpenFolderAndSelectItems(item, None, 0);
        ILFree(Some(item));
    }
}
