//! Linux capture: find keyboards and mice, read them on one thread each and
//! turn raw evdev events into `InputEvent`s on a single channel.

use crate::event::InputEvent;
use crate::platform::linux::{Device, RawEvent};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const SYN_REPORT: u16 = 0;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_HWHEEL: u16 = 6;
const REL_WHEEL: u16 = 8;
const REL_WHEEL_HI_RES: u16 = 11;
const REL_HWHEEL_HI_RES: u16 = 12;
const KEY_A: u16 = 30;
const KEY_Z: u16 = 44;
const BTN_LEFT: u16 = 0x110;
/// Mouse buttons are 0x110 (left) to 0x117; everything below 0x100 is a key.
const MOUSE_BUTTONS: std::ops::RangeInclusive<u16> = 0x110..=0x117;
/// One wheel notch in hi-res units.
const NOTCH: i32 = 120;

pub struct Found {
    pub device: Arc<Device>,
    pub name: String,
    pub is_keyboard: bool,
    hi_res_wheel: bool,
}

/// Opens every keyboard and mouse under `/dev/input` (the kernel's fixed
/// location for evdev nodes). Other devices, like the power button or the
/// touchpad, are skipped.
pub fn find_devices() -> io::Result<Vec<Found>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/dev/input")? {
        let path = entry?.path();
        let is_event_node = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("event"));
        if is_event_node && let Some(device) = classify(&path)? {
            found.push(device);
        }
    }
    Ok(found)
}

fn classify(path: &Path) -> io::Result<Option<Found>> {
    let device = Device::open(path)?;
    let is_keyboard = device.supports(EV_KEY, KEY_A)? && device.supports(EV_KEY, KEY_Z)?;
    let is_mouse = device.supports(EV_REL, REL_X)?
        && device.supports(EV_REL, REL_Y)?
        && device.supports(EV_KEY, BTN_LEFT)?;
    if !is_keyboard && !is_mouse {
        return Ok(None);
    }
    Ok(Some(Found {
        name: device.name()?,
        hi_res_wheel: device.supports(EV_REL, REL_WHEEL_HI_RES)?,
        is_keyboard,
        device: Arc::new(device),
    }))
}

/// Starts one reader thread per device. A thread ends when its device fails
/// (e.g. it was unplugged) or the receiver is gone.
pub fn spawn_readers(devices: &[Found], events: Sender<InputEvent>) {
    for found in devices {
        let device = Arc::clone(&found.device);
        let events = events.clone();
        let mut translator = Translator::new(found.hi_res_wheel);
        thread::spawn(move || {
            while let Ok(raw) = device.read_event() {
                let mut closed = false;
                translator.feed(raw, |event| closed |= events.send(event).is_err());
                if closed {
                    return;
                }
            }
        });
    }
}

/// Raw evdev events to `InputEvent`s, for one device. Motion and wheel
/// arrive as separate axis events and are complete only at `SYN_REPORT`,
/// so they are summed until then.
struct Translator {
    hi_res_wheel: bool,
    dx: i32,
    dy: i32,
    vertical: i32,
    horizontal: i32,
}

impl Translator {
    fn new(hi_res_wheel: bool) -> Translator {
        Translator {
            hi_res_wheel,
            dx: 0,
            dy: 0,
            vertical: 0,
            horizontal: 0,
        }
    }

    fn feed(&mut self, raw: RawEvent, mut out: impl FnMut(InputEvent)) {
        // value: 0 = released, 1 = pressed, 2 = auto-repeat (sent as a press,
        // because Windows does not repeat injected keys by itself).
        let pressed = raw.value != 0;
        let value = raw.value;
        match (raw.kind, raw.code) {
            (EV_KEY, code) if code < 0x100 => out(InputEvent::Key { code, pressed }),
            (EV_KEY, code) if MOUSE_BUTTONS.contains(&code) => {
                out(InputEvent::Button { code, pressed })
            }
            (EV_REL, REL_X) => self.dx = self.dx.saturating_add(value),
            (EV_REL, REL_Y) => self.dy = self.dy.saturating_add(value),
            // A hi-res wheel sends both kinds; count only one of them.
            (EV_REL, REL_WHEEL_HI_RES) => self.vertical = self.vertical.saturating_add(value),
            (EV_REL, REL_HWHEEL_HI_RES) => self.horizontal = self.horizontal.saturating_add(value),
            (EV_REL, REL_WHEEL) if !self.hi_res_wheel => {
                self.vertical = self.vertical.saturating_add(value.saturating_mul(NOTCH))
            }
            (EV_REL, REL_HWHEEL) if !self.hi_res_wheel => {
                self.horizontal = self.horizontal.saturating_add(value.saturating_mul(NOTCH))
            }
            (EV_SYN, SYN_REPORT) => self.flush(out),
            _ => {}
        }
    }

    fn flush(&mut self, mut out: impl FnMut(InputEvent)) {
        if self.dx != 0 || self.dy != 0 {
            out(InputEvent::Motion {
                dx: self.dx,
                dy: self.dy,
            });
        }
        if self.vertical != 0 || self.horizontal != 0 {
            out(InputEvent::Scroll {
                vertical: self.vertical,
                horizontal: self.horizontal,
            });
        }
        (self.dx, self.dy, self.vertical, self.horizontal) = (0, 0, 0, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(hi_res_wheel: bool, raw: &[(u16, u16, i32)]) -> Vec<InputEvent> {
        let mut translator = Translator::new(hi_res_wheel);
        let mut out = Vec::new();
        for &(kind, code, value) in raw {
            translator.feed(RawEvent { kind, code, value }, |event| out.push(event));
        }
        out
    }

    const SYN: (u16, u16, i32) = (EV_SYN, SYN_REPORT, 0);

    #[test]
    fn keys_pass_through_and_repeats_are_presses() {
        let out = run(
            false,
            &[(EV_KEY, KEY_A, 1), (EV_KEY, KEY_A, 2), (EV_KEY, KEY_A, 0)],
        );
        assert_eq!(
            out,
            [true, true, false].map(|pressed| InputEvent::Key {
                code: KEY_A,
                pressed
            })
        );
    }

    #[test]
    fn motion_is_summed_until_syn_report() {
        let out = run(
            false,
            &[
                (EV_REL, REL_X, 3),
                (EV_REL, REL_Y, -2),
                (EV_REL, REL_X, 1),
                SYN,
                SYN,
            ],
        );
        assert_eq!(out, [InputEvent::Motion { dx: 4, dy: -2 }]);
    }

    #[test]
    fn hi_res_wheels_are_not_counted_twice() {
        let hi_res = run(
            true,
            &[(EV_REL, REL_WHEEL, 1), (EV_REL, REL_WHEEL_HI_RES, 120), SYN],
        );
        assert_eq!(
            hi_res,
            [InputEvent::Scroll {
                vertical: 120,
                horizontal: 0
            }]
        );
        let classic = run(false, &[(EV_REL, REL_HWHEEL, -1), SYN]);
        assert_eq!(
            classic,
            [InputEvent::Scroll {
                vertical: 0,
                horizontal: -120
            }]
        );
    }

    #[test]
    fn buttons_are_kept_and_other_codes_dropped() {
        let out = run(
            false,
            &[(EV_KEY, BTN_LEFT, 1), (EV_KEY, 0x14a, 1), (4, 4, 30)],
        );
        assert_eq!(
            out,
            [InputEvent::Button {
                code: BTN_LEFT,
                pressed: true
            }]
        );
    }
}
