//! The hotkey: AltGr + Right Shift, firing on release when nothing else was
//! pressed in between.
//!
//! While focus is remote, a press of either key is held back until we know
//! what it is part of. If another key, button or wheel comes first, the held
//! keys are sent after all, so AltGr+Q or Shift+A still type on the target.
//! If the hotkey completes, the target never sees any of it.

use crate::event::InputEvent;

pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_RIGHTALT: u16 = 100;
const HOTKEY: [u16; 2] = [KEY_RIGHTALT, KEY_RIGHTSHIFT];

#[derive(Clone, Copy, Default)]
struct KeyState {
    /// Physically down right now.
    down: bool,
    /// Down, but its press has not been forwarded yet.
    held_back: bool,
    /// Its press was forwarded, so its release must be too.
    sent: bool,
}

pub struct Hotkey {
    keys: [KeyState; 2],
    /// Both keys were down at the same time.
    chord: bool,
    /// Something else was pressed while a hotkey key was down.
    spoiled: bool,
}

impl Hotkey {
    pub fn new() -> Hotkey {
        Hotkey {
            keys: [KeyState::default(); 2],
            chord: false,
            spoiled: false,
        }
    }

    /// Feeds one captured event. `forward` receives, in order, every event
    /// the target should get. Returns `true` when the hotkey fires.
    pub fn feed(&mut self, event: InputEvent, mut forward: impl FnMut(InputEvent)) -> bool {
        let (code, pressed) = match event {
            InputEvent::Key { code, pressed } if HOTKEY.contains(&code) => (code, pressed),
            InputEvent::Key { pressed: true, .. }
            | InputEvent::Button { pressed: true, .. }
            | InputEvent::Scroll { .. } => {
                if self.keys.iter().any(|key| key.down) {
                    self.spoil(&mut forward);
                }
                forward(event);
                return false;
            }
            _ => {
                forward(event);
                return false;
            }
        };
        let i = if code == HOTKEY[0] { 0 } else { 1 };

        if pressed {
            if self.keys[i].down {
                // Auto-repeat: only meaningful if the target saw the press.
                if self.keys[i].sent {
                    forward(event);
                }
            } else if self.spoiled {
                self.keys[i] = KeyState {
                    down: true,
                    held_back: false,
                    sent: true,
                };
                forward(event);
            } else {
                self.keys[i] = KeyState {
                    down: true,
                    held_back: true,
                    sent: false,
                };
                self.chord |= self.keys.iter().all(|key| key.down);
            }
            return false;
        }

        let key = self.keys[i];
        self.keys[i] = KeyState::default();
        if key.sent {
            forward(event);
        } else if key.held_back && !self.chord {
            // A lone tap: the target gets the whole press and release.
            forward(InputEvent::Key {
                code,
                pressed: true,
            });
            forward(event);
        }
        if self.keys.iter().any(|key| key.down) {
            return false;
        }
        let fired = self.chord && !self.spoiled;
        self.chord = false;
        self.spoiled = false;
        fired
    }

    fn spoil(&mut self, forward: &mut impl FnMut(InputEvent)) {
        self.spoiled = true;
        for (key, code) in self.keys.iter_mut().zip(HOTKEY) {
            if key.held_back {
                key.held_back = false;
                key.sent = true;
                forward(InputEvent::Key {
                    code,
                    pressed: true,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALT: u16 = KEY_RIGHTALT;
    const SHIFT: u16 = KEY_RIGHTSHIFT;
    const Q: u16 = 16;

    fn key(code: u16, pressed: bool) -> InputEvent {
        InputEvent::Key { code, pressed }
    }

    /// Feeds `events`; returns what was forwarded and how often it fired.
    fn run(events: &[InputEvent]) -> (Vec<InputEvent>, usize) {
        let mut hotkey = Hotkey::new();
        let mut forwarded = Vec::new();
        let mut fired = 0;
        for &event in events {
            if hotkey.feed(event, |out| forwarded.push(out)) {
                fired += 1;
            }
        }
        (forwarded, fired)
    }

    #[test]
    fn fires_on_release_in_either_order_and_sends_nothing() {
        for last in [ALT, SHIFT] {
            let first = if last == ALT { SHIFT } else { ALT };
            let (forwarded, fired) = run(&[
                key(ALT, true),
                key(SHIFT, true),
                key(ALT, true), // auto-repeat
                key(first, false),
                key(last, false),
            ]);
            assert_eq!((forwarded, fired), (vec![], 1));
        }
    }

    #[test]
    fn altgr_combinations_still_type() {
        let events = [key(ALT, true), key(Q, true), key(Q, false), key(ALT, false)];
        assert_eq!(run(&events), (events.to_vec(), 0));
    }

    #[test]
    fn a_third_key_spoils_the_hotkey_and_flushes_it() {
        let (forwarded, fired) = run(&[
            key(ALT, true),
            key(SHIFT, true),
            key(Q, true),
            key(Q, false),
            key(SHIFT, false),
            key(ALT, false),
        ]);
        assert_eq!(fired, 0);
        assert_eq!(
            forwarded,
            [
                key(ALT, true),
                key(SHIFT, true),
                key(Q, true),
                key(Q, false),
                key(SHIFT, false),
                key(ALT, false),
            ]
        );
    }

    #[test]
    fn a_lone_tap_is_forwarded_whole() {
        let (forwarded, fired) = run(&[key(SHIFT, true), key(SHIFT, true), key(SHIFT, false)]);
        assert_eq!(
            (forwarded, fired),
            (vec![key(SHIFT, true), key(SHIFT, false)], 0)
        );
    }

    #[test]
    fn shift_click_sends_shift_before_the_button() {
        let click = InputEvent::Button {
            code: 0x110,
            pressed: true,
        };
        let motion = InputEvent::Motion { dx: 1, dy: 0 };
        let (forwarded, _) = run(&[key(SHIFT, true), motion, click]);
        assert_eq!(forwarded, [motion, key(SHIFT, true), click]);
    }
}
