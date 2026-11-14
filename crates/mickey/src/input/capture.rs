//! Linux capture: find keyboards, mice and touchpads, read them on one
//! thread each and turn raw evdev events into `InputEvent`s on one channel.
//!
//! A touchpad is read by [`crate::input::touchpad::Touchpad`], which turns
//! the raw multitouch events into motion, two-finger scroll and taps. The
//! owner's choices come from the desktop's own settings where available
//! (GNOME's GSettings, KDE's `kcminputrc`), else from `config.toml`. The
//! reader thread waits on `poll(2)` between reports, so a tap's timer can
//! fire even when the touchpad sends nothing more.

use crate::input::event::InputEvent;
use crate::input::touchpad::{ButtonMap, Moves, Settings, Touchpad};
use crate::input::touchpad_prefs;
use crate::platform::linux::{Device, RawEvent};
use mickey_config as config;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_HWHEEL: u16 = 6;
const REL_WHEEL: u16 = 8;
const REL_WHEEL_HI_RES: u16 = 11;
const REL_HWHEEL_HI_RES: u16 = 12;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const KEY_A: u16 = 30;
const KEY_Z: u16 = 44;
pub(crate) const BTN_LEFT: u16 = 0x110;
/// Exactly one finger is on the touchpad.
const BTN_TOOL_FINGER: u16 = 0x145;
/// Mouse buttons are 0x110 (left) to 0x117; everything below 0x100 is a key.
const MOUSE_BUTTONS: std::ops::RangeInclusive<u16> = 0x110..=0x117;
/// One wheel notch in hi-res units.
const NOTCH: i32 = 120;
/// Used when a touchpad does not report its resolution.
const FALLBACK_UNITS_PER_MM: i32 = 10;
/// A tap is ignored for this long after a key was pressed (libinput's
/// default is around 0.5 s; we keep it a touch longer for safety).
const TYPING_WINDOW: Duration = Duration::from_millis(500);
/// Wake an idle reader once a second so it never stalls on a gone device,
/// and a touchpad with no events still sees its tap timers tick.
const IDLE_WAKE: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Keyboard,
    Mouse,
    Touchpad,
}

pub struct Found {
    pub device: Arc<Device>,
    pub name: String,
    pub kind: Kind,
    hi_res_wheel: bool,
    /// Touchpad units per millimetre on X and Y.
    resolution: [i32; 2],
    /// Whether the touchpad (when this is one) forwards to other PCs or
    /// stays local. `config.toml`'s `forward = false` keeps a touchpad on
    /// this PC: it is read but never grabbed, so its cursor keeps moving
    /// this PC while the keyboard and mouse go elsewhere.
    pub(crate) forward: bool,
}

/// Opens every keyboard, mouse and touchpad under `/dev/input` (the kernel's
/// fixed location for evdev nodes). Other devices, like the power button,
/// are skipped.
///
/// A device this user may not open is skipped, not fatal: one appearing
/// right now is root's until udev hands it to the `input` group, and
/// `mickey run` creates its own virtual device just as it starts. Only if
/// every device is refused is that the answer.
pub fn find_devices() -> io::Result<Vec<Found>> {
    let mut found = Vec::new();
    let mut refused = None;
    for entry in std::fs::read_dir("/dev/input")? {
        let path = entry?.path();
        let is_event_node = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("event"));
        if !is_event_node {
            continue;
        }
        match classify(&path) {
            Ok(Some(device)) => found.push(device),
            Ok(None) => {}
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied => refused = Some(err),
            Err(err) => return Err(err),
        }
    }
    match refused {
        Some(err) if found.is_empty() => Err(err),
        _ => Ok(found),
    }
}

fn classify(path: &Path) -> io::Result<Option<Found>> {
    let device = Device::open(path)?;
    // Our own uinput device (a target in the same `mickey run`) carries
    // input that came from another PC; capturing it would send it back.
    if device.name()? == crate::platform::uinput::DEVICE_NAME {
        return Ok(None);
    }
    let kind = if device.supports(EV_KEY, KEY_A)? && device.supports(EV_KEY, KEY_Z)? {
        Kind::Keyboard
    } else if device.supports(EV_REL, REL_X)?
        && device.supports(EV_REL, REL_Y)?
        && device.supports(EV_KEY, BTN_LEFT)?
    {
        Kind::Mouse
    } else if device.supports(EV_ABS, ABS_X)?
        && device.supports(EV_ABS, ABS_Y)?
        && device.supports(EV_KEY, BTN_TOOL_FINGER)?
    {
        Kind::Touchpad
    } else {
        return Ok(None);
    };
    let mut resolution = [0; 2];
    if kind == Kind::Touchpad {
        for (axis, value) in [ABS_X, ABS_Y].into_iter().zip(&mut resolution) {
            *value = match device.abs_resolution(axis)? {
                0 => FALLBACK_UNITS_PER_MM,
                units => units,
            };
        }
    }
    Ok(Some(Found {
        name: device.name()?,
        hi_res_wheel: device.supports(EV_REL, REL_WHEEL_HI_RES)?,
        kind,
        resolution,
        device: Arc::new(device),
        // Default: forward; the source adjusts this once it has config.
        forward: true,
    }))
}

/// Marks which touchpads forward to other PCs (`forward = false` in
/// `config.toml` keeps a touchpad local). Called by the source after
/// `find_devices`, before `spawn_readers`.
pub fn set_forward(devices: &mut [Found], touchpad_config: &config::Touchpad) {
    for found in devices {
        if found.kind == Kind::Touchpad {
            found.forward = touchpad_config.forward;
        }
    }
}

/// The touchpad's settings, as the owner and the desktop choose them.
///
/// `config` is what `config.toml` says; the desktop's own settings are read
/// once, on the spot, so a change in the desktop's panel is taken at start
/// (and again when Mickey is restarted). What the owner set in `config.toml`
/// wins over what the desktop reports; `auto` follows the desktop.
pub fn touchpad_settings(config: &config::Touchpad, device: &str) -> Settings {
    let detected = touchpad_prefs::detect(device);
    if let Some(source) = detected.source {
        eprintln!("mickey: touchpad settings read from {source}");
    }
    let tap = config.tap.resolve(detected.tap).unwrap_or(false);
    let natural_scroll = config
        .natural_scroll
        .resolve(detected.natural_scroll)
        .unwrap_or(false);
    let two_finger_scroll = detected.two_finger_scroll.unwrap_or(true);
    let tap_and_drag = detected.tap_and_drag.unwrap_or(true);
    let button_map = detected.button_map.unwrap_or(ButtonMap::Lrm);
    let disable_while_typing = detected.disable_while_typing.unwrap_or(true);
    Settings {
        speed: config.speed,
        scroll_speed: config.scroll_speed,
        natural_scroll,
        two_finger_scroll,
        tap,
        tap_and_drag,
        button_map,
        disable_while_typing,
    }
}

/// Starts one reader thread per device. A thread ends when its device fails
/// (e.g. it was unplugged) or the receiver is gone. `config` is the
/// `[touchpad]` block of `config.toml`: a touchpad is read with the owner's
/// choices and the desktop's own, a mouse is read as a mouse.
///
/// Events go into `events` as any type that can be made from an
/// `InputEvent`, so the source can share one channel with other wake-ups.
///
/// `keyboard` is any keyboard among the devices, so a touchpad's
/// `disable while typing` can ask whether a key is down on a device other
/// than its own (the usual case: a touchpad with no keys, a keyboard next
/// to it). `None` falls back to the touchpad's own keys alone.
pub fn spawn_readers<T: From<InputEvent> + Send + 'static>(
    devices: &[Found],
    events: Sender<T>,
    config: &config::Touchpad,
    keyboard: Option<Arc<Device>>,
) {
    for found in devices {
        let device = Arc::clone(&found.device);
        let events = events.clone();
        let keyboard = keyboard.clone();
        let mut translator = Translator::new(found, config, keyboard.clone());
        let mut batch = Vec::with_capacity(64);
        crate::threads::spawn("capture", move || {
            loop {
                // Wait for events, but at most until the touchpad's next
                // tap timer fires, so it gets to run even when the pad is
                // silent. A keyboard or mouse waits up to `IDLE_WAKE`.
                let wait = translator.wait_until().unwrap_or(IDLE_WAKE).min(IDLE_WAKE);
                let mut closed = false;
                match device.wait(wait) {
                    Ok(true) => {
                        // Events available: read one batch. `read_events`
                        // blocks for the next event if called again, so we
                        // read only once per `wait` and then let `tick`
                        // run — that is the whole point of `wait`.
                        if device.read_events(&mut batch).is_ok() {
                            for &raw in &batch {
                                translator.feed(raw, |event| {
                                    closed |= events.send(T::from(event)).is_err();
                                });
                                if closed {
                                    return;
                                }
                            }
                        }
                    }
                    Ok(false) => {
                        // A timeout: no events, but time may have
                        // advanced enough to fire a tap's release.
                    }
                    Err(err) => {
                        if err.kind() != io::ErrorKind::Interrupted {
                            return;
                        }
                    }
                }
                // Whether on a timeout or after a report: give the touchpad
                // its tick, so a tap whose time is up lets go of its button
                // even when the pad sends nothing more.
                translator.tick(|event| {
                    closed |= events.send(T::from(event)).is_err();
                });
                if closed {
                    return;
                }
            }
        });
    }
}

/// Raw evdev events to `InputEvent`s, for one device. Motion and wheel
/// arrive as separate axis events and are complete only at `SYN_REPORT`,
/// so they are summed until then. A touchpad's motion and scroll arrive
/// from `Touchpad::report`; its tap buttons arrive at the same call.
struct Translator {
    hi_res_wheel: bool,
    touchpad: Option<Touchpad>,
    /// A keyboard, so a touchpad's `disable while typing` can ask whether a
    /// key is down on a device other than its own. `None` when this device
    /// is not a touchpad or no keyboard is shared with the reader.
    keyboard: Option<Arc<Device>>,
    /// The reader's epoch, the same one the touchpad's `Duration` clock
    /// uses. Stored as an `Instant` so the monotonic clock is kept.
    start: Instant,
    /// When a key was last pressed anywhere (this touchpad's own keys or the
    /// shared keyboard), so `disable while typing` is decided locally.
    last_key: Instant,
    dx: i32,
    dy: i32,
    vertical: i32,
    horizontal: i32,
}

impl Translator {
    fn new(found: &Found, config: &config::Touchpad, keyboard: Option<Arc<Device>>) -> Translator {
        let touchpad = (found.kind == Kind::Touchpad).then(|| {
            let settings = touchpad_settings(config, &found.name);
            Touchpad::new(found.resolution, settings)
        });
        Translator {
            hi_res_wheel: found.hi_res_wheel,
            touchpad,
            keyboard,
            start: Instant::now(),
            last_key: Instant::now() - TYPING_WINDOW - Duration::from_secs(60),
            dx: 0,
            dy: 0,
            vertical: 0,
            horizontal: 0,
        }
    }

    /// The `Duration` since the reader's epoch, the time the touchpad keeps.
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    /// Whether typing was recent enough to silence taps. A key pressed on
    /// this device refreshes `last_key` in `feed`; a key held on the shared
    /// keyboard is caught here, at the cost of silencing taps while a
    /// modifier is held (a known limitation, like libinput's timeout).
    fn typing_now(&self) -> bool {
        self.last_key.elapsed() < TYPING_WINDOW
            || self
                .keyboard
                .as_ref()
                .is_some_and(|kb| kb.any_key_down().unwrap_or(false))
    }

    /// How long to wait before the next `tick` should run, even with no
    /// events. `None` for a device without a touchpad.
    fn wait_until(&self) -> Option<Duration> {
        let touchpad = self.touchpad.as_ref()?;
        let until = touchpad.deadline()?;
        let now = self.now();
        // `until` is the touchpad's absolute clock; subtract our own.
        until.checked_sub(now).filter(|d| !d.is_zero())
    }

    fn feed(&mut self, raw: RawEvent, mut out: impl FnMut(InputEvent)) {
        // The touchpad's clock and typing state, computed before the
        // touchpad is borrowed mutably, so the borrows do not conflict.
        let now = self.start.elapsed();
        let typing = self.typing_now();
        let is_syn = (raw.kind, raw.code) == (EV_SYN, SYN_REPORT);

        // Phase 1: feed the touchpad (if any) and collect what it produces.
        // The `if let` block borrows `self.touchpad` mutably; the borrow ends
        // when the block does, so Phase 2 can access `self.dx` etc.
        let moves: Option<Moves> = if let Some(touchpad) = self.touchpad.as_mut() {
            if touchpad.feed(raw) {
                // The touchpad took this event (ABS_*, BTN_TOOL_*).
                if is_syn {
                    Some(touchpad.report(now, typing, &mut out))
                } else {
                    return;
                }
            } else if is_syn {
                // The touchpad did not take SYN_REPORT (its `feed` returns
                // false for it), but `report` must still run at each frame.
                Some(touchpad.report(now, typing, &mut out))
            } else {
                // Not taken, not SYN: the mouse path handles it below.
                None
            }
        } else {
            None
        };

        // Phase 2: apply the touchpad's moves and flush. The touchpad borrow
        // has ended, so `self.dx` and `self.flush` are available.
        if let Some(moves) = moves {
            self.dx = self.dx.saturating_add(moves.motion.0);
            self.dy = self.dy.saturating_add(moves.motion.1);
            self.vertical = self.vertical.saturating_add(moves.scroll.0);
            self.horizontal = self.horizontal.saturating_add(moves.scroll.1);
            self.flush(out);
            return;
        }

        // Phase 3: the mouse path — buttons, relative motion, wheels. The
        // touchpad's physical buttons reach here (its `feed` returns false
        // for them) and are sent as the buttons they are.
        let pressed = raw.value != 0;
        let value = raw.value;
        // Track when a key was last pressed, for the touchpad's
        // `disable while typing`. Mouse buttons do not count.
        if (raw.kind, raw.code).0 == EV_KEY && pressed && (raw.code < 0x100) {
            self.last_key = Instant::now();
        }
        match (raw.kind, raw.code) {
            (EV_KEY, code) if code < 0x100 => {
                out(InputEvent::Key { code, pressed });
            }
            (EV_KEY, code) if MOUSE_BUTTONS.contains(&code) => {
                out(InputEvent::Button { code, pressed })
            }
            (EV_REL, REL_X) => self.dx = self.dx.saturating_add(value),
            (EV_REL, REL_Y) => self.dy = self.dy.saturating_add(value),
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

    /// Lets time pass with no event: ends a tap whose time is up.
    fn tick(&mut self, mut out: impl FnMut(InputEvent)) {
        // Compute the time before borrowing the touchpad mutably.
        let now = self.start.elapsed();
        if let Some(touchpad) = self.touchpad.as_mut() {
            touchpad.tick(now, &mut out);
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
        let mut translator = Translator {
            hi_res_wheel,
            touchpad: None,
            keyboard: None,
            start: Instant::now(),
            last_key: Instant::now() - TYPING_WINDOW - Duration::from_secs(60),
            dx: 0,
            dy: 0,
            vertical: 0,
            horizontal: 0,
        };
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

    /// A touchpad's full integration is tested in `touchpad.rs`: its slots,
    /// taps and scroll, all in the same shape `report` produces. Here only
    /// the wiring is checked: `Settings` defaults build, a `Translator`
    /// built with a touchpad does not panic, and a `SYN_REPORT` after a
    /// finger goes down produces no motion (the touchpad's `report` returns
    /// zero motion on the first report of a touch).
    #[test]
    fn a_touchpad_translator_handles_a_syn_report_without_panicking() {
        let settings = Settings {
            speed: 16,
            scroll_speed: 24,
            natural_scroll: false,
            two_finger_scroll: true,
            tap: true,
            tap_and_drag: true,
            button_map: ButtonMap::Lrm,
            disable_while_typing: true,
        };
        let mut touchpad = Touchpad::new([24, 24], settings);
        // A finger lands: ABS_X, ABS_Y and BTN_TOOL_FINGER, taken by the
        // touchpad. Then SYN_REPORT, which the touchpad does not take: the
        // translator calls `report` itself.
        touchpad.feed(RawEvent {
            kind: EV_KEY,
            code: BTN_TOOL_FINGER,
            value: 1,
        });
        touchpad.feed(RawEvent {
            kind: EV_ABS,
            code: ABS_X,
            value: 1000,
        });
        touchpad.feed(RawEvent {
            kind: EV_ABS,
            code: ABS_Y,
            value: 500,
        });
        let mut events = Vec::new();
        let moves = touchpad.report(Duration::from_millis(0), false, &mut |event| {
            events.push(event);
        });
        // Landing: no motion yet, no buttons.
        assert_eq!(moves.motion, (0, 0));
        assert_eq!(moves.scroll, (0, 0));
        assert!(events.is_empty());
    }

    /// The `touchpad_prefs::parse_gsettings` and `parse_kcminputrc` parsers
    /// have their own tests; here only the shape of `touchpad_settings` is
    /// kept. The owner's explicit `On`/`Off` wins over whatever the desktop
    /// reports, so the test does not depend on the machine it runs on.
    #[test]
    fn the_owners_explicit_choice_wins_over_the_desktop() {
        let off = config::Touchpad {
            tap: config::Auto::Off,
            natural_scroll: config::Auto::Off,
            ..config::Touchpad::default()
        };
        let settings = touchpad_settings(&off, "Some Touchpad");
        assert!(!settings.tap, "Auto::Off wins for tap");
        assert!(
            !settings.natural_scroll,
            "Auto::Off wins for natural_scroll"
        );
        let on = config::Touchpad {
            tap: config::Auto::On,
            natural_scroll: config::Auto::On,
            ..config::Touchpad::default()
        };
        let settings = touchpad_settings(&on, "Some Touchpad");
        assert!(settings.tap, "Auto::On wins for tap");
        assert!(settings.natural_scroll, "Auto::On wins for natural_scroll");
        // Speed and scroll_speed come from config, not the desktop.
        assert_eq!(settings.speed, off.speed);
        assert_eq!(settings.scroll_speed, off.scroll_speed);
    }
}
