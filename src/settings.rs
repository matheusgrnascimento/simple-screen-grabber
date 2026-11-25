//! The settings window, opened from the tray menu: the options of
//! `config.toml` as Windows 11 style cards, so the file need not be edited
//! by hand. Every change applies at once and rewrites only its own line of
//! the file.
//!
//! The window is drawn with Direct2D on a software render target (a
//! hardware one would keep tens of megabytes in the graphics driver for
//! the rest of the session), in DIPs, over Mica on Windows 11. [`Form`]
//! holds the state and the input logic, without a window, so it can be
//! tested.

use std::path::{Path, PathBuf};

use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows::Win32::Graphics::Dwm::{
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute, DWMWINDOWATTRIBUTE,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, InvalidateRect, MonitorFromPoint, ValidateRect, MONITORINFO,
    MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
    VK_CONTROL, VK_ESCAPE, VK_LEFT, VK_LWIN, VK_MENU, VK_RETURN, VK_RIGHT, VK_RWIN, VK_SHIFT,
    VK_SNAPSHOT, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName, FOS_FORCEFILESYSTEM,
    FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetClientRect, GetCursorPos, GetSystemMetrics, IsIconic,
    LoadCursorW, LoadImageW, RegisterClassExW, SetCursor, SetForegroundWindow, SetWindowPos,
    ShowWindow, HICON, HTCLIENT, IDC_ARROW, IDC_HAND, IMAGE_ICON, LR_DEFAULTSIZE, LR_SHARED,
    SM_CXSMICON, SM_CYSMICON, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE, SW_SHOWNORMAL, WA_INACTIVE,
    WINDOW_EX_STYLE, WM_ACTIVATE, WM_CHAR, WM_CLOSE, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN,
    WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_SIZE,
    WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW, WNDPROC, WS_CAPTION, WS_MINIMIZEBOX,
    WS_OVERLAPPED, WS_SYSMENU,
};

use crate::annotate::COLORS;
use crate::config::{Change, STROKE_RANGE};
use crate::fluent::{Color, Theme};
use crate::hotkey::Hotkey;
use crate::win;

const CLASS: PCWSTR = w!("SimpleScreenGrabber.Settings");
/// `WM_MOUSELEAVE`, from the common controls headers.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// `DWMWA_USE_IMMERSIVE_DARK_MODE` and `DWMWA_SYSTEMBACKDROP_TYPE`.
const DWMWA_DARK_MODE: i32 = 20;
const DWMWA_BACKDROP: i32 = 38;
/// `DWMSBT_MAINWINDOW`: Mica.
const BACKDROP_MICA: i32 = 2;

/// Client size and layout, in DIPs.
const WIDTH: f32 = 560.0;
const PAD: f32 = 24.0;
const CARDS_TOP: f32 = 76.0;
const CARD_HEIGHT: f32 = 68.0;
const CARD_GAP: f32 = 4.0;
const CARD_COUNT: usize = 5;
const LINK_TOP: f32 = CARDS_TOP + CARD_COUNT as f32 * (CARD_HEIGHT + CARD_GAP) + 8.0;
const HEIGHT: f32 = LINK_TOP + 20.0 + PAD;
/// The right edge of the controls in the cards.
const RIGHT: f32 = WIDTH - PAD - 16.0;
const SWATCH: f32 = 24.0;
const SWATCH_STEP: f32 = 32.0;

/// The values shown in the window.
#[derive(Clone, Debug, PartialEq)]
pub struct Values {
    pub hotkey: Hotkey,
    /// The configured folder; `None` for the Windows Screenshots folder.
    pub folder: Option<PathBuf>,
    /// The Windows Screenshots folder, to show when `folder` is `None`.
    pub default_folder: Option<PathBuf>,
    pub color: usize,
    pub width: i64,
    pub autostart: bool,
    /// Portable mode: Start with Windows is not offered.
    pub portable: bool,
}

/// What the app should do after a message.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Change(Change),
    /// The shortcut box starts (`true`) or stops listening for keys. While
    /// it listens, the global shortcut must be unregistered, or pressing it
    /// would start a capture instead of reaching the window.
    Recording(bool),
    /// Choose the screenshots folder (a modal dialog: run it with nothing
    /// borrowed).
    Browse,
    ToggleAutostart,
    OpenFile,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Hotkey,
    ResetFolder,
    Browse,
    Swatch(usize),
    Thinner,
    Thicker,
    Autostart,
    OpenFile,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    left: f32,
    top: f32,
    width: f32,
    height: f32,
}

impl Rect {
    const fn new(left: f32, top: f32, width: f32, height: f32) -> Rect {
        Rect {
            left,
            top,
            width,
            height,
        }
    }

    fn right(&self) -> f32 {
        self.left + self.width
    }

    fn bottom(&self) -> f32 {
        self.top + self.height
    }

    fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.left && x < self.right() && y >= self.top && y < self.bottom()
    }

    fn inset(&self, d: f32) -> Rect {
        Rect::new(
            self.left + d,
            self.top + d,
            self.width - 2.0 * d,
            self.height - 2.0 * d,
        )
    }

    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.left,
            top: self.top,
            right: self.right(),
            bottom: self.bottom(),
        }
    }
}

fn card(index: usize) -> Rect {
    Rect::new(
        PAD,
        CARDS_TOP + index as f32 * (CARD_HEIGHT + CARD_GAP),
        WIDTH - 2.0 * PAD,
        CARD_HEIGHT,
    )
}

/// The vertical center of a card.
fn middle(index: usize) -> f32 {
    let c = card(index);
    c.top + c.height / 2.0
}

/// The state of the window and its input logic.
#[derive(Debug)]
pub struct Form {
    values: Values,
    /// The shortcut box listens for keys.
    recording: bool,
    /// Why the last shortcut was not taken.
    error: Option<String>,
    hover: Option<Control>,
    pressed: Option<Control>,
    focus: Option<Control>,
    /// Focus rectangles show after keyboard use only, as in Windows.
    focus_visible: bool,
}

impl Form {
    pub fn new(values: Values) -> Form {
        Form {
            values,
            recording: false,
            error: None,
            hover: None,
            pressed: None,
            focus: None,
            focus_visible: false,
        }
    }

    /// The controls and their rectangles, in tab order.
    fn controls(&self) -> Vec<(Control, Rect)> {
        let mut list = vec![(
            Control::Hotkey,
            Rect::new(RIGHT - 200.0, middle(0) - 16.0, 200.0, 32.0),
        )];
        let browse = Rect::new(RIGHT - 110.0, middle(1) - 16.0, 110.0, 32.0);
        if self.values.folder.is_some() {
            list.push((
                Control::ResetFolder,
                Rect::new(browse.left - 8.0 - 90.0, browse.top, 90.0, 32.0),
            ));
        }
        list.push((Control::Browse, browse));
        let first = RIGHT - (COLORS.len() as f32 * SWATCH_STEP - (SWATCH_STEP - SWATCH));
        for i in 0..COLORS.len() {
            list.push((
                Control::Swatch(i),
                Rect::new(
                    first + i as f32 * SWATCH_STEP,
                    middle(2) - SWATCH / 2.0,
                    SWATCH,
                    SWATCH,
                ),
            ));
        }
        list.push((
            Control::Thinner,
            Rect::new(RIGHT - 32.0 - 56.0 - 32.0, middle(3) - 16.0, 32.0, 32.0),
        ));
        list.push((
            Control::Thicker,
            Rect::new(RIGHT - 32.0, middle(3) - 16.0, 32.0, 32.0),
        ));
        list.push((
            Control::Autostart,
            Rect::new(RIGHT - 84.0, middle(4) - 16.0, 84.0, 32.0),
        ));
        list.push((Control::OpenFile, Rect::new(PAD, LINK_TOP, 160.0, 20.0)));
        list
    }

    fn rect(&self, control: Control) -> Option<Rect> {
        self.controls()
            .into_iter()
            .find(|(c, _)| *c == control)
            .map(|(_, r)| r)
    }

    fn enabled(&self, control: Control) -> bool {
        match control {
            Control::Thinner => self.values.width > *STROKE_RANGE.start(),
            Control::Thicker => self.values.width < *STROKE_RANGE.end(),
            Control::Autostart => !self.values.portable,
            _ => true,
        }
    }

    fn hit(&self, x: f32, y: f32) -> Option<Control> {
        self.controls()
            .into_iter()
            .find(|(c, r)| r.contains(x, y) && self.enabled(*c))
            .map(|(c, _)| c)
    }

    /// Stops listening for keys, if listening.
    fn stop_recording(&mut self) -> Option<Action> {
        if self.recording {
            self.recording = false;
            self.error = None;
            Some(Action::Recording(false))
        } else {
            None
        }
    }

    fn activate(&mut self, control: Control) -> Option<Action> {
        if !self.enabled(control) {
            return None;
        }
        let v = &self.values;
        match control {
            Control::Hotkey => {
                if self.recording {
                    return self.stop_recording();
                }
                self.recording = true;
                self.error = None;
                Some(Action::Recording(true))
            }
            Control::ResetFolder => Some(Action::Change(Change::SaveFolder(None))),
            Control::Browse => Some(Action::Browse),
            Control::Swatch(i) if i != v.color => Some(Action::Change(Change::Color(i))),
            Control::Swatch(_) => None,
            Control::Thinner => Some(Action::Change(Change::StrokeWidth(v.width - 1))),
            Control::Thicker => Some(Action::Change(Change::StrokeWidth(v.width + 1))),
            Control::Autostart => Some(Action::ToggleAutostart),
            Control::OpenFile => Some(Action::OpenFile),
        }
    }

    /// Returns whether the hover changed.
    pub fn pointer_move(&mut self, x: f32, y: f32) -> bool {
        let hover = self.hit(x, y);
        let changed = hover != self.hover;
        self.hover = hover;
        changed
    }

    pub fn pointer_leave(&mut self) {
        self.hover = None;
    }

    pub fn pointer_down(&mut self, x: f32, y: f32) -> Option<Action> {
        self.pressed = self.hit(x, y);
        self.focus_visible = false;
        if self.pressed.is_some() {
            self.focus = self.pressed;
        }
        // A click anywhere but on the shortcut box stops listening.
        if self.pressed != Some(Control::Hotkey) {
            return self.stop_recording();
        }
        None
    }

    pub fn pointer_up(&mut self, x: f32, y: f32) -> Option<Action> {
        let pressed = self.pressed.take()?;
        if self.hit(x, y) == Some(pressed) {
            self.activate(pressed)
        } else {
            None
        }
    }

    /// A key press. Returns whether it was handled, and the action.
    pub fn key_down(&mut self, key: u16, mods: Modifiers) -> (bool, Option<Action>) {
        if self.recording {
            if key == VK_ESCAPE.0 {
                return (true, self.stop_recording());
            }
            return match Hotkey::from_keys(
                u32::from(key),
                mods.ctrl,
                mods.alt,
                mods.shift,
                mods.win,
            ) {
                None => (true, None),
                Some(Ok(hotkey)) => {
                    self.recording = false;
                    self.error = None;
                    if hotkey == self.values.hotkey {
                        (true, Some(Action::Recording(false)))
                    } else {
                        (true, Some(Action::Change(Change::Hotkey(hotkey))))
                    }
                }
                Some(Err(err)) => {
                    self.error = Some(err);
                    (true, None)
                }
            };
        }
        match key {
            k if k == VK_ESCAPE.0 => (true, Some(Action::Close)),
            k if k == VK_TAB.0 => {
                self.move_focus(if mods.shift { -1 } else { 1 });
                (true, None)
            }
            k if k == VK_RETURN.0 || k == VK_SPACE.0 => {
                let action = self.focus.and_then(|c| self.activate(c));
                (self.focus.is_some(), action)
            }
            k if k == VK_LEFT.0 || k == VK_RIGHT.0 => {
                let Some(Control::Swatch(i)) = self.focus else {
                    return (false, None);
                };
                let n = COLORS.len();
                let next = if k == VK_LEFT.0 {
                    (i + n - 1) % n
                } else {
                    (i + 1) % n
                };
                self.focus = Some(Control::Swatch(next));
                self.focus_visible = true;
                (true, self.activate(Control::Swatch(next)))
            }
            _ => (false, None),
        }
    }

    /// Moves the focus to the next (1) or previous (-1) enabled control.
    /// The color swatches are one stop, on the selected color, as in a
    /// radio group.
    fn move_focus(&mut self, step: isize) {
        let stops: Vec<Control> = self
            .controls()
            .into_iter()
            .map(|(c, _)| c)
            .filter(|c| self.enabled(*c))
            .filter(|c| !matches!(c, Control::Swatch(i) if *i != self.values.color))
            .collect();
        let current = self.focus.map(|f| match f {
            Control::Swatch(_) => Control::Swatch(self.values.color),
            other => other,
        });
        let index = current.and_then(|f| stops.iter().position(|c| *c == f));
        let n = stops.len() as isize;
        let next = match index {
            Some(i) => (i as isize + step).rem_euclid(n),
            None if step > 0 => 0,
            None => n - 1,
        };
        self.focus = Some(stops[next as usize]);
        self.focus_visible = true;
    }

    /// The window lost the focus.
    pub fn deactivated(&mut self) -> Option<Action> {
        self.pressed = None;
        self.stop_recording()
    }

    pub fn set_values(&mut self, values: Values) {
        self.values = values;
        // A control that went away (Default, when the folder is reset)
        // loses the focus.
        if let Some(focus) = self.focus {
            if self.rect(focus).is_none() || !self.enabled(focus) {
                self.focus = None;
            }
        }
    }

    /// The shortcut could not be registered.
    pub fn hotkey_failed(&mut self, message: String) {
        self.recording = false;
        self.error = Some(message);
    }
}

/// The modifier keys held during a key press.
#[derive(Clone, Copy, Debug, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
}

impl Modifiers {
    fn current() -> Modifiers {
        // SAFETY: reads this thread's keyboard state.
        let down = |vk: u16| unsafe { GetKeyState(i32::from(vk)) } < 0;
        Modifiers {
            ctrl: down(VK_CONTROL.0),
            alt: down(VK_MENU.0),
            shift: down(VK_SHIFT.0),
            win: down(VK_LWIN.0) || down(VK_RWIN.0),
        }
    }
}

/// Colors of the window, from the Windows 11 Settings app.
struct Colors {
    background: Color,
    card: Color,
    card_stroke: Color,
    text: Color,
    secondary: Color,
    disabled: Color,
    accent: Color,
    accent_hover: Color,
    on_accent: Color,
    control: Color,
    control_hover: Color,
    control_pressed: Color,
    control_stroke: Color,
    /// Unchecked toggle track and knob.
    strong_stroke: Color,
    swatch_ring: Color,
    critical: Color,
}

impl Colors {
    fn new(theme: &Theme) -> Colors {
        let p = &theme.palette;
        if theme.high_contrast {
            return Colors {
                background: p.flyout,
                card: p.flyout,
                card_stroke: p.text,
                text: p.text,
                secondary: p.text,
                disabled: p.text_disabled,
                accent: p.accent,
                accent_hover: p.accent,
                on_accent: p.on_accent,
                control: p.flyout,
                control_hover: p.accent,
                control_pressed: p.accent,
                control_stroke: p.text,
                strong_stroke: p.text,
                swatch_ring: p.text,
                critical: p.text,
            };
        }
        if p.dark {
            Colors {
                background: Color::rgb(0x20, 0x20, 0x20),
                card: Color::rgba(0xFF, 0xFF, 0xFF, 0x0D),
                card_stroke: Color::rgba(0x00, 0x00, 0x00, 0x19),
                text: p.text,
                secondary: p.text_secondary,
                disabled: p.text_disabled,
                accent: p.accent,
                accent_hover: Color::rgba(p.accent.r, p.accent.g, p.accent.b, 0xE6),
                on_accent: p.on_accent,
                control: Color::rgba(0xFF, 0xFF, 0xFF, 0x0F),
                control_hover: Color::rgba(0xFF, 0xFF, 0xFF, 0x15),
                control_pressed: Color::rgba(0xFF, 0xFF, 0xFF, 0x08),
                control_stroke: Color::rgba(0xFF, 0xFF, 0xFF, 0x12),
                strong_stroke: Color::rgba(0xFF, 0xFF, 0xFF, 0x8B),
                swatch_ring: p.swatch_ring,
                critical: Color::rgb(0xFF, 0x99, 0xA4),
            }
        } else {
            Colors {
                background: Color::rgb(0xF3, 0xF3, 0xF3),
                card: Color::rgba(0xFF, 0xFF, 0xFF, 0xB3),
                card_stroke: Color::rgba(0x00, 0x00, 0x00, 0x0F),
                text: p.text,
                secondary: p.text_secondary,
                disabled: p.text_disabled,
                accent: p.accent,
                accent_hover: Color::rgba(p.accent.r, p.accent.g, p.accent.b, 0xE6),
                on_accent: p.on_accent,
                control: Color::rgba(0xFF, 0xFF, 0xFF, 0xB3),
                control_hover: Color::rgba(0xF9, 0xF9, 0xF9, 0x80),
                control_pressed: Color::rgba(0xF9, 0xF9, 0xF9, 0x4D),
                control_stroke: Color::rgba(0x00, 0x00, 0x00, 0x14),
                strong_stroke: Color::rgba(0x00, 0x00, 0x00, 0x72),
                swatch_ring: p.swatch_ring,
                critical: Color::rgb(0xC4, 0x2B, 0x1C),
            }
        }
    }
}

fn d2d(c: Color) -> D2D1_COLOR_F {
    crate::gfx::color(c)
}

struct Formats {
    title: IDWriteTextFormat,
    body: IDWriteTextFormat,
    caption: IDWriteTextFormat,
    centered: IDWriteTextFormat,
    glyph: IDWriteTextFormat,
}

impl Formats {
    fn new() -> Result<Formats> {
        // SAFETY: plain DirectWrite factory calls with constant arguments.
        unsafe {
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
            let make = |family: PCWSTR,
                        weight: DWRITE_FONT_WEIGHT,
                        size: f32|
             -> Result<IDWriteTextFormat> {
                let f = dwrite.CreateTextFormat(
                    family,
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    w!("en-us"),
                )?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                let sign = dwrite.CreateEllipsisTrimmingSign(&f)?;
                let trimming = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                f.SetTrimming(&trimming, &sign)?;
                Ok(f)
            };
            let formats = Formats {
                title: make(display, DWRITE_FONT_WEIGHT_SEMI_BOLD, 28.0)?,
                body: make(text, DWRITE_FONT_WEIGHT_NORMAL, 14.0)?,
                caption: make(text, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?,
                centered: make(text, DWRITE_FONT_WEIGHT_NORMAL, 14.0)?,
                glyph: make(icons, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?,
            };
            for f in [&formats.centered, &formats.glyph] {
                f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            }
            Ok(formats)
        }
    }
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

/// The reply to a window message.
#[derive(Default)]
pub struct Reply {
    pub result: Option<LRESULT>,
    pub action: Option<Action>,
}

impl Reply {
    fn handled(action: Option<Action>) -> Reply {
        Reply {
            result: Some(LRESULT(0)),
            action,
        }
    }
}

/// The open settings window. Dropping it closes the window.
pub struct Settings {
    hwnd: HWND,
    form: Form,
    colors: Colors,
    mica: bool,
    factory: ID2D1Factory,
    target: Option<(ID2D1HwndRenderTarget, ID2D1SolidColorBrush)>,
    formats: Formats,
}

impl Settings {
    /// Creates and shows the window, centered on the monitor under the
    /// mouse.
    pub fn open(window_proc: WNDPROC, values: Values, theme: Theme) -> Result<Settings> {
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        // SAFETY: registers the class (again is harmless) and creates a
        // window of this thread; the icons are shared resources of the
        // executable.
        let hwnd = unsafe {
            let size = |x, y| (GetSystemMetrics(x), GetSystemMetrics(y));
            let big = LoadImageW(
                Some(win::instance()),
                win::resource_id(1),
                IMAGE_ICON,
                0,
                0,
                LR_DEFAULTSIZE | LR_SHARED,
            )
            .map(|h| HICON(h.0))
            .unwrap_or_default();
            let (sx, sy) = size(SM_CXSMICON, SM_CYSMICON);
            let small = LoadImageW(
                Some(win::instance()),
                win::resource_id(1),
                IMAGE_ICON,
                sx,
                sy,
                LR_SHARED,
            )
            .map(|h| HICON(h.0))
            .unwrap_or_default();
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: window_proc,
                hInstance: win::instance(),
                hIcon: big,
                hIconSm: small,
                lpszClassName: CLASS,
                ..Default::default()
            };
            RegisterClassExW(&class);
            let mut cursor = POINT::default();
            let _ = GetCursorPos(&mut cursor);
            let monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let work = info.rcWork;
            // Created on the target monitor first, to size it for its DPI.
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS,
                w!("Simple Screen Grabber settings"),
                style,
                work.left,
                work.top,
                100,
                100,
                None,
                None,
                Some(win::instance()),
                None,
            )?;
            let dpi = GetDpiForWindow(hwnd);
            let scale = dpi as f32 / 96.0;
            let mut r = RECT {
                left: 0,
                top: 0,
                right: (WIDTH * scale).round() as i32,
                bottom: (HEIGHT * scale).round() as i32,
            };
            let _ = AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE(0), dpi);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let x = work.left + ((work.right - work.left) - w) / 2;
            let y = work.top + ((work.bottom - work.top) - h) / 2;
            let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
            hwnd
        };
        let mut settings = Settings {
            hwnd,
            form: Form::new(values),
            colors: Colors::new(&theme),
            mica: false,
            // SAFETY: creates a factory owned by the window.
            factory: unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? },
            target: None,
            formats: Formats::new()?,
        };
        settings.apply_theme(&theme);
        // SAFETY: shows this thread's window.
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
            let _ = SetForegroundWindow(hwnd);
        }
        Ok(settings)
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Brings the open window to the front.
    pub fn show(&self) {
        // SAFETY: this thread's window.
        unsafe {
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    pub fn set_values(&mut self, values: Values) {
        self.form.set_values(values);
        self.invalidate();
    }

    pub fn hotkey_failed(&mut self, message: String) {
        self.form.hotkey_failed(message);
        self.invalidate();
    }

    pub fn set_theme(&mut self, theme: &Theme) {
        self.apply_theme(theme);
        self.invalidate();
    }

    /// The title bar theme, the backdrop and the colors.
    fn apply_theme(&mut self, theme: &Theme) {
        self.colors = Colors::new(theme);
        let dark = BOOL::from(theme.palette.dark);
        let backdrop = BACKDROP_MICA;
        // SAFETY: DWM attributes of this thread's window, with values of the
        // sizes they expect.
        unsafe {
            let _ = DwmSetWindowAttribute(
                self.hwnd,
                DWMWINDOWATTRIBUTE(DWMWA_DARK_MODE),
                (&dark as *const BOOL).cast(),
                4,
            );
            // Mica needs Windows 11 22H2; transparency effects off or high
            // contrast keep a solid background.
            let mica = theme.materials
                && DwmSetWindowAttribute(
                    self.hwnd,
                    DWMWINDOWATTRIBUTE(DWMWA_BACKDROP),
                    (&backdrop as *const i32).cast(),
                    4,
                )
                .is_ok();
            if mica != self.mica {
                let extend = if mica { -1 } else { 0 };
                let margins = MARGINS {
                    cxLeftWidth: extend,
                    cxRightWidth: extend,
                    cyTopHeight: extend,
                    cyBottomHeight: extend,
                };
                let _ = DwmExtendFrameIntoClientArea(self.hwnd, &margins);
                if !mica {
                    let none = 1i32; // DWMSBT_NONE
                    let _ = DwmSetWindowAttribute(
                        self.hwnd,
                        DWMWINDOWATTRIBUTE(DWMWA_BACKDROP),
                        (&none as *const i32).cast(),
                        4,
                    );
                }
                self.mica = mica;
            }
        }
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

    pub fn message(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Reply {
        let scale = self.scale();
        let point = || {
            (
                win::x_of(lparam.0) as f32 / scale,
                win::y_of(lparam.0) as f32 / scale,
            )
        };
        match msg {
            WM_PAINT => {
                // SAFETY: this thread's window.
                unsafe {
                    let _ = ValidateRect(Some(self.hwnd), None);
                }
                if let Err(err) = self.render() {
                    crate::log!("settings window not drawn: {err}");
                }
                Reply::handled(None)
            }
            WM_ERASEBKGND => Reply {
                result: Some(LRESULT(1)),
                action: None,
            },
            WM_SIZE => {
                if let Some((target, _)) = &self.target {
                    let (w, h) = (
                        win::low_word(lparam.0 as usize),
                        win::low_word((lparam.0 as usize) >> 16),
                    );
                    // SAFETY: resizes this window's target.
                    unsafe {
                        let _ = target.Resize(&D2D_SIZE_U {
                            width: w,
                            height: h,
                        });
                    }
                }
                Reply::handled(None)
            }
            WM_DPICHANGED => {
                // SAFETY: the suggested rectangle that comes with the message.
                unsafe {
                    let r = &*(lparam.0 as *const RECT);
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        r.left,
                        r.top,
                        r.right - r.left,
                        r.bottom - r.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                self.target = None;
                self.invalidate();
                Reply::handled(None)
            }
            WM_MOUSEMOVE => {
                let (x, y) = point();
                if self.form.pointer_move(x, y) {
                    self.invalidate();
                }
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: self.hwnd,
                    dwHoverTime: 0,
                };
                // SAFETY: an initialized structure for this window.
                unsafe {
                    let _ = TrackMouseEvent(&mut track);
                }
                Reply::handled(None)
            }
            WM_MOUSELEAVE => {
                self.form.pointer_leave();
                self.invalidate();
                Reply::handled(None)
            }
            WM_SETCURSOR if win::low_word(lparam.0 as usize) == HTCLIENT => {
                let id = if self.form.hover == Some(Control::OpenFile) {
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
                Reply {
                    result: Some(LRESULT(1)),
                    action: None,
                }
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point();
                let action = self.form.pointer_down(x, y);
                // SAFETY: this thread's window.
                unsafe { SetCapture(self.hwnd) };
                self.invalidate();
                Reply::handled(action)
            }
            WM_LBUTTONUP => {
                // SAFETY: releases the capture taken above.
                unsafe {
                    let _ = ReleaseCapture();
                }
                let (x, y) = point();
                let action = self.form.pointer_up(x, y);
                self.invalidate();
                Reply::handled(action)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let (handled, action) = self.form.key_down(wparam.0 as u16, Modifiers::current());
                self.invalidate();
                // While listening, Alt and F10 must not open the window menu.
                if handled || self.form.recording || action.is_some() {
                    Reply::handled(action)
                } else {
                    Reply::default()
                }
            }
            // Print Screen arrives as a key release only.
            WM_KEYUP | WM_SYSKEYUP if self.form.recording && wparam.0 as u16 == VK_SNAPSHOT.0 => {
                let (_, action) = self.form.key_down(VK_SNAPSHOT.0, Modifiers::current());
                self.invalidate();
                Reply::handled(action)
            }
            WM_SYSKEYUP | WM_SYSCHAR | WM_CHAR if self.form.recording => Reply::handled(None),
            WM_ACTIVATE if win::low_word(wparam.0) == WA_INACTIVE => {
                let action = self.form.deactivated();
                self.invalidate();
                Reply {
                    result: None,
                    action,
                }
            }
            WM_CLOSE => Reply::handled(Some(Action::Close)),
            _ => Reply::default(),
        }
    }

    fn render(&mut self) -> Result<()> {
        if self.target.is_none() {
            let scale = self.scale();
            let mut client = RECT::default();
            // SAFETY: plain query; the target is owned by this window.
            unsafe {
                GetClientRect(self.hwnd, &mut client)?;
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
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
                let brush = target.CreateSolidColorBrush(&d2d(self.colors.text), None)?;
                self.target = Some((target, brush));
            }
        }
        let Some((target, brush)) = &self.target else {
            return Ok(());
        };
        let painter = Painter {
            target,
            brush,
            formats: &self.formats,
        };
        // SAFETY: one BeginDraw/EndDraw pair on this thread's target.
        unsafe { target.BeginDraw() };
        draw(&painter, &self.form, &self.colors, self.mica);
        // SAFETY: closes the BeginDraw above.
        if unsafe { target.EndDraw(None, None) }.is_err() {
            // The target is recreated on the next paint.
            self.target = None;
        }
        Ok(())
    }
}

impl Drop for Settings {
    fn drop(&mut self) {
        self.target = None;
        // SAFETY: the window belongs to this thread and is not used after.
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

/// Drawing helpers over one render target.
struct Painter<'a> {
    target: &'a ID2D1HwndRenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    formats: &'a Formats,
}

impl Painter<'_> {
    fn fill(&self, r: Rect, color: Color, radius: f32) {
        // SAFETY: drawing with resources of this target, inside BeginDraw.
        unsafe {
            self.brush.SetColor(&d2d(color));
            self.target.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: r.d2d(),
                    radiusX: radius,
                    radiusY: radius,
                },
                self.brush,
            );
        }
    }

    fn stroke(&self, r: Rect, color: Color, radius: f32, width: f32) {
        let r = r.inset(width / 2.0);
        // SAFETY: as above.
        unsafe {
            self.brush.SetColor(&d2d(color));
            self.target.DrawRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: r.d2d(),
                    radiusX: radius,
                    radiusY: radius,
                },
                self.brush,
                width,
                None,
            );
        }
    }

    fn disc(&self, x: f32, y: f32, radius: f32, color: Color) {
        // SAFETY: as above.
        unsafe {
            self.brush.SetColor(&d2d(color));
            self.target.FillEllipse(&ellipse(x, y, radius), self.brush);
        }
    }

    fn ring(&self, x: f32, y: f32, radius: f32, color: Color, width: f32) {
        // SAFETY: as above.
        unsafe {
            self.brush.SetColor(&d2d(color));
            self.target
                .DrawEllipse(&ellipse(x, y, radius), self.brush, width, None);
        }
    }

    fn text(&self, s: &str, format: &IDWriteTextFormat, r: Rect, color: Color) {
        let wide: Vec<u16> = s.encode_utf16().collect();
        // SAFETY: as above.
        unsafe {
            self.brush.SetColor(&d2d(color));
            self.target.DrawText(
                &wide,
                format,
                &r.d2d(),
                self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// A standard button: fill by state, then a 1-DIP border.
    fn button(&self, r: Rect, c: &Colors, state: State, border: Color) {
        let fill = match state {
            State::Pressed => c.control_pressed,
            State::Hover => c.control_hover,
            State::Normal | State::Disabled => c.control,
        };
        self.fill(r, fill, 4.0);
        self.stroke(r, border, 4.0, 1.0);
    }
}

fn ellipse(x: f32, y: f32, radius: f32) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: windows_numerics::Vector2 { X: x, Y: y },
        radiusX: radius,
        radiusY: radius,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    Hover,
    Pressed,
    Disabled,
}

fn state(form: &Form, control: Control) -> State {
    if !form.enabled(control) {
        State::Disabled
    } else if form.pressed == Some(control) && form.hover == Some(control) {
        State::Pressed
    } else if form.hover == Some(control) && form.pressed.is_none() {
        State::Hover
    } else {
        State::Normal
    }
}

/// The folder as shown: the configured one, or the Windows one.
fn folder_text(values: &Values) -> String {
    match (&values.folder, &values.default_folder) {
        (Some(folder), _) => folder.display().to_string(),
        (None, Some(default)) => format!("{} (default)", default.display()),
        (None, None) => "Pictures\\Screenshots (default)".into(),
    }
}

fn draw(p: &Painter, form: &Form, c: &Colors, mica: bool) {
    let v = &form.values;
    let f = p.formats;
    // SAFETY: inside the BeginDraw of `p.target`.
    unsafe {
        let background = if mica {
            Color::rgba(0, 0, 0, 0)
        } else {
            c.background
        };
        p.target.Clear(Some(&d2d(background)));
    }
    p.text(
        "Settings",
        &f.title,
        Rect::new(PAD, 20.0, WIDTH - 2.0 * PAD, 40.0),
        c.text,
    );

    // The cards: title and description on the left, the control on the
    // right.
    let rows: [(&str, String, Option<Color>); CARD_COUNT] = [
        (
            "Capture shortcut",
            match (&form.error, form.recording) {
                (Some(err), _) => err.clone(),
                (None, true) => "Press the keys, or Esc to cancel".into(),
                (None, false) => "Starts a capture from any app".into(),
            },
            form.error.as_ref().map(|_| c.critical),
        ),
        ("Screenshots folder", folder_text(v), None),
        (
            "Default color",
            COLORS[v.color.min(COLORS.len() - 1)].1.to_string(),
            None,
        ),
        (
            "Stroke width",
            "For new annotations, at 100% display scale".into(),
            None,
        ),
        (
            "Start with Windows",
            if v.portable {
                "Not available in portable mode".into()
            } else {
                "Waits in the notification area after you sign in".into()
            },
            None,
        ),
    ];
    let lefts: [f32; CARD_COUNT] = [
        form.rect(Control::Hotkey).map_or(RIGHT, |r| r.left),
        form.rect(Control::ResetFolder)
            .or(form.rect(Control::Browse))
            .map_or(RIGHT, |r| r.left),
        form.rect(Control::Swatch(0)).map_or(RIGHT, |r| r.left),
        form.rect(Control::Thinner).map_or(RIGHT, |r| r.left),
        form.rect(Control::Autostart).map_or(RIGHT, |r| r.left),
    ];
    for (i, (title, description, tint)) in rows.iter().enumerate() {
        let r = card(i);
        p.fill(r, c.card, 4.0);
        p.stroke(r, c.card_stroke, 4.0, 1.0);
        let width = lefts[i] - 16.0 - (r.left + 16.0);
        let disabled = i == 4 && v.portable;
        let title_color = if disabled { c.disabled } else { c.text };
        p.text(
            title,
            &f.body,
            Rect::new(r.left + 16.0, r.top + 12.0, width, 22.0),
            title_color,
        );
        p.text(
            description,
            &f.caption,
            Rect::new(r.left + 16.0, r.top + 34.0, width, 20.0),
            tint.unwrap_or(c.secondary),
        );
    }

    // Shortcut box.
    if let Some(r) = form.rect(Control::Hotkey) {
        let s = state(form, Control::Hotkey);
        if form.recording {
            p.button(r, c, s, c.accent);
            p.stroke(r, c.accent, 4.0, 2.0);
            p.text("Press a shortcut\u{2026}", &f.centered, r, c.secondary);
        } else {
            p.button(r, c, s, c.control_stroke);
            p.text(&v.hotkey.to_string(), &f.centered, r, c.text);
        }
    }

    // Folder buttons.
    for (control, label) in [
        (Control::ResetFolder, "Default"),
        (Control::Browse, "Browse\u{2026}"),
    ] {
        if let Some(r) = form.rect(control) {
            p.button(r, c, state(form, control), c.control_stroke);
            p.text(label, &f.centered, r, c.text);
        }
    }

    // Color swatches.
    for (i, (color, _)) in COLORS.iter().enumerate() {
        let Some(r) = form.rect(Control::Swatch(i)) else {
            continue;
        };
        let (x, y) = (r.left + r.width / 2.0, r.top + r.height / 2.0);
        let radius = SWATCH / 2.0 - 2.0;
        if i == v.color {
            p.ring(x, y, radius + 3.0, c.accent, 2.0);
        } else if state(form, Control::Swatch(i)) == State::Hover {
            p.ring(x, y, radius + 3.0, c.secondary, 1.0);
        }
        p.disc(x, y, radius, *color);
        p.ring(x, y, radius, c.swatch_ring, 1.0);
    }

    // Stroke width stepper.
    for (control, glyph) in [
        (Control::Thinner, "\u{E738}"),
        (Control::Thicker, "\u{E710}"),
    ] {
        if let Some(r) = form.rect(control) {
            let s = state(form, control);
            p.button(r, c, s, c.control_stroke);
            let color = if s == State::Disabled {
                c.disabled
            } else {
                c.text
            };
            p.text(glyph, &f.glyph, r, color);
        }
    }
    if let (Some(thinner), Some(thicker)) =
        (form.rect(Control::Thinner), form.rect(Control::Thicker))
    {
        let r = Rect::new(
            thinner.right(),
            thinner.top,
            thicker.left - thinner.right(),
            thinner.height,
        );
        p.text(&format!("{} px", v.width), &f.centered, r, c.text);
    }

    // Start with Windows toggle: "On"/"Off", then the switch.
    if let Some(r) = form.rect(Control::Autostart) {
        let s = state(form, Control::Autostart);
        let on = v.autostart && !v.portable;
        let track = Rect::new(r.right() - 40.0, r.top + 6.0, 40.0, 20.0);
        let (cy, knob) = (track.top + 10.0, if s == State::Hover { 7.0 } else { 6.0 });
        let label_color = if s == State::Disabled {
            c.disabled
        } else {
            c.text
        };
        p.text(
            if on { "On" } else { "Off" },
            &f.body,
            Rect::new(r.left, r.top, track.left - 12.0 - r.left, r.height),
            label_color,
        );
        if s == State::Disabled {
            p.stroke(track, c.disabled, 10.0, 1.0);
            p.disc(track.left + 10.0, cy, 6.0, c.disabled);
        } else if on {
            let fill = if s == State::Hover {
                c.accent_hover
            } else {
                c.accent
            };
            p.fill(track, fill, 10.0);
            p.disc(track.right() - 10.0, cy, knob, c.on_accent);
        } else {
            if s == State::Hover {
                p.fill(track, c.control_hover, 10.0);
            }
            p.stroke(track, c.strong_stroke, 10.0, 1.0);
            p.disc(track.left + 10.0, cy, knob, c.strong_stroke);
        }
    }

    // Link to the file, for the options not shown here.
    if let Some(r) = form.rect(Control::OpenFile) {
        let color = if state(form, Control::OpenFile) == State::Hover {
            c.accent_hover
        } else {
            c.accent
        };
        p.text("Edit the settings file", &f.caption, r, color);
    }

    // Keyboard focus.
    if form.focus_visible {
        if let Some(r) = form.focus.and_then(|focus| form.rect(focus)) {
            let radius = if matches!(form.focus, Some(Control::Swatch(_))) {
                r.width / 2.0 + 5.0
            } else {
                7.0
            };
            p.stroke(r.inset(-5.0), c.text, radius, 2.0);
        }
    }
}

/// Asks for the screenshots folder, starting at `current`. `None` when
/// the user cancels. Modal: call it with nothing borrowed.
pub fn pick_folder(owner: HWND, current: Option<&Path>) -> Option<PathBuf> {
    // SAFETY: COM calls on a dialog created here; the returned path string
    // is copied, then freed with CoTaskMemFree as the API requires.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog
            .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)
            .ok()?;
        let _ = dialog.SetTitle(w!("Choose the screenshots folder"));
        if let Some(current) = current {
            let path = win::wide(&current.to_string_lossy());
            let item: Result<IShellItem> = SHCreateItemFromParsingName(win::pcwstr(&path), None);
            if let Ok(item) = item {
                let _ = dialog.SetFolder(&item);
            }
        }
        dialog.Show(Some(owner)).ok()?;
        let item = dialog.GetResult().ok()?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = name.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(name.0.cast_const().cast()));
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values() -> Values {
        Values {
            hotkey: Hotkey::DEFAULT,
            folder: None,
            default_folder: Some(PathBuf::from(r"C:\Users\me\Pictures\Screenshots")),
            color: 4,
            width: 3,
            autostart: false,
            portable: false,
        }
    }

    fn center(form: &Form, control: Control) -> (f32, f32) {
        let r = form.rect(control).expect("the control is shown");
        (r.left + r.width / 2.0, r.top + r.height / 2.0)
    }

    fn click(form: &mut Form, control: Control) -> Option<Action> {
        let (x, y) = center(form, control);
        form.pointer_move(x, y);
        let down = form.pointer_down(x, y);
        let up = form.pointer_up(x, y);
        up.or(down)
    }

    const NONE: Modifiers = Modifiers {
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
    };

    #[test]
    fn the_layout_fits_and_does_not_overlap() {
        let mut v = values();
        v.folder = Some(PathBuf::from(r"D:\Shots"));
        let form = Form::new(v);
        let controls = form.controls();
        for (i, (a, ra)) in controls.iter().enumerate() {
            assert!(
                ra.left >= 0.0 && ra.right() <= WIDTH && ra.bottom() <= HEIGHT,
                "{a:?}"
            );
            for (b, rb) in &controls[i + 1..] {
                let apart = ra.right() <= rb.left
                    || rb.right() <= ra.left
                    || ra.bottom() <= rb.top
                    || rb.bottom() <= ra.top;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn clicks_change_the_settings() {
        let mut form = Form::new(values());
        assert_eq!(
            click(&mut form, Control::Swatch(0)),
            Some(Action::Change(Change::Color(0)))
        );
        // The selected color is not a change.
        assert_eq!(click(&mut form, Control::Swatch(4)), None);
        assert_eq!(
            click(&mut form, Control::Thicker),
            Some(Action::Change(Change::StrokeWidth(4)))
        );
        assert_eq!(
            click(&mut form, Control::Thinner),
            Some(Action::Change(Change::StrokeWidth(2)))
        );
        assert_eq!(click(&mut form, Control::Browse), Some(Action::Browse));
        assert_eq!(
            click(&mut form, Control::Autostart),
            Some(Action::ToggleAutostart)
        );
        assert_eq!(click(&mut form, Control::OpenFile), Some(Action::OpenFile));
        // No Default button for the default folder.
        assert!(form.rect(Control::ResetFolder).is_none());
        let mut v = values();
        v.folder = Some(PathBuf::from(r"D:\Shots"));
        form.set_values(v);
        assert_eq!(
            click(&mut form, Control::ResetFolder),
            Some(Action::Change(Change::SaveFolder(None)))
        );
    }

    #[test]
    fn a_press_released_elsewhere_does_nothing() {
        let mut form = Form::new(values());
        let (x, y) = center(&form, Control::Thicker);
        form.pointer_down(x, y);
        assert_eq!(form.pointer_up(10.0, 10.0), None);
    }

    #[test]
    fn limits_and_portable_mode_disable_controls() {
        let mut v = values();
        v.width = *STROKE_RANGE.end();
        v.portable = true;
        let mut form = Form::new(v);
        assert_eq!(click(&mut form, Control::Thicker), None);
        assert_eq!(click(&mut form, Control::Autostart), None);
        assert!(click(&mut form, Control::Thinner).is_some());
    }

    #[test]
    fn the_shortcut_box_records_keys() {
        let mut form = Form::new(values());
        assert_eq!(
            click(&mut form, Control::Hotkey),
            Some(Action::Recording(true))
        );
        assert!(form.recording);
        // Modifiers alone wait for the key.
        let ctrl_shift = Modifiers {
            ctrl: true,
            shift: true,
            ..NONE
        };
        assert_eq!(form.key_down(VK_CONTROL.0, ctrl_shift), (true, None));
        // A letter alone is refused and listening goes on.
        assert_eq!(form.key_down(u16::from(b'X'), NONE), (true, None));
        assert!(form.error.is_some() && form.recording);
        let (_, action) = form.key_down(u16::from(b'X'), ctrl_shift);
        assert_eq!(
            action,
            Some(Action::Change(Change::Hotkey(
                Hotkey::parse("Ctrl+Shift+X").unwrap()
            )))
        );
        assert!(!form.recording && form.error.is_none());
    }

    #[test]
    fn recording_stops_on_escape_click_or_focus_loss() {
        let mut form = Form::new(values());
        click(&mut form, Control::Hotkey);
        assert_eq!(
            form.key_down(VK_ESCAPE.0, NONE),
            (true, Some(Action::Recording(false)))
        );
        // Esc when not listening closes the window.
        assert_eq!(
            form.key_down(VK_ESCAPE.0, NONE),
            (true, Some(Action::Close))
        );

        click(&mut form, Control::Hotkey);
        let (x, y) = center(&form, Control::Thicker);
        assert_eq!(form.pointer_down(x, y), Some(Action::Recording(false)));
        form.pointer_up(x, y);

        click(&mut form, Control::Hotkey);
        assert_eq!(form.deactivated(), Some(Action::Recording(false)));
        assert_eq!(form.deactivated(), None);

        // The same shortcut again just stops listening.
        click(&mut form, Control::Hotkey);
        let ctrl_alt = Modifiers {
            ctrl: true,
            alt: true,
            ..NONE
        };
        assert_eq!(
            form.key_down(u16::from(b'S'), ctrl_alt),
            (true, Some(Action::Recording(false)))
        );
    }

    #[test]
    fn the_keyboard_moves_the_focus_and_activates() {
        let mut form = Form::new(values());
        form.key_down(VK_TAB.0, NONE);
        assert_eq!(form.focus, Some(Control::Hotkey));
        form.key_down(VK_TAB.0, NONE);
        assert_eq!(form.focus, Some(Control::Browse));
        // The swatches are one stop, on the selected color.
        form.key_down(VK_TAB.0, NONE);
        assert_eq!(form.focus, Some(Control::Swatch(4)));
        let (_, action) = form.key_down(VK_RIGHT.0, NONE);
        assert_eq!(action, Some(Action::Change(Change::Color(5))));
        // Shift+Tab goes back.
        let shift = Modifiers {
            shift: true,
            ..NONE
        };
        form.key_down(VK_TAB.0, shift);
        assert_eq!(form.focus, Some(Control::Browse));
        assert_eq!(
            form.key_down(VK_SPACE.0, NONE),
            (true, Some(Action::Browse))
        );
        // Backwards from the start wraps to the link.
        let mut form = Form::new(values());
        form.key_down(VK_TAB.0, shift);
        assert_eq!(form.focus, Some(Control::OpenFile));
    }

    #[test]
    fn a_failed_shortcut_shows_why() {
        let mut form = Form::new(values());
        click(&mut form, Control::Hotkey);
        form.hotkey_failed("Ctrl+Alt+X is used by another app".into());
        assert!(!form.recording);
        assert_eq!(
            form.error.as_deref(),
            Some("Ctrl+Alt+X is used by another app")
        );
    }

    #[test]
    fn the_default_folder_is_labelled() {
        let mut v = values();
        assert!(folder_text(&v).ends_with("Screenshots (default)"));
        v.folder = Some(PathBuf::from(r"D:\Shots"));
        assert_eq!(folder_text(&v), r"D:\Shots");
    }
}
