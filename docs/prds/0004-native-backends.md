# 0004 — Native backends: GNOME and KDE

> Status: **draft** · Milestone M2 · Depends on: [0005](0005-screen-edges.md) (edges) · Vision: [0000](0000-vision.md)

## Goals

Outside Hyprland, Mickey cannot read the cursor or the monitors, so screen
edges and placing the cursor fail; the hotkeys and the switcher still
work. GNOME (Ubuntu's and Fedora's default) and KDE Plasma are where most
Linux users are. Decided with the owner on 2026-10-27:

- **GNOME first**, then KDE on the same path (both speak the same portals).
- **Receiving first**: a GNOME PC controlled by another (the owner's
  Windows PC drives): it reports its monitors, the cursor enters where it
  should, and pushing past its edge goes back. Driving from a GNOME PC
  (the InputCapture portal) is the next phase.
- **Wayland first**; X11 (XInput2, XTest) is a later phase.
- **Tested in VMs**: Ubuntu 24.04 (GNOME 46) and Fedora 44 (KDE),
  `tools/distro-test`.

## Approach

- **The RemoteDesktop portal with libei.** Mickey asks
  `org.freedesktop.portal.RemoteDesktop` for a session with the pointer
  and the keyboard, then `ConnectToEIS`, and speaks libei on the socket it
  gets. The person allows it once; the portal's restore token
  (`persist_mode = 2`) keeps the permission across restarts.
- **Monitors from libei.** The absolute pointer device lists its
  *regions*: one rectangle per monitor, in the desktop's coordinates. They
  become `Monitors` for the source, as Hyprland's answer does today.
- **Placing the cursor**: an absolute motion to the entry point.
- **Knowing where the cursor is, without asking.** While another PC
  controls this one, only Mickey moves the cursor. So Mickey keeps the
  position itself: it starts where the cursor entered, adds each motion,
  keeps it inside the regions, and moves the cursor there with absolute
  motion. That position feeds the edge check (`edges::beyond`), so
  leaving through an edge needs no question to the compositor. The
  compositor's pointer acceleration does not apply to absolute motion, so
  the two never drift apart.
- **Choosing the backend**: Hyprland's socket when
  `HYPRLAND_INSTANCE_SIGNATURE` is set, the portal otherwise.
- **Crates**: `ashpd` for the portal (it uses `zbus` 5, already in the tree
  through `ksni`) and `reis` for libei (pure Rust). Both justified in
  their PRs.

### Phase 2: driving from GNOME or KDE

Asked by the owner on 2026-10-27: every PC must be able to drive every
other, GNOME, KDE and Windows alike.

- **Reading the keyboard and mouse already works on any desktop** (evdev,
  the `input` group). What a GNOME or KDE PC lacks is knowing when its
  cursor is pushed against an edge that leads to a placed PC.
- **The InputCapture portal's pointer barriers** say so: one barrier on
  each edge of this PC's monitors that a placed peer touches. When the
  cursor pushes into one, the portal reports where; Mickey releases the
  capture at once, a few pixels inside, and switches the focus the way it
  does today, through evdev. Mickey never reads input through libei in
  this phase, so nothing about input changes.
- **Only when a peer is placed beside this PC**: on GNOME 46 the
  InputCapture dialog has no "remember", so it would come at every login;
  with no edge to watch, it never comes.
- **Coming back**: the target says where the cursor left it; the source
  places its own cursor there with the RemoteDesktop portal (phase 1).

## Requirements

- Every grab and every libei device is released on exit, panic and
  disconnect (hard rule 8).
- No input is logged above `trace` (hard rule 7).
- The portal's permission dialog appears once, from the settings window,
  never in the middle of a switch.
- Hyprland keeps its current backend, unchanged.

## Non-Goals (this phase)

- Driving from a GNOME or KDE PC (InputCapture): the next phase.
- X11, wlroots compositors other than Hyprland.
- GNOME older than 46, KDE older than 6.1 (no `ConnectToEIS`).

## Risks, to spike first

- **Does GNOME 46's RemoteDesktop portal give an EIS socket, with an
  absolute pointer whose regions match the monitors, and move the cursor
  to an exact point?** Spike in the Ubuntu VM before any other ticket.
- **Does the restore token spare the dialog after a restart?**
- **Keyboard through libei or uinput?** uinput already works on every
  desktop; libei would drop the need for `/dev/uinput`. Decide from the
  spike.

## Discoveries

- **The spike passed** (#352, Ubuntu 24.04 VM: GNOME Shell 46.0,
  xdg-desktop-portal 1.18.4, xdg-desktop-portal-gnome 46.2):
  - The RemoteDesktop portal asks once ("Allow Remote Interaction",
    "Remember This Selection" on), then `ConnectToEIS` gives a libei
    socket; the handshake took 22 ms.
  - The absolute pointer's region was `(0, 0, 1280, 800)`, scale 1.0: the
    VM's one monitor, in logical pixels.
  - `motion_absolute(200, 150)` put the cursor exactly there (checked on a
    screenshot).
  - With `PersistMode::ExplicitlyRevoked` the restore token spares the
    dialog: the next session started in 36 ms, and after a reboot in
    50 ms, with no dialog.
  - Besides the absolute pointer, GNOME gives a relative pointer and a
    keyboard: keys can go through libei too, so a GNOME target would not
    need `/dev/uinput` (decided in the ticket that emits).
  - While the session lasts, GNOME shows its orange "remote control"
    indicator in the top bar: the owner sees when Mickey is in control.
  - The crates built as planned: `ashpd` 0.13 (features `remote_desktop`
    and `screencast`, which the former needs) and `reis` 0.7 with tokio;
    the binary needs glibc 2.39 at most.
- **KDE Plasma works on the same backend, unchanged** (#355, Fedora 44 KDE
  live VM: Plasma 6.6, xdg-desktop-portal 1.21.1, xdg-desktop-portal-kde
  6.6.4). Its dialog reads "An application is asking for special
  privileges: Control input devices", with "Allow restoring on future
  sessions" ticked; the edge test passed (cursor to (1279, 100) after a
  5000 px push) and, the token kept, passed again with no dialog.
- **A fresh install has no data folder yet**: the KDE VM showed it (the
  token was not saved, so the portal would ask at every start); the
  folder is now made first (#357). The GNOME VM had it from earlier
  tests, which hid the bug.
- **Phase 2's spike passed on GNOME 46** (InputCapture version 1): one zone
  `(0, 0, 1280, 800)`, a barrier on its right edge `x = 1280` accepted, and
  a virtual mouse pushing right activated it at `(1280.0, 288.0)`;
  released at `(1278, 288)`. GNOME's "Capture Input" dialog ("Allow Remote
  Input Capturing") has no "remember" in version 1, so it comes once per
  session.
- **Two VMs paired end to end (#360): GNOME drives KDE, both ways
  through the edges.** Ubuntu GNOME and Fedora KDE on their own LAN,
  paired with the new six-digit code (both screens showed the same code),
  placed side by side. From GNOME, pushing right crossed into KDE after
  65 steps of 20 px; 80 px back came home through KDE's left edge (so the
  cursor entered KDE on its edge); two keys typed on GNOME's keyboard
  arrived on KDE (288 bytes on its virtual device, "ab" in its terminal).
  One-way latency between the VMs: p50 0.78 ms, p99 2.4 ms.
- **KDE drives GNOME too, after two fixes (#359).** KDE froze the pointer
  until Mickey shook hands on the capture's libei socket; and KDE's
  activation names where the push would have gone, past the barrier
  (x = 1319 or -20 on a 1280-wide screen), where GNOME names the pixel on
  it. With both fixed, KDE crossed into GNOME through its left edge,
  moved GNOME's cursor, typed on it, and came back. The Fedora live VM
  needed 3.5 GB to answer its dialogs in time (3 GB lagged by minutes).
- **After a refused pairing, `mickey connect` will not try again**: the
  PC stays listed, without a key, as "already one of this PC's PCs".
  Nothing is trusted, but the only way out is editing config.toml.
- **At login Mickey can start before the desktop's portal** ("startup job
  failed", seen on Fedora KDE): the portal is now tried again every 5 s,
  and the source reads its monitors again while it has none (#357, #359).
- **GNOME 46 asks "Capture Input" at every start of Mickey**, as expected
  from the spike: the price of screen edges on GNOME until its portal
  remembers.

- **X11 needs no XInput2 or XTest** (#178): the X server answers what
  Hyprland answers. RandR's `GetMonitors` gives the monitors, `QueryPointer`
  the cursor and `WarpPointer` places it, through `x11rb` (pure Rust,
  already in the tree through arboard). Input still goes in through
  uinput, which X reads like any device, and out through evdev. Mint 22.3
  (Cinnamon, X11) and Ubuntu 24.04 GNOME, in two VMs: Mint drove GNOME
  through its left edge and came back, placed on that edge at the same
  height; GNOME drove Mint (cursor moved about 640 px) and came back
  through Mint's left edge. Hyprland's XWayland sets `DISPLAY` too, so
  Hyprland is checked first.
- **Sway, when controlled** (#178): its IPC socket gives the monitors
  (`get_outputs`, logical rects) and places the cursor
  (`seat - cursor set x y`), but never says where the cursor is. So a
  controlled Sway moves in absolute steps and remembers where it put the
  cursor, as the portal does on GNOME and KDE (`portal::slide`). Checked
  with a headless Sway 1.9 and a second Mickey in the Ubuntu VM: GNOME
  sent the focus there, moved the cursor and left through Sway's left
  edge. Driving others from Sway needs to see the cursor reach an edge:
  next.
- **Sway driving others** (#178): no portal and no cursor position, so
  the barrier lines become one-pixel wlr-layer-shell surfaces, on the
  output Sway's IPC names, and the cursor entering one is the push. Two
  things keep it from bouncing: a strip arms only after the cursor has
  left it, and Sway answers a `cursor set` onto a strip with a leave and
  an enter, so an enter within 300 ms of Mickey placing the cursor does
  not count. Checked with a headless Sway 1.9 reading a uinput test
  mouse and the Ubuntu VM's GNOME placed to its right: three rounds of
  crossing right and coming back through GNOME's left edge, none
  bouncing back within 2 s.
