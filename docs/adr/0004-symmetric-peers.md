# 0004 — Symmetric peers: every PC runs both the source and the target side

- **Status:** Accepted
- **Date:** 2026-09-28 (recorded as the goal, commit `bed5ac2`);
  2026-10-21 (built: PRD 0008, PRs #77 and #78)

## Context

M0 had fixed roles: `mickey source` on the Linux laptop, `mickey target` on
the Windows PC ([PRD 0001](../prds/0001-m0-hotkey-switch.md)). The vision
asked: "should every peer be able to act as the source, or is the Linux
machine always the source?" On 2026-10-21 the owner asked for the other
direction too: the Windows keyboard driving the Linux laptop
([PRD 0008](../prds/0008-windows-drives-linux.md#goals)). The owner's
three PCs (Arch + Hyprland, Windows 10, Ubuntu + KDE) each have a keyboard,
and the one in use should be whichever one the owner touches.

## Decision

**Peers are symmetric.** The same `mickey` binary runs on every PC, and
`mickey run` plays both roles in one process
([architecture.md](../architecture.md#one-process-two-sides)):

- the **source side** (`source::run` in `source/mod.rs`) captures this PC's
  keyboard and mouse and keeps a link to every peer;
- the **target side** (`target` in `target/mod.rs`) listens and turns what peers
  send into real input here.

The two sides share the trusted keys (`trust::Shared`), the clipboard
(`clipboard::Keeper`) and a `TargetNews` channel. Each platform needs both
a capture and an emit backend ([ADR 0005](0005-input-below-the-desktop.md)).
`controls_others = false` or `controlled = false` in `[this_pc]` turns one
side off; `mickey source` and `mickey target` still run one side alone.

Events on the wire are Linux evdev codes in both directions; Windows
translates its scan codes to evdev when capturing and back when emitting
(`keymap.rs`, `emit.rs`).

## Alternatives considered

- **One fixed source (the Linux laptop) and targets only.** The M0 shape.
  Rejected by the owner: it leaves the Windows keyboard unable to drive
  anything, and the vision's long-term "done" is three PCs side by side,
  driven from any of them.
- **A central server that every PC connects to** (the Synergy/Deskflow
  server/client model). Not discussed in the docs (**inferred** as the
  obvious alternative). The glossary deliberately avoids "server" and
  "client" ([CONTEXT.md](../../CONTEXT.md)); with symmetric peers no PC
  has to be on for the others to work together.
- **Separate source and target programs.** Rejected in practice: one
  process lets both sides share trust, the clipboard and news without IPC.

## Consequences

Good:

- Any keyboard drives any PC. The Windows keyboard reaches the laptop with
  the same hotkey (Right Alt + Right Shift is AltGr's physical key, scan code
  `E0 38`).
- Everything between capture and emit (protocol, secure link, pairing,
  focus, hotkeys, switcher) was already platform-independent and is reused
  unchanged (PRD 0008, *Approach*).
- Pairing and groups follow: once accepted, each PC trusts the other and
  adds it to its own `config.toml`, so either can drive the other
  ([PRD 0010](../prds/0010-easy-pairing.md#approach)).

Bad, and known limits:

- **Feedback loops are possible**: a PC that is both source and target
  could capture the input it just emitted. On Windows the hooks let through
  every event with Windows' own injected flag (`LLKHF_INJECTED`,
  `LLMHF_INJECTED`), so input injected by other programs (on-screen
  keyboards, remote desktop tools) is never forwarded either (PRD 0008,
  *Discoveries*).
- **Twice the links**: two paired PCs may hold up to four TCP connections,
  an input link and a data link in each direction (protocol.md).
- **Only one source side per PC.** Two would both capture the keyboard and
  fight over the grab, so `source::run` refuses to start when another
  `mickey source` is already running (`control::source_running`).
- **Every platform needs both backends** before it can join as a full peer.
  macOS would need a capture and an emit backend (vision, *Roadmap*).

## Links

- [PRD 0000, vision](../prds/0000-vision.md#architecture), *Open
  Questions* (symmetric peers, resolved).
- [PRD 0008, Windows drives Linux](../prds/0008-windows-drives-linux.md).
- [architecture.md](../architecture.md#one-process-two-sides).
- PRs #75 (PRD), #77 (hooks and reverse keymap), #78 (both ways).
- Code: `crates/mickey/src/run.rs` (`run_from_config`),
  `crates/mickey/src/target/` (`target`, `serve_source`), `crates/mickey/src/source/`, `crates/mickey/src/input/mod.rs`,
  `crates/mickey/src/input/emit.rs`, `crates/mickey/src/input/keymap.rs`.
