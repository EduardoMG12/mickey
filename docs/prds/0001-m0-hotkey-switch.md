# 0001 — M0: It Crosses

> Status: **draft** · Depends on: nothing · Vision: [0000](0000-vision.md)

## Goals

Prove the whole pipeline end to end: read real input on one Linux machine,
send it over the network, and turn it into real input on another.

Done means: with two Linux machines on the same LAN,

```sh
# on the target (the machine to be controlled)
mickey receive --listen 192.168.0.20:4242

# on the source (the machine with the keyboard and mouse)
mickey send --to 192.168.0.20:4242
```

pressing the hotkey on the source moves the keyboard and mouse to the target:
the source's cursor freezes, the target's cursor moves, typing appears on the
target, clicks and scroll work. Pressing the hotkey again returns control.
`Ctrl+C`, a crash or a lost connection always gives the source its keyboard and
mouse back. Nothing more.

## Requirements

### CLI
- [ ] `mickey receive --listen <addr:port>` and `mickey send --to <addr:port>`.
- [ ] Bad arguments, a missing `/dev/uinput` or missing permissions print a
      one-line error that says how to fix it, and exit non-zero. No panic.
- [ ] Both sides print a startup warning: traffic is unencrypted, use only on
      a trusted network.

### Capture (source)
- [ ] Find every keyboard and mouse under `/dev/input/event*` by capability
      (`EV_KEY` with letter keys; `EV_REL` with `REL_X`/`REL_Y`).
- [ ] Read events from all of them while focus is local (to watch for the
      hotkey) without disturbing the desktop.
- [ ] While focus is remote, `EVIOCGRAB` every device so the local desktop
      sees nothing.
- [ ] The grab is released on hotkey, `Ctrl+C`/SIGTERM, panic and disconnect,
      through a guard type's `Drop`.

### Hotkey and focus
- [ ] A fixed hotkey toggles focus local ⇄ remote (chosen in step 0; see Open
      Questions).
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
      `Scroll { vertical, horizontal }` (including hi-res wheel),
      `ReleaseAll`, `Ping`/`Pong`.
- [ ] Unknown message types or malformed frames close the connection with a
      logged error; they never panic.

### Emit (target)
- [ ] Create one virtual device through `/dev/uinput` at startup with all
      keyboard keys, mouse buttons, relative axes and wheels.
- [ ] Apply received events and send `SYN_REPORT` in the same batch the source
      had.
- [ ] On disconnect: release everything held, keep listening for the next
      connection.

### Tests
- [ ] Unit tests: protocol round-trip for every message, malformed input
      rejection, hotkey state machine, pressed-set release.
- [ ] Integration test: `send` and `receive` on localhost with a fake capture
      and a fake emitter (no `/dev/input`, no `/dev/uinput`), asserting that
      the emitted sequence equals the captured one and that a dropped
      connection emits releases.
- [ ] Manual test (in the PR): two real machines, 10 minutes of normal use,
      latency p99 from `Ping`/`Pong` reported, no stuck keys after 20 switches.

## Non-Goals

- Screen edges, layout, more than two machines (M1).
- Encryption, pairing (M1). M0 is trusted-LAN only.
- Config file. Everything comes from CLI flags.
- Native desktop backends (X11, portals, wlroots), Windows (M2).
- Clipboard.
- Both directions at once: in M0 the source only sends and the target only
  receives.

## Constraints

- **Permissions.** `/dev/input/event*` needs the `input` group; `/dev/uinput`
  is usually root-only. Running as root is not the answer: document a udev rule
  (`KERNEL=="uinput", GROUP="input", MODE="0660"`) and adding the user to
  `input`. On the owner's Arch machine `/dev/uinput` is currently
  `crw------- root`.
- **The stuck-modifier trap.** If the grab starts while the hotkey is still
  physically held, the local desktop never sees those keys go up and thinks
  they stay pressed forever. Grab only after all keys are up.
- **Don't capture yourself.** If a machine ever runs both roles, capture must
  skip Mickey's own uinput device or events loop forever. M0 excludes it by
  name anyway.
- **Every `unsafe` block** (ioctls: `EVIOCGRAB`, `UI_DEV_SETUP`, `UI_SET_*`)
  lives in `platform/` with a `// SAFETY:` comment.
- **Keyboard layout is the target's.** We send physical key positions. If the
  two machines use different layouts (e.g. ABNT2 vs US), the target's wins.
  That is correct, but surprising; document it.

## Approach

1. **Kernel interfaces directly.** Read `struct input_event` from
   `/dev/input/eventN`, write it to `/dev/uinput`. Whether we write the ioctl
   wrappers by hand (with `libc`) or use the `evdev` crate is decided in the
   spike (step 0). Leaning toward by hand, like HypeStation's libretro
   bindings: it is small and every line teaches something.
2. **Threads and channels, no async.** One reader thread per input device,
   all feeding one `std::sync::mpsc` channel into the sender. The target has
   one thread reading the socket and writing uinput. Async (`tokio`) is not
   needed for two sockets and a handful of files.
3. **Our own `InputEvent` type** in `event.rs`. The platform module converts
   from/to kernel structs; the protocol and focus logic never see them.

Order of work (each step is one PR, each ≤ ~300 lines):

0. Spike: on the owner's Hyprland machine, read a keyboard with `evdev`, grab
   it, create a uinput device and type "hello". Hand-written ioctls vs the
   `evdev` crate; pick the hotkey. Record in *Discoveries*.
1. Workspace skeleton, CLI (`send`/`receive`), errors, CI.
2. `InputEvent` + protocol encode/decode + round-trip tests.
3. TCP transport + fake capture/emitter + localhost integration test.
4. uinput emit backend (target works; test with a local script).
5. evdev capture backend + grab guard.
6. Hotkey/focus state machine + pressed-set release.
7. Ping/Pong latency, two-machine manual verification, 10 minutes.

## Concepts in this milestone

- **State machine**: focus is `Local → WaitingForKeysUp → Remote → Local`.
  Closest LeetCode: #65 *Valid Number* (a classic explicit-state-machine
  problem). Explicit states make the "grab too early" bug impossible to write,
  where a pile of booleans would not.
- **Set of held keys**: a `HashSet`/bitset of pressed codes, so a lost
  connection releases exactly what is held. Closest LeetCode: #705 *Design
  HashSet*. With ~250 possible key codes a fixed bitset beats a hash set.
- **Framing a byte stream**: TCP delivers bytes, not messages; a length prefix
  turns a stream back into messages. Closest LeetCode: #271 *Encode and Decode
  Strings*.
- **RAII / guard types**: `Drop` releases the grab on every path out, including
  panics.
- **Nagle's algorithm**: why `TCP_NODELAY` matters for tiny, frequent messages.

## Open Questions

- **Hotkey.** Must be easy to press, not already bound in Hyprland, and exist
  on every keyboard (Scroll Lock often does not). Candidate: `Right Ctrl +
  Right Shift`. *(user decision, in step 0)*
- **Hand-written ioctls vs the `evdev` crate.** *(needs exploration: step 0)*
- **Which machine is the M0 target?** Depends on the owner's other two
  machines. *(user decision)*

## Discoveries

*(Filled in during execution.)*
