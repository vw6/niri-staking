//! Stacking mode: free floating windows managed like on a traditional desktop.
//!
//! The mode is global for the whole layout. When it is on, all windows are floating, have
//! titlebars, and can be snapped to output edges, maximized and minimized. Workspace switching
//! keeps working as usual.

use smithay::utils::{Logical, Point, Rectangle};

use super::titlebar::TitlebarPart;
use super::{InteractiveMoveState, Layout, LayoutElement, Options};
use crate::utils::ResizeEdge;

impl<W: LayoutElement> Layout<W> {
    pub fn is_stacking(&self) -> bool {
        self.stacking
    }

    pub fn toggle_stacking(&mut self) {
        self.set_stacking(!self.stacking);
    }

    pub fn set_stacking(&mut self, on: bool) {
        if self.stacking == on {
            return;
        }
        // The dragged window is not on any workspace; wait for the move to end.
        if matches!(self.interactive_move, Some(InteractiveMoveState::Moving(_))) {
            return;
        }
        self.stacking = on;

        let mut options = Options::clone(&self.options);
        options.layout.stacking = on;
        self.update_options(options);

        for ws in self.workspaces_mut() {
            if on {
                ws.enter_stacking();
            } else {
                ws.leave_stacking();
            }
        }
    }

    /// Sets the titlebar part under the pointer. Returns whether anything changed.
    pub fn set_titlebar_hover(&mut self, hover: Option<(&W::Id, TitlebarPart)>) -> bool {
        let mut changed = false;
        for ws in self.workspaces_mut() {
            for tile in ws.tiles_mut() {
                let want = hover
                    .filter(|(id, _)| *id == tile.window().id())
                    .map(|(_, part)| part);
                if tile.titlebar.hover != want {
                    tile.titlebar.hover = want;
                    changed = true;
                }
            }
        }
        changed
    }

    pub fn stacking_minimize(&mut self, id: &W::Id) -> bool {
        if !self.stacking {
            return false;
        }
        self.workspaces_mut()
            .find(|ws| ws.has_window(id))
            .is_some_and(|ws| ws.minimize_window(id))
    }

    pub fn stacking_toggle_maximize(&mut self, id: &W::Id) -> bool {
        if !self.stacking {
            return false;
        }
        self.workspaces_mut()
            .find(|ws| ws.has_window(id))
            .is_some_and(|ws| ws.toggle_stacking_maximize(id))
    }

    pub fn is_stacking_maximized(&self, id: &W::Id) -> bool {
        self.workspaces()
            .flat_map(|(_, _, ws)| ws.tiles())
            .any(|tile| tile.window().id() == id && tile.is_stacking_maximized())
    }

    /// Restores a maximized or snapped window that started being dragged. `anchor` is the pointer
    /// position within the output.
    pub fn stacking_unmaximize_for_drag(
        &mut self,
        id: &W::Id,
        anchor: Point<f64, Logical>,
    ) -> bool {
        self.workspaces_mut()
            .find(|ws| ws.has_window(id))
            .is_some_and(|ws| ws.unmaximize_for_drag(id, anchor))
    }

    /// Sets the taskbar button rectangle of a window, in output coordinates.
    pub fn set_taskbar_rect(&mut self, id: &W::Id, rect: Rectangle<f64, Logical>) {
        if let Some(ws) = self.workspaces_mut().find(|ws| ws.has_window(id)) {
            ws.set_taskbar_rect(id, rect);
        }
    }

    /// Ends an interactive move, snapping the window to where the snap hint showed.
    pub fn stacking_move_end(&mut self, id: &W::Id) {
        let mut target = None;
        for ws in self.workspaces_mut() {
            if let Some(t) = ws.snap_pending.take() {
                target = Some(t);
            }
        }
        let moving = matches!(
            &self.interactive_move,
            Some(InteractiveMoveState::Moving(move_)) if move_.tile.window().id() == id
        );
        self.interactive_move_end(id);
        if let (true, Some(target)) = (moving && self.stacking, target) {
            if let Some(ws) = self.workspaces_mut().find(|ws| ws.has_window(id)) {
                ws.apply_snap_target(id, target);
            }
        }
    }

    pub fn stacking_resize_begin(&mut self, id: &W::Id, edges: ResizeEdge) {
        if let Some(ws) = self.workspaces_mut().find(|ws| ws.has_window(id)) {
            ws.snap_resize_begin(id, edges);
        }
    }

    pub fn stacking_resize_update(&mut self, id: &W::Id, delta: Point<f64, Logical>) {
        if let Some(ws) = self.workspaces_mut().find(|ws| ws.has_window(id)) {
            ws.snap_resize_update(id, delta);
        }
    }

    pub fn stacking_resize_end(&mut self, id: &W::Id) {
        if let Some(ws) = self.workspaces_mut().find(|ws| ws.has_window(id)) {
            ws.snap_resize_end();
        }
    }
}
