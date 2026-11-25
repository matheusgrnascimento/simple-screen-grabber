//! The setup window: one page in the Windows 11 style. Mica backdrop (on
//! Windows 11; a solid background on Windows 10), the app icon, the choices
//! as Fluent checkboxes, an accent button, a progress bar, and the light or
//! dark theme and accent color of Windows. Drawn with Direct2D in DIPs.

use std::cell::RefCell;

use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Bitmap, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_HWND_RENDER_TARGET_PROPERTIES,
    D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES, D2D1_ROUNDED_RECT,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_WORD_WRAPPING_WRAP,
};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{InvalidateRect, ValidateRect};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{
    RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD,
};
use windows::Win32::UI::Controls::{MARGINS, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, LoadCursorW, LoadImageW, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetCursor, SetWindowPos, ShowWindow, TranslateMessage, CW_USEDEFAULT, HICON,
    HTCLIENT, IDC_ARROW, IDC_HAND, IMAGE_ICON, LR_DEFAULTCOLOR, MB_ICONINFORMATION, MB_OK, MSG,
    SM_CXSCREEN, SM_CYSCREEN, SWP_NOACTIVATE, SWP_NOZORDER, SW_SHOWNORMAL, WINDOW_EX_STYLE, WM_APP,
    WM_CLOSE, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_SIZE, WNDCLASSEXW, WS_CAPTION, WS_MINIMIZEBOX,
    WS_OVERLAPPED, WS_SYSMENU,
};

use crate::install::{self, Options, PostTarget, APP_NAME};
use crate::Mode;

/// Client size in DIPs.
const WIDTH: f32 = 480.0;
const HEIGHT: f32 = 330.0;
const PAD: f32 = 32.0;
const FOOTER: f32 = 80.0;

const WM_APP_PROGRESS: u32 = WM_APP + 1;
const WM_APP_FINISHED: u32 = WM_APP + 2;

const LICENSE: &str = include_str!("../../../LICENSE");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Primary,
    Secondary,
    First,
    Second,
    License,
}

enum Phase {
    Ready,
    Working { fraction: f32, status: String },
    Done,
    Failed(String),
}

#[derive(Clone, Copy)]
struct Rgba(u8, u8, u8, u8);

impl Rgba {
    fn d2d(self) -> D2D1_COLOR_F {
        D2D1_COLOR_F {
            r: f32::from(self.0) / 255.0,
            g: f32::from(self.1) / 255.0,
            b: f32::from(self.2) / 255.0,
            a: f32::from(self.3) / 255.0,
        }
    }
}

/// Windows 11 colors for the theme.
struct Colors {
    background: Rgba,
    footer: Rgba,
    divider: Rgba,
    text: Rgba,
    secondary: Rgba,
    accent: Rgba,
    accent_hover: Rgba,
    accent_pressed: Rgba,
    on_accent: Rgba,
    control: Rgba,
    control_hover: Rgba,
    control_pressed: Rgba,
    control_stroke: Rgba,
    check_stroke: Rgba,
    track: Rgba,
}

fn registry_dword(key: PCWSTR, value: PCWSTR) -> Option<u32> {
    let mut data = 0u32;
    let mut size = 4u32;
    // SAFETY: a 4-byte buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key,
            value,
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(data)
}

fn accent_shade(index: usize) -> Option<Rgba> {
    let mut data = [0u8; 32];
    let mut size = 32u32;
    // SAFETY: a 32-byte buffer.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"),
            w!("AccentPalette"),
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    (status.is_ok() && size >= 32).then(|| {
        Rgba(
            data[index * 4],
            data[index * 4 + 1],
            data[index * 4 + 2],
            0xFF,
        )
    })
}

impl Colors {
    fn load(dark: bool) -> Colors {
        if dark {
            let accent = accent_shade(1).unwrap_or(Rgba(0x60, 0xCD, 0xFF, 0xFF));
            Colors {
                background: Rgba(0x20, 0x20, 0x20, 0xFF),
                footer: Rgba(0x3A, 0x3A, 0x3A, 0x4D),
                divider: Rgba(0x00, 0x00, 0x00, 0x33),
                text: Rgba(0xFF, 0xFF, 0xFF, 0xFF),
                secondary: Rgba(0xC5, 0xC5, 0xC5, 0xFF),
                accent,
                accent_hover: Rgba(accent.0, accent.1, accent.2, 0xE6),
                accent_pressed: Rgba(accent.0, accent.1, accent.2, 0xCC),
                on_accent: Rgba(0, 0, 0, 0xFF),
                control: Rgba(0xFF, 0xFF, 0xFF, 0x0F),
                control_hover: Rgba(0xFF, 0xFF, 0xFF, 0x15),
                control_pressed: Rgba(0xFF, 0xFF, 0xFF, 0x08),
                control_stroke: Rgba(0xFF, 0xFF, 0xFF, 0x12),
                check_stroke: Rgba(0xFF, 0xFF, 0xFF, 0x8B),
                track: Rgba(0xFF, 0xFF, 0xFF, 0x28),
            }
        } else {
            let accent = accent_shade(4).unwrap_or(Rgba(0x00, 0x5F, 0xB8, 0xFF));
            Colors {
                background: Rgba(0xF3, 0xF3, 0xF3, 0xFF),
                footer: Rgba(0xFF, 0xFF, 0xFF, 0x80),
                divider: Rgba(0x00, 0x00, 0x00, 0x0F),
                text: Rgba(0x1B, 0x1B, 0x1B, 0xFF),
                secondary: Rgba(0x5F, 0x5F, 0x5F, 0xFF),
                accent,
                accent_hover: Rgba(accent.0, accent.1, accent.2, 0xE6),
                accent_pressed: Rgba(accent.0, accent.1, accent.2, 0xCC),
                on_accent: Rgba(0xFF, 0xFF, 0xFF, 0xFF),
                control: Rgba(0xFF, 0xFF, 0xFF, 0xB3),
                control_hover: Rgba(0xF9, 0xF9, 0xF9, 0x80),
                control_pressed: Rgba(0xF9, 0xF9, 0xF9, 0x4D),
                control_stroke: Rgba(0x00, 0x00, 0x00, 0x14),
                check_stroke: Rgba(0x00, 0x00, 0x00, 0x72),
                track: Rgba(0x00, 0x00, 0x00, 0x28),
            }
        }
    }
}

struct Ui {
    hwnd: HWND,
    mode: Mode,
    payload: &'static [u8],
    version: &'static str,
    installed: Option<String>,
    phase: Phase,
    /// Install: start with Windows. Uninstall: keep the settings.
    first: bool,
    /// Install: open when done.
    second: bool,
    hover: Option<Control>,
    pressed: Option<Control>,
    dark: bool,
    mica: bool,
    colors: Colors,
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    target: Option<(
        ID2D1HwndRenderTarget,
        ID2D1SolidColorBrush,
        Option<ID2D1Bitmap>,
    )>,
    title: IDWriteTextFormat,
    body: IDWriteTextFormat,
    caption: IDWriteTextFormat,
    button: IDWriteTextFormat,
    glyph: IDWriteTextFormat,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|cell| cell.try_borrow_mut().ok()?.as_mut().map(f))
}

fn rect(left: f32, top: f32, width: f32, height: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn contains(r: &D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn font_exists(dwrite: &IDWriteFactory, family: PCWSTR) -> bool {
    let mut collection = None;
    // SAFETY: plain queries.
    unsafe {
        if dwrite
            .GetSystemFontCollection(&mut collection, false)
            .is_err()
        {
            return false;
        }
        let (mut index, mut found) = (0, BOOL(0));
        collection.is_some_and(|c| {
            c.FindFamilyName(family, &mut index, &mut found).is_ok() && found.as_bool()
        })
    }
}

fn format(
    dwrite: &IDWriteFactory,
    family: PCWSTR,
    weight: DWRITE_FONT_WEIGHT,
    size: f32,
) -> Result<IDWriteTextFormat> {
    // SAFETY: constant arguments.
    unsafe {
        dwrite.CreateTextFormat(
            family,
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!("en-us"),
        )
    }
}

impl Ui {
    fn primary_label(&self) -> &'static str {
        match (&self.phase, self.mode) {
            (Phase::Done | Phase::Failed(_), _) => "Close",
            (_, Mode::Uninstall) => "Uninstall",
            (_, Mode::Install) if self.installed.is_some() => "Update",
            _ => "Install",
        }
    }

    /// The layout of the controls, in DIPs.
    fn control_rect(&self, control: Control) -> Option<D2D_RECT_F> {
        let footer_top = HEIGHT - FOOTER;
        let button_y = footer_top + (FOOTER - 32.0) / 2.0;
        let ready = matches!(self.phase, Phase::Ready);
        match control {
            Control::Primary => Some(rect(WIDTH - PAD - 120.0, button_y, 120.0, 32.0)),
            Control::Secondary if ready => Some(rect(WIDTH - PAD - 248.0, button_y, 120.0, 32.0)),
            Control::First if ready => Some(rect(PAD, 156.0, WIDTH - 2.0 * PAD, 32.0)),
            Control::Second if ready && self.mode == Mode::Install => {
                Some(rect(PAD, 188.0, WIDTH - 2.0 * PAD, 32.0))
            }
            Control::License if ready && self.mode == Mode::Install => {
                Some(rect(PAD, 224.0, 260.0, 20.0))
            }
            _ => None,
        }
    }

    fn hit(&self, x: f32, y: f32) -> Option<Control> {
        [
            Control::Primary,
            Control::Secondary,
            Control::First,
            Control::Second,
            Control::License,
        ]
        .into_iter()
        .find(|c| self.control_rect(*c).is_some_and(|r| contains(&r, x, y)))
    }

    fn scale(&self) -> f32 {
        // SAFETY: plain query.
        unsafe { GetDpiForWindow(self.hwnd) as f32 / 96.0 }
    }

    fn invalidate(&self) {
        // SAFETY: this thread's window.
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn activate(&mut self, control: Control) {
        match control {
            Control::Primary => match self.phase {
                Phase::Ready => self.start(),
                Phase::Done | Phase::Failed(_) => unsafe { PostQuitMessage(0) },
                Phase::Working { .. } => {}
            },
            Control::Secondary => unsafe { PostQuitMessage(1) },
            Control::First => self.first = !self.first,
            Control::Second => self.second = !self.second,
            Control::License => {
                let text: Vec<u16> = LICENSE.encode_utf16().chain([0]).collect();
                // SAFETY: a modal message box; nothing is borrowed across it
                // that a nested message would need (it is skipped).
                unsafe {
                    MessageBoxW(
                        Some(self.hwnd),
                        PCWSTR(text.as_ptr()),
                        w!("MIT License"),
                        MB_OK | MB_ICONINFORMATION,
                    )
                };
            }
        }
        self.invalidate();
    }

    /// Runs the install or uninstall on a worker thread.
    fn start(&mut self) {
        self.phase = Phase::Working {
            fraction: 0.0,
            status: "Starting".into(),
        };
        let target = PostTarget(self.hwnd);
        let (mode, payload, version, first) = (self.mode, self.payload, self.version, self.first);
        std::thread::spawn(move || {
            let target = target;
            // SAFETY: COM for this thread.
            let _ = unsafe {
                windows::Win32::System::Com::CoInitializeEx(
                    None,
                    windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
                )
            };
            let mut progress = |fraction: f32, status: &str| {
                let boxed = Box::into_raw(Box::new((fraction, status.to_string())));
                // SAFETY: the window takes ownership of the box.
                unsafe {
                    if PostMessageW(
                        Some(target.0),
                        WM_APP_PROGRESS,
                        WPARAM(0),
                        LPARAM(boxed as isize),
                    )
                    .is_err()
                    {
                        drop(Box::from_raw(boxed));
                    }
                }
            };
            let result = match mode {
                Mode::Install => install::install(
                    payload,
                    version,
                    Options { autostart: first },
                    &mut progress,
                )
                .map(Some),
                Mode::Uninstall => install::uninstall(first, &mut progress).map(|()| None),
            };
            let boxed = Box::into_raw(Box::new(result.map_err(|e| e.message())));
            // SAFETY: as above.
            unsafe {
                if PostMessageW(
                    Some(target.0),
                    WM_APP_FINISHED,
                    WPARAM(0),
                    LPARAM(boxed as isize),
                )
                .is_err()
                {
                    drop(Box::from_raw(boxed));
                }
            }
        });
    }

    fn render(&mut self) -> Result<()> {
        let scale = self.scale();
        if self.target.is_none() {
            let mut client = RECT::default();
            // SAFETY: plain query; the target and its resources are owned here.
            unsafe {
                GetClientRect(self.hwnd, &mut client)?;
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT {
                        format: DXGI_FORMAT_B8G8R8A8_UNORM,
                        alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                    },
                    ..Default::default()
                };
                let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd: self.hwnd,
                    pixelSize: D2D_SIZE_U {
                        width: client.right as u32,
                        height: client.bottom as u32,
                    },
                    presentOptions: D2D1_PRESENT_OPTIONS_NONE,
                };
                let target = self.factory.CreateHwndRenderTarget(&props, &hwnd_props)?;
                target.SetDpi(96.0 * scale, 96.0 * scale);
                target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                let brush = target.CreateSolidColorBrush(&self.colors.text.d2d(), None)?;
                let icon = icon_bitmap(&target, (48.0 * scale) as i32).ok();
                self.target = Some((target, brush, icon));
            }
        }
        let Some((target, brush, icon)) = &self.target else {
            return Ok(());
        };
        let c = &self.colors;
        let fill = |r: D2D_RECT_F, color: Rgba, radius: f32| {
            // SAFETY: drawing with resources of this target.
            unsafe {
                brush.SetColor(&color.d2d());
                target.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: r,
                        radiusX: radius,
                        radiusY: radius,
                    },
                    brush,
                );
            }
        };
        let text = |s: &str, f: &IDWriteTextFormat, r: D2D_RECT_F, color: Rgba| {
            let wide: Vec<u16> = s.encode_utf16().collect();
            // SAFETY: drawing with resources of this target.
            unsafe {
                brush.SetColor(&color.d2d());
                target.DrawText(
                    &wide,
                    f,
                    &r,
                    brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
        };
        // SAFETY: one BeginDraw/EndDraw pair on this thread's target.
        unsafe {
            target.BeginDraw();
            let background = if self.mica {
                Rgba(0, 0, 0, 0)
            } else {
                c.background
            };
            target.Clear(Some(&background.d2d()));
            if let Some(icon) = icon {
                target.DrawBitmap(
                    icon,
                    Some(&rect(PAD, PAD, 48.0, 48.0)),
                    1.0,
                    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                    None,
                );
            }
        }
        let heading = match self.mode {
            Mode::Install => APP_NAME.to_string(),
            Mode::Uninstall => format!("Uninstall {APP_NAME}"),
        };
        text(
            &heading,
            &self.title,
            rect(PAD + 64.0, PAD + 2.0, 360.0, 30.0),
            c.text,
        );
        let sub = match (&self.installed, self.mode) {
            (Some(old), Mode::Install) if old != self.version => {
                format!("Version {old} \u{2192} {}", self.version)
            }
            _ => format!("Version {}", self.version),
        };
        text(
            &sub,
            &self.caption,
            rect(PAD + 64.0, PAD + 30.0, 360.0, 20.0),
            c.secondary,
        );

        let message = match (&self.phase, self.mode) {
            (Phase::Ready, Mode::Install) if self.installed.is_some() => {
                "Updates the app in place. Your settings and screenshots stay.".to_string()
            }
            (Phase::Ready, Mode::Install) => {
                "Select an area of the screen, annotate it, then save or copy it. Installs for your account only, with no administrator rights.".to_string()
            }
            (Phase::Ready, Mode::Uninstall) => {
                "Removes the app, its Start menu shortcut and Start with Windows. Your screenshots stay.".to_string()
            }
            (Phase::Working { status, .. }, _) => format!("{status}\u{2026}"),
            (Phase::Done, Mode::Install) => "Installed. Press Ctrl+Alt+S anytime to take a screenshot; the app waits in the notification area.".to_string(),
            (Phase::Done, Mode::Uninstall) => "Simple Screen Grabber was removed.".to_string(),
            (Phase::Failed(err), _) => format!("Something went wrong: {err}"),
        };
        text(
            &message,
            &self.body,
            rect(PAD, 100.0, WIDTH - 2.0 * PAD, 48.0),
            c.text,
        );

        // Checkboxes, the license link, or the progress bar.
        match &self.phase {
            Phase::Ready => {
                let boxes: &[(Control, bool, &str)] = match self.mode {
                    Mode::Install => &[
                        (Control::First, self.first, "Start with Windows"),
                        (
                            Control::Second,
                            self.second,
                            "Open Simple Screen Grabber when done",
                        ),
                    ],
                    Mode::Uninstall => &[(Control::First, self.first, "Keep my settings")],
                };
                for &(control, checked, label) in boxes {
                    let Some(r) = self.control_rect(control) else {
                        continue;
                    };
                    let box_rect = rect(r.left, r.top + 6.0, 20.0, 20.0);
                    let hovered = self.hover == Some(control);
                    if checked {
                        fill(
                            box_rect,
                            if hovered { c.accent_hover } else { c.accent },
                            4.0,
                        );
                        text("\u{E73E}", &self.glyph, box_rect, c.on_accent);
                    } else {
                        fill(box_rect, c.check_stroke, 4.0);
                        let inner = rect(box_rect.left + 1.0, box_rect.top + 1.0, 18.0, 18.0);
                        fill(inner, c.background, 3.0);
                        if hovered {
                            fill(inner, c.control_hover, 3.0);
                        }
                    }
                    text(
                        label,
                        &self.body,
                        rect(r.left + 30.0, r.top + 6.0, r.right - r.left - 30.0, 20.0),
                        c.text,
                    );
                }
                if let Some(r) = self.control_rect(Control::License) {
                    text(
                        "By installing, you accept the",
                        &self.caption,
                        rect(r.left, r.top, 170.0, 20.0),
                        c.secondary,
                    );
                    let link = if self.hover == Some(Control::License) {
                        c.accent_pressed
                    } else {
                        c.accent
                    };
                    text(
                        "MIT License",
                        &self.caption,
                        rect(r.left + 162.0, r.top, 100.0, 20.0),
                        link,
                    );
                }
            }
            Phase::Working { fraction, .. } => {
                let track = rect(PAD, 168.0, WIDTH - 2.0 * PAD, 4.0);
                fill(
                    rect(track.left, track.top + 1.0, track.right - track.left, 2.0),
                    c.track,
                    1.0,
                );
                fill(
                    rect(
                        track.left,
                        track.top,
                        (track.right - track.left) * fraction.clamp(0.02, 1.0),
                        4.0,
                    ),
                    c.accent,
                    2.0,
                );
            }
            Phase::Done | Phase::Failed(_) => {}
        }

        // Footer with the buttons.
        let footer = rect(0.0, HEIGHT - FOOTER, WIDTH, FOOTER);
        // SAFETY: drawing with resources of this target.
        unsafe {
            brush.SetColor(&c.footer.d2d());
            target.FillRectangle(&footer, brush);
            brush.SetColor(&c.divider.d2d());
            target.FillRectangle(&rect(0.0, HEIGHT - FOOTER, WIDTH, 1.0), brush);
        }
        let working = matches!(self.phase, Phase::Working { .. });
        if let Some(r) = self.control_rect(Control::Primary) {
            let color = if working {
                c.track
            } else if self.pressed == Some(Control::Primary) {
                c.accent_pressed
            } else if self.hover == Some(Control::Primary) {
                c.accent_hover
            } else {
                c.accent
            };
            fill(r, color, 4.0);
            text(
                self.primary_label(),
                &self.button,
                r,
                if working { c.secondary } else { c.on_accent },
            );
        }
        if let Some(r) = self.control_rect(Control::Secondary) {
            fill(r, c.control_stroke, 4.0);
            let inner = rect(r.left + 1.0, r.top + 1.0, 118.0, 30.0);
            let color = match (self.pressed, self.hover) {
                (Some(Control::Secondary), _) => c.control_pressed,
                (_, Some(Control::Secondary)) => c.control_hover,
                _ => c.control,
            };
            fill(inner, c.background, 3.0);
            fill(inner, color, 3.0);
            text("Cancel", &self.button, r, c.text);
        }
        // SAFETY: closes the BeginDraw above.
        let ended = unsafe { target.EndDraw(None, None) };
        if ended.is_err() {
            self.target = None;
        }
        Ok(())
    }
}

/// The app icon as a Direct2D bitmap.
fn icon_bitmap(target: &ID2D1HwndRenderTarget, size: i32) -> Result<ID2D1Bitmap> {
    // SAFETY: loads the embedded icon and converts it with WIC; the icon is
    // shared (not destroyed) as LoadImage returns it without LR_SHARED but
    // the process ends soon after.
    unsafe {
        let module = GetModuleHandleW(PCWSTR::null())?;
        let handle = LoadImageW(
            Some(module.into()),
            PCWSTR(1 as _),
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        )?;
        let wic: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let bitmap = wic.CreateBitmapFromHICON(HICON(handle.0))?;
        let converter = wic.CreateFormatConverter()?;
        converter.Initialize(
            &bitmap,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        target.CreateBitmapFromWicBitmap(&converter, None)
    }
}

extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = with_ui(|ui| -> Option<LRESULT> {
        let scale = ui.scale();
        let point = || {
            let x = f32::from(lparam.0 as u16 as i16) / scale;
            let y = f32::from((lparam.0 >> 16) as u16 as i16) / scale;
            (x, y)
        };
        match msg {
            WM_PAINT => {
                // SAFETY: this thread's window.
                unsafe {
                    let _ = ValidateRect(Some(hwnd), None);
                }
                let _ = ui.render();
            }
            WM_ERASEBKGND => return Some(LRESULT(1)),
            WM_SIZE => {
                if let Some((target, ..)) = &ui.target {
                    let (w, h) = (
                        (lparam.0 & 0xFFFF) as u32,
                        ((lparam.0 >> 16) & 0xFFFF) as u32,
                    );
                    // SAFETY: resizes this window's target.
                    unsafe {
                        let _ = target.Resize(&D2D_SIZE_U {
                            width: w,
                            height: h,
                        });
                    }
                }
            }
            WM_DPICHANGED => {
                // SAFETY: the suggested rectangle from Windows.
                unsafe {
                    let r = &*(lparam.0 as *const RECT);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                ui.target = None;
                ui.invalidate();
            }
            WM_MOUSEMOVE => {
                let (x, y) = point();
                let hover = ui.hit(x, y);
                if hover != ui.hover {
                    ui.hover = hover;
                    ui.invalidate();
                }
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: an initialized structure for this window.
                unsafe {
                    let _ = TrackMouseEvent(&mut track);
                }
            }
            WM_MOUSELEAVE => {
                ui.hover = None;
                ui.invalidate();
            }
            WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT => {
                let id = if ui.hover == Some(Control::License) {
                    IDC_HAND
                } else {
                    IDC_ARROW
                };
                // SAFETY: shared system cursors.
                unsafe {
                    if let Ok(cursor) = LoadCursorW(None, id) {
                        SetCursor(Some(cursor));
                    }
                }
                return Some(LRESULT(1));
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point();
                ui.pressed = ui.hit(x, y);
                // SAFETY: this thread's window.
                unsafe { SetCapture(hwnd) };
                ui.invalidate();
            }
            WM_LBUTTONUP => {
                // SAFETY: releases the capture taken above.
                unsafe {
                    let _ = ReleaseCapture();
                }
                let (x, y) = point();
                let pressed = ui.pressed.take();
                ui.invalidate();
                if pressed.is_some() && pressed == ui.hit(x, y) {
                    return Some(LRESULT(pressed.map_or(0, |c| c as isize + 1)));
                }
            }
            WM_KEYDOWN => {
                let key = wparam.0 as u16;
                let control = match key {
                    k if k == VK_RETURN.0 => Some(Control::Primary),
                    k if k == VK_ESCAPE.0 && matches!(ui.phase, Phase::Ready) => {
                        Some(Control::Secondary)
                    }
                    k if k == VK_ESCAPE.0 && matches!(ui.phase, Phase::Done | Phase::Failed(_)) => {
                        Some(Control::Primary)
                    }
                    k if k == u16::from(b'S') || k == u16::from(b'K') => Some(Control::First),
                    k if k == u16::from(b'O') => Some(Control::Second),
                    _ => None,
                };
                if let Some(control) = control.filter(|c| ui.control_rect(*c).is_some()) {
                    return Some(LRESULT(control as isize + 1));
                }
            }
            WM_APP_PROGRESS => {
                // SAFETY: posted by the worker with Box::into_raw.
                let (fraction, status) = *unsafe { Box::from_raw(lparam.0 as *mut (f32, String)) };
                ui.phase = Phase::Working { fraction, status };
                ui.invalidate();
            }
            WM_APP_FINISHED => {
                // SAFETY: as above.
                let result = *unsafe {
                    Box::from_raw(
                        lparam.0 as *mut std::result::Result<Option<std::path::PathBuf>, String>,
                    )
                };
                ui.phase = match result {
                    Ok(exe) => {
                        if let (Some(exe), true) = (exe, ui.second) {
                            install::launch(&exe);
                        }
                        Phase::Done
                    }
                    Err(err) => Phase::Failed(err),
                };
                ui.invalidate();
            }
            WM_CLOSE => {
                if !matches!(ui.phase, Phase::Working { .. }) {
                    // SAFETY: ends this thread's message loop.
                    unsafe {
                        PostQuitMessage(if matches!(ui.phase, Phase::Done) {
                            0
                        } else {
                            1
                        });
                    }
                }
                return Some(LRESULT(0));
            }
            WM_DESTROY => {}
            _ => return None,
        }
        Some(LRESULT(0))
    })
    .flatten();
    // Clicks and keys run outside the borrow: the license box is modal.
    if let (Some(LRESULT(code)), WM_LBUTTONUP | WM_KEYDOWN) = (handled, msg) {
        if code > 0 {
            let control = [
                Control::Primary,
                Control::Secondary,
                Control::First,
                Control::Second,
                Control::License,
            ][code as usize - 1];
            if control == Control::License {
                let text: Vec<u16> = LICENSE.encode_utf16().chain([0]).collect();
                // SAFETY: a modal message box with nothing borrowed.
                unsafe {
                    MessageBoxW(
                        Some(hwnd),
                        PCWSTR(text.as_ptr()),
                        w!("MIT License"),
                        MB_OK | MB_ICONINFORMATION,
                    )
                };
            } else {
                with_ui(|ui| ui.activate(control));
            }
        }
        return LRESULT(0);
    }
    match handled {
        Some(result) => result,
        // SAFETY: default handling for this thread's window.
        None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Shows the window and runs until it closes. Returns the exit code.
pub fn run(mode: Mode, payload: &'static [u8], version: &'static str) -> i32 {
    match create(mode, payload, version) {
        Ok(()) => {}
        Err(err) => {
            let text: Vec<u16> = format!("The setup could not start.\n\n{}", err.message())
                .encode_utf16()
                .chain([0])
                .collect();
            // SAFETY: a modal message box.
            unsafe {
                MessageBoxW(
                    None,
                    PCWSTR(text.as_ptr()),
                    w!("Simple Screen Grabber Setup"),
                    MB_OK,
                )
            };
            return 1;
        }
    }
    let mut msg = MSG::default();
    let mut code = 0;
    // SAFETY: standard message loop.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if msg.message == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
            code = msg.wParam.0 as i32;
        }
    }
    UI.with(|cell| cell.borrow_mut().take());
    code
}

fn create(mode: Mode, payload: &'static [u8], version: &'static str) -> Result<()> {
    // SAFETY: registers the class and creates this thread's window; DWM
    // attributes are plain values.
    unsafe {
        let module = GetModuleHandleW(PCWSTR::null())?;
        let icon = LoadImageW(
            Some(module.into()),
            PCWSTR(1 as _),
            IMAGE_ICON,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::LR_DEFAULTSIZE,
        )
        .map(|h| HICON(h.0))
        .unwrap_or_default();
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: module.into(),
            hIcon: icon,
            hIconSm: icon,
            lpszClassName: w!("SimpleScreenGrabber.Setup"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let title = match mode {
            Mode::Install => w!("Simple Screen Grabber Setup"),
            Mode::Uninstall => w!("Uninstall Simple Screen Grabber"),
        };
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SimpleScreenGrabber.Setup"),
            title,
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            100,
            100,
            None,
            None,
            Some(module.into()),
            None,
        )?;

        // Size the client area for the window's DPI and center it.
        let dpi = GetDpiForWindow(hwnd);
        let scale = dpi as f32 / 96.0;
        let mut r = RECT {
            left: 0,
            top: 0,
            right: (WIDTH * scale).round() as i32,
            bottom: (HEIGHT * scale).round() as i32,
        };
        AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE(0), dpi)?;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        SetWindowPos(
            hwnd,
            None,
            (sw - w) / 2,
            (sh - h) / 2,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )?;

        let dark = std::env::var("SIMPLE_SCREEN_GRABBER_THEME")
            .map(|t| t == "dark")
            .unwrap_or_else(|_| {
                registry_dword(
                    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
                    w!("AppsUseLightTheme"),
                ) == Some(0)
            });
        let dark_bool = BOOL::from(dark);
        // DWMWA_USE_IMMERSIVE_DARK_MODE
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(20),
            (&dark_bool as *const BOOL).cast(),
            4,
        );
        // DWMWA_SYSTEMBACKDROP_TYPE = DWMSBT_MAINWINDOW (Mica), Windows 11 22H2+.
        let backdrop = 2i32;
        let mica = DwmSetWindowAttribute(
            hwnd,
            DWMWINDOWATTRIBUTE(38),
            (&backdrop as *const i32).cast(),
            4,
        )
        .is_ok();
        if mica {
            let margins = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            DwmExtendFrameIntoClientArea(hwnd, &margins)?;
        }

        let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let variable = font_exists(&dwrite, w!("Segoe UI Variable Text"));
        let (display, text) = if variable {
            (
                w!("Segoe UI Variable Display"),
                w!("Segoe UI Variable Text"),
            )
        } else {
            (w!("Segoe UI"), w!("Segoe UI"))
        };
        let icons = if font_exists(&dwrite, w!("Segoe Fluent Icons")) {
            w!("Segoe Fluent Icons")
        } else {
            w!("Segoe MDL2 Assets")
        };
        let title_format = format(&dwrite, display, DWRITE_FONT_WEIGHT_SEMI_BOLD, 20.0)?;
        let body = format(&dwrite, text, DWRITE_FONT_WEIGHT_NORMAL, 14.0)?;
        body.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
        let caption = format(&dwrite, text, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?;
        let button = format(&dwrite, text, DWRITE_FONT_WEIGHT_NORMAL, 14.0)?;
        let glyph = format(&dwrite, icons, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?;
        for f in [&button, &glyph] {
            f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }

        let installed = match mode {
            Mode::Install => install::installed_version(),
            Mode::Uninstall => None,
        };
        let first = match mode {
            // Keep the current choice on update.
            Mode::Install if installed.is_some() => install::autostart_enabled(),
            Mode::Install => true,
            // Keep the settings by default.
            Mode::Uninstall => true,
        };
        let ui = Ui {
            hwnd,
            mode,
            payload,
            version,
            installed,
            phase: Phase::Ready,
            first,
            second: true,
            hover: None,
            pressed: None,
            dark,
            mica,
            colors: Colors::load(dark),
            factory,
            dwrite,
            target: None,
            title: title_format,
            body,
            caption,
            button,
            glyph,
        };
        let _ = (ui.dark, &ui.dwrite);
        UI.with(|cell| *cell.borrow_mut() = Some(ui));
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
    }
    Ok(())
}
