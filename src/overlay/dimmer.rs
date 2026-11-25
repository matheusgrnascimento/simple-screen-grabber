//! Instant feedback for the shortcut: a plain translucent black window that
//! dims the screen right away, while the screen is still being copied.
//!
//! Copying the screen through GDI takes 30 to 40 ms on a 3440x1440 desktop
//! (the compositor reads the frame back from the GPU). This window shows in
//! about 1 ms and is on screen at the next composition, so the screen
//! reacts at once; the frozen overlay, with the same dimming, replaces it
//! when ready. It is excluded from screen capture
//! (`WDA_EXCLUDEFROMCAPTURE`, Windows 10 2004 and later), so it never ends
//! up in the screenshot; where that is not available it is not used.

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetSystemMetrics, RegisterClassExW,
    SetLayeredWindowAttributes, SetWindowDisplayAffinity, SetWindowPos, ShowWindow, HWND_TOPMOST,
    LWA_ALPHA, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WDA_EXCLUDEFROMCAPTURE, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::win;

const CLASS: PCWSTR = w!("SimpleScreenGrabber.Dimmer");

pub struct Dimmer {
    hwnd: HWND,
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: default handling for this thread's window (it paints black).
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl Dimmer {
    /// Creates the hidden window, or `None` where it cannot be excluded
    /// from screen capture.
    pub fn new(alpha: u8) -> Result<Option<Dimmer>> {
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: win::instance(),
            lpszClassName: CLASS,
            // SAFETY: stock objects are never freed.
            hbrBackground: HBRUSH(unsafe { GetStockObject(BLACK_BRUSH) }.0),
            ..Default::default()
        };
        // SAFETY: registers the class and creates a hidden, click-through
        // window of this thread that never takes the focus.
        unsafe {
            RegisterClassExW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                CLASS,
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(win::instance()),
                None,
            )?;
            SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA)?;
            if SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE).is_err() {
                let _ = DestroyWindow(hwnd);
                return Ok(None);
            }
            Ok(Some(Dimmer { hwnd }))
        }
    }

    /// Dims the whole virtual screen.
    pub fn show(&self) {
        // SAFETY: shows this thread's window without activating it.
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            );
        }
    }

    pub fn hide(&self) {
        // SAFETY: hides this thread's window.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}
