# 6. Safety nets

> **The question:** Mickey has grabbed the laptop's keyboard and sent it to
> the Windows PC. Now the Windows PC freezes. Or the Wi-Fi drops. Or
> Mickey panics. How do you get your own laptop back, and who makes sure
> you always can?

A tool that grabs your keyboard is one bug away from locking you out of
your own desktop. [CLAUDE.md](../../CLAUDE.md) hard rule 8: *never leave
the owner's machine without a way back*. This chapter collects every net.

## Net 1: the grab is a guard (`platform/linux.rs`, `input.rs`)

- [`linux.rs:136`](../../crates/mickey/src/platform/linux.rs#L136) `Grab`
  and its `Drop` ([`:142`](../../crates/mickey/src/platform/linux.rs#L142)):
  releasing is not a call you can forget, it happens when the value goes
  away. The source holds the grabs in one `Option`
  ([`source.rs:730`](../../crates/mickey/src/source.rs#L730)); setting it
  to `None`, returning, `?` and a panic's unwinding all drop it.
- If the process is killed outright, the kernel drops the grab when the
  device file closes (PRD 0001, *Spike 0a*).
- **Windows:** [`input.rs:115`](../../crates/mickey/src/input.rs#L115)
  `Grabbed::drop` stops swallowing. Beyond that, Windows itself removes the
  hooks when the process ends, skips a hook that stops answering
  (`LowLevelHooksTimeout`), and never routes Ctrl+Alt+Del through hooks
  ([`hooks.rs:1`](../../crates/mickey/src/platform/hooks.rs#L1)).

This is RAII: tie a resource to a value's lifetime, and the compiler
writes the cleanup on every path.

## Net 2: the escape no setting can remove (`hotkey.rs`)

Hold **Right Shift** alone for 3 s, or with **AltGr** for 2 s, press
nothing else, let go: the focus comes home
([`hotkey.rs:63`](../../crates/mickey/src/hotkey.rs#L63)).

- [`hotkey.rs:75`](../../crates/mickey/src/hotkey.rs#L75) `Escape` is
  watched **apart from** the configured hotkeys, and
  [`feed_at`](../../crates/mickey/src/hotkey.rs#L209) checks it first. A
  broken or shared hotkey setting cannot take it away.
- Every keyboard has a Right Shift, even one without AltGr.

## Net 3: a peer that goes quiet loses the focus (`source.rs`, `transport.rs`)

- [`source.rs:45`](../../crates/mickey/src/source.rs#L45) `SILENT_LIMIT`:
  4 s without a `Pong`, and [`:905`](../../crates/mickey/src/source.rs#L905)
  marks the peer lost.
- [`transport.rs:17`](../../crates/mickey/src/transport.rs#L17)
  `WRITE_TIMEOUT`: a send that cannot finish in 1 s fails; a failed send
  also marks the peer lost.
- [`source.rs:1355`](../../crates/mickey/src/source.rs#L1355): for a lost
  peer, [`focus.rs:187`](../../crates/mickey/src/focus.rs#L187)
  `peer_lost` drops the grab and the focus is `Local` again. A reconnect
  starts in the background.

## Net 4: nothing stays pressed (`focus.rs`, `transport.rs`)

- Going home, [`focus.rs:146`](../../crates/mickey/src/focus.rs#L146)
  `go_home` first sends a release for every key and button the peer
  holds because of us, then ungrabs.
- On the target, whatever the link ends with, cleanly or not, `serve`
  releases what is still held
  ([`transport.rs:218`](../../crates/mickey/src/transport.rs#L218)).

## Net 5: the target can say no (`transport.rs`, `source.rs`)

Windows will not let a normal program type into a window running as
administrator. When `SendInput` fails, the target sends `Refused` (at
most once a second), and
[`source.rs:808`](../../crates/mickey/src/source.rs#L808) brings the
focus home: your keys never vanish into a window that ignores them.

And the boring nets: **This PC** in the tray menu, and `mickey home`
([`source.rs:92`](../../crates/mickey/src/source.rs#L92)).

## Net 6: signals give the sound back (`platform/signals.rs`)

The grab needs no signal handling, the kernel cleans it up. The null sink
from chapter 5 does: the sound server keeps it after Mickey is gone.

- [`main.rs:326`](../../crates/mickey/src/main.rs#L326): at every start,
  undo what a previous Mickey left (`speakers::clean_up`).
- [`signals.rs:47`](../../crates/mickey/src/platform/signals.rs#L47)
  `on_stop`: block SIGTERM, SIGINT and SIGHUP, and wait for them on one
  thread with `sigwait`. Cleanup runs on an ordinary thread, which may
  talk to the sound server; a signal handler may not.

## Net 7: running as administrator, safely (Windows)

To type into administrator windows, Mickey must itself run elevated
(the installer's logon task does). That makes it a target:

- Elevated only when installed where only administrators can change it:
  [`autostart.rs:219`](../../crates/mickey/src/autostart.rs#L219)
  `install_is_safe`. Otherwise it steps down
  ([`main.rs:337`](../../crates/mickey/src/main.rs#L337)).
- The trusted list moves to `state` next to `mickey.exe`
  ([`trust.rs:30`](../../crates/mickey/src/trust.rs#L30)): in `%APPDATA%`
  any program of the user could add its own key and type into
  administrator windows.
- [`acl.rs:118`](../../crates/mickey/src/platform/acl.rs#L118)
  `refuse_user_junctions`, first thing in `main`: writing through a
  junction a standard user made would let them overwrite system files.

## Net 8: what you type is never written down

Hard rule 7. Key codes and typed text never reach a log or a file:

- no `println!` prints a key code; even the Windows log file says so
  ([`log_file.rs:1`](../../crates/mickey/src/platform/log_file.rs#L1));
- clipboard content prints as its kind and size only
  ([`clipboard.rs:25`](../../crates/mickey/src/clipboard.rs#L25));
- the history is memory only; the hotkey recorder names chords but
  refuses plain typing ([`record.rs:38`](../../crates/mickey/src/record.rs#L38)).

And every byte from the network is untrusted: `decode` never panics, a
Noise message over 256 bytes is refused on the input link, the handshake
must finish in 5 s, and each port takes at most 32 connections
([protocol.md](../protocol.md), *The Noise handshake*).

## Try it

```sh
cargo test -p mickey holding_altgr_and_right_shift_comes_home_whatever_is_set
cargo test -p mickey a_target_that_stops_reading_makes_send_fail
```

Read both: [`hotkey.rs:441`](../../crates/mickey/src/hotkey.rs#L441) and
[`transport.rs:522`](../../crates/mickey/src/transport.rs#L522). The
second sends motion to a target that never reads until the socket
buffers fill and a send fails: that is `WRITE_TIMEOUT` at work.

## Check yourself

1. Mickey panics while the focus is remote on Linux. Name two separate
   things that give the keyboard back.
2. Why is the escape checked apart from the configured hotkeys?
3. The Windows PC freezes. How long until the laptop's keyboard is back,
   and what triggers it?
4. Why does `on_stop` use `sigwait` on a thread instead of a signal
   handler?
5. Why must the administrator Mickey not read `trusted_peers` from
   `%APPDATA%`?

<details>
<summary>Answers</summary>

1. Unwinding drops the `Grab` guards (their `Drop` calls `EVIOCGRAB` off);
   and when the process ends, the kernel releases the grab as the device
   files close.
2. So no setting, shared from another PC or simply broken, can remove the
   way home.
3. At most about 4 s: the pings get no `Pong` for `SILENT_LIMIT`, or
   sooner if a send fails after the 1 s write timeout; the peer is lost,
   `peer_lost` ungrabs.
4. The cleanup talks to the sound server, which is not safe inside a
   signal handler; an ordinary thread waiting in `sigwait` may do
   anything.
5. Any program running as the user can write there and add its own key;
   the elevated Mickey would then type what that key sends into
   administrator windows, getting round UAC.

</details>
