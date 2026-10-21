//! Which peer gets the input: the focus state machine, for one or more
//! peers (numbered 0, 1, … in config order).
//!
//! ```text
//! Local --hotkey--> WaitingForKeysUp(p) --all keys up, grabbed--> Remote(p)
//!   ^                       ^                                        |
//!   |                       +----hotkey, next online peer q----------+
//!   +------------------------hotkey, no peer after p------------------+
//! ```
//!
//! So the hotkey cycles This PC → peer 0 → peer 1 → … → This PC, skipping
//! peers that are offline. With one peer that is the M0 toggle.

use crate::event::InputEvent;
use crate::held::HeldSet;
use crate::hotkey::Hotkey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// Input stays on this machine; devices are not grabbed.
    Local,
    /// The hotkey picked a peer, but grabbing now would leave keys stuck
    /// down on the desktop, so we wait until every key is up.
    WaitingForKeysUp(usize),
    /// Devices are grabbed and input goes to this peer.
    Remote(usize),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Send this event to this peer.
    Send(usize, InputEvent),
    Ungrab,
}

pub struct Switcher {
    focus: Focus,
    hotkey: Hotkey,
    /// What the focused peer holds down because we sent it.
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
        if let Focus::WaitingForKeysUp(peer) = self.focus {
            self.focus = Focus::Remote(peer);
        }
    }

    /// `online[p]` says whether peer `p` is connected and can take focus.
    pub fn handle(&mut self, event: InputEvent, online: &[bool], mut act: impl FnMut(Action)) {
        match self.focus {
            Focus::Local => {
                // The desktop sees every event itself; we only watch.
                if self.hotkey.feed(event, |_| {})
                    && let Some(peer) = next_online(None, online)
                {
                    self.focus = Focus::WaitingForKeysUp(peer);
                }
            }
            Focus::WaitingForKeysUp(_) => {
                self.hotkey.feed(event, |_| {});
            }
            Focus::Remote(peer) => {
                let (keys, buttons) = (&mut self.keys, &mut self.buttons);
                let fired = self.hotkey.feed(event, |out| {
                    match out {
                        InputEvent::Key { code, pressed } => keys.set(code, pressed),
                        InputEvent::Button { code, pressed } => buttons.set(code, pressed),
                        InputEvent::Motion { .. } | InputEvent::Scroll { .. } => {}
                    }
                    act(Action::Send(peer, out));
                });
                if fired {
                    // Release on that peer before giving the devices back.
                    self.keys.release_all(|code| {
                        act(Action::Send(
                            peer,
                            InputEvent::Key {
                                code,
                                pressed: false,
                            },
                        ))
                    });
                    self.buttons.release_all(|code| {
                        act(Action::Send(
                            peer,
                            InputEvent::Button {
                                code,
                                pressed: false,
                            },
                        ))
                    });
                    act(Action::Ungrab);
                    self.focus = match next_online(Some(peer), online) {
                        Some(next) => Focus::WaitingForKeysUp(next),
                        None => Focus::Local,
                    };
                }
            }
        }
    }

    /// Peer `peer`'s connection is gone. If it had the focus, the devices
    /// come home at once; that peer releases its own keys on disconnect.
    pub fn peer_lost(&mut self, peer: usize, mut act: impl FnMut(Action)) {
        match self.focus {
            Focus::Remote(focused) if focused == peer => {
                self.keys.release_all(|_| {});
                self.buttons.release_all(|_| {});
                act(Action::Ungrab);
                self.focus = Focus::Local;
            }
            Focus::WaitingForKeysUp(waiting) if waiting == peer => self.focus = Focus::Local,
            _ => {}
        }
    }
}

/// The first online peer after `after` (or from the start), if any.
fn next_online(after: Option<usize>, online: &[bool]) -> Option<usize> {
    let start = after.map_or(0, |peer| peer + 1);
    (start..online.len()).find(|&peer| online[peer])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::{KEY_RIGHTALT, KEY_RIGHTSHIFT};

    const A: u16 = 30;

    fn key(code: u16, pressed: bool) -> InputEvent {
        InputEvent::Key { code, pressed }
    }

    /// Presses and releases the hotkey, then grabs if a peer was picked.
    fn hotkey(switcher: &mut Switcher, online: &[bool], actions: &mut Vec<Action>) {
        for event in [
            key(KEY_RIGHTALT, true),
            key(KEY_RIGHTSHIFT, true),
            key(KEY_RIGHTSHIFT, false),
            key(KEY_RIGHTALT, false),
        ] {
            switcher.handle(event, online, |action| actions.push(action));
        }
        switcher.grabbed();
    }

    #[test]
    fn one_peer_toggles_like_m0() {
        let (mut switcher, mut actions) = (Switcher::new(), Vec::new());
        switcher.handle(key(A, true), &[true], |action| actions.push(action));
        assert!(actions.is_empty(), "local input is never sent");

        hotkey(&mut switcher, &[true], &mut actions);
        assert_eq!(switcher.focus(), Focus::Remote(0));
        switcher.handle(key(A, true), &[true], |action| actions.push(action));
        hotkey(&mut switcher, &[true], &mut actions);
        assert_eq!(switcher.focus(), Focus::Local);
        assert_eq!(
            actions,
            [
                Action::Send(0, key(A, true)),
                Action::Send(0, key(A, false)), // still held when focus left
                Action::Ungrab,
            ]
        );
    }

    #[test]
    fn the_hotkey_cycles_through_online_peers_and_home() {
        let online = [true, false, true];
        let (mut switcher, mut actions) = (Switcher::new(), Vec::new());
        hotkey(&mut switcher, &online, &mut actions);
        assert_eq!(switcher.focus(), Focus::Remote(0));
        hotkey(&mut switcher, &online, &mut actions);
        assert_eq!(switcher.focus(), Focus::Remote(2), "peer 1 is offline");
        hotkey(&mut switcher, &online, &mut actions);
        assert_eq!(switcher.focus(), Focus::Local);
    }

    #[test]
    fn with_every_peer_offline_the_hotkey_stays_home() {
        let (mut switcher, mut actions) = (Switcher::new(), Vec::new());
        hotkey(&mut switcher, &[false, false], &mut actions);
        assert_eq!(switcher.focus(), Focus::Local);
        assert!(actions.is_empty());
    }

    #[test]
    fn losing_the_focused_peer_brings_the_devices_home() {
        let (mut switcher, mut actions) = (Switcher::new(), Vec::new());
        hotkey(&mut switcher, &[true, true], &mut actions);
        switcher.handle(key(A, true), &[true, true], |action| actions.push(action));
        actions.clear();

        switcher.peer_lost(1, |action| actions.push(action));
        assert_eq!(
            switcher.focus(),
            Focus::Remote(0),
            "another peer is ignored"
        );
        switcher.peer_lost(0, |action| actions.push(action));
        assert_eq!(switcher.focus(), Focus::Local);
        assert_eq!(actions, [Action::Ungrab], "no releases to a gone peer");
    }

    #[test]
    fn grabbed_is_ignored_unless_waiting() {
        let mut switcher = Switcher::new();
        switcher.grabbed();
        assert_eq!(switcher.focus(), Focus::Local);
    }
}
