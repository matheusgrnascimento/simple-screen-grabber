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

    /// Parses a combination such as `Ctrl+Alt+S`, `Shift+PrintScreen` or a
    /// single key such as `NumMultiply` (alias `*`). Any key can be the
    /// shortcut, with or without modifiers; `0xNN` names a virtual-key code
    /// that has no name. A key without modifiers stops working as a typing
    /// key in every app while the shortcut is registered.
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
        Some(
            Hotkey::parse(&hotkey.to_string())
                .map_err(|_| "That key can't be used in a shortcut".to_string()),
        )
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

const NAMED_KEYS: &[(&str, u32)] = &[
    ("Backspace", 0x08),
    ("Tab", 0x09),
    ("Enter", 0x0D),
    ("Pause", 0x13),
    ("CapsLock", 0x14),
    ("Escape", 0x1B),
    ("Space", 0x20),
    ("PageUp", 0x21),
    ("PageDown", 0x22),
    ("End", 0x23),
    ("Home", 0x24),
    ("Left", 0x25),
    ("Up", 0x26),
    ("Right", 0x27),
    ("Down", 0x28),
    ("PrintScreen", 0x2C),
    ("Insert", 0x2D),
    ("Delete", 0x2E),
    ("Num0", 0x60),
    ("Num1", 0x61),
    ("Num2", 0x62),
    ("Num3", 0x63),
    ("Num4", 0x64),
    ("Num5", 0x65),
    ("Num6", 0x66),
    ("Num7", 0x67),
    ("Num8", 0x68),
    ("Num9", 0x69),
    ("NumMultiply", 0x6A),
    ("NumAdd", 0x6B),
    ("NumSubtract", 0x6D),
    ("NumDecimal", 0x6E),
    ("NumDivide", 0x6F),
    ("NumLock", 0x90),
    ("ScrollLock", 0x91),
    (";", 0xBA),
    ("=", 0xBB),
    (",", 0xBC),
    ("-", 0xBD),
    (".", 0xBE),
    ("/", 0xBF),
    ("`", 0xC0),
    ("[", 0xDB),
    ("\\", 0xDC),
    ("]", 0xDD),
    ("'", 0xDE),
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
    if let Some(code) = upper
        .strip_prefix("0X")
        .and_then(|h| u32::from_str_radix(h, 16).ok())
    {
        // Any virtual-key code but the modifiers (and the mouse buttons).
        let modifier = matches!(code, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C);
        return ((0x07..=0xFE).contains(&code) && !modifier).then_some(code);
    }
    let alias = match upper.as_str() {
        "*" | "NUM*" => "NumMultiply",
        "ESC" => "Escape",
        "RETURN" => "Enter",
        "BKSP" => "Backspace",
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
    fn any_key_works_with_or_without_modifiers() {
        for text in ["S", "Shift+S", "Space", "Alt+1", "F9", "Ctrl+Enter", "Up"] {
            assert!(Hotkey::parse(text).is_ok(), "{text}");
        }
        let star = Hotkey::parse("*").unwrap();
        assert_eq!(star.key, 0x6A);
        assert_eq!(star.to_string(), "NumMultiply");
        assert_eq!(Hotkey::parse("Shift+8").unwrap().to_string(), "Shift+8");
        assert_eq!(Hotkey::parse("Ctrl+;").unwrap().key, 0xBA);
        // A key with no name round trips as its code.
        let odd = Hotkey::parse("Ctrl+0xE2").unwrap();
        assert_eq!(odd.to_string(), "Ctrl+0xE2");
        assert_eq!(Hotkey::parse(&odd.to_string()), Ok(odd));
    }

    #[test]
    fn rejects_invalid_text() {
        for text in [
            "",
            "Ctrl+",
            "Ctrl+Ctrl+S",
            "Hyper+S",
            "Ctrl+F25",
            "Ctrl+Shift",
            "0x10",
            "0x00",
            "Ctrl+0x1FF",
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
        // A letter alone, and keys with no name, are fine.
        assert!(
            Hotkey::from_keys(u32::from(b'A'), false, false, false, false)
                .unwrap()
                .is_ok()
        );
        assert_eq!(
            Hotkey::from_keys(0x6A, false, false, false, false)
                .unwrap()
                .unwrap()
                .to_string(),
            "NumMultiply"
        );
        // A mouse button code is not a key.
        assert!(Hotkey::from_keys(0x01, false, false, false, false)
            .unwrap()
            .is_err());
    }
}
