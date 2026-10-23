# 0006 — Setup window

> Status: **built** (written after the fact, 2026-10-22) · Depends on: [0003](0003-several-pcs-and-switcher.md), [0007](0007-ways-to-switch.md), [0010](0010-easy-pairing.md) · Vision: [0000](0000-vision.md)

This PRD was never written before the work: the window grew PR by PR
(#83 onwards) while the vision listed it as "0006 (todo)". It records
what exists on `main` today, so later changes have something to compare
against.

## Goals

Step 3 of the plan: the owner sets Mickey up without editing
`config.toml` by hand. One window adds and pairs PCs, places each PC and
its monitors, and changes the hotkeys and the switcher's look.

Done means: on a fresh install, the owner opens *Mickey settings* (menu
shortcut, tray menu or `mickey settings`), connects a PC found on the
network, drags it next to this PC's monitors, saves, and the keyboard
crosses with the new layout and hotkeys without a restart.

## Approach

- **A second program, `mickey-ui`**, with egui. `mickey-ui settings` is
  the window; `mickey settings` and the tray's *Settings* start it from
  next to `mickey`. The window never talks to the network itself: it asks
  the running Mickey by running `mickey <command>` (`id`, `peers`,
  `discover --lines`, `connect`, `record`, `autostart`, `arranged`).
- **It edits the same `config.toml`** that `mickey` reads. Everything
  editable is a `Form` of plain fields; *Save* turns it into TOML and
  checks it with `mickey_config::parse`, the very rules `mickey` applies,
  before writing. A form that does not parse is not written. *Undo*
  returns to what was last saved.
- **Live apply through `mickey arranged`.** After writing, the window runs
  `mickey arranged`: the running Mickey reloads the file, uses the new
  positions, hotkeys and `[interface]` at once, and sends the positions
  and hotkeys to the other PCs. Other settings (roles, listen address,
  touchpad speed, name) still need a restart, and the save message says
  so.

### The tabs

1. **This PC**: its name (empty means the host name), its fingerprint
   (from `mickey id`), whether it controls other PCs and whether it can be
   controlled, *Start Mickey when this PC starts* (applied at once through
   `mickey autostart`, not on Save), touchpad speed, and, folded under
   *Advanced*, the listen address.
2. **PCs**: *Your PCs*, each with an online dot, address, monitor count,
   its own hotkey and *Remove*; *On this network*, the PCs heard by
   discovery that are not yours yet, each with *Connect* (`mickey connect
   <ip:port> <name>`: Mickey adds it to `config.toml` and pairs at once,
   the code showing on both PCs, PRD 0010); and, folded away, *Add by
   address* for a PC discovery cannot hear.
3. **Layout**: this PC's monitors and every peer's, drawn where they sit.
   A peer is dragged anywhere and, when dropped, **snaps** against the
   nearest monitor of this PC, touching it without overlapping, like
   Windows' display settings (`snap.rs`); within 120 desktop pixels it
   lines up with that monitor's corner. Peers with no position wait in a
   strip at the top.
4. **Hotkeys**: six rows (next PC / switcher, back to this PC, open the
   switcher, next and previous PC with one hand, double tap). Each can be
   **recorded** (`mickey record` names the keys the running Mickey hears),
   **typed** by name, or reset to its **default**; an empty row shows its
   default, faded. Below them, a slider for how close the two taps of a
   double tap must be (150 to 1000 ms).
5. **General**: the switcher's position (center or a corner), a row or a
   column, and its order (as the layout places the PCs, or moved by hand),
   with a **preview** drawn by the same code as the real switcher.

## Requirements

- [x] `mickey-ui settings`, started by `mickey settings`, the tray and a
      menu shortcut on both installers.
- [x] Save writes `config.toml` only if `mickey_config::parse` accepts it.
- [x] This PC, PCs, Layout, Hotkeys and General tabs as above.
- [x] Connect to a PC found on the network, with no restart.
- [x] Drag to place, with snap.
- [x] Layout and hotkeys apply on every PC at once (`mickey arranged`).
- [ ] Every other setting applied without a restart.

## Non-Goals

What the window does not do today:

- Edit the trusted peers: pairing adds them, `mickey forget` removes
  them.
- Move one monitor of a PC apart from its others: a PC is dragged as a
  whole, its monitors as its own OS arranged them.
- Keep its own settings store: the window and `mickey` share one
  `config.toml`.

## Discoveries

### What the window does that the plan did not say (2026-10-22)

- **It follows `config.toml` while open.** Once a second it checks the
  file's modification time; when another PC placed itself and rewrote the
  file, the window reloads, unless the owner has changes not saved yet,
  in which case it says that Save would replace them. Every 5 s it asks
  Mickey again about the PCs and the network.
- **The layout is kept in agreement on both PCs.** Saving a position on
  one PC makes Mickey tell the placed peer where this PC sits on its
  desktop (the mirror position), and the peer writes that one line into
  its own `config.toml` (PRD 0005).
- **Recording a hotkey goes through the running Mickey**, not egui: the
  window runs `mickey record`, and Mickey names the keys its own capture
  hears, with the same names `config.toml` uses. While it records, keys
  are named, never acted on.
- **Start with this PC is applied at once, off the UI thread**, because
  Windows may show an administrator prompt first.
- **On Windows the window hides `mickey.exe`'s console** when it runs a
  command (`CREATE_NO_WINDOW`), or a console would flash over it.
