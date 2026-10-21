# 0007 — Ways to switch

> Status: **draft** · Depends on: [0003](0003-several-pcs-and-switcher.md) · Vision: [0000](0000-vision.md)

## Goals

The owner asked (2026-10-21) that each person chooses **how** they move
between PCs and **what** they see, all in the settings:

- only hotkeys, with no indicator on screen;
- a **Waybar** module: where the focus is and how many PCs are online;
  left click opens the switcher, right click the settings;
- a small **always-visible box in a corner** of the screen, for people
  without Waybar (and, later, a tray icon on Windows);
- **more hotkeys**: open the switcher, go home, go straight to a given PC.

Done means: on the owner's laptop, Waybar shows `Windows · 1/1` while the
focus is on the Windows PC and changes the moment the hotkey is pressed; a
Hyprland bind runs `mickey go Windows`; and all of it is turned on or off in
`config.toml`.

## Approach

`mickey source` is the only program that knows the focus and the peers, so
it answers questions and takes orders through a **control socket**: a Unix
socket at `$XDG_RUNTIME_DIR/mickey/control.sock` (the directory is created
`0700`; only the owner's user can connect, and nothing listens on the
network). Small commands talk to it:

| Command | Does |
|---|---|
| `mickey status` | prints the focus and each PC once |
| `mickey status --waybar` | prints one JSON line for Waybar **every time something changes** (Waybar keeps it running; no polling) |
| `mickey switcher` | opens the switcher window |
| `mickey settings` | opens the settings window (PRD 0006) |
| `mickey go <name>` | sends the keyboard and mouse to that PC |
| `mickey home` | brings them back |

Because they are ordinary commands, any launcher, bar or compositor bind can
use them: `bind = SUPER, 1, exec, mickey go Windows` on Hyprland, for
example. Compositor binds only work while the focus is local (when it is
remote, the keyboard is grabbed), so Mickey's **own hotkeys** stay the way
to move while remote.

### Waybar

```jsonc
"custom/mickey": {
  "exec": "mickey status --waybar",
  "return-type": "json",
  "on-click": "mickey switcher",
  "on-click-right": "mickey settings"
}
```

Each line is `{"text": "Windows · 1/1", "tooltip": "...", "class": "remote"}`;
the `class` (`local`, `remote`, `offline`) lets the bar's CSS color it, so a
glance tells you where you are typing.

### Settings (in `config.toml`, later also in the settings window)

```toml
[interface]
indicator = ["waybar"]        # any of "waybar", "corner"; [] for none

[hotkeys]
next     = "AltGr+RightShift"      # M0 hotkey: with 2+ PCs, opens the switcher
home     = "AltGr+RightShift+H"
switcher = "AltGr+RightShift+S"

[[peer]]
name = "Windows"
address = "192.0.2.10:4242"
hotkey = "AltGr+RightShift+1"     # straight to this PC
```

Names and default keys here are a proposal for the owner to confirm in this
PRD's review.

## Requirements

### Control socket
- [ ] `mickey source` creates `$XDG_RUNTIME_DIR/mickey/` with mode `0700` and
      listens on `control.sock`; a stale socket from a crash is replaced.
- [ ] One line per request, one line per answer: `status`, `follow`,
      `switcher`, `go <name>`, `home`. Unknown requests get an error line.
- [ ] `follow` keeps the connection and sends a status line on every change
      (focus, a PC going online or offline).
- [ ] Commands print a clear error when `mickey source` is not running.

### Waybar and corner indicators
- [ ] `mickey status --waybar` (JSON lines, as above). When `mickey source`
      is not running it prints an "off" line and retries, so Waybar never
      shows an error.
- [ ] `mickey-ui corner`: a small pinned window with the same text, for
      people without a bar. Hyprland rule documented like the switcher's.
- [ ] `[interface] indicator` decides which ones `mickey source` starts or
      announces.

### Hotkeys
- [ ] A hotkey is written as `Key+Key+…` with evdev key names for the
      modifiers (`AltGr`, `RightShift`, `LeftCtrl`, …) and plain names for
      the last key (`H`, `S`, `1`, …). Parse errors name the bad part.
- [ ] `next`, `home`, `switcher` and each `[[peer]] hotkey`, all optional;
      defaults keep today's behavior.
- [ ] The rule from M0 holds for every hotkey: it fires on release, only if
      nothing else was pressed, and its keys never reach the target.

### Tests
- [ ] Request/response parsing, status formatting (text and Waybar JSON),
      hotkey parsing, the hotkey matcher with several hotkeys.
- [ ] Localhost: a fake source answers `status` and `follow` over a real
      Unix socket.
- [ ] Manual: Waybar on the owner's laptop, a Hyprland bind to `mickey go`.

## Non-Goals

- A tray icon on Windows (Windows is not a source until M2).
- Controlling a *target* through the socket; only the source has one.

## Constraints

- No new dependencies expected: Unix sockets and the JSON line are small
  enough for the standard library (the JSON is written, never parsed).
- The socket is a new way in: only the owner's user may connect (`0700`
  directory), and the requests can only move the focus, never send input.

## Concepts in this milestone

- **Publish/subscribe**: `follow` clients get every change pushed to them
  instead of asking over and over. Closest LeetCode: #355 *Design Twitter*
  (followers receive what the people they follow post).
- **Prefix matching of key chords**: several hotkeys share the same first
  keys (`AltGr+RightShift`, `AltGr+RightShift+1`); the matcher must wait for
  the release to know which one was meant. Closest LeetCode: #208 *Implement
  Trie*.

## Open Questions

- Final names and default keys (see the proposal above).

## Discoveries

*(Filled in during execution.)*
