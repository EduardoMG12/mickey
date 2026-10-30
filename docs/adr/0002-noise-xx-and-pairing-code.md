# 0002 — Noise `XX` for every link, and a six-digit pairing code

- **Status:** Accepted
- **Date:** 2026-10-21 (Noise link and pairing, PRD 0002, PRs #42 and #43);
  2026-10-22 (six-digit code in a window, PRD 0010, PRs #123 and #124)

## Context

M0 sent key presses as plaintext TCP on a trusted LAN only. "A keyboard sent
over the network is a keylogger unless it is protected"
([PRD 0000](../prds/0000-vision.md#security)). Issues #31 ("anyone on the
LAN can type on the Windows target") and #32 had to be closed before
anything else, so the owner brought encryption forward from M1a.

The goals ([PRD 0002](../prds/0002-secure-link.md#goals)): on an untrusted
network, nobody can **read** the traffic, **inject** input, **impersonate**
a paired peer, or **replay or change** messages. Pairing must also work with
no terminal, since an installed Mickey runs hidden
([PRD 0010](../prds/0010-easy-pairing.md)).

## Decision

1. **Each peer has an identity**: a long-term X25519 key pair made on first
   run. Only the 32-byte private key is stored (`identity`, mode `0600` on
   Linux); the public key is derived from it on load.
2. **Every TCP link runs `Noise_XX_25519_ChaChaPoly_BLAKE2s`** through the
   `snow` crate, right after connecting and before any Mickey message.
   Three handshake messages (`e` / `e, ee, s, es` / `s, se`) leave each side
   with the other's public key, proven by the handshake.
3. **Every frame after it is sealed** with ChaCha20-Poly1305 (16-byte tag).
   Noise counts messages, so a changed, replayed, dropped or reordered frame
   fails to decrypt and the link closes.
4. **Pairing compares a six-digit code** on both screens: the first four
   bytes of the Noise **handshake hash** as a big-endian `u32`, modulo
   1 000 000 (`code_from` in `secure.rs`), shown in a `mickey-ui pair`
   window with *Pair* and *Refuse*. The terminal prompt is only a fallback.
5. **Trust is by public key, never by address** (details in
   [ADR 0007](0007-peers-identified-by-public-key.md)). The target answers
   `Hello` only once it trusts the source, and the source sends nothing
   before that.

`snow` is built without default features (only `use-curve25519`,
`use-chacha20poly1305`, `use-blake2`, `use-getrandom`): 26 crates, all
Rust, no `ring` (PRD 0002, *Ticket 1*).

## Alternatives considered

- **TLS (`rustls`).** Rejected: peers are identified by their own keys, so
  TLS certificates add nothing (vision, *Open Questions*, resolved).
  Self-signed certificates would only be a wrapper around the same key.
- **QUIC (`quinn`).** Rejected: it would also change the transport from TCP
  (vision, *Open Questions*); see [ADR 0003](0003-tcp-links-udp-audio.md).
- **Writing the cryptography by hand.** Rejected even in a learning project:
  "hand-written cryptography is the classic way to be insecure without
  noticing" (PRD 0002, *Constraints*). This is the one place where "use
  reviewed code" wins.
- **Comparing 32-character fingerprints** (`y/N` in a terminal, the first
  version of PRD 0002). Replaced: it needed a terminal and was tedious to
  compare. The fingerprint still exists (`mickey id`, `mickey forget`).
- **A code from the two public keys, sorted and hashed** (PRD 0010's first
  plan). Replaced by the handshake hash, which covers both static **and
  both ephemeral** keys, so it still exposes a machine in the middle (it
  runs two handshakes, so each PC sees a different hash), and comes from
  `snow` for free.

## Consequences

Good:

- Nothing on the LAN can read or inject input without a pairing that a
  person confirmed on both screens.
- The same handshake and identities protect the data link and hand out the
  audio keys ([ADR 0003](0003-tcp-links-udp-audio.md),
  [ADR 0008](0008-shared-audio.md)).
- Cost per frame is one ChaCha20-Poly1305 seal, microseconds (PRD 0002);
  debug builds optimize dependencies, or the stuck-target test took 11 s
  instead of 2.4 s.

Bad, and known limits:

- **The code changes on every connection**: fine, since it is only compared
  while both screens show it. `% 1_000_000` on 32 bits leaves a bias under
  0.03 % (PRD 0010, *Discoveries*).
- **Six digits is a short check, and only as good as the person reading
  it.** A machine in the middle would have to make both handshakes land on
  the same code, a one-in-a-million guess per attempt (**inferred**; the
  docs give no figure), and the owner must actually compare the codes.
- **Not covered:** someone logged in as the user (they can read the
  identity file, as with SSH keys) and denial of service (anyone on the LAN
  can open the port and waste time). The handshake must finish in 5 s and a
  port takes at most 32 connections at once ([protocol.md](../protocol.md)).
- **One extra dependency tree** (26 crates), checked by `cargo audit` in CI.

## Links

- [PRD 0002, secure link](../prds/0002-secure-link.md), including
  *Discoveries*, "What changed after PRD 0010".
- [PRD 0010, easy pairing](../prds/0010-easy-pairing.md), *Discoveries*.
- [protocol.md](../protocol.md#the-noise-handshake), [flows.md](../flows.md#pairing).
- Issues #31, #32; PRs #42, #43, #123, #124.
- Code: `crates/mickey/src/net/secure.rs` (`Link`, `code_from`),
  `crates/mickey/src/net/identity.rs`, `crates/mickey/src/net/trust.rs`,
  `crates/mickey-ui/src/pair.rs`.
