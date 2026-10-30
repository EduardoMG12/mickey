# 7. How it is built, tested and released

> **The question:** you push a tag `v0.2.1`. A little later there is
> a Windows installer, a Linux tarball and a checksum file on GitHub.
> What ran, on which machines, and what would have stopped it?

## Three crates, one workspace

[`Cargo.toml`](../../Cargo.toml) at the root lists them:

| Crate | Job | Binary |
|---|---|---|
| `crates/mickey` | input, network, OS: everything that runs all day | `mickey` |
| `crates/mickey-config` | `config.toml` and hotkey text, shared by both programs | (library) |
| `crates/mickey-ui` | every window, with egui | `mickey-ui` |

One detail in the root manifest: dependencies are optimized even in debug
builds (`[profile.dev.package."*"]`), because unoptimized ChaCha20 made a
test four times slower; Mickey's own code stays unoptimized so the
debugger shows every variable.

## Every dependency has a reason

Each one is justified in a comment next to it
([`crates/mickey/Cargo.toml`](../../crates/mickey/Cargo.toml)) and in the
PR that added it ([CLAUDE.md](../../CLAUDE.md), hard rule 10):

| Crate | Why not by hand |
|---|---|
| `snow` | Noise: cryptography is the one place "write it to learn" loses; only the four primitives used are enabled |
| `arboard`, `png` | The clipboard on Wayland, X11 and Windows; PNG for images |
| `ksni` (Linux) | The tray icon's D-Bus protocol, too large to write by hand |
| `wayland-client`, `wayland-protocols-wlr` (Linux) | Hearing every copy without polling; already in the tree via `arboard` |
| `libpulse-binding`, `libpulse-simple-binding` (Linux) | Sound through PulseAudio's library, which PipeWire answers too |
| `zbus` (Linux) | GPaste on GNOME; already in the tree via `ksni` |
| `wasapi`, `windows` (Windows) | WASAPI and the output's mute, COM interfaces easy to get wrong |
| `serde`, `toml` | `config.toml` |
| `eframe` | The windows (only in `mickey-ui`) |

What is **not** there is as telling: no `tokio` or other async runtime
(plain threads and channels), no `libc`, `evdev` or `windows-sys` (the OS
bindings in `platform/` are hand-written).

## Tests: about 210, and none needs a keyboard

`cargo test` runs the `#[test]` functions that sit next to the code
([CONTRIBUTING.md](../../CONTRIBUTING.md), *Tests and CI*):

- **Pure logic**: the protocol, the hotkey state machine, the jitter
  buffer, the keymap. Fast and exact.
- **Link tests**: both ends of a link in one process over a real
  localhost socket, with a closure as the fake emitter
  ([`net/transport.rs:314`](../../crates/mickey/src/net/transport.rs#L314),
  [`net/secure.rs:567`](../../crates/mickey/src/net/secure.rs#L567) for a tampered
  byte). CI never needs real devices.
- **By hand**: about ten tests are `#[ignore]` with the reason written in
  (they create a real input device, play a tone, mute the speakers). Run
  them with `cargo test -- --ignored <name>` on a real PC.
- **Real machines**: manual checks go in each PR description; the distro
  matrix runs in QEMU VMs ([`tools/distro-test/`](../../tools/distro-test/README.md));
  latency is measured with [`tools/latency-bench/`](../../tools/latency-bench/README.md).

## CI: three workflows

- [`ci.yml`](../../.github/workflows/ci.yml): on every PR and on `main`,
  on **Ubuntu and Windows**: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings` (a warning fails the
  build), `cargo test`. Must be green before the owner merges.
- [`audit.yml`](../../.github/workflows/audit.yml): `cargo audit` against
  the RustSec advisory database, on PRs, on `main`, and every Monday (a new
  advisory can appear with no change on our side).
- [`release.yml`](../../.github/workflows/release.yml): next section.

## The release, step by step

[`release.yml`](../../.github/workflows/release.yml) runs on a tag
`v*`:

1. **Linux job** ([`:16`](../../.github/workflows/release.yml#L16)):
   builds inside an `ubuntu:22.04` container, so the binaries need glibc
   2.35 at most, and [`:33`](../../.github/workflows/release.yml#L33)
   checks it with `objdump`. Packs `mickey`, `mickey-ui`, the installer
   scripts, the README, the license and the third-party licenses.
2. **Windows job** ([`:56`](../../.github/workflows/release.yml#L56)):
   builds, then refuses a tag that does not match the version in
   `Cargo.toml` ([`:72`](../../.github/workflows/release.yml#L72)), then
   NSIS makes `Mickey-Setup-<version>.exe`.
3. **Publish** ([`:84`](../../.github/workflows/release.yml#L84)): only on
   a tag, after both jobs: `SHA256SUMS`, then `gh release create`.

On a PR that touches `packaging/`, the same jobs build the installers
without publishing, so they can be tried first.

## What the installers do

**Linux**, [`install.sh`](../../packaging/linux/install.sh):

- checks that `mickey` can start (PulseAudio's client library);
- copies both programs to `~/.local/bin`;
- with `sudo`, once: the udev rule for `/dev/uinput`, the `input` group
  (and warns that every program in that group can read the keyboard);
- opens TCP 4242–4243 and UDP 4243–4244 if firewalld or ufw is on;
- a systemd user service, plus an autostart entry for desktops that never
  start `graphical-session.target`.

**Windows**, [`mickey.nsi`](../../packaging/windows/mickey.nsi): installs
to Program Files, runs `mickey autostart on` (a logon task, elevated only
when the install is safe, chapter 6), allows Mickey through the firewall
on **private** networks only, adds *Mickey settings* to the Start menu.
`start-hidden.vbs` starts it with no console window.

For day-to-day work the owner cross-compiles the Windows build from Arch
(`x86_64-pc-windows-gnu`) and tests it over SSH (README, *Build from
source*; PRD 0001, *Spike 0b*).

## Try it

```sh
cargo test -p mickey-config
cargo clippy --all-targets -- -D warnings
```

The first is fast (config parsing and hotkey text). The second is the
exact CI gate: if it passes here, CI's clippy step will too.

## Check yourself

1. Why is the Linux release built in an Ubuntu 22.04 container instead of
   the newest runner?
2. What stops a release whose tag says `v0.3.0` while `Cargo.toml` still
   says `0.2.1`?
3. Why can CI test the secure link and the key path with no keyboard?
4. Why does `cargo audit` also run on a schedule?

<details>
<summary>Answers</summary>

1. A binary needs the glibc it was built against or newer; building on
   22.04 sets the minimum at 2.35, and the workflow checks the binaries
   ask for nothing newer.
2. The Windows job compares the tag with `Cargo.toml` and fails, so the
   publish job (which needs both jobs) never runs.
3. Link tests run both ends over a real localhost socket and replace the
   emitter with a closure that records what arrives.
4. An advisory can be published for a crate already in `Cargo.lock`
   without any change in the repository.

</details>
