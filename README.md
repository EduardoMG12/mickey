# Mickey

**One keyboard and mouse for all your computers, written in Rust.**
Push the cursor off the edge of one screen and it appears on the next PC;
copy on one and paste on another. Linux and Windows, and any of them can be
the one whose keyboard you use. Inspired by Synergy, Deskflow/Input Leap and
lan-mouse, built from scratch to be understood line by line.

> **Status: alpha.** Used every day between an Arch Linux laptop (Hyprland)
> and a Windows PC, and tested on eight more Linux distros (below). Expect
> rough edges, and read the [known limits](#known-limits).

---

## What it does

- **Move between PCs** across the screen edges (on Windows and Hyprland so
  far), with a hotkey, or with the **switcher**: one card per PC, shown on
  the screen you are looking at.
- **Any PC drives any other.** Every PC runs the same `mickey`; the one
  whose keyboard and mouse you touch is the one in charge.
- **A shared clipboard that follows the keyboard.** Copy on one PC, move to
  another, paste: text, images and files (up to 60 MiB). The recent copies
  are kept in a searchable history, in memory only.
- **One pair of headphones for every PC.** Pick the PC your headphones are
  on, and every PC's sound plays there, mixed, each at its own volume,
  with a delay you will not notice (Linux with PipeWire or PulseAudio, and
  Windows).
- **Finds your PCs and pairs them with a code.** PCs running Mickey on the
  network show up in the settings: press **Connect**, check that both
  screens show the same six-digit code, press **Pair**. A PC paired with one
  of yours is trusted by the rest of the group.
- **Encrypted.** Everything crosses the network inside a Noise channel, and
  only between paired PCs.
- **A way home that always works.** Hold **Right Shift** for 3 seconds (or
  **AltGr + Right Shift** for 2) and let go: the keyboard and mouse come
  back to this PC, whatever the settings say.
- **A tray icon** that shows where the keyboard is (a green dot: here; red:
  on another PC) with a menu to move it; a settings window for the rest;
  it starts with your session.

## Install

Download the latest release from the repository's **Releases** page:

- **Windows:** run `Mickey-Setup-<version>.exe`. It installs to Program
  Files, starts Mickey at logon (as administrator, so it can also drive
  administrator windows), allows it through the firewall on **private**
  networks only and adds **Mickey settings** to the Start menu. Remove it
  from *Settings → Apps*.
- **Linux** (x86-64, glibc 2.35 or newer, systemd, and PulseAudio's
  client library, which every desktop has; `install.sh` says how to add
  it if not): unpack
  `mickey-linux-x86_64.tar.gz` and run `./install.sh`. It asks for your
  password once: the `input` group and a udev rule let Mickey read your
  keyboard and create its virtual one, and if firewalld or ufw is on it
  opens TCP 4242-4243 and UDP 4243-4244. Mickey goes to `~/.local/bin` and starts
  with your desktop session. If it had to add you to the `input` group,
  restart the PC once. `./uninstall.sh` removes everything.

Each release also has `SHA256SUMS`: `sha256sum -c SHA256SUMS --ignore-missing`
in the download folder checks what you got.

### Tested on

| System | Desktop | Notes |
|---|---|---|
| Arch Linux | Hyprland (Wayland) | the owner's laptop; screen edges work |
| Windows 10 | | screen edges work |
| Ubuntu 24.04 | GNOME (Wayland) | |
| Ubuntu 22.04 | GNOME | the oldest supported (glibc 2.35); install, start and discovery checked |
| Linux Mint 22.3 | Cinnamon (X11) | |
| Fedora 44 | KDE Plasma (Wayland) | |
| Debian 13 | GNOME (Wayland) | no tray icon without an extension ([why](#known-limits)) |
| Pop!_OS 24.04 | COSMIC (Wayland) | |
| Xubuntu 24.04 | XFCE (X11) | |
| openSUSE Tumbleweed | KDE Plasma (Wayland) | install, start, firewall and discovery checked |

On the other Linux distros: the installer, starting at login, the windows,
the tray, pairing, keys both ways and the clipboard both ways, in test
virtual machines ([`tools/distro-test/`](tools/distro-test/), results in #171).
Screen edges need Windows or Hyprland for now.

## First steps

1. **Install Mickey on every PC.**
2. On one of them, open **Mickey settings** (from the applications or Start
   menu, the tray, or `mickey settings`) and go to the **PCs** tab. The
   other PCs appear under **On this network**: press **Connect**.
3. Both PCs show a window with the **same six-digit code**. If it matches,
   press **Pair** on both. (A code that differs means something is in
   between: press **Refuse**.)
4. In the **Layout** tab, drag the new PC to where its screens sit next to
   this one's, and **Save**. Every PC learns the layout.
5. Push the cursor off that edge, or press **AltGr + Right Shift**.

A PC on another network does not show up in the list: use **Add by
address…**, and on that PC press **Let a new PC pair (2 min)**.

## Everyday use

### Hotkeys

| Keys | Does |
|---|---|
| **AltGr + Right Shift** | One other PC: go there, or come back. Two or more: open the switcher |
| **AltGr + Right Shift + 1** … **+ 9** | Straight to that PC (in the order of the PCs tab) |
| **AltGr + Right Shift + 0** | Back to this PC |
| **AltGr + Up** | Open the switcher |
| **AltGr + Right** / **AltGr + Left** | The next / previous PC, skipping the offline ones |
| **Right Shift** twice, quickly | Same as AltGr + Right |
| **Right Shift** held 3 s, or **AltGr + Right Shift** held 2 s | Home, always: no setting can take this away |

On keyboards without AltGr, it is the Right Alt key. A hotkey fires when
you let go and only if nothing else was pressed, so AltGr + Q and Shift + 1
still type. Change them in the **Hotkeys** tab (**Type** records the keys);
saved hotkeys go to every PC in the group, so all use the same ones.

### The switcher

One card per PC; pick one with the arrow keys and Enter, or a click; Esc
cancels. It opens on the PC you are looking at, and a view-only copy shows
on the others. Choose its corner in the settings (**General**).

### The clipboard and its history

Nothing crosses on every copy: when the keyboard moves from one PC to
another, what was copied on the first is put on the second. Received files
land in a fresh folder under Mickey's cache (`~/.cache/mickey/received`,
`%LOCALAPPDATA%\mickey\received`). **Clipboard history** (tray menu, or
`mickey history`) lists the last 50 copies, searchable; pick one to copy it
again. It lives in memory only and is gone when Mickey stops. Already
using a clipboard manager (cliphist, CopyQ, Klipper, GPaste, Windows' own
Win+V history)? Mickey notices when it starts and leaves the history to
it: the clipboard is still shared, and the manager sees everything Mickey
writes. To choose yourself, pick *Always* or *Never* in the settings
(**General**), or set `[clipboard] history = true` or `false`.

### Shared sound

Pick the PC your headphones are on: in the settings (**This PC → Where
are your headphones?**) on any PC, or **Listen here** in its tray. Every
PC learns it at once, and from then on every PC's sound plays there,
mixed with that PC's own, while each PC goes quiet. Each PC's volume and
mute are next to it in **PCs**, the same on every PC. Per PC you can also
keep its sound playing there too, or keep it at home (**When another PC
has the headphones, this PC's sound**). Pick **Nobody** and every PC uses
its own speakers again.

The sound crosses encrypted, only between paired PCs, in 5 ms pieces over
UDP 4244; nothing is sent while a PC plays nothing. On Windows Mickey
mutes the speakers while it sends (and unmutes them after, even if it
ended unexpectedly); on Linux it moves the sound to a silent output of its
own and puts the real one back when it stops, on Ctrl+C or
`systemctl --user stop mickey` too.

### The tray, commands and Waybar

The tray menu moves the keyboard to any PC and opens the settings and the
history. The same things work from a terminal, for launchers and
compositor binds (`mickey --help` lists them all):

```sh
mickey status          # where the keyboard is, and which PCs are online
mickey go Desktop      # send the keyboard and mouse to the PC named Desktop
mickey home            # bring them back
mickey switcher        # show the switcher
```

A Hyprland bind such as `bind = SUPER ALT, 1, exec, mickey go Desktop` only
reaches Mickey while the keyboard is on this PC; from another PC, use the
hotkeys.

**Waybar:** a module that changes the moment the keyboard moves; left click
opens the switcher, right click the settings. Its class is `local`,
`remote` or `off`, for the bar's CSS:

```jsonc
"custom/mickey": {
  "exec": "mickey status --waybar",
  "return-type": "json",
  "on-click": "mickey switcher",
  "on-click-right": "mickey settings"
}
```

### Settings by hand

The settings window writes `config.toml` for you
(`~/.config/mickey/config.toml`; `%APPDATA%\mickey\config.toml` on Windows),
and follows it when another PC rewrites it. Editing it by hand works too;
restart Mickey afterwards, or run `mickey arranged` after changing positions.

### When something goes wrong

Mickey's log says what it is doing (never which keys you press):
`journalctl --user -u mickey` on Linux, and
`%LOCALAPPDATA%\mickey\mickey.log` on Windows (the previous run's is
`mickey.log.old`). Add it to an issue when you report a problem.

## Known limits

- **Screen edges only on Windows and Hyprland.** Elsewhere, use the hotkeys
  and the switcher; native GNOME and KDE support is planned (#178).
- **The switcher on GNOME (Wayland)** opens without a title bar, but where
  GNOME puts it, not in the chosen corner: GNOME lets no program place a
  window. On KDE Plasma and Hyprland it goes to its corner.
- **Touchpad:** one-finger motion and the physical click work; tap to click
  and two-finger scroll do not yet. A wheel mouse scrolls.
- **Windows' secure screens** (lock screen, UAC prompt, `Ctrl+Alt+Del`)
  take no input from any program; if a PC refuses the input, the keyboard
  comes home.
- **Keys are sent by position**, so give both PCs the same keyboard layout
  (e.g. ABNT2).
- **No tray icon on stock GNOME** (Debian, Fedora Workstation): GNOME has
  no area for status icons of its own. Ubuntu ships the *AppIndicator and
  KStatusNotifierItem Support* extension switched on; elsewhere install
  `gnome-shell-extension-appindicator` (Debian and Fedora name it the same),
  turn it on in *Extensions* and log in again. Mickey works without it:
  `mickey status` says where the keyboard is, and `mickey settings` opens
  the settings window.
- **The clipboard on GNOME (Wayland)** only sees what X11 apps copy: GNOME
  offers other programs no way to follow the clipboard. With **GPaste** and
  its GNOME Shell extension on (`gpaste-2` and
  `gnome-shell-extension-gpaste` on Debian and Ubuntu, then log in again),
  Mickey reads the copied text through it, from every app.
- **The `input` group** lets every program you run read the keyboard, not
  only Mickey; desktop portals are the long-term fix (#178).
- **Local network only:** no relay over the internet.
- **Windows Defender may block the installer** as
  `Trojan:Win32/Bearfoos.B!ml`. The `!ml` means its machine-learning guess,
  not a known virus: a new program, not code-signed yet, that reads the
  keyboard and talks over the network looks like a keylogger to it. Check
  the download against `SHA256SUMS`, then in *Windows Security → Virus &
  threat protection → Protection history* open the item and choose
  *Allow on device*. Signing is planned.
- **Shared sound** is stereo and local-network only; microphones and
  cameras do not cross (they would need a virtual device, on Windows a
  driver). Linux needs PipeWire or PulseAudio, which every current
  desktop has.

## Security

Everything you type crosses the network, so:

- **Encrypted and authenticated.** Everything travels inside a Noise `XX`
  channel (X25519, ChaCha20-Poly1305, BLAKE2s): it cannot be read, changed
  or replayed.
- **Paired PCs only.** Each PC has its own key (`mickey id` shows its
  fingerprint). The six-digit code both pairing windows show comes from the
  encrypted handshake itself: if both show the same code, no one is in
  between. Trust is by key, never by address; reconnects never ask again.
  `mickey forget <fingerprint>` removes a PC.
- **The group:** when one of your paired PCs introduces another PC, Mickey
  asks you once whether to let it in; nothing is trusted on another PC's
  word alone.
- **Never logged or stored:** what you type, and what you copy (only its
  kind and size).
- **Not covered:** someone who can already log in as you (they can read the
  key files, as with SSH keys), and someone on the network who just wants
  to break the connection.

Found a problem? See [SECURITY.md](SECURITY.md).

## Privacy

Mickey has no telemetry and no accounts. Its one request to the internet
is the update check (PRD 0013): once a day it asks GitHub which Mickey
release is the latest. That request carries nothing about you, only what
any HTTPS request does (your IP and `mickey/<version>`), and `[updates]
check = false` in `config.toml` turns it off. Everything else stays
between the PCs on your network that run Mickey: it announces this PC's
name and key so they can find it, and, once you pair them, sends them
your keyboard and mouse while they have the focus and your clipboard
when it moves there, all encrypted. Typed keys are never logged or
written to disk. Clipboard history and received files stay on your PCs.

## Code signing policy

The Windows releases are meant to be signed (free code signing for
open source by [SignPath.io](https://about.signpath.io), certificate by
[SignPath Foundation](https://signpath.org)); until then they are not.

- **Who builds:** only the release workflow
  ([`.github/workflows/release.yml`](.github/workflows/release.yml)),
  from a version tag on `main`, with `Cargo.lock`.
- **Who reviews:** every change reaches `main` through a pull request;
  pull requests from contributors are reviewed by the maintainer.
- **Who approves a release:** the maintainer,
  [@EduardoMG12](https://github.com/EduardoMG12), who also approves each
  signing request.
- **Checking a download:** every release has `SHA256SUMS`, signed with
  the release key in
  [`packaging/release-key.pub`](packaging/release-key.pub)
  (`SHA256SUMS.sig`). Run `tools/release-key/verify.sh SHA256SUMS`, then
  `sha256sum -c SHA256SUMS`. Mickey makes the same two checks before it
  installs an update.

## Build from source

Needs [Rust](https://rustup.rs), and on Linux PulseAudio's headers for
shared audio (`libpulse-dev` on Debian and Ubuntu, `pulseaudio-libs-devel`
on Fedora; Arch's `libpulse` has them). On Linux you can build both; the
Windows programs are cross-compiled:

```sh
cargo build --release                                   # Linux: target/release/mickey and mickey-ui
rustup target add x86_64-pc-windows-gnu                 # once
sudo pacman -S mingw-w64-gcc                            # once (Arch; other distros: mingw-w64)
cargo build --release --target x86_64-pc-windows-gnu    # Windows: target/x86_64-pc-windows-gnu/release/
```

`mickey-ui` (the windows) must sit next to `mickey`. `cargo test` runs the
tests; [CONTRIBUTING.md](CONTRIBUTING.md) has the rest.

## Roadmap

| Next | What you get |
|---|---|
| **Any desktop** | Native GNOME and KDE support through desktop portals: screen edges everywhere, no `input` group (#178) |
| **Packages** | AUR, `.deb`, `.rpm`, winget, Scoop (#176) |
| **macOS** | A first look at whether Mickey can run there (#177) |
| Someday | Sharing headphones and other USB devices, remote control over the internet |

## Architecture at a glance

```
┌─────────────── mickey on the machine with the keyboard ───────────────┐
│  capture backend ──► core: layout, focus, pressed keys ──► transport  │
└───────────────────────────────────────────────────────────────┬───────┘
                                                                │ LAN
┌─────────────── mickey on the controlled machine ──────────────▼───────┐
│  transport ──► core ──► emit backend (uinput on Linux, SendInput)     │
└───────────────────────────────────────────────────────────────────────┘
```

- **Capture** reads the physical keyboard and mouse. **Emit** creates input on
  the controlled machine. Each is per-platform; everything else is shared.
- **All `unsafe` code is confined to the platform modules**; the core and the
  protocol are safe Rust.

Full design: [docs/prds/0000-vision.md](docs/prds/0000-vision.md). To learn
the code, read [docs/architecture.md](docs/architecture.md) (crates, threads,
the platform layer), [docs/protocol.md](docs/protocol.md) (what crosses the
network) and [docs/flows.md](docs/flows.md) (pairing, focus and edges, the
switcher, the clipboard), each with diagrams.

## How this project is developed

Mickey is a learning project, and its process is part of the point:

- Every PR is small (≤ ~300 lines) and carries a **review challenge**,
  questions only someone who actually read (and sometimes debugged) the diff
  can answer.
- When a classic algorithm or data structure shows up (state machine, graph,
  set, framing a byte stream, …), the PR explains the concept and the matching
  LeetCode problem.
- Only the owner merges.

Details: [CONTRIBUTING.md](CONTRIBUTING.md). Rules for AI assistants:
[CLAUDE.md](CLAUDE.md).

## License

[GPL-3.0-or-later](LICENSE). The releases also carry `THIRD-PARTY-LICENSES.txt`,
the licenses of the crates built into Mickey
([`tools/licenses`](tools/licenses/third_party_licenses.py) makes it).
