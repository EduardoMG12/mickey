//! Which peer gets the input: the focus state machine.
//!
//! ```text
//! Local --hotkey--> WaitingForKeysUp --all keys up, grabbed--> Remote
//!   ^                                                             |
//!   +---------------------hotkey (held keys released)-------------+
//! ```

use crate::event::InputEvent;
use crate::held::HeldSet;
use crate::hotkey::Hotkey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// Input stays on this machine; devices are not grabbed.
    Local,
    /// The hotkey fired, but grabbing now would leave keys stuck down on
    /// the desktop, so we wait until every key is up.
    WaitingForKeysUp,
    /// Devices are grabbed and input goes to the target.
    Remote,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Send(InputEvent),
    Ungrab,
}

pub struct Switcher {
    focus: Focus,
    hotkey: Hotkey,
    /// What the target holds down because we sent it.
    keys: HeldSet,
    buttons: HeldSet,
}

impl Switcher {
    pub fn new() -> Switcher {
        Switcher {
            focus: Focus::Local,
            hotkey: Hotkey::new(),
            keys: HeldSet::new(),
            buttons: HeldSet::new(),
        }
    }

    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// The caller grabbed every device after no key was down any more.
    pub fn grabbed(&mut self) {
        if self.focus == Focus::WaitingForKeysUp {
            self.focus = Focus::Remote;
        }
    }

    pub fn handle(&mut self, event: InputEvent, mut act: impl FnMut(Action)) {
        match self.focus {
            Focus::Local => {
                // The desktop sees every event itself; we only watch.
                if self.hotkey.feed(event, |_| {}) {
                    self.focus = Focus::WaitingForKeysUp;
                }
            }
            Focus::WaitingForKeysUp => {
                self.hotkey.feed(event, |_| {});
            }
            Focus::Remote => {
                let (keys, buttons) = (&mut self.keys, &mut self.buttons);
                let fired = self.hotkey.feed(event, |out| {
                    match out {
                        InputEvent::Key { code, pressed } => keys.set(code, pressed),
                        InputEvent::Button { code, pressed } => buttons.set(code, pressed),
                        InputEvent::Motion { .. } | InputEvent::Scroll { .. } => {}
                    }
                    act(Action::Send(out));
                });
                if fired {
                    // Release on the target before giving the devices back.
                    self.keys.release_all(|code| {
                        act(Action::Send(InputEvent::Key {
                            code,
                            pressed: false,
                        }))
                    });
                    self.buttons.release_all(|code| {
                        act(Action::Send(InputEvent::Button {
                            code,
                            pressed: false,
                        }))
                    });
                    act(Action::Ungrab);
                    self.focus = Focus::Local;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::{KEY_RIGHTALT, KEY_RIGHTSHIFT};

    const A: u16 = 30;

    fn key(code: u16, pressed: bool) -> InputEvent {
        InputEvent::Key { code, pressed }
    }

    fn press_hotkey(switcher: &mut Switcher, actions: &mut Vec<Action>) {
        for event in [
            key(KEY_RIGHTALT, true),
            key(KEY_RIGHTSHIFT, true),
            key(KEY_RIGHTSHIFT, false),
            key(KEY_RIGHTALT, false),
        ] {
            switcher.handle(event, |action| actions.push(action));
        }
    }

    #[test]
    fn a_full_round_trip() {
        let mut switcher = Switcher::new();
        let mut actions = Vec::new();

        switcher.handle(key(A, true), |action| actions.push(action));
        assert!(actions.is_empty(), "local input is never sent");

        press_hotkey(&mut switcher, &mut actions);
        assert_eq!(switcher.focus(), Focus::WaitingForKeysUp);
        switcher.grabbed();
        assert_eq!(switcher.focus(), Focus::Remote);

        switcher.handle(key(A, true), |action| actions.push(action));
        press_hotkey(&mut switcher, &mut actions);
        assert_eq!(switcher.focus(), Focus::Local);
        assert_eq!(
            actions,
            [
                Action::Send(key(A, true)),
                Action::Send(key(A, false)), // still held when focus left
                Action::Ungrab,
            ]
        );
    }

    #[test]
    fn grabbed_is_ignored_unless_waiting() {
        let mut switcher = Switcher::new();
        switcher.grabbed();
        assert_eq!(switcher.focus(), Focus::Local);
    }
}
