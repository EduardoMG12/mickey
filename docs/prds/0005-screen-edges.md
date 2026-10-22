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
2. **Exchange.** After `Hello`, each side sends a `Monitors` message; the
   source keeps every peer's, and `mickey status` and the settings window
   show them.
3. **Layout.** `config.toml` says where each peer sits (`side = "right"`
   of this PC); the settings window draws the PCs and their monitors and
   lets the owner drag them.
4. **Edges.** The source polls the cursor while it moves; reaching the edge
   that has a peer switches focus to it, and the target places its cursor
   at the matching point (absolute positioning: `SetCursorPos` on Windows,
   an `EV_ABS` axis on the uinput device on Linux). On the remote side,
   motion is tracked from the deltas; reaching the edge facing home comes
   back.

## Requirements

- [ ] `mickey monitors` lists this PC's monitors on Windows and Hyprland.
- [ ] `Monitors` message (up to 8 monitors), protocol version 3.
- [ ] `[[peer]] side = "left" | "right" | "above" | "below"`.
- [ ] Edge crossing both ways, same relative height; no crossing while a
      mouse button is held (dragging a window must not throw it to another
      PC).
- [ ] Settings window: PCs online and their monitors, drawn; drag to set
      the side.

## Non-Goals

- Edges on compositors other than Hyprland, and on X11, until later.
- Different scaling per monitor is handled by using logical sizes only.

## Discoveries

*(Filled in during execution.)*
