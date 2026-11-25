//! Copies images to the clipboard with delayed rendering: copying only
//! announces the formats, which is instant. The bitmap (`CF_DIB`) or the PNG
//! is produced when an app pastes and asks for it (`WM_RENDERFORMAT`), or for
//! all formats when this app exits (`WM_RENDERALLFORMATS`).
//!
//! The image is kept only while this app owns the clipboard, and released
//! when another app takes it (`WM_DESTROYCLIPBOARD`).

use windows::core::{w, Result, HRESULT};
use windows::Win32::Foundation::GlobalFree;
use windows::Win32::Foundation::{SetLastError, HANDLE, HWND, WIN32_ERROR};
use windows::Win32::Graphics::Gdi::{BITMAPINFOHEADER, BI_RGB};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardOwner, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_DIB;

use super::{png, Image};
use crate::log;

pub struct Clipboard {
    /// The registered "PNG" format, read by Office, browsers and most image
    /// apps.
    png_format: u32,
    image: Option<Image>,
}

impl Clipboard {
    pub fn new() -> Clipboard {
        // SAFETY: registers (or looks up) a clipboard format name.
        let png_format = unsafe { RegisterClipboardFormatW(w!("PNG")) };
        Clipboard {
            png_format,
            image: None,
        }
    }

    /// Puts `image` on the clipboard, owned by `owner`.
    pub fn copy(&mut self, owner: HWND, image: Image) -> Result<()> {
        // SAFETY: the clipboard is opened, filled with delayed formats and
        // closed on every path.
        unsafe {
            OpenClipboard(Some(owner))?;
            let result = (|| -> Result<()> {
                EmptyClipboard()?;
                announce(u32::from(CF_DIB.0))?;
                if self.png_format != 0 {
                    announce(self.png_format)?;
                }
                Ok(())
            })();
            let _ = CloseClipboard();
            result?;
        }
        // The WM_DESTROYCLIPBOARD that EmptyClipboard sends for a previous
        // image of this app arrives while the app state is borrowed and is
        // skipped: the new image simply replaces it here.
        self.image = Some(image);
        Ok(())
    }

    /// Answers `WM_RENDERFORMAT`: the clipboard is already open by the app
    /// that pastes.
    pub fn render(&self, format: u32) {
        if let Some(data) = self.data(format) {
            set_data(format, &data);
        }
    }

    /// Answers `WM_RENDERALLFORMATS`, sent when this app exits while it
    /// still owns the clipboard.
    pub fn render_all(&self, owner: HWND) {
        if self.image.is_none() {
            return;
        }
        // SAFETY: opens the clipboard, fills it if this app still owns it,
        // and closes it.
        unsafe {
            if OpenClipboard(Some(owner)).is_err() {
                return;
            }
            if GetClipboardOwner().ok() == Some(owner) {
                for format in [u32::from(CF_DIB.0), self.png_format] {
                    self.render(format);
                }
            }
            let _ = CloseClipboard();
        }
    }

    /// Answers `WM_DESTROYCLIPBOARD`: another app took the clipboard.
    pub fn release(&mut self) {
        self.image = None;
    }

    fn data(&self, format: u32) -> Option<Vec<u8>> {
        let image = self.image.as_ref()?;
        if format == u32::from(CF_DIB.0) {
            Some(dib(image))
        } else if format == self.png_format && format != 0 {
            png::to_bytes(image)
                .map_err(|err| log!("PNG for the clipboard failed: {err}"))
                .ok()
        } else {
            None
        }
    }
}

/// A packed device-independent bitmap: header then bottom-up rows of 32-bit
/// pixels with opaque alpha.
fn dib(image: &Image) -> Vec<u8> {
    let header = BITMAPINFOHEADER {
        biSize: size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: image.width as i32,
        biHeight: image.height as i32,
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        biSizeImage: (image.stride() * image.height as usize) as u32,
        ..Default::default()
    };
    let mut data = Vec::with_capacity(size_of::<BITMAPINFOHEADER>() + image.pixels.len());
    // SAFETY: BITMAPINFOHEADER is a plain C struct without padding.
    data.extend_from_slice(unsafe {
        std::slice::from_raw_parts(
            (&header as *const BITMAPINFOHEADER).cast::<u8>(),
            size_of::<BITMAPINFOHEADER>(),
        )
    });
    for row in image.pixels.chunks_exact(image.stride()).rev() {
        data.extend_from_slice(row);
    }
    data
}

/// Announces `format` on the open clipboard for delayed rendering.
///
/// `SetClipboardData` with no data returns NULL even on success, which the
/// `windows` crate turns into an error; only a set last-error code is a real
/// failure.
fn announce(format: u32) -> Result<()> {
    // SAFETY: called with the clipboard open by this thread; the last error
    // is cleared first so that a stale code is not taken for a failure.
    unsafe {
        SetLastError(WIN32_ERROR(0));
        match SetClipboardData(format, None) {
            Err(err) if err.code() != HRESULT(0) => Err(err),
            _ => Ok(()),
        }
    }
}

/// Hands `data` to the open clipboard in global memory, which the system
/// then owns.
fn set_data(format: u32, data: &[u8]) {
    // SAFETY: the global block is allocated with the data's size, filled
    // while locked, and freed here only if the clipboard did not take it.
    unsafe {
        let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, data.len()) else {
            return;
        };
        let target = GlobalLock(memory);
        if target.is_null() {
            let _ = GlobalFree(Some(memory));
            return;
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), target.cast(), data.len());
        let _ = GlobalUnlock(memory);
        if SetClipboardData(format, Some(HANDLE(memory.0))).is_err() {
            let _ = GlobalFree(Some(memory));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_rows_are_bottom_up() {
        let image = Image {
            width: 1,
            height: 2,
            pixels: vec![1, 2, 3, 255, 4, 5, 6, 255],
        };
        let data = dib(&image);
        let header = size_of::<BITMAPINFOHEADER>();
        assert_eq!(data.len(), header + 8);
        assert_eq!(&data[header..], &[4, 5, 6, 255, 1, 2, 3, 255]);
    }
}
