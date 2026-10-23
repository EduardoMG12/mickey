# Mickey

**One keyboard and mouse for all your computers, written in Rust.**
Move the cursor off the edge of one screen and it appears on the next
computer, whatever its distro, desktop or operating system.
Inspired by Synergy, Deskflow/Input Leap and lan-mouse, built from scratch to
be understood line by line.

> **Status: pre-alpha.** Milestone M0 works: a Linux laptop's keyboard and
> touchpad drive a Windows 10 PC, switched with a hotkey, over an encrypted
> link between paired machines (see [Security](#security)). The plan lives in [`docs/prds/`](docs/prds/).

---

## What it will be

- **One binary, every machine.** The same `mickey` runs on each computer. You
  tell it where the other screens are (left, right, above, below).
- **Works on any desktop.** X11, GNOME, KDE, Hyprland, Sway: on Linux the
  hard part is the compositor, not the distro, and Mickey has a backend
  for each, plus a kernel-level fallback (`evdev`/`uinput`) that works
  everywhere. Windows too, from the first milestone.
- **Feels local.** Moving the mouse on a remote screen should add no latency
  you can notice. Hard budgets, measured.
- **Safe on a shared network.** Everything you type crosses the network, so
  traffic is encrypted and machines must be paired before they can talk
  (from M1a on).

## Install

Download the latest release from the repository's **Releases** page:

- **Windows:** run `Mickey-Setup-<version>.exe`. Mickey starts with Windows
  (no window), is allowed through the firewall on **private** networks only,
  and can be removed from *Settings → Apps*.
- **Linux** (x86-64, glibc 2.39 or newer with systemd: Ubuntu 24.04, Linux
  Mint 22, Fedora 40, Debian 13 or newer; tested on the first three):
  unpack `mickey-linux-x86_64.tar.gz` and run `./install.sh`. It
  asks for your password once (input group and a udev rule for the virtual
  keyboard), installs to `~/.local/bin` and starts Mickey with your desktop
  session (`systemctl --user status mickey`). `./uninstall.sh` removes it.

Each release also has `SHA256SUMS`: `sha256sum -c SHA256SUMS --ignore-missing`
in the download folder checks what you got.

Then open **Mickey settings** (from the applications or Start menu, or
`mickey settings`) to add your other PCs, choose what this PC does and where
the switcher appears; it writes `config.toml` for you. Pair new PCs: the first
time two PCs meet, run Mickey in a terminal on both (on Windows: Start menu
→ *Mickey (terminal, for pairing)*) and confirm the fingerprints.

## Try it from source

Mickey is a command-line program today; a setup window comes later (M1b).

### Build

Needs [Rust](https://rustup.rs). On Linux you can build both sides; the
Windows `.exe` is cross-compiled:

```sh
cargo build --release                                   # Linux: target/release/mickey
rustup target add x86_64-pc-windows-gnu                 # once
sudo pacman -S mingw-w64-gcc                            # once (Arch; other distros: mingw-w64)
cargo build --release --target x86_64-pc-windows-gnu    # Windows: target/x86_64-pc-windows-gnu/release/mickey.exe
```

### Run

On **Windows**, the machine to be controlled (allow the firewall prompt on
**private networks only**):

```sh
mickey.exe target --listen 192.0.2.10:4242      # this PC's own LAN address
```

On **Linux**, the machine with the keyboard. Reading input devices needs the
`input` group, once:

```sh
sudo usermod -aG input $USER                    # then log out and in again
mickey source --to 192.0.2.10:4242              # the Windows PC's address
```

Instead of `--to`, you can list your PCs in `~/.config/mickey/config.toml`
(`%APPDATA%\mickey\config.toml` on Windows) and run `mickey source`:

```toml
[[peer]]
name = "Desktop"
address = "192.0.2.10:4242"

[touchpad]
speed = 16      # higher is faster (1-200)
```

Press and release **AltGr + Right Shift** (Right Alt + Right Shift on
keyboards without AltGr: it is the same key) to send the keyboard and
touchpad (or mouse) to Windows; press it again to bring them back. To jump
straight to a PC, add its number: **AltGr + Right Shift + 1** goes to the
first PC in `config.toml`, **+ 2** to the second, and **+ 0** comes home.
All of them can be changed in the settings window. `mickey --help`
lists the commands.

With **two or more PCs** in `config.toml`, the hotkey opens the **switcher**:
one card per PC; pick one with the arrow keys and Enter, or click it; Esc
cancels. From another PC, the hotkey brings you home and opens it. The
switcher is a separate program, `mickey-ui`, which must sit next to `mickey`
(`cargo build --release` builds both).

`mickey status` shows where the focus is and which PCs are online.

**Waybar.** Add a custom module (and put `"custom/mickey"` in one of the
`modules-*` lists). It updates the moment the focus changes; left click
opens the switcher, right click will open the settings:

```jsonc
"custom/mickey": {
  "exec": "mickey status --waybar",
  "return-type": "json",
  "on-click": "mickey switcher",
  "on-click-right": "mickey settings"
}
```

The module's class is `local`, `remote` or `off`, so the bar can show at a
glance where you are typing:

```css
#custom-mickey.remote { color: #f5a97f; }
#custom-mickey.off    { opacity: 0.5; }
```

**Commands.** While `mickey source` runs, `mickey go <name>` sends the
keyboard and mouse to a PC from `config.toml`, `mickey home` brings them
back and `mickey switcher` shows the switcher. They are meant for bars,
launchers and compositor binds, e.g. on Hyprland (classic syntax):

```ini
bind = SUPER ALT, 1, exec, mickey go Windows
```

Compositor binds only reach Mickey while the focus is on this PC; to come
back from another PC, use Mickey's hotkey.

The switcher appears in the **bottom-right corner** by default; choose
another place in `config.toml`:

```toml
[interface]
switcher_position = "bottom-right"   # or center, top-left, top-right, bottom-left
```

On Windows and X11 the window moves itself there. On Hyprland, `mickey-ui`
adds the window rule itself (`hyprctl eval`), so nothing needs configuring.
Other Wayland desktops place it themselves (usually centered) for now.

On older Hyprland versions without `hyprctl eval`, the switcher tiles like
any window unless a rule makes it float. Lua config (Hyprland 0.56+):

```lua
hl.window_rule({ name = "mickey-ui", match = { class = "^(mickey-ui)$" },
                 float = true, center = true, pin = true })
```

Classic `hyprland.conf`:

```ini
windowrulev2 = float, class:^(mickey-ui)$
windowrulev2 = center, class:^(mickey-ui)$
windowrulev2 = pin, class:^(mickey-ui)$
```

### Known limits

- **Hotkey only**, no screen edges yet (M1a). The hotkey is fixed.
- **Touchpad:** one-finger motion and the physical click work; tap to click
  and two-finger scroll do not yet. The speed is set in `config.toml`.
- **Windows won't take input** in the lock screen, the UAC prompt,
  `Ctrl+Alt+Del` or windows running as administrator. That is a Windows rule.
- **The keyboard layout is Windows'**: keys are sent by position, so set
  Windows to the same layout as your keyboard (e.g. ABNT2).
- If something goes wrong while your keyboard is on Windows, the hotkey
  still works; if Mickey itself dies, Linux gets the keyboard back at once.

## Security

Everything you type crosses the network, so Mickey protects it:

- **Encrypted and authenticated.** Everything travels inside a Noise `XX`
  channel (X25519, ChaCha20-Poly1305, BLAKE2s): it cannot be read, changed
  or replayed.
- **Paired machines only.** Each machine has its own key (`mickey id` shows
  its fingerprint). The first time two machines meet, **both** screens show
  both fingerprints:

  ```text
  mickey: new peer at 192.0.2.10:4242
  mickey:   this PC:   NRMP-6SSY-5DJZ-2PXV-ZELA-PCWY-EWPC-BF43
  mickey:   other PC:  E6AK-SYJ6-CDML-5HAY-445U-DABI-3N4X-BYZP
  ```

  Check that "other PC" on one screen matches "this PC" on the other, then
  type `y` on both. Until both say yes, the target emits nothing and the
  source sends nothing. After that they connect without asking. A machine
  with any other key is asked about again, never trusted silently.
  `mickey forget <fingerprint>` removes a paired machine.
- **Never logged or stored:** what you type. Input goes only to the address
  you give, and only if that machine holds the paired key.
- **Not covered:** someone who can already log in as you (they can read the
  key files, as with SSH keys), and someone on the network who just wants to
  break the connection.

## Roadmap

| Milestone | What you get | Status |
|---|---|---|
| **M0: It crosses** | Press a hotkey, and a Linux PC's keyboard and mouse drive a Windows PC; press it again to come back ([PRD](docs/prds/0001-m0-hotkey-switch.md)) | working; final check pending |
| **M1a: Screens have edges** | 3+ PCs in a layout: the cursor leaves your screen and enters the neighbor you configured, and back; Linux targets; encryption and pairing | planned |
| **M1b: Point and click setup** | A window that draws each PC's monitors so you can arrange them with the mouse | planned |
| **M2: Any desktop** | Native backends: X11, Wayland portals (GNOME/KDE), wlroots; Windows as the source | planned |
| Later | Shared clipboard, file drag and drop, automatic discovery (mDNS), tray icon, macOS | ideas |
| Someday | Sharing headphones and other USB peripherals, TeamViewer-style remote control | very long term |

## Architecture at a glance

```
┌─────────────── mickey on the machine with the keyboard ───────────────┐
│  capture backend ──► core: layout, focus, pressed keys ──► transport  │
└───────────────────────────────────────────────────────────────┬───────┘
                                                                │ LAN
┌─────────────── mickey on the controlled machine ──────────────▼───────┐
│  transport ──► core ──► emit backend (uinput, XTest, SendInput, …)    │
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

[GPL-3.0-or-later](LICENSE).
