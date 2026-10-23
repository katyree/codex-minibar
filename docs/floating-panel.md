# Floating usage panel

Enable **Settings > Customize > Floating panel** to detach the popup from the
taskbar. Enabling it also pins the panel open. Drag the dedicated header to move
it, including across monitors. Tabs and action buttons remain interactive.
The header shows the app logo and Codex Minibar title on the left; that area
remains draggable.
The header's Edit Home button opens the Home editor and becomes Done while
editing. The bottom bar hides its Edit/Done button while the floating header is
shown, including when editing was started from Settings.
The pin button is in the header next to the close button in floating mode,
and in the bottom bar in tray mode. It controls outside-click and Escape
dismissal. The close button hides the panel without clearing its pin or position;
the tray icon and popup hotkey also hide or reopen it. Header buttons do not drag
the window.

The two-column button next to the pin changes **Settings > Customize > Use two
columns** immediately and saves the preference. Its accent state reflects that
setting, including changes made in Settings. Home and Usage follow the toggle;
provider tabs keep their single-column layout.

**Always on top** and the popup pin control the same persisted preference.
Enabling either keeps the panel above other applications and prevents dismissal
on focus loss, outside clicks, and Escape. Disabling either restores dismissal
when another application takes the foreground or the user clicks outside.
Settings remains a live preview surface and does not trigger dismissal.
Notifications keep their own topmost behavior. An unpinned popup still opens in
front, but only a pinned panel stays in the topmost band. Changes apply to an
open panel without restarting.

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

## Height resizing

Both the tray popup and floating panel support manual height resizing. Drag the
free vertical edge: the top edge for a bottom-anchored tray popup, or the bottom
edge for a top-anchored popup. Floating panels can be resized from either edge.
The opposite edge stays fixed, including when resizing a floating panel upward.
The height follows the pointer directly and is saved when the drag finishes;
the same height preference applies across views, modes, and application restarts.
The maximum is the smaller of the owning monitor's full screen height and the
current tab's measured content plus window chrome. Switching to shorter content
automatically removes unused space while retaining the preferred height for
longer tabs. Content exceeding the selected height remains scrollable.
Width continues to follow each tab's column layout. Normal content/layout
transitions still respect the animation preferences.

## Verification

Unit tests cover old settings defaults, persistence, negative and off-screen
origins, fixed origins across column counts and DPI values, and height
independence from work-area edges. Native drag, stacking, and physical monitor
transitions require manual verification; automated checks do not launch the app.
