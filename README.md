# Mickey

**One keyboard and mouse for all your computers, written in Rust.**
Move the cursor off the edge of one screen and it appears on the next
computer, whatever its distro, desktop or operating system.
Inspired by Synergy, Deskflow/Input Leap and lan-mouse, built from scratch to
be understood line by line.

> **Status: pre-alpha.** Milestone M0 works: a Linux laptop's keyboard and
> touchpad drive a Windows 10 PC, switched with a hotkey. **Not safe on an
> untrusted network yet**: traffic is encrypted, but the target still accepts
> any machine until pairing lands (see [Security](#security)). The plan lives in [`docs/prds/`](docs/prds/).

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

## Try it (M0: Linux → Windows)

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

Press and release **AltGr + Right Shift** to send the keyboard and touchpad
(or mouse) to Windows; press it again to bring them back. `mickey --help`
lists the commands.

### Known limits

- **Hotkey only**, no screen edges yet (M1a). The hotkey is fixed.
- **Touchpad:** one-finger motion and the physical click work; tap to click
  and two-finger scroll do not yet. The speed is fixed.
- **Windows won't take input** in the lock screen, the UAC prompt,
  `Ctrl+Alt+Del` or windows running as administrator. That is a Windows rule.
- **The keyboard layout is Windows'**: keys are sent by position, so set
  Windows to the same layout as your keyboard (e.g. ABNT2).
- If something goes wrong while your keyboard is on Windows, the hotkey
  still works; if Mickey itself dies, Linux gets the keyboard back at once.

## Security

Everything you type crosses the network, so read this before using M0:

- **Encrypted.** Everything crosses the network inside a Noise `XX` channel
  (X25519, ChaCha20-Poly1305, BLAKE2s): it cannot be read, changed or
  replayed. Each machine has its own key; `mickey id` shows its fingerprint.
- **No pairing yet.** Both sides print the other's fingerprint, but neither
  refuses an unknown one, so anyone who can reach the target's port can type
  on that Windows PC, and a fake target could receive your typing. Only run
  Mickey on a network you trust until pairing lands.
  ([#31](https://github.com/EduardoMG12/mickey/issues/31),
  [#37](https://github.com/EduardoMG12/mickey/issues/37))
- Mickey never logs or stores what you type, and sends input only to the
  address you give it.

Pairing (both screens confirm the fingerprints) comes next, in [#37](https://github.com/EduardoMG12/mickey/issues/37).

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

Full design: [docs/prds/0000-vision.md](docs/prds/0000-vision.md).

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
