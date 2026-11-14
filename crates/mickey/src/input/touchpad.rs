//! A touchpad turned into what a mouse sends: motion, scroll and clicks.
//!
//! A touchpad reports where its fingers are, not what they mean. On a PC
//! of its own, libinput (inside the compositor) reads those fingers and
//! makes motion, two-finger scrolling and taps out of them. But while
//! Mickey controls another PC it *grabs* the device, and then libinput
//! gets nothing: Mickey has to do that reading itself, with the owner's
//! choices (`Settings`, from the desktop's own settings or config.toml).
//! The grab is also why this never fights libinput: whenever this code
//! runs for real, libinput is not looking at the device.
//!
//! The rules follow libinput's defaults: a tap is a touch under 180 ms
//! that moves under 3 mm; one finger taps left, two right, three middle;
//! a second touch right after a tap drags with the button held.

use crate::input::event::InputEvent;
use crate::platform::linux::RawEvent;
use std::time::Duration;

const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TOOL_TYPE: u16 = 0x37;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const MT_TOOL_PALM: i32 = 2;

pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
const BTN_TOOL_FINGER: u16 = 0x145;
const BTN_TOOL_QUINTTAP: u16 = 0x148;
const BTN_TOOL_DOUBLETAP: u16 = 0x14d;
const BTN_TOOL_TRIPLETAP: u16 = 0x14e;
const BTN_TOOL_QUADTAP: u16 = 0x14f;

/// Touch slots tracked; a touchpad has ten at most.
const SLOTS: usize = 16;

/// A touch longer than this is not a tap (libinput: 180 ms).
const TAP_TIMEOUT: Duration = Duration::from_millis(180);
/// After a tap, how long a new touch still counts as dragging it, and when
/// the click is let go if none comes (libinput: 180 ms).
const DRAG_TIMEOUT: Duration = Duration::from_millis(180);
/// A touch that moves farther than this is not a tap, in tenths of a
/// millimetre (libinput: 3 mm).
const TAP_MOVE_TENTH_MM: i64 = 30;
/// Fingers that jump farther than this between two reports are a finger
/// that changed (a palm, a lifted finger), not movement, in millimetres.
const MAX_JUMP_MM: i32 = 25;

/// What one finger count means for a tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonMap {
    /// One finger left, two right, three middle.
    Lrm,
    /// One finger left, two middle, three right.
    Lmr,
}

/// Everything the owner can choose, already decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Mouse counts per millimetre of one finger's travel.
    pub speed: i32,
    /// Wheel units (120 per notch) per millimetre of two-finger travel.
    pub scroll_speed: i32,
    pub natural_scroll: bool,
    pub two_finger_scroll: bool,
    pub tap: bool,
    pub tap_and_drag: bool,
    pub button_map: ButtonMap,
    /// Taps are ignored for a moment after typing (palms rest on the pad).
    pub disable_while_typing: bool,
}

/// What a report asks the cursor and the wheel to do.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Moves {
    pub motion: (i32, i32),
    /// Vertical and horizontal wheel units, positive up and right.
    pub scroll: (i32, i32),
}

#[derive(Debug, Clone, Copy, Default)]
struct Slot {
    active: bool,
    palm: bool,
    position: [i32; 2],
}

/// Where a touch is on its way to being a tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tap {
    Idle,
    /// Fingers are down and may still become a tap; `fingers` is the most
    /// there were at once.
    Touch {
        since: Duration,
        fingers: u8,
    },
    /// A tap was made: its button is down, waiting to see whether a new
    /// touch drags it, or to let go at `until`.
    Tapped {
        button: u16,
        until: Duration,
    },
    /// A new touch right after a one-finger tap: a second tap if it is
    /// lifted at once, a drag if it stays or moves.
    Again {
        button: u16,
        since: Duration,
    },
    /// The button is held while the finger moves; lifting lets it go.
    Drag {
        button: u16,
    },
    /// Not a tap (it moved, took long, was a click or came while typing):
    /// nothing until every finger is up.
    Ignore,
}

pub struct Touchpad {
    settings: Settings,
    /// Device units per millimetre on X and Y.
    resolution: [i32; 2],
    /// Which `BTN_TOOL_*` are down: finger, double, triple, quad, quint tap.
    tools: [bool; 5],
    slots: [Slot; SLOTS],
    slot: usize,
    multitouch: bool,
    /// `ABS_X`/`ABS_Y`: the one finger (or the middle of the box) on
    /// devices without slots.
    position: [i32; 2],
    /// Where the fingers were at the previous report and at the start of
    /// this stretch with the same number of them; `None` right after the
    /// number changes, so landing or lifting never makes a jump.
    last: Option<[i32; 2]>,
    origin: [i32; 2],
    fingers: u8,
    /// What the integer divisions left over, so slow movement is not lost.
    motion_rest: [i32; 2],
    scroll_rest: [i32; 2],
    tap: Tap,
    /// A physical button went down since the fingers landed.
    clicked: bool,
}

impl Touchpad {
    pub fn new(resolution: [i32; 2], settings: Settings) -> Touchpad {
        Touchpad {
            settings,
            resolution,
            tools: [false; 5],
            slots: [Slot::default(); SLOTS],
            slot: 0,
            multitouch: false,
            position: [0; 2],
            last: None,
            origin: [0; 2],
            fingers: 0,
            motion_rest: [0; 2],
            scroll_rest: [0; 2],
            tap: Tap::Idle,
            clicked: false,
        }
    }

    /// Takes the events only a touchpad sends; `false` for anything else.
    /// A physical click is noticed (a click is not a tap) but still goes on
    /// to be sent as the button it is.
    pub fn feed(&mut self, raw: RawEvent) -> bool {
        match (raw.kind, raw.code) {
            (EV_ABS, ABS_X) => self.position[0] = raw.value,
            (EV_ABS, ABS_Y) => self.position[1] = raw.value,
            (EV_ABS, ABS_MT_SLOT) => {
                self.slot = usize::try_from(raw.value).unwrap_or(0).min(SLOTS - 1)
            }
            (EV_ABS, ABS_MT_TRACKING_ID) => {
                self.multitouch = true;
                let slot = &mut self.slots[self.slot];
                slot.active = raw.value >= 0;
                slot.palm = false;
            }
            (EV_ABS, ABS_MT_POSITION_X) => {
                self.multitouch = true;
                self.slots[self.slot].position[0] = raw.value;
            }
            (EV_ABS, ABS_MT_POSITION_Y) => {
                self.multitouch = true;
                self.slots[self.slot].position[1] = raw.value;
            }
            (EV_ABS, ABS_MT_TOOL_TYPE) => self.slots[self.slot].palm = raw.value == MT_TOOL_PALM,
            (EV_ABS, _) => {}
            (EV_KEY, BTN_TOOL_FINGER) => self.tools[0] = raw.value != 0,
            (EV_KEY, BTN_TOOL_DOUBLETAP) => self.tools[1] = raw.value != 0,
            (EV_KEY, BTN_TOOL_TRIPLETAP) => self.tools[2] = raw.value != 0,
            (EV_KEY, BTN_TOOL_QUADTAP) => self.tools[3] = raw.value != 0,
            (EV_KEY, BTN_TOOL_QUINTTAP) => self.tools[4] = raw.value != 0,
            (EV_KEY, BTN_LEFT | BTN_RIGHT | BTN_MIDDLE) => {
                self.clicked |= raw.value != 0;
                return false;
            }
            _ => return false,
        }
        true
    }

    /// When the next tap timer fires, if one is running: the reader waits
    /// at most until then, since a touchpad sends nothing while still.
    pub fn deadline(&self) -> Option<Duration> {
        match self.tap {
            Tap::Tapped { until, .. } => Some(until),
            _ => None,
        }
    }

    /// Lets time pass with no event: ends a tap whose time is up.
    pub fn tick(&mut self, now: Duration, out: &mut impl FnMut(InputEvent)) {
        if let Tap::Tapped { button, until } = self.tap
            && now >= until
        {
            out(button_event(button, false));
            self.tap = Tap::Idle;
        }
    }

    /// At `SYN_REPORT`: everything this report added up to. Buttons (taps)
    /// go to `out` at once and in order; motion and scroll are returned, to
    /// be summed with the rest of the device's report. `typing` says a key
    /// was pressed a moment ago.
    pub fn report(
        &mut self,
        now: Duration,
        typing: bool,
        out: &mut impl FnMut(InputEvent),
    ) -> Moves {
        let fingers = self.finger_count();
        if fingers != self.fingers {
            self.fingers = fingers;
            self.last = None;
            self.motion_rest = [0; 2];
            self.scroll_rest = [0; 2];
        }
        let mut moves = Moves::default();
        let mut moved_far = false;
        if fingers > 0 {
            let here = self.centre();
            match self.last.replace(here) {
                None => self.origin = here,
                Some(last) => {
                    let delta = [here[0] - last[0], here[1] - last[1]];
                    if self.jumped(delta) {
                        // Not movement: start again from here.
                        self.origin = here;
                    } else {
                        match fingers {
                            1 => moves.motion = self.move_cursor(delta),
                            2 if self.settings.two_finger_scroll => {
                                moves.scroll = self.scroll(delta)
                            }
                            _ => {}
                        }
                    }
                    moved_far = self.farther_than_a_tap(here);
                }
            }
        } else {
            self.last = None;
        }
        self.step_tap(now, fingers, moved_far, typing, out);
        if fingers == 0 {
            self.clicked = false;
        }
        moves
    }

    /// How many fingers are on the pad: the `BTN_TOOL_*` that is down, less
    /// the palms. Those bits are exact where slots are not (a device may
    /// have two slots and report three fingers).
    fn finger_count(&self) -> u8 {
        let tools = self
            .tools
            .iter()
            .rposition(|&down| down)
            .map_or(0, |index| index as u8 + 1);
        let palms = self
            .slots
            .iter()
            .filter(|slot| slot.active && slot.palm)
            .count() as u8;
        tools.saturating_sub(palms)
    }

    /// The middle of the fingers, in device units.
    fn centre(&self) -> [i32; 2] {
        let live: Vec<&Slot> = self
            .slots
            .iter()
            .filter(|slot| slot.active && !slot.palm)
            .collect();
        if !self.multitouch || live.is_empty() {
            return self.position;
        }
        let n = live.len() as i64;
        [0, 1].map(|axis| {
            (live
                .iter()
                .map(|slot| i64::from(slot.position[axis]))
                .sum::<i64>()
                / n) as i32
        })
    }

    fn jumped(&self, delta: [i32; 2]) -> bool {
        (0..2).any(|axis| delta[axis].abs() > MAX_JUMP_MM * self.resolution[axis])
    }

    fn farther_than_a_tap(&self, here: [i32; 2]) -> bool {
        let tenths = |axis: usize| {
            i64::from(here[axis] - self.origin[axis]) * 10 / i64::from(self.resolution[axis])
        };
        tenths(0).pow(2) + tenths(1).pow(2) > TAP_MOVE_TENTH_MM.pow(2)
    }

    /// One finger: the difference between two reports, scaled from
    /// millimetres to mouse counts.
    fn move_cursor(&mut self, delta: [i32; 2]) -> (i32, i32) {
        let moved = self.scale(delta, self.settings.speed, true);
        (moved[0], moved[1])
    }

    /// Two fingers: wheel units. Fingers moving down scroll down, unless
    /// natural scrolling makes the page follow the fingers.
    fn scroll(&mut self, delta: [i32; 2]) -> (i32, i32) {
        let units = self.scale(delta, self.settings.scroll_speed, false);
        // Wheel up and right are positive; a finger moving down is +Y.
        let sign = if self.settings.natural_scroll { 1 } else { -1 };
        (sign * units[1], -sign * units[0])
    }

    /// `delta` device units to `per_mm` units per millimetre, keeping what
    /// is left over for the next report.
    fn scale(&mut self, delta: [i32; 2], per_mm: i32, motion: bool) -> [i32; 2] {
        let rest = if motion {
            &mut self.motion_rest
        } else {
            &mut self.scroll_rest
        };
        let mut done = [0; 2];
        for axis in 0..2 {
            let scaled = i64::from(delta[axis]) * i64::from(per_mm) + i64::from(rest[axis]);
            let resolution = i64::from(self.resolution[axis]);
            done[axis] = (scaled / resolution) as i32;
            rest[axis] = (scaled % resolution) as i32;
        }
        done
    }

    /// The tap's state machine, once per report.
    fn step_tap(
        &mut self,
        now: Duration,
        fingers: u8,
        moved_far: bool,
        typing: bool,
        out: &mut impl FnMut(InputEvent),
    ) {
        if !self.settings.tap {
            return;
        }
        let typing = typing && self.settings.disable_while_typing;
        self.tick(now, out);
        let click = |button: u16, out: &mut dyn FnMut(InputEvent)| {
            out(button_event(button, true));
            out(button_event(button, false));
        };
        self.tap = match self.tap {
            Tap::Idle if fingers > 0 && typing => Tap::Ignore,
            Tap::Idle if fingers > 0 => Tap::Touch {
                since: now,
                fingers,
            },
            Tap::Idle => Tap::Idle,
            Tap::Touch {
                since,
                fingers: most,
            } => {
                let most = most.max(fingers);
                if self.clicked || moved_far || now - since > TAP_TIMEOUT || most > 3 {
                    Tap::Ignore
                } else if fingers > 0 {
                    Tap::Touch {
                        since,
                        fingers: most,
                    }
                } else if let (Some(button), false) = (self.button_for(most), typing) {
                    // Lifted in time and without moving: a tap.
                    if self.settings.tap_and_drag && button == BTN_LEFT {
                        // Down now, up when the time for a drag is over.
                        out(button_event(button, true));
                        Tap::Tapped {
                            button,
                            until: now + DRAG_TIMEOUT,
                        }
                    } else {
                        click(button, out);
                        Tap::Idle
                    }
                } else {
                    Tap::Idle
                }
            }
            Tap::Tapped { button, .. } if fingers > 0 => {
                if typing {
                    out(button_event(button, false));
                    Tap::Ignore
                } else {
                    Tap::Again { button, since: now }
                }
            }
            tapped @ Tap::Tapped { .. } => tapped,
            Tap::Again { button, since } => {
                if self.clicked || fingers > 1 {
                    out(button_event(button, false));
                    Tap::Ignore
                } else if fingers == 0 {
                    // Lifted at once without moving: a second tap, which
                    // is a double click (let go, press again).
                    out(button_event(button, false));
                    out(button_event(button, true));
                    Tap::Tapped {
                        button,
                        until: now + DRAG_TIMEOUT,
                    }
                } else if moved_far || now - since > TAP_TIMEOUT {
                    Tap::Drag { button }
                } else {
                    Tap::Again { button, since }
                }
            }
            Tap::Drag { button } => {
                if fingers == 0 || self.clicked {
                    out(button_event(button, false));
                    if fingers == 0 { Tap::Idle } else { Tap::Ignore }
                } else {
                    Tap::Drag { button }
                }
            }
            Tap::Ignore if fingers == 0 => Tap::Idle,
            Tap::Ignore => Tap::Ignore,
        };
    }

    /// The button a tap of `fingers` makes; none for four or five.
    fn button_for(&self, fingers: u8) -> Option<u16> {
        match (fingers, self.settings.button_map) {
            (1, _) => Some(BTN_LEFT),
            (2, ButtonMap::Lrm) | (3, ButtonMap::Lmr) => Some(BTN_RIGHT),
            (3, ButtonMap::Lrm) | (2, ButtonMap::Lmr) => Some(BTN_MIDDLE),
            _ => None,
        }
    }
}

fn button_event(code: u16, pressed: bool) -> InputEvent {
    InputEvent::Button { code, pressed }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn settings() -> Settings {
        Settings {
            speed: 16,
            scroll_speed: 24,
            natural_scroll: false,
            two_finger_scroll: true,
            tap: true,
            tap_and_drag: true,
            button_map: ButtonMap::Lrm,
            disable_while_typing: true,
        }
    }

    /// A pad with 24 units per millimetre.
    struct Pad {
        touchpad: Touchpad,
        out: Vec<InputEvent>,
        moves: Vec<Moves>,
    }

    impl Pad {
        fn new(settings: Settings) -> Pad {
            Pad {
                touchpad: Touchpad::new([24, 24], settings),
                out: Vec::new(),
                moves: Vec::new(),
            }
        }

        /// Feeds `events` and then a `SYN_REPORT` at `at` ms.
        fn frame(&mut self, at: u64, events: &[(u16, u16, i32)]) {
            self.frame_typing(at, false, events);
        }

        fn frame_typing(&mut self, at: u64, typing: bool, events: &[(u16, u16, i32)]) {
            for &(kind, code, value) in events {
                self.touchpad.feed(RawEvent { kind, code, value });
            }
            let out = &mut self.out;
            let moves = self
                .touchpad
                .report(MS(at), typing, &mut |event| out.push(event));
            self.moves.push(moves);
        }

        fn wait(&mut self, at: u64) {
            let out = &mut self.out;
            self.touchpad.tick(MS(at), &mut |event| out.push(event));
        }

        fn scrolled(&self) -> (i32, i32) {
            self.moves
                .iter()
                .fold((0, 0), |(v, h), m| (v + m.scroll.0, h + m.scroll.1))
        }

        fn moved(&self) -> (i32, i32) {
            self.moves
                .iter()
                .fold((0, 0), |(x, y), m| (x + m.motion.0, y + m.motion.1))
        }

        fn buttons(&self) -> Vec<(u16, bool)> {
            self.out
                .iter()
                .filter_map(|event| match event {
                    InputEvent::Button { code, pressed } => Some((*code, *pressed)),
                    _ => None,
                })
                .collect()
        }
    }

    const ONE: (u16, u16, i32) = (EV_KEY, BTN_TOOL_FINGER, 1);
    const TWO: (u16, u16, i32) = (EV_KEY, BTN_TOOL_DOUBLETAP, 1);
    const THREE: (u16, u16, i32) = (EV_KEY, BTN_TOOL_TRIPLETAP, 1);
    const NONE: [(u16, u16, i32); 5] = [
        (EV_KEY, BTN_TOOL_FINGER, 0),
        (EV_KEY, BTN_TOOL_DOUBLETAP, 0),
        (EV_KEY, BTN_TOOL_TRIPLETAP, 0),
        (EV_KEY, BTN_TOOL_QUADTAP, 0),
        (EV_KEY, BTN_TOOL_QUINTTAP, 0),
    ];
    const LEFT: u16 = BTN_LEFT;

    fn at(x: i32, y: i32) -> [(u16, u16, i32); 2] {
        [(EV_ABS, ABS_X, x), (EV_ABS, ABS_Y, y)]
    }

    /// Fingers land at (x, y) and nothing else happens in that report.
    fn land(pad: &mut Pad, time: u64, finger: (u16, u16, i32), x: i32, y: i32) {
        let mut events = vec![finger];
        events.extend(at(x, y));
        pad.frame(time, &events);
    }

    // ----- one finger: the cursor -----

    #[test]
    fn one_finger_moves_the_cursor_without_a_first_jump() {
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, ONE, 1000, 500);
        assert_eq!(pad.moved(), (0, 0), "landing: no motion yet");
        pad.frame(10, &[(EV_ABS, ABS_X, 1024)]);
        assert_eq!(pad.moved(), (16, 0), "1 mm right is `speed` counts");
    }

    #[test]
    fn lifting_and_landing_elsewhere_does_not_jump() {
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, ONE, 100, 100);
        pad.frame(10, &NONE);
        land(&mut pad, 20, ONE, 2000, 900);
        assert_eq!(pad.moved(), (0, 0));
    }

    #[test]
    fn slow_motion_adds_up_through_the_remainder() {
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, ONE, 0, 0);
        for x in 1..=3 {
            pad.frame(10 * x as u64, &[(EV_ABS, ABS_X, x)]);
        }
        // 1 unit is 16/24 of a count: 0, then 1, then 1.
        assert_eq!(pad.moved().0, 2);
    }

    // ----- two fingers: the wheel -----

    /// Two fingers at (x, y), then moved by `steps` of (dx, dy) units.
    fn two_finger_swipe(pad: &mut Pad, steps: &[(i32, i32)]) {
        land(pad, 0, TWO, 1000, 1000);
        let (mut x, mut y) = (1000, 1000);
        for (n, (dx, dy)) in steps.iter().enumerate() {
            x += dx;
            y += dy;
            pad.frame(8 * (n as u64 + 1), &at(x, y));
        }
    }

    #[test]
    fn two_fingers_scroll_vertically_and_do_not_move_the_cursor() {
        let mut pad = Pad::new(settings());
        // 10 mm down, in 10 steps.
        two_finger_swipe(&mut pad, &[(0, 24); 10]);
        assert_eq!(pad.moved(), (0, 0));
        // Traditional: fingers down scroll the page down: wheel negative.
        // 10 mm * 24 units per mm.
        assert_eq!(pad.scrolled(), (-240, 0));
    }

    #[test]
    fn natural_scrolling_reverses_both_directions() {
        let mut pad = Pad::new(Settings {
            natural_scroll: true,
            ..settings()
        });
        two_finger_swipe(&mut pad, &[(24, 24); 5]);
        // The page follows the fingers: down moves it down (wheel up) and
        // right moves it right (wheel left).
        assert_eq!(pad.scrolled(), (120, -120));
    }

    #[test]
    fn horizontal_scroll_goes_right_when_the_fingers_go_right() {
        let mut pad = Pad::new(settings());
        two_finger_swipe(&mut pad, &[(24, 0); 5]);
        assert_eq!(pad.scrolled(), (0, 120));
        let mut pad = Pad::new(settings());
        two_finger_swipe(&mut pad, &[(-24, 0); 5]);
        assert_eq!(pad.scrolled(), (0, -120));
    }

    #[test]
    fn slow_scrolling_loses_nothing() {
        let mut pad = Pad::new(settings());
        // 1 unit a report is 1 wheel unit at 24 per 24: no loss either way,
        // so use a slower scroll speed: 5/24 of a unit per device unit.
        let mut pad2 = Pad::new(Settings {
            scroll_speed: 5,
            ..settings()
        });
        two_finger_swipe(&mut pad, &[(0, 1); 48]);
        two_finger_swipe(&mut pad2, &[(0, 1); 48]);
        assert_eq!(pad.scrolled().0, -48);
        // 48 units * 5 / 24 = 10 exactly, though no report reached 1 whole unit.
        assert_eq!(pad2.scrolled().0, -10);
    }

    #[test]
    fn the_scroll_speed_is_the_owners() {
        let mut slow = Pad::new(Settings {
            scroll_speed: 12,
            ..settings()
        });
        two_finger_swipe(&mut slow, &[(0, 24); 10]);
        assert_eq!(slow.scrolled().0, -120, "10 mm at 12 per mm");
    }

    #[test]
    fn a_second_finger_landing_does_not_jump_the_page() {
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, ONE, 1000, 1000);
        // The second finger lands 20 mm away: the middle shifts 10 mm.
        pad.frame(
            8,
            &[TWO, (EV_KEY, BTN_TOOL_FINGER, 0), (EV_ABS, ABS_X, 1240)],
        );
        pad.frame(16, &[(EV_ABS, ABS_X, 1241)]);
        // Only the one unit after landing counts, not the 10 mm the middle
        // shifted when the second finger came down.
        assert_eq!(pad.scrolled(), (0, 1));
    }

    #[test]
    fn three_fingers_and_a_jump_do_nothing() {
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, THREE, 1000, 1000);
        pad.frame(8, &at(1500, 1500));
        assert_eq!((pad.moved(), pad.scrolled()), ((0, 0), (0, 0)));
        // A finger that jumps 40 mm in one report is a different finger.
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, TWO, 1000, 1000);
        pad.frame(8, &at(1000, 2000));
        assert_eq!(pad.scrolled(), (0, 0));
        pad.frame(16, &at(1000, 2024));
        assert_eq!(pad.scrolled(), (-24, 0), "scrolling goes on from there");
    }

    #[test]
    fn scrolling_can_be_turned_off() {
        let mut pad = Pad::new(Settings {
            two_finger_scroll: false,
            ..settings()
        });
        two_finger_swipe(&mut pad, &[(0, 24); 4]);
        assert_eq!(pad.scrolled(), (0, 0));
    }

    // ----- multitouch slots -----

    fn slot(n: i32, id: i32, x: i32, y: i32) -> Vec<(u16, u16, i32)> {
        vec![
            (EV_ABS, ABS_MT_SLOT, n),
            (EV_ABS, ABS_MT_TRACKING_ID, id),
            (EV_ABS, ABS_MT_POSITION_X, x),
            (EV_ABS, ABS_MT_POSITION_Y, y),
        ]
    }

    #[test]
    fn slots_scroll_by_the_middle_of_the_fingers() {
        let mut pad = Pad::new(settings());
        let mut start = vec![TWO];
        start.extend(slot(0, 1, 1000, 1000));
        start.extend(slot(1, 2, 1100, 1000));
        pad.frame(0, &start);
        // Both fingers move 24 units (1 mm) down.
        let mut next = slot(0, 1, 1000, 1024);
        next.extend(slot(1, 2, 1100, 1024));
        pad.frame(8, &next);
        assert_eq!(pad.scrolled(), (-24, 0));
        // One finger alone moving would not: the middle moves half as far.
        let mut half = vec![(EV_ABS, ABS_MT_SLOT, 0), (EV_ABS, ABS_MT_POSITION_Y, 1048)];
        half.push((EV_ABS, ABS_MT_SLOT, 1));
        pad.frame(16, &half);
        assert_eq!(pad.scrolled(), (-36, 0));
    }

    #[test]
    fn a_palm_is_not_a_finger() {
        let mut pad = Pad::new(settings());
        let mut raw = vec![TWO];
        raw.extend(slot(0, 1, 1000, 1000));
        raw.extend(slot(1, 2, 3000, 3000));
        raw.push((EV_ABS, ABS_MT_TOOL_TYPE, MT_TOOL_PALM));
        pad.frame(0, &raw);
        // One real finger and a palm: it is one finger, so it moves the cursor.
        pad.frame(
            8,
            &[(EV_ABS, ABS_MT_SLOT, 0), (EV_ABS, ABS_MT_POSITION_X, 1024)],
        );
        assert_eq!(pad.moved(), (16, 0));
        assert_eq!(pad.scrolled(), (0, 0));
    }

    // ----- taps -----

    fn tap(pad: &mut Pad, start: u64, finger: (u16, u16, i32), hold: u64) {
        land(pad, start, finger, 1000, 1000);
        pad.frame(start + hold, &NONE);
    }

    #[test]
    fn one_finger_tap_is_a_left_click() {
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, ONE, 80);
        assert_eq!(pad.buttons(), [(LEFT, true)], "down at once...");
        pad.wait(80 + 179);
        assert_eq!(pad.buttons().len(), 1, "...still down until the time is up");
        assert_eq!(pad.touchpad.deadline(), Some(MS(260)));
        pad.wait(80 + 180);
        assert_eq!(pad.buttons(), [(LEFT, true), (LEFT, false)]);
        assert_eq!(pad.touchpad.deadline(), None);
    }

    #[test]
    fn without_tap_and_drag_a_tap_clicks_at_once() {
        let mut pad = Pad::new(Settings {
            tap_and_drag: false,
            ..settings()
        });
        tap(&mut pad, 0, ONE, 80);
        assert_eq!(pad.buttons(), [(LEFT, true), (LEFT, false)]);
        assert_eq!(pad.touchpad.deadline(), None);
    }

    #[test]
    fn two_fingers_tap_right_and_three_middle() {
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, TWO, 80);
        assert_eq!(pad.buttons(), [(BTN_RIGHT, true), (BTN_RIGHT, false)]);
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, THREE, 80);
        assert_eq!(pad.buttons(), [(BTN_MIDDLE, true), (BTN_MIDDLE, false)]);
    }

    #[test]
    fn the_owners_button_map_decides_two_and_three_fingers() {
        let mut pad = Pad::new(Settings {
            button_map: ButtonMap::Lmr,
            ..settings()
        });
        tap(&mut pad, 0, TWO, 80);
        assert_eq!(pad.buttons(), [(BTN_MIDDLE, true), (BTN_MIDDLE, false)]);
        let mut pad = Pad::new(Settings {
            button_map: ButtonMap::Lmr,
            ..settings()
        });
        tap(&mut pad, 0, THREE, 80);
        assert_eq!(pad.buttons(), [(BTN_RIGHT, true), (BTN_RIGHT, false)]);
    }

    #[test]
    fn two_fingers_that_land_one_after_the_other_still_tap_right() {
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, ONE, 1000, 1000);
        pad.frame(30, &[TWO, (EV_KEY, BTN_TOOL_FINGER, 0)]);
        pad.frame(40, &[(EV_KEY, BTN_TOOL_FINGER, 1)]);
        pad.frame(100, &NONE);
        assert_eq!(pad.buttons(), [(BTN_RIGHT, true), (BTN_RIGHT, false)]);
    }

    #[test]
    fn taps_are_off_unless_the_owner_has_them_on() {
        let mut pad = Pad::new(Settings {
            tap: false,
            ..settings()
        });
        tap(&mut pad, 0, ONE, 80);
        pad.wait(1000);
        assert_eq!(pad.buttons(), []);
    }

    #[test]
    fn a_long_touch_is_not_a_tap() {
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, ONE, 181);
        pad.wait(2000);
        assert_eq!(pad.buttons(), []);
    }

    #[test]
    fn a_touch_that_moves_is_not_a_tap_but_a_small_wobble_is() {
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, ONE, 1000, 1000);
        pad.frame(10, &at(1040, 1000)); // 1.7 mm
        pad.frame(60, &NONE);
        assert_eq!(pad.buttons(), [(LEFT, true)], "still a tap");
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, ONE, 1000, 1000);
        pad.frame(10, &at(1100, 1000)); // 4.2 mm
        pad.frame(60, &NONE);
        pad.wait(1000);
        assert_eq!(pad.buttons(), []);
    }

    #[test]
    fn a_scroll_is_not_a_two_finger_tap() {
        let mut pad = Pad::new(settings());
        two_finger_swipe(&mut pad, &[(0, 24); 5]);
        pad.frame(60, &NONE);
        pad.wait(1000);
        assert_eq!(pad.buttons(), []);
    }

    #[test]
    fn a_physical_click_is_never_also_a_tap() {
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, ONE, 1000, 1000);
        pad.frame(20, &[(EV_KEY, BTN_LEFT, 1)]);
        pad.frame(60, &[(EV_KEY, BTN_LEFT, 0)]);
        pad.frame(70, &NONE);
        pad.wait(1000);
        assert_eq!(
            pad.buttons(),
            [],
            "the pad's own button is sent by the capture"
        );
        // And the next touch is clean again.
        tap(&mut pad, 2000, ONE, 50);
        assert_eq!(pad.buttons(), [(LEFT, true)]);
    }

    #[test]
    fn a_tap_right_after_typing_is_a_palm() {
        let mut pad = Pad::new(settings());
        land(&mut pad, 0, ONE, 1000, 1000);
        pad.frame_typing(50, true, &NONE);
        pad.wait(1000);
        assert_eq!(pad.buttons(), []);
        // Where the owner has turned that off, it taps.
        let mut pad = Pad::new(Settings {
            disable_while_typing: false,
            ..settings()
        });
        land(&mut pad, 0, ONE, 1000, 1000);
        pad.frame_typing(50, true, &NONE);
        assert_eq!(pad.buttons(), [(LEFT, true)]);
    }

    #[test]
    fn two_taps_in_a_row_are_a_double_click_not_a_drag() {
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, ONE, 60);
        // The second touch comes 100 ms later and is lifted at once.
        land(&mut pad, 160, ONE, 1000, 1000);
        pad.frame(220, &NONE);
        pad.wait(220 + 180);
        assert_eq!(
            pad.buttons(),
            [(LEFT, true), (LEFT, false), (LEFT, true), (LEFT, false)]
        );
    }

    #[test]
    fn touch_after_a_tap_that_stays_drags_with_the_button_held() {
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, ONE, 60);
        land(&mut pad, 160, ONE, 1000, 1000);
        // The finger moves 5 mm: a drag. The button was down since the tap.
        pad.frame(180, &at(1120, 1000));
        pad.frame(400, &at(1240, 1000));
        pad.wait(5000);
        assert_eq!(pad.buttons(), [(LEFT, true)], "held all through the drag");
        assert!(pad.moved().0 > 0, "and the cursor moves with it");
        pad.frame(500, &NONE);
        assert_eq!(
            pad.buttons(),
            [(LEFT, true), (LEFT, false)],
            "lifting drops it"
        );
    }

    #[test]
    fn a_drag_that_stays_still_for_a_moment_also_holds() {
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, ONE, 60);
        land(&mut pad, 160, ONE, 1000, 1000);
        pad.frame(400, &[]);
        pad.frame(900, &NONE);
        assert_eq!(pad.buttons(), [(LEFT, true), (LEFT, false)]);
    }

    #[test]
    fn a_late_second_touch_is_a_new_tap_not_a_drag() {
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, ONE, 60);
        pad.wait(60 + 181);
        assert_eq!(pad.buttons(), [(LEFT, true), (LEFT, false)]);
        tap(&mut pad, 1000, ONE, 60);
        pad.wait(2000);
        assert_eq!(pad.buttons().len(), 4, "two separate clicks");
    }

    #[test]
    fn a_second_tap_with_two_fingers_after_a_tap_lets_the_first_go() {
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, ONE, 60);
        land(&mut pad, 120, ONE, 1000, 1000);
        pad.frame(130, &[TWO, (EV_KEY, BTN_TOOL_FINGER, 0)]);
        assert_eq!(pad.buttons(), [(LEFT, true), (LEFT, false)]);
    }

    #[test]
    fn pending_taps_release_their_button_when_taps_end() {
        // The reader stops at a disconnect: nothing may stay pressed. The
        // capture releases via `tick` once the time is up.
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, ONE, 60);
        assert!(pad.touchpad.deadline().is_some());
        pad.wait(10_000);
        assert_eq!(pad.buttons().last(), Some(&(LEFT, false)));
    }

    #[test]
    fn four_fingers_make_no_click() {
        let mut pad = Pad::new(settings());
        tap(&mut pad, 0, (EV_KEY, BTN_TOOL_QUADTAP, 1), 60);
        pad.wait(1000);
        assert_eq!(pad.buttons(), []);
    }

    #[test]
    fn a_non_touchpad_event_is_left_for_the_capture() {
        let mut touchpad = Touchpad::new([24, 24], settings());
        let raw = |kind, code, value| RawEvent { kind, code, value };
        assert!(touchpad.feed(raw(EV_ABS, ABS_X, 5)));
        assert!(touchpad.feed(raw(EV_KEY, BTN_TOOL_FINGER, 1)));
        assert!(
            !touchpad.feed(raw(EV_KEY, BTN_LEFT, 1)),
            "a click goes on as a button"
        );
        assert!(!touchpad.feed(raw(EV_KEY, 30, 1)));
    }
}
