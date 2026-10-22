# 0009 — Tray icon

> Status: **draft** · Depends on: [0007](0007-ways-to-switch.md), [0008](0008-windows-drives-linux.md) · Vision: [0000](0000-vision.md)

## Goals

The owner wants to see, on every PC, whether the keyboard and mouse they
are using control that PC right now, without a Waybar module that only
some Linux setups have. Decided with the owner on 2026-10-22:

- **An icon in the system tray** of every PC: the StatusNotifierItem tray
  on Linux (Waybar's `tray`, KDE, GNOME with the AppIndicator extension,
  any distro) and the notification area on Windows. The Waybar text module
  from 0007 stays for whoever prefers it.
- **The icon** is the owner's line-art rat (`assets/icons/option-01*.png`),
  white or black to suit the bar, with **a dot in its corner**:
  - **green**: the keyboard and mouse control this PC;
  - **red**: they control another PC;
  - **grey**: Mickey is connected to no other PC.
- **Its tooltip** names the PC being controlled.
- **Clicking it** opens a menu: the PCs (choosing one sends the focus
  there), *Settings*, and *Quit Mickey*.

Done means: with the Windows PC driving the laptop, the Windows icon turns
red and the laptop's green the moment the focus crosses, and back.

## Approach

1. **Where the focus is, on every PC.** A source already knows when its
   focus leaves or comes home. It now also tells the peer: a `Focus`
   message, "my keyboard is on you now" or "not any more". Each PC keeps
   one answer, *is this PC controlled now?*, and the last event wins:
   - its own source goes remote → no; comes home → yes;
   - a peer says "on you" → yes; "not any more" → no.

   The control channel's status carries it, so `mickey status` and the
   Waybar module show it too.
2. **The pictures.** The rat is turned into small raw RGBA images at build
   time (no image crate); the dot is drawn on them in code, so one picture
   serves the three colours.
3. **Linux tray**: StatusNotifierItem over D-Bus, through the `ksni` crate
   (pure Rust; the protocol is too large to write by hand in one PR).
4. **Windows tray**: `Shell_NotifyIconW` and a popup menu, by hand in
   `platform/`, like the hooks: a few Win32 calls, no crate.

Mickey itself shows the icon (not a separate process): it knows the focus
first, and the vision's budget is per peer.

## Requirements

- [ ] `Focus` message and "controlled here" in the status.
- [ ] Icon pictures with a green, red or grey dot.
- [ ] Linux tray with tooltip and menu.
- [ ] Windows tray with tooltip and menu.
- [ ] Memory stays under the vision's 20 MB per peer, measured.

## Non-Goals

- Animations, notifications, or a different icon per PC.
- A tray when Mickey is not running (the icon *is* Mickey running).

## Discoveries

*(Filled in during execution.)*
