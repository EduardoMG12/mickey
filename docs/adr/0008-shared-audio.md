# 0008 — Shared audio: one listener chosen by hand, per-OS silencing, an adaptive jitter buffer

- **Status:** Accepted
- **Date:** 2026-10-24 (decided with the owner, PRD 0012; spike #265;
  PRs #273 to #283)

## Context

The owner wants to hear every PC in one pair of headphones, each PC at its
own volume, "with no delay you can notice"
([PRD 0012](../prds/0012-shared-audio.md#goals)). Targets: about 20 ms on a
cable and 40 ms on Wi-Fi; budget **p99 < 40 ms on a cable**; no crackle on a
steady network; Linux (PipeWire or PulseAudio) and Windows. Two risks were
spiked first (#265): can Windows silence its speakers while still capturing
them, and can Linux always give the real output back?

## Decision

- **One listener, chosen by hand** ("Listen here" in the tray or settings),
  stored as the listener's **public key** in `[audio] listener`, shared with
  every PC over the data link (payload kind 21). It does **not** follow the
  keyboard. Shared settings carry `since`, and only newer ones are taken.
- **Per PC, a mode**: *Send only* (default, its speakers go silent), *Send
  and keep*, or *Keep only*. Per PC on the listener, a volume and a mute.
- **Windows silences by muting the default output.** WASAPI loopback
  ignores the output's mute and volume: the same 440 Hz tone gave RMS
  0.0354 normal, 0.0352 muted, 0.0353 at volume 0. A marker file
  `muted-by-mickey` is written *before* muting, so only a mute Mickey made
  is undone, on stop, on drop, and at the next start after a crash.
- **Linux moves the sound to a null sink.** Mickey loads a PulseAudio
  `module-null-sink` named `mickey` (PipeWire speaks the same protocol),
  makes it the default and records `mickey.monitor` by name. A thread waits
  for SIGTERM, SIGINT and SIGHUP to put the real output back; at start
  Mickey removes any leftover `mickey` sink. It talks to `libpulse` directly.
- **5 ms datagrams over UDP** (48 kHz stereo 16-bit, 960 bytes),
  uncompressed, sealed per datagram ([ADR 0003](0003-tcp-links-udp-audio.md)).
  The key is a random per-stream key the sender hands to the listener over
  the data link (kind 22), resent every 2 s. (PRD 0012's *Approach* still
  says "the peers' Noise session keys"; the code, `audio/wire.rs`, and
  protocol.md use the stream key.)
- **An adaptive jitter buffer per sending PC** (`jitter.rs`): a ring of 64
  slots (320 ms); it starts holding 2 datagrams (10 ms), grows by one each
  time one was missing when its turn came, up to 16 (80 ms), and gives one
  back after 2000 calm turns (10 s). A missing datagram plays as silence.
  Each sending PC is its own playback stream, so the OS mixes them.

## Alternatives considered

- **The listener follows the keyboard.** Rejected: "the headphones are
  plugged into one PC" (PRD 0012, *Goals*).
- **On Windows, switch the default output to another device**, or a virtual
  device or driver. Not needed once the spike showed loopback ignores mute.
- **On Linux, mute like Windows.** Rejected by measurement: a sink's monitor
  follows its mute and volume on PipeWire (0.21 RMS became 0.0).
- **`pactl` and `parec` command-line tools.** Rejected: Ubuntu's desktop
  ships neither; `libpulse` is on every desktop.
- **`@DEFAULT_MONITOR@`.** Rejected: right after switching the default it
  still named the speakers' monitor, so Mickey recorded silence.
- **A fixed buffer.** **Inferred** from `jitter.rs`: a large fixed buffer
  adds delay all the time, a small one crackles on a jittery network; 10 s
  of calm before shrinking avoids "a crackle every few seconds".
- **Opus.** Deferred until Wi-Fi shows it is needed (1.5 Mbit/s per PC).

## Consequences

Good:

- No driver or virtual device on Windows; the owner's Voicemeeter setup
  works as it is.
- The owner is never left without sound: Windows unmutes even after a
  crash; Linux restores on signals and cleans up at start.
- First two-PC run (Wi-Fi): about **65 ms** Windows → laptop (capture ~11
  ms, Wi-Fi ~8 ms, jitter buffer 15 ms, Linux playback ~30 ms).

Bad, and known limits:

- **The cable budget is not measured yet** (#272); 65 ms on Wi-Fi is above
  the 40 ms aim, and Linux playback is the first thing to trim.
- **Loudness is set on the listener**, not with the sending PC's slider
  (loopback ignores it).
- **Stereo only**; no microphone or camera (each needs a virtual device, on
  Windows a driver); LAN only.
- **A `kill -9` on Linux** skips the signal handler; the real output comes
  back only at the next start (**inferred** from the code: cleanup runs on
  signals and at start).
- **Linux needs `libpulse`** at build and run time.

## Links

- [PRD 0012, shared audio](../prds/0012-shared-audio.md), *Discoveries*.
- [protocol.md](../protocol.md#audio-datagrams), *Audio datagrams*;
  [architecture.md](../architecture.md#threads) (audio threads).
- Issues #265 (spike), #272; PRs #263, #273-#283.
- Code: `crates/mickey/src/audio/mod.rs`, `audio/wire.rs`, `jitter.rs`,
  `speakers/linux.rs`, `speakers/windows.rs`, `sound/`, `platform/output.rs`,
  `platform/signals.rs`; `crates/mickey-config/src/lib.rs` (`Audio`).
