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
