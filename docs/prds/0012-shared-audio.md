# 0012 — Shared audio

> Status: **draft** · Depends on: [0002](0002-secure-link.md), [0010](0010-easy-pairing.md) · Vision: [0000](0000-vision.md) (was *Someday*)

## Goals

Hear every PC in one pair of headphones: the sound of each paired PC plays
on the PC where the headphones are, each at its own volume, with no delay
you can notice. Decided with the owner on 2026-10-24:

- **One PC listens, chosen by the owner** ("Listen here", in the tray or
  the settings). It stays the listener until the owner picks another; it
  does not follow the keyboard, since the headphones are plugged into one
  PC.
- **Every other PC sends its sound there.** A newly paired PC sends too.
  Per PC, in the settings, the owner can pick:
  - **Send only** (the default): its own speakers go silent while it
    sends, and come back as they were when it stops;
  - **Send and keep**: it plays on its own speakers as well;
  - **Keep only**: it does not send.
- **A volume per PC**, on the listener: a mixer window, opened from the
  tray, with one slider (and mute) for each PC that sends, remembered.
- **Delay is automatic**: as low as the network allows (about 20 ms on a
  cable, 40 ms on Wi-Fi), growing a little only when the network jitters,
  so sound never crackles; video and sound stay in sync to the eye.
- **Linux (PipeWire or PulseAudio) and Windows.** PipeWire also speaks
  PulseAudio's protocol, so one backend covers both on Linux.

No listener chosen, or the listener offline: every PC plays on its own
speakers, as without Mickey.

Done means: the owner marks the laptop "Listen here", plays a video on the
Windows PC and music on the Arch laptop, hears both in the headphones on
the laptop, turns the Windows PC down in the mixer, and the Windows PC's
speakers stay silent the whole time.

## Approach

- **Capture what a PC plays.** Linux: Mickey adds a virtual output (a
  PulseAudio *null sink*, which PipeWire also provides), makes it the
  default while sending, and records its monitor; the real speakers get
  nothing unless *Send and keep*. Windows: WASAPI loopback capture of the
  default output; how to silence the speakers while still capturing is the
  first spike (below).
- **Send it as small encrypted UDP datagrams**, 5 ms of 48 kHz stereo
  16-bit PCM each (960 bytes, under any MTU): no head-of-line blocking,
  so one lost packet costs 5 ms of sound, not a stall. Encrypted with the
  peers' Noise session keys, each datagram carrying its counter as the
  nonce (snow's stateless transport), so a lost or reordered datagram does
  not break the next. Uncompressed: 1.5 Mbit/s per PC is fine on a LAN; a
  codec (Opus) only if Wi-Fi shows it is needed.
- **Play it with a jitter buffer** per sending PC on the listener: it
  starts at 2 packets (10 ms), grows when packets arrive late and shrinks
  back when the network is calm; a missing packet is played as silence.
  Each sending PC is its own playback stream, so the system mixes them and
  each has its own volume.
- **Who listens** travels like the group map: every PC learns which PC
  listens, and starts or stops sending.

## Requirements

- Audio only between paired PCs, always encrypted; an unpaired PC's
  datagrams are dropped before decoding.
- Delay, from a PC playing a sound to the listener's output, measured and
  reported per network: budget **p99 < 40 ms on a cable**, measured and
  reported on Wi-Fi.
- No crackle on a steady network; on a lost packet, a 5 ms gap, never a
  pause that grows.
- A sending PC whose listener goes away restores its own speakers at once.
- Idle (nothing playing): no datagrams sent.
- The vision's budgets still hold: input latency is untouched by audio
  (separate socket and thread), memory stays under the budget per peer.

## Non-Goals

- Microphone and camera (each needs a virtual device; on Windows a driver).
- Audio over the internet.
- Surround (stereo only), and per-app routing.

## Risks, to spike first

1. **Windows: silence the speakers while capturing them.** Loopback
   capture records the default output's mix; whether muting that output
   also silences the capture decides between muting it and switching the
   default to another device.
2. **Linux: the null-sink switch** must restore the real default output
   when Mickey stops, crashes or the listener goes away (like the input
   grab: never leave the owner without sound).

## Discoveries

None yet.
