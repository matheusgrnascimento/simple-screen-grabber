//! Small helpers and owning wrappers around Win32 handles.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HINSTANCE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, DestroyMenu, HICON, HMENU};

/// A null-terminated UTF-16 copy of `s`.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// A `PCWSTR` pointing into a null-terminated buffer from [`wide`]. The
/// buffer must outlive every use of the pointer.
pub fn pcwstr(buffer: &[u16]) -> PCWSTR {
    PCWSTR(buffer.as_ptr())
}

/// A resource id as a `PCWSTR` (the `MAKEINTRESOURCEW` macro).
pub fn resource_id(id: u16) -> PCWSTR {
    PCWSTR(usize::from(id) as *const u16)
}

/// Copies `s` into a fixed-size UTF-16 field, truncated and null-terminated.
pub fn copy_to_field(field: &mut [u16], s: &str) {
    let Some(max) = field.len().checked_sub(1) else {
        return;
    };
    let mut n = 0;
    for (slot, unit) in field.iter_mut().zip(s.encode_utf16().take(max)) {
        *slot = unit;
        n += 1;
    }
    field[n..].fill(0);
}

/// The module handle of the executable.
pub fn instance() -> HINSTANCE {
    // SAFETY: a null module name asks for the executable, which is always
    // loaded while the process runs.
    unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map(HINSTANCE::from)
        .unwrap_or_default()
}

/// The signed x coordinate packed in the low word of an `LPARAM`.
pub fn x_of(value: isize) -> i32 {
    i32::from(value as u16 as i16)
}

/// The signed y coordinate packed in the high word of an `LPARAM`.
pub fn y_of(value: isize) -> i32 {
    i32::from((value >> 16) as u16 as i16)
}

/// The low word of a message parameter.
pub fn low_word(value: usize) -> u32 {
    (value & 0xFFFF) as u32
}

/// An icon destroyed on drop.
pub struct Icon(pub HICON);

impl Drop for Icon {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the icon was created by this process and is not used
            // after the wrapper is dropped.
            let _ = unsafe { DestroyIcon(self.0) };
        }
    }
}

/// A menu destroyed on drop.
pub struct Menu(pub HMENU);

impl Drop for Menu {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the menu was created by this process and is not
            // attached to a window.
            let _ = unsafe { DestroyMenu(self.0) };
        }
    }
}

/// A kernel handle closed on drop.
pub struct Handle(pub HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the handle is owned by this wrapper.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_to_field_truncates_and_terminates() {
        let mut field = [1u16; 4];
        copy_to_field(&mut field, "abcdef");
        assert_eq!(field, [b'a'.into(), b'b'.into(), b'c'.into(), 0]);
        copy_to_field(&mut field, "x");
        assert_eq!(field, [b'x'.into(), 0, 0, 0]);
    }

    #[test]
    fn coordinates_are_signed() {
        let packed = ((-5i32 as u16 as isize) << 16) | (-20i32 as u16 as isize);
        assert_eq!((x_of(packed), y_of(packed)), (-20, -5));
    }
}
