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

- **One binary: `mickey`.** The same program runs on every peer, built for
  Linux and for Windows. In M0 roles are fixed per run (`mickey send` on
  Linux, `mickey receive` on Windows). The goal is **symmetric peers**: every
  peer can be the source or a target, whichever keyboard you touch.
- **Backends per platform.** Capture and emit are the only platform-specific
  parts. The core (focus, layout, pressed keys, hotkeys) and the protocol are
  plain, testable Rust.
- **Events are Linux evdev codes on the wire.** Key codes are *physical key
  positions* (`KEY_A` = the key where A sits on a US layout); the target
  applies its own keyboard layout. Mouse motion is raw relative deltas; the
  target's own acceleration applies. Non-Linux backends translate to/from
  evdev codes (e.g. Windows scan codes).
- **All `unsafe` lives in `platform/`.** Everything else is safe Rust and never
  sees a file descriptor ioctl or an OS handle.

### The backend matrix

| Platform | Capture | Emit | Milestone |
|---|---|---|---|
| Linux, any compositor (fallback) | `evdev` read + `EVIOCGRAB`: **M0** | `uinput`: M1 | M0 / M1 |
| Windows | low-level hooks / Raw Input: M2 | `SendInput`: **M0** | M0 / M2 |
| Hyprland (owner's desktop) | cursor position for edge detection: M1 | — | M1 |
| Sway / other wlroots | layer-shell edge surface + pointer constraints | virtual-pointer/keyboard protocols | M2 |
| GNOME 45+ / KDE 6.1+ | InputCapture portal + libei | RemoteDesktop portal + libei | M2 |
| X11 | XInput2 + grab | XTest | M2 |
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
│           └── platform/   # all unsafe: linux/ (evdev, uinput), windows/ (SendInput)
├── docs/prds/              # product requirement docs
├── CLAUDE.md               # rules for AI assistants
├── CONTRIBUTING.md         # rules for humans (and AI)
└── README.md
```

This layout is the target. Folders appear when their first file does, not
before.

### Installed layout

| What | Linux | Windows |
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

1. **Linux → Windows**: the owner's Arch + Hyprland machine drives a Windows
   PC (M0).
2. **Several PCs at once**: screens in a layout, the cursor leaves one screen
   and enters the configured neighbor, Linux and Windows peers mixed (M1).
3. Linux in general (native backends for the main desktops) and Windows as the
   source (M2).
4. macOS (only if someone needs it).

### Milestones

| # | Name | PRD | Summary |
|---|---|---|---|
| M0 | It crosses | [0001](0001-m0-hotkey-switch.md) | Linux → Windows: `evdev` capture, `SendInput` emit, hotkey switches focus, plaintext TCP |
| M1 | Screens have edges | 0002 (todo) | Layout of 3+ peers; cursor crosses to the configured neighbor and back; Linux targets (`uinput`); encryption and pairing |
| M2 | Any desktop | 0003 (todo) | Native backends (wlroots, portals, X11); Windows as the source |

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
  emit trait appears when the second emit backend, `uinput`, does in M1).

## Open Questions

- **The owner's three machines.** Known so far: machine 1 is Arch + Hyprland
  (Wayland) and is the source; one machine runs Windows (10 or 11?). The
  third: OS and desktop? This decides which M1/M2 backends come first.
  *(user decision)*
- **Edge detection on Hyprland (M1).** The source must know when its cursor
  hits a screen edge. Hyprland's IPC (`cursorpos`) is simple but
  Hyprland-only; a wlroots layer-shell surface on the edge works on Sway too.
  *(needs exploration)*
- **Coming back (M1).** While focus is on Windows, who notices the cursor hit
  the edge that leads home: the target (reads its own cursor position after
  each move and tells the source) or the source (tracks a virtual cursor)?
  Leaning toward the target: it knows its real cursor, acceleration included.
  *(needs exploration)*
- **Protocol transport.** TCP for everything (simple, ordered, no lost key
  releases) vs UDP for motion + TCP for keys (lower worst-case latency when
  packets drop). M0 uses TCP; revisit in M1 with numbers. *(needs exploration)*
- **Encryption in M1.** TLS (`rustls`) vs Noise (`snow`) vs QUIC (`quinn`).
  *(needs exploration)*
- ~~**Symmetric peers.** Should every peer be able to act as the source, or is
  the Linux machine always the source?~~ *(resolved: symmetric. Every peer can
  drive every other one: the Windows keyboard can control Linux too. Linux
  becomes a target in M1 (`uinput`), Windows becomes a source in M2.)*

## Discoveries

*(Filled in during execution: things we learned that change this map.)*
