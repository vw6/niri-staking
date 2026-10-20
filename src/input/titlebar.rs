//! Pointer handling for titlebars and window edges in stacking mode.

use smithay::desktop::Window;
use smithay::input::pointer::{
    CursorIcon, CursorImageStatus, Focus, GrabStartData as PointerGrabStartData,
};
use smithay::utils::SERIAL_COUNTER;

use super::move_grab::MoveGrab;
use super::resize_grab::ResizeGrab;
use super::{PointerOrTouchStartData, DOUBLE_CLICK_TIME};
use crate::layout::titlebar::TitlebarPart;
use crate::layout::HitType;
use crate::niri::{PointContents, State};
use crate::utils::{get_monotonic_time, ResizeEdge};

/// Returns the titlebar part or window edge under the pointer.
pub fn titlebar_hit(under: &PointContents) -> Option<(&Window, TitlebarPart)> {
    match &under.window {
        Some((
            window,
            HitType::Activate {
                titlebar: Some(part),
                ..
            },
        )) => Some((window, *part)),
        _ => None,
    }
}

impl State {
    /// Updates the titlebar button hover and the resize cursor over window edges.
    pub fn update_titlebar_hover(&mut self, under: &PointContents) {
        let hover = titlebar_hit(under);
        if self.niri.layout.set_titlebar_hover(hover) {
            // FIXME: granular.
            self.niri.queue_redraw_all();
        }

        let pointer = self.niri.seat.get_pointer().unwrap();
        if pointer.is_grabbed() {
            return;
        }
        match hover {
            Some((_, TitlebarPart::Edge(edges))) => {
                self.niri
                    .cursor_manager
                    .set_cursor_image(CursorImageStatus::Named(edges.cursor_icon()));
                self.niri.titlebar_edge_cursor = true;
            }
            _ if self.niri.titlebar_edge_cursor => {
                self.niri.titlebar_edge_cursor = false;
                self.niri
                    .cursor_manager
                    .set_cursor_image(CursorImageStatus::default_named());
            }
            _ => (),
        }
    }

    fn start_move(&mut self, button_code: u32, window: Window) {
        let pointer = self.niri.seat.get_pointer().unwrap();
        let start_data = PointerGrabStartData {
            focus: None,
            button: button_code,
            location: pointer.current_location(),
        };
        let start_data = PointerOrTouchStartData::Pointer(start_data);
        let icon = CursorIcon::Grabbing;
        if let Some(grab) = MoveGrab::new(self, start_data, window, false, Some(icon)) {
            pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
            self.niri
                .cursor_manager
                .set_cursor_image(CursorImageStatus::Named(icon));
        }
    }

    fn start_resize(&mut self, button_code: u32, window: Window, edges: ResizeEdge) {
        let pointer = self.niri.seat.get_pointer().unwrap();
        if !self
            .niri
            .layout
            .interactive_resize_begin(window.clone(), edges)
        {
            return;
        }
        self.niri.layout.stacking_resize_begin(&window, edges);
        let start_data = PointerGrabStartData {
            focus: None,
            button: button_code,
            location: pointer.current_location(),
        };
        let grab = ResizeGrab::new(start_data, window);
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
        self.niri
            .cursor_manager
            .set_cursor_image(CursorImageStatus::Named(edges.cursor_icon()));
    }

    /// Returns whether this click on the window title completes a double click.
    fn is_double_click(&mut self, window: &Window) -> bool {
        let now = get_monotonic_time();
        let double = matches!(
            &self.niri.last_title_click,
            Some((last, time)) if last == window && now.saturating_sub(*time) <= DOUBLE_CLICK_TIME
        );
        self.niri.last_title_click = (!double).then(|| (window.clone(), now));
        double
    }

    /// Handles a left button press over a titlebar or a window edge. Returns whether the press was
    /// handled and should not go to the window.
    pub fn on_titlebar_press(&mut self, button_code: u32, under: &PointContents) -> bool {
        let Some((window, part)) = titlebar_hit(under) else {
            return false;
        };
        let window = window.clone();
        self.niri.layout.activate_window(&window);

        let pointer = self.niri.seat.get_pointer().unwrap();
        let grabbed = pointer.is_grabbed();
        match part {
            TitlebarPart::Title if self.is_double_click(&window) => {
                self.niri.layout.stacking_toggle_maximize(&window);
            }
            // A maximized window is restored by the move grab once it is actually dragged.
            TitlebarPart::Title if !grabbed => self.start_move(button_code, window),
            TitlebarPart::Title => (),
            TitlebarPart::Edge(edges) if !grabbed => self.start_resize(button_code, window, edges),
            TitlebarPart::Edge(_) => (),
            TitlebarPart::Close => {
                if let Some((_, mapped)) =
                    self.niri.layout.windows().find(|(_, m)| m.window == window)
                {
                    mapped.toplevel().send_close();
                }
            }
            TitlebarPart::Maximize => {
                self.niri.layout.stacking_toggle_maximize(&window);
            }
            TitlebarPart::Minimize => {
                self.niri.layout.stacking_minimize(&window);
            }
        }

        // Suppress the release after button clicks. Moves and resizes need the release to end their
        // grabs.
        if part.is_button() {
            self.niri.suppressed_buttons.insert(button_code);
        }
        // FIXME: granular.
        self.niri.queue_redraw_all();
        true
    }
}
