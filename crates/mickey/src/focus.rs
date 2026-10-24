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
use crate::hotkey::{Hotkey, Trigger};

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
    /// Show it on this peer, where the owner is looking: the focus stays
    /// there, and the peer sends the pick back.
    OpenSwitcherOn(usize),
}

pub struct Switcher {
    focus: Focus,
    hotkey: Hotkey,
    /// What the focused peer holds down because we sent it.
    keys: HeldSet,
    buttons: HeldSet,
}

impl Switcher {
    /// With only the M0 hotkey.
    #[cfg(test)]
    pub fn new() -> Switcher {
        Switcher::with_hotkeys(Hotkey::new())
    }

    /// New hotkeys, from the settings or another PC, used at once.
    pub fn set_hotkeys(&mut self, hotkey: Hotkey) {
        self.hotkey = hotkey;
    }

    pub fn with_hotkeys(hotkey: Hotkey) -> Switcher {
        Switcher {
            focus: Focus::Local,
            hotkey,
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
                match self.hotkey.feed(event, |_| {}) {
                    Some(Trigger::Next) if online.len() >= 2 => act(Action::OpenSwitcher),
                    Some(Trigger::Next) => {
                        if let Some(peer) = first_online(online) {
                            self.focus = Focus::WaitingForKeysUp(peer);
                        }
                    }
                    Some(Trigger::Switcher) => act(Action::OpenSwitcher),
                    Some(Trigger::Go(peer)) => self.go_to(Some(peer), online),
                    Some(trigger @ (Trigger::Forward | Trigger::Back)) => {
                        self.go_to(step(None, trigger, online), online);
                    }
                    Some(Trigger::Home) | None => {}
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
                match fired {
                    // Already there.
                    Some(Trigger::Go(target)) if target == peer => {}
                    Some(Trigger::Go(target)) => {
                        if online.get(target) == Some(&true) {
                            self.go_home(&mut act);
                            self.focus = Focus::WaitingForKeysUp(target);
                        }
                    }
                    Some(Trigger::Home) => self.go_home(&mut act),
                    Some(trigger @ (Trigger::Forward | Trigger::Back)) => {
                        self.go_home(&mut act);
                        if let Some(target) = step(Some(peer), trigger, online) {
                            self.focus = Focus::WaitingForKeysUp(target);
                        }
                    }
                    Some(Trigger::Switcher) => act(Action::OpenSwitcherOn(peer)),
                    Some(Trigger::Next) if online.len() >= 2 => act(Action::OpenSwitcherOn(peer)),
                    // With one peer, Next simply comes back.
                    Some(Trigger::Next) => self.go_home(&mut act),
                    None => {}
                }
            }
        }
    }

    /// Brings the devices home from wherever the focus is, releasing on the
    /// focused peer everything it holds first.
    pub fn go_home(&mut self, mut act: impl FnMut(Action)) {
        match self.focus {
            Focus::Remote(peer) => {
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
            }
            Focus::WaitingForKeysUp(_) | Focus::Local => {}
        }
        self.focus = Focus::Local;
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

/// One step through the list "this PC, peer 0, peer 1, …" (wrapping),
/// skipping offline peers. `from` is the focused peer (`None`: this PC);
/// the result is the peer to go to, or `None` for this PC.
fn step(from: Option<usize>, trigger: Trigger, online: &[bool]) -> Option<usize> {
    // Positions: 0 is this PC, p + 1 is peer p.
    let len = online.len() + 1;
    let mut position = from.map_or(0, |peer| peer + 1);
    loop {
        position = match trigger {
            Trigger::Back => (position + len - 1) % len,
            _ => (position + 1) % len,
        };
        if position == 0 {
            return None;
        }
        if online[position - 1] {
            return Some(position - 1);
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
        // The hotkey's modifiers reach the peer as pressed and released
        // (#188); A is let go there since it was still held.
        assert_eq!(
            actions,
            [
                Action::Send(0, key(A, true)),
                Action::Send(0, key(KEY_RIGHTALT, true)),
                Action::Send(0, key(KEY_RIGHTSHIFT, true)),
                Action::Send(0, key(KEY_RIGHTSHIFT, false)),
                Action::Send(0, key(KEY_RIGHTALT, false)),
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
    fn from_a_peer_the_hotkey_asks_there() {
        let online = [true, true];
        let (mut switcher, mut actions) = (Switcher::new(), Vec::new());
        switcher.go_to(Some(1), &online);
        switcher.grabbed();
        switcher.handle(key(A, true), &online, |action| actions.push(action));
        actions.clear();

        // The owner is looking at peer 1: the switcher shows there, and the
        // focus stays until a card is picked.
        hotkey(&mut switcher, &online, &mut actions);
        assert_eq!(switcher.focus(), Focus::Remote(1));
        // The modifiers reached peer 1 as they were pressed (#188).
        assert_eq!(
            actions,
            [
                Action::Send(1, key(KEY_RIGHTALT, true)),
                Action::Send(1, key(KEY_RIGHTSHIFT, true)),
                Action::Send(1, key(KEY_RIGHTSHIFT, false)),
                Action::Send(1, key(KEY_RIGHTALT, false)),
                Action::OpenSwitcherOn(1),
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
    fn a_direct_hotkey_hops_from_one_peer_to_another() {
        use crate::hotkey::Hotkey;
        let set = mickey_config::parse(
            "[[peer]]\nname = \"A\"\naddress = \"192.0.2.10:1\"\n\n[[peer]]\nname = \"B\"\naddress = \"192.0.2.11:1\"\n",
        )
        .unwrap()
        .hotkeys()
        .unwrap();
        let mut switcher = Switcher::with_hotkeys(Hotkey::from_config(&set));
        let mut actions = Vec::new();
        let two = 3; // KEY_2
        for event in [
            key(KEY_RIGHTALT, true),
            key(KEY_RIGHTSHIFT, true),
            key(two, true),
            key(two, false),
            key(KEY_RIGHTSHIFT, false),
            key(KEY_RIGHTALT, false),
        ] {
            switcher.handle(event, &[true, true], |action| actions.push(action));
        }
        switcher.grabbed();
        assert_eq!(
            switcher.focus(),
            Focus::Remote(1),
            "straight to the second PC"
        );
        assert!(actions.is_empty(), "no switcher window");
    }

    #[test]
    fn forward_and_back_walk_the_list_and_skip_offline_peers() {
        let online = [true, false, true];
        assert_eq!(step(None, Trigger::Forward, &online), Some(0));
        assert_eq!(
            step(Some(0), Trigger::Forward, &online),
            Some(2),
            "peer 1 is offline"
        );
        assert_eq!(step(Some(2), Trigger::Forward, &online), None, "back home");
        assert_eq!(step(None, Trigger::Back, &online), Some(2));
        assert_eq!(step(Some(0), Trigger::Back, &online), None);
        assert_eq!(
            step(None, Trigger::Forward, &[false]),
            None,
            "nobody online"
        );
    }

    #[test]
    fn grabbed_is_ignored_unless_waiting() {
        let mut switcher = Switcher::new();
        switcher.grabbed();
        assert_eq!(switcher.focus(), Focus::Local);
    }
}
