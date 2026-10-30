# 5. The journey of a sound

> **The question:** a video plays on the Windows PC; your headphones are
> on the laptop. How does its sound reach your ears about 65 ms later on
> Wi-Fi, without crackling, while the Windows speakers stay silent? And
> why does it not ride the encrypted TCP link like everything else?

Read [0012-shared-audio.md](../prds/0012-shared-audio.md) *Goals* first
(five minutes): one PC **listens**, chosen by the owner; every other PC
sends its sound there, each at its own volume.

## Who sends where (`source/sound.rs`)

- *Who listens* is a shared setting (`[audio] listener`), sent to every
  peer over the data link as kind 21. It carries `since`, and a PC takes
  it only if it is newer than its own: a PC restarted with an old
  `config.toml` must not undo a newer choice (PRD 0012, *Discoveries*).
- [`source/sound.rs:41`](../../crates/mickey/src/source/sound.rs#L41)
  `audio_target`: send only if another PC listens, it is online, and this
  PC's mode is not `Here` (the modes: `Listener` = send only, `Both` =
  send and keep, `Here` = keep only).
- [`source/mod.rs:606`](../../crates/mickey/src/source/mod.rs#L606): when the
  target changes, the old `Sending` is dropped (which gives the speakers
  back) and a new one starts.

## Step 1: a key for this stream (`audio.rs`, `audio/wire.rs`)

[`audio/mod.rs:209`](../../crates/mickey/src/audio/mod.rs#L209) `Sending::start`
picks a random stream id and ChaCha20-Poly1305 key
([`audio/wire.rs:44`](../../crates/mickey/src/audio/wire.rs#L44)) and
hands them to the listener as kind 22 over the **data link**, which is
already encrypted and authenticated. It repeats every 2 s, in case the
data link was not open yet.

## Step 2: off the speakers (`speakers/`)

Before recording, *Send only* takes the sound off this PC's speakers:

- **Linux** ([`audio/speakers/linux.rs:144`](../../crates/mickey/src/audio/speakers/linux.rs#L144)
  `divert`): load a PulseAudio *null sink* named `mickey`, noting the
  real output in its description, and make it the default. Every app now
  plays into a silent sink. (Muting would not do: on PipeWire a sink's
  monitor follows its mute.)
- **Windows** ([`audio/speakers/windows.rs:18`](../../crates/mickey/src/audio/speakers/windows.rs#L18)):
  WASAPI loopback ignores the output's mute (the spike measured the same
  RMS muted and unmuted), so Mickey just mutes the default output. A
  marker file is written **first**, so a Mickey that dies right after
  muting still unmutes at the next start.

Both return a `Diverted` guard whose `Drop` gives the sound back.

## Step 3: record, seal, send (`audio.rs`, `audio/wire.rs`)

[`audio/mod.rs:240`](../../crates/mickey/src/audio/mod.rs#L240) `send`, in a
loop:

1. `Recorder::read` gives 5 ms: 240 frames of 48 kHz stereo 16-bit PCM,
   960 bytes. On Linux it records `mickey.monitor` **by name**
   ([`audio/sound/linux.rs:68`](../../crates/mickey/src/audio/sound/linux.rs#L68));
   asking for "the default monitor" right after switching still gave the
   speakers' one, and the two-PC test recorded silence.
2. All zeros? Skip it ([`audio/mod.rs:268`](../../crates/mickey/src/audio/mod.rs#L268)):
   silence is never sent.
3. [`audio/wire.rs:97`](../../crates/mickey/src/audio/wire.rs#L97) `seal`:
   `[version | stream id | counter]` in the clear but authenticated, then
   the PCM sealed with the **counter as the nonce**. 989 bytes, asserted to
   fit under any MTU ([`:29`](../../crates/mickey/src/audio/wire.rs#L29)).
4. `send_to` over UDP to the listener's port 4244.

**Why UDP, and why not Noise's own stream?** TCP retransmits a lost
packet and holds everything behind it; for sound, a late packet is a
useless packet. Noise's transport counts messages, so one lost datagram
would break every later one. Each datagram here opens on its own: a lost
one costs 5 ms, never a stall.

## Step 4: the listener checks (`audio.rs`, `audio/wire.rs`)

- [`audio/mod.rs:57`](../../crates/mickey/src/audio/mod.rs#L57) `listen`: one UDP
  socket, always ready.
- [`audio/mod.rs:68`](../../crates/mickey/src/audio/mod.rs#L68) `heard_key`: only
  called from a trusted data link (`serve_clipboard`, kind 22).
- [`audio/mod.rs:79`](../../crates/mickey/src/audio/mod.rs#L79) `receive` drops a
  datagram unless its stream id has a key **and** it comes from the
  address that sent the key.
- [`audio/wire.rs:138`](../../crates/mickey/src/audio/wire.rs#L138)
  `open`: checks the counter is fresh, decrypts, and only then marks it
  seen ([`fresh`](../../crates/mickey/src/audio/wire.rs#L158),
  [`mark`](../../crates/mickey/src/audio/wire.rs#L169)): a 64-bit mask of
  the last 64 counters. A replay, or anything more than 64 behind, is
  dropped. Forged counters cannot push real ones out, since only an
  authentic datagram moves the window.

## Step 5: the jitter buffer, then play (`jitter.rs`, `audio.rs`)

The network delivers datagrams early, late or out of order; the sound card
wants exactly one every 5 ms.

- [`audio/mod.rs:157`](../../crates/mickey/src/audio/mod.rs#L157) `play`: one
  thread per sending PC, its own playback stream, so the system mixer
  shows each PC with its own slider. It ends after 250 ms of nothing.
- [`audio/jitter.rs:53`](../../crates/mickey/src/audio/jitter.rs#L53) `push` puts a
  datagram in slot `counter % 64`.
- [`audio/jitter.rs:69`](../../crates/mickey/src/audio/jitter.rs#L69) `pull`: starts
  once 2 are in hand (10 ms). A missing one plays as 5 ms of silence and
  the target grows by one (up to 16, 80 ms); after 2000 calm turns (10 s)
  it gives one back by skipping ahead.
- The player's `write` blocks until the sound card has room: the card's
  clock paces the loop. Volume is applied per sample and re-read from
  `config.toml` every second.

**Concepts.** The slots are a **ring buffer** indexed by `counter % 64`
(LeetCode 622, *Design Circular Queue*): fixed memory, no shifting. The
replay mask is a **sliding window as a bitmask** (closest: LeetCode 219,
*Contains Duplicate II*, "seen within the last k?"): 64 bits instead of a
set of counters.

## Numbers

First two-PC run, Windows → laptop over Wi-Fi: capture ~11 ms, Wi-Fi ~8
ms, jitter buffer 15 ms, Linux playback ~30 ms, **about 65 ms** in all.
The cable budget (p99 under 40 ms) is still to be measured (PRD 0012,
*Discoveries*).

## Try it

```sh
cargo test -p mickey jitter
cargo test -p mickey a_replay_is_refused_and_late_ones_still_play
```

Read [`audio/jitter.rs:185`](../../crates/mickey/src/audio/jitter.rs#L185)
(a lost datagram) and [`:223`](../../crates/mickey/src/audio/jitter.rs#L223)
(a calm network gives the delay back): each is a little timeline you can
follow on paper.

## Check yourself

1. Why does the sending PC pick a fresh key instead of reusing the Noise
   session's?
2. How does the Linux sender stop the speakers without stopping the
   recording?
3. One datagram is lost. What does the listener hear, and what changes
   for the next ten seconds?
4. Someone on the LAN records datagrams and replays them a minute later.
   What happens?
5. Mickey is killed with `systemctl --user stop mickey` while sending.
   Who puts the speakers back?

<details>
<summary>Answers</summary>

1. Noise counts messages, so with its transport one lost UDP datagram
   would break all the next ones; a per-stream key with the counter as
   the explicit nonce lets every datagram open on its own. The key still
   travels inside the authenticated data link.
2. It moves every app to a silent null sink named `mickey` and records
   that sink's monitor; the real output just gets nothing.
3. 5 ms of silence; the target grows by one datagram (5 ms more delay),
   and only after 10 s with nothing missing does it shrink again.
4. Dropped: the counters are older than the newest by more than 64, or
   already marked in the mask. (A stream's key is also new every time
   sending starts.)
5. The signal thread ([chapter 6](06-safety-nets.md)) runs
   `speakers::clean_up` before exiting; and if even that was missed, the
   next start runs it first.

</details>
