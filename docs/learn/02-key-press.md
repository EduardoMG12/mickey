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

- [`hooks.rs:578`](../../crates/mickey/src/platform/hooks.rs#L578)
  `keyboard_proc`: reads the scan code and the extended flag.
- [`hooks.rs:584`](../../crates/mickey/src/platform/hooks.rs#L584):
  events with `LLKHF_INJECTED` pass untouched. That is how a PC that is
  both source and target never loops on its own `SendInput` (PRD 0008,
  *Discoveries*).
- [`hooks.rs:568`](../../crates/mickey/src/platform/hooks.rs#L568)
  `report`: puts the event on a channel and returns `swallowing()`. When
  the focus is remote, the hook returns `1` and **Windows drops the key**:
  that is the Windows version of a grab. The hook must be quick; Windows
  skips a hook that takes too long.

## Hop 2: scan code becomes an evdev code (`input.rs`, `keymap.rs`)

- [`input.rs:132`](../../crates/mickey/src/input.rs#L132) `start`: a
  second thread reads the hook's channel, so the hook itself does nothing
  slow.
- [`input.rs:180`](../../crates/mickey/src/input.rs#L180) `translate`
  calls [`keymap.rs:100`](../../crates/mickey/src/keymap.rs#L100)
  `evdev_code`. **A** becomes `30`, Linux's `KEY_A`.

Why Linux codes on both systems? One vocabulary on the wire, and they are
*physical positions*: the target's own layout decides the character
([0000-vision.md](../prds/0000-vision.md), *Architecture*). That is why
the README asks for the same layout on both PCs.

The result is an [`InputEvent`](../../crates/mickey/src/event.rs#L5):
`Key { code: 30, pressed: true }`. No OS types past this point.

## Hop 3: the main loop decides (`source.rs`, `focus.rs`, `hotkey.rs`)

- [`source.rs:48`](../../crates/mickey/src/source.rs#L48) `Wake`: one
  enum for everything that can wake the main loop: input, the tray,
  control commands, news from the target side.
  [`source.rs:70`](../../crates/mickey/src/source.rs#L70) turns an
  `InputEvent` into `Wake::Input`.
- [`source.rs:989`](../../crates/mickey/src/source.rs#L989): the loop
  blocks in `recv_timeout`. Idle, it wakes once a second, for the ping.
- [`source.rs:1017`](../../crates/mickey/src/source.rs#L1017): input goes
  to `switcher.handle`.
- [`focus.rs:86`](../../crates/mickey/src/focus.rs#L86) `Switcher::handle`
  is a three-state machine: `Local`, `WaitingForKeysUp(peer)`,
  `Remote(peer)` ([`focus.rs:23`](../../crates/mickey/src/focus.rs#L23)).
  In `Remote`, every event goes through `hotkey.feed`, which forwards it
  (or holds back the last key of a chord, in case it completes a hotkey;
  [`hotkey.rs:204`](../../crates/mickey/src/hotkey.rs#L204)), records it
  in a `HeldSet` and emits `Action::Send(peer, event)`.
- [`source.rs:952`](../../crates/mickey/src/source.rs#L952): `Action::Send`
  calls `link.send`. If it fails, the peer goes on the `lost` list and the
  focus comes home.

How did the focus become `Remote` in the first place? The hotkey moved it
to `WaitingForKeysUp`, and
[`source.rs:840`](../../crates/mickey/src/source.rs#L840) grabs only once
no key is down. Grabbing with a key held would leave it stuck down on the
desktop you are leaving.

## Hop 4: sealed and sent (`secure.rs`, `protocol.rs`, `transport.rs`)

- [`secure.rs:92`](../../crates/mickey/src/secure.rs#L92) `Link::send`:
  1. [`protocol.rs:117`](../../crates/mickey/src/protocol.rs#L117)
     `encode` writes `[type 2][code 0x001E][pressed 1]` into a fixed array
     on the stack;
  2. Noise's `write_message` encrypts it with ChaCha20-Poly1305 and adds a
     16-byte tag;
  3. [`secure.rs:248`](../../crates/mickey/src/secure.rs#L248)
     `write_noise` puts a 2-byte length in front and writes it in one
     go.
- No heap allocation on this path: fixed arrays, and `Message` is `Copy`
  ([architecture.md](../architecture.md), end of *One key press*).
- [`transport.rs:31`](../../crates/mickey/src/transport.rs#L31)
  `connect` set two things on this socket long ago: `TCP_NODELAY` (the
  kernel never holds a tiny frame back to batch it) and a 1 s write
  timeout (a stuck target cannot keep your keyboard).

## Hop 5: the laptop opens it (`main.rs`, `transport.rs`, `held.rs`)

- [`main.rs:463`](../../crates/mickey/src/main.rs#L463) `target`: accepts
  connections, one thread per source, at most 32 at once
  ([`main.rs:528`](../../crates/mickey/src/main.rs#L528)).
- [`main.rs:733`](../../crates/mickey/src/main.rs#L733) `serve_source`:
  handshake, trust check (chapter 3), then
  [`transport.rs:122`](../../crates/mickey/src/transport.rs#L122) `serve`.
- `serve` loops on [`secure.rs:110`](../../crates/mickey/src/secure.rs#L110)
  `Link::recv`: read, decrypt, `protocol::decode`. A changed, replayed or
  reordered frame fails to decrypt and closes the link.
- [`held.rs:21`](../../crates/mickey/src/held.rs#L21) `HeldSet::set`
  marks key 30 as down: a 768-bit bitset, so if the link dies now, the
  target knows to release **A**.

## Hop 6: a virtual keyboard types it (`emit.rs`, `platform/uinput.rs`)

- [`main.rs:800`](../../crates/mickey/src/main.rs#L800): the `output`
  closure hands `Effect::Input` to `emitter.emit`.
- [`emit.rs:149`](../../crates/mickey/src/emit.rs#L149) (Linux): evdev
  codes pass straight through.
- [`uinput.rs:82`](../../crates/mickey/src/platform/uinput.rs#L82) queues
  a 24-byte `input_event`;
  [`uinput.rs:95`](../../crates/mickey/src/platform/uinput.rs#L95) adds
  `SYN_REPORT` and writes the batch in **one** system call. The kernel
  feeds "Mickey virtual input" to the compositor like a real keyboard, and
  the letter appears.

## The other direction: Linux drives Windows

| Step | Linux source | Windows target |
|---|---|---|
| Read | [`capture.rs:131`](../../crates/mickey/src/capture.rs#L131): one thread per `/dev/input/event*` device | |
| Translate | [`capture.rs:181`](../../crates/mickey/src/capture.rs#L181): auto-repeat (value 2) sent as a press; motion summed until `SYN_REPORT` ([`:212`](../../crates/mickey/src/capture.rs#L212)) | |
| Grab | [`linux.rs:130`](../../crates/mickey/src/platform/linux.rs#L130): `EVIOCGRAB` | |
| Emit | | [`emit.rs:20`](../../crates/mickey/src/emit.rs#L20): evdev code back to a scan code, then [`windows.rs:130`](../../crates/mickey/src/platform/windows.rs#L130) `SendInput` with `KEYEVENTF_SCANCODE` |

Everything between (focus, hotkeys, the link) is the same code on both.

## Try it

Run the link test that plays both ends in one process:

```sh
cargo test -p mickey events_cross_and_disconnect_releases_held_keys
```

Then read it at
[`transport.rs:297`](../../crates/mickey/src/transport.rs#L297): a key
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
   to tens of milliseconds ([`transport.rs:24`](../../crates/mickey/src/transport.rs#L24)).
5. `serve` tracks held keys in a `HeldSet` and, when the link ends cleanly
   or not, releases every one of them (`release_all` at the end of
   `serve`).

</details>
