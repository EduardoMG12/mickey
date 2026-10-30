# 1. What Mickey is and why it exists

> **The question:** you have a Linux laptop and a Windows PC on the same
> desk. Why would you write 20 000 lines of Rust instead of buying a
> second keyboard?

## The problem

Two or more computers on one desk, each with its own screen, and one pair
of hands. A second keyboard and mouse means reaching across the desk and
never quite knowing which keyboard you are typing on. A hardware KVM
switch swaps one set of devices between PCs, but it costs money and only
one PC at a time is "live".

Mickey's answer, in the README's words: *one keyboard and mouse for all
your computers*. Push the cursor off the edge of one screen and it shows
up on the next PC; press a hotkey and the keyboard moves; copy on one PC
and paste on another; hear every PC in one pair of headphones.

The owner's own setup is the first user: an Arch Linux laptop running
Hyprland and a Windows 10 PC ([0000-vision.md](../prds/0000-vision.md),
*Goals*: "the owner's three computers sit side by side, and one keyboard
and mouse drive all three").

## The words you need

Mickey's glossary is short and the code uses it everywhere
([CONTEXT.md](../../CONTEXT.md)):

| Word | Means |
|---|---|
| **Peer** | A computer running Mickey |
| **Source** | The peer whose physical keyboard and mouse are in use right now |
| **Target** | The peer receiving that input |
| **Focus** | Which peer is the target; exactly one, maybe the source itself |
| **Switch** | A change of focus |
| **Identity** | A peer's long-term key pair; the private half never leaves |
| **Pairing** | Both screens show the same code, and each peer trusts the other's key |

Learn these before chapter 2: "the source sends to the focused peer" is a
sentence you will read in the code.

## The alternatives

Mickey says openly where it comes from (README, first paragraph):
*inspired by Synergy, Deskflow/Input Leap and lan-mouse, built from
scratch to be understood line by line*.

| Tool | In one line |
|---|---|
| **Synergy** | The original software KVM; a commercial product today |
| **Deskflow** | Synergy's free, open-source upstream |
| **Barrier / Input Leap** | Community forks of Synergy; Input Leap continues Barrier |
| **lan-mouse** | A Rust tool built around Wayland's newer protocols |

(These one-liners are general background, not something this repository
documents. Check each project's own page before quoting it in an
interview: their status changes.)

## What is different about Mickey

Everything in this list is backed by the repository; the pointer says
where.

1. **Pairing like Bluetooth, with real cryptography.** Both screens show
   the same six-digit code, derived from the Noise handshake itself, so a
   machine in the middle shows a different code
   ([0010-easy-pairing.md](../prds/0010-easy-pairing.md), *Discoveries*).
   Chapter 3.
2. **One pairing reaches the whole group.** A PC paired with one of yours
   can be let into the rest ([flows.md](../flows.md), *Group trust*).
3. **Symmetric peers.** Every PC runs the same `mickey run`, with a
   source side and a target side; whichever keyboard you touch is in
   charge ([architecture.md](../architecture.md), *One process, two
   sides*).
4. **Shared audio.** Every PC's sound in one pair of headphones, mixed,
   each at its own volume ([0012-shared-audio.md](../prds/0012-shared-audio.md)).
   Chapter 5.
5. **A way home no setting can remove.** Hold Right Shift for 3 seconds
   ([`input/hotkey.rs:66`](../../crates/mickey/src/input/hotkey.rs#L66)). Chapter 6.
6. **Open source and readable.** GPL-3.0-or-later, no async runtime,
   hand-written OS bindings, and every `unsafe` block confined to
   `crates/mickey/src/platform/` with a `// SAFETY:` comment
   ([CLAUDE.md](../../CLAUDE.md), hard rule 6).
7. **Private by design.** No telemetry, no accounts, no internet; LAN
   only, encrypted, paired PCs only (README, *Privacy*).

## Where it stands

Version 0.2.1, **alpha** (README, top). Used daily between the owner's
Arch laptop and Windows PC, and tested on eight more Linux distros
(Ubuntu 24.04 and 22.04, Mint, Fedora, Debian, Pop!_OS, Xubuntu,
openSUSE; README, *Tested on*). Honest limits are listed too: screen edges only on Windows
and Hyprland, touchpad tap and two-finger scroll missing, no code signing
yet (README, *Known limits*).

## Why it is built the way it is

Mickey is **a learning project first** ([CLAUDE.md](../../CLAUDE.md)).
That explains choices that look odd for a product:

- PRs stay under about 300 lines and carry a `## Review challenge`, so
  every line in `main` was read by a human ([CONTRIBUTING.md](../../CONTRIBUTING.md)).
- Few dependencies, each justified in the PR that added it; the
  cryptography is the exception that proves the rule: "write it ourselves
  to learn" loses to "use reviewed code" ([0002-secure-link.md](../prds/0002-secure-link.md),
  *Approach*).
- Performance has budgets written down before the code: p99 under 2 ms
  added latency on a wired LAN, under 1 % idle CPU, under 20 MB per peer
  ([0000-vision.md](../prds/0000-vision.md), *Performance budgets*).

## Try it

On a PC where Mickey is installed, run:

```sh
mickey --help
mickey status
mickey id
```

`--help` is the whole command-line surface in one screen
([`cli.rs:5`](../../crates/mickey/src/cli.rs#L5)). `status` says where the
keyboard is and which PCs are online. `id` prints this PC's fingerprint,
the human-readable hash of its public key. Nothing here grabs input.

## Check yourself

1. In one sentence each: what is a *source*, a *target* and the *focus*?
2. Name two things Mickey does that a plain software KVM does not.
3. Why is the GUI (`mickey-ui`) a separate program from `mickey`?
4. What does "symmetric peers" mean in practice?

<details>
<summary>Answers</summary>

1. The source is the PC whose physical keyboard and mouse are in use; the
   target is the PC receiving that input; the focus says which peer is the
   target right now (exactly one, possibly the source itself).
2. Any two of: pairing with a six-digit code from the handshake hash,
   group trust (one pairing reaches the group), shared audio, a shared
   clipboard with history, a way home no setting can remove.
3. egui costs 90 to 125 MB while a window is open, and `mickey` holds the
   keyboard all day: it must stay small and never hang because a window
   did ([architecture.md](../architecture.md), *Three crates*; PRD 0003,
   *Ticket 3*).
4. Every PC runs the same `mickey run` with both a source side and a
   target side, so any keyboard can drive any PC; there is no fixed
   "server".

</details>
