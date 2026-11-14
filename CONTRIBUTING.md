# Contributing to Mickey

By taking part you agree to the [code of conduct](CODE_OF_CONDUCT.md).
Security problems go through [SECURITY.md](SECURITY.md), never a public issue.

Mickey is a learning project. The rules below exist so that **every line
in `main` has been read, understood and, when it matters, debugged by a human**.
They apply equally to code written by people and by AI assistants.

## Getting started

1. Build and test: see [Build from source](README.md#build-from-source) in
   the README. `cargo test` must pass before you open a PR.
2. Pick an issue labelled `good first issue` or `help wanted`, and say in it
   that you are taking it, so two people do not do the same work.
3. Something new, not in an issue yet? Open one first (bug or feature
   template) and wait for a reply before writing code. A change the
   maintainer did not agree to is likely to be declined, however good.
4. Read the PRD the issue links (see [PRDs](#prds)).

## The review contract

1. **Small PRs.** Aim for ≤ 300 changed lines (excluding lockfiles and
   generated files). Larger work is split into several PRs.
2. **Every PR has a review challenge.** The PR description contains a
   `## Review challenge` section with 2–3 questions that can only be answered
   by reading the diff (e.g. "what happens if the peer disconnects while a key is held?",
   "why is this delta `i32` and not `i16`?"). When the PR touches timing,
   buffers, callbacks or anything better understood at runtime, it may add one
   **debugger question** answered by setting a breakpoint and running.
3. **The reviewer answers in a PR comment.** The author checks the answers. A
   wrong answer means the code or the explanation is unclear, and that gets
   fixed in the PR, not waved through.
4. **Only the owner merges.** Nobody else, and no AI assistant, merges to
   `main`, ever. Merge happens after the challenge is answered correctly and
   CI is green.
5. **Concepts are explained.** When a PR uses a classic data structure or
   algorithm (ring buffer, LRU cache, trie, hashing, state machine, …), the
   description has a `## Concept` section: what it is, the closest LeetCode
   problem, and *why it solves this problem better than the naive approach*.

### Reviewing in VS Code (recommended setup)

- **rust-analyzer**: types, go-to-definition, inline errors.
- **CodeLLDB**: debugger for Rust, breakpoints in PR code.
- **GitHub Pull Requests**: check out a PR, review the diff side by side,
  comment on lines, and answer the challenge without leaving the editor.

Review flow: *check out PR → read diff → run it → set a breakpoint where it is
interesting → answer the challenge → approve → merge.*

## Branches and commits

- Never commit directly to `main`. Branch names: `<type>/<short-slug>`, e.g.
  `feat/audio-ring-buffer`, `fix/pitch-stride`, `docs/m1-prd`.
- [Conventional Commits](https://www.conventionalcommits.org/):
  `feat:`, `fix:`, `refactor:`, `perf:`, `test:`, `docs:`, `chore:`, `ci:`.
  Optional scope: `feat(audio): add ring buffer`.
- One logical change per commit. The commit message says *why*, the diff
  says *what*.
- Everything in the repository is in **English**: code, comments, commits,
  docs, PR descriptions.

## Code rules

### `unsafe`
- All `unsafe` code lives in `crates/mickey/src/platform/` (ioctls, OS APIs).
  Anywhere else it is a review blocker.
- Every `unsafe` block has a `// SAFETY:` comment explaining which invariant
  makes it sound. "It works" is not an invariant.
- No `static mut`.

### Errors
- No `unwrap()`/`expect()` on anything that depends on user input, files, the
  network or a peer. Return `Result` and show a clear message.
- `expect("reason")` is allowed only for true invariants, and the reason
  explains why it cannot fail.
- A misbehaving or malicious peer must never crash Mickey. Every byte from
  the network is untrusted input.

### Simplicity
- No abstraction (trait, generic, plugin system, config knob) until a second
  real use case exists.
- Prefer the standard library. Each new dependency is justified in the PR
  description: what it does, why the stdlib or a few lines of code is not
  enough, and its size.
- Deliberate shortcuts with a known ceiling get a comment:
  `// ponytail: <limit>, <upgrade path>`.

### Performance
- The budgets in [docs/prds/0000-vision.md](docs/prds/0000-vision.md#performance-budgets)
  are requirements. Breaking one is a bug.
- **Measure before optimizing.** A `perf:` PR shows before/after numbers and
  how they were measured.
- No allocations in the per-event hot path (reading, encoding, sending,
  decoding and emitting one input event) unless justified in the PR.

### Paths
- No hardcoded absolute paths. Use config or the platform's standard
  directories (see the vision's *Installed layout*).

### Input safety
- Anything that grabs a device (e.g. `EVIOCGRAB`) releases it on every exit
  path: normal exit, error, panic, peer disconnect. Use a guard type whose
  `Drop` releases the grab.
- When control leaves a machine, every key and button still held there is
  released first. Stuck keys are bugs.
- Key codes and typed text are never logged above `trace` level and never
  written to disk.

## Tests and CI

- Unit tests next to the code (`#[cfg(test)] mod tests`) for pure logic
  (protocol encoding, layout, hotkey state machine).
- Link tests, also inside the crates (e.g. `transport.rs`, `secure.rs`),
  run both ends of a link in one process over a real localhost socket:
  the test sends the events a capture would, and a fake emitter (a
  closure) records what arrives, so CI never needs real input devices.
- Not there yet: integration tests in `tests/` that drive two whole
  Mickey instances on localhost with a fake capture and a fake emitter.
  That is the aim once a change needs more than the link tests cover.
- Manual verification steps for real input (on the real Linux and Windows
  machines) go in the PR description.
- CI runs on every PR and must be green before merge:
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`, `tools/check-unsafe.sh`, `cargo deny check` (settings in
  `deny.toml`) and `zizmor` on the workflows.

## PRDs

- Work starts from a PRD in `docs/prds/`. Read the milestone PRD and the
  vision before starting an issue.
- When implementation reveals something the PRD got wrong or did not know,
  add it to that PRD's **Discoveries** section in the same PR.

## Security-sensitive areas

A mistake in these parts lets someone read what you type, control your PC,
or lock you out of it. A PR that touches them needs, **before any code**,
an issue where the maintainer agreed to the change, and it says in its
description which of these it touches and why that is safe.

| Area | Where | What must hold |
|---|---|---|
| The encrypted link | `crates/mickey/src/net/secure.rs`, `transport.rs`, `protocol.rs` | Noise `XX` with the same parameters; nothing but the handshake is sent before it completes; every byte from a peer is untrusted. |
| Pairing and trust | `net/secure.rs` (the six-digit code), `net/trust.rs`, `net/group.rs`, `net/identity.rs` | The code comes from the handshake; trust is by key, never by IP address or hostname; a PC joins the group only after the user says yes. |
| The way home | `input/hotkey.rs`, `source/mod.rs`, `platform/hooks.rs` | Holding Right Shift (see [SECURITY.md](SECURITY.md)) always brings the keyboard and mouse back, whatever the configuration. No setting may turn it off. |
| Grabs and held keys | `input/`, `platform/` | See [Input safety](#input-safety). |
| The clipboard | `crates/mickey/src/clipboard/` | It carries passwords and private files. Content goes only to and from the PC the keyboard is on, never to the log (its kind and size at most), and received files stay in Mickey's cache folder. |
| Updates and releases | `crates/mickey/src/update.rs`, `packaging/release-key.pub`, `tools/release-key/`, `.github/workflows/` | An update installs only after both checks pass (the `SHA256SUMS` signature and the file's hash). Only the maintainer changes the release key. |
| What gets installed | `packaging/` (udev rule, service, installer, firewall rule) | Grant no permission beyond what is needed: no world-writable device, no wider group, no firewall rule beyond Mickey's program on private networks. |

Also, anywhere in the code:

- No new network listener, port or outgoing connection without an issue
  that agreed to it. Input and clipboard go only to paired PCs (see the
  [vision PRD](docs/prds/0000-vision.md)). The only other connection is
  the daily update check to GitHub, which the user can turn off. No
  telemetry, no analytics, no crash reports sent anywhere.
- No secrets in the repository: keys, tokens, real IP addresses or
  usernames. Examples use `192.0.2.0/24`.
- Workflows (`.github/workflows/`) pin every action to a full commit SHA,
  never run on `pull_request_target`, and never print secrets.
- A new dependency in these areas gets more scrutiny than elsewhere: say
  who maintains it and whether it has had a security audit.

Found a weakness instead of changing one? Report it privately, as
[SECURITY.md](SECURITY.md) says.

## AI-assisted contributions

Welcome, under the same rules as any other code. You are the author: you
must understand and be able to explain every line, and answer the review
challenge yourself. Say in the PR description which tool you used.

## License of contributions

Mickey is under [GPL-3.0-or-later](LICENSE). By opening a PR you agree that
your contribution is published under the same license. There is no CLA.
Do not paste code from elsewhere unless its license is compatible with
GPL-3.0-or-later; when you do, say where it came from in the PR.
