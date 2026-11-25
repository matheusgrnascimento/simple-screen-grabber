//! Light or dark popup menus, like Explorer's.
//!
//! Win32 menus follow the dark theme only through two exports of
//! uxtheme.dll that have no names, only ordinals, and are stable since
//! Windows 10 1903: 135 `SetPreferredAppMode` and 136 `FlushMenuThemes`.
//! Where they are missing the menus simply stay light.

use windows::core::{w, PCSTR};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

/// `PreferredAppMode` values.
const FORCE_DARK: i32 = 2;
const FORCE_LIGHT: i32 = 3;

/// Makes this app's menus dark or light.
pub fn set_dark(dark: bool) {
    // SAFETY: loads a system DLL by name from System32 (it stays loaded)
    // and calls two exports with the signatures they have had since
    // Windows 10 1903.
    unsafe {
        let Ok(uxtheme) = LoadLibraryExW(w!("uxtheme.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)
        else {
            return;
        };
        let by_ordinal = |ordinal: usize| GetProcAddress(uxtheme, PCSTR(ordinal as *const u8));
        if let Some(set_preferred_app_mode) = by_ordinal(135) {
            let f: extern "system" fn(i32) -> i32 = std::mem::transmute(set_preferred_app_mode);
            f(if dark { FORCE_DARK } else { FORCE_LIGHT });
        }
        if let Some(flush_menu_themes) = by_ordinal(136) {
            let f: extern "system" fn() = std::mem::transmute(flush_menu_themes);
            f();
        }
    }
}
