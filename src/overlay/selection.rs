//! Selection geometry: handles, hit testing, resizing and moving. Pure logic
//! in physical pixels.

use crate::geometry::{Point, Rect};

/// A resize handle on the selection border.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

pub const ALL_HANDLES: [Handle; 8] = [
    Handle::TopLeft,
    Handle::Top,
    Handle::TopRight,
    Handle::Right,
    Handle::BottomRight,
    Handle::Bottom,
    Handle::BottomLeft,
    Handle::Left,
];

pub const CORNER_HANDLES: [Handle; 4] = [
    Handle::TopLeft,
    Handle::TopRight,
    Handle::BottomRight,
    Handle::BottomLeft,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Start,
    Middle,
    End,
}

impl Handle {
    /// Which horizontal and vertical edges the handle moves.
    const fn sides(self) -> (Side, Side) {
        match self {
            Handle::TopLeft => (Side::Start, Side::Start),
            Handle::Top => (Side::Middle, Side::Start),
            Handle::TopRight => (Side::End, Side::Start),
            Handle::Right => (Side::End, Side::Middle),
            Handle::BottomRight => (Side::End, Side::End),
            Handle::Bottom => (Side::Middle, Side::End),
            Handle::BottomLeft => (Side::Start, Side::End),
            Handle::Left => (Side::Start, Side::Middle),
        }
    }

    /// True for the handles that resize diagonally from top-left to
    /// bottom-right (for the cursor shape).
    pub const fn is_nwse(self) -> bool {
        matches!(self, Handle::TopLeft | Handle::BottomRight)
    }

    pub const fn is_nesw(self) -> bool {
        matches!(self, Handle::TopRight | Handle::BottomLeft)
    }

    pub const fn is_horizontal(self) -> bool {
        matches!(self, Handle::Left | Handle::Right)
    }
}

/// The center of a handle: on the selection border, between the last pixel
/// inside and the first pixel outside.
pub fn handle_center(sel: Rect, handle: Handle) -> (f32, f32) {
    let along = |side: Side, start: i32, end: i32| match side {
        Side::Start => start as f32,
        Side::Middle => (start + end) as f32 / 2.0,
        Side::End => end as f32,
    };
    let (h, v) = handle.sides();
    (along(h, sel.left, sel.right), along(v, sel.top, sel.bottom))
}

/// The handles shown for a selection: only the corners when it is too
/// small for the edge handles to fit between them.
pub fn visible_handles(sel: Rect, diameter: f32) -> &'static [Handle] {
    let min = diameter * 3.0;
    if (sel.width() as f32) < min || (sel.height() as f32) < min {
        &CORNER_HANDLES
    } else {
        &ALL_HANDLES
    }
}

/// The handle closest to `p` within `radius` pixels.
pub fn hit_handle(sel: Rect, handles: &[Handle], p: Point, radius: f32) -> Option<Handle> {
    let (px, py) = (p.x as f32 + 0.5, p.y as f32 + 0.5);
    handles
        .iter()
        .map(|&h| {
            let (cx, cy) = handle_center(sel, h);
            (h, (px - cx).powi(2) + (py - cy).powi(2))
        })
        .filter(|&(_, d2)| d2 <= radius * radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(h, _)| h)
}

/// One axis of a resize: moves the `start` or `end` edge by `delta`. When
/// the moved edge crosses the other one, the range flips and keeps the
/// pixel of the fixed edge.
fn resize_axis(start: i32, end: i32, side: Side, delta: i32, min: i32, max: i32) -> (i32, i32) {
    match side {
        Side::Middle => (start, end),
        Side::Start => {
            let moved = (start + delta).clamp(min, max);
            if moved < end {
                (moved, end)
            } else {
                (end - 1, (moved + 1).min(max))
            }
        }
        Side::End => {
            let moved = (end + delta).clamp(min, max);
            if moved > start {
                (start, moved)
            } else {
                ((moved - 1).max(min), start + 1)
            }
        }
    }
}

/// `start` resized by dragging `handle` by (`dx`, `dy`), inside `bounds`.
pub fn resized(start: Rect, handle: Handle, dx: i32, dy: i32, bounds: Rect) -> Rect {
    let (h, v) = handle.sides();
    let (left, right) = resize_axis(start.left, start.right, h, dx, bounds.left, bounds.right);
    let (top, bottom) = resize_axis(start.top, start.bottom, v, dy, bounds.top, bounds.bottom);
    Rect::new(left, top, right, bottom)
}

/// `start` moved by (`dx`, `dy`), kept inside `bounds`.
pub fn moved(start: Rect, dx: i32, dy: i32, bounds: Rect) -> Rect {
    start.offset(dx, dy).moved_inside(bounds)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOUNDS: Rect = Rect::new(0, 0, 1000, 800);

    #[test]
    fn handles_sit_on_the_border() {
        let sel = Rect::new(10, 20, 110, 220);
        assert_eq!(handle_center(sel, Handle::TopLeft), (10.0, 20.0));
        assert_eq!(handle_center(sel, Handle::Bottom), (60.0, 220.0));
        assert_eq!(handle_center(sel, Handle::Right), (110.0, 120.0));
    }

    #[test]
    fn small_selections_show_only_corners() {
        assert_eq!(visible_handles(Rect::new(0, 0, 40, 400), 14.0).len(), 4);
        assert_eq!(visible_handles(Rect::new(0, 0, 60, 60), 14.0).len(), 8);
    }

    #[test]
    fn hit_test_picks_the_nearest_handle() {
        let sel = Rect::new(100, 100, 300, 300);
        let hit = |x, y| hit_handle(sel, &ALL_HANDLES, Point::new(x, y), 10.0);
        assert_eq!(hit(101, 99), Some(Handle::TopLeft));
        assert_eq!(hit(200, 302), Some(Handle::Bottom));
        assert_eq!(hit(150, 150), None);
    }

    #[test]
    fn resize_moves_only_the_dragged_edges() {
        let sel = Rect::new(100, 100, 300, 300);
        assert_eq!(
            resized(sel, Handle::BottomRight, 50, -20, BOUNDS),
            Rect::new(100, 100, 350, 280)
        );
        assert_eq!(
            resized(sel, Handle::Left, -30, 99, BOUNDS),
            Rect::new(70, 100, 300, 300)
        );
    }

    #[test]
    fn resize_past_the_opposite_edge_flips() {
        let sel = Rect::new(100, 100, 200, 200);
        // The right edge dragged to x = 50: the range now ends at the old
        // left edge, which stays included.
        assert_eq!(
            resized(sel, Handle::Right, -150, 0, BOUNDS),
            Rect::new(49, 100, 101, 200)
        );
        // Exactly onto the opposite edge never gives an empty range.
        let r = resized(sel, Handle::Top, 0, 100, BOUNDS);
        assert!(!r.is_empty());
    }

    #[test]
    fn resize_stays_inside_bounds() {
        let sel = Rect::new(100, 100, 200, 200);
        assert_eq!(
            resized(sel, Handle::TopLeft, -500, -500, BOUNDS),
            Rect::new(0, 0, 200, 200)
        );
        assert_eq!(
            resized(sel, Handle::BottomRight, 5000, 5000, BOUNDS),
            Rect::new(100, 100, 1000, 800)
        );
    }

    #[test]
    fn move_keeps_the_size_inside_bounds() {
        let sel = Rect::new(100, 100, 200, 200);
        assert_eq!(moved(sel, 950, 0, BOUNDS), Rect::new(900, 100, 1000, 200));
    }
}
