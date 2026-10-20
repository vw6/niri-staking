### Overview

<sup>niri-staking</sup>

Stacking mode turns niri into a traditional stacking window manager.
All windows become floating, get a titlebar, and can be moved, resized, snapped to output edges, maximized and minimized with the mouse.
Workspaces keep working as usual.

The mode is global: it applies to all workspaces at once.
Toggle it with the `toggle-stacking` action:

```kdl
binds {
    Mod+Shift+S { toggle-stacking; }
}
```

Or from the command line with `niri msg action toggle-stacking`.

To start niri in stacking mode, set the `stacking` flag in the [layout section](./Configuration:-Layout.md#stacking).

When stacking mode is turned on, tiled windows become floating and stay where they were on screen.
When it is turned off, they go back into their columns.
Windows opened while in stacking mode go to the end of the scrolling layout, and windows that were floating before stay floating.

### Titlebars

Windows that use server-side decorations get a titlebar with the application icon, the window title, and the minimize, maximize and close buttons.
Drag the titlebar to move the window, and double-click it to maximize or restore the window.

Windows that draw their own titlebar (client-side decorations) keep it.
Their own maximize and minimize buttons work the same way as the titlebar buttons.

Drag the window edges to resize the window.
This also works for client-side decorated windows that don't draw resize borders, for example GTK 4 applications with `prefer-no-csd`.

Windows in stacking mode have square corners regardless of the `geometry-corner-radius` window rule.

### Snapping

Drag a window to the left or right edge of the output to snap it to that half of the working area, or to a corner to snap it to a quarter.
Drag it to the top edge to maximize it.
A hint shows where the window will go when you release it.

Dropping a window into a corner of a half that already holds a snapped window splits that half: the other window moves into the remaining quarter.
Similarly, dropping a window over the top or bottom quarter of a snapped window splits that window in two.

Resizing a snapped window also resizes the snapped windows next to it.

Dragging a maximized or snapped window restores its previous size.

### Minimizing

Minimized windows are hidden until activated, for example by clicking them on a taskbar that uses the wlr-foreign-toplevel-management protocol.
When the taskbar reports the button rectangle of a window, the window animates into that button.

### Window placement

New windows cascade from the topmost window.
Dialogs open centered over their parent.

When a window closes, niri remembers its position, size and maximized state for its application, and the next window of that application opens the same way.
These are stored in `$XDG_STATE_HOME/niri-staking/placement.json`.
