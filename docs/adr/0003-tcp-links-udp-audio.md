# 0003 — TCP for input and data, UDP sealed datagrams for audio

- **Status:** Accepted
- **Date:** 2026-10-21 (TCP input link, M0); 2026-10-22 (separate data link,
  PR #136); 2026-10-24 (UDP audio, PRD 0012, PR #273)

## Context

Three kinds of traffic cross between two peers, with opposite needs:

| Traffic | Size and rate | What hurts most |
|---|---|---|
| Input (keys, motion, pings) | at most 146 bytes sealed, up to 1000 a second for a mouse | a **lost key release** (a stuck key), and latency |
| Bulk (clipboard, group map, switcher cards) | up to 64 MiB per payload | delaying the input behind it |
| Audio | 5 ms of 48 kHz stereo PCM per piece, 960 bytes, 200 a second | a **stall**: a late piece must cost 5 ms, not hold back the rest |

The vision listed the transport as an open question: "TCP for everything
(simple, ordered, no lost key releases) vs UDP for motion + TCP for keys
(lower worst-case latency when packets drop). M0 uses TCP; revisit in M1
with numbers" ([PRD 0000](../prds/0000-vision.md#open-questions)).

## Decision

Four channels ([protocol.md](../protocol.md#four-channels)):

- **Input link: TCP**, port 4242 by default, with `TCP_NODELAY` (the kernel
  never holds a key press back to batch it), a 2 s connect timeout and a
  1 s write timeout. Each Noise message has a 2-byte length in front;
  longer than 256 bytes is refused.
- **Data link: a second TCP connection**, the input port + 1 (4243), same
  Noise handshake and keys, opened in the background once the input link is
  up. Payloads up to 64 MiB, split into Noise messages of at most 65 535
  bytes. Writes may take 10 s.
- **Audio: UDP**, the input port + 2 (4244). Each datagram is 989 bytes: a
  clear but authenticated header (version, stream id, counter) and the PCM
  sealed with ChaCha20-Poly1305, the counter as the nonce. Each datagram
  opens on its own.
- **Discovery: UDP broadcast** on 4243 every 3 s, never trusted (PRD 0010).

## Alternatives considered

- **UDP for motion, TCP for keys.** Not adopted. The vision asked for
  numbers first; the measured input latency over TCP (p50 2.1 ms, p99 22.6
  ms one way on Wi-Fi, PRD 0001 *Step 8*) did not reopen it. The question
  is still listed as open in the vision; no document closes it with a
  measurement, so "TCP stays" is the state of the code, not a recorded
  verdict. TCP's ordering means a key release can never be lost.
- **QUIC** (streams without head-of-line blocking): considered only as part
  of the encryption choice, and rejected there because it would change the
  transport too ([ADR 0002](0002-noise-xx-and-pairing-code.md)).
- **Bulk data on the input link.** Rejected: "a 5 MB image on the same TCP
  stream would hold every key press behind it"
  ([PRD 0011](../prds/0011-shared-clipboard.md)).
- **Audio over the data link (TCP).** Rejected: one lost segment stalls
  everything after it until it is resent; with UDP "one lost packet costs
  5 ms of sound, not a stall" ([PRD 0012](../prds/0012-shared-audio.md#approach)).
- **Audio sealed with the Noise session as a stream.** Not possible: Noise
  transport messages must arrive in order, and a lost datagram would break
  every one after it. Each datagram carries its own counter as the nonce
  instead.
- **A codec (Opus).** Deferred: uncompressed is 1.5 Mbit/s per PC, fine on a
  LAN; "a codec only if Wi-Fi shows it is needed" (PRD 0012).

## Consequences

Good:

- A press and its release always arrive, in order; a target that stops
  reading fails the send within 1 s and the focus comes home.
- A screenshot on the clipboard never delays a key press.
- A lost audio datagram is a 5 ms gap; a replayed or too-old one (more than
  64 behind the newest) is dropped; nothing is sent while a PC is silent.
- Input latency is untouched by audio: separate socket and thread.

Bad, and known limits:

- **Head-of-line blocking on the input link**: on a lossy network a lost
  segment delays every event after it until TCP resends it. This is the
  price of never losing a release.
- **Up to four TCP connections per pair of PCs** (an input and a data link
  each way), plus UDP sockets.
- **Four ports to open** on a firewall (TCP 4242-4243, UDP 4243-4244;
  `packaging/linux/install.sh`). TCP 4243 and UDP 4243 do not collide.
- **The data link needs its own check**: the source keeps it only if it
  answers with the key of the input link just opened (PR #246), and the
  target never pairs on it.

## Links

- [protocol.md](../protocol.md): every channel, byte by byte.
- [PRD 0001](../prds/0001-m0-hotkey-switch.md) (TCP, `TCP_NODELAY`,
  framing), [PRD 0011](../prds/0011-shared-clipboard.md) (data link),
  [PRD 0012](../prds/0012-shared-audio.md) (audio).
- PRs #136 (data link), #246 (data link key check), #273 (audio wire).
- Code: `crates/mickey/src/transport.rs`, `secure.rs` (`send_bulk`),
  `clip_sync.rs`, `audio_wire.rs`, `audio.rs`, `discovery.rs`.
