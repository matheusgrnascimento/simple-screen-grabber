//! Minimal diagnostics: messages go to the debugger output
//! (`OutputDebugStringW`), visible in Visual Studio or DebugView. Never log
//! screenshot contents or file contents.

use std::fmt;

use windows::core::PCWSTR;
use windows::Win32::System::Diagnostics::Debug::OutputDebugStringW;

/// Writes one line to the debugger output.
pub fn write(args: fmt::Arguments<'_>) {
    let line = format!("simple-screen-grabber: {args}\n");
    let wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
    // SAFETY: `wide` is a null-terminated UTF-16 string that outlives the call.
    unsafe { OutputDebugStringW(PCWSTR(wide.as_ptr())) };
}

/// Logs a formatted line to the debugger output.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write(format_args!($($arg)*))
    };
}
