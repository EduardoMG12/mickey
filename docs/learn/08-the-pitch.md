# 8. The pitch

> **The question:** an interviewer says "tell me about a project you're
> proud of", or a friend asks "so what's this Mickey thing?". You have
> 30 seconds. Then they lean in. What do you say, and can you back every
> sentence with a file?

Say these out loud. Then say them again without reading.

## 30 seconds

> Mickey lets one keyboard and mouse drive several computers on the same
> network, Linux and Windows. You push the cursor off one screen and it's
> on the next PC; copy on one, paste on the other; and every PC's sound
> plays in one pair of headphones. It's about 20 000 lines of Rust, open
> source, everything encrypted end to end, and you pair PCs by checking
> the same six-digit code on both screens, like Bluetooth.

## 2 minutes

Add the *how*, one sentence per chapter:

1. **Capture and emit are the only per-OS parts.** Linux reads
   `/dev/input` and grabs it with `EVIOCGRAB`, Windows uses low-level
   hooks; the target injects through `uinput` or `SendInput`. Everything
   between is shared Rust, and key codes cross as Linux evdev codes
   (chapter 2).
2. **The link is Noise `XX`** through the `snow` crate. The pairing code
   comes from the handshake hash, so a machine in the middle would make
   the two screens disagree (chapter 3).
3. **Bulk data has its own connection**, so a 5 MB screenshot never sits
   in front of a key press (chapter 4).
4. **Audio is UDP** with a per-stream key and a jitter buffer that starts
   at 10 ms and grows only when the network jitters (chapter 5).
5. **It can't lock you out**: grabs are RAII guards, a silent peer loses
   the focus in 4 s, and holding Right Shift for 3 s always comes home
   (chapter 6).

## 5 minutes

The 2-minute version, plus the parts that show judgement:

- **Process.** It's a learning project: every PR is under about 300 lines
  and carries review questions the owner must answer before merging
  ([CONTRIBUTING.md](../../CONTRIBUTING.md)). It was built with AI help,
  and that process is how the owner makes sure every line is understood.
- **Constraints written first.** Latency, CPU and memory budgets were
  written before the code ([0000-vision.md](../prds/0000-vision.md)). The
  first Wi-Fi numbers: p50 2.1 ms, p99 22.6 ms one way (PRD 0001,
  *Step 8*); the wired budget, p99 under 2 ms, is still to measure, and
  [`tools/latency-bench/`](../../tools/latency-bench/README.md) exists for
  that.
- **Security as threat modelling, not a checkbox.** Trust by key not by
  address; a paired PC can read your clipboard only once, right after the
  keyboard leaves it; the administrator mode on Windows keeps its trusted
  list where only administrators can write (chapters 3, 4, 6).
- **Learning from reality.** Each PRD has a *Discoveries* section: what
  the spike or the real two-PC run proved wrong. Good stories: recording
  silence because the sound server still answered the old default
  monitor; a logon task that never started on battery (PRD 0012 and PRD
  0001, *Discoveries*).
- **Honest limits.** Edges only on Windows and Hyprland, the `input`
  group on Linux, no code signing yet, so Windows Defender may flag it
  (README, *Known limits*).

## The commercial angle

What follows is a set of hypotheses to discuss, not something the
repository has tested. Be clear about that when you say it.

**Who has the problem.** Anyone with two or more computers on one desk:
developers with a Linux and a Windows machine, people with a work laptop
next to a personal PC, gamers and streamers (the owner's own Windows PC
routes audio through Voicemeeter, and Mickey works with it: PRD 0012,
*Discoveries*). Shared audio is the feature that is easiest to show in
ten seconds.

**What blocks adoption today,** each with its fix already on the roadmap:

| Blocker | Roadmap |
|---|---|
| Edges only on Windows and Hyprland; the `input` group | Desktop portals for GNOME and KDE (#178) |
| No package managers | AUR, `.deb`, `.rpm`, winget, Scoop (#176) |
| No macOS | A first look (#177) |
| Defender flags the unsigned installer | SignPath's free signing for open source (README, *Code signing policy*) |

**How it could be offered.** Mickey is GPL-3.0-or-later, so the code
stays open. Common models for open-source desktop tools: free builds plus
sponsorship; paid convenience (signed builds, store listings) while the
source stays free; paid support for small teams. Selling under another
license would need every contributor's agreement. A company would also
ask what Mickey sends where: the privacy answer (LAN only, no telemetry,
README *Privacy*) is a selling point.

## Interview questions, with model answers

| Question | Short answer | Backed by |
|---|---|---|
| Why Rust? | Parsing untrusted bytes from the network and calling OS APIs; Rust keeps the memory safety, and every `unsafe` is confined to one folder with a `// SAFETY:` comment | `crates/mickey/src/platform/`, CLAUDE.md rule 6 |
| Why TCP for input and UDP for audio? | A lost key release is a stuck key, so input needs order and delivery; a late audio packet is useless, so audio takes a 5 ms gap instead of a stall | [`net/transport.rs:24`](../../crates/mickey/src/net/transport.rs#L24), [`audio/wire.rs:1`](../../crates/mickey/src/audio/wire.rs#L1) |
| How is it secure? | Noise `XX` with X25519, ChaCha20-Poly1305 and BLAKE2s; trust by key; a pairing code from the handshake hash | [`net/secure.rs:238`](../../crates/mickey/src/net/secure.rs#L238), [protocol.md](../protocol.md) |
| What if the network dies with a key held? | The target releases everything it saw pressed; the source drops the grab after 4 s of silence | [`net/transport.rs:234`](../../crates/mickey/src/net/transport.rs#L234), [`source/mod.rs:61`](../../crates/mickey/src/source/mod.rs#L61) |
| Why no async runtime? | Each thread blocks on exactly one thing; the main loop never waits on the network or a window. Simpler to read and debug | [architecture.md](../architecture.md), *Threads* |
| How do you test input code in CI? | Both ends of the link in one process on localhost, with a closure as the emitter | [`net/transport.rs:314`](../../crates/mickey/src/net/transport.rs#L314) |
| Is it fast? | Wi-Fi p50 2.1 ms, p99 22.6 ms one way; no allocation per event | PRD 0001 *Step 8*, [`net/secure.rs:93`](../../crates/mickey/src/net/secure.rs#L93) |
| How did you use AI? | As a pair programmer under rules: small PRs, review questions, only the owner merges | [CLAUDE.md](../../CLAUDE.md), [CONTRIBUTING.md](../../CONTRIBUTING.md) |
| What's next? | Portals for any Linux desktop, packages, macOS, signed builds | README, *Roadmap* |

## Try it

Record yourself giving the 2-minute version (a phone is enough). Listen
back once and note every place you said "kind of" or "I think": those are
the chapters to reread.

## Check yourself

1. In one sentence, what makes Mickey's pairing safe against a machine in
   the middle?
2. Give one number you can quote about latency, and say what it measured.
3. Name one honest limitation and its planned fix.
4. Why is "it's GPL" relevant to how Mickey could make money?

<details>
<summary>Answers</summary>

1. The six digits come from the Noise handshake hash, which covers both
   PCs' keys and fresh ephemeral keys, so an attacker running two
   handshakes makes the two screens show different codes.
2. Over Wi-Fi, laptop to Windows: p50 2.1 ms, p99 22.6 ms one way,
   measured as half the ping round trip over the same link (PRD 0001,
   *Step 8*). Or the audio: about 65 ms end to end on Wi-Fi (PRD 0012).
3. For example: screen edges work only on Windows and Hyprland; desktop
   portals for GNOME and KDE are planned (#178).
4. Anyone may use, change and share the code, so money has to come from
   something around it (support, convenience, signed builds, sponsorship);
   relicensing would need every contributor's agreement.

</details>
