//! Reads values of the current user's registry hive.

use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD,
};

use crate::win;

/// A `REG_DWORD` value under `HKEY_CURRENT_USER\key`.
pub fn dword(key: &str, value: &str) -> Option<u32> {
    let (key, value) = (win::wide(key), win::wide(value));
    let mut data = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: the strings are null-terminated and `data` holds `size` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            win::pcwstr(&key),
            win::pcwstr(&value),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(data)
}

/// A `REG_BINARY` value of up to 256 bytes under `HKEY_CURRENT_USER\key`.
pub fn binary(key: &str, value: &str) -> Option<Vec<u8>> {
    let (key, value) = (win::wide(key), win::wide(value));
    let mut data = vec![0u8; 256];
    let mut size = data.len() as u32;
    // SAFETY: the strings are null-terminated and `data` holds `size` bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            win::pcwstr(&key),
            win::pcwstr(&value),
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then(|| {
        data.truncate(size as usize);
        data
    })
}
