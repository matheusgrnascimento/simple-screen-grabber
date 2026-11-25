//! PNG encoding with the Windows Imaging Component.

use std::path::Path;

use windows::core::{Error, Result};
use windows::Win32::Foundation::GENERIC_WRITE;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat24bppBGR,
    IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::{
    CoCreateInstance, IStream, CLSCTX_INPROC_SERVER, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET,
};
use windows::Win32::UI::Shell::SHCreateMemStream;

use super::Image;
use crate::win;

fn factory() -> Result<IWICImagingFactory> {
    // SAFETY: creates the in-process WIC factory; COM is initialized on
    // every thread that encodes.
    unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }
}

/// Encodes `image` into `stream` as an 8-bit RGB PNG (the image is opaque,
/// so no alpha channel is stored). Lossless: every pixel is kept as is.
fn encode(factory: &IWICImagingFactory, image: &Image, stream: &IStream) -> Result<()> {
    let row = image.width as usize * 3;
    let mut rgb = vec![0u8; row * image.height as usize];
    for (out, pixel) in rgb
        .as_chunks_mut::<3>()
        .0
        .iter_mut()
        .zip(image.pixels.as_chunks::<4>().0)
    {
        *out = [pixel[0], pixel[1], pixel[2]];
    }
    // SAFETY: plain COM calls; `rgb` holds height rows of `row` bytes and
    // outlives WritePixels.
    unsafe {
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(Error::empty)?;
        frame.Initialize(None)?;
        frame.SetSize(image.width, image.height)?;
        let mut format = GUID_WICPixelFormat24bppBGR;
        frame.SetPixelFormat(&mut format)?;
        if format != GUID_WICPixelFormat24bppBGR {
            return Err(Error::empty());
        }
        frame.WritePixels(image.height, row as u32, &rgb)?;
        frame.Commit()?;
        encoder.Commit()
    }
}

/// Writes `image` as a PNG file at `path`.
pub fn write_file(image: &Image, path: &Path) -> Result<()> {
    let factory = factory()?;
    let wide = win::wide(&path.to_string_lossy());
    // SAFETY: the path is null-terminated and outlives the call.
    let stream = unsafe {
        let stream = factory.CreateStream()?;
        stream.InitializeFromFilename(win::pcwstr(&wide), GENERIC_WRITE.0)?;
        stream
    };
    encode(&factory, image, &stream.into())
}

/// Decodes a PNG file into top-down BGRA rows, for tests.
#[cfg(test)]
pub fn decode_file(path: &Path) -> Result<Image> {
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::{
        GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
        WICDecodeMetadataCacheOnDemand,
    };
    let factory = factory()?;
    let wide = win::wide(&path.to_string_lossy());
    // SAFETY: plain COM calls; the output buffer holds the whole image.
    unsafe {
        let decoder = factory.CreateDecoderFromFilename(
            win::pcwstr(&wide),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let (mut width, mut height) = (0, 0);
        converter.GetSize(&mut width, &mut height)?;
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut pixels)?;
        Ok(Image {
            width,
            height,
            pixels,
        })
    }
}

/// Encodes `image` as PNG bytes in memory.
pub fn to_bytes(image: &Image) -> Result<Vec<u8>> {
    let factory = factory()?;
    // SAFETY: plain COM calls on a memory stream owned here.
    unsafe {
        let stream = SHCreateMemStream(None).ok_or_else(Error::from_thread)?;
        encode(&factory, image, &stream)?;
        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        let mut data = vec![0u8; stat.cbSize as usize];
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut read = 0u32;
        stream
            .Read(data.as_mut_ptr().cast(), data.len() as u32, Some(&mut read))
            .ok()?;
        data.truncate(read as usize);
        Ok(data)
    }
}
