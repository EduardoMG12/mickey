# 0009 — Windows: administrator mode only from Program Files, an admin-only trust list, redirection trust

- **Status:** Accepted
- **Date:** 2026-10-22 (elevated logon task, commit `d2f6229`);
  2026-10-24 (hardening: #183, PRs #247, #248, #249)

## Context

Windows refuses input that a normal process injects into a window running
as administrator (UIPI). With Mickey started normally, the keyboard could
not type or click in an admin window on that PC (PRD 0001, *Constraints*;
commit `d2f6229`). So the installer started Mickey at logon through a
scheduled task with the highest privileges.

The security review before going public (#172) found that this turned
Mickey into a way around UAC:

- **#183 (High for portable installs):** a hand-copied `C:\mickey` grants
  Authenticated Users *Modify*; any program could replace `mickey.exe` or
  `start-hidden.vbs` and run as administrator at the next logon, with no
  prompt. Checking only the folder's own access list is not enough either:
  whoever can change the folder *above* it can rename it away and put a
  new one in its place.
- **The trust list:** the admin Mickey read `%APPDATA%\mickey\trusted_peers`,
  which any user program can edit. A program could add its own key,
  connect to `127.0.0.1:4242` as a "paired PC", and have its keys typed
  into administrator windows.
- **Junctions:** the admin Mickey still writes into the user's folders
  (`control.txt`, received files, `config.toml`). A user program can turn
  one into a junction to a system folder, so the administrator overwrites
  system files.

## Decision

Keep administrator mode, and make each of those paths safe (PR #247 calls
this "option b: keep the administrator mode, make it safe"):

1. **Elevate only from inside Program Files** (#247), where nobody but
   administrators can rename anything, and only when the folder and files
   are admin-only by SID. The Program Files path comes from Windows
   (`SHGetKnownFolderPath`), not the user-settable `ProgramFiles` variable;
   both paths are canonicalized first. Unknown access-list entry kinds
   count as not safe. Otherwise `mickey autostart on` makes a task
   **without** administrator rights.
2. **An admin-only trust list** (#248): the admin Mickey keeps
   `trusted_peers` in `state` next to `mickey.exe`. It **starts empty**
   (the old list may already have been changed), so PCs pair again once.
   `mickey forget` without admin rights asks the running Mickey over the
   control channel: forgetting only removes trust, so any program asking
   is harmless. The pairing window runs as administrator too, so a normal
   program cannot click *Pair* in it (UIPI).
3. **Redirection trust** (#249): every `mickey` running as administrator
   turns on `ProcessRedirectionTrustPolicy` first thing in `main`, so
   opening a path through a junction a standard user made fails (error
   448). Where Windows cannot turn it on, the task is made without admin
   rights.
4. **Step down**: an admin Mickey found running from a folder others can
   change remakes its task without admin rights, starts it, and ends.

## Alternatives considered

- **Drop administrator mode** (presumably "option a"). **Inferred**: the
  docs name option b but do not write down option a or why it lost. The
  commit that added elevation gives the cost of dropping it: no typing or
  clicking in admin windows on that PC.
- **A Windows service.** Not chosen: hooks and `SendInput` work in the
  user's session, and a service runs in session 0, where `SendInput` fails
  with "Access denied" (PRD 0001, *Spike 0b*; PRD 0008, *Discoveries*).
- **Check only the folder's access list** (the first fix, `53198eb`).
  Not enough, because of the rename attack above; hence Program Files.
- **Copy the old trust list into `state`.** Rejected: it may already hold
  an attacker's key.

## Consequences

Good:

- The installed Mickey drives admin windows, and a standard user's program
  cannot use it to get administrator rights by swapping files, adding a
  key, or redirecting a write.
- Every fix was checked on the owner's Windows 10 22H2 (build 19045) with
  test folders (PRs #247-#249).

Bad, and known limits:

- **Two kinds of install behave differently**: only the installer's
  Program Files copy is elevated; a hand-copied one cannot reach admin
  windows.
- **Pairing again once** after upgrading to an admin-only list.
- **More Windows-only code**: access-list checks (`platform/acl.rs`) and a
  mitigation policy, all `unsafe` FFI.
- **Still out of reach:** UAC prompts, the lock screen and Ctrl+Alt+Del
  (the secure desktop); a `Refused` sends the focus home.
- **Not covered:** a program already running as administrator, or someone
  logged in as the owner reading the identity file.

## Links

- [PRD 0002](../prds/0002-secure-link.md), *Discoveries*, "What changed
  after PRD 0010" (last two bullets).
- [flows.md](../flows.md#pairing) (where the trust list lives).
- Issues #172, #183; PRs #247, #248, #249; commits `d2f6229`, `53198eb`.
- Code: `crates/mickey/src/autostart.rs`, `crates/mickey/src/net/trust.rs`
  (`dir`), `crates/mickey/src/platform/acl.rs`,
  `crates/mickey/src/main.rs`, `crates/mickey/src/run.rs`, `packaging/windows/mickey.nsi`.
