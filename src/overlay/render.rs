//! Draws the overlay with Direct2D: the frozen screen, the annotations, the
//! dimmed area outside the selection, the selection border, its handles,
//! the size label and the toolbars. 1 DIP is 1 physical pixel; UI sizes are
//! scaled per monitor.

use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::{
    ID2D1DeviceContext, ID2D1SolidColorBrush, ID2D1StrokeStyle, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_CAP_STYLE_ROUND, D2D1_DASH_STYLE_SOLID,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
    D2D1_LINE_JOIN_ROUND, D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_MEASURING_MODE_NATURAL,
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, DWRITE_TEXT_METRICS,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::toolbar::{Bar, Button, Hit, Slot, Toolbars};
use super::{layout, selection, Drag, Hover, Session};
use crate::annotate::draw::ShapePainter;
use crate::annotate::{Tool, COLORS};
use crate::fluent::{Color, Palette};
use crate::geometry::{Point, Rect, RectF};
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
    /// Corner radius of controls (buttons, labels).
    pub const CORNER_RADIUS: f32 = 4.0;
    /// Corner radius of flyouts and toolbars.
    pub const OVERLAY_RADIUS: f32 = 8.0;
    pub const GAP: f32 = 8.0;
    pub const ICON: f32 = 16.0;
    /// Radius of a color swatch's disc, and of the current color's ring.
    pub const SWATCH_RADIUS: f32 = 10.0;
    pub const SWATCH_RING_RADIUS: f32 = 13.0;
    /// Radius of the current color in the color button.
    pub const COLOR_DOT: f32 = 8.0;
}

/// Glyphs of Segoe Fluent Icons (the same code points in Segoe MDL2 Assets
/// on Windows 10).
mod glyph {
    pub const PEN: char = '\u{E76D}';
    pub const HIGHLIGHTER: char = '\u{E7E6}';
    pub const UNDO: char = '\u{E7A7}';
    pub const REDO: char = '\u{E7A6}';
    pub const COPY: char = '\u{E8C8}';
    pub const SAVE: char = '\u{E74E}';
    pub const CLOSE: char = '\u{E711}';
}

/// Brushes, fonts and text layouts reused across frames.
pub struct Painter {
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    dwrite: IDWriteFactory,
    /// Segoe UI Variable (Windows 11) is installed; otherwise Segoe UI.
    variable_font: bool,
    /// Segoe Fluent Icons (Windows 11) is installed; otherwise Segoe MDL2
    /// Assets.
    fluent_icons: bool,
    label_format: Option<(f32, IDWriteTextFormat)>,
    /// The size label's layout for (width, height, scale).
    label: Option<((i32, i32, f32), IDWriteTextLayout, DWRITE_TEXT_METRICS)>,
    icon_format: Option<(f32, IDWriteTextFormat)>,
    /// The tooltip's layout for (button, scale).
    tooltip: Option<((Button, f32), IDWriteTextLayout, DWRITE_TEXT_METRICS)>,
}

/// Segoe UI Variable (Windows 11), or Segoe UI (Windows 10).
fn text_family(variable_font: bool) -> PCWSTR {
    if variable_font {
        w!("Segoe UI Variable Text")
    } else {
        w!("Segoe UI")
    }
}

fn icon_family(fluent: bool) -> PCWSTR {
    if fluent {
        w!("Segoe Fluent Icons")
    } else {
        w!("Segoe MDL2 Assets")
    }
}

fn rounded(r: RectF, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT {
        rect: gfx::rect_f(r.left, r.top, r.right, r.bottom),
        radiusX: radius,
        radiusY: radius,
    }
}

fn disc(x: f32, y: f32, radius: f32) -> D2D1_ELLIPSE {
    D2D1_ELLIPSE {
        point: Vector2 { X: x, Y: y },
        radiusX: radius,
        radiusY: radius,
    }
}

impl Painter {
    pub fn new(ctx: &ID2D1DeviceContext, dwrite: &IDWriteFactory) -> Result<Painter> {
        // SAFETY: COM calls on valid objects; out pointers are locals.
        unsafe {
            let brush = ctx.CreateSolidColorBrush(&D2D1_COLOR_F::default(), None)?;
            let round = ctx.GetFactory()?.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    dashCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    miterLimit: 10.0,
                    dashStyle: D2D1_DASH_STYLE_SOLID,
                    dashOffset: 0.0,
                },
                None,
            )?;
            let mut collection = None;
            dwrite.GetSystemFontCollection(&mut collection, false)?;
            let exists = |family: PCWSTR| -> bool {
                let (mut index, mut found) = (0, BOOL(0));
                collection.as_ref().is_some_and(|c| {
                    c.FindFamilyName(family, &mut index, &mut found).is_ok() && found.as_bool()
                })
            };
            Ok(Painter {
                brush,
                round,
                dwrite: dwrite.clone(),
                variable_font: exists(text_family(true)),
                fluent_icons: exists(icon_family(true)),
                label_format: None,
                label: None,
                icon_format: None,
                tooltip: None,
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

    /// A rounded rectangle with a hairline border inside its edge.
    fn panel(
        &self,
        ctx: &ID2D1DeviceContext,
        r: RectF,
        radius: f32,
        fill: Color,
        border: Color,
        scale: f32,
    ) {
        let stroke = scale.round().max(1.0);
        let half = stroke / 2.0;
        let inner = RectF::new(r.left + half, r.top + half, r.right - half, r.bottom - half);
        // SAFETY: draws with a brush of the same device context.
        unsafe {
            ctx.FillRoundedRectangle(&rounded(r, radius), self.brush(fill));
            ctx.DrawRoundedRectangle(&rounded(inner, radius), self.brush(border), stroke, None);
        }
    }

    /// Draws one frame of the overlay.
    pub fn frame(
        &mut self,
        ctx: &ID2D1DeviceContext,
        session: &Session,
        shapes: &ShapePainter,
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
        let clip = gfx::rect_f(
            sel.left as f32,
            sel.top as f32,
            sel.right as f32,
            sel.bottom as f32,
        );
        shapes.draw(
            ctx,
            session.history.shapes().iter().chain(&session.drawing),
            clip,
            &Matrix3x2::identity(),
        );
        self.scrim(ctx, bounds, sel, palette.scrim);
        self.border(ctx, session, sel, palette.accent);
        if !matches!(session.drag, Drag::Drawing) {
            self.handles(ctx, session, sel, palette);
        }
        self.size_label(ctx, session, sel, palette)?;
        if let Some(bars) = session.toolbars() {
            self.toolbars(ctx, session, &bars, palette)?;
            if let Some(button) = session.tooltip {
                self.tooltip(ctx, session, button, bars.scale, palette)?;
            }
        }
        self.brush_outline(ctx, session);
        Ok(())
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
            } else if session.hover == Some(Hover::Handle(handle)) && pressed.is_none() {
                metrics::HANDLE_DOT_HOVER
            } else {
                metrics::HANDLE_DOT
            };
            // SAFETY: draws with a brush of the same device context.
            unsafe {
                ctx.FillEllipse(
                    &disc(x, y, (metrics::HANDLE_RADIUS + 1.0) * scale),
                    self.brush(palette.handle_border),
                );
                ctx.FillEllipse(
                    &disc(x, y, metrics::HANDLE_RADIUS * scale),
                    self.brush(palette.handle),
                );
                ctx.FillEllipse(&disc(x, y, dot * scale), self.brush(palette.accent));
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
        let monitor = session.capture.monitor_at(Point::new(sel.left, sel.top));
        let scale = monitor.scale;
        let (layout, m) = self.label_layout(sel.width(), sel.height(), scale)?;
        let width = (m.width + 2.0 * metrics::LABEL_PADDING * scale).ceil();
        let height = (metrics::LABEL_HEIGHT * scale).round();
        let r = layout::size_label(sel, monitor.rect, width, height, metrics::GAP * scale);
        self.panel(
            ctx,
            r,
            metrics::CORNER_RADIUS * scale,
            palette.flyout,
            palette.flyout_border,
            scale,
        );
        let origin = Vector2 {
            X: r.left + ((r.width() - m.width) / 2.0).round(),
            Y: r.top + ((r.height() - m.height) / 2.0).round(),
        };
        // SAFETY: draws a layout created by this painter's factory.
        unsafe {
            ctx.DrawTextLayout(
                origin,
                &layout,
                self.brush(palette.text),
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            )
        };
        Ok(())
    }

    /// The text layout of the size label, reused while the size and scale
    /// stay the same.
    fn label_layout(
        &mut self,
        width: i32,
        height: i32,
        scale: f32,
    ) -> Result<(IDWriteTextLayout, DWRITE_TEXT_METRICS)> {
        let key = (width, height, scale);
        if let Some((k, layout, m)) = &self.label {
            if *k == key {
                return Ok((layout.clone(), *m));
            }
        }
        let text = format!("{width} \u{00D7} {height}");
        let format = match &self.label_format {
            Some((s, f)) if *s == scale => f.clone(),
            _ => {
                let f =
                    self.text_format(text_family(self.variable_font), metrics::LABEL_FONT * scale)?;
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
        self.label = Some((key, layout.clone(), m));
        Ok((layout, m))
    }

    fn text_format(&self, family: PCWSTR, size: f32) -> Result<IDWriteTextFormat> {
        // SAFETY: creates a text format with constant arguments.
        unsafe {
            self.dwrite.CreateTextFormat(
                family,
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("en-us"),
            )
        }
    }

    fn icon_format(&mut self, scale: f32) -> Result<IDWriteTextFormat> {
        if let Some((s, f)) = &self.icon_format {
            if *s == scale {
                return Ok(f.clone());
            }
        }
        let format = self.text_format(icon_family(self.fluent_icons), metrics::ICON * scale)?;
        // SAFETY: plain setters on a format owned here.
        unsafe {
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }
        self.icon_format = Some((scale, format.clone()));
        Ok(format)
    }

    fn toolbars(
        &mut self,
        ctx: &ID2D1DeviceContext,
        session: &Session,
        bars: &Toolbars,
        palette: &Palette,
    ) -> Result<()> {
        self.bar(ctx, session, &bars.tools, bars.scale, palette)?;
        self.bar(ctx, session, &bars.actions, bars.scale, palette)?;
        if let Some(flyout) = &bars.flyout {
            let s = bars.scale;
            self.panel(
                ctx,
                flyout.rect,
                metrics::OVERLAY_RADIUS * s,
                palette.flyout,
                palette.flyout_border,
                s,
            );
            for (i, r) in flyout.swatches.iter().enumerate() {
                if session.hover == Some(Hover::Toolbar(Hit::Swatch(i))) {
                    // SAFETY: draws with a brush of the same device context.
                    unsafe {
                        ctx.FillRoundedRectangle(
                            &rounded(*r, metrics::CORNER_RADIUS * s),
                            self.brush(palette.subtle_hover),
                        )
                    };
                }
                let (cx, cy) = (r.left + r.width() / 2.0, r.top + r.height() / 2.0);
                if i == session.color {
                    // SAFETY: as above.
                    unsafe {
                        ctx.DrawEllipse(
                            &disc(cx, cy, metrics::SWATCH_RING_RADIUS * s - s),
                            self.brush(palette.accent),
                            2.0 * s,
                            None,
                        )
                    };
                }
                self.swatch(
                    ctx,
                    cx,
                    cy,
                    metrics::SWATCH_RADIUS * s,
                    COLORS[i].0,
                    palette,
                    s,
                );
            }
        }
        Ok(())
    }

    /// A color disc with a thin ring, so black and white stay visible on
    /// any background.
    #[allow(clippy::too_many_arguments)]
    fn swatch(
        &self,
        ctx: &ID2D1DeviceContext,
        x: f32,
        y: f32,
        radius: f32,
        color: Color,
        palette: &Palette,
        scale: f32,
    ) {
        let stroke = scale.max(1.0);
        // SAFETY: draws with a brush of the same device context.
        unsafe {
            ctx.FillEllipse(&disc(x, y, radius), self.brush(color));
            ctx.DrawEllipse(
                &disc(x, y, radius - stroke / 2.0),
                self.brush(palette.swatch_ring),
                stroke,
                None,
            );
        }
    }

    fn bar<const N: usize>(
        &mut self,
        ctx: &ID2D1DeviceContext,
        session: &Session,
        bar: &Bar<N>,
        s: f32,
        palette: &Palette,
    ) -> Result<()> {
        self.panel(
            ctx,
            bar.rect,
            metrics::OVERLAY_RADIUS * s,
            palette.flyout,
            palette.flyout_border,
            s,
        );
        for slot in &bar.slots {
            match *slot {
                Slot::Separator(r) => self.fill(ctx, r, palette.divider),
                Slot::Button(button, r) => {
                    let enabled = session.is_enabled(button);
                    let checked = match button {
                        Button::Tool(tool) => session.tool == Some(tool),
                        _ => false,
                    };
                    // The color button looks hovered while its flyout is open.
                    let hovered = session.hover == Some(Hover::Toolbar(Hit::Button(button)))
                        || (button == Button::Color && session.flyout_open);
                    let pressed = hovered && session.drag == Drag::Pressing(Hit::Button(button));
                    let background = match (enabled, checked, pressed, hovered) {
                        (false, ..) => None,
                        (_, true, _, true) => Some(palette.accent_hover),
                        (_, true, _, false) => Some(palette.accent),
                        (_, false, true, _) => Some(palette.subtle_pressed),
                        (_, false, false, true) => Some(palette.subtle_hover),
                        _ => None,
                    };
                    if let Some(fill) = background {
                        // SAFETY: draws with a brush of the same device context.
                        unsafe {
                            ctx.FillRoundedRectangle(
                                &rounded(r, metrics::CORNER_RADIUS * s),
                                self.brush(fill),
                            )
                        };
                    }
                    let foreground = if !enabled {
                        palette.text_disabled
                    } else if checked {
                        palette.on_accent
                    } else if pressed {
                        palette.text_secondary
                    } else {
                        palette.text
                    };
                    self.icon(ctx, session, button, r, foreground, palette, s)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn icon(
        &mut self,
        ctx: &ID2D1DeviceContext,
        session: &Session,
        button: Button,
        r: RectF,
        color: Color,
        palette: &Palette,
        s: f32,
    ) -> Result<()> {
        let (cx, cy) = (r.left + r.width() / 2.0, r.top + r.height() / 2.0);
        // Line icons are drawn on a 16-unit grid centered in the button.
        let (ox, oy) = ((cx - 8.0 * s).round(), (cy - 8.0 * s).round());
        let at = |x: f32, y: f32| Vector2 {
            X: ox + x * s,
            Y: oy + y * s,
        };
        let stroke = 1.25 * s;
        let glyph = match button {
            Button::Tool(Tool::Arrow) => {
                // SAFETY: draws with resources of the same device context.
                unsafe {
                    let brush = self.brush(color);
                    ctx.DrawLine(at(3.0, 13.0), at(12.5, 3.5), brush, stroke, &self.round);
                    ctx.DrawLine(at(12.5, 3.5), at(6.5, 3.5), brush, stroke, &self.round);
                    ctx.DrawLine(at(12.5, 3.5), at(12.5, 9.5), brush, stroke, &self.round);
                }
                None
            }
            Button::Tool(Tool::Line) => {
                // SAFETY: as above.
                unsafe {
                    ctx.DrawLine(
                        at(3.0, 13.0),
                        at(13.0, 3.0),
                        self.brush(color),
                        stroke,
                        &self.round,
                    )
                };
                None
            }
            Button::Tool(Tool::Rectangle) => {
                let w = s.round().max(1.0);
                let half = w / 2.0;
                let rect = gfx::rect_f(
                    ox + 2.0 * s + half,
                    oy + 3.0 * s + half,
                    ox + 14.0 * s - half,
                    oy + 13.0 * s - half,
                );
                // SAFETY: as above.
                unsafe {
                    ctx.DrawRoundedRectangle(
                        &D2D1_ROUNDED_RECT {
                            rect,
                            radiusX: 1.5 * s,
                            radiusY: 1.5 * s,
                        },
                        self.brush(color),
                        w,
                        None,
                    )
                };
                None
            }
            Button::Color => {
                self.swatch(
                    ctx,
                    cx,
                    cy,
                    metrics::COLOR_DOT * s,
                    COLORS[session.color].0,
                    palette,
                    s,
                );
                None
            }
            Button::Tool(Tool::Pen) => Some(glyph::PEN),
            Button::Tool(Tool::Highlighter) => Some(glyph::HIGHLIGHTER),
            Button::Undo => Some(glyph::UNDO),
            Button::Redo => Some(glyph::REDO),
            Button::Copy => Some(glyph::COPY),
            Button::Save => Some(glyph::SAVE),
            Button::Close => Some(glyph::CLOSE),
        };
        if let Some(glyph) = glyph {
            let format = self.icon_format(s)?;
            let mut text = [0u16; 2];
            let text = glyph.encode_utf16(&mut text);
            // SAFETY: draws a local string with a format of this painter.
            unsafe {
                ctx.DrawText(
                    text,
                    &format,
                    &gfx::rect_f(r.left, r.top, r.right, r.bottom),
                    self.brush(color),
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                )
            };
        }
        Ok(())
    }

    /// A Windows 11 tooltip below the mouse, or above it at the bottom of
    /// the monitor.
    fn tooltip(
        &mut self,
        ctx: &ID2D1DeviceContext,
        session: &Session,
        button: Button,
        s: f32,
        palette: &Palette,
    ) -> Result<()> {
        let key = (button, s);
        let (layout, m) = match &self.tooltip {
            Some((k, layout, m)) if *k == key => (layout.clone(), *m),
            _ => {
                let format =
                    self.text_format(text_family(self.variable_font), metrics::LABEL_FONT * s)?;
                let text: Vec<u16> = button.tooltip().encode_utf16().collect();
                // SAFETY: lays out a local string with a valid format.
                let (layout, m) = unsafe {
                    let layout =
                        self.dwrite
                            .CreateTextLayout(&text, &format, f32::MAX, f32::MAX)?;
                    let mut m = DWRITE_TEXT_METRICS::default();
                    layout.GetMetrics(&mut m)?;
                    (layout, m)
                };
                self.tooltip = Some((key, layout.clone(), m));
                (layout, m)
            }
        };
        let (pad_x, pad_y) = ((9.0 * s).round(), (6.0 * s).round());
        let (w, h) = (
            (m.width + 2.0 * pad_x).ceil(),
            (m.height + 2.0 * pad_y).ceil(),
        );
        let monitor = session.capture.monitor_at(session.mouse).rect;
        let (mouse_x, mouse_y) = (session.mouse.x as f32, session.mouse.y as f32);
        let below = mouse_y + (24.0 * s).round();
        let top = if below + h <= monitor.bottom as f32 {
            below
        } else {
            mouse_y - (8.0 * s).round() - h
        };
        let left = mouse_x
            .min(monitor.right as f32 - w)
            .max(monitor.left as f32);
        let r = RectF::sized(left.round(), top.round(), w, h);
        self.panel(
            ctx,
            r,
            metrics::CORNER_RADIUS * s,
            palette.flyout,
            palette.flyout_border,
            s,
        );
        // SAFETY: draws a layout created by this painter's factory.
        unsafe {
            ctx.DrawTextLayout(
                Vector2 {
                    X: r.left + pad_x,
                    Y: r.top + pad_y,
                },
                &layout,
                self.brush(palette.text),
                D2D1_DRAW_TEXT_OPTIONS_NONE,
            )
        };
        Ok(())
    }

    /// The outline of the brush under the mouse while a tool is on: shows
    /// the stroke width the mouse wheel sets.
    fn brush_outline(&self, ctx: &ID2D1DeviceContext, session: &Session) {
        let Some(tool) = session.tool else {
            return;
        };
        if session.hover.is_some() || !matches!(session.drag, Drag::None | Drag::Drawing) {
            return;
        }
        let radius = (tool.stroke_width(session.width) / 2.0).max(2.0);
        let (x, y) = (session.mouse.x as f32 + 0.5, session.mouse.y as f32 + 0.5);
        // SAFETY: draws with a brush of the same device context.
        unsafe {
            ctx.DrawEllipse(
                &disc(x, y, radius + 0.5),
                self.brush(Color::rgba(255, 255, 255, 0xD0)),
                1.0,
                None,
            );
            ctx.DrawEllipse(
                &disc(x, y, radius - 0.5),
                self.brush(Color::rgba(0, 0, 0, 0xD0)),
                1.0,
                None,
            );
        }
    }
}
