# 3. Pairing and trust

> **The question:** you install Mickey on a new PC and press **Connect**.
> A window shows `042 917`. How does Mickey know the PC answering is the
> one on your desk, and not a laptop in the next room pretending to be
> it? And why is six digits enough?

## Who you are: the identity (`identity.rs`)

- [`identity.rs:26`](../../crates/mickey/src/identity.rs#L26)
  `load_or_create`: on first run, an X25519 key pair. Only the 32-byte
  private key is saved; the public key is derived on load, so the two can
  never disagree (PRD 0002, *Ticket 1*).
- [`identity.rs:125`](../../crates/mickey/src/identity.rs#L125) `load`:
  on Linux a key file readable by others is **refused**, with the `chmod`
  to fix it.
- [`identity.rs:91`](../../crates/mickey/src/identity.rs#L91)
  `fingerprint`: 160 bits of a hash of the public key, in base32
  (`mickey id` prints it).

## Finding each other: discovery (`discovery.rs`)

- [`discovery.rs:137`](../../crates/mickey/src/discovery.rs#L137) `start`:
  every 3 s, a UDP broadcast on port 4243 with
  `"MKY1" | port | public key | name`
  ([`:46`](../../crates/mickey/src/discovery.rs#L46)). A PC that hears
  one answers it directly, at most every 3 s, because a broadcast can
  leave by the wrong network adapter.
- [`discovery.rs:58`](../../crates/mickey/src/discovery.rs#L58) `decode`
  returns `None` for anything that is not exactly a Mickey packet;
  only local addresses are heard
  ([`group.rs:41`](../../crates/mickey/src/group.rs#L41) `is_local`); at
  most 64 PCs are remembered, forgotten after 10 s of silence.
- **Nothing here is trusted.** Discovery only fills the "On this network"
  list and puts a name in the pairing window.

## The handshake and the code (`secure.rs`)

*Connect* runs `mickey connect`, which reaches
[`source.rs:1390`](../../crates/mickey/src/source.rs#L1390)
`connect_and_pair`:

1. `transport::connect` opens TCP and runs
   [`secure.rs:52`](../../crates/mickey/src/secure.rs#L52)
   `Link::initiate`: the three Noise `XX` messages
   ([`secure.rs:183`](../../crates/mickey/src/secure.rs#L183)). Afterwards
   each side holds the other's public key, **proven**: only the holder of
   the private key could finish the handshake.
2. [`secure.rs:233`](../../crates/mickey/src/secure.rs#L233) `code_from`:
   the first 4 bytes of the handshake hash, as a `u32`, modulo 1 000 000.

Why this catches a machine in the middle: the hash covers both static keys
**and** both fresh ephemeral keys. An attacker in between runs two
separate handshakes, so your two PCs end with two different hashes and
show two different codes. And because the ephemeral keys are fresh, every
connection shows a new code
([0010-easy-pairing.md](../prds/0010-easy-pairing.md), *Discoveries*).

## Asking the human (`trust.rs`)

- [`trust.rs:175`](../../crates/mickey/src/trust.rs#L175) `trust_or_ask`:
  a known key passes at once; an unknown one goes to the owner.
- [`trust.rs:242`](../../crates/mickey/src/trust.rs#L242) `ask_once`:
  one question on screen at a time; an address that was refused waits
  60 s before it may ask again. A stranger cannot flood you with windows.
- [`trust.rs:289`](../../crates/mickey/src/trust.rs#L289)
  `ask_in_window` runs `mickey-ui pair <who> <code>`: exit 0 is *Pair*, 1
  is *Refuse*, 2 means "no display", and the terminal is asked instead
  ([`:305`](../../crates/mickey/src/trust.rs#L305)).

On the other PC, [`main.rs:733`](../../crates/mickey/src/main.rs#L733)
`serve_source` does the same, with one extra gate:
[`trust.rs:211`](../../crates/mickey/src/trust.rs#L211) `pairing_open`.
A PC that trusts nobody yet (a fresh install) may be asked; otherwise only
for 2 minutes after the owner opens pairing (**Let a new PC pair**, or
`mickey pairing`). Outside that window an unknown PC is refused with no
window at all.

**Nothing crosses until both said yes.** The target answers `Hello` only
once it trusts the source, and the source sends nothing before that
`Hello`, waiting up to 90 s for the other owner
([`transport.rs:40`](../../crates/mickey/src/transport.rs#L40)).

## Remembering: `trusted_peers`

[`trust.rs:1`](../../crates/mickey/src/trust.rs#L1): one line per peer,
`<public key, 64 hex> <note>`. Trust is **by key, never by address**: a
DHCP change does not matter, and a PC that takes over an address does not
inherit its trust. Two more places enforce that:

- [`source.rs:752`](../../crates/mickey/src/source.rs#L752): a PC
  answering a known address with another key is refused.
- [`source.rs:1416`](../../crates/mickey/src/source.rs#L1416)
  `open_data_link`: the data link (chapter 4) must answer with the same
  key as the input link.

[`source.rs:1438`](../../crates/mickey/src/source.rs#L1438) `reconnect`
never asks: it runs in the background while you may be typing elsewhere,
so an unknown key is just left alone.

## The group: one pairing, every PC

Each source side sends its **group map** (itself and every peer it knows,
with positions) over every data link, kind 16 ([`group.rs:91`](../../crates/mickey/src/group.rs#L91)).
The receiver:

1. moves positions into its own coordinates,
   [`group.rs:148`](../../crates/mickey/src/group.rs#L148) `as_seen_by`;
2. for a PC it does not trust yet, asks once per run in a
   `mickey-ui join` window, "*Y* says *X* is one of your PCs"
   ([`source.rs:332`](../../crates/mickey/src/source.rs#L332));
3. on *Let in*, [`source.rs:355`](../../crates/mickey/src/source.rs#L355)
   `join` trusts it with the note `via <fingerprint of Y>`, adds it to
   `config.toml` and connects.

That note matters: `mickey forget` on *Y* also forgets every PC *Y*
introduced ([`trust.rs:120`](../../crates/mickey/src/trust.rs#L120)).

## Try it

```sh
cargo test -p mickey both_sides_show_the_same_code_and_each_connection_a_new_one
mickey-ui pair "Test PC" "123 456"; echo $?     # fish: echo $status
```

The test ([`secure.rs:437`](../../crates/mickey/src/secure.rs#L437))
runs two handshakes on localhost and checks both claims of this chapter.
The window is the real pairing window with a made-up code: click *Pair*
or *Refuse* and see the exit code `trust.rs` reads. No Mickey is involved,
so nothing is paired.

## Check yourself

1. Why derive the code from the handshake hash instead of showing each
   PC's fingerprint?
2. The code changes on every connection. Is that a problem?
3. A laptop you never paired connects to port 4242 while pairing is
   closed. What does it see, and what do you see?
4. Why does `reconnect` never open a pairing window?
5. You forget PC *Y*. Why does PC *X*, which *Y* introduced, disappear
   too?

<details>
<summary>Answers</summary>

1. It is short enough to compare at a glance, comes free from `snow`, and
   still catches a machine in the middle: the hash covers both static and
   both ephemeral keys, so two separate handshakes give two codes.
2. No: the code is only compared while both screens show it, never
   written down. (The leftover bias of `% 1 000 000` is under 0.03 %.)
3. The handshake completes, then `serve_source` sees an unknown key and
   `pairing_open` is false: it logs a refusal and closes. You see no
   window. The laptop gets no `Hello`, so it can send nothing that is
   emitted.
4. Reconnects run in the background while the owner may be typing on
   another PC; a surprise window there is both confusing and a way to
   trick someone into clicking *Pair*.
5. `join` stored *X* with the note `via <fingerprint of Y>`, and `forget`
   removes every line whose note names the forgotten fingerprint.

</details>
