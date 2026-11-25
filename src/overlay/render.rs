//! Draws the overlay with Direct2D: the frozen screen, the dimmed area
//! outside the selection, the selection border, its handles and the size
//! label. 1 DIP is 1 physical pixel; UI sizes are scaled per monitor.

use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::{
    ID2D1DeviceContext, ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
    D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_METRICS,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::selection;
use super::{layout, Drag, Session};
use crate::fluent::{Color, Palette};
use crate::geometry::{Rect, RectF};
use crate::gfx;

/// UI metrics in DIPs (multiplied by the monitor scale).
pub mod metrics {
    /// Radius of the outer disc of a selection handle.
    pub const HANDLE_RADIUS: f32 = 7.0;
    /// Radius of the accent dot inside a handle: at rest, hovered, pressed.
    pub const HANDLE_DOT: f32 = 4.0;
    pub const HANDLE_DOT_HOVER: f32 = 5.0;
    pub const HANDLE_DOT_PRESSED: f32 = 3.5;
    /// Distance from a handle center that still grabs it.
    pub const HANDLE_HIT: f32 = 12.0;
    pub const LABEL_FONT: f32 = 12.0;
    pub const LABEL_HEIGHT: f32 = 24.0;
    pub const LABEL_PADDING: f32 = 8.0;
    pub const CORNER_RADIUS: f32 = 4.0;
    pub const GAP: f32 = 8.0;
}

/// Brushes, fonts and text layouts reused across frames.
pub struct Painter {
    brush: ID2D1SolidColorBrush,
    dwrite: IDWriteFactory,
    /// Segoe UI Variable (Windows 11) is installed; otherwise Segoe UI.
    variable_font: bool,
    label_format: Option<(f32, IDWriteTextFormat)>,
    label: Option<(String, f32, IDWriteTextLayout, DWRITE_TEXT_METRICS)>,
}

/// Segoe UI Variable (Windows 11), or Segoe UI (Windows 10).
fn text_family(variable_font: bool) -> PCWSTR {
    if variable_font {
        w!("Segoe UI Variable Text")
    } else {
        w!("Segoe UI")
    }
}

impl Painter {
    pub fn new(ctx: &ID2D1DeviceContext, dwrite: &IDWriteFactory) -> Result<Painter> {
        // SAFETY: COM calls on valid objects; out pointers are locals.
        unsafe {
            let brush = ctx.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)?;
            let mut collection = None;
            dwrite.GetSystemFontCollection(&mut collection, false)?;
            let mut variable_font = false;
            if let Some(collection) = collection {
                let (mut index, mut exists) = (0, BOOL(0));
                collection.FindFamilyName(text_family(true), &mut index, &mut exists)?;
                variable_font = exists.as_bool();
            }
            Ok(Painter {
                brush,
                dwrite: dwrite.clone(),
                variable_font,
                label_format: None,
                label: None,
            })
        }
    }

    fn brush(&self, c: Color) -> &ID2D1SolidColorBrush {
        // SAFETY: sets the color of a brush owned by this painter.
        unsafe { self.brush.SetColor(&gfx::color(c)) };
        &self.brush
    }

    fn fill(&self, ctx: &ID2D1DeviceContext, r: RectF, c: Color) {
        // SAFETY: draws with a brush of the same device context.
        unsafe {
            ctx.FillRectangle(
                &gfx::rect_f(r.left, r.top, r.right, r.bottom),
                self.brush(c),
            )
        };
    }

    fn rounded(r: RectF, radius: f32) -> D2D1_ROUNDED_RECT {
        D2D1_ROUNDED_RECT {
            rect: gfx::rect_f(r.left, r.top, r.right, r.bottom),
            radiusX: radius,
            radiusY: radius,
        }
    }

    /// Draws one frame of the overlay.
    pub fn frame(
        &mut self,
        ctx: &ID2D1DeviceContext,
        session: &Session,
        palette: &Palette,
    ) -> Result<()> {
        // SAFETY: draws between BeginDraw and EndDraw of the caller with
        // resources of the same device context.
        unsafe {
            ctx.SetTransform(&Matrix3x2::identity());
            ctx.DrawBitmap(
                &session.bitmap,
                None,
                1.0,
                D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
                None,
                None,
            );
        }
        let bounds = session.capture.bounds();
        let Some(sel) = session.selection else {
            self.fill(ctx, bounds.into(), palette.scrim);
            return Ok(());
        };
        self.scrim(ctx, bounds, sel, palette.scrim);
        self.border(ctx, session, sel, palette.accent);
        self.handles(ctx, session, sel, palette);
        self.size_label(ctx, session, sel, palette)
    }

    /// Dims everything outside the selection, without overdraw.
    fn scrim(&self, ctx: &ID2D1DeviceContext, bounds: Rect, sel: Rect, c: Color) {
        let parts = [
            Rect::new(bounds.left, bounds.top, bounds.right, sel.top),
            Rect::new(bounds.left, sel.bottom, bounds.right, bounds.bottom),
            Rect::new(bounds.left, sel.top, sel.left, sel.bottom),
            Rect::new(sel.right, sel.top, bounds.right, sel.bottom),
        ];
        for part in parts.iter().filter(|r| !r.is_empty()) {
            self.fill(ctx, (*part).into(), c);
        }
    }

    /// A border just outside the selection, so it never covers a selected
    /// pixel.
    fn border(&self, ctx: &ID2D1DeviceContext, session: &Session, sel: Rect, c: Color) {
        let scale = session.scale_at(sel.left, sel.top);
        let w = scale.round().max(1.0) as i32;
        let parts = [
            Rect::new(sel.left - w, sel.top - w, sel.right + w, sel.top),
            Rect::new(sel.left - w, sel.bottom, sel.right + w, sel.bottom + w),
            Rect::new(sel.left - w, sel.top, sel.left, sel.bottom),
            Rect::new(sel.right, sel.top, sel.right + w, sel.bottom),
        ];
        // SAFETY: plain state change of the device context.
        unsafe { ctx.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED) };
        for part in parts {
            self.fill(ctx, part.into(), c);
        }
        // SAFETY: as above.
        unsafe { ctx.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
    }

    /// Windows 11 slider-thumb handles: a disc with an accent dot.
    fn handles(&self, ctx: &ID2D1DeviceContext, session: &Session, sel: Rect, palette: &Palette) {
        let pressed = match session.drag {
            Drag::Resizing { handle, .. } => Some(handle),
            _ => None,
        };
        let diameter = 2.0 * metrics::HANDLE_RADIUS * session.scale_at(sel.left, sel.top);
        for &handle in selection::visible_handles(sel, diameter) {
            let (x, y) = selection::handle_center(sel, handle);
            let scale = session.scale_at(x as i32, y as i32);
            let dot = if pressed == Some(handle) {
                metrics::HANDLE_DOT_PRESSED
            } else if session.hover == Some(handle) && pressed.is_none() {
                metrics::HANDLE_DOT_HOVER
            } else {
                metrics::HANDLE_DOT
            };
            let center = Vector2 { X: x, Y: y };
            let disc = |radius: f32| D2D1_ELLIPSE {
                point: center,
                radiusX: radius * scale,
                radiusY: radius * scale,
            };
            // SAFETY: draws with a brush of the same device context.
            unsafe {
                ctx.FillEllipse(
                    &disc(metrics::HANDLE_RADIUS + 1.0),
                    self.brush(palette.handle_border),
                );
                ctx.FillEllipse(&disc(metrics::HANDLE_RADIUS), self.brush(palette.handle));
                ctx.FillEllipse(&disc(dot), self.brush(palette.accent));
            }
        }
    }

    fn size_label(
        &mut self,
        ctx: &ID2D1DeviceContext,
        session: &Session,
        sel: Rect,
        palette: &Palette,
    ) -> Result<()> {
        let monitor = session
            .capture
            .monitor_at(crate::geometry::Point::new(sel.left, sel.top));
        let scale = monitor.scale;
        let text = format!("{} \u{00D7} {}", sel.width(), sel.height());
        let (layout, metrics_text) = self.label_layout(&text, scale)?;
        let width = (metrics_text.width + 2.0 * metrics::LABEL_PADDING * scale).ceil();
        let height = (metrics::LABEL_HEIGHT * scale).round();
        let r = layout::size_label(sel, monitor.rect, width, height, metrics::GAP * scale);
        let radius = metrics::CORNER_RADIUS * scale;
        let stroke = scale.round().max(1.0);
        // SAFETY: draws with resources of the same device context.
        unsafe {
            ctx.FillRoundedRectangle(&Self::rounded(r, radius), self.brush(palette.flyout));
            let inset = r.offset(0.0, 0.0);
            let half = stroke / 2.0;
            ctx.DrawRoundedRectangle(
                &Self::rounded(
                    RectF::new(
                        inset.left + half,
                        inset.top + half,
                        inset.right - half,
                        inset.bottom - half,
                    ),
                    radius,
                ),
                self.brush(palette.flyout_border),
                stroke,
                None,
            );
            let origin = Vector2 {
                X: r.left + ((r.width() - metrics_text.width) / 2.0).round(),
                Y: r.top + ((r.height() - metrics_text.height) / 2.0).round(),
            };
            ctx.DrawTextLayout(
                origin,
                &layout,
                self.brush(palette.text),
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            );
        }
        Ok(())
    }

    /// The text layout of the size label, reused while the text and scale
    /// stay the same.
    fn label_layout(
        &mut self,
        text: &str,
        scale: f32,
    ) -> Result<(IDWriteTextLayout, DWRITE_TEXT_METRICS)> {
        if let Some((t, s, layout, m)) = &self.label {
            if t == text && *s == scale {
                return Ok((layout.clone(), *m));
            }
        }
        let format = match &self.label_format {
            Some((s, f)) if *s == scale => f.clone(),
            _ => {
                // SAFETY: creates a text format with constant arguments.
                let f = unsafe {
                    self.dwrite.CreateTextFormat(
                        text_family(self.variable_font),
                        None,
                        DWRITE_FONT_WEIGHT_NORMAL,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        metrics::LABEL_FONT * scale,
                        w!("en-us"),
                    )?
                };
                self.label_format = Some((scale, f.clone()));
                f
            }
        };
        let wide: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: lays out a local string with a valid format.
        let (layout, m) = unsafe {
            let layout = self
                .dwrite
                .CreateTextLayout(&wide, &format, f32::MAX, f32::MAX)?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            (layout, m)
        };
        self.label = Some((text.to_owned(), scale, layout.clone(), m));
        Ok((layout, m))
    }
}
