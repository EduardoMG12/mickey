//! Recording a hotkey by pressing it (PRD 0007): the settings window asks
//! the running Mickey, which sees the physical keys (left from right,
//! AltGr), to name the next keys pressed together, or a key tapped twice.
//! While it records, those keys do not switch PCs.
//!
//! Any program of the user can ask for a recording, so it never names
//! plain typing (Shift, or nothing, with a key that types text): asked
//! again and again, it would otherwise spell out capitals and symbols
//! as they are typed. Such a chord would make a poor hotkey anyway.

use mickey_config::chord;
use std::time::{Duration, Instant};

/// What one key event made of the recording so far.
#[derive(Debug, PartialEq, Eq)]
pub enum Heard {
    Waiting,
    /// Keys pressed together, e.g. `AltGr+Right`.
    Chord(String),
    /// One key tapped twice, e.g. `RightShift`.
    DoubleTap(String),
    /// Something that cannot be a hotkey, and why.
    Mistake(String),
}

/// The answer to plain typing: it names no key.
const TYPING: &str =
    "Shift with a key that types text is typing, not a hotkey; add Ctrl, Alt, AltGr or Super";

/// Keys that type text: digits, letters, punctuation and space (Linux
/// key codes, as the hotkeys use).
fn types_text(code: u16) -> bool {
    matches!(code, 2..=13 | 16..=27 | 30..=41 | 43..=53 | 57)
}

/// Shift (either) or no modifier at all with a key that types text: what
/// typing a capital or a symbol looks like, never a hotkey.
fn is_typing(keys: &[u16]) -> bool {
    const SHIFTS: [u16; 2] = [42, 54];
    let others: Vec<u16> = keys
        .iter()
        .copied()
        .filter(|code| !SHIFTS.contains(code))
        .collect();
    matches!(others[..], [key] if types_text(key))
}

pub struct Recording {
    /// Every key pressed since the last time all were up, in order.
    pressed: Vec<u16>,
    /// The keys down now.
    down: Vec<u16>,
    /// A key pressed alone, and when it went up: half a double tap.
    tapped: Option<(u16, Instant)>,
    /// How close two taps must be: the same as the hotkeys use.
    window: Duration,
}

impl Recording {
    pub fn new(window: Duration) -> Recording {
        Recording {
            pressed: Vec::new(),
            down: Vec::new(),
            tapped: None,
            window,
        }
    }

    pub fn key(&mut self, code: u16, pressed: bool, now: Instant) -> Heard {
        if pressed {
            if !self.down.contains(&code) {
                self.down.push(code);
            }
            if !self.pressed.contains(&code) {
                self.pressed.push(code);
            }
            return Heard::Waiting;
        }
        self.down.retain(|&down| down != code);
        if !self.down.is_empty() || self.pressed.is_empty() {
            return Heard::Waiting;
        }
        let keys = std::mem::take(&mut self.pressed);
        if let [alone] = keys[..] {
            // A key alone: the first half of a double tap, or the second.
            let second = self
                .tapped
                .is_some_and(|(key, up)| key == alone && now.duration_since(up) <= self.window);
            if second {
                if !chord::fits_a_double_tap(alone) {
                    return Heard::Mistake(
                        "a double tap on a key typed alone would get in the way; use Shift, Ctrl, Alt, AltGr, Super or an F key".into(),
                    );
                }
                return match chord::key_name(alone) {
                    Some(name) => Heard::DoubleTap(name),
                    None => Heard::Mistake("Mickey has no name for that key yet".into()),
                };
            }
            self.tapped = Some((alone, now));
            return Heard::Waiting;
        }
        if is_typing(&keys) {
            return Heard::Mistake(TYPING.into());
        }
        match chord::name_keys(&keys) {
            Ok(text) => Heard::Chord(text),
            Err(err) => Heard::Mistake(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALTGR: u16 = 100;
    const RIGHT_SHIFT: u16 = 54;
    const RIGHT: u16 = 106;
    const WINDOW: Duration = Duration::from_millis(400);

    #[test]
    fn keys_pressed_together_are_named_when_all_are_up() {
        let (mut recording, now) = (Recording::new(WINDOW), Instant::now());
        assert_eq!(recording.key(ALTGR, true, now), Heard::Waiting);
        assert_eq!(recording.key(RIGHT, true, now), Heard::Waiting);
        assert_eq!(recording.key(RIGHT, false, now), Heard::Waiting);
        assert_eq!(
            recording.key(ALTGR, false, now),
            Heard::Chord("AltGr+Right".into())
        );
    }

    #[test]
    fn a_key_tapped_twice_quickly_is_a_double_tap() {
        let (mut recording, now) = (Recording::new(WINDOW), Instant::now());
        recording.key(RIGHT_SHIFT, true, now);
        assert_eq!(recording.key(RIGHT_SHIFT, false, now), Heard::Waiting);
        recording.key(RIGHT_SHIFT, true, now);
        let later = now + Duration::from_millis(200);
        assert_eq!(
            recording.key(RIGHT_SHIFT, false, later),
            Heard::DoubleTap("RightShift".into())
        );
    }

    #[test]
    fn a_letter_tapped_twice_is_refused() {
        const W: u16 = 17;
        let (mut recording, now) = (Recording::new(WINDOW), Instant::now());
        recording.key(W, true, now);
        assert_eq!(recording.key(W, false, now), Heard::Waiting);
        recording.key(W, true, now);
        assert!(matches!(recording.key(W, false, now), Heard::Mistake(_)));
    }

    #[test]
    fn too_slow_is_not_a_double_tap_and_a_plain_key_is_a_mistake() {
        let (mut recording, now) = (Recording::new(WINDOW), Instant::now());
        recording.key(RIGHT_SHIFT, true, now);
        recording.key(RIGHT_SHIFT, false, now);
        recording.key(RIGHT_SHIFT, true, now);
        let late = now + WINDOW + Duration::from_millis(50);
        assert_eq!(recording.key(RIGHT_SHIFT, false, late), Heard::Waiting);
        recording.key(30, true, late);
        recording.key(31, true, late);
        recording.key(30, false, late);
        assert!(matches!(recording.key(31, false, late), Heard::Mistake(_)));
    }

    #[test]
    fn typing_is_never_named() {
        const LEFT_SHIFT: u16 = 42;
        const A: u16 = 30;
        const SLASH: u16 = 53;
        const F5: u16 = 63;
        const LEFT_CTRL: u16 = 29;
        let now = Instant::now();
        // A capital and a symbol: refused without naming the key.
        for key in [A, SLASH] {
            let mut recording = Recording::new(WINDOW);
            recording.key(LEFT_SHIFT, true, now);
            recording.key(key, true, now);
            recording.key(key, false, now);
            assert_eq!(
                recording.key(LEFT_SHIFT, false, now),
                Heard::Mistake(TYPING.into())
            );
        }
        // Shift with a key that types nothing, and Ctrl with a letter: hotkeys.
        for (modifier, key, name) in [
            (LEFT_SHIFT, F5, "LeftShift+F5"),
            (LEFT_CTRL, A, "LeftCtrl+A"),
        ] {
            let mut recording = Recording::new(WINDOW);
            recording.key(modifier, true, now);
            recording.key(key, true, now);
            recording.key(key, false, now);
            assert_eq!(
                recording.key(modifier, false, now),
                Heard::Chord(name.into())
            );
        }
    }
}
