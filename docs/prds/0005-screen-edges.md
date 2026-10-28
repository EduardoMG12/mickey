# 0005 — Screens have edges

> Status: **draft** · Depends on: [0003](0003-several-pcs-and-switcher.md), [0008](0008-windows-drives-linux.md) · Vision: [0000](0000-vision.md)

## Goals

Step 4 of the plan (2026-10-21): move the mouse off the edge of one screen
and it appears on the PC placed there, at the same height, and back. The
hotkeys stay as the way home. The owner also wants the settings window to
show the PCs that are online, their monitors and how they are arranged,
and to arrange them by dragging.

Done means: the Windows PC placed to the right of the laptop; the cursor
leaves the laptop's right edge and appears on the Windows PC's left edge at
the same relative height; leaving the Windows PC's left edge brings it back.

## The Wayland barrier, and how we pass it

A program may not read the cursor position on Wayland. Mickey asks whoever
may:

| System | How Mickey knows the cursor reached an edge |
|---|---|
| Windows | `GetCursorPos` |
| Hyprland | its IPC socket, `cursorpos` (measured: 20 µs p50, 56 µs p99) |
| X11 | `XQueryPointer` (later) |
| Other Wayland (Sway, KDE, GNOME) | 1-pixel layer-shell surfaces on the edges, or the InputCapture portal (later) |

The position is read only while the mouse moves.

## Approach

1. **Monitors.** Each PC finds its monitors (position and logical size):
   `EnumDisplayMonitors` on Windows, Hyprland's `monitors` on Linux.
   `mickey monitors` prints them.
2. **Exchange.** After `Hello`, each side sends a `Monitors` message
   *(only the target does; see Discoveries, 2026-10-22)*; the
   source keeps every peer's, and `mickey status` and the settings window
   show them.
3. **Layout.** `config.toml` says where each peer's desktop sits next to
   this one (`position = [x, y]`, see *Discoveries*); the settings window
   draws the PCs and their monitors and lets the owner drag them.
4. **Edges.** The source polls the cursor while it moves; reaching the edge
   that has a peer switches focus to it, and the target places its cursor
   at the matching point (absolute positioning: `SetCursorPos` on Windows,
   an `EV_ABS` axis on the uinput device on Linux). On the remote side,
   motion is tracked from the deltas; reaching the edge facing home comes
   back.

## Requirements

- [x] `mickey monitors` lists this PC's monitors on Windows and Hyprland.
- [x] `Monitors` message (up to 8 monitors), protocol version 3.
- [x] `[[peer]] position = [x, y]`.
- [x] Edge crossing both ways, where the desktops touch; no crossing while a
      mouse button is held (dragging a window must not throw it to another
      PC).
- [x] Settings window: PCs online and their monitors, drawn; drag to
      place, snapping against the nearest monitor.

## Non-Goals

- Edges on compositors other than Hyprland, and on X11, until later.
- Different scaling per monitor is handled by using logical sizes only.

## Discoveries

- **A free position, not a side.** The first version put each peer on
  one of four sides of the whole desktop, centred (#96, closed). Trying
  it, the owner wanted the laptop next to one particular monitor, in its
  corner, as Windows' display settings allow. So a peer has a position:
  where its point 0,0 sits on this desktop. Dropped in the settings
  window, it snaps against the nearest monitor, touching without
  overlapping, and the cursor will cross wherever the two touch.
- **The settings window asks the running Mickey.** It is a separate
  process, so it runs `mickey peers` over the control channel (a Unix
  socket on Linux; on Windows TCP on localhost with a token in the user's
  own folder). It asks again every 2 s while Mickey is still starting.
- **Crossing, both ways.** The source crosses when the cursor pushes
  against the edge of its desktop onto a pixel that, shifted by a peer's
  position, is on one of the peer's monitors; it sends `Enter` with that
  point. The way back is noticed by the target, which knows its own
  cursor: when it pushes off the target's desktop, the target sends
  `Leave` with the pixel just off it, and the source comes home if that
  pixel lands on one of its monitors. A Leave is sent once per push.
- **Placing the cursor on Linux** uses Hyprland's `movecursor` instead of
  an `EV_ABS` axis on the uinput device: an absolute axis would make the
  virtual device look like a tablet, and Hyprland already uses the
  coordinates `monitors` reports.
- **Windows reads raw mouse motion while local too**, to see the cursor
  pushing against an edge: the mouse hook is installed only while the
  focus is remote (it slowed games down otherwise), and raw input costs
  nothing in the game's path.
- **Games and raw input (Windows).** Found while testing with
  Counter-Strike open: games read input with raw input, which hooks cannot
  block, and pin the cursor while aiming. While the focus is remote, an
  invisible Mickey window takes the foreground (games only get raw input
  in the foreground) and motion is read from raw input.

- **Monitors move while Mickey runs** (#365). Each PC read its monitors
  once, at start. After a monitor was moved in Windows' display settings,
  the source kept checking the target's `Leave` against the old layout,
  found no monitor there and ignored it silently: the way back through
  that monitor was gone until Mickey restarted. The source now reads them
  again once a second, off its main loop; the target resending its own
  `Monitors` is the second half of #365.

### Checked against the code (2026-10-22)

- **Only the target sends `Monitors`**, right after `Hello`
  (`transport::serve`). The source never sends its own: it reads them
  locally (`monitors::this_pc`) and is the one that needs the peer's to
  cross, while the way back is decided by the target with its own
  monitors. So "each side sends" above never happened; nothing needs it.
- **"No crossing while a mouse button is held" was enforced only on the
  way out.** The source refused to cross with a button down, but a
  target still sent `Leave` mid-drag, so dragging a window or file
  against a peer's edge brought the focus home with the button still held
  there. Fixed in PR #208: the target sends `Leave` only when no button is
  held.
