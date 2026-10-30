//! Turns Mickey events into real input on the target: `SendInput` on
//! Windows, a uinput virtual device on Linux (PRD 0008).

use crate::event::InputEvent;
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
        use crate::keymap;
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

    /// Puts the cursor at `x, y` of the desktop, where it came in.
    pub fn place(&mut self, x: i32, y: i32) -> io::Result<()> {
        place(x, y)
    }
}

/// Where the cursor was before `park`, to put it back on `unpark`.
static PARKED_FROM: std::sync::Mutex<Option<(i32, i32)>> = std::sync::Mutex::new(None);

/// Moves the cursor out of sight while another PC has the keyboard and
/// mouse: into the bottom-right corner, where only its tip's pixel shows.
/// Nothing is hidden for real, so a crash never leaves the owner without a
/// cursor (CLAUDE.md rule 8).
pub fn park() {
    let Some(corner) = crate::monitors::this_pc().ok().as_deref().and_then(corner) else {
        return;
    };
    let mut parked = PARKED_FROM.lock().unwrap_or_else(|p| p.into_inner());
    if parked.is_none() {
        *parked = cursor();
    }
    let _ = place(corner.0, corner.1);
}

/// Puts the cursor back where `park` found it, if it was parked.
pub fn unpark() {
    let from = PARKED_FROM.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some((x, y)) = from {
        let _ = place(x, y);
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

/// Puts this PC's cursor at `x, y` of its desktop. uinput moves are
/// relative, so Hyprland places it (PRD 0005 *Discoveries*).
#[cfg(target_os = "linux")]
pub fn place(x: i32, y: i32) -> io::Result<()> {
    use crate::monitors::hyprland;
    if crate::x11::session() {
        return crate::x11::place(x, y).map_err(io::Error::other);
    }
    if crate::sway::session() {
        return crate::sway::place(x, y).map_err(io::Error::other);
    }
    if !crate::monitors::on_hyprland() {
        return crate::portal::get()
            .ok_or_else(|| io::Error::other("no RemoteDesktop portal to place the cursor"))?
            .place(x, y);
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
    if crate::monitors::on_hyprland() {
        return crate::monitors::hyprland::cursor().ok();
    }
    if crate::x11::session() {
        return crate::x11::cursor().ok();
    }
    if crate::sway::session() {
        return crate::sway::cursor();
    }
    crate::portal::get()?.cursor()
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
                // Sway, or the portal: absolute moves, so Mickey knows
                // where the cursor is (PRD 0004, #178).
                if crate::sway::session() {
                    return crate::sway::move_by(dx, dy).map_err(io::Error::other);
                }
                if crate::monitors::uses_portal()
                    && let Some(portal) = crate::portal::get()
                {
                    return portal.move_by(dx, dy);
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

    /// Puts the cursor at `x, y` of the desktop, where it came in.
    pub fn place(&mut self, x: i32, y: i32) -> io::Result<()> {
        place(x, y)
    }
}

#[cfg(test)]
mod park_tests {
    use super::corner;
    use crate::monitors::Monitor;

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
}
