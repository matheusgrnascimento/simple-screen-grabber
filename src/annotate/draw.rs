//! Draws annotations with Direct2D. The overlay and the export use this same
//! code, so a saved image matches what was on screen.

use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1DeviceContext, ID2D1PathGeometry, ID2D1SolidColorBrush, ID2D1StrokeStyle,
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_CAP_STYLE_FLAT, D2D1_CAP_STYLE_ROUND,
    D2D1_DASH_STYLE_SOLID, D2D1_ELLIPSE, D2D1_LINE_JOIN_MITER, D2D1_LINE_JOIN_ROUND,
    D2D1_STROKE_STYLE_PROPERTIES,
};
use windows_numerics::{Matrix3x2, Vector2};

use super::{PointF, Shape, Tool};
use crate::gfx;

/// Half the width of an arrow head relative to its length.
const HEAD_SPREAD: f32 = 0.55;

/// The length of an arrow head for a stroke width.
pub fn arrow_head_length(width: f32) -> f32 {
    3.5 * width + 8.0
}

fn vector(p: PointF) -> Vector2 {
    Vector2 { X: p.x, Y: p.y }
}

/// Direct2D resources for drawing shapes, created once.
pub struct ShapePainter {
    brush: ID2D1SolidColorBrush,
    /// Round caps and joins: lines, arrows and freehand strokes.
    round: ID2D1StrokeStyle,
    /// Sharp corners: rectangles.
    miter: ID2D1StrokeStyle,
    /// An arrow head of length 1 with its tip at the origin, pointing
    /// right; drawn with a transform, so no geometry is built per frame.
    head: ID2D1PathGeometry,
}

impl ShapePainter {
    pub fn new(ctx: &ID2D1DeviceContext) -> Result<ShapePainter> {
        let style = |cap, join| D2D1_STROKE_STYLE_PROPERTIES {
            startCap: cap,
            endCap: cap,
            dashCap: cap,
            lineJoin: join,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        // SAFETY: creates device-independent resources and one brush from
        // the context's factory; the geometry sink is closed before use.
        unsafe {
            let factory = ctx.GetFactory()?;
            let round = factory
                .CreateStrokeStyle(&style(D2D1_CAP_STYLE_ROUND, D2D1_LINE_JOIN_ROUND), None)?;
            let miter = factory
                .CreateStrokeStyle(&style(D2D1_CAP_STYLE_FLAT, D2D1_LINE_JOIN_MITER), None)?;
            let head = factory.CreatePathGeometry()?;
            let sink = head.Open()?;
            sink.BeginFigure(Vector2 { X: 0.0, Y: 0.0 }, D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&[
                Vector2 {
                    X: -1.0,
                    Y: -HEAD_SPREAD,
                },
                Vector2 {
                    X: -1.0,
                    Y: HEAD_SPREAD,
                },
            ]);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
            let brush = ctx.CreateSolidColorBrush(&gfx::color(super::COLORS[0].0), None)?;
            Ok(ShapePainter {
                brush,
                round,
                miter,
                head,
            })
        }
    }

    /// Draws `shapes` in order, clipped to `clip`, with `base` as the
    /// current transform (identity on screen, a translation for export).
    pub fn draw<'a>(
        &self,
        ctx: &ID2D1DeviceContext,
        shapes: impl IntoIterator<Item = &'a Shape>,
        clip: D2D_RECT_F,
        base: &Matrix3x2,
    ) {
        // SAFETY: drawing calls between the caller's BeginDraw and EndDraw,
        // with resources of the same device; the clip is popped below.
        unsafe {
            ctx.SetTransform(base);
            ctx.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
            for shape in shapes {
                self.shape(ctx, shape, base);
            }
            ctx.PopAxisAlignedClip();
        }
    }

    fn shape(&self, ctx: &ID2D1DeviceContext, shape: &Shape, base: &Matrix3x2) {
        // SAFETY: sets the color of this painter's brush.
        unsafe { self.brush.SetColor(&gfx::color(shape.color)) };
        let width = shape.stroke_width();
        match shape.tool {
            Tool::Line => self.line(ctx, shape.points[0], shape.points[1], width),
            Tool::Arrow => self.arrow(ctx, shape.points[0], shape.points[1], width, base),
            Tool::Rectangle => self.rectangle(ctx, shape.points[0], shape.points[1], width),
            Tool::Pen => self.stroke(ctx, &shape.points, width),
        }
    }

    fn line(&self, ctx: &ID2D1DeviceContext, a: PointF, b: PointF, width: f32) {
        // SAFETY: a drawing call with this painter's resources.
        unsafe { ctx.DrawLine(vector(a), vector(b), &self.brush, width, &self.round) };
    }

    /// A freehand stroke: segments with round caps, which join smoothly.
    fn stroke(&self, ctx: &ID2D1DeviceContext, points: &[PointF], width: f32) {
        match points {
            [] => {}
            [dot] => {
                let ellipse = D2D1_ELLIPSE {
                    point: vector(*dot),
                    radiusX: width / 2.0,
                    radiusY: width / 2.0,
                };
                // SAFETY: a drawing call with this painter's resources.
                unsafe { ctx.FillEllipse(&ellipse, &self.brush) };
            }
            _ => {
                for pair in points.windows(2) {
                    self.line(ctx, pair[0], pair[1], width);
                }
            }
        }
    }

    fn rectangle(&self, ctx: &ID2D1DeviceContext, a: PointF, b: PointF, width: f32) {
        // Points are pixel centers: odd widths are sharp there; even widths
        // need the edges on pixel boundaries.
        let shift = if (width.round() as i32) % 2 == 0 {
            0.5
        } else {
            0.0
        };
        let rect = gfx::rect_f(
            a.x.min(b.x) - shift,
            a.y.min(b.y) - shift,
            a.x.max(b.x) - shift,
            a.y.max(b.y) - shift,
        );
        // SAFETY: a drawing call with this painter's resources.
        unsafe { ctx.DrawRectangle(&rect, &self.brush, width, &self.miter) };
    }

    fn arrow(
        &self,
        ctx: &ID2D1DeviceContext,
        from: PointF,
        to: PointF,
        width: f32,
        base: &Matrix3x2,
    ) {
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        let length = (dx * dx + dy * dy).sqrt();
        if length < 1.0 {
            return;
        }
        let head = arrow_head_length(width).min(length);
        let (ux, uy) = (dx / length, dy / length);
        // The shaft stops inside the head, so its round cap never shows.
        let shaft_end = PointF::new(to.x - ux * head * 0.8, to.y - uy * head * 0.8);
        if head < length {
            self.line(ctx, from, shaft_end, width);
        }
        let transform = Matrix3x2::scale(head, head)
            * Matrix3x2::rotation(dy.atan2(dx).to_degrees())
            * Matrix3x2::translation(to.x, to.y)
            * base;
        // SAFETY: drawing calls with this painter's resources; the base
        // transform is restored.
        unsafe {
            ctx.SetTransform(&transform);
            ctx.FillGeometry(&self.head, &self.brush, None);
            ctx.SetTransform(base);
        }
    }
}
