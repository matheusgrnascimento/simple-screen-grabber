//! Captures the whole virtual screen (every monitor) in physical pixels.

use std::ffi::c_void;
use std::ptr::null_mut;

use windows::core::{Error, Result, BOOL};
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, EnumDisplayMonitors,
    GetDC, GetMonitorInfoW, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    CAPTUREBLT, DIB_RGB_COLORS, HBITMAP, HDC, HMONITOR, MONITORINFO, SRCCOPY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use crate::geometry::{Point, Rect};

/// A monitor, in overlay coordinates (virtual screen, origin at 0, 0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub rect: Rect,
    /// Display scale: 1.0 at 96 DPI (100%), 1.5 at 144 DPI (150%).
    pub scale: f32,
}

/// The pixels of the virtual screen: top-down rows of BGRX, 4 bytes per
/// pixel (the fourth byte is undefined and treated as opaque).
pub struct Capture {
    /// The virtual screen's top-left corner in desktop coordinates.
    pub origin: Point,
    pub width: i32,
    pub height: i32,
    pub monitors: Vec<Monitor>,
    bitmap: HBITMAP,
    bits: *const u8,
}

impl Capture {
    /// The whole capture as a rectangle in overlay coordinates.
    pub const fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    pub const fn stride(&self) -> usize {
        self.width as usize * 4
    }

    pub fn pixels(&self) -> &[u8] {
        // SAFETY: `bits` points to the DIB section owned by `bitmap`, which
        // holds height rows of stride bytes and lives as long as `self`.
        unsafe { std::slice::from_raw_parts(self.bits, self.stride() * self.height as usize) }
    }

    /// The monitor that contains `p`, or the nearest one.
    pub fn monitor_at(&self, p: Point) -> Monitor {
        let distance = |m: &Monitor| {
            let c = m.rect.clamp(p);
            i64::from(c.x - p.x).pow(2) + i64::from(c.y - p.y).pow(2)
        };
        self.monitors
            .iter()
            .copied()
            .min_by_key(distance)
            .unwrap_or(Monitor {
                rect: self.bounds(),
                scale: 1.0,
            })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // SAFETY: the DIB section is owned by this capture and no longer
        // selected into a DC.
        unsafe {
            let _ = DeleteObject(self.bitmap.into());
        }
    }
}

/// A screen DC released on drop.
struct ScreenDc(HDC);

impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: the DC came from GetDC(None).
        unsafe { ReleaseDC(None, self.0) };
    }
}

/// A memory DC deleted on drop.
struct MemoryDc(HDC);

impl Drop for MemoryDc {
    fn drop(&mut self) {
        // SAFETY: the DC came from CreateCompatibleDC.
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}

/// Copies the virtual screen into memory.
pub fn capture_screen() -> Result<Capture> {
    // SAFETY: system metric queries; with per-monitor DPI awareness they
    // are in physical pixels.
    let (x, y, width, height) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if width <= 0 || height <= 0 {
        return Err(Error::from_thread());
    }
    let monitors = monitors(Point::new(x, y));

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            // Negative height: rows top-down.
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: the DCs and the DIB section are owned by wrappers or by the
    // returned capture; the bitmap is deselected before the DC goes away.
    unsafe {
        let screen = ScreenDc(GetDC(None));
        let memory = MemoryDc(CreateCompatibleDC(Some(screen.0)));
        let mut bits: *mut c_void = null_mut();
        let bitmap = CreateDIBSection(Some(memory.0), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let capture = Capture {
            origin: Point::new(x, y),
            width,
            height,
            monitors,
            bitmap,
            bits: bits.cast_const().cast(),
        };
        let previous = SelectObject(memory.0, bitmap.into());
        // CAPTUREBLT includes layered windows (menus, tooltips).
        let copied = BitBlt(
            memory.0,
            0,
            0,
            width,
            height,
            Some(screen.0),
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        );
        SelectObject(memory.0, previous);
        copied?;
        Ok(capture)
    }
}

/// Every monitor, in overlay coordinates (relative to `origin`).
fn monitors(origin: Point) -> Vec<Monitor> {
    let mut monitors: Vec<Monitor> = Vec::new();
    extern "system" fn collect(monitor: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        // SAFETY: `data` is the address of the vector passed below, alive
        // for the whole EnumDisplayMonitors call.
        let list = unsafe { &mut *(data.0 as *mut Vec<(HMONITOR, RECT, u32)>) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let (mut dpi_x, mut dpi_y) = (96, 96);
        // SAFETY: queries a monitor handle given by the enumeration.
        unsafe {
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
                list.push((monitor, info.rcMonitor, dpi_x));
            }
        }
        true.into()
    }
    let mut raw: Vec<(HMONITOR, RECT, u32)> = Vec::new();
    // SAFETY: the callback only touches `raw` during the call.
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut raw as *mut _ as isize),
        );
    }
    for (_, r, dpi) in raw {
        monitors.push(Monitor {
            rect: Rect::new(r.left, r.top, r.right, r.bottom).offset(-origin.x, -origin.y),
            scale: dpi as f32 / 96.0,
        });
    }
    monitors
}
