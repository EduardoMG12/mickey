# 0008 — Both ways: Windows drives Linux

> Status: **draft** · Depends on: [0002](0002-secure-link.md), [0003](0003-several-pcs-and-switcher.md) · Vision: [0000](0000-vision.md)

## Goals

The owner asked on 2026-10-21 for the other direction: the Windows PC's
keyboard and mouse driving the Linux laptop. With it, peers become
symmetric, as the vision always intended: each machine can be the source
or a target.

Done means: on the Windows PC, `mickey source` (with the laptop in its
`config.toml`) and the same hotkey, **Right Alt + Right Shift** (the
physical key is the same as AltGr: scan code `E0 38`), send the Windows
keyboard and mouse to the laptop, where `mickey target` types and moves
the cursor on Hyprland. Back with the hotkey. Pairing works as today.

## Approach

Two new platform backends, the mirror images of M0's:

| | M0 had | New |
|---|---|---|
| Linux | capture (`evdev`, `EVIOCGRAB`) | **emit** with `uinput`: a virtual keyboard and mouse the kernel feeds to the compositor like real ones |
| Windows | emit (`SendInput`) | **capture** with low-level hooks (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`): see every key and mouse event; while focus is remote, *swallow* them, which is Windows' version of a grab |

Everything between them (protocol, secure link, pairing, focus, hotkey,
switcher) is already platform-independent and is reused. Events keep using
evdev codes on the wire; Windows translates scan codes to evdev codes,
the reverse of `keymap.rs`.

## Requirements

### Linux target (uinput)
- [ ] `mickey target` runs on Linux: it creates one virtual device with the
      keys and buttons Mickey sends, `REL_X/REL_Y`, and both hi-res wheels.
- [ ] `/dev/uinput` is root-only on Arch. The README gives a one-line udev
      rule that lets the `input` group use it; Mickey's error says so.
- [ ] Relative motion only; the compositor applies its own acceleration.
- [ ] The virtual device disappears when Mickey exits (the kernel removes
      it when the file is closed).

### Windows source (low-level hooks)
- [ ] Keyboard and mouse hooks on a dedicated thread with a message loop.
- [ ] While focus is local, every event passes through untouched. While it
      is remote, the hooks return non-zero so Windows drops the event,
      and it is sent to the target instead.
- [ ] Mouse motion as deltas. Low-level mouse hooks report absolute
      positions, so the cursor is pinned at one point and deltas are
      measured from it (or Raw Input is used; decided in the spike).
- [ ] Scan codes (with the extended flag) → evdev codes, tested on every
      platform, including the ABNT2 and US-International keys.
- [ ] **The way home** (hard rule 8): the hotkey; Windows itself removes a
      hook that stops answering (`LowLevelHooksTimeout`); Ctrl+Alt+Del is
      never hooked; hooks are removed on exit, panic and disconnect.
- [ ] Our own `SendInput` events are marked (`dwExtraInfo`) and ignored by
      the hooks, so a PC that is both source and target never loops.

### Tests and checks
- [ ] Pure parts tested everywhere: reverse keymap, hook decision ("pass or
      swallow"), delta computation.
- [ ] Manual: the owner's Windows keyboard types on the laptop, the mouse
      moves it, the hotkey brings control back, no stuck keys; then both
      directions in one session.

## Non-Goals

- Screen edges (PRD 0005).
- A Windows service or start at login; Mickey runs from a terminal or the
  tray later.
- Injecting into elevated windows or the secure desktop on Windows; a hook
  does not see the secure desktop either, which is the safe direction.

## Constraints

- `unsafe` stays in `platform/`: hooks, message loop and `uinput` ioctls are
  all FFI.
- No new crates: hand-written bindings, like spike 0b decided.

## Order of work (one PR each)

1. Spike: `uinput` on Hyprland (permissions, a key and a motion appear).
2. Spike: Windows low-level hooks: see, swallow and release; how to get
   mouse deltas.
3. Reverse keymap (Windows scan codes → evdev).
4. Linux emit backend and `mickey target` on Linux.
5. Windows capture backend and `mickey source` on Windows.
6. Real check, both directions.

## Concepts in this milestone

- **Callbacks with a time limit**: Windows calls the hook for every event
  and skips it if it does not answer in time, so the hook must never block.
- **Inverse mapping**: building `scan code → evdev` from the existing table
  instead of writing a second one. Closest LeetCode: #205 *Isomorphic
  Strings* (a mapping and its inverse must agree).

## Discoveries

*(Filled in during execution.)*
