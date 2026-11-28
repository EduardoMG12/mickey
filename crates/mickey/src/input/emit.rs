//! Turns Mickey events into real input on the target: `SendInput` on
//! Windows, a uinput virtual device on Linux (PRD 0008).

use crate::input::event::InputEvent;
use crate::threads::LockAnyway;
use std::io;

#[cfg(windows)]
pub struct Emitter;

#[cfg(windows)]
impl Emitter {
    pub fn new() -> Result<Emitter, String> {
        Ok(Emitter)
    }

    /// Injects one event. Keys and buttons with no Windows equivalent are
    /// dropped.
    // ponytail: dropped silently; the PRD wants each unknown code logged
    // once, which waits for a logger that keeps codes at trace level.
    pub fn emit(&mut self, event: InputEvent) -> io::Result<()> {
        use crate::input::keymap;
        use crate::platform::windows;
        match event {
            InputEvent::Key { code, pressed } => match keymap::scan_code(code) {
                Some(scan) => windows::send_key(u16::from(scan.code), scan.extended, pressed),
                None => Ok(()),
            },
            InputEvent::Button { code, pressed } => match keymap::mouse_button(code) {
                Some(button) => windows::send_button(button, pressed),
                None => Ok(()),
            },
            InputEvent::Motion { dx, dy } => windows::move_mouse(dx, dy),
            InputEvent::Scroll {
                vertical,
                horizontal,
            } => windows::scroll(vertical, horizontal),
        }
    }
}

/// Tracks left and right modifier keys separately so holding both sides and
/// releasing one keeps the corresponding `CGEventFlags` bit set until both
/// are released.
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct MacModifiers {
    /// Bit 0: LShift, 1: RShift, 2: LCtrl, 3: RCtrl, 4: LAlt, 5: RAlt,
    /// 6: LMeta, 7: RMeta, 8: CapsLock.
    held: u16,
}

#[cfg(any(target_os = "macos", test))]
impl MacModifiers {
    const LSHIFT: u16 = 1 << 0;
    const RSHIFT: u16 = 1 << 1;
    const LCTRL: u16 = 1 << 2;
    const RCTRL: u16 = 1 << 3;
    const LALT: u16 = 1 << 4;
    const RALT: u16 = 1 << 5;
    const LMETA: u16 = 1 << 6;
    const RMETA: u16 = 1 << 7;
    const CAPSLOCK: u16 = 1 << 8;

    fn update(&mut self, evdev_code: u16, pressed: bool) {
        if evdev_code == 58 {
            if pressed {
                self.held ^= Self::CAPSLOCK;
            }
            return;
        }
        let bit = match evdev_code {
            42 => Self::LSHIFT,
            54 => Self::RSHIFT,
            29 => Self::LCTRL,
            97 => Self::RCTRL,
            56 => Self::LALT,
            100 => Self::RALT,
            125 => Self::LMETA,
            126 => Self::RMETA,
            _ => return,
        };
        if pressed {
            self.held |= bit;
        } else {
            self.held &= !bit;
        }
    }

    fn flags(&self) -> u64 {
        const FLAG_ALPHA_SHIFT: u64 = 0x0001_0000;
        const FLAG_SHIFT: u64 = 0x0002_0000;
        const FLAG_CONTROL: u64 = 0x0004_0000;
        const FLAG_ALTERNATE: u64 = 0x0008_0000;
        const FLAG_COMMAND: u64 = 0x0010_0000;

        let mut flags = 0u64;
        if self.held & Self::CAPSLOCK != 0 {
            flags |= FLAG_ALPHA_SHIFT;
        }
        if self.held & (Self::LSHIFT | Self::RSHIFT) != 0 {
            flags |= FLAG_SHIFT;
        }
        if self.held & (Self::LCTRL | Self::RCTRL) != 0 {
            flags |= FLAG_CONTROL;
        }
        if self.held & (Self::LALT | Self::RALT) != 0 {
            flags |= FLAG_ALTERNATE;
        }
        if self.held & (Self::LMETA | Self::RMETA) != 0 {
            flags |= FLAG_COMMAND;
        }
        flags
    }
}

#[cfg(target_os = "macos")]
pub struct Emitter {
    /// Active modifier keys, tracked left/right independently so synthetic
    /// keys and clicks carry Shift/Control/Option/Command accurately.
    modifiers: MacModifiers,
    /// Bitmask of currently pressed mouse buttons (bit 0 = left, 1 = right,
    /// 2 = middle, 3 = X1, 4 = X2) so motion emits drag events while held.
    held_buttons: u8,
    /// Multi-click state so double- and triple-clicks work natively on macOS.
    last_click: Option<(
        crate::input::keymap::MouseButton,
        std::time::Instant,
        (i32, i32),
        i64,
    )>,
    /// Sub-pixel scroll remainders per axis (`[vertical, horizontal]`), in
    /// hi-res wheel units (120 per notch).
    scroll_rem: [i32; 2],
    /// Cached display bounds, refreshed periodically so per-event mouse motion
    /// never allocates or queries CoreGraphics's display list at 1000 Hz.
    displays: Vec<crate::monitors::Monitor>,
    displays_at: std::time::Instant,
}

#[cfg(target_os = "macos")]
impl Emitter {
    pub fn new() -> Result<Emitter, String> {
        if !crate::platform::macos::accessibility_trusted() {
            return Err(
                "macOS Accessibility permission is required to receive keyboard and mouse input; \
                 enable Mickey (or your terminal) in System Settings > Privacy & Security > Accessibility"
                    .into(),
            );
        }
        let displays = crate::platform::macos::monitors().unwrap_or_default();
        Ok(Emitter {
            modifiers: MacModifiers::default(),
            held_buttons: 0,
            last_click: None,
            scroll_rem: [0; 2],
            displays,
            displays_at: std::time::Instant::now(),
        })
    }

    pub fn emit(&mut self, event: InputEvent) -> io::Result<()> {
        use crate::input::{keymap, keymap_macos};
        use crate::platform::macos;
        const PX_PER_NOTCH: i32 = 36;
        const NOTCH: i32 = 120;
        const DISPLAY_TTL: std::time::Duration = std::time::Duration::from_secs(1);

        match event {
            InputEvent::Key { code, pressed } => {
                self.modifiers.update(code, pressed);
                match keymap_macos::macos_keycode(code) {
                    Some(vk) => macos::send_key(vk, pressed, self.modifiers.flags()),
                    None => Ok(()),
                }
            }
            InputEvent::Button { code, pressed } => match keymap::mouse_button(code) {
                Some(button) => {
                    let bit = button_bit(button);
                    if pressed {
                        self.held_buttons |= bit;
                    } else {
                        self.held_buttons &= !bit;
                    }
                    let (x, y) = macos::cursor_pos().unwrap_or((0, 0));
                    let click_state = self.next_click_state(button, pressed, (x, y));
                    macos::send_button(x, y, button, pressed, click_state, self.modifiers.flags())
                }
                None => Ok(()),
            },
            InputEvent::Motion { dx, dy } => {
                if dx == 0 && dy == 0 {
                    return Ok(());
                }
                if self.displays_at.elapsed() >= DISPLAY_TTL {
                    if let Ok(found) = macos::monitors() {
                        self.displays = found;
                    }
                    self.displays_at = std::time::Instant::now();
                }
                let (cx, cy) = macos::cursor_pos().unwrap_or((0, 0));
                let (nx, ny) =
                    clamp_to_monitors(cx.saturating_add(dx), cy.saturating_add(dy), &self.displays);
                macos::move_mouse(nx, ny, dx, dy, self.held_buttons, self.modifiers.flags())
            }
            InputEvent::Scroll {
                vertical,
                horizontal,
            } => {
                let mut px = [0i32; 2];
                for (axis, amount) in [vertical, horizontal].into_iter().enumerate() {
                    if amount == 0 {
                        continue;
                    }
                    let scaled = amount
                        .saturating_mul(PX_PER_NOTCH)
                        .saturating_add(self.scroll_rem[axis]);
                    px[axis] = scaled / NOTCH;
                    self.scroll_rem[axis] = scaled % NOTCH;
                }
                macos::scroll(px[0], px[1], self.modifiers.flags())
            }
        }
    }

    fn next_click_state(
        &mut self,
        button: crate::input::keymap::MouseButton,
        pressed: bool,
        pos: (i32, i32),
    ) -> i64 {
        const DOUBLE_CLICK_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);
        const MAX_DRIFT: i32 = 4;
        let now = std::time::Instant::now();
        if pressed {
            let count = match self.last_click {
                Some((prev_btn, prev_time, (px, py), prev_count))
                    if prev_btn == button
                        && now.duration_since(prev_time) <= DOUBLE_CLICK_WINDOW
                        && (pos.0 - px).abs() <= MAX_DRIFT
                        && (pos.1 - py).abs() <= MAX_DRIFT =>
                {
                    (prev_count % 3) + 1
                }
                _ => 1,
            };
            self.last_click = Some((button, now, pos, count));
            count
        } else {
            self.last_click
                .filter(|(prev_btn, ..)| *prev_btn == button)
                .map_or(1, |(.., count)| count)
        }
    }
}

#[cfg(target_os = "macos")]
fn button_bit(button: crate::input::keymap::MouseButton) -> u8 {
    use crate::input::keymap::MouseButton;
    match button {
        MouseButton::Left => 1 << 0,
        MouseButton::Right => 1 << 1,
        MouseButton::Middle => 1 << 2,
        MouseButton::X1 => 1 << 3,
        MouseButton::X2 => 1 << 4,
    }
}

/// Clamps `(x, y)` to the nearest point inside any active monitor, so relative
/// moves never push the synthetic cursor outside the display layout.
#[cfg(any(target_os = "macos", test))]
fn clamp_to_monitors(x: i32, y: i32, monitors: &[crate::monitors::Monitor]) -> (i32, i32) {
    if monitors.is_empty() {
        return (x, y);
    }
    let mut best = (x, y);
    let mut best_dist = i64::MAX;
    for m in monitors {
        if m.width <= 0 || m.height <= 0 {
            continue;
        }
        let cx = x.clamp(m.x, m.x + m.width - 1);
        let cy = y.clamp(m.y, m.y + m.height - 1);
        if cx == x && cy == y {
            return (x, y);
        }
        let dx = i64::from(x - cx);
        let dy = i64::from(y - cy);
        let dist = dx * dx + dy * dy;
        if dist < best_dist {
            best_dist = dist;
            best = (cx, cy);
        }
    }
    best
}

/// Puts this PC's cursor at `x, y` of its desktop, clamped to active displays.
#[cfg(target_os = "macos")]
pub fn place(x: i32, y: i32) -> io::Result<()> {
    let displays = crate::platform::macos::monitors().unwrap_or_default();
    let (nx, ny) = clamp_to_monitors(x, y, &displays);
    crate::platform::macos::set_cursor_pos(nx, ny)
}

/// Where this PC's cursor is, in the coordinates of its monitors.
#[cfg(target_os = "macos")]
pub fn cursor() -> Option<(i32, i32)> {
    crate::platform::macos::cursor_pos()
}

/// Where the cursor was before `park`, to put it back on `unpark`.
static PARKED: Parking = Parking(std::sync::Mutex::new(None));

impl Emitter {
    /// Puts the cursor at `x, y` of the desktop, where it came in across
    /// an edge. The focus news that follows must not put it back where it
    /// was parked from: that is where it left, not where it came in (#554).
    pub fn enter(&mut self, x: i32, y: i32) -> io::Result<()> {
        PARKED.entered(|| place(x, y))
    }
}

/// Moves the cursor out of sight while another PC has the keyboard and
/// mouse: into the bottom-right corner, where only its tip's pixel shows.
/// Nothing is hidden for real, so a crash never leaves the owner without a
/// cursor (CLAUDE.md rule 8).
pub fn park() {
    let Some(corner) = crate::monitors::this_pc().ok().as_deref().and_then(corner) else {
        return;
    };
    PARKED.park(cursor);
    let _ = place(corner.0, corner.1);
}

/// Puts the cursor back where `park` found it, if it was parked and has
/// not come in across an edge since.
pub fn unpark() {
    if let Some((x, y)) = PARKED.unpark() {
        let _ = place(x, y);
    }
}

/// Where the cursor was when it was parked, if it is still to go back
/// there. Kept apart from moving the cursor, so the order of park, enter
/// and unpark can be tested.
struct Parking(std::sync::Mutex<Option<(i32, i32)>>);

impl Parking {
    /// Remembers where `cursor` says it is, unless already parked (parking
    /// twice must not forget where it was before the first).
    fn park(&self, cursor: impl FnOnce() -> Option<(i32, i32)>) {
        let mut parked = self.0.lock_anyway();
        if parked.is_none() {
            *parked = cursor();
        }
    }

    /// Where to put the cursor back, once.
    fn unpark(&self) -> Option<(i32, i32)> {
        self.0.lock_anyway().take()
    }

    /// The cursor came in across an edge: `place` puts it there, and the
    /// parked position is forgotten. Held while placing, so a `park` from
    /// another source in between remembers the new place, not the old.
    fn entered(&self, place: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
        let mut parked = self.0.lock_anyway();
        place()?;
        *parked = None;
        Ok(())
    }
}

/// The last pixel of the monitor whose bottom-right corner lies farthest
/// right (then lowest).
fn corner(monitors: &[crate::monitors::Monitor]) -> Option<(i32, i32)> {
    monitors
        .iter()
        .map(|m| (m.x + m.width - 1, m.y + m.height - 1))
        .max()
}

/// Puts this PC's cursor at `x, y` of its desktop.
#[cfg(windows)]
pub fn place(x: i32, y: i32) -> io::Result<()> {
    crate::platform::windows::set_cursor_pos(x, y)
}

/// Who places and follows the cursor on this PC (#459).
#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, PartialEq)]
enum Path {
    /// Its own IPC or server: Hyprland, X11, Sway.
    Session(crate::desktop::Session),
    /// The RemoteDesktop portal (GNOME, KDE).
    Portal,
    /// No portal yet on GNOME or KDE (its dialog unanswered): wait for it.
    Wait,
    /// No portal at all (COSMIC on the 24.04 ISO): the absolute pointer.
    Absolute,
}

/// The path for `session`, given whether its RemoteDesktop portal is up
/// and whether this is GNOME or KDE, which always have one.
#[cfg(target_os = "linux")]
fn path(session: crate::desktop::Session, portal_up: bool, portal_desktop: bool) -> Path {
    use crate::desktop::Session;
    match session {
        Session::Portal if portal_up => Path::Portal,
        Session::Portal if portal_desktop => Path::Wait,
        Session::Portal => Path::Absolute,
        other => Path::Session(other),
    }
}

/// This PC's path, with its portal when that is the one.
#[cfg(target_os = "linux")]
fn here() -> (Path, Option<&'static crate::desktop::portal::Portal>) {
    let session = crate::desktop::session();
    let portal = (session == crate::desktop::Session::Portal)
        .then(crate::desktop::portal::get)
        .flatten();
    (
        path(session, portal.is_some(), crate::monitors::portal_desktop()),
        portal,
    )
}

/// Puts this PC's cursor at `x, y` of its desktop. uinput moves are
/// relative, so the desktop places it, each its own way (`Path`: Hyprland,
/// Sway, the X server or the portal; PRD 0005 *Discoveries*).
#[cfg(target_os = "linux")]
pub fn place(x: i32, y: i32) -> io::Result<()> {
    use crate::desktop::Session;
    use crate::desktop::hyprland;
    match here() {
        (Path::Session(Session::X11), _) => {
            return crate::desktop::x11::place(x, y).map_err(io::Error::other);
        }
        (Path::Session(Session::Sway), _) => {
            return crate::desktop::sway::place(x, y).map_err(io::Error::other);
        }
        (Path::Portal, Some(portal)) => return portal.place(x, y),
        (Path::Absolute, _) => return crate::desktop::absolute::place(x, y),
        (Path::Wait | Path::Portal, _) => {
            return Err(io::Error::other(
                "no RemoteDesktop portal to place the cursor",
            ));
        }
        (Path::Session(_), _) => {}
    }
    // A Lua config (Hyprland 0.55+) takes Lua; the older config language
    // takes the plain dispatcher. Try the first, then the second.
    let lua = format!("dispatch hl.dsp.cursor.move({{x={x},y={y}}})");
    let mut answer = hyprland::ask(&lua).map_err(io::Error::other)?;
    if answer.trim() != "ok" {
        answer =
            hyprland::ask(&format!("dispatch movecursor {x} {y}")).map_err(io::Error::other)?;
    }
    match answer.trim() {
        "ok" => Ok(()),
        _ => Err(io::Error::other(format!("Hyprland answered `{answer}`"))),
    }
}

/// Where this PC's cursor is, in the coordinates of its monitors.
#[cfg(windows)]
pub fn cursor() -> Option<(i32, i32)> {
    let point = crate::platform::hooks::cursor_pos();
    Some((point.x, point.y))
}

/// Where this PC's cursor is: Hyprland says; elsewhere, where Mickey put
/// it through the portal (PRD 0004).
#[cfg(target_os = "linux")]
pub fn cursor() -> Option<(i32, i32)> {
    use crate::desktop::Session;
    match here() {
        (Path::Session(Session::Hyprland), _) => crate::desktop::hyprland::cursor().ok(),
        (Path::Session(Session::X11), _) => crate::desktop::x11::cursor().ok(),
        (Path::Session(Session::Sway), _) => crate::desktop::sway::cursor(),
        (Path::Portal, portal) => portal?.cursor(),
        (Path::Absolute, _) => crate::desktop::absolute::cursor(),
        (Path::Wait | Path::Session(Session::Portal), _) => None,
    }
}

#[cfg(target_os = "linux")]
pub struct Emitter {
    device: crate::platform::uinput::VirtualDevice,
    /// Hi-res wheel units not yet worth a whole notch, per axis, so that
    /// programs reading only `REL_WHEEL` still scroll.
    partial: [i32; 2],
}

#[cfg(target_os = "linux")]
impl Emitter {
    pub fn new() -> Result<Emitter, String> {
        use crate::platform::uinput::VirtualDevice;
        let device =
            VirtualDevice::create(crate::platform::uinput::DEVICE_NAME).map_err(|err| match err
                .kind()
            {
                io::ErrorKind::PermissionDenied | io::ErrorKind::NotFound => format!(
                    "cannot create a virtual keyboard ({err}); allow the input group with the \
                 udev rule in the README, or for this boot: \
                 sudo chgrp input /dev/uinput && sudo chmod 660 /dev/uinput"
                ),
                _ => format!("cannot create a virtual keyboard: {err}"),
            })?;
        Ok(Emitter {
            device,
            partial: [0; 2],
        })
    }

    /// Codes are evdev codes already, so keys pass straight through.
    pub fn emit(&mut self, event: InputEvent) -> io::Result<()> {
        use crate::platform::uinput::{EV_KEY, EV_REL};
        const REL_X: u16 = 0;
        const REL_Y: u16 = 1;
        // (low-res, hi-res) codes for vertical and horizontal.
        const WHEELS: [(u16, u16); 2] = [(8, 11), (6, 12)];
        const NOTCH: i32 = 120;
        match event {
            InputEvent::Key { code, pressed } | InputEvent::Button { code, pressed } => {
                self.device.emit(EV_KEY, code, i32::from(pressed));
            }
            InputEvent::Motion { dx, dy } => {
                // Only the axes that moved: a zero is an event for nothing.
                if dx == 0 && dy == 0 {
                    return Ok(());
                }
                // Sway, the portal or the absolute pointer: absolute moves,
                // so Mickey knows where the cursor is (PRD 0004, #178, #459).
                match here() {
                    (Path::Session(crate::desktop::Session::Sway), _) => {
                        return crate::desktop::sway::move_by(dx, dy).map_err(io::Error::other);
                    }
                    (Path::Portal, Some(portal)) => return portal.move_by(dx, dy),
                    // No absolute pointer (none could be made): relative,
                    // as before #459.
                    (Path::Absolute, _) if crate::desktop::absolute::move_by(dx, dy).is_ok() => {
                        return Ok(());
                    }
                    _ => {}
                }
                if dx != 0 {
                    self.device.emit(EV_REL, REL_X, dx);
                }
                if dy != 0 {
                    self.device.emit(EV_REL, REL_Y, dy);
                }
            }
            InputEvent::Scroll {
                vertical,
                horizontal,
            } => {
                for (axis, amount) in [vertical, horizontal].into_iter().enumerate() {
                    if amount == 0 {
                        continue;
                    }
                    let (low, high) = WHEELS[axis];
                    self.device.emit(EV_REL, high, amount);
                    self.partial[axis] += amount;
                    let notches = self.partial[axis] / NOTCH;
                    if notches != 0 {
                        self.device.emit(EV_REL, low, notches);
                        self.partial[axis] -= notches * NOTCH;
                    }
                }
            }
        }
        self.device.sync()
    }
}

#[cfg(all(test, target_os = "linux"))]
mod path_tests {
    use super::*;
    use crate::desktop::Session;

    /// Which way each desktop places the cursor: the new absolute pointer
    /// only where there is no portal and the desktop is not GNOME or KDE.
    #[test]
    fn each_desktop_keeps_its_way_and_only_portal_less_ones_go_absolute() {
        let cases = [
            // (session, portal up, GNOME or KDE) -> path
            (
                Session::Hyprland,
                false,
                false,
                Path::Session(Session::Hyprland),
            ),
            (Session::X11, false, false, Path::Session(Session::X11)),
            (Session::Sway, false, false, Path::Session(Session::Sway)),
            (Session::Portal, true, true, Path::Portal),
            (Session::Portal, true, false, Path::Portal),
            (Session::Portal, false, true, Path::Wait),
            (Session::Portal, false, false, Path::Absolute),
        ];
        for (session, portal_up, portal_desktop, expected) in cases {
            assert_eq!(
                path(session, portal_up, portal_desktop),
                expected,
                "{session:?}, portal up: {portal_up}, GNOME or KDE: {portal_desktop}"
            );
        }
    }
}

#[cfg(test)]
mod park_tests {
    use super::{Parking, corner};
    use crate::monitors::Monitor;
    use std::sync::Mutex;

    #[test]
    fn coming_in_across_an_edge_is_not_undone_by_unparking() {
        let parked = Parking(Mutex::new(None));
        // It left near the top: parked from there.
        parked.park(|| Some((1919, 120)));
        // It comes back in near the bottom, then the focus news unparks.
        let mut placed = None;
        let place = || {
            placed = Some((0, 885));
            Ok(())
        };
        parked.entered(place).unwrap();
        assert_eq!(placed, Some((0, 885)));
        assert_eq!(parked.unpark(), None, "not back to where it left (#554)");
    }

    #[test]
    fn the_hotkey_puts_the_cursor_back_where_it_was_parked() {
        let parked = Parking(Mutex::new(None));
        parked.park(|| Some((500, 300)));
        parked.park(|| Some((1919, 1079)));
        assert_eq!(parked.unpark(), Some((500, 300)), "not the corner");
        assert_eq!(parked.unpark(), None, "once");
    }

    #[test]
    fn a_place_that_fails_keeps_where_to_go_back() {
        let parked = Parking(Mutex::new(None));
        parked.park(|| Some((500, 300)));
        let failed = parked.entered(|| Err(std::io::Error::other("no portal")));
        assert!(failed.is_err());
        assert_eq!(parked.unpark(), Some((500, 300)));
    }

    #[test]
    fn the_corner_is_the_right_most_monitors_last_pixel() {
        let left = Monitor {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let right = Monitor {
            x: 1920,
            y: 200,
            width: 1280,
            height: 720,
        };
        assert_eq!(corner(&[left, right]), Some((3199, 919)));
        assert_eq!(corner(&[left]), Some((1919, 1079)));
        assert_eq!(corner(&[]), None);
    }

    #[test]
    fn clamping_keeps_points_inside_the_display_layout() {
        let left = Monitor {
            x: 0,
            y: 0,
            width: 1470,
            height: 956,
        };
        let right = Monitor {
            x: 1470,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            super::clamp_to_monitors(500, 300, &[left, right]),
            (500, 300)
        );
        assert_eq!(super::clamp_to_monitors(-10, 300, &[left, right]), (0, 300));
        assert_eq!(
            super::clamp_to_monitors(4000, 2000, &[left, right]),
            (3389, 1079)
        );
        assert_eq!(super::clamp_to_monitors(50, 50, &[]), (50, 50));
    }

    #[test]
    fn releasing_one_modifier_side_keeps_the_other_active() {
        let mut mods = super::MacModifiers::default();
        // Hold both Left Shift (42) and Right Shift (54), release Left Shift.
        mods.update(42, true);
        mods.update(54, true);
        mods.update(42, false);
        assert_ne!(mods.flags() & 0x0002_0000, 0, "Right Shift still held");
        mods.update(54, false);
        assert_eq!(mods.flags() & 0x0002_0000, 0, "both Shifts released");

        // Same check for Ctrl (29/97), Option (56/100), Command (125/126).
        for (left, right, mask) in [
            (29, 97, 0x0004_0000),
            (56, 100, 0x0008_0000),
            (125, 126, 0x0010_0000),
        ] {
            mods.update(left, true);
            mods.update(right, true);
            mods.update(right, false);
            assert_ne!(mods.flags() & mask, 0);
            mods.update(left, false);
            assert_eq!(mods.flags() & mask, 0);
        }
    }
}
