//! The global capture shortcut: parsing ("Ctrl+Alt+S"), display and
//! registration with `RegisterHotKey`.
//!
//! There is no low-level keyboard hook: it would route every keystroke on
//! the system through this process.

use std::fmt;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};

/// The id passed to `RegisterHotKey`, echoed in `WM_HOTKEY`.
pub const CAPTURE_ID: i32 = 1;

/// A key combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// The virtual-key code.
    pub key: u32,
}

impl Hotkey {
    /// The default shortcut, Ctrl+Alt+S.
    pub const DEFAULT: Hotkey = Hotkey {
        ctrl: true,
        alt: true,
        shift: false,
        win: false,
        key: b'S' as u32,
    };

    fn modifiers(&self) -> HOT_KEY_MODIFIERS {
        let mut flags = MOD_NOREPEAT;
        for (on, flag) in [
            (self.ctrl, MOD_CONTROL),
            (self.alt, MOD_ALT),
            (self.shift, MOD_SHIFT),
            (self.win, MOD_WIN),
        ] {
            if on {
                flags |= flag;
            }
        }
        flags
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (on, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.win, "Win"),
        ] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        f.write_str(&key_name(self.key))
    }
}

const NAMED_KEYS: [(&str, u32); 10] = [
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("PrintScreen", 0x2C),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("Pause", 0x13),
    ("ScrollLock", 0x91),
];

fn key_name(code: u32) -> String {
    match code {
        0x30..=0x39 | 0x41..=0x5A => char::from(code as u8).to_string(),
        0x70..=0x87 => format!("F{}", code - 0x70 + 1),
        _ => NAMED_KEYS
            .iter()
            .find(|&&(_, c)| c == code)
            .map_or_else(|| format!("0x{code:02X}"), |&(n, _)| n.to_string()),
    }
}

/// A registered shortcut, unregistered on drop.
pub struct Registration {
    hwnd: HWND,
}

/// Registers `hotkey` for `hwnd`, which then receives `WM_HOTKEY` with
/// [`CAPTURE_ID`]. Fails when another app already owns the combination.
pub fn register(hwnd: HWND, hotkey: Hotkey) -> windows::core::Result<Registration> {
    // SAFETY: `hwnd` is a window of this thread.
    unsafe { RegisterHotKey(Some(hwnd), CAPTURE_ID, hotkey.modifiers(), hotkey.key)? };
    Ok(Registration { hwnd })
}

impl Drop for Registration {
    fn drop(&mut self) {
        // SAFETY: the shortcut was registered for this window and id.
        let _ = unsafe { UnregisterHotKey(Some(self.hwnd), CAPTURE_ID) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_in_canonical_order() {
        let hotkey = Hotkey {
            ctrl: true,
            alt: false,
            shift: true,
            win: true,
            key: 0x74,
        };
        assert_eq!(hotkey.to_string(), "Ctrl+Shift+Win+F5");
        assert_eq!(Hotkey::DEFAULT.to_string(), "Ctrl+Alt+S");
    }
}
