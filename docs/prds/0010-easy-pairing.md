# 0010 — Connect PCs without a terminal

> Status: **draft** · Depends on: [0002](0002-secure-link.md), [0009](0009-tray-icon.md) · Vision: [0000](0000-vision.md)

## Goals

After installing Mickey on two PCs, the owner wants to connect them
without a terminal and without typing addresses. Today that is
impossible: pairing asks "type y to pair" on a terminal (PRD 0002), and
an installed Mickey runs hidden, with no terminal to answer on.

Decided with the owner on 2026-10-22:

- **PCs find each other on the local network.** The settings window lists
  "PCs found" with a *Connect* button; nobody types an IP.
- **The same six-digit code on both screens**, as when pairing Bluetooth:
  the PC that asked shows it, the other PC shows it with *Accept* and
  *Refuse*. Equal codes mean nobody sits in between.
- **After connecting, the Layout tab opens** with the new PC in the strip,
  ready to be dragged into place.

Done means: on two PCs with a fresh install (no config, no keys), the
owner opens the settings on one, clicks *Connect* next to the other, sees
the same code on both, clicks *Accept*, places the PC, and the keyboard
crosses, with no terminal and no restart.

## Approach

1. **A name for this PC** (`[this_pc] name`, the host name by default),
   since the other PCs show it.
2. **Discovery**: every Mickey sends a small UDP broadcast on the LAN
   every few seconds (name, port, key fingerprint) and listens for the
   others'. *(Built with the full public key instead; see Discoveries.)* No mDNS crate: one packet type, one port. It says nothing
   secret; trust still comes from pairing.
3. **The code**: both sides derive six digits from the two public keys of
   the secure link (sorted, hashed). A machine in the middle would have
   its own key on each side, so the two screens would show different
   codes. *(Built from the handshake hash instead; see Discoveries.)*
4. **No terminal**: the PC asked to pair shows a small `mickey-ui` dialog
   (code, *Accept*, *Refuse*) instead of reading the terminal; the asking
   PC shows the code in its settings window while it waits.
5. **Both sides remember each other**: once accepted, each PC trusts the
   other's key and adds it to its own `config.toml`, so either can drive
   the other; the running Mickey connects to the new peer without a
   restart.
6. **Check on a fresh install** of both of the owner's PCs.

## Requirements

- [ ] `[this_pc] name`.
- [ ] PCs found on the LAN, listed by `mickey discover` and the settings.
- [ ] Six-digit pairing code from both keys.
- [ ] Pairing accepted in a window, not a terminal.
- [ ] Both sides add each other; no restart needed.
- [ ] *Connect* in the settings, then the Layout tab.
- [ ] Fresh install on both PCs, connected with no terminal.

## Non-Goals

- PCs on different networks (VPN, internet).
- Pairing more than two PCs at once.

## Discoveries

### The code and the discovery packet as built (2026-10-22)

- **The pairing code comes from the Noise handshake hash**, not from the
  two public keys. `code_from` in `crates/mickey/src/secure.rs` takes the
  first four bytes of the hash both sides hold when the `XX` handshake
  ends, as a big-endian `u32`, modulo 1 000 000, shown as `"042 917"`.
  That hash covers both static keys **and both ephemeral keys**, so it
  still catches a machine in the middle (it would run two handshakes and
  each PC would see a different hash), and it comes for free from `snow`
  with no sorting or extra hashing of our own.
- **So the code changes on every connection.** Two PCs that pair, drop the
  link and try again see a new code the second time. That is fine: the
  code is only compared while both screens show it, never written down.
  The leftover bias of `% 1_000_000` on 32 bits is under 0.03 %.
- **Discovery sends the full 32-byte public key**, not a fingerprint
  (`crates/mickey/src/discovery.rs`, packet
  `"MKY1" | port: u16 | public key: 32 bytes | name length: u8 | name`).
  The table of PCs heard and the trusted peers file are both keyed by the
  full key, so a PC heard on the LAN is matched against them directly,
  and the name of a PC asking to pair is looked up by the key the
  handshake proved. A public key is no more secret than its fingerprint,
  and the packet is still not trusted: trust comes from pairing.
- **The terminal is still a fallback.** When `mickey-ui pair` cannot open a
  window (exit code 2, no display), `trust.rs` asks "type y to pair" on
  the terminal, with the same code printed above it.
