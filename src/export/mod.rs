//! The output: the selected image, PNG files and the clipboard.

pub mod clipboard;
pub mod png;

use std::path::{Path, PathBuf};

use windows::core::{Error, Result, HRESULT};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH, MOVE_FILE_FLAGS};

use crate::capture::{Capture, Timestamp};
use crate::geometry::Rect;
use crate::paths;
use crate::win;

/// An opaque image: top-down rows of BGRA pixels with alpha 255.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub const fn stride(&self) -> usize {
        self.width as usize * 4
    }

    /// A copy of a rectangle of the capture, with the undefined fourth byte
    /// of each pixel set to opaque. The color bytes are copied unchanged.
    pub fn crop(capture: &Capture, rect: Rect) -> Image {
        let rect = rect.intersect(capture.bounds());
        let (width, height) = (rect.width().max(0) as u32, rect.height().max(0) as u32);
        let row_bytes = width as usize * 4;
        let mut pixels = Vec::with_capacity(row_bytes * height as usize);
        let source = capture.pixels();
        for y in rect.top..rect.bottom {
            let start = y as usize * capture.stride() + rect.left as usize * 4;
            pixels.extend_from_slice(&source[start..start + row_bytes]);
        }
        for alpha in pixels.iter_mut().skip(3).step_by(4) {
            *alpha = 0xFF;
        }
        Image {
            width,
            height,
            pixels,
        }
    }
}

/// Writes `image` as a PNG file in `folder`, named after `time`, without
/// overwriting any file: the PNG goes to a temporary file in the same folder
/// that is then renamed, so a partial file never appears under the final
/// name.
pub fn save(image: &Image, folder: &Path, time: Timestamp) -> Result<PathBuf> {
    std::fs::create_dir_all(folder).map_err(|e| io_error(&e))?;
    let first = folder.join(paths::file_name(time, 1));
    let temp = first.with_extension("png.tmp");
    let written = png::write_file(image, &temp);
    if let Err(err) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    let temp_wide = win::wide(&temp.to_string_lossy());
    for attempt in 1..=1000 {
        let target = folder.join(paths::file_name(time, attempt));
        let target_wide = win::wide(&target.to_string_lossy());
        // SAFETY: both paths are null-terminated and outlive the call.
        // Without MOVEFILE_REPLACE_EXISTING the rename fails if the name is
        // taken, so an existing file is never replaced.
        let moved = unsafe {
            MoveFileExW(
                win::pcwstr(&temp_wide),
                win::pcwstr(&target_wide),
                MOVE_FILE_FLAGS(MOVEFILE_WRITE_THROUGH.0),
            )
        };
        match moved {
            Ok(()) => return Ok(target),
            Err(err)
                if err.code() == HRESULT::from_win32(ERROR_ALREADY_EXISTS.0)
                    || err.code() == HRESULT::from_win32(ERROR_FILE_EXISTS.0) => {}
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                return Err(err);
            }
        }
    }
    let _ = std::fs::remove_file(&temp);
    Err(Error::new(
        HRESULT::from_win32(ERROR_ALREADY_EXISTS.0),
        "every file name for this second is taken",
    ))
}

fn io_error(err: &std::io::Error) -> Error {
    match err.raw_os_error() {
        Some(code) => Error::from_hresult(HRESULT::from_win32(code as u32)),
        None => Error::new(HRESULT(-1), err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Monitor;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    /// A synthetic capture whose pixel at (x, y) is (x, y, x ^ y, junk).
    fn capture(width: i32, height: i32) -> Capture {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[x as u8, y as u8, (x ^ y) as u8, 0x5A]);
            }
        }
        let monitor = Monitor {
            rect: Rect::new(0, 0, width, height),
            scale: 1.0,
        };
        Capture::synthetic(width, height, vec![monitor], pixels)
    }

    #[test]
    fn crop_copies_colors_exactly_and_makes_them_opaque() {
        let image = Image::crop(&capture(16, 8), Rect::new(3, 2, 7, 5));
        assert_eq!((image.width, image.height), (4, 3));
        let pixel = |x: usize, y: usize| &image.pixels[(y * 4 + x) * 4..][..4];
        assert_eq!(pixel(0, 0), &[3, 2, 3 ^ 2, 0xFF]);
        assert_eq!(pixel(3, 2), &[6, 4, 6 ^ 4, 0xFF]);
    }

    #[test]
    fn crop_is_clipped_to_the_capture() {
        let image = Image::crop(&capture(10, 10), Rect::new(8, 8, 20, 20));
        assert_eq!((image.width, image.height), (2, 2));
    }

    #[test]
    fn save_is_lossless_and_never_overwrites() {
        // SAFETY: COM for this test thread.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let folder =
            std::env::temp_dir().join(format!("simple-screen-grabber-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let image = Image::crop(&capture(64, 40), Rect::new(0, 0, 64, 40));
        let time = Timestamp {
            year: 2026,
            month: 11,
            day: 25,
            hour: 14,
            minute: 32,
            second: 10,
        };

        let first = save(&image, &folder, time).unwrap();
        let second = save(&image, &folder, time).unwrap();
        assert_eq!(first.file_name().unwrap(), "2026-11-25_14-32-10.png");
        assert_eq!(second.file_name().unwrap(), "2026-11-25_14-32-10_2.png");

        let decoded = png::decode_file(&first).unwrap();
        assert_eq!((decoded.width, decoded.height), (64, 40));
        assert_eq!(decoded.pixels, image.pixels);

        let leftovers: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
            .collect();
        assert!(leftovers.is_empty());
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
