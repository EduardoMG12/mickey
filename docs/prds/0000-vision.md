# 0000 — Mickey Vision

> Status: **draft** · Owner: EduardoMG12 · Last updated: 2026-10-22
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

Domain terms (peer, source, target, focus, switch, hotkey, layout) live in
[CONTEXT.md](../../CONTEXT.md). Two architecture terms used below:

| Term | Meaning |
|---|---|
| **Capture backend** | Reads physical input on the source, and stops the local desktop from seeing it while focus is remote. |
| **Emit backend** | Creates input on the target as if a real device produced it. |

## Architecture

- **One binary: `mickey`.** The same program runs on every peer, built for
  Linux and for Windows. In M0 roles are fixed per run (`mickey source` on
  Linux, `mickey target` on Windows). The goal is **symmetric peers**: every
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
| Linux, any compositor (fallback) | `evdev` read + `EVIOCGRAB`: **M0** | `uinput`: **done** ([0008](0008-windows-drives-linux.md)) | M0 / M1 |
| Windows | low-level hooks: **done** ([0008](0008-windows-drives-linux.md)) | `SendInput`: **M0** | M0 / M2 |
| Hyprland (owner's desktop) | cursor position for edge detection: M1 | — | M1 |
| Sway / other wlroots | layer-shell edge surface + pointer constraints | virtual-pointer/keyboard protocols | M2 |
| GNOME 45+ / KDE 6.1+ | InputCapture portal + libei | RemoteDesktop portal + libei | M2 |
| X11 | XInput2 + grab | XTest | M2 |
| macOS | `CGEventTap` | `CGEventPost` | later |

`evdev`/`uinput` work on every Linux desktop because they sit below the
compositor, in the kernel. Their cost: they need permission to
`/dev/input/*` and `/dev/uinput`, and they cannot see where the cursor is on
screen (so they cannot detect screen edges alone). That is why M0 switches with
a hotkey and edges come in M1a.

The hotkey stays after edges exist: it then always brings focus back to the
source. It is the way home when a target freezes or an edge cannot be
reached (hard rule 8).

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

- M0 was **plaintext on a trusted LAN only**, and said so loudly at
  startup. That ended with the secure link ([0002](0002-secure-link.md)):
  every link is encrypted now.
- From M1: encrypted transport and explicit pairing (trust on first use with a
  fingerprint the owner confirms on both screens, like SSH). An unpaired peer
  can connect but can inject nothing.
- Never: listening on all interfaces by default, internet relays, or any
  telemetry. *(The default listen address changed; see Discoveries,
  2026-10-22.)*

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

Order agreed with the owner on 2026-10-21: secure link, then steps 1–4.


| # | Name | PRD | Summary |
|---|---|---|---|
| M0 | It crosses | [0001](0001-m0-hotkey-switch.md) | Linux → Windows: `evdev` capture, `SendInput` emit, hotkey switches focus, plaintext TCP |
| M1a, first | Secure link | [0002](0002-secure-link.md) | Noise `XX` encryption and SSH-style pairing (fingerprints confirmed on both screens), brought forward from M1a so Mickey is safe before anything else |
| Step 1–2 | Several PCs and the switcher | [0003](0003-several-pcs-and-switcher.md) | `config.toml` with several peers; the hotkey opens a "Win+Tab for computers" window (egui) when there are 3+ PCs |
| Step 2b | Ways to switch | [0007](0007-ways-to-switch.md) | Control socket and commands (`mickey status`, `go`, `home`, `switcher`), a Waybar module, a corner indicator, configurable hotkeys including one per PC |
| Step 3 | Setup window | [0006](0006-setup-window.md) | A GUI that adds and pairs PCs, changes settings (touchpad speed, hotkey) and places each PC and its monitors, editing the same `config.toml` |
| Step 4 | Screens have edges | [0005](0005-screen-edges.md) | The cursor crosses to the neighbor placed in step 3, and back; the hotkey stays the way home. On Wayland this needs a spike: apps cannot read the cursor position |
| Both ways | Windows drives Linux | [0008](0008-windows-drives-linux.md) | Linux targets (`uinput`) and Windows as the source (low-level hooks), brought forward from M1/M2 at the owner's request: peers become symmetric |
| Tray | Tray icon | [0009](0009-tray-icon.md) | An icon in the system tray of every PC, with a green or red dot for "the keyboard and mouse are here", and a menu of PCs |
| Pairing | Connect without a terminal | [0010](0010-easy-pairing.md) | PCs find each other on the LAN; the same six-digit code on both screens; Accept in a window; no restart |
| Clipboard | Shared clipboard and history | [0011](0011-shared-clipboard.md) | Copy on one PC, paste on another (text, images, files) and a history of every PC's copies; synced when the keyboard crosses, big things on paste, on a second link |
| M2 | Any desktop | 0004 (todo) | Native backends (wlroots, portals, X11); Windows as the source |

### Later (unordered, not committed)

- ~~Shared clipboard (text first, then images)~~ *(done: [0011](0011-shared-clipboard.md))*
- File drag and drop between screens
- ~~Automatic peer discovery (mDNS)~~ *(done without mDNS: a UDP broadcast, [0010](0010-easy-pairing.md))*
- ~~Tray icon~~ *(done: [0009](0009-tray-icon.md))*
- macOS backend
- Packaging: AUR, Flatpak, `.deb`, Windows installer

### Someday (very long term, not committed)

Ideas the owner wants to keep in view. Each one is a different problem from
moving input, so each needs its own PRD before any work starts, and they stay
non-goals until then.

- ~~Sharing headphones and audio between peers.~~ *(in progress: [0012](0012-shared-audio.md))*
- Sharing other USB peripherals.
- Remote control in the style of TeamViewer: screen sharing and control over
  the internet.

## Non-Goals

Until a *Someday* item gets its own PRD:

- Screen sharing or remote desktop. Mickey moves input, not pixels: every
  screen is physically in front of the owner.
- Control over the internet. LAN only.
- Distro-specific code paths. If something needs a distro check, the design is
  wrong.
- Abstractions built for a second use case that does not exist yet (the
  emit trait appears when the second emit backend, `uinput`, does in M1a).

## Open Questions

- **The third peer's session.** The owner's three peers: Arch + Hyprland
  (Wayland, 2 monitors), Windows 10 (3 monitors) and Ubuntu with KDE
  (1 monitor). Does the Ubuntu peer run KDE on Wayland or X11? This decides
  how it detects edges in M1a. *(user fact, M1a grill)*
- **Layout unit (M1a).** Does the layout place whole peers (each one a block
  with its monitors inside, like Deskflow) or individual monitors? And do we
  need separate words for a monitor and for a peer's whole desktop?
  *(user decision, M1a grill)*
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
- ~~**Encryption in M1.** TLS (`rustls`) vs Noise (`snow`) vs QUIC (`quinn`).~~
  *(resolved: Noise `XX` through `snow`. Peers are identified by their own
  keys, so TLS certificates add nothing, and QUIC would also change the
  transport. See [0002](0002-secure-link.md).)*
- ~~**Symmetric peers.** Should every peer be able to act as the source, or is
  the Linux machine always the source?~~ *(resolved: symmetric. Every peer can
  drive every other one: the Windows keyboard can control Linux too. Linux
  becomes a target in M1 (`uinput`), Windows becomes a source in M2.)*

## Discoveries

### Listening on all interfaces (2026-10-22)

- **The default listen address is `0.0.0.0:4242`**, every interface of
  the PC (`[this_pc] listen`, `crates/mickey-config/src/lib.rs`), and
  discovery binds UDP `0.0.0.0:4243` (`crates/mickey/src/discovery.rs`).
  This breaks the "never listening on all interfaces by default" line
  above, on purpose: the other PCs on the LAN must be able to reach this
  one, and asking the owner for this PC's own IP is what PRD 0010 set out
  to remove (an IP also changes with DHCP and differs per adapter).
- **What still protects it:** nothing reaches input without pairing. A
  machine that is not paired can open the TCP port but can inject and
  read nothing (PRD 0002), and discovery packets carry no secret and are
  never trusted. On Windows the installer's firewall rule lets Mickey in
  on **private** networks only (`packaging/windows/mickey.nsi`), so a
  public Wi-Fi cannot reach it. The principle that remains: no internet
  relay, no telemetry, LAN only.
- **M0 is no longer plaintext.** Every link has been Noise-encrypted since
  PRD 0002; the "not encrypted" startup warning is gone.
