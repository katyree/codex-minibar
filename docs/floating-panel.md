# Floating usage panel

Enable **Settings > Customize > Floating panel** to detach the popup from the
taskbar. Enabling it also pins the panel open. Drag the dedicated header to move
it, including across monitors. Tabs and action buttons remain interactive.
The pin button is in the header next to the close button in floating mode,
and in the bottom bar in tray mode. It controls outside-click and Escape
dismissal. The close button hides the panel without clearing its pin or position;
the tray icon and popup hotkey also hide or reopen it. Header buttons do not drag
the window.

The two-column button next to the pin changes **Settings > Customize > Use two
columns** immediately and saves the preference. Its accent state reflects that
setting, including changes made in Settings. Home and Usage follow the toggle;
provider tabs keep their single-column layout.

**Always on top** independently controls the floating panel's stacking order.
Turning it off also disables automatic raising when another application becomes
active. Notifications keep their own topmost behavior. Tray mode keeps its usual
topmost behavior. All changes apply to an open panel without restarting.

The panel grows downward from its saved top edge. Its maximum height is based
on the monitor size, independently of its position; overflowing content scrolls.
With two columns enabled, Home and Usage use two columns. Provider tabs resize
the visible window to one column. Resizing preserves the top-left origin and
can extend beyond the screen edge; the app never snaps the panel into a work area.
Header and content size changes follow
the application's and Windows' animation preferences.

The saved position restores the exact screen coordinates, including positions
partially outside a monitor. An open panel uses its live native window origin
when resizing or processing display changes. Monitor and work-area metadata in
older settings remains compatible, but does not override the screen coordinates.
**Reset position** clears the saved origin and places the panel at the upper-right
of the current monitor. Disable **Floating panel** to restore tray positioning;
the saved floating position remains available when it is enabled again.

The tray/worker bridge uses its idle polling cadence while a pinned panel is
open and Settings is closed. GPUI drives animations itself and worker updates
continue to publish normally.

## Verification

Unit tests cover old settings defaults, persistence, negative and off-screen
origins, fixed origins across column counts and DPI values, and height
independence from work-area edges. Native drag, stacking, and physical monitor
transitions require manual verification; automated checks do not launch the app.
