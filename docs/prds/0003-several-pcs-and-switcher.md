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
- [ ] Built with **egui/eframe** (decided with the owner), on the Linux
      source. The same crate is reused by the setup window (step 3).
- [ ] While the switcher is open, nothing is grabbed: it is a normal window
      on the local desktop.

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
- **The window and the capture loop are separate.** eframe wants the main
  thread; the source's loop moves to its own thread and talks to the window
  over channels (`OpenSwitcher`, `Picked(peer)`, `Closed`).
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

*(Filled in during execution.)*
