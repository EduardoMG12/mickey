# 0003 — Several PCs and the switcher

> Status: **draft** · Depends on: [0002](0002-secure-link.md) · Vision: [0000](0000-vision.md)

## Goals

Step 1 and 2 of the plan agreed with the owner on 2026-10-21 (the order is
in the vision's roadmap):

1. **Several PCs from a config file.** `mickey source` reads the list of
   peers from `config.toml` instead of one `--to` address, and connects to
   all of them.
2. **The switcher**, a "Win+Tab for computers". With three or more peers,
   the hotkey opens a small window that lists the PCs; click one, or use the
   arrow keys and Enter, and focus goes there. With two peers (this PC and
   one other) the hotkey keeps switching directly, as today.

Done means: with a `config.toml` listing two targets, the owner presses the
hotkey on the laptop, picks a PC in the switcher window and types on it;
the hotkey from any remote PC brings focus home and opens the switcher
again.

## Requirements

### Config file
- [ ] `config.toml` in `$XDG_CONFIG_HOME/mickey/` (`~/.config/mickey/`) or
      `%APPDATA%\mickey\`, as in the vision's *Installed layout*:

      ```toml
      [[peer]]
      name = "Desktop"
      address = "192.0.2.10:4242"

      [[peer]]
      name = "Work laptop"
      address = "192.0.2.11:4242"

      [touchpad]
      speed = 16        # counts per millimetre (the M0 ponytail)
      ```
- [ ] `mickey source` with no `--to` uses the config; `--to` still works and
      overrides it, for one-off use.
- [ ] A missing file explains where to create it, with an example. A bad
      file names the line and what is wrong. Never a panic.
- [ ] Unknown keys are an error, so a typo (`adress`) is not silently
      ignored.

### Several targets
- [ ] The source connects to every peer at start, pairing each one the
      first time (PRD 0002). A peer that is down is retried every few
      seconds and shown as offline; it never blocks the others.
- [ ] Focus is `Local` or `Remote(peer)`. Grabs and the held-key release
      work exactly as in M0, per peer.
- [ ] With exactly one peer, the hotkey toggles as in M0.
- [ ] With two or more peers, the hotkey from `Local` opens the switcher,
      and the hotkey from any remote peer brings focus home and opens it.

### Switcher window
- [ ] A small window in the middle of the source's screen with one **card**
      per PC: *This PC* and every peer, with its name, its OS, whether it is
      online and whether it has the focus. Win+Tab was the owner's visual
      reference, not something to reuse.
- [ ] Arrow keys move the selection, Enter or a click switches, Esc closes
      without switching. It never takes longer than one frame to appear
      after the hotkey is released.
- [ ] Built with **egui/eframe** (decided with the owner), as a **separate
      program, `mickey-ui`** (its own crate in the workspace). `mickey
      source` starts it when the switcher is needed, passes the PCs on the
      command line and reads the choice from its standard output; it exits
      after the choice. The same program will hold the setup window (step 3).
- [ ] While the switcher is open, nothing is grabbed: it is a normal window
      on the local desktop.
- [ ] Offline PCs are shown but cannot be picked.
- [ ] On Hyprland, the README shows the window rules that make it float,
      centered and pinned (`class:^(mickey-ui)$`).

### Tests
- [ ] Config parsing: valid file, missing file, bad TOML, unknown key,
      bad address.
- [ ] Focus with several peers: switch to each, hotkey home, releases sent
      to the right peer, an offline peer cannot be picked.
- [ ] Manual, in the PR: the owner's laptop with two targets. Only one
      Windows PC exists, so the second target is a second `mickey target`
      on another port of the same PC, both paired.

## Non-Goals

- Screen edges and placing PCs and monitors (steps 3 and 4: PRDs 0006 and
  0005).
- Editing the config from a window (step 3). This PRD only reads it.
- Windows as a source (M2): the switcher runs where the keyboard is, which
  is the Linux laptop for now.
- Automatic discovery of peers on the network (mDNS, later).
- **Live screen thumbnails** in the cards (like Win+Tab's previews). Each PC
  would have to capture its screen and stream it, which is heavy and puts
  screen contents on the network. An idea for later, decided with the owner.

## Constraints

- **New dependencies, to justify in their PRs:**
  - `toml` and `serde` (derive) to read the config. TOML is the format the
    vision chose; writing a TOML parser by hand is a project of its own.
  - `eframe`/`egui` for the switcher. It is the largest dependency so far,
    so a spike (ticket 3) measures binary size, build time and whether a
    window shows up on top on Hyprland before anything depends on it.
- **The window and the capture loop are separate processes** (see the
  spike in *Discoveries*): the GUI costs 90–125 MB while open, far over the
  vision's 20 MB budget, so it only exists while it is on screen.
- Hard rule 8 still holds: the window can crash or hang without the
  keyboard staying grabbed, because nothing is grabbed while it is open.

## Concepts in this milestone

- **State machine, extended**: `Remote` gets a payload (`Remote(peer)`),
  and a new transition goes through the switcher. Same #65 *Valid Number*
  idea as M0, one level up.
- **Producer/consumer over channels**: the capture loop and the UI thread
  never share state, only messages. Closest LeetCode: #1188 *Design Bounded
  Blocking Queue*.

## Open Questions

- Should the switcher also list the monitors of each PC, or only PCs? Only
  PCs until monitors exist (step 3).

## Discoveries

### Ticket 1 — config.toml (2026-10-21)

- **`toml` with only `parse` and `serde`**, plus `serde` with `derive`:
  11 more crates (37 in total), all Rust; `cargo audit` is clean.
- **`deny_unknown_fields` gives the best error for free:** a misspelled key
  is reported with its line, column and the keys that were expected
  (`unknown field adress, expected name or address`).
- **The touchpad speed left its `ponytail`:** `[touchpad] speed` (1–200,
  default 16) replaces the constant from M0.
- Until #46, `mickey source` without `--to` uses only the first peer.

### Ticket 2 — every peer, one focus (2026-10-21)

- **Checked on the owner's laptop with two targets** (two ports on the
  Windows PC, throw-away identities, run over SSH so they could not inject):
  Windows B was offline at start and reported as such; once started it was
  picked up within 5 s without asking to pair; three presses of the hotkey
  went Windows A → Windows B → This PC.
- **Reconnects never ask to pair.** They run in the background while the
  owner may be typing on another PC, so a peer whose key is not trusted is
  left alone until `mickey source` is restarted.
- **Connecting gives up after 2 s**, so an offline peer does not delay the
  others at start.
- Moving from one peer to the next ungrabs for a moment; the log says
  "back here" only when the focus really is local.

### Ticket 3 — spike: eframe/egui (2026-10-21)

Throw-away program outside the repository: a window with three PC cards,
arrows/Enter/Esc/click, and a second thread sending a message every second.

- **Works on both systems.** Hyprland: opens focused, renders, keys and
  clicks work, and `ctx.request_repaint()` from another thread updates it
  (checked by the agent itself with `grim` screenshots and `wtype` keys).
  Windows 10, cross-compiled: opens on top, renders.
- **Hyprland tiles it** like any window, ignores the requested size, and
  opens it on whatever workspace has focus (even the special one). A
  switcher should float in the middle: a window rule on the app id
  (`with_app_id`) does that, and the README will show it.
- **Windows shows a console** next to it; `#![windows_subsystem = "windows"]`
  removes it for the GUI program.
- **Cost**, with `default-features = false` and only `glow`,
  `default_fonts`, `wayland`, `x11`:

  | | Mickey today | Spike |
  |---|---|---|
  | Crates in the tree | 37 | 124 |
  | Release build, clean | — | 70 s Linux, 53 s Windows (cross) |
  | Binary (stripped) | 1.2 MB | 8.5 MB Linux, 13 MB Windows |
  | Resident memory | ~1 MB | 90 MB Linux, 125 MB Windows |

- **Decision (with the owner): the GUI is its own program, `mickey-ui`**,
  started only while a window is on screen. `mickey source`, which runs all
  day and holds the keyboard, stays small, never links the GUI crates, and
  cannot be stuck by a GUI that hangs or crashes.

### Ticket 4 — the switcher (2026-10-21)

- **The source talks to `mickey-ui` through its command line and exit
  code**: `mickey-ui switcher <focused> online:<name>|offline:<name>...`
  prints the picked card and exits 0, exits 1 on Esc. A thread waits for it
  and hands the pick to the main loop, which polls every 10 ms while the
  window is open. If `mickey-ui` cannot run, the hotkey falls back to going
  to the first online peer.
- **From a remote peer the hotkey first comes home** (releases, ungrab),
  then opens the switcher, because the window lives on this PC's screen.
- **Hyprland 0.56 uses a Lua config**; `hyprctl keyword` no longer accepts
  rules. `hyprctl eval 'hl.window_rule({...})'` adds one at runtime, which
  is how the float/center/pin rule in the README was checked: the window
  opened floating, pinned and centred (580, 347 on a 1536×864 logical
  screen).
- **The built-in egui fonts have no arrows or bullets**; the labels are
  plain ASCII.

