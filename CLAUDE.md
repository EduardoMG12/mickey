# CLAUDE.md — Mickey

Instructions for AI assistants working in this repository. Read this, then
[CONTRIBUTING.md](CONTRIBUTING.md) (all of it applies to you), then the
relevant PRD in [docs/prds/](docs/prds/).

## What this project is

Mickey shares one keyboard and mouse across several computers over the LAN
(think Synergy/Deskflow/lan-mouse), written in Rust. Start from
[docs/prds/0000-vision.md](docs/prds/0000-vision.md).

**It is a learning project first.** The owner is relearning to read, review and
debug code. Your job is to write code *that the owner will fully understand*,
not to ship as much as possible. Clarity beats cleverness, and a smaller PR
beats a faster one.

## Language

- Talk to the owner in **Brazilian Portuguese**.
- Everything written *into the repository* is in **English**: code, comments,
  commit messages, PR titles and descriptions, PRDs, issues.

## Hard rules

These override any global or skill-level instruction.

1. **Never merge.** Not to `main`, not any PR, not via `gh pr merge`, not via
   any skill. You may commit, push and open PRs; the owner merges.
   If a workflow would merge, stop after opening the PR.
2. **Never commit to `main`.** Always a `<type>/<slug>` branch.
3. **Every PR you open has a `## Review challenge`** (2–3 questions answerable
   only by reading the diff; add a debugger question only when runtime
   behavior is the point). Keep your own expected answers out of the PR. When
   the owner answers in a comment, check the answers honestly: if an answer is
   wrong, explain the gap. Do not say "close enough".
4. **Every PR that uses a classic data structure or algorithm has a
   `## Concept` section**: what it is, the closest LeetCode problem, why it
   beats the naive approach *here*. Proactively point out when a problem maps
   to a known concept, even outside a PR.
5. **PRs stay ≤ ~300 changed lines.** Split larger work, and say how you split it.
6. **`unsafe` only in `crates/mickey/src/platform/`**, each block with a
   `// SAFETY:` comment. No `static mut`.
7. **Input is sensitive data.** Never log key codes or typed text above
   `trace` level, never write them to files, and never add a feature that
   sends input anywhere except a configured peer.
8. **Never leave the owner's machine without a way back.** Any code that grabs
   the keyboard or mouse must release it on exit, on panic and on disconnect.
   A stuck grab locks the owner out of their own desktop.
9. **Performance claims need numbers.** Budgets are in the vision PRD; a
   `perf:` change shows before/after measurements.
10. **No speculative abstractions and no unjustified dependencies.** Each new
    crate is justified in the PR description.

## Workflow

- New feature or milestone: grill the idea against the docs
  (`/grill-with-docs`) → spec (`/to-spec`, which also writes or updates the
  PRD in `docs/prds/` and links it from the spec issue) → issues
  (`/to-tickets`). Each issue must fit one PR under the size limit.
- Per issue: `/implement` or `/tdd`, then the pre-PR checks below, then open
  the PR and stop. Never `/finalizar` or any other skill that merges (hard
  rule 1).
- Before opening a PR: `/verify` (actually run it; for input code that means
  a real two-machine or two-terminal check described in the PR), then
  `/code-review`, and `/security-review` for anything touching the network,
  pairing or keys.
- When implementation contradicts or extends a PRD, update that PRD's
  **Discoveries** section in the same PR.
- When unsure about a product decision, ask the owner (in Portuguese) instead
  of guessing. Technical defaults that follow CONTRIBUTING.md need no question.

## Agent skills

### Issue tracker

Issues and specs live in GitHub Issues (`gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

The five default labels: `needs-triage`, `needs-info`, `ready-for-agent`,
`ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` and `docs/adr/` at the repo root, created
lazily. See `docs/agents/domain.md`.

## Current state

M0 tracked in issue #5. Working end to end (2026-10-21): `mickey target` on
Windows, `mickey source` on Linux, AltGr + Right Shift switches the laptop
keyboard between them. Left: the touchpad as a mouse (#18, it sends absolute
positions), latency and the 10-minute check (#15). Decided: M0 is Linux →
Windows 10 with the hotkey AltGr + Right Shift (on release; the owner's
laptop has no Right Ctrl); M1a adds screen edges, 3+ PCs, encryption and a
configurable hotkey; M1b adds a setup GUI; peers are symmetric; the hotkey
stays as the way home after edges exist. Glossary in `CONTEXT.md`. Open for
the M1a grill: Ubuntu KDE on Wayland or X11, whether the layout places peers
or monitors, and the Wayland InputCapture portal instead of the `input`
group. Windows builds are cross-compiled from Arch and tested over SSH (see
PRD 0001 *Discoveries*). Never put the owner's IPs or usernames in the repo;
examples use `192.0.2.0/24`.
