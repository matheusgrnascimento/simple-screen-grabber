//! The two toolbars next to the selection and the color flyout: their
//! buttons, placement and hit testing. Pure logic in physical pixels.

use crate::annotate::{Tool, COLORS};
use crate::geometry::{Point, Rect, RectF};

/// A toolbar button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Tool(Tool),
    Color,
    Undo,
    Redo,
    Copy,
    Save,
    Close,
}

impl Button {
    /// The tooltip: the action and its shortcut.
    pub fn tooltip(self) -> String {
        match self {
            Button::Tool(tool) => format!("{} ({})", tool.name(), char::from(tool.key())),
            Button::Color => "Color (1\u{2013}8)".into(),
            Button::Undo => "Undo (Ctrl+Z)".into(),
            Button::Redo => "Redo (Ctrl+Y)".into(),
            Button::Copy => "Copy (Ctrl+C)".into(),
            Button::Save => "Save (Ctrl+S)".into(),
            Button::Close => "Close (Esc)".into(),
        }
    }
}

/// The tools bar, top to bottom; `None` is a separator.
const TOOLS: [Option<Button>; 9] = [
    Some(Button::Tool(Tool::Arrow)),
    Some(Button::Tool(Tool::Rectangle)),
    Some(Button::Tool(Tool::Line)),
    Some(Button::Tool(Tool::Pen)),
    None,
    Some(Button::Color),
    None,
    Some(Button::Undo),
    Some(Button::Redo),
];

/// The actions bar, left to right.
const ACTIONS: [Option<Button>; 4] = [
    Some(Button::Copy),
    Some(Button::Save),
    None,
    Some(Button::Close),
];

/// Sizes in DIPs, multiplied by the monitor scale.
pub mod metrics {
    pub const BUTTON: f32 = 36.0;
    pub const PADDING: f32 = 4.0;
    pub const SPACING: f32 = 2.0;
    /// A separator: a 1 DIP line with this margin on each side.
    pub const SEPARATOR_MARGIN: f32 = 4.0;
    /// Between the selection and a bar, and between bars.
    pub const GAP: f32 = 8.0;
    /// Kept between a bar and the monitor edges.
    pub const MARGIN: f32 = 8.0;
    pub const SWATCH: f32 = 32.0;
    pub const SWATCH_COLUMNS: usize = 4;
}

/// One slot of a bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    Button(Button, RectF),
    /// The separator line's rectangle (1 DIP thick).
    Separator(RectF),
}

/// A bar and its slots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bar<const N: usize> {
    pub rect: RectF,
    pub vertical: bool,
    pub slots: [Slot; N],
}

/// The color flyout: one swatch per color.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flyout {
    pub rect: RectF,
    pub swatches: [RectF; COLORS.len()],
}

/// Everything placed around a selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Toolbars {
    pub scale: f32,
    pub tools: Bar<9>,
    pub actions: Bar<4>,
    pub flyout: Option<Flyout>,
}

/// What is under the mouse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Button(Button),
    Swatch(usize),
    /// A bar or the flyout, between buttons.
    Background,
}

fn px(dips: f32, scale: f32) -> f32 {
    (dips * scale).round()
}

/// The length of a bar along its direction.
fn bar_length(items: &[Option<Button>], scale: f32) -> f32 {
    let mut length = 2.0 * px(metrics::PADDING, scale);
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            length += px(metrics::SPACING, scale);
        }
        length += match item {
            Some(_) => px(metrics::BUTTON, scale),
            None => 1.0f32.max(scale.round()) + 2.0 * px(metrics::SEPARATOR_MARGIN, scale),
        };
    }
    length
}

fn bar_thickness(scale: f32) -> f32 {
    px(metrics::BUTTON, scale) + 2.0 * px(metrics::PADDING, scale)
}

fn layout_bar<const N: usize>(
    items: &[Option<Button>; N],
    left: f32,
    top: f32,
    vertical: bool,
    scale: f32,
) -> Bar<N> {
    let (length, thickness) = (bar_length(items, scale), bar_thickness(scale));
    let rect = if vertical {
        RectF::sized(left, top, thickness, length)
    } else {
        RectF::sized(left, top, length, thickness)
    };
    let pad = px(metrics::PADDING, scale);
    let button = px(metrics::BUTTON, scale);
    let line = 1.0f32.max(scale.round());
    let margin = px(metrics::SEPARATOR_MARGIN, scale);
    let mut along = pad;
    let mut slots = [Slot::Separator(RectF::default()); N];
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            along += px(metrics::SPACING, scale);
        }
        let (size, slot) = match item {
            Some(b) => {
                let r = if vertical {
                    RectF::sized(left + pad, top + along, button, button)
                } else {
                    RectF::sized(left + along, top + pad, button, button)
                };
                (button, Slot::Button(*b, r))
            }
            None => {
                let size = line + 2.0 * margin;
                let r = if vertical {
                    RectF::sized(
                        left + pad + margin,
                        top + along + margin,
                        button - 2.0 * margin,
                        line,
                    )
                } else {
                    RectF::sized(
                        left + along + margin,
                        top + pad + margin,
                        line,
                        button - 2.0 * margin,
                    )
                };
                (size, Slot::Separator(r))
            }
        };
        slots[i] = slot;
        along += size;
    }
    Bar {
        rect,
        vertical,
        slots,
    }
}

/// Places the toolbars around `sel` on `monitor`:
///
/// - the tools bar (vertical) to the right of the selection, top-aligned;
///   to its left when there is no room, or inside it when neither side
///   has room;
/// - the actions bar (horizontal) below the selection, right-aligned;
///   above it, or inside it, likewise;
/// - the color flyout next to the color button, when open.
pub fn toolbars(sel: Rect, monitor: Rect, scale: f32, flyout_open: bool) -> Toolbars {
    let mon = RectF::from(monitor);
    let sel = RectF::from(sel);
    let gap = px(metrics::GAP, scale);
    let margin = px(metrics::MARGIN, scale);
    let (min_x, max_x) = (mon.left + margin, mon.right - margin);
    let (min_y, max_y) = (mon.top + margin, mon.bottom - margin);
    let clamp = |v: f32, min: f32, max: f32| v.min(max).max(min);

    let (tools_w, tools_h) = (bar_thickness(scale), bar_length(&TOOLS, scale));
    let tools_x = if sel.right + gap + tools_w <= max_x {
        sel.right + gap
    } else if sel.left - gap - tools_w >= min_x {
        sel.left - gap - tools_w
    } else {
        clamp(sel.right - gap - tools_w, min_x, max_x - tools_w)
    };
    let tools_y = clamp(sel.top, min_y, max_y - tools_h);
    let tools = layout_bar(&TOOLS, tools_x, tools_y, true, scale);

    let (actions_w, actions_h) = (bar_length(&ACTIONS, scale), bar_thickness(scale));
    let actions_y = if sel.bottom + gap + actions_h <= max_y {
        sel.bottom + gap
    } else if sel.top - gap - actions_h >= min_y {
        sel.top - gap - actions_h
    } else {
        clamp(sel.bottom - gap - actions_h, min_y, max_y - actions_h)
    };
    let mut actions_x = clamp(sel.right - actions_w, min_x, max_x - actions_w);
    let actions_rect = RectF::sized(actions_x, actions_y, actions_w, actions_h);
    if overlaps(actions_rect, tools.rect) {
        actions_x = clamp(tools.rect.left - gap - actions_w, min_x, max_x - actions_w);
    }
    let actions = layout_bar(&ACTIONS, actions_x, actions_y, false, scale);

    let flyout = flyout_open.then(|| {
        let color = tools
            .slots
            .iter()
            .find_map(|s| match s {
                Slot::Button(Button::Color, r) => Some(*r),
                _ => None,
            })
            .unwrap_or(tools.rect);
        flyout(tools.rect, color, mon, scale)
    });

    Toolbars {
        scale,
        tools,
        actions,
        flyout,
    }
}

fn overlaps(a: RectF, b: RectF) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// The color flyout beside the tools bar, top-aligned with the color
/// button.
fn flyout(bar: RectF, button: RectF, mon: RectF, scale: f32) -> Flyout {
    let pad = px(metrics::PADDING, scale);
    let spacing = px(metrics::SPACING, scale);
    let swatch = px(metrics::SWATCH, scale);
    let gap = px(metrics::GAP, scale);
    let margin = px(metrics::MARGIN, scale);
    let columns = metrics::SWATCH_COLUMNS;
    let rows = COLORS.len().div_ceil(columns);
    let width = 2.0 * pad + columns as f32 * swatch + (columns - 1) as f32 * spacing;
    let height = 2.0 * pad + rows as f32 * swatch + (rows - 1) as f32 * spacing;
    let left = if bar.right + gap + width <= mon.right - margin {
        bar.right + gap
    } else {
        bar.left - gap - width
    };
    let top = (button.top - pad)
        .min(mon.bottom - margin - height)
        .max(mon.top + margin);
    let mut swatches = [RectF::default(); COLORS.len()];
    for (i, r) in swatches.iter_mut().enumerate() {
        let (col, row) = (i % columns, i / columns);
        *r = RectF::sized(
            left + pad + col as f32 * (swatch + spacing),
            top + pad + row as f32 * (swatch + spacing),
            swatch,
            swatch,
        );
    }
    Flyout {
        rect: RectF::sized(left, top, width, height),
        swatches,
    }
}

impl Toolbars {
    /// What is under `p`, with the flyout on top.
    pub fn hit(&self, p: Point) -> Option<Hit> {
        if let Some(flyout) = &self.flyout {
            if flyout.rect.contains(p) {
                let swatch = flyout.swatches.iter().position(|r| r.contains(p));
                return Some(swatch.map_or(Hit::Background, Hit::Swatch));
            }
        }
        for (rect, slots) in [
            (self.tools.rect, &self.tools.slots[..]),
            (self.actions.rect, &self.actions.slots[..]),
        ] {
            if rect.contains(p) {
                let button = slots.iter().find_map(|s| match s {
                    Slot::Button(b, r) if r.contains(p) => Some(*b),
                    _ => None,
                });
                return Some(button.map_or(Hit::Background, Hit::Button));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect::new(0, 0, 1920, 1080);

    #[test]
    fn bars_sit_beside_and_below_the_selection() {
        let sel = Rect::new(400, 300, 1000, 700);
        let bars = toolbars(sel, MONITOR, 1.0, false);
        assert_eq!(bars.tools.rect.left, 1008.0);
        assert_eq!(bars.tools.rect.top, 300.0);
        assert_eq!(bars.tools.rect.width(), 44.0);
        assert_eq!(bars.actions.rect.top, 708.0);
        assert_eq!(bars.actions.rect.right, 1000.0);
    }

    #[test]
    fn bars_move_inside_a_full_screen_selection() {
        let bars = toolbars(MONITOR, MONITOR, 1.0, false);
        assert!(bars.tools.rect.right <= 1920.0 - 8.0);
        assert!(bars.actions.rect.bottom <= 1080.0 - 8.0);
        assert!(!overlaps(bars.tools.rect, bars.actions.rect));
    }

    #[test]
    fn tools_bar_goes_left_at_the_right_edge() {
        let bars = toolbars(Rect::new(1500, 100, 1900, 400), MONITOR, 1.0, false);
        assert_eq!(bars.tools.rect.right, 1500.0 - 8.0);
    }

    #[test]
    fn sizes_follow_the_display_scale() {
        let sel = Rect::new(400, 300, 1000, 700);
        let one = toolbars(sel, MONITOR, 1.0, false);
        let two = toolbars(sel, MONITOR, 2.0, false);
        assert_eq!(two.tools.rect.width(), 2.0 * one.tools.rect.width());
    }

    #[test]
    fn hit_finds_buttons_and_swatches() {
        let bars = toolbars(Rect::new(400, 300, 1000, 700), MONITOR, 1.0, true);
        let Slot::Button(first, r) = bars.tools.slots[0] else {
            panic!("the first slot is a button");
        };
        let center = Point::new(
            (r.left + r.width() / 2.0) as i32,
            (r.top + r.height() / 2.0) as i32,
        );
        assert_eq!(bars.hit(center), Some(Hit::Button(first)));
        let flyout = bars.flyout.unwrap();
        let s = flyout.swatches[5];
        assert_eq!(
            bars.hit(Point::new(s.left as i32 + 3, s.top as i32 + 3)),
            Some(Hit::Swatch(5))
        );
        assert_eq!(bars.hit(Point::new(5, 5)), None);
    }
}
