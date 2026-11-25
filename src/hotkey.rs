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

    /// Parses a combination such as `Ctrl+Alt+S` or `Shift+PrintScreen`.
    /// Letters, digits and Space need at least one modifier, so typing is
    /// never captured.
    pub fn parse(text: &str) -> Result<Hotkey, String> {
        let mut hotkey = Hotkey {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            key: 0,
        };
        let parts: Vec<&str> = text.split('+').map(str::trim).collect();
        let Some((key_name, modifiers)) = parts.split_last() else {
            return Err("the shortcut is empty".into());
        };
        for modifier in modifiers {
            let flag = match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut hotkey.ctrl,
                "alt" => &mut hotkey.alt,
                "shift" => &mut hotkey.shift,
                "win" | "windows" => &mut hotkey.win,
                "" => return Err(format!("\"{text}\" has an empty part")),
                other => return Err(format!("\"{other}\" is not a modifier key")),
            };
            if *flag {
                return Err(format!("\"{modifier}\" appears twice"));
            }
            *flag = true;
        }
        hotkey.key =
            key_code(key_name).ok_or_else(|| format!("\"{key_name}\" is not a supported key"))?;
        let typing_key = matches!(hotkey.key, 0x20 | 0x30..=0x39 | 0x41..=0x5A);
        if typing_key && !(hotkey.ctrl || hotkey.alt || hotkey.win) {
            return Err(format!(
                "\"{text}\" needs Ctrl, Alt or Win, so it does not block typing"
            ));
        }
        Ok(hotkey)
    }

    /// The combination of a key pressed with modifiers, as recorded by the
    /// settings window. `None` for a modifier alone (the user is still
    /// pressing the combination); an error for a key that cannot be a
    /// shortcut.
    pub fn from_keys(
        key: u32,
        ctrl: bool,
        alt: bool,
        shift: bool,
        win: bool,
    ) -> Option<Result<Hotkey, String>> {
        // Shift, Ctrl, Alt (and their left/right codes), and the Win keys.
        if matches!(key, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C) {
            return None;
        }
        let hotkey = Hotkey {
            ctrl,
            alt,
            shift,
            win,
            key,
        };
        // The text form validates the key and the typing rule.
        Some(Hotkey::parse(&hotkey.to_string()).map_err(|_| {
            if key_code(&key_name(key)).is_some() {
                "Add Ctrl, Alt or Win, so typing still works".to_string()
            } else {
                "That key can't be used in a shortcut".to_string()
            }
        }))
    }

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

fn key_code(name: &str) -> Option<u32> {
    let upper = name.to_ascii_uppercase();
    if let [c] = upper.as_bytes() {
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(u32::from(*c));
        }
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        if (1..=24).contains(&n) {
            return Some(0x70 + n - 1);
        }
    }
    let alias = match upper.as_str() {
        "PRTSC" | "PRTSCN" | "PRINT" => "PrintScreen",
        "DEL" => "Delete",
        "INS" => "Insert",
        "PGUP" => "PageUp",
        "PGDN" => "PageDown",
        _ => name,
    };
    NAMED_KEYS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(alias))
        .map(|&(_, code)| code)
}

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
    fn parses_the_default() {
        assert_eq!(Hotkey::parse("Ctrl+Alt+S"), Ok(Hotkey::DEFAULT));
        assert_eq!(Hotkey::parse(" ctrl + ALT + s "), Ok(Hotkey::DEFAULT));
    }

    #[test]
    fn displays_in_canonical_order() {
        let hotkey = Hotkey::parse("Shift+Win+Ctrl+F5").unwrap();
        assert_eq!(hotkey.to_string(), "Ctrl+Shift+Win+F5");
        assert_eq!(Hotkey::DEFAULT.to_string(), "Ctrl+Alt+S");
    }

    #[test]
    fn named_keys_round_trip() {
        for text in [
            "PrintScreen",
            "Shift+PrintScreen",
            "Ctrl+Alt+Delete",
            "Win+F24",
        ] {
            assert_eq!(Hotkey::parse(text).unwrap().to_string(), text);
        }
        assert_eq!(Hotkey::parse("PrtSc").unwrap().key, 0x2C);
    }

    #[test]
    fn typing_keys_need_a_modifier() {
        assert!(Hotkey::parse("S").is_err());
        assert!(Hotkey::parse("Shift+S").is_err());
        assert!(Hotkey::parse("Space").is_err());
        assert!(Hotkey::parse("Alt+1").is_ok());
        assert!(Hotkey::parse("F9").is_ok());
    }

    #[test]
    fn rejects_invalid_text() {
        for text in [
            "",
            "Ctrl+",
            "Ctrl+Ctrl+S",
            "Hyper+S",
            "Ctrl+F25",
            "Ctrl+Enter",
        ] {
            assert!(Hotkey::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn records_pressed_keys() {
        assert_eq!(
            Hotkey::from_keys(u32::from(b'S'), true, true, false, false),
            Some(Ok(Hotkey::DEFAULT))
        );
        assert_eq!(
            Hotkey::from_keys(0x2C, false, false, true, false)
                .unwrap()
                .unwrap()
                .to_string(),
            "Shift+PrintScreen"
        );
        // A modifier alone: still pressing.
        assert_eq!(Hotkey::from_keys(0x11, true, false, false, false), None);
        assert_eq!(Hotkey::from_keys(0xA4, false, true, false, false), None);
        // A letter alone would block typing; Enter is not supported.
        assert!(
            Hotkey::from_keys(u32::from(b'A'), false, false, true, false)
                .unwrap()
                .is_err()
        );
        assert!(Hotkey::from_keys(0x0D, true, false, false, false)
            .unwrap()
            .is_err());
    }
}
