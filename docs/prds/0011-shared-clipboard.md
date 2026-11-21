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
2. **Everything goes at once** when the keyboard crosses: text, images
   and files up to 60 MB. The plan was "small at once, big when pasted"
   (delayed rendering on Windows, a data source on Wayland and X11), but
   `arboard` cannot do delayed rendering; see *Discoveries*. Big content
   on paste is not built and not decided: it needs a spike first (#425).
3. **On a second connection**, not the input link: a 5 MB image on the
   same TCP stream would hold every key press behind it. The second link
   uses the same Noise handshake and the same trusted keys.

## Approach

1. **A data link**: a second secure connection per peer for bulk data,
   with frames up to 64 KB (the input link's frames stay tiny).
2. **Reading and writing the system clipboard** on Wayland (the
   data-control protocol: Hyprland, Sway, KDE), X11 and Windows. GNOME on
   Wayland has no data-control; it is noted, not solved, in this PRD.
3. **Sync on crossing**: text and images, sent whole (not on paste, see point 2 above).
4. **History**: Mickey watches the clipboard on every PC, keeps the
   recent entries in memory (not on disk unless the owner turns it on),
   and a `mickey-ui` window lists them with search; picking one copies it.
5. **Files**: sent whole on crossing, up to 60 MB. Big files and folders,
   streamed and verified, are [0015](0015-file-transfer.md) (#431), which
   replaces this path.
6. **Other managers**: when the owner prefers cliphist or another
   manager, Mickey only syncs and leaves the history to it (what Mickey
   writes into the clipboard already reaches those managers).

## Requirements

- [x] Data link with large frames, separate from the input link.
- [x] Clipboard read/write on Wayland (data-control), X11 and Windows.
- [x] Text and images synced when the keyboard crosses, sent whole.
- [ ] Big content only when pasted (needs a spike first, #425; see *Discoveries*).
- [x] History window (this PC's copies, including what arrived from others).
- [x] Files (up to 60 MB per crossing, no folders yet; any size and folders in [0015](0015-file-transfer.md)).
- [x] Option to leave the history to an existing manager (`[clipboard] history = false`, #134).
- [x] Option to turn sharing off (`[clipboard] share = "off"`, #437).
- [ ] Option to share only on a hotkey (#437, a later PR).

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
- **GNOME on Wayland** offers no clipboard protocol to Wayland programs,
  but **XWayland does the job, both ways** (measured 2026-11-20, GNOME 46
  on Ubuntu 24.04, in the `tools/distro-test` VM). GNOME hands the
  clipboard to X11 programs, focused or not, as CopyQ relies on, and
  Mickey's service gets `DISPLAY` and `XAUTHORITY` from GNOME's session.
  With `arboard`, what Mickey already uses:
  - text written there pasted in GNOME Text Editor, and a file list
    pasted in Nautilus;
  - a file copied in Nautilus, and new text copied in GNOME Text Editor
    (both Wayland apps), were read in the background, with GPaste
    stopped.
  The earlier note that Mickey "sees only what X11 apps copy" (#258) was
  never measured, and is wrong on GNOME 46.
  **The Clipboard portal works too, but is not needed** (spike #497 in
  Python, 2026-11-19; a Rust spike and a working build for #522,
  2026-11-20, `ashpd` 0.13), and it costs every GNOME owner a dialog:
  - the owner approves it in the RemoteDesktop dialog, once, with a
    restore token;
  - writing works (Nautilus needs `x-special/gnome-copied-files` as well
    as `text/uri-list`), and so does reading (`SelectionRead`, text and
    `text/uri-list`);
  - a session for the clipboard alone cannot start: GNOME keeps Share
    disabled until *Allow Remote Interaction* is on;
  - a restore token kept from a session without the clipboard is
    restored silently, still without it: adding the clipboard later means
    asking every existing owner once more;
  - `ashpd` 0.13 reads `SelectionOwnerChanged` wrong (no types, owner
    unknown); asking for each type in turn works.
  It stays the way in should GNOME ever run without XWayland.
- **The history is this PC's**: it includes what arrived from other PCs,
  but not copies made on another PC that never crossed. Showing the other
  PCs' histories is left for later.
- **Leaving the history to another manager** (2026-10-24, #134):
  `[clipboard] history = false`, or the checkbox in the settings window's
  General tab, starts no history recorder; the clipboard is still shared,
  and what Mickey writes reaches cliphist, CopyQ or Klipper, which watch
  the clipboard themselves. `mickey history` then says the history is off.
- **Turning sharing off** (2026-11-13, #437): `[clipboard] share = "off"`
  is each PC's own setting, never sent to the group. The PC whose keyboard
  moves decides every crossing (it reads one side and writes the other),
  so its setting alone is checked there; a PC set to off can still receive
  from another PC's keyboard. Turning it off deletes the received-files
  folder and the whole in-memory history, which cannot tell received
  copies from local ones. It applies when the settings window saves.
- **Only with a hotkey** (2026-11-20, #437): `[clipboard] share =
  "hotkey"` leaves the clipboard where it is when the keyboard moves.
  **AltGr + Right Shift + C** (`[hotkeys] clipboard`) brings the clipboard
  of the PC the keyboard just left, however it moved, by an edge or a
  hotkey. It works in "auto" too, and never when sharing is off.
  - Copied on the PC with the keyboard, it comes at any time.
  - Copied on another PC, that PC still answers only within 5 seconds of
    the keyboard leaving it (#185). Allowing longer would let a paired PC
    read another's clipboard whenever it likes, and the PC asked cannot
    tell whether the owner pressed anything.
  - The hotkey is this PC's own, like `share`: it is never sent with the
    shared hotkeys, which an older Mickey would refuse. Its default is
    skipped where a config.toml already gives those keys to something
    else.
