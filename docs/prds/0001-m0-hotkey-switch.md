# 0001 — M0: It Crosses

> Status: **draft** · Depends on: nothing · Vision: [0000](0000-vision.md)

## Goals

Prove the whole pipeline end to end, across operating systems: read real input
on the owner's Linux machine, send it over the network, and turn it into real
input on a Windows PC.

Done means: with the Linux machine and the Windows PC on the same LAN,

```sh
# on Windows (the machine to be controlled)
mickey.exe target --listen 192.0.2.10:4242

# on Linux (the machine with the keyboard and mouse)
mickey source --to 192.0.2.10:4242
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
- [ ] A fixed hotkey toggles focus local ⇄ remote: **AltGr (Right Alt) +
      Right Shift**, firing when the second of the two is released and no
      other key was pressed in between (so `AltGr+Shift+Q` never switches).
      The owner's laptop has no Right Ctrl (see *Discoveries*). It becomes
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

### Spike 0b — `SendInput` on Windows (2026-10-21)

- **Bindings: hand-written.** One function (`SendInput`) and three structs
  (`INPUT`, `MOUSEINPUT`, `KEYBDINPUT`) in `platform/windows.rs`, about 60
  lines. A compile-time assertion checks `size_of::<Input>() == 40`, the size
  of `INPUT` on 64-bit Windows, because a wrong layout makes the call fail.
  `windows-sys` is not needed.
- **Build: cross-compile from Arch.** `rustup target add
  x86_64-pc-windows-gnu` plus `mingw-w64-gcc`, then
  `cargo build --target x86_64-pc-windows-gnu`. No Rust on the Windows PC.
  The `.exe` is copied over SSH (OpenSSH Server on Windows 10 19045).
- **SSH cannot inject input directly.** A program started from `sshd` runs in
  session 0 (`services`), not the owner's desktop (session 1, `console`), and
  `SendInput` fails there with "Access denied" (os error 5). To test from
  Linux, run it in the interactive session through a scheduled task:
  `schtasks /create /tn mickey-spike /tr "cmd /k C:\mickey\mickey.exe" /sc once /st 23:59 /it /f`
  then `schtasks /run /tn mickey-spike`.
- **The console window takes focus.** `mickey.exe` is a console program, so
  launching it opens a terminal that receives the injected keys. Expected:
  in M0 the target runs in a terminal anyway, and input goes to whatever
  window has focus, like a real keyboard.
- **Checked:** "hello" typed by scan code and the cursor drew a square on
  the owner's Windows 10.

### Spike 0a — evdev grab on Hyprland (2026-10-21)

- **Bindings: hand-written, no crate.** `ioctl` is declared in
  `platform/linux.rs` (glibc is already linked by `std`); `EVIOCGRAB` and
  `EVIOCGKEY` match the values from `<linux/input.h>` (`0x40044590`,
  `0x80604518` for 96 bytes). Events are read with plain `File::read_exact`
  into 24 bytes, so reading needs no `unsafe`. The `evdev` crate is not
  needed.
- **Grab works on Hyprland.** While grabbed, typing reaches nothing on the
  desktop; releasing the hotkey gives the keyboard back at once. The kernel
  also drops the grab when the file is closed, so even a killed process
  frees the keyboard.
- **Ctrl+C cannot work during a grab**: the terminal never sees the keys.
  The way home is the hotkey; the spike adds a 10-second limit as a net.
- **Hotkey changed to AltGr + Right Shift.** The owner's laptop (Acer,
  ABNT2) has no Right Ctrl. AltGr arrives as `KEY_RIGHTALT` (100).
- **Permissions without logging out.** After `usermod -aG input`, the group
  only applies to new logins; `newgrp input` gives a shell with it now.
  Open question for later: the `input` group lets any program of the user
  read the keyboard. The Wayland InputCapture portal (libei) avoids it.
- **The touchpad sends absolute positions, not motion.** Its two devices:
  "…Touchpad" sends `EV_ABS` (multitouch positions) and was the only one
  with data; "…Mouse" has `REL_X`/`REL_Y` but stayed silent. Capturing
  only `REL_X`/`REL_Y` mice, as planned, would ignore the owner's touchpad.

### Step 2 — wire format (2026-10-21)

- **One length byte is enough.** The longest frame is 10 bytes (length,
  type, two `i32`), so the length prefix is a single `u8`. A future message
  longer than 255 bytes would need a wider prefix and a new `VERSION`.
- **`Ping` and `Pong` carry a `u32` id**, which the PRD did not specify, so
  the source can match each `Pong` to the `Ping` it timed.
- **Big-endian** (network byte order) for every number.

### Step 5 — `SendInput` emit (2026-10-21)

- **Checked on the owner's Windows 10** with a fake source over the LAN: a
  square of motion, letters, Shift + the ABNT2 `/ ?` key (`0x73`) typing
  `?`, and an Up arrow held at disconnect released by the target.
- **Windows Firewall** asks once, the first time `mickey.exe` listens.
  Allowing it on private networks only was enough for the LAN.

### Step 6 — evdev capture (2026-10-21)

- **Discovery by capability works on the owner's laptop.** Of the eleven
  `/dev/input/event*` nodes, exactly two qualify: the "AT Translated Set 2
  keyboard" (`KEY_A` and `KEY_Z`) and the touchpad's silent "…Mouse" node
  (`REL_X`, `REL_Y`, `BTN_LEFT`). Power buttons, video bus and the Acer
  hotkey nodes are skipped.
- **Reading blocks, one thread per device.** The non-blocking polling of
  spike 0a is gone; the main thread waits on the channel with a timeout.
- **Checked:** grabbed both devices, 99 key events read while nothing
  reached the desktop, AltGr + Right Shift released them at once.
