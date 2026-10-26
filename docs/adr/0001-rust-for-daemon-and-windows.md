# 0001 — Rust for the whole project, daemon and windows alike

- **Status:** Accepted
- **Date:** 2026-09-28 (the first project plan, commit `e349299`, already
  says "written in Rust")

## Context

Mickey reads a physical keyboard and mouse on one PC and recreates them on
another. The vision ([PRD 0000](../prds/0000-vision.md#performance-budgets))
sets budgets that a language choice has to meet:

- added latency on a wired LAN: **p99 < 2 ms** from the kernel event to the
  emitted event;
- keep up with a **1000 Hz** mouse with no growing queue;
- **< 1 %** of one core while idle, **< 20 MB** resident per peer.

It also has to talk to the operating system at a low level: Linux `ioctl`s
on `/dev/input/*` and `/dev/uinput`, Win32 `SendInput` and low-level hooks.
And it is a learning project: the owner must read and debug every line.

> The reasons below are **inferred**. No document records a comparison of
> languages: Rust was the starting assumption of the project plan, never a
> question in a PRD's *Open Questions*. What follows is what the docs and
> the code show Rust is used for.

## Decision

Everything is Rust, in one Cargo workspace with three crates
([architecture.md](../architecture.md#three-crates)):

- `crates/mickey`: the long-running program (capture, links, emit);
- `crates/mickey-ui`: every window, with egui (see [ADR 0006](0006-egui-in-a-separate-ui-process.md));
- `crates/mickey-config`: `config.toml` rules, shared by the other two, so
  both programs apply exactly the same parsing and checks.

Platform code is hand-written FFI inside `crates/mickey/src/platform/`, the
only place `unsafe` is allowed (CLAUDE.md, hard rule 6). Windows builds are
cross-compiled from Arch for development (`x86_64-pc-windows-gnu`, PRD 0001,
*Spike 0b*).

## Alternatives considered

None is written down. These are the obvious ones, with what the existing
docs say against each (**inferred**):

- **Go.** A garbage collector and a runtime work against "no allocations in
  the per-event hot path" (CONTRIBUTING.md) and a p99 budget in the low
  milliseconds. Calling `ioctl` and Win32 needs cgo or `syscall` wrappers.
- **C or C++.** Meets the budgets and talks to the OS natively, but nothing
  confines memory-unsafe code: every line could be a buffer overflow on
  bytes that come from the network ("every byte from the network is
  untrusted input", CONTRIBUTING.md). Rust lets the project keep `unsafe`
  to one folder and review it there.
- **Electron or Node.** The egui spike measured a *native* GUI at 90 to 125
  MB resident ([PRD 0003](../prds/0003-several-pcs-and-switcher.md),
  *Ticket 3*); a browser engine is larger still, far over 20 MB, and Node
  has no direct access to evdev or low-level hooks.

## Consequences

Good:

- **One language end to end.** The protocol, the focus state machine and
  the key maps are plain, testable Rust that runs in CI on Linux; the
  windows reuse `mickey-config` instead of re-implementing the config rules.
- **`unsafe` is fenced.** All OS calls sit in `platform/`, each block with a
  `// SAFETY:` comment; the rest of the code never sees a file descriptor or
  an OS handle.
- **Small, fast binaries without a runtime.** `mickey` was 1.2 MB stripped
  and about 1 MB resident before the GUI existed (PRD 0003, *Ticket 3*). The
  link does not allocate per event: `Link::send` builds frames in fixed
  stack arrays, and `Message` is `Copy` (architecture.md).
- **Hand-written bindings are small.** `SendInput` with its structs was
  about 60 lines, with a compile-time size check; no `windows-sys` or
  `evdev` crate was needed for input (PRD 0001, *Spikes 0a and 0b*).
- **Reviewed crypto is available in the same language** (`snow`, see
  [ADR 0002](0002-noise-xx-and-pairing-code.md)).

Bad, and known limits:

- **Rust is harder to learn**, which matters in a learning project; the
  review challenge in every PR exists partly to check understanding.
- **Some OS APIs were too large to write by hand**: shared audio pulls in
  `wasapi` and Microsoft's `windows` crate on Windows and `libpulse-binding`
  on Linux (`crates/mickey/Cargo.toml`), so "no `windows-sys` crate" in
  architecture.md now holds for input only.
- **Linux binaries depend on the glibc they were built against**, which
  shaped the release build ([ADR 0010](0010-releases-and-license.md)).

## Links

- [PRD 0000, vision](../prds/0000-vision.md): architecture, budgets.
- [PRD 0001, M0](../prds/0001-m0-hotkey-switch.md): spikes 0a and 0b
  (hand-written bindings, cross-compiling).
- [CONTRIBUTING.md](../../CONTRIBUTING.md): `unsafe`, errors, performance.
- [architecture.md](../architecture.md): crates and the platform layer.
- Code: `crates/mickey/src/platform/`, `Cargo.toml` (workspace).
