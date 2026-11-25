//! Annotations: the tools, the shapes they draw and the undo history. Pure
//! logic; [`draw`] renders the shapes with Direct2D.
//!
//! Shapes are vectors in overlay pixels, so undo and redo never copy
//! images, and the export draws them again at native resolution.

pub mod draw;

use crate::fluent::Color;

/// A drawing tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Arrow,
    Rectangle,
    Line,
    Pen,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Arrow, Tool::Rectangle, Tool::Line, Tool::Pen];

    pub const fn name(self) -> &'static str {
        match self {
            Tool::Arrow => "Arrow",
            Tool::Rectangle => "Rectangle",
            Tool::Line => "Line",
            Tool::Pen => "Pen",
        }
    }

    /// The letter key that picks the tool.
    pub const fn key(self) -> u8 {
        match self {
            Tool::Arrow => b'A',
            Tool::Rectangle => b'R',
            Tool::Line => b'L',
            Tool::Pen => b'P',
        }
    }

    pub fn from_key(key: u16) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| u16::from(t.key()) == key)
    }

    /// Freehand tools record every mouse position; the others keep only
    /// the start and end points.
    pub const fn is_freehand(self) -> bool {
        matches!(self, Tool::Pen)
    }
}

/// The annotation colors.
pub const COLORS: [(Color, &str); 8] = [
    (Color::rgb(0xFF, 0x4B, 0x4B), "Red"),
    (Color::rgb(0xFF, 0x8C, 0x00), "Orange"),
    (Color::rgb(0xFF, 0xC8, 0x3D), "Yellow"),
    (Color::rgb(0x16, 0xC6, 0x0C), "Green"),
    (Color::rgb(0x00, 0x78, 0xD4), "Blue"),
    (Color::rgb(0x8E, 0x44, 0xAD), "Purple"),
    (Color::rgb(0x00, 0x00, 0x00), "Black"),
    (Color::rgb(0xFF, 0xFF, 0xFF), "White"),
];

/// Stroke width limits, in pixels.
pub const MIN_WIDTH: f32 = 1.0;
pub const MAX_WIDTH: f32 = 64.0;

/// The default color: blue.
pub const DEFAULT_COLOR: usize = 4;

/// A point in overlay pixels; pixel centers are at `.5`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PointF {
    pub x: f32,
    pub y: f32,
}

impl PointF {
    pub const fn new(x: f32, y: f32) -> PointF {
        PointF { x, y }
    }

    /// The center of a pixel.
    pub fn center(x: i32, y: i32) -> PointF {
        PointF::new(x as f32 + 0.5, y as f32 + 0.5)
    }

    fn distance(self, other: PointF) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/// One annotation.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub tool: Tool,
    pub color: Color,
    /// The stroke width in pixels.
    pub width: f32,
    /// Start and end for arrows, rectangles and lines; every position for
    /// freehand tools.
    pub points: Vec<PointF>,
}

impl Shape {
    pub fn new(tool: Tool, color: Color, width: f32, start: PointF) -> Shape {
        let points = if tool.is_freehand() {
            vec![start]
        } else {
            vec![start, start]
        };
        Shape {
            tool,
            color,
            width,
            points,
        }
    }

    /// The width the stroke is drawn with.
    pub const fn stroke_width(&self) -> f32 {
        self.width
    }

    /// Follows the mouse to `p`. With `constrain` (Shift), lines and arrows
    /// snap to multiples of 45 degrees and rectangles become squares.
    pub fn extend(&mut self, p: PointF, constrain: bool) {
        if self.tool.is_freehand() {
            // Skip positions closer than a pixel: they add nothing.
            if self
                .points
                .last()
                .is_none_or(|last| last.distance(p) >= 1.0)
            {
                self.points.push(p);
            }
            return;
        }
        let start = self.points[0];
        let end = if constrain {
            constrained(self.tool, start, p)
        } else {
            p
        };
        self.points[1] = end;
    }

    /// Whether the shape draws anything: two-point shapes need a length;
    /// freehand strokes draw at least a dot.
    pub fn is_visible(&self) -> bool {
        if self.tool.is_freehand() {
            !self.points.is_empty()
        } else {
            self.points[0].distance(self.points[1]) >= 1.0
        }
    }

    /// The area the shape covers, including its stroke and arrow head.
    pub fn bounds(&self) -> (PointF, PointF) {
        let margin = self.stroke_width() / 2.0
            + if self.tool == Tool::Arrow {
                draw::arrow_head_length(self.width)
            } else {
                0.0
            };
        let mut min = PointF::new(f32::MAX, f32::MAX);
        let mut max = PointF::new(f32::MIN, f32::MIN);
        for p in &self.points {
            min = PointF::new(min.x.min(p.x), min.y.min(p.y));
            max = PointF::new(max.x.max(p.x), max.y.max(p.y));
        }
        (
            PointF::new(min.x - margin, min.y - margin),
            PointF::new(max.x + margin, max.y + margin),
        )
    }
}

/// `p` moved so that the shape from `start` is constrained: an angle that
/// is a multiple of 45 degrees, or a square.
fn constrained(tool: Tool, start: PointF, p: PointF) -> PointF {
    let (dx, dy) = (p.x - start.x, p.y - start.y);
    if tool == Tool::Rectangle {
        let side = dx.abs().max(dy.abs());
        return PointF::new(start.x + side.copysign(dx), start.y + side.copysign(dy));
    }
    let length = (dx * dx + dy * dy).sqrt();
    let step = std::f32::consts::FRAC_PI_4;
    let angle = (dy.atan2(dx) / step).round() * step;
    PointF::new(
        start.x + length * angle.cos(),
        start.y + length * angle.sin(),
    )
}

/// The shapes drawn so far, with undo and redo.
#[derive(Default)]
pub struct History {
    shapes: Vec<Shape>,
    undone: Vec<Shape>,
}

impl History {
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// Adds a finished shape; it can no longer redo what was undone.
    pub fn push(&mut self, shape: Shape) {
        self.shapes.push(shape);
        self.undone.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.shapes.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        match self.shapes.pop() {
            Some(shape) => {
                self.undone.push(shape);
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        match self.undone.pop() {
            Some(shape) => {
                self.shapes.push(shape);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Color = COLORS[0].0;

    fn line(x: f32, y: f32) -> Shape {
        let mut shape = Shape::new(Tool::Line, RED, 3.0, PointF::new(0.5, 0.5));
        shape.extend(PointF::new(x, y), false);
        shape
    }

    #[test]
    fn keys_pick_tools() {
        assert_eq!(Tool::from_key(u16::from(b'P')), Some(Tool::Pen));
        assert_eq!(Tool::from_key(u16::from(b'H')), None);
        assert_eq!(Tool::from_key(u16::from(b'Z')), None);
    }

    #[test]
    fn two_point_shapes_keep_start_and_end() {
        let shape = line(10.5, 20.5);
        assert_eq!(
            shape.points,
            vec![PointF::new(0.5, 0.5), PointF::new(10.5, 20.5)]
        );
        assert!(shape.is_visible());
        assert!(!line(0.5, 0.5).is_visible());
    }

    #[test]
    fn freehand_skips_positions_under_a_pixel() {
        let mut pen = Shape::new(Tool::Pen, RED, 3.0, PointF::new(0.5, 0.5));
        pen.extend(PointF::new(0.9, 0.5), false);
        pen.extend(PointF::new(2.5, 0.5), false);
        assert_eq!(pen.points.len(), 2);
        assert!(pen.is_visible());
    }

    #[test]
    fn shift_snaps_lines_to_45_degrees() {
        let mut shape = Shape::new(Tool::Line, RED, 3.0, PointF::new(0.0, 0.0));
        shape.extend(PointF::new(100.0, 8.0), true);
        let end = shape.points[1];
        assert!(end.y.abs() < 0.01);
        assert!((end.x - (100.0f32 * 100.0 + 64.0).sqrt()).abs() < 0.01);
        shape.extend(PointF::new(50.0, 46.0), true);
        let end = shape.points[1];
        assert!((end.x - end.y).abs() < 0.01);
    }

    #[test]
    fn shift_makes_squares() {
        let mut shape = Shape::new(Tool::Rectangle, RED, 3.0, PointF::new(10.0, 10.0));
        shape.extend(PointF::new(40.0, -5.0), true);
        assert_eq!(shape.points[1], PointF::new(40.0, -20.0));
    }

    #[test]
    fn bounds_include_the_stroke() {
        let (min, max) = line(10.5, 20.5).bounds();
        assert_eq!(
            (min, max),
            (PointF::new(-1.0, -1.0), PointF::new(12.0, 22.0))
        );
    }

    #[test]
    fn undo_and_redo() {
        let mut history = History::default();
        assert!(!history.undo());
        history.push(line(1.5, 1.5));
        history.push(line(2.5, 2.5));
        assert!(history.undo());
        assert_eq!(history.shapes().len(), 1);
        assert!(history.can_redo());
        assert!(history.redo());
        assert_eq!(history.shapes().len(), 2);
        history.undo();
        history.push(line(3.5, 3.5));
        assert!(!history.can_redo());
    }
}
