# 0006 — egui (eframe) for every window, in a separate `mickey-ui` program

- **Status:** Accepted
- **Date:** 2026-10-21 (spike #47, decision recorded in PR #54; first
  window in PR #55)

## Context

With three or more PCs the hotkey opens a switcher, a "Win+Tab for
computers" ([PRD 0003](../prds/0003-several-pcs-and-switcher.md#goals)).
Later came the pairing question, the settings window and the clipboard
history ([PRD 0006](../prds/0006-setup-window.md), [PRD 0010](../prds/0010-easy-pairing.md),
[PRD 0011](../prds/0011-shared-clipboard.md)). They must run on Linux
(Wayland and X11) and on Windows, cross-compiled from Arch.

The program that holds the keyboard runs all day and has a budget of
**< 20 MB** resident (vision). The spike (#47) measured eframe/egui with
only `glow`, `default_fonts`, `wayland` and `x11`:

| | Mickey then | Spike |
|---|---|---|
| Crates in the tree | 37 | 124 |
| Binary, stripped | 1.2 MB | 8.5 MB Linux, 13 MB Windows |
| Resident memory | ~1 MB | 90 MB Linux, 125 MB Windows |

## Decision

- **egui through eframe** for every window, "decided with the owner"
  (PRD 0003, *Requirements*).
- **A separate program, `mickey-ui`** (its own crate), started only while a
  window is on screen. `mickey` never links the GUI crates.
- **They talk through processes, not shared memory**
  ([architecture.md](../architecture.md#three-crates)): `mickey` starts
  `mickey-ui switcher`, `pair`, `join` as children and reads the answer from
  stdout and the exit code (exit 2 means "no display", so the question
  falls back to the terminal). The settings and history windows run
  `mickey <command>` (`peers`, `connect`, `arranged`, …), which reach the
  running Mickey over the control socket.
- **Both read `config.toml` through `mickey-config`**, so the settings
  window checks a form with `mickey_config::parse`, the very rules
  `mickey` applies, before writing it.

## Alternatives considered

- **The GUI inside `mickey`, on a thread.** Rejected after the spike: 90 to
  125 MB "far over the vision's 20 MB budget", and a GUI that hangs or
  crashes could stall the process that holds the keyboard (PRD 0003,
  *Constraints* and *Ticket 3*). The spike PR's review challenge also asks
  for a security reason; the likely answer (**inferred**, not written
  down): 87 more crates kept out of the process that reads every key press
  is less code that could leak or mishandle it.
- **Other toolkits** (GTK, Qt, iced, Tauri or a web view). **Not recorded.**
  The docs show only that egui was spiked and accepted; no comparison was
  written down. What the spike checked is what egui had to prove: it opens
  focused on Hyprland, opens on top on Windows when cross-compiled, and
  repaints from another thread.
- **Live screen thumbnails in the switcher cards.** Rejected with the
  owner: each PC would stream its screen, heavy and puts screen contents on
  the network (PRD 0003, *Non-Goals*).

## Consequences

Good:

- `mickey` stays small and cannot be stuck by a window; nothing is grabbed
  while the switcher is open, so a crash there never locks the owner out.
- One GUI code base for Linux and Windows, in the same language
  ([ADR 0001](0001-rust-for-daemon-and-windows.md)).
- The settings preview of the switcher is drawn by the same code as the
  real switcher.

Bad, and known limits:

- **Each window costs a process start**, and 90 to 125 MB while open.
- **A command-line protocol to maintain** between the two programs, e.g.
  `mickey-ui switcher [--mirror] [--at <corner>] [--vertical] <focused-id> <id>=online:<name>...`.
- **Placement is the desktop's call on Wayland.** Hyprland needs a window
  rule (`place.rs`), Plasma a KWin script over D-Bus (#218), and GNOME
  cannot place it at all (README, *Known limits*).
- **The built-in fonts have no arrows or bullets**; labels are plain ASCII.
- `mickey-ui` must sit next to `mickey`, and needs glibc 2.35 for `hypot`
  ([ADR 0010](0010-releases-and-license.md)).

## Links

- [PRD 0003](../prds/0003-several-pcs-and-switcher.md), *Constraints* and
  *Discoveries* (*Ticket 3*, *Ticket 4*).
- [PRD 0006, setup window](../prds/0006-setup-window.md), *Approach*.
- [architecture.md](../architecture.md#three-crates).
- Issues #47, #218; PRs #54, #55, #56.
- Code: `crates/mickey-ui/` (`main.rs`, `settings.rs`, `pair.rs`,
  `place.rs`), `crates/mickey/src/window.rs`, `crates/mickey/src/control.rs`,
  `crates/mickey-config/`.
