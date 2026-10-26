# 0007 — Peers identified by public key; group introductions asked once

- **Status:** Accepted
- **Date:** 2026-10-21 (trust by key, PRD 0002); 2026-10-22 (full keys in
  `trusted_peers` and discovery, PRD 0010); 2026-10-23 and 24 (keys pinned in
  `config.toml` and introductions asked once, #184, PRs #201 and #260)

## Context

A PC's address is weak evidence of who it is: DHCP changes it, a PC has
one per network adapter, and anyone on the LAN can take one over. After
the secure link ([ADR 0002](0002-noise-xx-and-pairing-code.md)) the only
thing a handshake proves is the other side's **public key**.

Two later features leaned on that:

- **Discovery** (PRD 0010) lists PCs heard on the LAN and must match them
  against the PCs already known.
- **The group** (PRD 0005, 0010): every PC sends its *group map* (each PC it
  knows, with key, name, address and position), so one pairing reaches the
  whole group. The security review (#172) found this too generous (#184,
  severity Medium): `join` trusted every key in a trusted peer's map,
  wrote it to `config.toml` without asking, kept it after the introducer
  was forgotten, and **peers were not bound to a key**, so any trusted key
  answering at a peer's IP got the input meant for that peer.

## Decision

- **Trust is by key, never by address.** `trusted_peers` holds one line per
  peer: `<public key, 64 hex digits> <note>`; the note (e.g. the address it
  paired from) is informational (`trust.rs`).
- **Full keys, not fingerprints, are stored and announced.** A fingerprint
  is a hash and cannot be turned back into the key, and the code needs the
  key to compare it with the one the handshake proved. Discovery sends the
  32-byte key (`"MKY1" | port | key | name`). `mickey forget <fingerprint>`
  hashes each stored key to find the right one.
- **`config.toml` pins each peer's key** (`[[peer]] key`, written when it
  first connects). From then on Mickey talks to that address only with that
  key; a different trusted key answering there is refused
  (`crates/mickey-config/src/lib.rs`, `source.rs`).
- **A PC the group introduces is asked about once** (#184). Each PC shows
  "Let *X* in? *Y* says *X* is one of your PCs" in a `mickey-ui join`
  window, on its own thread so keys keep moving, and trusts *X* only on
  *Let in*. Asked once per run, whatever the answer; no window means no.
- **Introductions are traced.** A key added through the group is noted
  `via <introducer's fingerprint>`, and forgetting a PC also forgets every
  PC it introduced. A group map can add only addresses on a local network
  (private and link-local ranges, 100.64.0.0/10 for VPNs such as
  Tailscale, IPv6 unique-local), never a public address or loopback.

## Alternatives considered

- **Trust by IP address.** Rejected from the start (PRD 0002, *Trusted
  peers*): it is exactly what an attacker on the LAN can fake.
- **Store fingerprints only.** The first plan for `trusted_peers` (PRD
  0002). Changed once the code needed the key itself (PRD 0002,
  *Discoveries*, "What changed after PRD 0010").
- **Trust group members at once** (the first group design). Fixed by #184.
- **Let the introduced PC in and notify the owner.** Rejected: "decided
  with the owner on 2026-10-24 (ask once, rather than let it in and
  notify)" (PRD 0010, *Discoveries*).

## Consequences

Good:

- A new address after DHCP changes nothing; a stranger with the old address
  gets nothing.
- A PC that took over a trusted peer's address, even another trusted PC,
  does not receive input meant for that peer.
- One pairing still reaches the whole group, now with the owner's consent
  on every PC.

Bad, and known limits:

- **A reinstalled PC is a new identity** and must pair again; Mickey now
  says so when a PC answers with a new key (PR #294).
- **The question can be missed**: no display, or a *No* this run, leaves
  the PC out until the next run asks again.
- **The note `via …` is the only link between a PC and its introducer**,
  kept in a text file any process of the user can edit (except for the
  administrator mode on Windows, [ADR 0009](0009-windows-administrator-mode.md)).
- **README is behind here**: its *Security* section still says a PC a
  paired PC introduces "is trusted too", without the question.

## Links

- [PRD 0002](../prds/0002-secure-link.md) (*Trusted peers*, *Discoveries*),
  [PRD 0010](../prds/0010-easy-pairing.md) (*Discoveries*).
- [flows.md](../flows.md#group-trust), *Group trust*;
  [protocol.md](../protocol.md#discovery), *Discovery*.
- Issues #172, #184; PRs #150-#152 (group map), #201, #260, #294.
- Code: `crates/mickey/src/trust.rs`, `crates/mickey/src/group.rs`
  (`is_local`), `crates/mickey/src/source.rs` (`join`, `ask_to_join`),
  `crates/mickey/src/discovery.rs`, `crates/mickey-config/src/lib.rs`
  (`Peer::key`).
