//! Integer pixel geometry. Coordinates are physical pixels of the overlay,
//! which covers the virtual screen with its top-left corner at (0, 0).

/// A pixel position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Point {
        Point { x, y }
    }
}

/// A rectangle of pixels: `left` and `top` are included, `right` and
/// `bottom` are not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub const fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    pub const fn contains(&self, p: Point) -> bool {
        p.x >= self.left && p.x < self.right && p.y >= self.top && p.y < self.bottom
    }

    /// The overlap of two rectangles, empty if they do not touch.
    pub fn intersect(&self, other: Rect) -> Rect {
        let r = Rect::new(
            self.left.max(other.left),
            self.top.max(other.top),
            self.right.min(other.right),
            self.bottom.min(other.bottom),
        );
        if r.is_empty() {
            Rect::default()
        } else {
            r
        }
    }

    pub const fn offset(&self, dx: i32, dy: i32) -> Rect {
        Rect::new(
            self.left + dx,
            self.top + dy,
            self.right + dx,
            self.bottom + dy,
        )
    }

    /// The smallest rectangle that includes the pixels at `a` and `b`.
    pub fn spanning(a: Point, b: Point) -> Rect {
        Rect::new(
            a.x.min(b.x),
            a.y.min(b.y),
            a.x.max(b.x) + 1,
            a.y.max(b.y) + 1,
        )
    }

    /// This rectangle moved (not resized) to lie inside `bounds`, or
    /// clipped to it when it is larger.
    pub fn moved_inside(&self, bounds: Rect) -> Rect {
        let dx = if self.left < bounds.left {
            bounds.left - self.left
        } else if self.right > bounds.right {
            (bounds.right - self.right).max(bounds.left - self.left)
        } else {
            0
        };
        let dy = if self.top < bounds.top {
            bounds.top - self.top
        } else if self.bottom > bounds.bottom {
            (bounds.bottom - self.bottom).max(bounds.top - self.top)
        } else {
            0
        };
        self.offset(dx, dy).intersect(bounds)
    }

    /// The point inside the rectangle closest to `p`.
    pub fn clamp(&self, p: Point) -> Point {
        Point::new(
            p.x.clamp(self.left, (self.right - 1).max(self.left)),
            p.y.clamp(self.top, (self.bottom - 1).max(self.top)),
        )
    }
}

/// A rectangle in fractional pixels, for UI elements.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RectF {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl RectF {
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> RectF {
        RectF {
            left,
            top,
            right,
            bottom,
        }
    }

    /// A rectangle from its top-left corner and size.
    pub const fn sized(left: f32, top: f32, width: f32, height: f32) -> RectF {
        RectF::new(left, top, left + width, top + height)
    }

    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }

    pub fn offset(&self, dx: f32, dy: f32) -> RectF {
        RectF::new(
            self.left + dx,
            self.top + dy,
            self.right + dx,
            self.bottom + dy,
        )
    }

    /// The rectangle with its edges rounded to whole pixels.
    pub fn snapped(&self) -> RectF {
        RectF::new(
            self.left.round(),
            self.top.round(),
            self.right.round(),
            self.bottom.round(),
        )
    }
}

impl From<Rect> for RectF {
    fn from(r: Rect) -> RectF {
        RectF::new(r.left as f32, r.top as f32, r.right as f32, r.bottom as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanning_includes_both_pixels() {
        let r = Rect::spanning(Point::new(20, 5), Point::new(10, 15));
        assert_eq!(r, Rect::new(10, 5, 21, 16));
        assert!(r.contains(Point::new(20, 15)));
        assert_eq!(
            Rect::spanning(Point::new(3, 3), Point::new(3, 3)).width(),
            1
        );
    }

    #[test]
    fn moved_inside_keeps_the_size() {
        let bounds = Rect::new(0, 0, 100, 100);
        let r = Rect::new(90, -5, 110, 15);
        assert_eq!(r.moved_inside(bounds), Rect::new(80, 0, 100, 20));
        let big = Rect::new(-10, 10, 150, 20);
        assert_eq!(big.moved_inside(bounds), Rect::new(0, 10, 100, 20));
    }

    #[test]
    fn intersect_of_disjoint_rectangles_is_empty() {
        let a = Rect::new(0, 0, 10, 10);
        assert!(a.intersect(Rect::new(10, 0, 20, 10)).is_empty());
        assert_eq!(
            a.intersect(Rect::new(5, 5, 20, 20)),
            Rect::new(5, 5, 10, 10)
        );
    }

    #[test]
    fn clamp_stays_on_pixels_inside() {
        let r = Rect::new(0, 0, 10, 10);
        assert_eq!(r.clamp(Point::new(-4, 30)), Point::new(0, 9));
    }
}
