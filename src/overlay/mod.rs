//! The capture overlay: one borderless, topmost window over the virtual
//! screen that shows the frozen screen, lets the user select an area and
//! annotate it.
//!
//! The window, the devices and the swap chain are created at startup and
//! kept hidden (and cloaked). A capture grabs the screen, uploads it once,
//! renders the first frame while still cloaked, then uncloaks: the overlay
//! appears complete, with no flash. Closing hides it and releases the
//! capture, the bitmap and the swap chain buffers.

mod dimmer;
mod layout;
mod render;
pub mod selection;
mod toolbar;

use std::time::Instant;

use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_IGNORE, D2D1_PIXEL_FORMAT, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap1, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_NONE,
    D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_CLOAK, DWMWA_TRANSITIONS_FORCEDISABLED,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWM_WINDOW_CORNER_PREFERENCE,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    IDXGISurface, IDXGISwapChain1, DXGI_MWA_NO_ALT_ENTER, DXGI_PRESENT, DXGI_SCALING_NONE,
    DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_DISCARD,
    DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Gdi::{InvalidateRect, ValidateRect};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_HOVER, TRACKMOUSEEVENT,
    VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_OEM_4, VK_OEM_6, VK_RETURN, VK_RIGHT, VK_SHIFT,
    VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, GetCursorPos, GetSystemMetrics, KillTimer, LoadCursorW, PostMessageW,
    RegisterClassExW, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow,
    CS_DBLCLKS, HCURSOR, HTCLIENT, HWND_TOPMOST, IDC_ARROW, IDC_CROSS, IDC_SIZEALL, IDC_SIZENESW,
    IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, SM_CXDRAG, SM_CYDRAG, SWP_NOACTIVATE, SWP_SHOWWINDOW,
    SW_HIDE, WA_INACTIVE, WHEEL_DELTA, WM_ACTIVATE, WM_APP, WM_CAPTURECHANGED, WM_CLOSE,
    WM_DISPLAYCHANGE, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT, WM_SETCURSOR, WM_TIMER, WNDCLASSEXW,
    WNDPROC, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use self::selection::Handle;
use self::toolbar::{Button, Hit, Toolbars};
use crate::annotate::draw::ShapePainter;
use crate::annotate::{History, PointF, Shape, Tool, COLORS, MAX_WIDTH, MIN_WIDTH};
use crate::capture::{self, Capture, Timestamp};
use crate::export::{self, Image};
use crate::fluent::Theme;
use crate::geometry::{Point, Rect};
use crate::gfx::Gfx;
use crate::log;
use crate::win;

const CLASS: PCWSTR = w!("SimpleScreenGrabber.Overlay");
/// Sent after the mouse rests for the hover time (`TrackMouseEvent`); the
/// constants live in the Common Controls bindings, not needed otherwise.
const WM_MOUSEHOVER: u32 = 0x02A1;
const HOVER_DEFAULT: u32 = 0xFFFF_FFFF;
/// Posted to itself once the overlay shows: prepare the Acrylic backdrop.
const WM_APP_PREPARE: u32 = WM_APP + 10;
/// Drives the toolbar fade-in, only while it runs.
const FADE_TIMER: usize = 1;
/// The fade-in duration (the Fluent "fast" duration).
const FADE_SECONDS: f32 = 0.167;
/// Swap chain size while hidden: the buffers are freed, not the chain.
const IDLE_BUFFER: u32 = 8;

/// How long the steps of opening the overlay took.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpenTimings {
    /// Copying the screen.
    pub capture: std::time::Duration,
    /// Uploading it for drawing.
    pub upload: std::time::Duration,
    /// Drawing and presenting the first frame.
    pub first_frame: std::time::Duration,
}

/// What the overlay asks the app to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Close,
    /// Copy the selection, then close.
    Copy,
    /// Save the selection, then close.
    Save,
}

/// The result of handling a window message.
#[derive(Default)]
pub struct Reply {
    pub result: Option<LRESULT>,
    pub event: Option<Event>,
}

impl Reply {
    fn handled(value: isize) -> Reply {
        Reply {
            result: Some(LRESULT(value)),
            event: None,
        }
    }

    fn event(event: Event) -> Reply {
        Reply {
            result: Some(LRESULT(0)),
            event: Some(event),
        }
    }

    fn maybe(event: Option<Event>) -> Reply {
        Reply {
            result: Some(LRESULT(0)),
            event,
        }
    }
}

/// What the left mouse button is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    None,
    /// Pressed outside the selection; becomes a new selection once the
    /// mouse moves past the drag threshold.
    Pending {
        anchor: Point,
    },
    Creating {
        anchor: Point,
    },
    Moving {
        start: Rect,
        grab: Point,
    },
    Resizing {
        start: Rect,
        handle: Handle,
        grab: Point,
    },
    /// Drawing the shape in [`Session::drawing`].
    Drawing,
    /// Pressed on a toolbar item; it acts if released over the same item.
    Pressing(Hit),
}

/// What the mouse is over, for hover effects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hover {
    Handle(Handle),
    Toolbar(Hit),
}

/// One capture, from the shortcut until the overlay closes.
pub struct Session {
    capture: Capture,
    bitmap: ID2D1Bitmap1,
    selection: Option<Rect>,
    drag: Drag,
    mouse: Point,
    hover: Option<Hover>,
    tool: Option<Tool>,
    /// Index in [`COLORS`].
    color: usize,
    /// Stroke width in pixels.
    width: f32,
    history: History,
    drawing: Option<Shape>,
    flyout_open: bool,
    /// The button whose tooltip shows: the mouse rested on it.
    tooltip: Option<Button>,
}

impl Session {
    /// The display scale of the monitor under a point.
    fn scale_at(&self, x: i32, y: i32) -> f32 {
        self.capture.monitor_at(Point::new(x, y)).scale
    }

    /// The toolbars, while a selection exists and is not being dragged.
    fn toolbars(&self) -> Option<Toolbars> {
        let sel = self.selection?;
        if matches!(
            self.drag,
            Drag::Creating { .. } | Drag::Moving { .. } | Drag::Resizing { .. }
        ) {
            return None;
        }
        let monitor = self
            .capture
            .monitor_at(Point::new(sel.right - 1, sel.bottom - 1));
        Some(toolbar::toolbars(
            sel,
            monitor.rect,
            monitor.scale,
            self.flyout_open,
        ))
    }

    fn handle_at(&self, p: Point) -> Option<Handle> {
        let sel = self.selection?;
        let scale = self.scale_at(p.x, p.y);
        let diameter = 2.0 * render::metrics::HANDLE_RADIUS * self.scale_at(sel.left, sel.top);
        let handles = selection::visible_handles(sel, diameter);
        selection::hit_handle(sel, handles, p, render::metrics::HANDLE_HIT * scale)
    }

    fn hover_at(&self, p: Point) -> Option<Hover> {
        if let Some(hit) = self.toolbars().and_then(|bars| bars.hit(p)) {
            return Some(Hover::Toolbar(hit));
        }
        self.handle_at(p).map(Hover::Handle)
    }

    fn is_enabled(&self, button: Button) -> bool {
        match button {
            Button::Undo => self.history.can_undo(),
            Button::Redo => self.history.can_redo(),
            _ => true,
        }
    }

    fn select_tool(&mut self, tool: Tool) {
        self.tool = if self.tool == Some(tool) {
            None
        } else {
            Some(tool)
        };
        self.flyout_open = false;
    }

    fn set_width(&mut self, width: f32) {
        self.width = width.clamp(MIN_WIDTH, MAX_WIDTH);
    }

    /// Runs a toolbar item; some ask the app to act.
    fn activate(&mut self, hit: Hit) -> Option<Event> {
        match hit {
            Hit::Button(Button::Tool(tool)) => self.select_tool(tool),
            Hit::Button(Button::Color) => self.flyout_open = !self.flyout_open,
            Hit::Button(Button::Undo) => {
                self.history.undo();
            }
            Hit::Button(Button::Redo) => {
                self.history.redo();
            }
            Hit::Button(Button::Copy) => return Some(Event::Copy),
            Hit::Button(Button::Save) => return Some(Event::Save),
            Hit::Button(Button::Close) => return Some(Event::Close),
            Hit::Swatch(i) => {
                self.color = i;
                self.flyout_open = false;
            }
            Hit::Background => {}
        }
        None
    }
}

struct Cursors {
    arrow: HCURSOR,
    cross: HCURSOR,
    move_all: HCURSOR,
    nwse: HCURSOR,
    nesw: HCURSOR,
    we: HCURSOR,
    ns: HCURSOR,
}

impl Cursors {
    fn load() -> Result<Cursors> {
        // SAFETY: loads shared system cursors, which are never destroyed.
        let load = |id: PCWSTR| unsafe { LoadCursorW(None, id) };
        Ok(Cursors {
            arrow: load(IDC_ARROW)?,
            cross: load(IDC_CROSS)?,
            move_all: load(IDC_SIZEALL)?,
            nwse: load(IDC_SIZENWSE)?,
            nesw: load(IDC_SIZENESW)?,
            we: load(IDC_SIZEWE)?,
            ns: load(IDC_SIZENS)?,
        })
    }
}

pub struct Overlay {
    hwnd: HWND,
    gfx: Gfx,
    swap_chain: IDXGISwapChain1,
    target: Option<ID2D1Bitmap1>,
    painter: render::Painter,
    shapes: ShapePainter,
    theme: Theme,
    cursors: Cursors,
    session: Option<Session>,
    /// When the toolbars started fading in, and whether the timer runs.
    fade_start: Option<Instant>,
    fade_timer: bool,
    /// The color (index in `COLORS`) and stroke width (DIPs) a capture
    /// starts with, from the settings.
    default_color: usize,
    default_width: f32,
    /// Dims the screen at once while it is being copied.
    dimmer: Option<dimmer::Dimmer>,
    /// The steps of the last `open`, for the benchmark.
    pub last_open: OpenTimings,
    /// The size of the last capture.
    last_size: (i32, i32),
}

impl Overlay {
    /// Creates the hidden overlay window and its graphics resources.
    pub fn new(window_proc: WNDPROC, theme: Theme) -> Result<Overlay> {
        let gfx = Gfx::new()?;
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DBLCLKS,
            lpfnWndProc: window_proc,
            hInstance: win::instance(),
            lpszClassName: CLASS,
            ..Default::default()
        };
        // SAFETY: registers the class and creates a hidden popup window of
        // this thread; the DWM attributes are plain values.
        let hwnd = unsafe {
            RegisterClassExW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                CLASS,
                w!("Simple Screen Grabber"),
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
            set_dwm_bool(hwnd, DWMWA_CLOAK.0 as u32, true);
            set_dwm_bool(hwnd, DWMWA_TRANSITIONS_FORCEDISABLED.0 as u32, true);
            let corners: DWM_WINDOW_CORNER_PREFERENCE = DWMWCP_DONOTROUND;
            // Windows 10 has no rounded corners: ignore the error there.
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                (&corners as *const DWM_WINDOW_CORNER_PREFERENCE).cast(),
                size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
            );
            hwnd
        };
        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: IDLE_BUFFER,
            Height: IDLE_BUFFER,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            // Never stretched: the buffers always match the window.
            Scaling: DXGI_SCALING_NONE,
            SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            ..Default::default()
        };
        // SAFETY: COM calls on the devices created above, for this window.
        let swap_chain = unsafe {
            let chain = gfx
                .dxgi_factory
                .CreateSwapChainForHwnd(&gfx.d3d, hwnd, &desc, None, None)?;
            gfx.dxgi_factory
                .MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER)?;
            chain
        };
        let painter = render::Painter::new(&gfx.ctx, &gfx.dwrite)?;
        let shapes = ShapePainter::new(&gfx.ctx)?;
        let dimmer = dimmer::Dimmer::new(theme.palette.scrim.a)?;
        Ok(Overlay {
            hwnd,
            gfx,
            swap_chain,
            target: None,
            painter,
            shapes,
            theme,
            cursors: Cursors::load()?,
            session: None,
            fade_start: None,
            fade_timer: false,
            default_color: 0,
            default_width: 3.0,
            dimmer,
            last_open: OpenTimings::default(),
            last_size: (0, 0),
        })
    }

    fn new_session(&self, capture: Capture, bitmap: ID2D1Bitmap1, mouse: Point) -> Session {
        let scale = capture.monitor_at(mouse).scale;
        Session {
            capture,
            bitmap,
            selection: None,
            drag: Drag::None,
            mouse,
            hover: None,
            tool: None,
            color: self.default_color,
            width: (self.default_width * scale)
                .round()
                .clamp(MIN_WIDTH, MAX_WIDTH),
            history: History::default(),
            drawing: None,
            flyout_open: false,
            tooltip: None,
        }
    }

    /// The size of the last capture (the virtual screen).
    pub fn screen_size(&self) -> (i32, i32) {
        self.last_size
    }

    /// Benchmark: shows `selection` with the toolbars and draws a frame;
    /// returns the drawing time, before the wait for vsync.
    pub fn bench_frame(&mut self, selection: Rect) -> Result<std::time::Duration> {
        if let Some(session) = &mut self.session {
            session.selection = Some(selection.intersect(session.capture.bounds()));
            session.drag = Drag::None;
        }
        self.render()
    }

    /// Benchmark: adds a line across `area`, so the export draws.
    pub fn bench_annotate(&mut self, area: Rect) {
        if let Some(session) = &mut self.session {
            let mut line = Shape::new(
                Tool::Line,
                COLORS[0].0,
                3.0,
                PointF::center(area.left + 10, area.top + 10),
            );
            line.extend(PointF::center(area.right - 10, area.bottom - 10), false);
            session.history.push(line);
        }
    }

    /// The color and stroke width the next captures start with.
    pub fn set_defaults(&mut self, color: usize, width: f32) {
        self.default_color = color.min(COLORS.len() - 1);
        self.default_width = width;
    }

    /// Follows a new theme or new visual settings.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.invalidate();
    }

    /// Captures the screen and shows the overlay.
    pub fn open(&mut self) -> Result<()> {
        if self.session.is_some() {
            return Ok(());
        }
        let started = Instant::now();
        if let Some(dimmer) = &self.dimmer {
            dimmer.show();
        }
        let capture = match capture::capture_screen() {
            Ok(capture) => capture,
            Err(err) => {
                self.hide_dimmer();
                return Err(err);
            }
        };
        let captured = started.elapsed();
        let bitmap = self.upload(&capture)?;
        let uploaded = started.elapsed();
        self.last_size = (capture.width, capture.height);
        let mut cursor = POINT::default();
        // SAFETY: plain query.
        unsafe {
            let _ = GetCursorPos(&mut cursor);
        }
        let mouse = Point::new(cursor.x - capture.origin.x, cursor.y - capture.origin.y);
        let (origin, width, height) = (capture.origin, capture.width, capture.height);
        self.session = Some(self.new_session(capture, bitmap, mouse));
        // SAFETY: positions and shows this thread's window while it is
        // still cloaked, so nothing appears yet.
        unsafe {
            SetWindowPos(
                self.hwnd,
                Some(HWND_TOPMOST),
                origin.x,
                origin.y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )?;
        }
        if let Err(err) = self
            .resize_buffers(width as u32, height as u32)
            .and_then(|()| self.render().map(|_| ()))
        {
            self.close();
            return Err(err);
        }
        let rendered = started.elapsed();
        self.last_open = OpenTimings {
            capture: captured,
            upload: uploaded - captured,
            first_frame: rendered - uploaded,
        };
        // SAFETY: uncloaks and activates this thread's window; the posted
        // message prepares the Acrylic backdrop once the overlay is shown.
        unsafe {
            set_dwm_bool(self.hwnd, DWMWA_CLOAK.0 as u32, false);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(Some(self.hwnd));
            SetCursor(Some(self.cursors.cross));
            let _ = PostMessageW(Some(self.hwnd), WM_APP_PREPARE, WPARAM(0), LPARAM(0));
        }
        // The overlay now covers it, with the same dimming.
        self.hide_dimmer();
        log!(
            "capture {width}x{height}: screen {:.1} ms, overlay shown after {:.1} ms",
            captured.as_secs_f64() * 1000.0,
            started.elapsed().as_secs_f64() * 1000.0
        );
        Ok(())
    }

    /// The image to save or copy: the selection, or the whole monitor under
    /// the mouse when nothing is selected, with the annotations. Taken
    /// before the overlay closes; `None` when no capture is open.
    pub fn export(&self) -> Option<Result<(Image, Timestamp)>> {
        let session = self.session.as_ref()?;
        let area = session
            .selection
            .unwrap_or_else(|| session.capture.monitor_at(session.mouse).rect);
        let shapes = session.history.shapes();
        let touched = shapes.iter().any(|shape| {
            let (min, max) = shape.bounds();
            min.x < area.right as f32
                && max.x > area.left as f32
                && min.y < area.bottom as f32
                && max.y > area.top as f32
        });
        let image = if touched {
            export::render(&self.gfx.ctx, &self.shapes, &session.bitmap, shapes, area)
        } else {
            // Nothing drawn here: copy the captured pixels directly.
            Ok(Image::crop(&session.capture, area))
        };
        Some(image.map(|image| (image, session.capture.time)))
    }

    fn hide_dimmer(&self) {
        if let Some(dimmer) = &self.dimmer {
            dimmer.hide();
        }
    }

    /// Benchmark: dims the screen as the shortcut does first; `false`
    /// when this Windows version cannot exclude it from capture.
    pub fn bench_dim(&self, on: bool) -> bool {
        match &self.dimmer {
            Some(dimmer) if on => dimmer.show(),
            Some(dimmer) => dimmer.hide(),
            None => return false,
        }
        true
    }

    /// Hides the overlay and frees the capture.
    pub fn close(&mut self) {
        self.hide_dimmer();
        if self.session.take().is_none() {
            return;
        }
        self.painter.release();
        self.stop_fade();
        // SAFETY: hides this thread's window; releases the mouse if held.
        unsafe {
            set_dwm_bool(self.hwnd, DWMWA_CLOAK.0 as u32, true);
            let _ = ShowWindow(self.hwnd, SW_HIDE);
            let _ = ReleaseCapture();
        }
        if let Err(err) = self.resize_buffers(IDLE_BUFFER, IDLE_BUFFER) {
            log!("swap chain not shrunk: {err}");
        }
        self.gfx.release_caches();
        win::trim_working_set();
    }

    /// Uploads the captured pixels for drawing.
    fn upload(&self, capture: &Capture) -> Result<ID2D1Bitmap1> {
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            ..Default::default()
        };
        let size = D2D_SIZE_U {
            width: capture.width as u32,
            height: capture.height as u32,
        };
        // SAFETY: the pixel slice holds height rows of `stride` bytes and
        // outlives the call, which copies it.
        unsafe {
            self.gfx.ctx.CreateBitmap(
                size,
                Some(capture.pixels().as_ptr().cast()),
                capture.stride() as u32,
                &props,
            )
        }
    }

    fn resize_buffers(&mut self, width: u32, height: u32) -> Result<()> {
        self.target = None;
        // SAFETY: the device context no longer references the back buffer,
        // so the swap chain can reallocate it.
        unsafe {
            self.gfx.ctx.SetTarget(None);
            self.swap_chain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )
        }
    }

    /// The Direct2D bitmap of the swap chain's back buffer.
    fn target(&mut self) -> Result<ID2D1Bitmap1> {
        if let Some(target) = &self.target {
            return Ok(target.clone());
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        // SAFETY: wraps the swap chain's back buffer for Direct2D.
        let target = unsafe {
            let surface: IDXGISurface = self.swap_chain.GetBuffer(0)?;
            self.gfx
                .ctx
                .CreateBitmapFromDxgiSurface(&surface, Some(&props))?
        };
        self.target = Some(target.clone());
        Ok(target)
    }

    /// The toolbars' opacity: they fade in when they appear, if Windows
    /// animations are on. A timer drives the fade only while it runs.
    fn fade(&mut self, visible: bool) -> f32 {
        if !visible || !self.theme.animations {
            self.stop_fade();
            return 1.0;
        }
        let start = *self.fade_start.get_or_insert_with(Instant::now);
        let t = start.elapsed().as_secs_f32() / FADE_SECONDS;
        if t >= 1.0 {
            if self.fade_timer {
                // SAFETY: stops this thread's window timer.
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), FADE_TIMER);
                }
                self.fade_timer = false;
            }
            return 1.0;
        }
        if !self.fade_timer {
            // SAFETY: starts a timer for this thread's window.
            unsafe { SetTimer(Some(self.hwnd), FADE_TIMER, 15, None) };
            self.fade_timer = true;
        }
        // Ease out (decelerate), like Fluent motion.
        1.0 - (1.0 - t).powi(3)
    }

    fn stop_fade(&mut self) {
        self.fade_start = None;
        if self.fade_timer {
            // SAFETY: stops this thread's window timer.
            unsafe {
                let _ = KillTimer(Some(self.hwnd), FADE_TIMER);
            }
            self.fade_timer = false;
        }
    }

    /// Draws and presents one frame; returns the drawing time, before the
    /// wait for vsync.
    fn render(&mut self) -> Result<std::time::Duration> {
        let visible = match &self.session {
            Some(session) => session.toolbars().is_some(),
            None => return Ok(std::time::Duration::ZERO),
        };
        let started = Instant::now();
        let fade = self.fade(visible);
        let target = self.target()?;
        let ctx = self.gfx.ctx.clone();
        let Some(session) = &self.session else {
            return Ok(std::time::Duration::ZERO);
        };
        let bars = session.toolbars();
        self.painter
            .prepare(&ctx, session, bars.as_ref(), &self.theme)?;
        // SAFETY: one BeginDraw/EndDraw pair on this thread's context.
        unsafe {
            ctx.SetTarget(&target);
            ctx.BeginDraw();
        }
        let drawn = self.painter.frame(
            &ctx,
            session,
            &self.shapes,
            &self.theme,
            bars.as_ref(),
            fade,
        );
        // SAFETY: closes the BeginDraw above, then presents with vsync.
        unsafe {
            let ended = ctx.EndDraw(None, None);
            drawn?;
            ended?;
            let elapsed = started.elapsed();
            self.swap_chain.Present(1, DXGI_PRESENT(0)).ok()?;
            Ok(elapsed)
        }
    }

    fn invalidate(&self) {
        if self.session.is_some() {
            // SAFETY: invalidates this thread's window; WM_PAINT renders.
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }
    }

    /// Handles a message of the overlay window.
    pub fn message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Reply {
        match msg {
            WM_PAINT => {
                // SAFETY: validates this thread's window.
                unsafe {
                    let _ = ValidateRect(Some(self.hwnd), None);
                }
                if let Err(err) = self.render() {
                    log!("render failed: {err}");
                    return Reply::event(Event::Close);
                }
                Reply::handled(0)
            }
            WM_ERASEBKGND => Reply::handled(1),
            // Only the app's host window decides to exit.
            WM_CLOSE => Reply::handled(0),
            WM_TIMER if wparam.0 == FADE_TIMER => {
                self.invalidate();
                Reply::handled(0)
            }
            WM_APP_PREPARE => {
                if let (Some(session), true) = (&self.session, self.theme.materials) {
                    if let Err(err) = self.painter.prepare_backdrop(&self.gfx.ctx, session) {
                        log!("backdrop failed: {err}");
                    }
                }
                Reply::handled(0)
            }
            WM_SETCURSOR if win::low_word(lparam.0 as usize) == HTCLIENT => {
                self.update_cursor();
                Reply::handled(1)
            }
            WM_MOUSEMOVE => {
                self.mouse_move(point(lparam));
                Reply::handled(0)
            }
            WM_MOUSEHOVER => {
                if let Some(session) = &mut self.session {
                    if let (Drag::None, Some(Hover::Toolbar(Hit::Button(button)))) =
                        (session.drag, session.hover)
                    {
                        session.tooltip = Some(button);
                        self.invalidate();
                    }
                }
                Reply::handled(0)
            }
            WM_LBUTTONDOWN => {
                self.button_down(point(lparam));
                Reply::handled(0)
            }
            WM_LBUTTONUP => Reply::maybe(self.button_up(point(lparam))),
            WM_LBUTTONDBLCLK => {
                let p = point(lparam);
                let copy = self.session.as_ref().is_some_and(|s| {
                    s.tool.is_none()
                        && s.selection.is_some_and(|sel| sel.contains(p))
                        && s.toolbars().and_then(|bars| bars.hit(p)).is_none()
                });
                if copy {
                    Reply::event(Event::Copy)
                } else {
                    // The second click of a double-click still counts.
                    self.button_down(p);
                    Reply::handled(0)
                }
            }
            WM_MOUSEWHEEL => {
                let notches = i32::from((wparam.0 >> 16) as u16 as i16) / WHEEL_DELTA as i32;
                if let Some(session) = &mut self.session {
                    session.set_width(session.width + notches as f32);
                    self.invalidate();
                }
                Reply::handled(0)
            }
            WM_CAPTURECHANGED => {
                if lparam.0 != self.hwnd.0 as isize {
                    self.cancel_drag();
                }
                Reply::handled(0)
            }
            WM_KEYDOWN => self.key_down(wparam.0 as u16),
            WM_ACTIVATE if win::low_word(wparam.0) == WA_INACTIVE => Reply {
                result: None,
                event: self.session.as_ref().map(|_| Event::Close),
            },
            WM_DISPLAYCHANGE => Reply::event(Event::Close),
            // The window spans monitors on purpose: ignore DPI changes.
            WM_DPICHANGED => Reply::handled(0),
            _ => Reply::default(),
        }
    }

    fn update_cursor(&self) {
        let Some(session) = &self.session else {
            return;
        };
        let handle = match (session.drag, session.hover) {
            (Drag::Resizing { handle, .. }, _) => Some(handle),
            (Drag::None, Some(Hover::Handle(handle))) => Some(handle),
            _ => None,
        };
        let on_toolbar = matches!(session.drag, Drag::Pressing(_))
            || (session.drag == Drag::None && matches!(session.hover, Some(Hover::Toolbar(_))));
        let moving = matches!(session.drag, Drag::Moving { .. })
            || (session.drag == Drag::None
                && session.tool.is_none()
                && session.selection.is_some_and(|s| s.contains(session.mouse)));
        let cursor = match handle {
            Some(h) if h.is_nwse() => self.cursors.nwse,
            Some(h) if h.is_nesw() => self.cursors.nesw,
            Some(h) if h.is_horizontal() => self.cursors.we,
            Some(_) => self.cursors.ns,
            None if on_toolbar => self.cursors.arrow,
            None if moving => self.cursors.move_all,
            None => self.cursors.cross,
        };
        // SAFETY: sets a shared system cursor.
        unsafe { SetCursor(Some(cursor)) };
    }

    fn button_down(&mut self, p: Point) {
        let Some(session) = &mut self.session else {
            return;
        };
        session.mouse = p;
        session.tooltip = None;
        if let Some(hit) = session.toolbars().and_then(|bars| bars.hit(p)) {
            session.drag = Drag::Pressing(hit);
        } else if session.flyout_open {
            // A click outside the open flyout only closes it.
            session.flyout_open = false;
            session.drag = Drag::None;
            self.invalidate();
            return;
        } else if let (Some(handle), Some(start)) = (session.handle_at(p), session.selection) {
            session.drag = Drag::Resizing {
                start,
                handle,
                grab: p,
            };
        } else if let (Some(tool), Some(_)) = (session.tool, session.selection) {
            let color = COLORS[session.color].0;
            session.drawing = Some(Shape::new(
                tool,
                color,
                session.width,
                PointF::center(p.x, p.y),
            ));
            session.drag = Drag::Drawing;
        } else if let Some(start) = session.selection.filter(|s| s.contains(p)) {
            session.drag = Drag::Moving { start, grab: p };
        } else {
            session.drag = Drag::Pending { anchor: p };
        }
        // SAFETY: captures the mouse for this thread's window.
        unsafe { SetCapture(self.hwnd) };
        self.invalidate();
    }

    fn mouse_move(&mut self, p: Point) {
        let Some(session) = &mut self.session else {
            return;
        };
        session.mouse = p;
        let bounds = session.capture.bounds();
        let clamped = bounds.clamp(p);
        match session.drag {
            Drag::None | Drag::Pressing(_) => {
                let hover = session.hover_at(p);
                // The brush outline follows the mouse while a tool is on.
                let follow = session.tool.is_some() && session.drag == Drag::None;
                if hover != session.hover {
                    session.hover = hover;
                    session.tooltip = None;
                    if matches!(hover, Some(Hover::Toolbar(Hit::Button(_)))) {
                        track_hover(self.hwnd);
                    }
                    self.invalidate();
                } else if follow {
                    self.invalidate();
                }
                return;
            }
            Drag::Pending { anchor } => {
                // SAFETY: plain system metric queries.
                let (dx, dy) =
                    unsafe { (GetSystemMetrics(SM_CXDRAG), GetSystemMetrics(SM_CYDRAG)) };
                if (p.x - anchor.x).abs() < dx && (p.y - anchor.y).abs() < dy {
                    return;
                }
                let anchor = bounds.clamp(anchor);
                session.drag = Drag::Creating { anchor };
                session.selection = Some(Rect::spanning(anchor, clamped));
            }
            Drag::Creating { anchor } => {
                session.selection = Some(Rect::spanning(anchor, clamped));
            }
            Drag::Moving { start, grab } => {
                session.selection =
                    Some(selection::moved(start, p.x - grab.x, p.y - grab.y, bounds));
            }
            Drag::Resizing {
                start,
                handle,
                grab,
            } => {
                session.selection = Some(selection::resized(
                    start,
                    handle,
                    p.x - grab.x,
                    p.y - grab.y,
                    bounds,
                ));
            }
            Drag::Drawing => {
                // SAFETY: reads the keyboard state of this thread.
                let shift = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
                if let Some(shape) = &mut session.drawing {
                    shape.extend(PointF::center(p.x, p.y), shift);
                }
            }
        }
        self.invalidate();
    }

    fn button_up(&mut self, p: Point) -> Option<Event> {
        // SAFETY: releases the mouse captured in button_down.
        unsafe {
            let _ = ReleaseCapture();
        }
        let session = self.session.as_mut()?;
        session.mouse = p;
        let mut event = None;
        match session.drag {
            Drag::Pressing(hit) => {
                if session.toolbars().and_then(|bars| bars.hit(p)) == Some(hit) {
                    let enabled = match hit {
                        Hit::Button(b) => session.is_enabled(b),
                        _ => true,
                    };
                    if enabled {
                        event = session.activate(hit);
                    }
                }
            }
            Drag::Drawing => {
                if let Some(shape) = session.drawing.take().filter(Shape::is_visible) {
                    session.history.push(shape);
                }
            }
            _ => {}
        }
        session.drag = Drag::None;
        session.hover = session.hover_at(p);
        self.invalidate();
        event
    }

    /// Ends a drag without finishing it (the mouse capture was lost).
    fn cancel_drag(&mut self) {
        if let Some(session) = &mut self.session {
            if session.drag != Drag::None {
                session.drag = Drag::None;
                session.drawing = None;
                session.hover = session.hover_at(session.mouse);
                self.invalidate();
            }
        }
    }

    fn key_down(&mut self, key: u16) -> Reply {
        // SAFETY: reads the keyboard state of this thread.
        let (ctrl, shift) = unsafe {
            (
                GetKeyState(i32::from(VK_CONTROL.0)) < 0,
                GetKeyState(i32::from(VK_SHIFT.0)) < 0,
            )
        };
        let Some(session) = &mut self.session else {
            return Reply::default();
        };
        let busy = session.drag != Drag::None;
        match key {
            k if k == VK_ESCAPE.0 => {
                if session.flyout_open {
                    session.flyout_open = false;
                } else {
                    return Reply::event(Event::Close);
                }
            }
            k if k == VK_RETURN.0 => return Reply::event(Event::Copy),
            k if ctrl && k == u16::from(b'C') => return Reply::event(Event::Copy),
            k if ctrl && k == u16::from(b'S') => return Reply::event(Event::Save),
            k if ctrl && !busy && k == u16::from(b'Z') => {
                if shift {
                    session.history.redo();
                } else {
                    session.history.undo();
                }
            }
            k if ctrl && !busy && k == u16::from(b'Y') => {
                session.history.redo();
            }
            k if !ctrl && Tool::from_key(k).is_some() => {
                if let Some(tool) = Tool::from_key(k) {
                    session.select_tool(tool);
                }
            }
            k if !ctrl && (u16::from(b'1')..=u16::from(b'8')).contains(&k) => {
                session.color = usize::from(k - u16::from(b'1'));
                session.flyout_open = false;
            }
            k if k == VK_OEM_4.0 => session.set_width(session.width - 1.0),
            k if k == VK_OEM_6.0 => session.set_width(session.width + 1.0),
            k if [VK_LEFT.0, VK_RIGHT.0, VK_UP.0, VK_DOWN.0].contains(&k) => {
                let step = if shift { 10 } else { 1 };
                let (dx, dy) = match k {
                    k if k == VK_LEFT.0 => (-step, 0),
                    k if k == VK_RIGHT.0 => (step, 0),
                    k if k == VK_UP.0 => (0, -step),
                    _ => (0, step),
                };
                if let (Some(sel), false) = (session.selection, busy) {
                    session.selection =
                        Some(selection::moved(sel, dx, dy, session.capture.bounds()));
                }
            }
            _ => return Reply::default(),
        }
        self.invalidate();
        Reply::handled(0)
    }
}

/// Asks for one WM_MOUSEHOVER after the mouse rests for the system hover
/// time.
fn track_hover(hwnd: HWND) {
    let mut track = TRACKMOUSEEVENT {
        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
        dwFlags: TME_HOVER,
        hwndTrack: hwnd,
        dwHoverTime: HOVER_DEFAULT,
    };
    // SAFETY: the structure is initialized for this thread's window.
    unsafe {
        let _ = TrackMouseEvent(&mut track);
    }
}

/// The client point of a mouse message.
fn point(lparam: LPARAM) -> Point {
    Point::new(win::x_of(lparam.0), win::y_of(lparam.0))
}

/// Sets a boolean DWM window attribute.
fn set_dwm_bool(hwnd: HWND, attribute: u32, value: bool) {
    let value = BOOL::from(value);
    // SAFETY: passes a BOOL of the size the attribute expects.
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE(attribute as i32),
            (&value as *const BOOL).cast(),
            size_of::<BOOL>() as u32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Monitor;

    /// A synthetic screen: a light desktop with a window, lines of "text"
    /// and a colored picture. No real screen content.
    fn screen(width: i32, height: i32) -> Capture {
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let at = (y as usize * width as usize + x as usize) * 4;
                let (wx, wy) = (x - width / 8, y - height / 8);
                let inside = wx >= 0 && wy >= 0 && wx < width * 3 / 4 && wy < height * 3 / 4;
                let color: [u8; 3] = if !inside {
                    [0x8A, 0x6A, 0x3C] // desktop (BGR)
                } else if wy < 32 {
                    [0xF3, 0xF3, 0xF3] // title bar
                } else if wx > width * 3 / 8 && wy > 60 && wy < 60 + height / 3 {
                    let t = (wx + wy) as u8;
                    [t, 0x80u8.wrapping_add(t / 2), 0xC0] // picture
                } else if (wy - 50) % 22 < 9 && wx > 24 && wx < width * 3 / 8 - 24 - (wy % 7) * 9 {
                    [0x30, 0x30, 0x30] // text line
                } else {
                    [0xFF, 0xFF, 0xFF] // page
                };
                pixels[at..at + 3].copy_from_slice(&color);
            }
        }
        let monitor = Monitor {
            rect: Rect::new(0, 0, width, height),
            scale: 1.0,
        };
        Capture::synthetic(width, height, vec![monitor], pixels)
    }

    #[test]
    #[ignore = "writes preview images to SIMPLE_SCREEN_GRABBER_PREVIEW"]
    fn render_previews() {
        let Ok(folder) = std::env::var("SIMPLE_SCREEN_GRABBER_PREVIEW") else {
            return;
        };
        // SAFETY: COM for this test thread.
        let _ = unsafe {
            windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            )
        };
        let gfx = Gfx::new().unwrap();
        let mut painter = render::Painter::new(&gfx.ctx, &gfx.dwrite).unwrap();
        let shapes = ShapePainter::new(&gfx.ctx).unwrap();
        for (name, dark, scale) in [("light-100", false, 1.0), ("dark-150", true, 1.5)] {
            let (w, h) = ((1280.0 * scale) as i32, (800.0 * scale) as i32);
            let mut capture = screen(w, h);
            capture.monitors[0].scale = scale;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                ..Default::default()
            };
            let size = D2D_SIZE_U {
                width: w as u32,
                height: h as u32,
            };
            // SAFETY: the pixels outlive the call, which copies them.
            let bitmap = unsafe {
                gfx.ctx
                    .CreateBitmap(
                        size,
                        Some(capture.pixels().as_ptr().cast()),
                        capture.stride() as u32,
                        &props,
                    )
                    .unwrap()
            };
            let px = |v: f32| (v * scale) as i32;
            let sel = Rect::new(px(220.0), px(150.0), px(860.0), px(560.0));
            let mut history = History::default();
            let mut add = |tool, color: usize, a: (f32, f32), b: &[(f32, f32)]| {
                let mut shape = Shape::new(
                    tool,
                    COLORS[color].0,
                    3.0 * scale,
                    PointF::new(a.0 * scale, a.1 * scale),
                );
                for p in b {
                    shape.extend(PointF::new(p.0 * scale, p.1 * scale), false);
                }
                history.push(shape);
            };
            add(Tool::Arrow, 0, (300.0, 470.0), &[(430.0, 380.0)]);
            add(Tool::Rectangle, 4, (520.0, 220.0), &[(760.0, 330.0)]);
            add(Tool::Line, 3, (260.0, 520.0), &[(500.0, 520.0)]);
            add(
                Tool::Highlighter,
                2,
                (262.0, 213.0),
                &[(330.0, 213.0), (420.0, 213.0)],
            );
            add(
                Tool::Pen,
                5,
                (600.0, 420.0),
                &[
                    (620.0, 400.0),
                    (650.0, 395.0),
                    (680.0, 410.0),
                    (700.0, 440.0),
                    (690.0, 470.0),
                ],
            );
            let mut session = Session {
                capture,
                bitmap,
                selection: Some(sel),
                drag: Drag::None,
                mouse: Point::new(px(700.0), px(260.0)),
                hover: Some(Hover::Toolbar(Hit::Button(Button::Tool(Tool::Rectangle)))),
                tool: Some(Tool::Arrow),
                color: 0,
                width: 3.0 * scale,
                history,
                drawing: None,
                flyout_open: dark,
                tooltip: (!dark).then_some(Button::Save),
            };
            if !dark {
                session.hover = Some(Hover::Toolbar(Hit::Button(Button::Save)));
            }
            let theme = Theme {
                palette: crate::fluent::Palette::new(dark, None),
                materials: true,
                animations: false,
                high_contrast: false,
            };
            let bars = session.toolbars();
            painter
                .prepare(&gfx.ctx, &session, bars.as_ref(), &theme)
                .unwrap();
            // SAFETY: one BeginDraw/EndDraw pair on an offscreen target.
            let image = unsafe {
                let target = gfx
                    .ctx
                    .CreateBitmap(size, None, 0, &export::target_properties())
                    .unwrap();
                gfx.ctx.SetTarget(&target);
                gfx.ctx.BeginDraw();
                painter
                    .frame(&gfx.ctx, &session, &shapes, &theme, bars.as_ref(), 1.0)
                    .unwrap();
                gfx.ctx.EndDraw(None, None).unwrap();
                gfx.ctx.SetTarget(None);
                export::read_back(&gfx.ctx, &target, size).unwrap()
            };
            let path = std::path::Path::new(&folder).join(format!("overlay-{name}.png"));
            export::png::write_file(&image, &path).unwrap();
        }
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;
    use crate::capture::Monitor;
    use crate::fluent::Palette;
    use windows::Win32::UI::WindowsAndMessaging::DefWindowProcW;

    extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: default handling for this thread's window.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    fn lparam(x: i32, y: i32) -> LPARAM {
        LPARAM(((y as u16 as isize) << 16) | (x as u16 as isize))
    }

    /// An overlay showing a synthetic 800x600 screen (no window shown).
    fn overlay() -> Overlay {
        let theme = Theme {
            palette: Palette::new(false, None),
            materials: false,
            animations: false,
            high_contrast: false,
        };
        let mut overlay = Overlay::new(Some(window_proc), theme).unwrap();
        let (w, h) = (800, 600);
        let pixels: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % 251) as u8, (i % 241) as u8, (i % 239) as u8, 0])
            .collect();
        let monitor = Monitor {
            rect: Rect::new(0, 0, w, h),
            scale: 1.0,
        };
        let capture = Capture::synthetic(w, h, vec![monitor], pixels);
        let bitmap = overlay.upload(&capture).unwrap();
        overlay.session = Some(overlay.new_session(capture, bitmap, Point::new(400, 300)));
        overlay
    }

    fn drag(overlay: &mut Overlay, from: (i32, i32), to: (i32, i32)) -> Reply {
        overlay.message(WM_MOUSEMOVE, WPARAM(0), lparam(from.0, from.1));
        overlay.message(WM_LBUTTONDOWN, WPARAM(1), lparam(from.0, from.1));
        overlay.message(
            WM_MOUSEMOVE,
            WPARAM(1),
            lparam((from.0 + to.0) / 2, (from.1 + to.1) / 2),
        );
        overlay.message(WM_MOUSEMOVE, WPARAM(1), lparam(to.0, to.1));
        overlay.message(WM_LBUTTONUP, WPARAM(0), lparam(to.0, to.1))
    }

    fn click(overlay: &mut Overlay, at: (i32, i32)) -> Reply {
        overlay.message(WM_MOUSEMOVE, WPARAM(0), lparam(at.0, at.1));
        overlay.message(WM_LBUTTONDOWN, WPARAM(1), lparam(at.0, at.1));
        overlay.message(WM_LBUTTONUP, WPARAM(0), lparam(at.0, at.1))
    }

    fn key(overlay: &mut Overlay, vk: u16) -> Reply {
        overlay.message(WM_KEYDOWN, WPARAM(usize::from(vk)), LPARAM(0))
    }

    fn session(overlay: &Overlay) -> &Session {
        overlay.session.as_ref().unwrap()
    }

    /// The center of a toolbar button, in overlay pixels.
    fn button_center(overlay: &Overlay, button: Button) -> (i32, i32) {
        let bars = session(overlay).toolbars().expect("toolbars are shown");
        let mut slots = bars.tools.slots.iter().chain(bars.actions.slots.iter());
        let r = slots
            .find_map(|s| match s {
                toolbar::Slot::Button(b, r) if *b == button => Some(*r),
                _ => None,
            })
            .expect("the button exists");
        (
            (r.left + r.width() / 2.0) as i32,
            (r.top + r.height() / 2.0) as i32,
        )
    }

    #[test]
    fn select_draw_undo_move_resize_export_close() {
        let mut overlay = overlay();

        // A click alone selects nothing.
        click(&mut overlay, (50, 50));
        assert_eq!(session(&overlay).selection, None);

        // Dragging selects both corner pixels.
        drag(&mut overlay, (100, 100), (300, 250));
        assert_eq!(
            session(&overlay).selection,
            Some(Rect::new(100, 100, 301, 251))
        );
        assert!(session(&overlay).toolbars().is_some());

        // R picks the rectangle tool; dragging inside draws one.
        key(&mut overlay, u16::from(b'R'));
        assert_eq!(session(&overlay).tool, Some(Tool::Rectangle));
        drag(&mut overlay, (150, 150), (200, 200));
        assert_eq!(session(&overlay).history.shapes().len(), 1);
        assert_eq!(
            session(&overlay).selection,
            Some(Rect::new(100, 100, 301, 251))
        );

        // The Undo button removes it; 3 then picks green.
        let undo = button_center(&overlay, Button::Undo);
        click(&mut overlay, undo);
        assert_eq!(session(&overlay).history.shapes().len(), 0);
        key(&mut overlay, u16::from(b'4'));
        assert_eq!(session(&overlay).color, 3);

        // Clicking the active tool's button turns it off; dragging inside
        // the selection then moves it.
        let rectangle = button_center(&overlay, Button::Tool(Tool::Rectangle));
        click(&mut overlay, rectangle);
        assert_eq!(session(&overlay).tool, None);
        drag(&mut overlay, (200, 200), (220, 190));
        assert_eq!(
            session(&overlay).selection,
            Some(Rect::new(120, 90, 321, 241))
        );

        // The bottom-right handle resizes.
        drag(&mut overlay, (321, 241), (401, 301));
        assert_eq!(
            session(&overlay).selection,
            Some(Rect::new(120, 90, 401, 301))
        );

        // The arrow keys nudge it.
        key(&mut overlay, VK_LEFT.0);
        assert_eq!(
            session(&overlay).selection,
            Some(Rect::new(119, 90, 400, 301))
        );

        // The mouse wheel changes the stroke width.
        overlay.message(WM_MOUSEWHEEL, WPARAM((2 * 120) << 16), lparam(10, 10));
        assert_eq!(session(&overlay).width, 5.0);

        // Export: the selection's pixels.
        let (image, _) = overlay.export().unwrap().unwrap();
        assert_eq!((image.width, image.height), (281, 211));

        // The Save button asks the app to save; Esc to close.
        let save = button_center(&overlay, Button::Save);
        assert_eq!(click(&mut overlay, save).event, Some(Event::Save));
        assert_eq!(key(&mut overlay, VK_ESCAPE.0).event, Some(Event::Close));
        overlay.close();
        assert!(overlay.session.is_none());
    }

    #[test]
    fn double_click_inside_copies() {
        let mut overlay = overlay();
        drag(&mut overlay, (100, 100), (300, 250));
        let reply = overlay.message(WM_LBUTTONDBLCLK, WPARAM(1), lparam(200, 200));
        assert_eq!(reply.event, Some(Event::Copy));
        let reply = overlay.message(WM_LBUTTONDBLCLK, WPARAM(1), lparam(500, 500));
        assert_eq!(reply.event, None);
    }

    #[test]
    fn the_color_flyout_opens_picks_and_closes() {
        let mut overlay = overlay();
        drag(&mut overlay, (100, 100), (300, 250));
        let color = button_center(&overlay, Button::Color);
        click(&mut overlay, color);
        assert!(session(&overlay).flyout_open);
        let swatch = session(&overlay)
            .toolbars()
            .unwrap()
            .flyout
            .unwrap()
            .swatches[4];
        click(
            &mut overlay,
            (
                (swatch.left + swatch.width() / 2.0) as i32,
                (swatch.top + swatch.height() / 2.0) as i32,
            ),
        );
        assert_eq!(session(&overlay).color, 4);
        assert!(!session(&overlay).flyout_open);
        // Esc closes an open flyout before the overlay.
        click(&mut overlay, color);
        assert_eq!(key(&mut overlay, VK_ESCAPE.0).event, None);
        assert!(!session(&overlay).flyout_open);
    }
}
