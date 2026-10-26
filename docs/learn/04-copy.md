# 4. The journey of a copy

> **The question:** you copy a screenshot on the Windows PC, press the
> hotkey, and paste on the laptop. *When* did the image cross the
> network, on *which* connection, and why not the moment you copied it?

## The rule: the clipboard follows the keyboard

Nothing crosses on every copy. When the focus moves from PC X to PC Y,
what is copied on X is put on Y, once
([0011-shared-clipboard.md](../prds/0011-shared-clipboard.md), *When the
clipboard travels*). A copy you never paste elsewhere never leaves its
PC.

## Hop 1: the focus moved (`source.rs`)

[`source.rs:856`](../../crates/mickey/src/source.rs#L856): when the
focus changes, the main loop calls `mover.moved(was, focus)` (`None`
means "this PC"), and right after tells both peers with
`Message::Focus` ([`:861`](../../crates/mickey/src/source.rs#L861)).
That is all the main loop does: one message on a channel. It never waits
for a clipboard.

## Hop 2: the Mover does the work (`clip_sync.rs`)

- [`clip_sync.rs:157`](../../crates/mickey/src/clip_sync.rs#L157)
  `Mover::start`: one thread that owns every outgoing **data link**. It
  also carries the group map, hotkeys, switcher cards and audio keys.
- [`clip_sync.rs:272`](../../crates/mickey/src/clip_sync.rs#L272)
  `carry(from, to)`:
  - from this PC: `keeper.read()`;
  - from a peer: send `Ask` (kind 0) and wait for its answer;
  - to this PC: `keeper.write(content)`;
  - to a peer: send the content.
- It logs only the **kind and size**, and only past 64 KB: what you copy
  may be a password.

So in the question's case (Windows source, laptop target), the Windows
Mover reads its own clipboard and sends it to the laptop. Coming back,
it **asks** the laptop for whatever you copied there.

## Hop 3: why a second connection (`secure.rs`)

The data link is TCP port 4243, the input port + 1, with the same Noise
handshake and keys. Why not reuse the input link? Head-of-line blocking:
on one TCP stream, a 5 MB image would sit in front of every key press
behind it.

[`secure.rs:126`](../../crates/mickey/src/secure.rs#L126) `send_bulk`:
a Noise message holds at most 65 535 bytes, so a payload goes as its
length, then pieces of 65 519 bytes (+16 of tag each). At most 64 MiB
(`MAX_BULK`); [`recv_bulk`](../../crates/mickey/src/secure.rs#L149)
refuses a bigger announced length and pieces that do not add up.

## Hop 4: the other PC answers (`main.rs`, `clip_sync.rs`)

- [`main.rs:563`](../../crates/mickey/src/main.rs#L563)
  `serve_clipboard`: listens on 4243;
  [`transport.rs:84`](../../crates/mickey/src/transport.rs#L84)
  `accept_data` refuses any key not already trusted. A data link never
  asks to pair.
- [`clip_sync.rs:17`](../../crates/mickey/src/clip_sync.rs#L17)
  `answer`: payloads of kind 16 and up are not the clipboard's (group map,
  hotkeys, switcher, audio); they go to `other`. The rest is decoded by
  [`clip_wire.rs:87`](../../crates/mickey/src/clip_wire.rs#L87).
- **`Ask` is answered only once, right after the keyboard left**:
  [`clip_sync.rs:66`](../../crates/mickey/src/clip_sync.rs#L66)
  `ClipGrant` allows one read within 5 s of the focus leaving this PC.
  Otherwise the answer is `Nothing`. A paired PC therefore cannot poll
  your clipboard for passwords (#185).

## What crosses (`clip_wire.rs`, `clipboard.rs`, `received.rs`)

| Kind | Body | Guard |
|---|---|---|
| 1 Text | UTF-8 | |
| 2 Image | PNG ([`clip_wire.rs:41`](../../crates/mickey/src/clip_wire.rs#L41)) | at most 256 MiB unpacked, checked before allocating |
| 4 Files | names and bytes | 60 MiB in all; unsafe names refused |

- [`clipboard.rs:88`](../../crates/mickey/src/clipboard.rs#L88) `Keeper`:
  one thread owns the system clipboard (through `arboard`). On Wayland and
  X11, copied data lives only while its owner does, so the owner never
  goes away.
- [`clipboard.rs:25`](../../crates/mickey/src/clipboard.rs#L25): even the
  `Debug` print of `Content` shows only `text, 12 bytes`.
- [`received.rs:66`](../../crates/mickey/src/received.rs#L66) `save`:
  received files land in a fresh folder under the cache, the last five
  kept. A name from the network that could leave the folder (`..`, a
  slash, `CON`) is refused, never "cleaned up"
  ([`:18`](../../crates/mickey/src/received.rs#L18)). On Windows they get
  the Mark of the Web, like a download
  ([`:42`](../../crates/mickey/src/received.rs#L42)).

## The history (`history.rs`, `clip_watch.rs`)

- [`clip_watch.rs:16`](../../crates/mickey/src/clip_watch.rs#L16): notices
  every copy without reading the clipboard over and over: Wayland
  selection events, Windows' clipboard counter, or else the text compared
  once a second.
- [`history.rs:124`](../../crates/mickey/src/history.rs#L124) `record`:
  on each change, read and add.
- [`history.rs:88`](../../crates/mickey/src/history.rs#L88) `add`: a
  `VecDeque`, newest first. Copying something already there moves it to
  the top; past 50 entries or 16 MiB the oldest go. Images are kept as
  PNG. **Memory only, never disk.**

**Concept: a bounded most-recent-first list.** This is the eviction half
of an LRU cache (LeetCode 146, *LRU Cache*): touching an entry moves it
to the front, and the back falls off when full. Here a linear search is
fine (50 entries); the hash map that makes LeetCode's version O(1) would
be complexity with no gain.

## Try it

```sh
mickey clipboard
cargo test -p mickey the_clipboard_may_be_read_once_right_after_the_keyboard_leaves
```

`mickey clipboard` says what *kind* of thing is copied now, never what
it is. The test ([`clip_sync.rs:328`](../../crates/mickey/src/clip_sync.rs#L328))
is the `ClipGrant` rule in a few lines: read it before the answers below.

## Check yourself

1. Why does the clipboard travel on its own TCP connection?
2. The focus goes from the laptop (source) to PC B and then to PC C. Which
   PCs does B's clipboard pass through?
3. Why can a paired PC not read your clipboard whenever it likes?
4. Why does one thread (`Keeper`) own the system clipboard?
5. What happens to the history when Mickey stops?

<details>
<summary>Answers</summary>

1. On the input link, a big payload would delay every key press queued
   behind it (head-of-line blocking). The data link also has a 10 s write
   timeout instead of 1 s.
2. Through the laptop: going from B to C passes through "this PC" (going
   home first), so B's clipboard comes to the laptop, then goes on to C
   ([protocol.md](../protocol.md), end).
3. `ClipGrant` allows one `Ask` within 5 s after that source's keyboard
   left this PC; any other `Ask` gets `Nothing`.
4. On Wayland and X11 the data you copied is served by its owner on
   paste; if the owner went away, so would the copy.
5. It is gone: it lives in memory only, by design.

</details>
