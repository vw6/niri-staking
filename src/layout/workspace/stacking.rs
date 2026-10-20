//! Stacking mode on a single workspace.
//!
//! Entering stacking moves every tiled window into the floating space, keeping it where it was on
//! screen, and remembers its place in the scrolling layout. Leaving stacking puts those windows
//! back into their columns; windows opened during stacking go to the end of the strip, and windows
//! that were floating before stay floating.

use niri_ipc::{PositionChange, SizeChange};
use smithay::utils::{Logical, Point, Rectangle, Size};

use super::{FloatingActive, Workspace};
use crate::animation::Animation;
use crate::layout::snap::{
    rects_match, split_rect, zone_at, zone_rect, SnapState, SnapTarget, SnapZone,
};
use crate::layout::tile::StackingOrigin;
use crate::layout::LayoutElement;
use crate::utils::transaction::Transaction;
use crate::utils::ResizeEdge;

/// Minimum size of a snapped window side when resizing a snap group.
const SNAP_RESIZE_MIN: f64 = 100.;

/// Minimum height of a snapped window that can be split in two by dropping a window onto it.
const SPLIT_MIN_HEIGHT: f64 = 300.;

/// Where a dragged window will go on release, and the rectangle to show as the hint.
pub type SnapHit<Id> = (SnapTarget<Id>, Rectangle<f64, Logical>);

impl<W: LayoutElement> Workspace<W> {
    fn render_positions(&self) -> Vec<(W::Id, Point<f64, Logical>)> {
        self.tiles_with_render_positions()
            .map(|(tile, pos, _)| (tile.window().id().clone(), pos))
            .collect()
    }

    fn animate_from(&mut self, id: &W::Id, old: &[(W::Id, Point<f64, Logical>)]) {
        let Some((_, from)) = old.iter().find(|(x, _)| x == id) else {
            return;
        };
        if let Some((tile, to)) = self
            .tiles_with_render_positions_mut(false)
            .find(|(tile, _)| tile.window().id() == id)
        {
            tile.animate_move_from(*from - to);
        }
    }

    pub fn enter_stacking(&mut self) {
        let active = self.active_window().map(|w| w.id().clone());
        let old = self.render_positions();

        for tile in self.floating.tiles_mut() {
            tile.stacking_origin = Some(StackingOrigin::Floating);
        }

        let mut places = Vec::new();
        for (col, column) in self.scrolling.columns().enumerate() {
            for (in_col, (tile, _)) in column.tiles().enumerate() {
                places.push((tile.window().id().clone(), col, in_col));
            }
        }

        for (id, col, in_col) in places {
            let mut removed = self.scrolling.remove_tile(&id, Transaction::new());
            removed.tile.stop_move_animations();
            removed.tile.stacking_origin = Some(StackingOrigin::Column {
                col,
                tile: in_col,
                width: removed.width,
                is_full_width: removed.is_full_width,
            });

            // Keep the window where it is on screen unless it has a stored floating position.
            if self
                .floating
                .stored_or_default_tile_pos(&removed.tile)
                .is_none()
            {
                let pos = old
                    .iter()
                    .find(|(x, _)| *x == id)
                    .map(|(_, pos)| *pos)
                    .unwrap_or_default();
                let size = removed.tile.tile_size();
                let pos = self.floating.clamp_within_working_area(pos, size);
                removed.tile.floating_pos = Some(self.floating.logical_to_size_frac(pos));
            }

            let activate = active.as_ref() == Some(&id);
            self.floating.add_tile(removed.tile, activate);
            self.animate_from(&id, &old);
        }

        if active.is_some() {
            self.floating_is_active = FloatingActive::Yes;
        }
    }

    pub fn leave_stacking(&mut self) {
        let active = self.active_window().map(|w| w.id().clone());
        let old = self.render_positions();

        // Windows that came from columns go back in column order. Windows opened during stacking go
        // to the end, unless a rule opens them floating.
        let mut back = Vec::new();
        for tile in self.floating.tiles_mut() {
            match tile.stacking_origin.take() {
                Some(StackingOrigin::Column {
                    col,
                    tile: in_col,
                    width,
                    is_full_width,
                }) => {
                    let id = tile.window().id().clone();
                    back.push((id, (col, in_col), Some((width, is_full_width))));
                }
                Some(StackingOrigin::Floating) => (),
                None if tile.window().rules().open_floating == Some(true) => (),
                None => back.push((tile.window().id().clone(), (usize::MAX, 0), None)),
            }
        }
        back.sort_by_key(|(_, place, _)| *place);

        // Original column and its new index, to put the following tiles of that column into it.
        let mut placed: Option<(usize, usize)> = None;
        for (id, (col, in_col), width) in back {
            let mut removed = self.floating.remove_tile(&id);
            removed.tile.stop_move_animations();
            removed.tile.minimized = false;
            removed.tile.restore_geometry = None;
            removed.tile.snap = None;
            let count = self.scrolling.columns().count();

            match (width, placed) {
                (Some(_), Some((orig, idx))) if orig == col && in_col > 0 => {
                    self.scrolling
                        .add_tile_to_column(idx, None, removed.tile, false);
                }
                (Some((width, is_full_width)), _) => {
                    let idx = col.min(count);
                    self.scrolling.add_tile(
                        Some(idx),
                        removed.tile,
                        false,
                        width,
                        is_full_width,
                        None,
                    );
                    placed = Some((col, idx));
                }
                (None, _) => {
                    self.scrolling.add_tile(
                        Some(count),
                        removed.tile,
                        false,
                        removed.width,
                        false,
                        None,
                    );
                }
            }
            self.animate_from(&id, &old);
        }

        if let Some(id) = active {
            self.activate_window(&id);
        }
    }

    /// Minimizes a floating window. When nothing is left to focus in the floating space, focus
    /// moves to the scrolling layout if it has windows.
    pub fn minimize_window(&mut self, id: &W::Id) -> bool {
        if !self.floating.has_window(id) {
            return false;
        }
        if !self.floating.minimize_window(id) && !self.scrolling.is_empty() {
            self.floating_is_active = FloatingActive::No;
        }
        true
    }

    /// Sets the tile position (in workspace coordinates) and the window size of a floating window.
    fn set_floating_geometry(
        &mut self,
        id: &W::Id,
        pos: Point<f64, Logical>,
        size: Size<i32, Logical>,
        animate: bool,
    ) {
        let area = self.floating.working_area();
        self.floating
            .set_window_width(Some(id), SizeChange::SetFixed(size.w), animate);
        self.floating
            .set_window_height(Some(id), SizeChange::SetFixed(size.h), animate);
        self.floating.move_window(
            Some(id),
            PositionChange::SetFixed(pos.x - area.loc.x),
            PositionChange::SetFixed(pos.y - area.loc.y),
            animate,
        );
    }

    /// Maximizes a floating window to the working area, or restores its previous geometry.
    pub fn toggle_stacking_maximize(&mut self, id: &W::Id) -> bool {
        let area = self.floating.working_area();
        let Some((tile, pos)) = self
            .floating
            .tiles_with_offsets_mut()
            .find(|(tile, _)| tile.window().id() == id)
        else {
            return false;
        };

        match tile.restore_geometry.take() {
            Some((old_pos, old_size)) => {
                tile.snap = None;
                self.set_floating_geometry(id, old_pos, old_size, true);
            }
            None => {
                let win = tile.window();
                let size = win.expected_size().unwrap_or_else(|| win.size());
                tile.restore_geometry = Some((pos, size));
                let full = Size::from((
                    tile.window_width_for_tile_width(area.size.w).round() as i32,
                    tile.window_height_for_tile_height(area.size.h).round() as i32,
                ));
                self.set_floating_geometry(id, area.loc, full, true);
            }
        }
        true
    }

    /// Restores the size of a maximized or snapped window that started being dragged by its
    /// titlebar. `anchor` is the pointer position in workspace coordinates.
    pub fn unmaximize_for_drag(&mut self, id: &W::Id, anchor: Point<f64, Logical>) -> bool {
        let Some((tile, pos)) = self
            .floating
            .tiles_with_offsets_mut()
            .find(|(tile, _)| tile.window().id() == id)
        else {
            return false;
        };
        let old_size = match (tile.restore_geometry.take(), tile.snap.take()) {
            (Some((_, size)), _) => size,
            (None, Some(snap)) => snap.restore,
            (None, None) => return false,
        };
        // The move grab keeps the pointer at the same fraction of the window width, and the window
        // shrinks only once it commits. Center the current tile under the pointer so that the
        // pointer ends up in the middle of the titlebar after the resize.
        let new_pos = Point::from((anchor.x - tile.tile_size().w / 2., pos.y));
        self.set_floating_geometry(id, new_pos, old_size, false);
        true
    }

    /// Snaps a window into `zone`. The top edge maximizes.
    pub fn snap_window(&mut self, id: &W::Id, zone: SnapZone) -> bool {
        if zone == SnapZone::Maximize {
            let maximized = self
                .floating
                .tiles()
                .any(|t| t.window().id() == id && t.restore_geometry.is_some());
            return maximized || self.toggle_stacking_maximize(id);
        }

        let area = self.floating.working_area();
        let rect = zone_rect(zone, area);
        if !self.place_snapped(id, rect) {
            return false;
        }

        // A quarter splits the half that already holds a snapped window: that window moves into the
        // other quarter.
        let (half, other) = match zone {
            SnapZone::TopLeft => (SnapZone::Left, SnapZone::BottomLeft),
            SnapZone::BottomLeft => (SnapZone::Left, SnapZone::TopLeft),
            SnapZone::TopRight => (SnapZone::Right, SnapZone::BottomRight),
            SnapZone::BottomRight => (SnapZone::Right, SnapZone::TopRight),
            _ => return true,
        };
        let half = zone_rect(half, area);
        let neighbor = self
            .floating
            .tiles()
            .find(|t| {
                t.window().id() != id
                    && !t.minimized
                    && t.snap.is_some_and(|s| rects_match(s.rect, half))
            })
            .map(|t| t.window().id().clone());
        if let Some(neighbor) = neighbor {
            self.place_snapped(&neighbor, zone_rect(other, area));
        }
        true
    }

    /// Returns where the dragged window `moving` will go if released with the pointer at `p`.
    ///
    /// Near the output edges this is a snap zone. Over the top or bottom quarter of a snapped
    /// window, this is the corresponding half of that window.
    pub fn snap_target_at(&self, moving: &W::Id, p: Point<f64, Logical>) -> Option<SnapHit<W::Id>> {
        let area = self.floating.working_area();
        if let Some(zone) = zone_at(self.view_size, area, p) {
            return Some((SnapTarget::Zone(zone), zone_rect(zone, area)));
        }

        // Tiles go from top to bottom, so this finds the topmost window under the pointer.
        let tile = self.floating.tiles_with_offsets().find_map(|(tile, pos)| {
            let rect = Rectangle::new(pos, tile.tile_size());
            (!tile.minimized && rect.contains(p)).then_some(tile)
        })?;
        let rect = tile.snap?.rect;
        if tile.window().id() == moving || rect.size.h < SPLIT_MIN_HEIGHT || !rect.contains(p) {
            return None;
        }
        let top = if p.y < rect.loc.y + rect.size.h / 4. {
            true
        } else if p.y > rect.loc.y + rect.size.h * 3. / 4. {
            false
        } else {
            return None;
        };
        let target = SnapTarget::Split {
            neighbor: tile.window().id().clone(),
            top,
        };
        Some((target, split_rect(rect, top)))
    }

    /// Updates the snap hint. A new hint grows out of `from` (the dragged window), and a changed
    /// hint animates from where it is currently shown.
    pub fn set_snap_hint(
        &mut self,
        new: Option<Rectangle<f64, Logical>>,
        from: Rectangle<f64, Logical>,
    ) {
        let old = self.snap_hint_prev.take();
        self.snap_hint = new;
        let Some(new) = new else {
            self.snap_hint_anim = None;
            return;
        };
        if old == Some(new) {
            return;
        }
        let start = old.map_or(from, |old| self.animated_snap_hint(old));
        let anim = Animation::new(
            self.clock.clone(),
            0.,
            1.,
            0.,
            self.options.animations.window_movement.0,
        );
        self.snap_hint_anim = Some((anim, start));
    }

    /// Returns the currently shown snap hint while it animates towards `rect`.
    pub fn animated_snap_hint(&self, rect: Rectangle<f64, Logical>) -> Rectangle<f64, Logical> {
        let Some((anim, from)) = &self.snap_hint_anim else {
            return rect;
        };
        let t = anim.value();
        let lerp = |a: f64, b: f64| a + (b - a) * t;
        Rectangle::new(
            Point::from((lerp(from.loc.x, rect.loc.x), lerp(from.loc.y, rect.loc.y))),
            Size::from((
                lerp(from.size.w, rect.size.w).max(1.),
                lerp(from.size.h, rect.size.h).max(1.),
            )),
        )
    }

    pub fn set_taskbar_rect(&mut self, id: &W::Id, rect: Rectangle<f64, Logical>) {
        self.floating.set_taskbar_rect(id, rect);
    }

    /// Puts a dropped window where the snap hint showed.
    pub fn apply_snap_target(&mut self, id: &W::Id, target: SnapTarget<W::Id>) {
        match target {
            SnapTarget::Zone(zone) => {
                self.snap_window(id, zone);
            }
            SnapTarget::Split { neighbor, top } => {
                let Some(rect) = self
                    .floating
                    .tiles()
                    .find(|t| *t.window().id() == neighbor)
                    .and_then(|t| t.snap)
                    .map(|s| s.rect)
                else {
                    return;
                };
                if self.place_snapped(id, split_rect(rect, top)) {
                    self.place_snapped(&neighbor, split_rect(rect, !top));
                }
            }
        }
    }

    /// Snaps a window into `rect`, remembering its size from before it was first snapped.
    fn place_snapped(&mut self, id: &W::Id, rect: Rectangle<f64, Logical>) -> bool {
        let Some(tile) = self.floating.tiles_mut().find(|t| t.window().id() == id) else {
            return false;
        };
        let win = tile.window();
        let current = win.expected_size().unwrap_or_else(|| win.size());
        let restore = tile.snap.map_or(current, |s| s.restore);
        tile.restore_geometry = None;
        tile.snap = Some(SnapState { rect, restore });
        let size = Size::from((
            tile.window_width_for_tile_width(rect.size.w).round() as i32,
            tile.window_height_for_tile_height(rect.size.h).round() as i32,
        ));
        self.set_floating_geometry(id, rect.loc, size, true);
        true
    }

    /// Starts resizing a snapped window by `edges`. Snapped neighbors whose sides touch these edges
    /// will move those sides along.
    pub fn snap_resize_begin(&mut self, id: &W::Id, edges: ResizeEdge) {
        let Some(r1) = self
            .floating
            .tiles()
            .find(|t| t.window().id() == id)
            .and_then(|t| t.snap)
            .map(|s| s.rect)
        else {
            return;
        };
        let close = |a: f64, b: f64| (a - b).abs() < 1.;
        let (l1, t1) = (r1.loc.x, r1.loc.y);
        let (r1x, b1) = (r1.loc.x + r1.size.w, r1.loc.y + r1.size.h);
        for tile in self.floating.tiles_mut() {
            let Some(snap) = tile.snap else {
                continue;
            };
            if tile.window().id() == id {
                tile.snap_follow = Some((edges, r1));
                continue;
            }
            let r2 = snap.rect;
            let (l2, t2) = (r2.loc.x, r2.loc.y);
            let (r2x, b2) = (r2.loc.x + r2.size.w, r2.loc.y + r2.size.h);
            let mut sides = ResizeEdge::empty();
            if edges.contains(ResizeEdge::RIGHT) && close(l2, r1x) {
                sides |= ResizeEdge::LEFT;
            }
            if edges.contains(ResizeEdge::LEFT) && close(r2x, l1) {
                sides |= ResizeEdge::RIGHT;
            }
            if edges.contains(ResizeEdge::BOTTOM) && close(t2, b1) {
                sides |= ResizeEdge::TOP;
            }
            if edges.contains(ResizeEdge::TOP) && close(b2, t1) {
                sides |= ResizeEdge::BOTTOM;
            }
            if !sides.is_empty() {
                tile.snap_follow = Some((sides, r2));
            }
        }
    }

    /// Applies the resize `delta` (from the resize start) to the snap group. The resized window
    /// itself is resized by the regular interactive resize; here only its rectangle is tracked.
    pub fn snap_resize_update(&mut self, id: &W::Id, delta: Point<f64, Logical>) {
        let mut moved = Vec::new();
        for tile in self.floating.tiles_mut() {
            let Some((sides, orig)) = tile.snap_follow else {
                continue;
            };
            let (mut l, mut t) = (orig.loc.x, orig.loc.y);
            let (mut r, mut b) = (orig.loc.x + orig.size.w, orig.loc.y + orig.size.h);
            if sides.contains(ResizeEdge::LEFT) {
                l = (l + delta.x).min(r - SNAP_RESIZE_MIN);
            }
            if sides.contains(ResizeEdge::RIGHT) {
                r = (r + delta.x).max(l + SNAP_RESIZE_MIN);
            }
            if sides.contains(ResizeEdge::TOP) {
                t = (t + delta.y).min(b - SNAP_RESIZE_MIN);
            }
            if sides.contains(ResizeEdge::BOTTOM) {
                b = (b + delta.y).max(t + SNAP_RESIZE_MIN);
            }
            let rect = Rectangle::new(Point::from((l, t)), Size::from((r - l, b - t)));
            if let Some(snap) = tile.snap.as_mut() {
                snap.rect = rect;
            }
            if tile.window().id() != id {
                let size: Size<i32, Logical> = Size::from((
                    tile.window_width_for_tile_width(rect.size.w).round() as i32,
                    tile.window_height_for_tile_height(rect.size.h).round() as i32,
                ));
                moved.push((tile.window().id().clone(), rect.loc, size));
            }
        }
        // Not through set_floating_geometry(): moving a window there ends its interactive resize.
        for (other, pos, size) in moved {
            self.floating
                .set_window_width(Some(&other), SizeChange::SetFixed(size.w), false);
            self.floating
                .set_window_height(Some(&other), SizeChange::SetFixed(size.h), false);
            self.floating.set_logical_pos_keeping_resize(&other, pos);
        }
    }

    pub fn snap_resize_end(&mut self) {
        for tile in self.floating.tiles_mut() {
            tile.snap_follow = None;
        }
    }
}
