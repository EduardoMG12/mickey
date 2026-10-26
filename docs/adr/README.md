# Architecture decision records

Each file records one decision that shapes Mickey: the problem, what was
decided, what else was considered and why it lost, and what it costs. They
were written on 2026-10-26 from the PRDs, the code and the git history, so
the *Date* of each is when the decision was made, not when it was written
down.

Where a reason is not written anywhere and only the code or the context
suggests it, the record says **inferred**. Where no reason exists at all,
it says **not recorded**. Words such as *peer*, *source*, *target* and
*focus* mean what [CONTEXT.md](../../CONTEXT.md) says.

| # | Decision | Status |
|---|---|---|
| [0001](0001-rust-for-daemon-and-windows.md) | Rust for the whole project, daemon and windows alike | Accepted |
| [0002](0002-noise-xx-and-pairing-code.md) | Noise `XX` for every link, and a six-digit pairing code | Accepted |
| [0003](0003-tcp-links-udp-audio.md) | TCP for input and data, UDP sealed datagrams for audio | Accepted |
| [0004](0004-symmetric-peers.md) | Symmetric peers: every PC runs both the source and the target side | Accepted |
| [0005](0005-input-below-the-desktop.md) | Input below the desktop: evdev and uinput on Linux, hooks and SendInput on Windows | Accepted |
| [0006](0006-egui-in-a-separate-ui-process.md) | egui (eframe) for every window, in a separate `mickey-ui` program | Accepted |

## Writing a new one

- Name it `NNNN-short-title.md`, the next number, and add it to the table.
- Sections: *Status* and *Date*, *Context* (with numbers when there are
  any), *Decision*, *Alternatives considered* (each with why it lost),
  *Consequences* (good and bad, known limits included), *Links* (PRD
  sections, issues and PRs, source files).
- A decision is never edited away. When one is replaced, its status
  becomes *Superseded by NNNN* and the new record explains why.
- PRDs say what to build; their *Discoveries* say what building it
  taught. An ADR is the short answer to "why is it like this?".
