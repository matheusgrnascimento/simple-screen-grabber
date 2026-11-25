//! "Start with Windows": a value under the current user's `Run` key, the
//! same one the installer writes.

use windows::core::{w, Result, PCWSTR};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

use crate::win;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("SimpleScreenGrabber");

/// Whether the app starts with Windows.
pub fn is_enabled() -> bool {
    let mut size = 0u32;
    // SAFETY: asks only for the size of the value, if it exists.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    status.is_ok()
}

/// Turns starting with Windows on (for this executable) or off.
pub fn set_enabled(enabled: bool) -> Result<()> {
    if !enabled {
        // SAFETY: deletes one value under the current user's Run key.
        return unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) }.ok();
    }
    let exe = std::env::current_exe()
        .map_err(|e| windows::core::Error::new(windows::core::HRESULT(-1), e.to_string()))?;
    let command = win::wide(&format!("\"{}\" --autostart", exe.display()));
    // SAFETY: writes a null-terminated string value of the given size.
    unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            REG_SZ.0,
            Some(command.as_ptr().cast()),
            (command.len() * 2) as u32,
        )
    }
    .ok()
}
