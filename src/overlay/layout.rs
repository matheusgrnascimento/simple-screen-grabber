//! Where the overlay's UI elements go around the selection. Pure logic in
//! physical pixels; sizes are already scaled for the monitor.

use crate::geometry::{Rect, RectF};

/// The size label: above the selection's top-left corner, or inside it
/// when there is no room above on the monitor. Kept on the monitor.
pub fn size_label(sel: Rect, monitor: Rect, width: f32, height: f32, gap: f32) -> RectF {
    let mon = RectF::from(monitor);
    let left = (sel.left as f32).clamp(mon.left, (mon.right - width).max(mon.left));
    let above = sel.top as f32 - gap - height;
    let top = if above >= mon.top {
        above
    } else {
        (sel.top as f32 + gap).clamp(mon.top, (mon.bottom - height).max(mon.top))
    };
    let left = if above >= mon.top { left } else { left + gap };
    RectF::sized(left, top, width, height).snapped()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn label_sits_above_the_selection() {
        let r = size_label(Rect::new(100, 200, 400, 500), MONITOR, 80.0, 24.0, 8.0);
        assert_eq!(r, RectF::new(100.0, 168.0, 180.0, 192.0));
    }

    #[test]
    fn label_moves_inside_at_the_top_edge() {
        let r = size_label(Rect::new(100, 10, 400, 500), MONITOR, 80.0, 24.0, 8.0);
        assert_eq!(r, RectF::new(108.0, 18.0, 188.0, 42.0));
    }

    #[test]
    fn label_stays_on_the_monitor() {
        let r = size_label(Rect::new(1900, 200, 1920, 300), MONITOR, 80.0, 24.0, 8.0);
        assert_eq!((r.left, r.right), (1840.0, 1920.0));
    }
}
