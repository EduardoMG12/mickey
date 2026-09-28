# 0000 — Mickey Vision

> Status: **draft** · Owner: EduardoMG12 · Last updated: 2026-09-28
>
> This is the map. Milestone PRDs (`0001`, `0002`, …) are the territory.
> When they disagree, the milestone PRD wins and this file gets updated.

## Goals

Mickey lets one keyboard and mouse control several computers on the same
network. Screens are arranged in a layout (left, right, above, below); moving
the cursor past the edge of one screen moves control to the next computer.

It must not care which distro or desktop each computer runs. On Linux the
real variable is the **display server/compositor** (X11, GNOME, KDE, Hyprland,
Sway, …), not the distro, so Mickey is built around per-platform backends.

It is also a **learning project**. Every line is read and understood by its
owner before it is merged. Speed of delivery is explicitly less important than
understanding what was delivered. It will be open source; the repository stays
private until the owner decides otherwise.

Long-term "done": the owner's three computers sit side by side, and one
keyboard and mouse drive all three by moving across the screens, with no
noticeable lag, whatever each one runs.

## Vocabulary

| Term | Meaning |
|---|---|
| **Peer** | A computer running Mickey. |
| **Source** | The peer whose physical keyboard/mouse is being used right now. |
| **Target** | The peer currently receiving input from the source. |
| **Focus** | Which peer is the target. Exactly one at a time; may be the source itself (local). |
| **Capture backend** | Reads physical input on the source, and stops the local desktop from seeing it while focus is remote. |
| **Emit backend** | Creates input on the target as if a real device produced it. |
| **Layout** | Where each peer's screen sits relative to the others. |

## Architecture

- **One binary: `mickey`.** The same program runs on every peer. In M0 roles
  are fixed per run (`mickey send` / `mickey receive`); later every peer can be
  both.
- **Backends per platform.** Capture and emit are the only platform-specific
  parts. The core (focus, layout, pressed keys, hotkeys) and the protocol are
  plain, testable Rust.
- **Events are Linux evdev codes on the wire.** Key codes are *physical key
  positions* (`KEY_A` = the key where A sits on a US layout); the target
  applies its own keyboard layout. Mouse motion is raw relative deltas; the
  target's own acceleration applies. Non-Linux backends translate to/from
  evdev codes.
- **All `unsafe` lives in `platform/`.** Everything else is safe Rust and never
  sees a file descriptor ioctl or an OS handle.

### The backend matrix

| Platform | Capture | Emit | Milestone |
|---|---|---|---|
| Linux, any compositor (fallback) | `evdev` read + `EVIOCGRAB` | `uinput` | M0 |
| Hyprland / Sway (wlroots) | layer-shell edge surface + pointer constraints | `uinput` or virtual-pointer/keyboard protocols | M2 |
| GNOME 45+ / KDE 6.1+ | InputCapture portal + libei | RemoteDesktop portal + libei | M2 |
| X11 | XInput2 + grab | XTest | M2 |
| Windows | low-level hooks / Raw Input | `SendInput` | M2+ |
| macOS | `CGEventTap` | `CGEventPost` | later |

`evdev`/`uinput` work on every Linux desktop because they sit below the
compositor, in the kernel. Their cost: they need permission to
`/dev/input/*` and `/dev/uinput`, and they cannot see where the cursor is on
screen (so they cannot detect screen edges alone). That is why M0 switches with
a hotkey and edges come in M1.

### Repository layout

```
mickey/
├── Cargo.toml              # workspace
├── crates/
│   └── mickey/
│       └── src/
│           ├── main.rs     # CLI entry, wiring only
│           ├── event.rs    # InputEvent: our own type, no OS types
│           ├── protocol.rs # encode/decode events on the wire
│           ├── focus.rs    # hotkey + focus state machine, pressed keys
│           └── platform/   # all unsafe: evdev, uinput, later X11/portals/Windows
├── docs/prds/              # product requirement docs
├── CLAUDE.md               # rules for AI assistants
├── CONTRIBUTING.md         # rules for humans (and AI)
└── README.md
```

This layout is the target. Folders appear when their first file does, not
before.

### Installed layout

| What | Linux | Windows (later) |
|---|---|---|
| Config (`config.toml`): peers, layout, hotkey | `$XDG_CONFIG_HOME/mickey/` (`~/.config/…`) | `%APPDATA%\mickey\` |
| Pairing keys (from M1) | `$XDG_DATA_HOME/mickey/` | `%APPDATA%\mickey\` |

Rule: **no hardcoded absolute paths in code**, except well-known kernel paths
(`/dev/uinput`, `/dev/input/`). Config format: **TOML**.

## Performance budgets

Remote input has to feel local. These numbers are the bar. A change that
breaks one is a bug.

| Metric | Budget |
|---|---|
| Added latency, wired LAN | p99 < 2 ms from the source's kernel event to the target's `uinput` write |
| Added latency, Wi-Fi | measured and reported; no budget until we have numbers |
| Mouse rate | keeps up with a 1000 Hz mouse with no growing queue |
| Idle CPU | < 1 % of one core with no input |
| Memory | < 20 MB resident per peer |
| Switching focus | local cursor stops and remote input starts in the same event |

Latency is measured with a ping/echo over the same connection (RTT / 2), since
the two machines' clocks are not synchronized.

**Measure before optimizing.** A PR that claims a performance improvement shows
before/after numbers and how they were measured.

## Security

A keyboard sent over the network is a keylogger unless it is protected.

- M0 is **plaintext on a trusted LAN only**, and says so loudly at startup.
  The receiver listens only on the address given on the command line.
- From M1: encrypted transport and explicit pairing (trust on first use with a
  fingerprint the owner confirms on both screens, like SSH). An unpaired peer
  can connect but can inject nothing.
- Never: listening on all interfaces by default, internet relays, or any
  telemetry.

## Roadmap

### Platforms (in order)

1. The owner's machines (Linux; see Open Questions).
2. Linux in general, native backends for the main desktops.
3. Windows.
4. macOS (only if someone needs it).

### Milestones

| # | Name | PRD | Summary |
|---|---|---|---|
| M0 | It crosses | [0001](0001-m0-hotkey-switch.md) | Linux → Linux, `evdev` + `uinput`, hotkey switches focus, plaintext TCP |
| M1 | Screens have edges | 0002 (todo) | Edge crossing, 3+ peers in a layout, any peer can be the source, encryption and pairing |
| M2 | Any desktop | 0003 (todo) | Native backends (wlroots, portals, X11), Windows |

### Later (unordered, not committed)

- Shared clipboard (text first, then images)
- File drag and drop between screens
- Automatic peer discovery (mDNS)
- Tray icon / GUI for layout
- macOS backend
- Packaging: AUR, Flatpak, `.deb`, Windows installer

## Non-Goals

- Screen sharing or remote desktop. Mickey moves input, not pixels: every
  screen is physically in front of the owner.
- Control over the internet. LAN only.
- Distro-specific code paths. If something needs a distro check, the design is
  wrong.
- Abstractions built for a second use case that does not exist yet (the
  capture/emit traits appear when the second backend does, in M2).

## Open Questions

- **The owner's three machines.** OS, distro, desktop/compositor and X11 vs
  Wayland of each. Known so far: machine 1 is Arch + Hyprland (Wayland). This
  decides which M2 backends come first. *(user decision)*
- **Protocol transport.** TCP for everything (simple, ordered, no lost key
  releases) vs UDP for motion + TCP for keys (lower worst-case latency when
  packets drop). M0 uses TCP; revisit in M1 with numbers. *(needs exploration)*
- **Encryption in M1.** TLS (`rustls`) vs Noise (`snow`) vs QUIC (`quinn`).
  *(needs exploration)*
- **Symmetric peers.** Should every peer be able to act as the source (use
  any machine's keyboard to drive the others), or is one fixed source enough?
  *(user decision)*

## Discoveries

*(Filled in during execution: things we learned that change this map.)*
