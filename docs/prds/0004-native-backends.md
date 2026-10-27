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

