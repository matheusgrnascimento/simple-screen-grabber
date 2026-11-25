//! Direct3D 11, Direct2D and DirectWrite devices, created once at startup
//! and shared by the overlay and the export.

use windows::core::{Interface, Result};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1DeviceContext, ID2D1Factory1, D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
    D2D1_FACTORY_OPTIONS, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, DWRITE_FACTORY_TYPE_SHARED,
};
use windows::Win32::Graphics::Dxgi::{IDXGIDevice1, IDXGIFactory2};

use crate::fluent::Color;

pub struct Gfx {
    pub d3d: ID3D11Device,
    pub dxgi_factory: IDXGIFactory2,
    pub ctx: ID2D1DeviceContext,
    pub dwrite: IDWriteFactory,
}

impl Gfx {
    /// Creates the devices on WARP, the Direct3D software rasterizer of
    /// Windows.
    ///
    /// A hardware device costs far more than it gives here. Measured on a
    /// 3440x1440 screen with an NVIDIA GPU: creating it takes 240 ms and 52
    /// MB of private memory that the driver keeps even after the device is
    /// released, and uploading a capture takes 19 ms. WARP is created in 16
    /// ms with 2.6 MB, uploads in 2.3 ms and draws a full overlay frame in
    /// under 2 ms, with the same Direct2D output.
    pub fn new() -> Result<Gfx> {
        let d3d = create_d3d(D3D_DRIVER_TYPE_WARP)?;
        let dxgi: IDXGIDevice1 = d3d.cast()?;
        // SAFETY: COM calls on the objects created above.
        unsafe {
            // At most one frame queued: input-to-screen latency stays at one
            // frame.
            dxgi.SetMaximumFrameLatency(1)?;
            let adapter = dxgi.GetAdapter()?;
            let dxgi_factory: IDXGIFactory2 = adapter.GetParent()?;
            let options = D2D1_FACTORY_OPTIONS::default();
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&options))?;
            let device = factory.CreateDevice(&dxgi)?;
            let ctx = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            // Grayscale like Windows 11 controls: ClearType fringes would
            // show over translucent surfaces.
            ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Gfx {
                d3d,
                dxgi_factory,
                ctx,
                dwrite,
            })
        }
    }
}

fn create_d3d(driver: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut device = None;
    // SAFETY: creates a device with default feature levels; the result is
    // written to `device`.
    unsafe {
        D3D11CreateDevice(
            None,
            driver,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )?;
    }
    device.ok_or_else(windows::core::Error::empty)
}

/// A color for Direct2D.
pub fn color(c: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: f32::from(c.a) / 255.0,
    }
}

pub fn rect_f(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right,
        bottom,
    }
}
