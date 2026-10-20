//! Snapping floating windows to output edges in stacking mode.
//!
//! Dragging a window to the left or right edge snaps it to that half of the working area, to a
//! corner snaps it to a quarter, and to the top edge maximizes it.

use smithay::utils::{Logical, Point, Rectangle, Size};

/// Distance from an output edge where snapping triggers.
///
/// Wide enough to not require pixel-precise aiming, and to work on edges shared between two
/// outputs, where the pointer does not stop.
const EDGE: f64 = 20.;

/// Part of the edge length near a corner where snapping goes to the quarter.
const CORNER_FRAC: f64 = 0.15;

/// Minimum length of the corner part of an edge.
const CORNER_MIN: f64 = 120.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapZone {
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Maximize,
}

/// Where a dragged window goes on release.
#[derive(Debug, Clone)]
pub enum SnapTarget<Id> {
    /// A zone at the output edge.
    Zone(SnapZone),
    /// The top or bottom half of a snapped window, which shrinks into the other half.
    Split { neighbor: Id, top: bool },
}

/// Geometry of a snapped window.
#[derive(Debug, Clone, Copy)]
pub struct SnapState {
    /// Tile rectangle in workspace coordinates.
    pub rect: Rectangle<f64, Logical>,
    /// Window size from before the window was snapped.
    pub restore: Size<i32, Logical>,
}

/// Returns the snap zone under the pointer `p`.
///
/// `view` is the output size and `area` is its working area. Exclusive zones at the top (such as
/// a bar) count as the top edge.
pub fn zone_at(
    view: Size<f64, Logical>,
    area: Rectangle<f64, Logical>,
    p: Point<f64, Logical>,
) -> Option<SnapZone> {
    let left = p.x <= EDGE;
    let right = p.x >= view.w - EDGE;
    let top = p.y <= f64::max(EDGE, area.loc.y);
    let bottom = p.y >= view.h - EDGE;
    let corner_h = f64::max(CORNER_MIN, view.h * CORNER_FRAC);
    let corner_w = f64::max(CORNER_MIN, view.w * CORNER_FRAC);
    let near_top = p.y < area.loc.y + corner_h;
    let near_bottom = p.y > view.h - corner_h;
    let near_left = p.x < corner_w;
    let near_right = p.x > view.w - corner_w;

    if (left && near_top) || (top && near_left) {
        Some(SnapZone::TopLeft)
    } else if (right && near_top) || (top && near_right) {
        Some(SnapZone::TopRight)
    } else if (left && near_bottom) || (bottom && near_left) {
        Some(SnapZone::BottomLeft)
    } else if (right && near_bottom) || (bottom && near_right) {
        Some(SnapZone::BottomRight)
    } else if left {
        Some(SnapZone::Left)
    } else if right {
        Some(SnapZone::Right)
    } else if top {
        Some(SnapZone::Maximize)
    } else {
        None
    }
}

/// Returns the rectangle of a snap zone within the working area.
pub fn zone_rect(zone: SnapZone, area: Rectangle<f64, Logical>) -> Rectangle<f64, Logical> {
    let (x, y) = (area.loc.x, area.loc.y);
    let (w, h) = (area.size.w, area.size.h);
    let (hw, hh) = ((w / 2.).round(), (h / 2.).round());
    let (loc, size) = match zone {
        SnapZone::Left => ((x, y), (hw, h)),
        SnapZone::Right => ((x + hw, y), (w - hw, h)),
        SnapZone::TopLeft => ((x, y), (hw, hh)),
        SnapZone::TopRight => ((x + hw, y), (w - hw, hh)),
        SnapZone::BottomLeft => ((x, y + hh), (hw, h - hh)),
        SnapZone::BottomRight => ((x + hw, y + hh), (w - hw, h - hh)),
        SnapZone::Maximize => ((x, y), (w, h)),
    };
    Rectangle::new(Point::from(loc), Size::from(size))
}

/// Returns the top or bottom half of a rectangle.
pub fn split_rect(rect: Rectangle<f64, Logical>, top: bool) -> Rectangle<f64, Logical> {
    let hh = (rect.size.h / 2.).round();
    if top {
        Rectangle::new(rect.loc, Size::from((rect.size.w, hh)))
    } else {
        Rectangle::new(
            Point::from((rect.loc.x, rect.loc.y + hh)),
            Size::from((rect.size.w, rect.size.h - hh)),
        )
    }
}

/// Returns whether two rectangles are equal up to rounding.
pub fn rects_match(a: Rectangle<f64, Logical>, b: Rectangle<f64, Logical>) -> bool {
    (a.loc.x - b.loc.x).abs() < 1.
        && (a.loc.y - b.loc.y).abs() < 1.
        && (a.size.w - b.size.w).abs() < 1.
        && (a.size.h - b.size.h).abs() < 1.
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view() -> Size<f64, Logical> {
        Size::from((1920., 1080.))
    }

    fn area() -> Rectangle<f64, Logical> {
        Rectangle::new(Point::from((0., 30.)), Size::from((1920., 1050.)))
    }

    fn zone(x: f64, y: f64) -> Option<SnapZone> {
        zone_at(view(), area(), Point::from((x, y)))
    }

    #[test]
    fn zones() {
        assert_eq!(zone(960., 540.), None);
        assert_eq!(zone(0., 540.), Some(SnapZone::Left));
        assert_eq!(zone(1919., 540.), Some(SnapZone::Right));
        assert_eq!(zone(960., 0.), Some(SnapZone::Maximize));
        // The top bar counts as the top edge.
        assert_eq!(zone(960., 29.), Some(SnapZone::Maximize));
        assert_eq!(zone(0., 100.), Some(SnapZone::TopLeft));
        assert_eq!(zone(100., 0.), Some(SnapZone::TopLeft));
        assert_eq!(zone(1919., 1079.), Some(SnapZone::BottomRight));
        assert_eq!(zone(100., 1079.), Some(SnapZone::BottomLeft));
        assert_eq!(zone(960., 1079.), None);
    }

    #[test]
    fn zone_rects_tile_the_area() {
        let area = Rectangle::new(Point::from((0., 30.)), Size::from((1921., 1051.)));
        let left = zone_rect(SnapZone::Left, area);
        let right = zone_rect(SnapZone::Right, area);
        assert_eq!(left.size.w + right.size.w, area.size.w);
        assert_eq!(left.loc.x + left.size.w, right.loc.x);

        let tl = zone_rect(SnapZone::TopLeft, area);
        let bl = zone_rect(SnapZone::BottomLeft, area);
        assert_eq!(tl.size.h + bl.size.h, area.size.h);
        assert_eq!(tl.loc.y + tl.size.h, bl.loc.y);
        assert!(rects_match(zone_rect(SnapZone::Maximize, area), area));
    }

    #[test]
    fn split() {
        let rect = Rectangle::new(Point::from((10., 20.)), Size::from((100., 301.)));
        let top = split_rect(rect, true);
        let bottom = split_rect(rect, false);
        assert_eq!(top.loc, rect.loc);
        assert_eq!(top.loc.y + top.size.h, bottom.loc.y);
        assert_eq!(top.size.h + bottom.size.h, rect.size.h);
    }
}
