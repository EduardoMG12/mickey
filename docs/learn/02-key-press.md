# 2. The journey of a key press

> **The question:** the keyboard is on the laptop (the focus is remote).
> What happens between your finger pressing **A** on the Windows keyboard
> and the letter appearing on the laptop? How many threads touch it, and
> where could it get lost?

Guess first: count the hops. Then follow them.

## The map

```text
Windows PC (source)                                      Linux laptop (target)
hook thread -> translator thread -> main loop ==TCP 4242==> serve thread -> uinput
 hooks.rs        input.rs           source.rs   Noise XX     transport.rs    emit.rs
                                    focus.rs                 held.rs
```

Four threads, one channel, one encrypted TCP stream, one kernel device.

## Hop 1: Windows sees the key first (`platform/hooks.rs`)

A low-level keyboard hook is a function Windows calls for **every** key,
before any program sees it.

- [`platform/hooks.rs:577`](../../crates/mickey/src/platform/hooks.rs#L577)
  `keyboard_proc`: reads the scan code and the extended flag.
- [`platform/hooks.rs:583`](../../crates/mickey/src/platform/hooks.rs#L583):
  events with `LLKHF_INJECTED` pass untouched. That is how a PC that is
  both source and target never loops on its own `SendInput` (PRD 0008,
  *Discoveries*).
- [`platform/hooks.rs:567`](../../crates/mickey/src/platform/hooks.rs#L567)
  `report`: puts the event on a channel and returns `swallowing()`. When
  the focus is remote, the hook returns `1` and **Windows drops the key**:
  that is the Windows version of a grab. The hook must be quick; Windows
  skips a hook that takes too long.

## Hop 2: scan code becomes an evdev code (`input.rs`, `keymap.rs`)

- [`input/mod.rs:159`](../../crates/mickey/src/input/mod.rs#L159) `start`: a
  second thread reads the hook's channel, so the hook itself does nothing
  slow.
- [`input/mod.rs:202`](../../crates/mickey/src/input/mod.rs#L202) `translate`
  calls [`input/keymap.rs:100`](../../crates/mickey/src/input/keymap.rs#L100)
  `evdev_code`. **A** becomes `30`, Linux's `KEY_A`.

Why Linux codes on both systems? One vocabulary on the wire, and they are
*physical positions*: the target's own layout decides the character
([0000-vision.md](../prds/0000-vision.md), *Architecture*). That is why
the README asks for the same layout on both PCs.

The result is an [`InputEvent`](../../crates/mickey/src/input/event.rs#L5):
`Key { code: 30, pressed: true }`. No OS types past this point.

## Hop 3: the main loop decides (`source/mod.rs`, `focus.rs`, `input/hotkey.rs`)

- [`source/mod.rs:64`](../../crates/mickey/src/source/mod.rs#L64) `Wake`: one
  enum for everything that can wake the main loop: input, the tray,
  control commands, news from the target side.
  [`source/mod.rs:88`](../../crates/mickey/src/source/mod.rs#L88) turns an
  `InputEvent` into `Wake::Input`.
- [`source/mod.rs:669`](../../crates/mickey/src/source/mod.rs#L669): the loop
  blocks in `recv_timeout`. Idle, it wakes once a second, for the ping.
- [`source/mod.rs:697`](../../crates/mickey/src/source/mod.rs#L697): input goes
  to `switcher.handle`.
- [`focus.rs:86`](../../crates/mickey/src/focus.rs#L86) `Switcher::handle`
  is a three-state machine: `Local`, `WaitingForKeysUp(peer)`,
  `Remote(peer)` ([`focus.rs:23`](../../crates/mickey/src/focus.rs#L23)).
  In `Remote`, every event goes through `hotkey.feed`, which forwards it
  (or holds back the last key of a chord, in case it completes a hotkey;
  [`input/hotkey.rs:204`](../../crates/mickey/src/input/hotkey.rs#L204)), records it
  in a `HeldSet` and emits `Action::Send(peer, event)`.
- [`source/mod.rs:627`](../../crates/mickey/src/source/mod.rs#L627): `Action::Send`
  calls `link.send`. If it fails, the peer goes on the `lost` list and the
  focus comes home.

How did the focus become `Remote` in the first place? The hotkey moved it
to `WaitingForKeysUp`, and
[`source/mod.rs:481`](../../crates/mickey/src/source/mod.rs#L481) grabs only once
no key is down. Grabbing with a key held would leave it stuck down on the
desktop you are leaving.

## Hop 4: sealed and sent (`secure.rs`, `protocol.rs`, `transport.rs`)

- [`net/secure.rs:93`](../../crates/mickey/src/net/secure.rs#L93) `Link::send`:
  1. [`net/protocol.rs:117`](../../crates/mickey/src/net/protocol.rs#L117)
     `encode` writes `[type 2][code 0x001E][pressed 1]` into a fixed array
     on the stack;
  2. Noise's `write_message` encrypts it with ChaCha20-Poly1305 and adds a
     16-byte tag;
  3. [`net/secure.rs:356`](../../crates/mickey/src/net/secure.rs#L356)
     `write_noise` puts a 2-byte length in front and writes it in one
     go.
- No heap allocation on this path: fixed arrays, and `Message` is `Copy`
  ([architecture.md](../architecture.md), end of *One key press*).
- [`net/transport.rs:31`](../../crates/mickey/src/net/transport.rs#L31)
  `connect` set two things on this socket long ago: `TCP_NODELAY` (the
  kernel never holds a tiny frame back to batch it) and a 1 s write
  timeout (a stuck target cannot keep your keyboard).

## Hop 5: the laptop opens it (`target/mod.rs`, `net/transport.rs`, `input/held.rs`)

- [`target/mod.rs:22`](../../crates/mickey/src/target/mod.rs#L22) `target`: accepts
  connections, one thread per source, at most 32 at once
  ([`target/slots.rs:9`](../../crates/mickey/src/target/slots.rs#L9)).
- [`target/mod.rs:153`](../../crates/mickey/src/target/mod.rs#L153) `serve_source`:
  handshake, trust check (chapter 3), then
  [`net/transport.rs:123`](../../crates/mickey/src/net/transport.rs#L123) `serve`.
- `serve` loops on [`net/secure.rs:108`](../../crates/mickey/src/net/secure.rs#L108)
  `Link::recv`: read, decrypt, `protocol::decode`. A changed, replayed or
  reordered frame fails to decrypt and closes the link.
- [`input/held.rs:21`](../../crates/mickey/src/input/held.rs#L21) `HeldSet::set`
  marks key 30 as down: a 768-bit bitset, so if the link dies now, the
  target knows to release **A**.

## Hop 6: a virtual keyboard types it (`emit.rs`, `platform/uinput.rs`)

- [`target/mod.rs:218`](../../crates/mickey/src/target/mod.rs#L218): the `output`
  closure hands `Effect::Input` to `emitter.emit`.
- [`input/emit.rs:168`](../../crates/mickey/src/input/emit.rs#L168) (Linux): evdev
  codes pass straight through.
- [`platform/uinput.rs:82`](../../crates/mickey/src/platform/uinput.rs#L82) queues
  a 24-byte `input_event`;
  [`platform/uinput.rs:95`](../../crates/mickey/src/platform/uinput.rs#L95) adds
  `SYN_REPORT` and writes the batch in **one** system call. The kernel
  feeds "Mickey virtual input" to the compositor like a real keyboard, and
  the letter appears.

## The other direction: Linux drives Windows

| Step | Linux source | Windows target |
|---|---|---|
| Read | [`input/capture.rs:130`](../../crates/mickey/src/input/capture.rs#L130): one thread per `/dev/input/event*` device | |
| Translate | [`input/capture.rs:180`](../../crates/mickey/src/input/capture.rs#L180): auto-repeat (value 2) sent as a press; motion summed until `SYN_REPORT` ([`:212`](../../crates/mickey/src/input/capture.rs#L211)) | |
| Grab | [`platform/linux.rs:130`](../../crates/mickey/src/platform/linux.rs#L130): `EVIOCGRAB` | |
| Emit | | [`input/emit.rs:21`](../../crates/mickey/src/input/emit.rs#L21): evdev code back to a scan code, then [`platform/windows.rs:130`](../../crates/mickey/src/platform/windows.rs#L130) `SendInput` with `KEYEVENTF_SCANCODE` |

Everything between (focus, hotkeys, the link) is the same code on both.

## Try it

Run the link test that plays both ends in one process:

```sh
cargo test -p mickey events_cross_and_disconnect_releases_held_keys
```

Then read it at
[`net/transport.rs:314`](../../crates/mickey/src/net/transport.rs#L314): a key
goes down, the source disconnects without releasing it, and the target
still releases it. Set a breakpoint in `HeldSet::release_all` and watch
the code come out.

## Check yourself

1. Why do key codes cross as Linux evdev codes, even from Windows to
   Windows?
2. How does the Windows source stop its own desktop from seeing a key
   while the focus is remote?
3. Why does the focus wait in `WaitingForKeysUp` before grabbing?
4. What would happen to typing latency without `TCP_NODELAY`?
5. The source crashes with **A** held. Why does the laptop not type
   `aaaaaa` forever?

<details>
<summary>Answers</summary>

1. One vocabulary on the wire, and they are physical key positions: each
   target applies its own layout. Windows translates scan codes to evdev
   when capturing (`keymap::evdev_code`) and back when emitting
   (`keymap::scan_code`).
2. The low-level hook returns non-zero (`report` returns `swallowing()`),
   so Windows drops the event; Mickey's own injected events pass because
   of the `LLKHF_INJECTED` flag.
3. Grabbing while a key is down leaves that key stuck down on the local
   desktop, which never sees its release.
4. Nagle's algorithm could hold tiny frames back to batch them, adding up
   to tens of milliseconds ([`net/transport.rs:24`](../../crates/mickey/src/net/transport.rs#L24)).
5. `serve` tracks held keys in a `HeldSet` and, when the link ends cleanly
   or not, releases every one of them (`release_all` at the end of
   `serve`).

</details>
