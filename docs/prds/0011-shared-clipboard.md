# 0011 — Shared clipboard and history

> Status: **draft** · Depends on: [0002](0002-secure-link.md), [0010](0010-easy-pairing.md) · Vision: [0000](0000-vision.md)

## Goals

Copy on one PC, paste on another: text, images and files. And a clipboard
history of Mickey's own, like Windows' Win+V, holding what was copied on
every PC. Decided with the owner on 2026-10-22:

- **Mickey's own clipboard first**, working on any Linux distro (Wayland
  and X11) and on Windows; working *with* a clipboard manager the owner
  already has (cliphist, CopyQ, Klipper) comes later.
- **Text, images and files**, in that order.
- **Everything travels**, passwords included, always encrypted and only
  between paired PCs.
- **When it travels** is chosen for performance (below).

Done means: copy text or a screenshot on the Windows PC, cross to the
laptop, paste; and open Mickey's history on either PC to see both PCs'
recent copies.

## When the clipboard travels

1. **When the keyboard crosses to another PC**, Mickey reads what is
   copied there and offers it to the PC the keyboard goes to (as Synergy
   does). Nothing is sent on every copy: what is copied and never needed
   elsewhere never leaves the PC.
2. **Small things go at once** with the offer (text, small images).
   **Big things go when pasted**: the offer says "an image, 4 MB", and the
   bytes cross only if an application on the other PC asks for them
   (delayed rendering on Windows, a data source on Wayland and X11).
3. **On a second connection**, not the input link: a 5 MB image on the
   same TCP stream would hold every key press behind it. The second link
   uses the same Noise handshake and the same trusted keys.

## Approach

1. **A data link**: a second secure connection per peer for bulk data,
   with frames up to 64 KB (the input link's frames stay tiny).
2. **Reading and writing the system clipboard** on Wayland (the
   data-control protocol: Hyprland, Sway, KDE), X11 and Windows. GNOME on
   Wayland has no data-control; it is noted, not solved, in this PRD.
3. **Sync on crossing**: text and images, small at once, big on paste.
4. **History**: Mickey watches the clipboard on every PC, keeps the
   recent entries in memory (not on disk unless the owner turns it on),
   and a `mickey-ui` window lists them with search; picking one copies it.
5. **Files**: offered by name and size, sent when pasted.
6. **Other managers**: when the owner prefers cliphist or another
   manager, Mickey only syncs and leaves the history to it (what Mickey
   writes into the clipboard already reaches those managers).

## Requirements

- [x] Data link with large frames, separate from the input link.
- [x] Clipboard read/write on Wayland (data-control), X11 and Windows.
- [x] Text and images synced when the keyboard crosses (big ones on paste: not yet, see *Discoveries*).
- [x] History window (this PC's copies, including what arrived from others).
- [x] Files (up to 60 MB per crossing, no folders yet).
- [ ] Option to leave the history to an existing manager.

## Non-Goals

- Syncing through the cloud, or with PCs not paired.
- GNOME on Wayland until it offers a clipboard protocol to other programs.

## Discoveries

- **`arboard` was already in the tree** (the settings window's eframe uses
  it): Wayland through data-control, X11, Windows, text, images and file
  lists, with no new crate.
- **Everything crosses at once, for now.** Delayed rendering (bytes only
  when an app pastes) needs owning the clipboard protocol per system,
  which `arboard` does not offer. With the data link on its own
  connection and its own thread, an eager transfer never delays input:
  a worst-case 1920x1080 noise image took 369 ms to read, compress (2.2
  MB) and send; 293 KB of files took 27 ms.
- **Images cross as PNG** (fast compression); a flat screenshot shrinks
  more than 50 times.
- **Files** land in a fresh folder under Mickey's cache
  (`~/.cache/mickey/received`, `%LOCALAPPDATA%\mickey\received`), the
  last five transfers kept; names from the network that could leave the
  folder are refused. At most 60 MB per crossing; more is skipped and
  logged.
- **Noticing copies** without polling: Wayland's selection events,
  Windows' clipboard counter; X11 and GNOME compare the text once a
  second (their image copies reach the history only when they cross).
- **GNOME on Wayland** offers no clipboard protocol to other programs:
  there Mickey sees only what X11 apps copy.
- **The history is this PC's**: it includes what arrived from other PCs,
  but not copies made on another PC that never crossed. Showing the other
  PCs' histories is left for later.
