# 0001 — M0: It Crosses

> Status: **draft** · Depends on: nothing · Vision: [0000](0000-vision.md)

## Goals

Prove the whole pipeline end to end, across operating systems: read real input
on the owner's Linux machine, send it over the network, and turn it into real
input on a Windows PC.

Done means: with the Linux machine and the Windows PC on the same LAN,

```sh
# on Windows (the machine to be controlled)
mickey.exe target --listen 192.168.0.20:4242

# on Linux (the machine with the keyboard and mouse)
mickey source --to 192.168.0.20:4242
```

pressing the hotkey on Linux moves the keyboard and mouse to Windows: the Linux
cursor freezes, the Windows cursor moves, typing appears on Windows, clicks,
side buttons and scroll work. Pressing the hotkey again returns control.
`Ctrl+C`, a crash or a lost connection always gives Linux its keyboard and
mouse back. Nothing more.

Screen edges and more than two machines come in M1a; the hotkey is the
simplest possible way to switch, so M0 can focus on the pipeline.

## Requirements

### CLI
- [ ] `mickey target --listen <addr:port>` (Windows) and
      `mickey source --to <addr:port>` (Linux). The names match
      [CONTEXT.md](../../CONTEXT.md).
- [ ] Bad arguments, missing permissions on `/dev/input` or a port already in
      use print a one-line error that says how to fix it, and exit non-zero.
      No panic.
- [ ] Both sides print a startup warning: traffic is unencrypted, use only on
      a trusted network.

### Capture (Linux source)
- [ ] Find every keyboard and mouse under `/dev/input/event*` by capability
      (`EV_KEY` with letter keys; `EV_REL` with `REL_X`/`REL_Y`).
- [ ] Read events from all of them while focus is local (to watch for the
      hotkey) without disturbing the desktop.
- [ ] While focus is remote, `EVIOCGRAB` every device so the local desktop
      sees nothing.
- [ ] The grab is released on hotkey, `Ctrl+C`/SIGTERM, panic and disconnect,
      through a guard type's `Drop`.

### Hotkey and focus
- [ ] A fixed hotkey toggles focus local ⇄ remote: **Right Ctrl + Right
      Shift**, firing when the second of the two is released and no other key
      was pressed in between (so `Ctrl+Shift+T` never switches). It becomes
      configurable in M1a, when `config.toml` exists.
- [ ] The hotkey's own key presses are never sent to the target.
- [ ] Before the grab starts, wait until every key is released locally (see
      Constraints).
- [ ] Track the set of keys and buttons currently held on the target. When
      focus returns, or the connection drops, send a release for each.

### Protocol and transport
- [ ] One TCP connection, `TCP_NODELAY` on.
- [ ] Our own binary encoding: a length-prefixed frame per message, fixed
      endianness, a version byte in a hello message. No serialization crate.
- [ ] Messages: `Hello { version }`, `Key { code, pressed }`,
      `Button { code, pressed }`, `Motion { dx, dy }`,
      `Scroll { vertical, horizontal }` (hi-res units, 120 per notch),
      `ReleaseAll`, `Ping`/`Pong`.
- [ ] Unknown message types or malformed frames close the connection with a
      logged error; they never panic.

### Key mapping (pure Rust, tested on every platform)
- [ ] Translate evdev key codes to Windows **scan codes**, including the
      "extended" keys (arrows, Right Ctrl/Alt, Insert/Delete/Home/End,
      Page Up/Down, keypad Enter and `/`, Super) that need the `E0` prefix.
- [ ] Keys with no Windows equivalent are dropped and logged once at debug
      level (by code, never by typed text).
- [ ] Map mouse buttons: left, right, middle, side/extra (`BTN_SIDE`,
      `BTN_EXTRA` → `XBUTTON1`, `XBUTTON2`).

### Emit (Windows target)
- [ ] Inject with `SendInput`: keys as scan codes (`KEYEVENTF_SCANCODE`), so
      Windows applies its own keyboard layout; relative mouse motion;
      buttons; vertical and horizontal wheel.
- [ ] On disconnect: release everything held, keep listening for the next
      connection.

### Tests
- [ ] Unit tests: protocol round-trip for every message, malformed input
      rejection, key mapping table, hotkey state machine, pressed-set release.
- [ ] Integration test: `source` and `target` on localhost with a fake capture
      and a fake emitter (no `/dev/input`, no `SendInput`), asserting that the
      emitted sequence equals the captured one and that a dropped connection
      emits releases.
- [ ] CI runs on **Linux and Windows** runners.
- [ ] Manual test (in the PR): Linux → Windows 10 (the owner's PC; Windows 11
      should work but is untested), 10 minutes of normal use
      (typing with accents, shortcuts like `Ctrl+C`/`Alt+Tab`, scrolling,
      side buttons), latency p99 from `Ping`/`Pong` reported, no stuck keys
      after 20 switches.

## Non-Goals

- Screen edges, layout, more than two machines (M1a).
- Linux as the target (`uinput`, M1a). Windows as the source (M2).
- Encryption, pairing (M1a). M0 is trusted-LAN only.
- Config file. Everything comes from CLI flags.
- Clipboard.
- Running as a Windows service or at login. M0 runs from a terminal.

## Constraints

- **Linux permissions.** `/dev/input/event*` needs the `input` group (the owner
  is not in it today). Document adding the user to `input`; never run Mickey
  as root.
- **The stuck-modifier trap.** If the grab starts while the hotkey is still
  physically held, the Linux desktop never sees those keys go up and thinks
  they stay pressed forever. Grab only after all keys are up.
- **What Windows won't let us touch.** `SendInput` from a normal process
  cannot reach windows running as administrator (UIPI), the UAC prompt, the
  lock screen or `Ctrl+Alt+Del` (secure desktop). This is a Windows rule, not
  a bug; document it as a known limit.
- **Windows Firewall.** The receiver listens, so Windows asks to allow it the
  first time. Allow it on private networks only; document it.
- **Keyboard layout is the target's.** We send physical key positions. If
  Linux uses ABNT2 and Windows uses US, Windows' layout decides what is
  typed. Correct, but surprising; document it. Same for Caps/Num Lock state,
  which lives on each machine.
- **Mouse acceleration.** evdev gives raw deltas, before Linux's acceleration;
  Windows applies its own ("Enhance pointer precision"). The cursor may feel
  slightly different on each screen. Expected, not a bug.
- **Every `unsafe` block** (Linux ioctls, Win32 calls) lives in `platform/`
  with a `// SAFETY:` comment.

## Approach

1. **Kernel interface on Linux.** Read `struct input_event` from
   `/dev/input/eventN`. Hand-written ioctl wrappers (with `libc`) vs the
   `evdev` crate is decided in spike 0a.
2. **Win32 on Windows.** `SendInput` and its `INPUT` struct. Hand-written
   `extern "system"` bindings vs Microsoft's `windows-sys` crate is decided in
   spike 0b. Leaning toward hand-written for both OSes, like HypeStation's
   libretro bindings: they are small and every line teaches something.
3. **Threads and channels, no async.** One reader thread per input device,
   all feeding one `std::sync::mpsc` channel into the sender. The receiver has
   one thread reading the socket and calling `SendInput`.
4. **Our own `InputEvent` type** in `event.rs`, using evdev codes. The key
   mapping to Windows is a separate, pure module, so it is tested on Linux CI
   too.
5. **No traits yet.** One capture backend (evdev) and one emit backend
   (Windows) behind `#[cfg(target_os)]`. The emit trait arrives with the
   second emit backend (`uinput`, M1a).

Order of work (each step is one PR, each ≤ ~300 lines):

- 0a. Spike, Linux: read a keyboard with evdev on Hyprland, grab it, release
  it. Hand-written ioctls vs `evdev` crate; check that the hotkey works.
- 0b. Spike, Windows: a tiny program that types "hello" and moves the mouse
  with `SendInput`. Hand-written bindings vs `windows-sys`; build natively
  on Windows vs cross-compile from Arch (`x86_64-pc-windows-gnu`). Both
  spikes record their decisions in *Discoveries*.
1. Workspace skeleton, CLI (`source`/`target`), errors, CI on Linux + Windows.
2. `InputEvent` + protocol encode/decode + round-trip tests.
3. TCP transport + fake capture/emitter + localhost integration test.
4. evdev → Windows key and button mapping + tests.
5. Windows `SendInput` emit backend.
6. evdev capture backend + grab guard.
7. Hotkey/focus state machine + pressed-set release.
8. Ping/Pong latency, Linux → Windows manual verification, 10 minutes.

## Concepts in this milestone

- **State machine**: focus is `Local → WaitingForKeysUp → Remote → Local`.
  Closest LeetCode: #65 *Valid Number* (a classic explicit-state-machine
  problem). Explicit states make the "grab too early" bug impossible to write,
  where a pile of booleans would not.
- **Lookup table (direct addressing)**: evdev codes are small integers
  (0–255 for keys), so the evdev → scan code mapping is an array indexed by
  code: O(1), no hashing, and the whole mapping is visible in one place.
  Closest LeetCode: #13 *Roman to Integer* (symbol → value mapping).
- **Set of held keys**: a set of pressed codes, so a lost connection releases
  exactly what is held. Closest LeetCode: #705 *Design HashSet*. With ~250
  possible key codes a fixed bitset beats a hash set.
- **Framing a byte stream**: TCP delivers bytes, not messages; a length prefix
  turns a stream back into messages. Closest LeetCode: #271 *Encode and Decode
  Strings*.
- **RAII / guard types**: `Drop` releases the grab on every path out, including
  panics.
- **Nagle's algorithm**: why `TCP_NODELAY` matters for tiny, frequent messages.

## Open Questions

- **Hand-written bindings vs crates** (`evdev`, `windows-sys`).
  *(needs exploration: spikes 0a/0b)*
- **Build for Windows**: Rust installed on the Windows PC, or cross-compile
  from Arch and copy the `.exe`? *(needs exploration: spike 0b)*

## Discoveries

*(Filled in during execution.)*
