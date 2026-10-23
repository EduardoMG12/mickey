# 0002 — Secure link: encryption and pairing

> Status: **draft** · Depends on: [0001](0001-m0-hotkey-switch.md) · Vision: [0000](0000-vision.md)
> · Fixes issues #31 and #32

## Goals

The first part of M1a, brought forward because the owner wants Mickey safe
before anything else. After this milestone, on a network you do not trust:

- **Nobody can read** what crosses the link (typed text, passwords).
- **Nobody can inject** input: a peer that is not paired can connect, but its
  messages are never emitted.
- **Nobody can impersonate** a paired peer, replay old messages or change
  them in flight.

Done means: `mickey target` and `mickey source` pair once, with the owner
comparing a fingerprint on both screens. After that they connect on their
own, and a third machine on the LAN can neither type on the target nor read
the traffic.

## Approach

Each peer has an **identity**: a long-term X25519 key pair, created on first
run. The private key never leaves the machine and is never printed. Peers
talk over the **Noise protocol**, pattern `XX`
(`Noise_XX_25519_ChaChaPoly_BLAKE2s`):

1. The handshake gives both sides a shared secret and **each other's public
   key**, which is authenticated: whoever completes it holds the matching
   private key.
2. Each side looks the other's public key up in its **trusted peers**.
   Known: carry on. Unknown: **pairing** (below). Rejected: close.
3. Every frame after the handshake is encrypted and authenticated
   (ChaCha20-Poly1305). Noise counts messages, so a replayed, dropped or
   reordered frame fails to decrypt and the connection closes.

**We do not write cryptography by hand.** The handshake and the cipher come
from the `snow` crate (a widely used, pure-Rust Noise implementation, also
used by projects like libp2p). This is the one place where "write it
ourselves to learn" loses to "use reviewed code".

### Pairing (trust on first use, like SSH)

*(Replaced by a six-digit code in a window, PRD 0010; see Discoveries.)*

When the other side's key is unknown, both screens show:

```text
mickey: new peer at 192.0.2.10
mickey:   this PC:   K7QM-2XFD-9PLA-W3TE-R8BN-5CJY-HU4G-6ZSV
mickey:   other PC:  T2WE-8MNA-...
mickey: pair with it? Check that "other PC" here matches "this PC" there. [y/N]
```

Only after **both** sides answer `y` is the key saved and input emitted.
Until then the target emits nothing and the source sends nothing, so typing
never goes to an unconfirmed machine.

The fingerprint is the first 160 bits of the BLAKE2s hash of the public key,
shown as 32 base32 characters in groups of four. A man in the middle would
need a key whose fingerprint matches the real one, which 160 bits makes
impossible to brute-force.

## Requirements

### Identity
- [ ] On first run, generate a key pair and store it in
      `$XDG_DATA_HOME/mickey/identity` (Linux, created with mode `0600`) or
      `%APPDATA%\mickey\identity` (Windows, inside the user's profile).
- [ ] Refuse to start if the Linux file is readable by others, saying how to
      fix it.
- [ ] `mickey id` prints this peer's fingerprint. The private key is never
      printed, logged or sent.

### Trusted peers
- [ ] Stored next to the identity in `trusted_peers`: one line per peer,
      fingerprint plus the address it was paired from (informational).
      *(Stored as the full key in hex; see Discoveries.)*
- [ ] A peer is trusted by its **key**, never by its IP address.
- [ ] `mickey forget <fingerprint>` removes a peer.

### Link
- [ ] Noise `XX` handshake right after TCP connect, before any input frame.
      Our `Hello` with the protocol version travels inside the encrypted
      channel; the version becomes 2.
- [ ] Every M0 message is sent as one Noise transport message, framed by a
      2-byte length (Noise messages can be larger than our old 1-byte
      frames).
- [ ] A frame that fails to decrypt closes the connection, and the target
      releases everything held (as in M0).
- [ ] Handshake and decrypt errors are logged without key material.
- [ ] The "traffic is not encrypted" warning is removed.

### Tests
- [ ] Localhost: a paired source and target exchange input; an unpaired peer
      gets nothing emitted; a tampered byte closes the connection; a
      replayed frame closes the connection.
- [ ] A packet capture of a real session shows no plaintext key frames.
- [ ] Manual, in the PR: pairing between the owner's laptop and Windows PC,
      reconnect without prompts, a third command-line client refused.

## Non-Goals

- Screen edges, layout, Linux targets: the rest of M1a.
- Pairing from a GUI (M1b reuses the same trusted peers file).
- Protection against someone with access to your user account: they can read
  the identity file, as with SSH keys.
- Denial of service: anyone on the LAN can still connect and waste the
  target's time. They cannot inject or read.

## Constraints

- **New dependency `snow`** (and, through it, the RustCrypto crates for
  X25519, ChaCha20-Poly1305 and BLAKE2s, plus `getrandom` for keys). This
  is justified because hand-written cryptography is the classic way to be
  insecure without noticing. Only the `XX` pattern with those three
  primitives is used.
- No `unsafe` is added; the crate's own `unsafe`, if any, is reviewed code.
- Latency: one ChaCha20-Poly1305 seal per frame costs microseconds; step 8's
  measurement is repeated to show it.

## Concepts in this milestone

- **Authenticated encryption (AEAD)**: one operation that both hides the data
  and proves nobody changed it. No LeetCode equivalent.
- **Diffie-Hellman key agreement**: two parties agree on a secret over a
  public channel. Closest LeetCode: #50 *Pow(x, n)*, which uses the fast
  exponentiation behind classic Diffie-Hellman.
- **Trust on first use and fingerprints**: why comparing a short hash on two
  screens defeats a man in the middle.

## Open Questions

- Pairing prompt on the Windows target: the target runs in a terminal today,
  so `y/N` is typed there. Fine until the GUI (M1b).

## Discoveries

### Ticket 1 — identity (2026-10-21)

- **Only the private key is stored** (32 bytes). The public key is derived
  from it on load with snow's X25519, so the two can never disagree.
- **`snow` without default features.** Its `std` feature also turns on
  `ring` (C and assembly) and `sha2`, which we do not use. Enabling only
  `use-curve25519`, `use-chacha20poly1305`, `use-blake2` and
  `use-getrandom` leaves 26 crates, all Rust. `cargo audit` reports no
  advisories for them.
- **Windows permissions come from the profile.** `%APPDATA%\mickey\identity`
  inherits SYSTEM, Administrators and the user, nobody else, so no ACL code
  is needed. On Linux the directory is `0700` and the file `0600`, and a
  file readable by others is refused.
- **Checked:** `mickey id` prints the same fingerprint on every run on the
  owner's laptop and Windows PC.

### Ticket 2 — encrypted link (2026-10-21)

- **Checked between the owner's machines:** the source's handshake reported
  the target fingerprint `53VF-…-UZG5`, the same that `mickey id` prints on
  the Windows PC, and focus switched over the encrypted link.
- **Debug builds optimize dependencies** (`[profile.dev.package."*"]`):
  unoptimized ChaCha20 made the stuck-target test take 11 s instead of
  2.4 s. Mickey's own code stays unoptimized for the debugger.
- **The startup warning changed** from "not encrypted" to "not paired yet":
  the traffic is encrypted now, but until #37 any machine can still connect.

### Ticket 3 — pairing (2026-10-21)

- **The target answers `Hello` only once it trusts the source**, and the
  source waits for it before capturing anything. So "both sides confirmed"
  is a real step in the protocol, not two prompts that happen to line up.
- **Checked between the owner's machines** with throw-away identities
  (`XDG_DATA_HOME` / `APPDATA` pointed at a temp directory), the answers fed
  on stdin: first pairing (fingerprints matched crosswise on both screens),
  reconnect without prompts, an unknown source refused by the target, and a
  fake target with another key questioned again by the source, which then
  sent nothing.
- **Not done:** a packet capture of a real session (needs `sudo tcpdump`);
  the localhost tests already show every frame is sealed.
- **No answer means no.** A prompt with no terminal (stdin closed) reads
  nothing and refuses.

### What changed after PRD 0010 (2026-10-22)

- **`trusted_peers` holds the full public key, not the fingerprint.**
  Each line is `<public key, 64 hex digits> <note>`, the note being
  informational, e.g. the address the peer was paired from
  (`crates/mickey/src/trust.rs`). A fingerprint is a hash and cannot be
  turned back into the key, and the code needs the key itself: it is
  compared with the key the Noise handshake proved, and discovery
  (PRD 0010) announces and matches PCs by it. `mickey forget` still takes
  the fingerprint that `mickey id` prints and hashes each stored key to
  find it.
- **Pairing is no longer `y/N` on two 32-character fingerprints.** Both
  PCs show the same six-digit code, derived from the handshake hash, in a
  `mickey-ui pair` window with *Pair* and *Refuse* (PRD 0010). The
  terminal prompt ("type y to pair") is only the fallback when no window
  can open. The fingerprint is still printed next to the new PC's
  address in the log. This also answers the open question about the
  prompt on the Windows target: it is a window now.
