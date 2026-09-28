# Contributing to Mickey

Mickey is a learning project. The rules below exist so that **every line
in `main` has been read, understood and, when it matters, debugged by a human**.
They apply equally to code written by people and by AI assistants.

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
- Integration tests in `tests/` drive two Mickey instances on localhost with
  a fake capture and a fake emitter, so CI never needs real input devices.
- Manual verification steps for real input (two machines, or one machine and
  a uinput device) go in the PR description.
- CI runs on every PR and must be green before merge:
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  `cargo test`.

## PRDs

- Work starts from a PRD in `docs/prds/`. Read the milestone PRD and the
  vision before starting an issue.
- When implementation reveals something the PRD got wrong or did not know,
  add it to that PRD's **Discoveries** section in the same PR.
