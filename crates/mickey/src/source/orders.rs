//! The answers to the control socket (`mickey home`, `mickey peers`,
//! `mickey discover`…).

use super::Peer;
use crate::control::Order;
use crate::discovery;
use crate::focus::{Action, Focus, Switcher};
use crate::identity;
use crate::monitors::Monitor;
use crate::trust;
use mickey_config::peers::{PeerInfo, Peers};
use std::time::Instant;

/// Carries out an order from the control socket and returns the answer
/// line. Orders only move the focus.
pub(super) fn obey(
    order: Order,
    names: &[String],
    online: &[bool],
    switcher: &mut Switcher,
    act: &mut impl FnMut(Action),
) -> String {
    match order {
        Order::Home => {
            switcher.go_home(act);
            "ok: the focus is on this PC".into()
        }
        Order::Switcher if switcher.focus() == Focus::Local => {
            act(Action::OpenSwitcher);
            "ok: showing the switcher".into()
        }
        Order::Switcher => match switcher.focus() {
            Focus::Remote(peer) => {
                act(Action::OpenSwitcherOn(peer));
                "ok: showing the switcher on the PC in use".into()
            }
            _ => "error: the focus is moving; try again".into(),
        },
        // The main loop answers these, since it knows the answers.
        Order::Peers
        | Order::Discover
        | Order::History
        | Order::HistoryCopy(_)
        | Order::Quit
        | Order::Arranged
        | Order::Record
        | Order::Forget(_)
        | Order::Connect { .. } => "error: not an order".into(),
        Order::Pairing => {
            trust::open_pairing();
            format!(
                "ok: a new PC may ask to pair for {} s",
                trust::PAIRING_OPEN.as_secs()
            )
        }
        Order::Go(name) => {
            let Some(peer) = names.iter().position(|known| *known == name) else {
                return format!("error: no PC named `{name}` in config.toml");
            };
            if !online[peer] {
                return format!("error: {name} is offline");
            }
            if switcher.focus() != Focus::Local {
                return "error: the focus is on another PC; use the hotkey or `mickey home`".into();
            }
            switcher.go_to(Some(peer), online);
            format!("ok: going to {name} once every key is released")
        }
    }
}

/// The answer to `mickey discover`: one PC per line, its name, address
/// and fingerprint, tab-separated.
pub(super) fn found_text(found: Option<&discovery::Table>) -> String {
    let Some(table) = found else {
        return "error: this Mickey is not looking for other PCs".into();
    };
    let lines: Vec<String> = table
        .list(Instant::now())
        .iter()
        .map(|pc| {
            format!(
                "{}\t{}\t{}",
                pc.name,
                pc.address,
                identity::fingerprint(&pc.key)
            )
        })
        .collect();
    lines.join("\n")
}

/// The answer to `mickey peers`.
pub(super) fn peers_text(this_pc: &[Monitor], peers: &[Peer], online: &[bool]) -> String {
    Peers {
        this_pc: this_pc.to_vec(),
        peers: peers
            .iter()
            .zip(online)
            .map(|(peer, &online)| PeerInfo {
                name: peer.name.clone(),
                online,
                reinstalled: peer.new_key,
                version: peer.version.clone(),
                monitors: peer.monitors.clone(),
            })
            .collect(),
    }
    .to_text()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(order: Order, switcher: &mut Switcher, online: &[bool]) -> (String, Vec<Action>) {
        let names = ["Desktop".to_string(), "Windows".to_string()];
        let mut actions = Vec::new();
        let answer = obey(order, &names, online, switcher, &mut |action| {
            actions.push(action)
        });
        (answer, actions)
    }

    #[test]
    fn go_moves_the_focus_to_an_online_peer_only() {
        let mut switcher = Switcher::new();
        let (answer, _) = run(Order::Go("Desktop".into()), &mut switcher, &[false, true]);
        assert_eq!(answer, "error: Desktop is offline");
        let (answer, _) = run(Order::Go("Nope".into()), &mut switcher, &[false, true]);
        assert!(answer.starts_with("error: no PC named"));
        let (answer, _) = run(Order::Go("Windows".into()), &mut switcher, &[false, true]);
        assert!(answer.starts_with("ok"));
        assert_eq!(switcher.focus(), Focus::WaitingForKeysUp(1));
    }

    #[test]
    fn home_and_switcher_from_a_remote_peer() {
        let mut switcher = Switcher::new();
        switcher.go_to(Some(1), &[true, true]);
        switcher.grabbed();
        let (answer, actions) = run(Order::Switcher, &mut switcher, &[true, true]);
        assert!(answer.starts_with("ok"));
        assert_eq!(actions, [Action::OpenSwitcherOn(1)], "on the screen in use");
        let (answer, actions) = run(Order::Home, &mut switcher, &[true, true]);
        assert!(answer.starts_with("ok"));
        assert_eq!(actions, [Action::Ungrab]);
        assert_eq!(switcher.focus(), Focus::Local);
    }
}
