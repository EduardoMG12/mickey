# Mickey

**One keyboard and mouse for all your computers, written in Rust.**
Move the cursor off the edge of one screen and it appears on the next
computer, whatever its distro, desktop or operating system.
Inspired by Synergy, Deskflow/Input Leap and lan-mouse, built from scratch to
be understood line by line.

> **Status: pre-alpha, documentation only.** No code yet. The plan lives in
> [`docs/prds/`](docs/prds/). Milestone M0 is next.

---

## What it will be

- **One binary, every machine.** The same `mickey` runs on each computer. You
  tell it where the other screens are (left, right, above, below).
- **Works on any desktop.** X11, GNOME, KDE, Hyprland, Sway: on Linux the
  hard part is the compositor, not the distro, and Mickey has a backend
  for each, plus a kernel-level fallback (`evdev`/`uinput`) that works
  everywhere. Windows too, from the first milestone.
- **Feels local.** Moving the mouse on a remote screen should add no latency
  you can notice. Hard budgets, measured.
- **Safe on a shared network.** Everything you type crosses the network, so
  traffic is encrypted and machines must be paired before they can talk
  (from M1 on).

## Roadmap

| Milestone | What you get | Status |
|---|---|---|
| **M0: It crosses** | Press a hotkey, and a Linux PC's keyboard and mouse drive a Windows PC; press it again to come back ([PRD](docs/prds/0001-m0-hotkey-switch.md)) | next |
| **M1: Screens have edges** | 3+ PCs in a layout: the cursor leaves your screen and enters the neighbor you configured, and back; Linux targets; encryption and pairing | planned |
| **M2: Any desktop** | Native backends: X11, Wayland portals (GNOME/KDE), wlroots; Windows as the source | planned |
| Later | Shared clipboard, file drag and drop, automatic discovery (mDNS), GUI/tray, macOS | ideas |

## Architecture at a glance

```
┌─────────────── mickey on the machine with the keyboard ───────────────┐
│  capture backend ──► core: layout, focus, pressed keys ──► transport  │
└───────────────────────────────────────────────────────────────┬───────┘
                                                                │ LAN
┌─────────────── mickey on the controlled machine ──────────────▼───────┐
│  transport ──► core ──► emit backend (uinput, XTest, SendInput, …)    │
└───────────────────────────────────────────────────────────────────────┘
```

- **Capture** reads the physical keyboard and mouse. **Emit** creates input on
  the controlled machine. Each is per-platform; everything else is shared.
- **All `unsafe` code is confined to the platform modules**; the core and the
  protocol are safe Rust.

Full design: [docs/prds/0000-vision.md](docs/prds/0000-vision.md).

## How this project is developed

Mickey is a learning project, and its process is part of the point:

- Every PR is small (≤ ~300 lines) and carries a **review challenge**,
  questions only someone who actually read (and sometimes debugged) the diff
  can answer.
- When a classic algorithm or data structure shows up (state machine, graph,
  set, framing a byte stream, …), the PR explains the concept and the matching
  LeetCode problem.
- Only the owner merges.

Details: [CONTRIBUTING.md](CONTRIBUTING.md). Rules for AI assistants:
[CLAUDE.md](CLAUDE.md).

## License

[GPL-3.0-or-later](LICENSE).
