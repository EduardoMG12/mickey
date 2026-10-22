# Security

Mickey carries every key you press from one PC to another, so security
problems matter more here than in most projects.

## Reporting a problem

**Do not open a public issue.** Report it privately through GitHub:
*Security → Report a vulnerability* in this repository
(<https://github.com/EduardoMG12/mickey/security/advisories/new>).

Please include what an attacker needs (on the same network? paired? a
program on the same PC?), what they get, and how to reproduce it. You will
get an answer within a week; fixes for confirmed problems come before any
new feature.

## What Mickey promises

- Input and clipboard travel only to PCs you paired, over an encrypted
  link (Noise `XX`). A PC you never paired gets nothing past the handshake.
- Pairing needs the same six-digit code on both screens.
- Mickey never logs the keys you type or what you copy, and never writes
  them to disk. The clipboard history lives in memory only.
- A way home always exists: **hold Right Shift alone for 3 seconds** (or
  with AltGr for 2) and let go, and the keyboard and mouse come back,
  whatever is configured.

How it works in detail: [PRD 0002](docs/prds/0002-secure-link.md) (the
link), [PRD 0010](docs/prds/0010-easy-pairing.md) (pairing and group trust)
and [PRD 0011](docs/prds/0011-shared-clipboard.md) (the clipboard).

## Known limits

- On Linux, Mickey reads input devices through the `input` group. Every
  program of a user in that group can read the keyboard, so keep it to
  your own account on your own PCs. Desktop portals will replace this
  (issue #178).
- A paired PC is trusted, and so are the PCs it introduces to the group.
  Pair only PCs you control.

## Supported versions

Only the latest release gets security fixes.
