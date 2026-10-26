# 0005 — Input below the desktop: evdev and uinput on Linux, hooks and SendInput on Windows

- **Status:** Accepted
- **Date:** 2026-10-21 (spikes 0a and 0b, PRD 0001; the other direction,
  PRD 0008, PRs #77 and #78)

## Context

A **capture backend** must read the physical keyboard and mouse on the
source and, while focus is remote, stop the local desktop from seeing them.
An **emit backend** must create input on the target as if a real device
made it ([PRD 0000](../prds/0000-vision.md#vocabulary)).

On Linux the real variable is the display server, not the distro: X11,
GNOME, KDE, Hyprland and Sway each have different (or no) APIs for this.
Mickey "must not care which distro or desktop each computer runs". And a
capture that takes the keyboard away is dangerous: "a stuck grab locks the
owner out of their own desktop" (CLAUDE.md, hard rule 8).

## Decision

| | Capture | Emit |
|---|---|---|
| **Linux** | read `/dev/input/event*` (evdev), `EVIOCGRAB` while focus is remote | `/dev/uinput`: one virtual keyboard and mouse |
| **Windows** | low-level hooks `WH_KEYBOARD_LL` and `WH_MOUSE_LL` that *swallow* events while focus is remote, plus raw input for mouse motion | `SendInput`, keys as scan codes |

- **Bindings are hand-written** (`ioctl`, `SendInput`, hooks), no `evdev`
  or `windows-sys` crate; all of it in `crates/mickey/src/platform/`.
- **Linux permissions**: the user joins the `input` group, and a udev rule
  gives that group `/dev/uinput` (`packaging/linux/99-mickey-uinput.rules`).
  Mickey never runs as root.
- **A grab is always released**, as a guard type: `Grab` (Linux,
  `platform/linux.rs`) and `Grabbed` (Windows, `input.rs`) release in `Drop`,
  so returning for any reason, `?` and panics included, gives the devices
  back. The grab starts only after every key is up (the stuck-modifier
  trap). Keys still held on a target are released when focus leaves or the
  link drops (`held.rs`, `transport::serve`).
- **Ways home that do not depend on the guard**: on Linux the kernel drops
  the grab when the file closes (even on `kill -9`); on Windows a hook that
  does not answer in time is skipped (`LowLevelHooksTimeout`), Ctrl+Alt+Del
  is never hooked, and hooks die with the process. A peer silent for 4 s
  loses the focus. Holding Right Shift for 3 s (or with AltGr for 2 s)
  always comes home, whatever the shared hotkeys say (#182).

## Alternatives considered

- **Compositor or desktop APIs** (InputCapture and RemoteDesktop portals
  with libei on GNOME 45+ and KDE 6.1+; wlroots virtual-pointer protocols;
  XInput2 and XTest on X11). Not first: each covers some desktops, while
  evdev/uinput "work on every Linux desktop because they sit below the
  compositor" (vision, *The backend matrix*). They are planned for M2
  (#178), mainly to drop the `input` group.
- **The `evdev` crate and Microsoft's `windows-sys`.** Rejected after the
  spikes: the needed bindings were small (about 60 lines for `SendInput`)
  and "every line teaches something" (PRD 0001, *Spikes 0a and 0b*).
- **Marking our own `SendInput` events with `dwExtraInfo`** to avoid loops.
  Replaced by Windows' own injected flags (PRD 0008, *Discoveries*).
- **Running as root on Linux.** Rejected outright (PRD 0001, *Constraints*).

## Consequences

Good:

- One Linux backend works on Hyprland, GNOME, KDE, Cinnamon, XFCE and
  COSMIC (README, *Tested on*).
- Key codes are physical positions (evdev codes); the target applies its
  own layout. Mouse motion is raw deltas; the target applies its own
  acceleration.

Bad, and known limits:

- **The `input` group lets every program the user runs read the
  keyboard**, not only Mickey (README, *Known limits*; the installer says
  so). Portals are the long-term fix (#178).
- **evdev cannot see the cursor**, so screen edges on Linux need
  Hyprland's IPC (`monitors.rs`); elsewhere only hotkeys and the switcher.
- **Ctrl+C cannot reach a grabbed terminal**; the ways home above exist for
  that reason.
- **Windows refuses some targets**: `SendInput` cannot reach windows run as
  administrator from a normal process (UIPI), the UAC prompt, the lock
  screen or Ctrl+Alt+Del. The target sends `Refused` and the focus comes
  home ([ADR 0009](0009-windows-administrator-mode.md) covers admin
  windows).
- **Hooks cannot stop raw input**, which games read; Mickey shows an
  invisible foreground window while focus is remote (`platform/hooks.rs`).
- **The touchpad** sends absolute positions, converted to motion; tap to
  click and two-finger scroll are not done (PRD 0001, *Step 6b*).

## Links

- [PRD 0000](../prds/0000-vision.md#the-backend-matrix), *The backend matrix*.
- [PRD 0001, M0](../prds/0001-m0-hotkey-switch.md): spikes 0a and 0b,
  *Constraints*, steps 6 and 7.
- [PRD 0008](../prds/0008-windows-drives-linux.md); [PRD 0007](../prds/0007-ways-to-switch.md)
  (*A way home no setting can take away*).
- [architecture.md](../architecture.md#the-ways-home), *The ways home*.
- Issues #178, #182; PRs #16 (SendInput spike), #77, #78.
- Code: `crates/mickey/src/platform/linux.rs`, `platform/uinput.rs`,
  `platform/hooks.rs`, `platform/windows.rs`, `input.rs`, `emit.rs`,
  `held.rs`, `capture.rs`.
