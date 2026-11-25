//! Generates `assets/icon.ico` and `assets/icon.png`.
//!
//! The icon follows the family of Simple RTSP Viewer and Simple Clip
//! Trimmer: a blue rounded square with a vertical gradient, a white main
//! symbol and one accent color. Here the symbol is a screen selection: a
//! white dashed rectangle with red handles on its corners.
//!
//! Each size is rendered directly with 8x8 supersampling. Sizes of 24 px and
//! below use a simpler, pixel-aligned variant (a solid border and slightly
//! larger handles), so the tray icon stays sharp.
//!
//! Usage, from the repository root:
//!
//! ```text
//! cargo run -p genicon
//! ```

use std::path::Path;

use windows::core::Result;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STATFLAG_NONAME,
    STATSTG, STREAM_SEEK_SET,
};
use windows::Win32::UI::Shell::SHCreateMemStream;

const SIZES: [u32; 10] = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256];
const SAMPLES: u32 = 8;

type Rgb = [f64; 3];

const TOP: Rgb = [43.0, 136.0, 216.0]; // #2B88D8
const BOTTOM: Rgb = [0.0, 90.0, 166.0]; // #005AA6
const WHITE: Rgb = [255.0, 255.0, 255.0];
const RED: Rgb = [255.0, 75.0, 75.0]; // #FF4B4B

/// An axis-aligned rectangle with rounded corners, in pixels.
#[derive(Clone, Copy)]
struct Shape {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    radius: f64,
}

impl Shape {
    fn contains(&self, x: f64, y: f64) -> bool {
        if x < self.x0 || x > self.x1 || y < self.y0 || y > self.y1 {
            return false;
        }
        let r = self.radius;
        let cx = x.clamp(self.x0 + r, self.x1 - r);
        let cy = y.clamp(self.y0 + r, self.y1 - r);
        (x - cx) * (x - cx) + (y - cy) * (y - cy) <= r * r
    }
}

/// The shapes of one icon size, painted in order: background, border, handles.
struct Icon {
    background: Shape,
    border: Vec<Shape>,
    handles: Vec<Shape>,
}

/// Selection rectangle in unit coordinates, from the approved sketch.
const SEL_X0: f64 = 0.24;
const SEL_X1: f64 = 0.76;
const SEL_Y0: f64 = 0.28;
const SEL_Y1: f64 = 0.72;

fn icon(size: u32) -> Icon {
    let s = f64::from(size);
    let background = Shape {
        x0: 0.03 * s,
        y0: 0.03 * s,
        x1: 0.97 * s,
        y1: 0.97 * s,
        radius: 0.22 * s,
    };
    if size <= 24 {
        small_icon(s, background)
    } else {
        large_icon(s, background)
    }
}

/// Dashed border with handles of 0.12 centered on the rectangle corners.
fn large_icon(s: f64, background: Shape) -> Icon {
    let stroke = 0.05 * s;
    let handle = 0.12 * s;
    let (x0, x1, y0, y1) = (SEL_X0 * s, SEL_X1 * s, SEL_Y0 * s, SEL_Y1 * s);
    let half = stroke / 2.0;
    let mut border = Vec::new();
    // Each edge gets two dashes between the handles, with the gaps two
    // thirds of a dash, so every edge looks balanced.
    let dashes = |from: f64, to: f64| -> [(f64, f64); 2] {
        let length = to - from;
        let dash = length / 4.0;
        let gap = dash * 2.0 / 3.0;
        [
            (from + gap, from + gap + dash),
            (from + 2.0 * gap + dash, from + 2.0 * gap + 2.0 * dash),
        ]
    };
    for (a, b) in dashes(x0 + handle / 2.0, x1 - handle / 2.0) {
        for y in [y0, y1] {
            border.push(Shape {
                x0: a,
                y0: y - half,
                x1: b,
                y1: y + half,
                radius: 0.0,
            });
        }
    }
    for (a, b) in dashes(y0 + handle / 2.0, y1 - handle / 2.0) {
        for x in [x0, x1] {
            border.push(Shape {
                x0: x - half,
                y0: a,
                x1: x + half,
                y1: b,
                radius: 0.0,
            });
        }
    }
    let handles = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)]
        .iter()
        .map(|&(cx, cy)| Shape {
            x0: cx - handle / 2.0,
            y0: cy - handle / 2.0,
            x1: cx + handle / 2.0,
            y1: cy + handle / 2.0,
            radius: 0.02 * s,
        })
        .collect();
    Icon {
        background,
        border,
        handles,
    }
}

/// Solid, pixel-aligned border and larger square handles.
fn small_icon(s: f64, background: Shape) -> Icon {
    let stroke = (0.07 * s).round().max(1.0);
    let handle = (0.2 * s).round().max(3.0);
    let x0 = (SEL_X0 * s).round();
    let x1 = (SEL_X1 * s).round();
    let y0 = (SEL_Y0 * s).round();
    let y1 = (SEL_Y1 * s).round();
    let line = |x0: f64, y0: f64, x1: f64, y1: f64| Shape {
        x0,
        y0,
        x1,
        y1,
        radius: 0.0,
    };
    let border = vec![
        line(x0, y0, x1, y0 + stroke),
        line(x0, y1 - stroke, x1, y1),
        line(x0, y0, x0 + stroke, y1),
        line(x1 - stroke, y0, x1, y1),
    ];
    // Center each handle on the middle of the corner's stroke, snapped to
    // whole pixels.
    let snap = |center: f64| (center - handle / 2.0).round();
    let handles = [
        (x0 + stroke / 2.0, y0 + stroke / 2.0),
        (x1 - stroke / 2.0, y0 + stroke / 2.0),
        (x0 + stroke / 2.0, y1 - stroke / 2.0),
        (x1 - stroke / 2.0, y1 - stroke / 2.0),
    ]
    .iter()
    .map(|&(cx, cy)| {
        let (hx, hy) = (snap(cx), snap(cy));
        line(hx, hy, hx + handle, hy + handle)
    })
    .collect();
    Icon {
        background,
        border,
        handles,
    }
}

/// The color of a point in pixels, or `None` outside the icon.
fn shade(icon: &Icon, x: f64, y: f64) -> Option<Rgb> {
    if !icon.background.contains(x, y) {
        return None;
    }
    if icon.handles.iter().any(|h| h.contains(x, y)) {
        return Some(RED);
    }
    if icon.border.iter().any(|b| b.contains(x, y)) {
        return Some(WHITE);
    }
    let b = icon.background;
    let t = ((y - b.y0) / (b.y1 - b.y0)).clamp(0.0, 1.0);
    Some([
        TOP[0] + (BOTTOM[0] - TOP[0]) * t,
        TOP[1] + (BOTTOM[1] - TOP[1]) * t,
        TOP[2] + (BOTTOM[2] - TOP[2]) * t,
    ])
}

/// Renders one size as straight (not premultiplied) BGRA pixels.
fn render(size: u32) -> Vec<u8> {
    let icon = icon(size);
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let mut sum = [0.0; 3];
            let mut hits = 0u32;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = f64::from(px) + (f64::from(sx) + 0.5) / f64::from(SAMPLES);
                    let y = f64::from(py) + (f64::from(sy) + 0.5) / f64::from(SAMPLES);
                    if let Some(c) = shade(&icon, x, y) {
                        for i in 0..3 {
                            sum[i] += c[i];
                        }
                        hits += 1;
                    }
                }
            }
            if hits == 0 {
                continue;
            }
            let n = f64::from(hits);
            let at = ((py * size + px) * 4) as usize;
            let channel = |v: f64| (v / n).round().clamp(0.0, 255.0) as u8;
            pixels[at] = channel(sum[2]);
            pixels[at + 1] = channel(sum[1]);
            pixels[at + 2] = channel(sum[0]);
            pixels[at + 3] = (255.0 * n / f64::from(SAMPLES * SAMPLES)).round() as u8;
        }
    }
    pixels
}

/// Encodes BGRA pixels as PNG with the Windows Imaging Component.
fn encode_png(factory: &IWICImagingFactory, size: u32, pixels: &[u8]) -> Result<Vec<u8>> {
    // SAFETY: plain COM calls on objects created here; the pixel slice
    // outlives the WritePixels call and holds size * size * 4 bytes.
    unsafe {
        let stream = SHCreateMemStream(None).ok_or_else(windows::core::Error::from_thread)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(windows::core::Error::empty)?;
        frame.Initialize(None)?;
        frame.SetSize(size, size)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WritePixels(size, size * 4, pixels)?;
        frame.Commit()?;
        encoder.Commit()?;

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

/// Builds an .ico file holding PNG-compressed images.
fn ico(entries: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * entries.len();
    for (size, data) in entries {
        // 0 means 256.
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for (_, data) in entries {
        out.extend_from_slice(data);
    }
    out
}

fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // SAFETY: initializes COM for this thread before any COM call.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok()? };
    // SAFETY: creates the in-process WIC factory.
    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };

    let mut entries = Vec::new();
    for size in SIZES {
        let png = encode_png(&factory, size, &render(size))?;
        entries.push((size, png));
    }

    let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    std::fs::create_dir_all(&assets)?;
    std::fs::write(assets.join("icon.ico"), ico(&entries))?;
    if let Some((_, largest)) = entries.last() {
        std::fs::write(assets.join("icon.png"), largest)?;
    }
    println!("wrote assets/icon.ico and assets/icon.png");
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("genicon: {err}");
        std::process::exit(1);
    }
}
