//! The output: the selected image, PNG files and the clipboard.

pub mod clipboard;
pub mod png;

use std::path::{Path, PathBuf};

use windows::core::{Error, Result, HRESULT};
use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, ID2D1DeviceContext, D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
    D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_MAP_OPTIONS_READ,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH, MOVE_FILE_FLAGS};
use windows_numerics::Matrix3x2;

use crate::annotate::draw::ShapePainter;
use crate::annotate::Shape;
use crate::capture::{Capture, Timestamp};
use crate::geometry::Rect;
use crate::gfx;
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

fn bitmap_properties(options: D2D1_BITMAP_OPTIONS) -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: options,
        ..Default::default()
    }
}

/// Draws `area` of the screen with the annotations at native resolution,
/// with the same drawing code as the overlay, and reads it back. The screen
/// is drawn 1:1 with nearest-neighbor sampling, so pixels away from the
/// annotations keep their exact values.
pub fn render(
    ctx: &ID2D1DeviceContext,
    painter: &ShapePainter,
    screen: &ID2D1Bitmap1,
    shapes: &[Shape],
    area: Rect,
) -> Result<Image> {
    let (width, height) = (area.width() as u32, area.height() as u32);
    let size = D2D_SIZE_U { width, height };
    let base = Matrix3x2::translation(-(area.left as f32), -(area.top as f32));
    // SAFETY: one BeginDraw/EndDraw pair on this thread's context; the
    // mapped memory is read within its pitch and unmapped.
    unsafe {
        let target = ctx.CreateBitmap(
            size,
            None,
            0,
            &bitmap_properties(D2D1_BITMAP_OPTIONS_TARGET),
        )?;
        ctx.SetTarget(&target);
        ctx.BeginDraw();
        ctx.SetTransform(&base);
        ctx.DrawBitmap(
            screen,
            None,
            1.0,
            D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            None,
            None,
        );
        painter.draw(
            ctx,
            shapes,
            gfx::rect_f(
                area.left as f32,
                area.top as f32,
                area.right as f32,
                area.bottom as f32,
            ),
            &base,
        );
        let ended = ctx.EndDraw(None, None);
        ctx.SetTarget(None);
        ended?;
        read_back(ctx, &target, size)
    }
}

/// Copies a drawn bitmap to memory as an opaque image.
pub fn read_back(
    ctx: &ID2D1DeviceContext,
    bitmap: &ID2D1Bitmap1,
    size: D2D_SIZE_U,
) -> Result<Image> {
    // SAFETY: the mapped memory is read within its pitch and unmapped.
    unsafe {
        let readable = ctx.CreateBitmap(
            size,
            None,
            0,
            &bitmap_properties(D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW),
        )?;
        readable.CopyFromBitmap(None, bitmap, None)?;
        let mapped = readable.Map(D2D1_MAP_OPTIONS_READ)?;
        let row = size.width as usize * 4;
        let mut pixels = Vec::with_capacity(row * size.height as usize);
        for y in 0..size.height as usize {
            let line = std::slice::from_raw_parts(mapped.bits.add(y * mapped.pitch as usize), row);
            pixels.extend_from_slice(line);
        }
        readable.Unmap()?;
        for alpha in pixels.iter_mut().skip(3).step_by(4) {
            *alpha = 0xFF;
        }
        Ok(Image {
            width: size.width,
            height: size.height,
            pixels,
        })
    }
}

/// Properties of a drawable bitmap in the screen's pixel format.
#[cfg(test)]
pub fn target_properties() -> D2D1_BITMAP_PROPERTIES1 {
    bitmap_properties(D2D1_BITMAP_OPTIONS_TARGET)
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

    /// Uploads a capture the way the overlay does.
    fn upload(gfx: &gfx::Gfx, capture: &Capture) -> ID2D1Bitmap1 {
        // SAFETY: the pixels outlive the call, which copies them.
        unsafe {
            gfx.ctx
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: capture.width as u32,
                        height: capture.height as u32,
                    },
                    Some(capture.pixels().as_ptr().cast()),
                    capture.stride() as u32,
                    &bitmap_properties(
                        windows::Win32::Graphics::Direct2D::D2D1_BITMAP_OPTIONS_NONE,
                    ),
                )
                .unwrap()
        }
    }

    #[test]
    fn rendering_without_annotations_keeps_every_pixel() {
        let gfx = gfx::Gfx::new().unwrap();
        let painter = ShapePainter::new(&gfx.ctx).unwrap();
        let capture = capture(97, 61);
        let screen = upload(&gfx, &capture);
        let area = Rect::new(13, 7, 90, 55);
        let rendered = render(&gfx.ctx, &painter, &screen, &[], area).unwrap();
        assert_eq!(rendered.pixels, Image::crop(&capture, area).pixels);
    }

    #[test]
    fn annotations_change_only_the_pixels_they_cover() {
        use crate::annotate::{PointF, Tool, COLORS};
        let gfx = gfx::Gfx::new().unwrap();
        let painter = ShapePainter::new(&gfx.ctx).unwrap();
        let capture = capture(120, 80);
        let screen = upload(&gfx, &capture);
        let mut line = Shape::new(Tool::Line, COLORS[0].0, 3.0, PointF::center(10, 40));
        line.extend(PointF::center(100, 40), false);
        let area = Rect::new(0, 0, 120, 80);
        let rendered = render(&gfx.ctx, &painter, &screen, &[line], area).unwrap();
        let original = Image::crop(&capture, area);
        let at =
            |image: &Image, x: usize, y: usize| image.pixels[(y * 120 + x) * 4..][..4].to_vec();
        // On the line: red.
        assert_eq!(at(&rendered, 50, 40), vec![0x4B, 0x4B, 0xFF, 0xFF]);
        // Far from it: unchanged.
        assert_eq!(at(&rendered, 50, 10), at(&original, 50, 10));
        assert_eq!(at(&rendered, 5, 70), at(&original, 5, 70));
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
