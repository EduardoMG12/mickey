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
- **A volume per PC**, on the listener: one slider (and mute) next to
  each PC in the settings' **PCs** tab, remembered.
- **Where the settings live** (the owner, 2026-10-24): spread over the
  tabs that already exist, not a tab of their own. **This PC**: "Listen
  here" and this PC's mode; **PCs**: each PC's volume. The tray gets
  "Listen here" too.
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
  so one lost packet costs 5 ms of sound, not a stall. Encrypted with a
  random per-stream key handed over the data link, each datagram carrying
  its counter as the nonce, so a lost or reordered datagram does not break
  the next (see *Discoveries*: the Noise session keys were the first plan). Uncompressed: 1.5 Mbit/s per PC is fine on a LAN; a
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

- **Audio uses its own key, not the Noise session keys.** The plan was
  snow's stateless transport over the peers' session keys. The code
  (`audio_wire.rs`) instead seals each datagram with ChaCha20-Poly1305
  under a random key the sender makes per stream and hands to the
  listener as a kind 22 payload over the already-encrypted data link,
  resent every 2 s (see protocol.md, *Audio datagrams*). The datagrams
  stay independent of the input link's Noise state, and a replay window
  drops copies.

- **Windows: loopback capture ignores the output's mute and volume**
  (#265, spike on the owner's Windows 10 22H2). A quiet 440 Hz tone
  captured through WASAPI loopback of the default output gave the same RMS
  with the output normal (0.0354), muted (0.0352) and at volume 0
  (0.0353). So *Send only* just mutes the default output
  (`IAudioEndpointVolume::SetMute`) and keeps capturing; no virtual device
  and no driver. The capture is also independent of that PC's volume
  slider, so loudness is set in the listener's mixer. Mickey must unmute
  on stop, on panic, and on the next start if it died while muted.
- **Linux: the null sink works as planned, on PipeWire through its
  PulseAudio protocol** (#265, Ubuntu 24.04 VM, PipeWire 1.0.5). With a
  stand-in speaker sink as the default and a tone playing, loading
  `module-null-sink sink_name=mickey` and making it the default **moved
  the playing stream at once**: the Mickey sink's monitor read RMS 0.21,
  the speakers 0.0. Setting the speakers back as default brought the tone
  back to them.
- **A null sink can outlive its unload.** After an earlier run, a `mickey`
  sink was still there although `pactl unload-module <id>` had returned 0;
  only unloading the module listed under that name removed it. Mickey
  therefore removes every module that made a `mickey` sink when it starts,
  and sets the real default back, whatever happened before (like the
  input grab: never leave the owner without sound).
- **Voicemeeter works as it is.** On the owner's Windows PC the default
  output is Voicemeeter Banana's virtual input ("VoiceMeeter Input"),
  which Voicemeeter mixes out to the real headphones and speakers. Loopback
  capture of it gets the apps' sound (RMS 0.0353, the same muted), and
  muting it silenced the headphones: a tone heard, muted, then heard again
  by the owner. (Voicemeeter drives the hardware its own way, so loopback
  of the physical outputs sees nothing: only an ear can check them.) The
  listener's sound plays into the default output too, so it follows the
  owner's Voicemeeter routing.
- **First two-PC run, and its delay** (2026-10-24, the owner's Arch
  laptop and Windows 10 PC over Wi-Fi). Both ways worked: the Windows
  PC's sound on the laptop's headphones ("nearly real time", the owner),
  and the laptop's on the Windows PC's, mixed with its own. Measured
  pieces of Windows → laptop: capture on Windows ~11 ms (half of a 22.8 ms
  play-and-capture round trip), Wi-Fi ~8 ms average one way (ping 16 ms
  average, 69 ms worst of 50), the jitter buffer 15 ms (once 20), playback
  on Linux ~30 ms (half of the VM's 65 ms round trip): about **65 ms** in
  all on Wi-Fi. The cable budget (p99 < 40 ms) was not measured: the
  laptop had no cable, and the owner judged 65 ms on Wi-Fi good enough
  (2026-10-27, closing #272). Linux playback (20 ms queued, plus
  PipeWire's quantum) is the first thing to trim if it ever needs to be
  faster.
- **Two bugs the run found, fixed before merging.** Each Mickey shares who
  listens when it starts, so a PC restarted with an old config.toml undid
  a newer choice: the shared settings carry `since` and only newer ones
  are taken. And asking PipeWire for `@DEFAULT_MONITOR@` right after
  switching the default still gave the speakers' monitor, so Linux
  recorded silence: Mickey records `mickey.monitor` by name.
- **On Linux, muting is not the way, unlike Windows.** A sink's monitor
  follows its mute and volume on PipeWire (tone at 0.21 RMS: muted 0.0,
  volume 0 0.0), so the null sink stays: Mickey cannot mute the speakers
  and still record what they would play.
- **Stopping Mickey must give the sound back.** The null sink outlives
  the process that made it default, and Mickey handled no signal so far
  (the input grab did not need it: the kernel releases it when the
  process dies). `systemctl --user stop mickey` or Ctrl+C would leave the
  owner without sound until the next start. The Linux backend therefore
  waits for SIGTERM, SIGINT and SIGHUP on a thread of its own and puts
  the real output back before exiting.
- **No command-line tools to rely on.** Ubuntu's desktop ships neither
  `pactl` nor `parec`; `libpulse` itself is on every desktop. The Linux
  backend talks to it directly. (A PipeWire VM with no sound card has only
  `auto_null`, which disappears once any other sink exists: tests need a
  stand-in speaker sink.)
- **Record with a small buffer.** `parec`'s default buffer held a second
  of audio back; `--latency-msec=20` was needed for timely data. The
  backend asks for small fragments (5 ms) for the same reason.
