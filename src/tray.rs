//! The notification area icon, its menu and its notifications.

use windows::core::{Error, Result};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_LARGE_ICON,
    NIIF_NOSOUND, NIIF_RESPECT_QUIET_TIME, NIIF_USER, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NIM_SETVERSION, NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, GetSystemMetrics, LoadImageW, SetForegroundWindow,
    SetMenuDefaultItem, TrackPopupMenuEx, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, MENU_ITEM_FLAGS,
    MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, SM_CXICON, SM_CXSMICON, SM_MENUDROPALIGNMENT,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTALIGN, TPM_RIGHTBUTTON,
};

use crate::win::{self, Icon, Menu};

/// The id of the one icon of the app.
const ICON_UID: u32 = 1;
/// The app icon's resource id (see build.rs).
const ICON_RESOURCE: u16 = 1;

/// The app's icon in the notification area. Removed on drop.
pub struct Tray {
    hwnd: HWND,
    callback: u32,
    small: Icon,
    large: Icon,
    tooltip: String,
}

impl Tray {
    /// Adds the icon. `callback` is the message the shell sends to `hwnd`
    /// for clicks and notifications (version 4 semantics).
    pub fn new(hwnd: HWND, callback: u32, tooltip: &str) -> Result<Tray> {
        let (small, large) = load_icons()?;
        let tray = Tray {
            hwnd,
            callback,
            small,
            large,
            tooltip: tooltip.to_owned(),
        };
        // At sign-in the taskbar may not exist yet; the icon is added again
        // on `TaskbarCreated`.
        if let Err(err) = tray.add() {
            crate::log!("tray icon not added yet: {err}");
        }
        Ok(tray)
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ICON_UID,
            ..Default::default()
        }
    }

    /// Adds the icon again, after Explorer restarts (`TaskbarCreated`).
    pub fn add(&self) -> Result<()> {
        let mut data = self.data();
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = self.callback;
        data.hIcon = self.small.0;
        win::copy_to_field(&mut data.szTip, &self.tooltip);
        // SAFETY: `data` is a fully initialized NOTIFYICONDATAW.
        if !unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.as_bool() {
            return Err(Error::from_thread());
        }
        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: as above.
        if !unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) }.as_bool() {
            return Err(Error::from_thread());
        }
        Ok(())
    }

    /// Reloads the icon at the current system DPI (after a display change).
    pub fn refresh_icon(&mut self) {
        if let Ok((small, large)) = load_icons() {
            self.small = small;
            self.large = large;
            let mut data = self.data();
            data.uFlags = NIF_ICON;
            data.hIcon = self.small.0;
            // SAFETY: `data` is a fully initialized NOTIFYICONDATAW.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
            }
        }
    }

    /// Shows a notification. Windows shows it as a toast and sends
    /// `NIN_BALLOONUSERCLICK` when it is clicked.
    pub fn notify(&self, title: &str, text: &str) {
        let mut data = self.data();
        data.uFlags = NIF_INFO;
        win::copy_to_field(&mut data.szInfoTitle, title);
        win::copy_to_field(&mut data.szInfo, text);
        data.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON | NIIF_NOSOUND | NIIF_RESPECT_QUIET_TIME;
        data.hBalloonIcon = self.large.0;
        // SAFETY: `data` is a fully initialized NOTIFYICONDATAW.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        let data = self.data();
        // SAFETY: removes the icon this wrapper added.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
        }
    }
}

fn load_icons() -> Result<(Icon, Icon)> {
    // SAFETY: plain system metric queries.
    let dpi = unsafe { GetDpiForSystem() };
    let load = |metric| -> Result<Icon> {
        // SAFETY: as above.
        let size = unsafe { GetSystemMetricsForDpi(metric, dpi) };
        // SAFETY: loads the icon group embedded by build.rs; the result is
        // owned by the returned wrapper.
        let handle = unsafe {
            LoadImageW(
                Some(win::instance()),
                win::resource_id(ICON_RESOURCE),
                IMAGE_ICON,
                size,
                size,
                LR_DEFAULTCOLOR,
            )?
        };
        Ok(Icon(HICON(handle.0)))
    };
    Ok((load(SM_CXSMICON)?, load(SM_CXICON)?))
}

/// One entry of the tray menu.
pub enum MenuItem {
    Command {
        id: u32,
        label: String,
        checked: bool,
        enabled: bool,
        default: bool,
    },
    Separator,
}

/// Shows a popup menu at `anchor` (screen coordinates) and returns the id
/// of the chosen command, if any. Runs a modal loop: call it while no app
/// state is borrowed.
pub fn show_menu(hwnd: HWND, anchor: POINT, items: &[MenuItem]) -> Option<u32> {
    // SAFETY: the menu is owned by the wrapper and destroyed after use;
    // every string outlives the AppendMenuW call that copies it.
    unsafe {
        let menu = Menu(CreatePopupMenu().ok()?);
        for item in items {
            match item {
                MenuItem::Separator => {
                    AppendMenuW(menu.0, MF_SEPARATOR, 0, None).ok()?;
                }
                MenuItem::Command {
                    id,
                    label,
                    checked,
                    enabled,
                    default,
                } => {
                    let mut flags: MENU_ITEM_FLAGS = MF_STRING;
                    if *checked {
                        flags |= MF_CHECKED;
                    }
                    if !*enabled {
                        flags |= MF_GRAYED;
                    }
                    let text = win::wide(label);
                    AppendMenuW(menu.0, flags, *id as usize, win::pcwstr(&text)).ok()?;
                    if *default {
                        let _ = SetMenuDefaultItem(menu.0, *id, 0);
                    }
                }
            }
        }
        // The menu only closes on a click elsewhere if the window that owns
        // it is in the foreground.
        let _ = SetForegroundWindow(hwnd);
        let align = if GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0 {
            TPM_RIGHTALIGN
        } else {
            TPM_LEFTALIGN
        };
        let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | align;
        let id = TrackPopupMenuEx(menu.0, flags.0, anchor.x, anchor.y, hwnd, None).0;
        u32::try_from(id).ok().filter(|&id| id != 0)
    }
}
