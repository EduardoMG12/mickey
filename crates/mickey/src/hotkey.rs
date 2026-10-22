//! Hotkeys (PRD 0007): several chords such as `AltGr+RightShift` (next PC
//! or the switcher), `AltGr+RightShift+1` (straight to PC 1) or
//! `AltGr+RightShift+0` (home). A hotkey fires when all its keys are
//! released and nothing else was pressed in between.
//!
//! While the focus is remote, a press that may belong to a hotkey is held
//! back until it is clear what it is part of. If something else is pressed
//! first, the held keys are sent after all, so AltGr+Q, Shift+A or Shift+1
//! still type on the target; if a hotkey completes, the target sees none of
//! it.

use crate::event::InputEvent;
use mickey_config::HotkeySet;
use mickey_config::chord::{self, Chord};
use std::time::{Duration, Instant};

/// Evdev codes of the two keys the escape uses.
pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_RIGHTALT: u16 = 100;

/// What a hotkey asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// The M0 hotkey: toggle with one peer, the switcher with more.
    Next,
    Home,
    Switcher,
    /// Straight to this peer (index in config order).
    Go(usize),
    /// The next or previous PC in the list, this PC included.
    Forward,
    Back,
}

/// One hotkey key pressed during the current attempt.
#[derive(Clone, Copy)]
struct Pressed {
    code: u16,
    /// Physically down right now.
    down: bool,
    /// Its press was forwarded, so its release must be too.
    sent: bool,
}

pub struct Hotkey {
    chords: Vec<(Chord, Trigger)>,
    /// Tapping this key twice quickly, alone, means `Forward`.
    double_tap: Option<u16>,
    /// Two taps closer than this are a double tap (config.toml).
    window: Duration,
    /// When it was last tapped alone, while waiting for a second tap.
    last_tap: Option<Instant>,
    /// The hotkey keys pressed since the first one went down, in order.
    attempt: Vec<Pressed>,
    /// Something else was pressed: this attempt can no longer fire.
    spoiled: bool,
    /// The way home that no setting changes.
    escape: Escape,
}

/// How long AltGr and Right Shift must be held, alone, to come home.
pub const ESCAPE_HOLD: Duration = Duration::from_secs(2);

/// The way home that no setting changes (CLAUDE.md rule 8, #182): hold
/// AltGr and Right Shift together for `ESCAPE_HOLD`, nothing else, and let
/// go. Watched apart from the configured hotkeys, so a shared or broken
/// setting cannot take it away; the default hotkeys use the same keys, so
/// no other key is held back while away.
#[derive(Debug, Default)]
struct Escape {
    /// AltGr, Right Shift.
    down: [bool; 2],
    /// When both went down.
    since: Option<Instant>,
    /// Both were held long enough, alone.
    held: bool,
    /// Something else was pressed meanwhile.
    spoiled: bool,
}

impl Escape {
    /// Whether `event` completes the escape: the last of the two let go
    /// after holding both long enough.
    fn feed(&mut self, event: InputEvent, now: Instant) -> bool {
        let (code, pressed) = match event {
            InputEvent::Key { code, pressed } => (code, pressed),
            InputEvent::Button { pressed: true, .. } => {
                self.spoiled = true;
                return false;
            }
            _ => return false,
        };
        let index = match code {
            KEY_RIGHTALT => 0,
            KEY_RIGHTSHIFT => 1,
            _ => {
                self.spoiled |= pressed;
                return false;
            }
        };
        let both = self.down == [true, true];
        self.down[index] = pressed;
        if pressed && self.down == [true, true] && !both {
            self.since = Some(now);
            self.spoiled = false;
        }
        if !pressed && both {
            // The first of the two let go: was it long enough?
            self.held = self
                .since
                .take()
                .is_some_and(|since| now.duration_since(since) >= ESCAPE_HOLD);
        }
        if self.down == [false, false] {
            let fired = self.held && !self.spoiled;
            self.held = false;
            self.spoiled = false;
            return fired;
        }
        false
    }
}

impl Hotkey {
    /// Only the M0 hotkey.
    #[cfg(test)]
    pub fn new() -> Hotkey {
        Hotkey::with(vec![(default_chord(DEFAULT_NEXT), Trigger::Next)])
    }

    pub fn with(chords: Vec<(Chord, Trigger)>) -> Hotkey {
        Hotkey {
            chords,
            double_tap: None,
            window: Duration::from_millis(mickey_config::DEFAULT_DOUBLE_TAP_MS),
            last_tap: None,
            attempt: Vec::new(),
            spoiled: false,
            escape: Escape::default(),
        }
    }

    /// The hotkeys from `config.toml`, plus the defaults the owner asked
    /// for where a setting is empty: `…+0` home and `…+1` to `…+9` for the
    /// first nine peers. A default that clashes with a setting is skipped.
    pub fn from_config(set: &HotkeySet) -> Hotkey {
        let mut chords = vec![(set.next.clone(), Trigger::Next)];
        let mut add = |chord: Chord, trigger| {
            if !chords.iter().any(|(known, _)| *known == chord) {
                chords.push((chord, trigger));
            }
        };
        add(set.switcher.clone(), Trigger::Switcher);
        add(set.forward.clone(), Trigger::Forward);
        add(set.back.clone(), Trigger::Back);
        add(
            set.home
                .clone()
                .unwrap_or_else(|| default_chord(&format!("{DEFAULT_NEXT}+0"))),
            Trigger::Home,
        );
        for (peer, chord) in set.peers.iter().enumerate() {
            let chord = match chord {
                Some(chord) => chord.clone(),
                None if peer < 9 => default_chord(&format!("{DEFAULT_NEXT}+{}", peer + 1)),
                None => continue,
            };
            add(chord, Trigger::Go(peer));
        }
        let mut hotkey = Hotkey::with(chords);
        hotkey.double_tap = set.double_tap;
        hotkey.window = set.double_tap_window;
        hotkey
    }

    /// Two taps closer than this are a double tap; recording uses it too.
    pub fn double_tap_window(&self) -> Duration {
        self.window
    }

    /// Feeds one captured event. `forward` receives, in order, every event
    /// the target should get. Returns the hotkey that fired, if any.
    pub fn feed(&mut self, event: InputEvent, forward: impl FnMut(InputEvent)) -> Option<Trigger> {
        self.feed_at(event, Instant::now(), forward)
    }

    /// `feed` with the time given, so tests can control it.
    fn feed_at(
        &mut self,
        event: InputEvent,
        now: Instant,
        forward: impl FnMut(InputEvent),
    ) -> Option<Trigger> {
        let escape = self.escape.feed(event, now);
        let fired = self.match_hotkeys(event, now, forward);
        if escape {
            // Instead of whatever the same keys would do as a hotkey.
            self.attempt.clear();
            self.spoiled = false;
            return Some(Trigger::Home);
        }
        fired
    }

    fn match_hotkeys(
        &mut self,
        event: InputEvent,
        now: Instant,
        mut forward: impl FnMut(InputEvent),
    ) -> Option<Trigger> {
        match event {
            InputEvent::Key {
                code,
                pressed: true,
            } => {
                if let Some(key) = self.attempt.iter().find(|key| key.code == code && key.down) {
                    // Auto-repeat: only meaningful if the target saw the press.
                    if key.sent {
                        forward(event);
                    }
                    return None;
                }
                if !self.spoiled && self.may_continue(code) {
                    self.attempt.push(Pressed {
                        code,
                        down: true,
                        sent: false,
                    });
                    return None;
                }
                if self.attempt.iter().any(|key| key.down) {
                    self.spoil(&mut forward);
                }
                if self.is_hotkey_key(code) {
                    // Tracked so the attempt ends when every key is up.
                    self.attempt.push(Pressed {
                        code,
                        down: true,
                        sent: true,
                    });
                }
                forward(event);
                None
            }
            InputEvent::Key {
                code,
                pressed: false,
            } => {
                let Some(key) = self
                    .attempt
                    .iter_mut()
                    .find(|key| key.code == code && key.down)
                else {
                    forward(event);
                    return None;
                };
                key.down = false;
                if key.sent {
                    forward(event);
                }
                if self.attempt.iter().any(|key| key.down) {
                    return None;
                }
                self.finish(now, &mut forward)
            }
            InputEvent::Button { pressed: true, .. } | InputEvent::Scroll { .. } => {
                if self.attempt.iter().any(|key| key.down) {
                    self.spoil(&mut forward);
                }
                forward(event);
                None
            }
            _ => {
                forward(event);
                None
            }
        }
    }

    /// Whether pressing `code` now can still lead to some hotkey: as a
    /// modifier while only modifiers of that chord are down, or as its last
    /// key once exactly its modifiers are down.
    fn may_continue(&self, code: u16) -> bool {
        // The double-tap key, alone, may be the start of a double tap.
        if Some(code) == self.double_tap && self.attempt.iter().all(|key| !key.down) {
            return true;
        }
        let mut down: Vec<u16> = self
            .attempt
            .iter()
            .filter(|key| key.down)
            .map(|key| key.code)
            .collect();
        down.sort_unstable();
        self.chords.iter().any(|(chord, _)| {
            let modifier_so_far = chord.modifiers.contains(&code)
                && down.iter().all(|held| chord.modifiers.contains(held));
            let last_key = chord.key == Some(code) && down == chord.modifiers;
            modifier_so_far || last_key
        })
    }

    fn is_hotkey_key(&self, code: u16) -> bool {
        self.chords
            .iter()
            .any(|(chord, _)| chord.keys().any(|key| key == code))
    }

    /// Something else came first: send the held-back presses after all.
    fn spoil(&mut self, forward: &mut impl FnMut(InputEvent)) {
        self.spoiled = true;
        for key in self.attempt.iter_mut().filter(|key| key.down && !key.sent) {
            key.sent = true;
            forward(InputEvent::Key {
                code: key.code,
                pressed: true,
            });
        }
    }

    /// Every key is up: fire the hotkey these keys make, or give the target
    /// the presses it never saw (a lone Shift tap, say).
    fn finish(&mut self, now: Instant, forward: &mut impl FnMut(InputEvent)) -> Option<Trigger> {
        let mut keys: Vec<u16> = self.attempt.iter().map(|key| key.code).collect();
        keys.sort_unstable();
        let lone_tap = !self.spoiled && keys.len() == 1 && Some(keys[0]) == self.double_tap;
        if lone_tap && self.last_tap.is_some_and(|last| now - last < self.window) {
            // The second tap: the target got the first one as a plain tap,
            // which is harmless for a modifier, and never sees this one.
            self.last_tap = None;
            self.attempt.clear();
            return Some(Trigger::Forward);
        }
        self.last_tap = lone_tap.then_some(now);
        let fired = (!self.spoiled)
            .then(|| {
                self.chords.iter().find_map(|(chord, trigger)| {
                    let mut wanted: Vec<u16> = chord.keys().collect();
                    wanted.sort_unstable();
                    (wanted == keys).then_some(*trigger)
                })
            })
            .flatten();
        if fired.is_none() {
            for key in self.attempt.iter().filter(|key| !key.sent) {
                forward(InputEvent::Key {
                    code: key.code,
                    pressed: true,
                });
                forward(InputEvent::Key {
                    code: key.code,
                    pressed: false,
                });
            }
        }
        self.attempt.clear();
        self.spoiled = false;
        fired
    }
}

const DEFAULT_NEXT: &str = mickey_config::DEFAULT_NEXT;

fn default_chord(text: &str) -> Chord {
    chord::parse(text).expect("the built-in hotkeys parse")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALT: u16 = KEY_RIGHTALT;
    const SHIFT: u16 = KEY_RIGHTSHIFT;
    const Q: u16 = 16;
    const ONE: u16 = 2;
    const ZERO: u16 = 11;

    fn key(code: u16, pressed: bool) -> InputEvent {
        InputEvent::Key { code, pressed }
    }

    fn all() -> Hotkey {
        let set = mickey_config::parse("[[peer]]\nname = \"A\"\naddress = \"192.0.2.10:1\"\n")
            .unwrap()
            .hotkeys()
            .unwrap();
        Hotkey::from_config(&set)
    }

    /// Feeds `events`; returns what was forwarded and what fired.
    fn run(hotkey: &mut Hotkey, events: &[InputEvent]) -> (Vec<InputEvent>, Vec<Trigger>) {
        let mut forwarded = Vec::new();
        let mut fired = Vec::new();
        for &event in events {
            if let Some(trigger) = hotkey.feed(event, |out| forwarded.push(out)) {
                fired.push(trigger);
            }
        }
        (forwarded, fired)
    }

    #[test]
    fn holding_altgr_and_right_shift_comes_home_whatever_is_set() {
        // Someone set the hotkeys away from these keys and turned home off.
        let set = mickey_config::parse(
            "[[peer]]\nname = \"A\"\naddress = \"192.0.2.10:1\"\n\n[hotkeys]\nnext = \"LeftCtrl+F12\"\nhome = \"LeftCtrl+F11\"\ndouble_tap = \"\"\n",
        )
        .unwrap()
        .hotkeys()
        .unwrap();
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let press = |hotkey: &mut Hotkey, events: &[(InputEvent, u64)]| {
            let mut fired = Vec::new();
            for &(event, ms) in events {
                fired.extend(hotkey.feed_at(event, at(ms), |_| {}));
            }
            fired
        };
        let hold = |ms| {
            [
                (key(ALT, true), 0),
                (key(SHIFT, true), 10),
                (key(SHIFT, false), ms),
                (key(ALT, false), ms + 10),
            ]
        };
        let mut hotkey = Hotkey::from_config(&set);
        assert_eq!(
            press(&mut hotkey, &hold(2100)),
            [Trigger::Home],
            "held 2 s: home"
        );
        assert_eq!(
            press(&mut hotkey, &hold(500)),
            [],
            "a short press is not the escape"
        );
        // With the default hotkeys a short press is Next, a long one home.
        let mut defaults = Hotkey::new();
        assert_eq!(press(&mut defaults, &hold(500)), [Trigger::Next]);
        assert_eq!(press(&mut defaults, &hold(2100)), [Trigger::Home]);
        // Another key while holding spoils it.
        let spoiled = [
            (key(ALT, true), 0),
            (key(SHIFT, true), 10),
            (key(Q, true), 100),
            (key(Q, false), 150),
            (key(SHIFT, false), 2200),
            (key(ALT, false), 2210),
        ];
        assert!(!press(&mut Hotkey::from_config(&set), &spoiled).contains(&Trigger::Home));
    }

    #[test]
    fn the_m0_hotkey_fires_on_release_in_either_order() {
        for last in [ALT, SHIFT] {
            let first = if last == ALT { SHIFT } else { ALT };
            let events = [
                key(ALT, true),
                key(SHIFT, true),
                key(ALT, true),
                key(first, false),
                key(last, false),
            ];
            assert_eq!(
                run(&mut Hotkey::new(), &events),
                (vec![], vec![Trigger::Next])
            );
        }
    }

    #[test]
    fn a_digit_after_the_chord_goes_straight_to_a_pc_or_home() {
        let events = [
            key(ALT, true),
            key(SHIFT, true),
            key(ONE, true),
            key(ONE, false),
            key(SHIFT, false),
            key(ALT, false),
        ];
        assert_eq!(run(&mut all(), &events), (vec![], vec![Trigger::Go(0)]));
        let events = [
            key(SHIFT, true),
            key(ALT, true),
            key(ZERO, true),
            key(ALT, false),
            key(ZERO, false),
            key(SHIFT, false),
        ];
        assert_eq!(run(&mut all(), &events), (vec![], vec![Trigger::Home]));
    }

    #[test]
    fn typing_with_altgr_or_shift_still_works() {
        let events = [key(ALT, true), key(Q, true), key(Q, false), key(ALT, false)];
        assert_eq!(run(&mut all(), &events), (events.to_vec(), vec![]));
        // Shift+1 is "!": Shift alone is not a full chord, so 1 spoils it.
        let events = [
            key(SHIFT, true),
            key(ONE, true),
            key(ONE, false),
            key(SHIFT, false),
        ];
        assert_eq!(run(&mut all(), &events), (events.to_vec(), vec![]));
        // A plain digit is never held back.
        let events = [key(ONE, true), key(ONE, false)];
        assert_eq!(run(&mut all(), &events), (events.to_vec(), vec![]));
    }

    #[test]
    fn a_third_key_spoils_and_flushes_in_order() {
        let events = [
            key(ALT, true),
            key(SHIFT, true),
            key(Q, true),
            key(Q, false),
            key(SHIFT, false),
            key(ALT, false),
        ];
        assert_eq!(run(&mut all(), &events), (events.to_vec(), vec![]));
    }

    #[test]
    fn a_lone_tap_is_forwarded_whole() {
        let (forwarded, fired) = run(
            &mut all(),
            &[key(SHIFT, true), key(SHIFT, true), key(SHIFT, false)],
        );
        assert_eq!(
            (forwarded, fired),
            (vec![key(SHIFT, true), key(SHIFT, false)], vec![])
        );
    }

    #[test]
    fn shift_click_sends_shift_before_the_button() {
        let click = InputEvent::Button {
            code: 0x110,
            pressed: true,
        };
        let motion = InputEvent::Motion { dx: 1, dy: 0 };
        let (forwarded, _) = run(&mut all(), &[key(SHIFT, true), motion, click]);
        assert_eq!(forwarded, [motion, key(SHIFT, true), click]);
    }

    const LEFT: u16 = 105;
    const RIGHT: u16 = 106;

    #[test]
    fn altgr_and_the_arrows_move_with_one_hand() {
        let events = [
            key(ALT, true),
            key(RIGHT, true),
            key(RIGHT, false),
            key(ALT, false),
        ];
        assert_eq!(run(&mut all(), &events), (vec![], vec![Trigger::Forward]));
        let events = [
            key(ALT, true),
            key(LEFT, true),
            key(LEFT, false),
            key(ALT, false),
        ];
        assert_eq!(run(&mut all(), &events), (vec![], vec![Trigger::Back]));
    }

    #[test]
    fn a_quick_double_tap_on_right_shift_goes_forward() {
        let mut hotkey = all();
        let start = Instant::now();
        let tap = |hotkey: &mut Hotkey, at: u64| {
            let mut out = Vec::new();
            let mut fired = None;
            for pressed in [true, false] {
                let now = start + Duration::from_millis(at);
                fired = fired.or(hotkey.feed_at(key(SHIFT, pressed), now, |e| out.push(e)));
            }
            (out, fired)
        };
        // First tap: forwarded whole; second, 200 ms later: fires, not sent.
        assert_eq!(
            tap(&mut hotkey, 0),
            (vec![key(SHIFT, true), key(SHIFT, false)], None)
        );
        assert_eq!(tap(&mut hotkey, 200), (vec![], Some(Trigger::Forward)));
        // Too slow: two plain taps.
        assert_eq!(tap(&mut hotkey, 1000).1, None);
        assert_eq!(tap(&mut hotkey, 1600).1, None);
    }

    #[test]
    fn the_double_tap_window_comes_from_the_config() {
        let set =
            mickey_config::parse("[hotkeys]\ndouble_tap = \"LeftCtrl\"\ndouble_tap_ms = 700\n")
                .unwrap()
                .hotkeys()
                .unwrap();
        let mut hotkey = Hotkey::from_config(&set);
        let start = Instant::now();
        let mut fired = None;
        // Two taps on Left Ctrl, 600 ms apart: inside a 700 ms window.
        for (at, pressed) in [(0, true), (50, false), (600, true), (650, false)] {
            let now = start + Duration::from_millis(at);
            fired = fired.or(hotkey.feed_at(key(29, pressed), now, |_| {}));
        }
        assert_eq!(fired, Some(Trigger::Forward));
    }

    #[test]
    fn shift_then_a_letter_is_not_a_tap() {
        let mut hotkey = all();
        let events = [
            key(SHIFT, true),
            key(Q, true),
            key(Q, false),
            key(SHIFT, false),
        ];
        for _ in 0..2 {
            assert_eq!(run(&mut hotkey, &events).1, []);
        }
    }

    #[test]
    fn defaults_never_override_a_setting() {
        let set = mickey_config::parse(
            "[hotkeys]\nhome = \"AltGr+RightShift+1\"\n\n[[peer]]\nname = \"A\"\naddress = \"192.0.2.10:1\"\n",
        )
        .unwrap()
        .hotkeys()
        .unwrap();
        let events = [
            key(ALT, true),
            key(SHIFT, true),
            key(ONE, true),
            key(ONE, false),
            key(SHIFT, false),
            key(ALT, false),
        ];
        assert_eq!(
            run(&mut Hotkey::from_config(&set), &events).1,
            [Trigger::Home]
        );
    }
}
