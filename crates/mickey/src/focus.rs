//! Which peer gets the input: the focus state machine, for one or more
//! peers (numbered 0, 1, … in config order).
//!
//! ```text
//!           one peer: hotkey                 all keys up, grabbed
//! Local ---------------------> WaitingForKeysUp(p) -----------> Remote(p)
//!   |  ^                              ^                            |
//!   |  |    2+ peers: OpenSwitcher,   |                            |
//!   |  +--- go_to(p) from the window--+                            |
//!   |                                                              |
//!   +<---- hotkey: releases, Ungrab (and OpenSwitcher if 2+) ------+
//! ```
//!
//! With one peer the hotkey is the M0 toggle. With two or more it asks for
//! the switcher window (`Action::OpenSwitcher`), and `go_to` follows the
//! owner's pick; from a remote peer it first brings the devices home.

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
    /// Show the switcher window; its answer comes back through `go_to`.
    OpenSwitcher,
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
                if self.hotkey.feed(event, |_| {}) {
                    if online.len() >= 2 {
                        act(Action::OpenSwitcher);
                    } else if let Some(peer) = first_online(online) {
                        self.focus = Focus::WaitingForKeysUp(peer);
                    }
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
                    self.focus = Focus::Local;
                    if online.len() >= 2 {
                        act(Action::OpenSwitcher);
                    }
                }
            }
        }
    }

    /// The owner picked `peer` in the switcher (`None`: this PC). Ignored
    /// unless the focus is local and the peer is online.
    pub fn go_to(&mut self, peer: Option<usize>, online: &[bool]) {
        if self.focus == Focus::Local
            && let Some(peer) = peer
            && online.get(peer) == Some(&true)
        {
            self.focus = Focus::WaitingForKeysUp(peer);
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

/// The first online peer, if any.
fn first_online(online: &[bool]) -> Option<usize> {
    online.iter().position(|&up| up)
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
    fn with_two_peers_the_hotkey_asks_for_the_switcher() {
        let online = [true, false, true];
        let (mut switcher, mut actions) = (Switcher::new(), Vec::new());
        hotkey(&mut switcher, &online, &mut actions);
        assert_eq!(switcher.focus(), Focus::Local);
        assert_eq!(actions, [Action::OpenSwitcher]);

        switcher.go_to(Some(1), &online);
        assert_eq!(switcher.focus(), Focus::Local, "peer 1 is offline");
        switcher.go_to(Some(2), &online);
        switcher.grabbed();
        assert_eq!(switcher.focus(), Focus::Remote(2));
    }

    #[test]
    fn from_a_peer_the_hotkey_comes_home_then_asks() {
        let online = [true, true];
        let (mut switcher, mut actions) = (Switcher::new(), Vec::new());
        switcher.go_to(Some(1), &online);
        switcher.grabbed();
        switcher.handle(key(A, true), &online, |action| actions.push(action));
        actions.clear();

        hotkey(&mut switcher, &online, &mut actions);
        assert_eq!(switcher.focus(), Focus::Local);
        assert_eq!(
            actions,
            [
                Action::Send(1, key(A, false)),
                Action::Ungrab,
                Action::OpenSwitcher,
            ]
        );
    }

    #[test]
    fn with_its_only_peer_offline_the_hotkey_stays_home() {
        let (mut switcher, mut actions) = (Switcher::new(), Vec::new());
        hotkey(&mut switcher, &[false], &mut actions);
        assert_eq!(switcher.focus(), Focus::Local);
        assert!(actions.is_empty());
    }

    #[test]
    fn losing_the_focused_peer_brings_the_devices_home() {
        let (mut switcher, mut actions) = (Switcher::new(), Vec::new());
        switcher.go_to(Some(0), &[true, true]);
        switcher.grabbed();
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
