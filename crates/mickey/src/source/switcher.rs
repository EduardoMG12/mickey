//! The switcher window, shown when the hotkey is pressed (PRD 0007).

use super::{Peer, Wake};
use crate::window;
use mickey_config as config;
use std::sync::mpsc::Sender;

/// The switcher's cards: the focused card's id first, then one card per
/// PC, as `id=online:name` or `id=offline:name`, in the order to show them
/// (`Interface::switcher_sort`). Id 0 is this PC, `i + 1` is peer `i`.
pub(super) fn switcher_cards(
    focused: usize,
    my_name: &str,
    peers: &[Peer],
    positions: &[Option<[i32; 2]>],
    interface: &config::Interface,
) -> Vec<String> {
    let mut all = vec![(my_name, Some([0, 0]), true)];
    for (peer, place) in peers.iter().zip(positions) {
        all.push((peer.name.as_str(), *place, peer.link.is_some()));
    }
    let sortable: Vec<(&str, Option<[i32; 2]>)> =
        all.iter().map(|&(name, place, _)| (name, place)).collect();
    let mut cards = vec![focused.to_string()];
    for id in interface.switcher_sort(&sortable) {
        let (name, _, online) = all[id];
        let state = if online { "online" } else { "offline" };
        cards.push(format!("{id}={state}:{name}"));
    }
    cards
}

/// Opens a view-only copy of the switcher on this PC; the caller kills it
/// once a pick is made.
pub(super) fn open_mirror(
    cards: &[String],
    interface: &config::Interface,
) -> Option<std::process::Child> {
    crate::window::ui_command(&crate::window::switcher_args(cards, interface, true))
        .and_then(|mut ui| ui.stdout(std::process::Stdio::null()).spawn())
        .ok()
}

/// Shows `mickey-ui switcher` (installed next to this program) on its own
/// thread, and sends the owner's pick when the window closes: `Some(peer)`,
/// `None` for this PC or Esc, or an error if the window could not run.
/// The GUI is a separate program so this one stays small (PRD 0003).
pub(super) fn open_switcher(
    cards: &[String],
    interface: &config::Interface,
    wake: &Sender<Wake>,
    slot: &window::Slot,
) {
    let args = crate::window::switcher_args(cards, interface, false);
    let (wake, slot) = (wake.clone(), slot.clone());
    crate::threads::spawn("switcher", move || {
        let pick = match window::run(&args, &slot) {
            // The window prints the card's id; 0 is this PC.
            window::Outcome::Answered(text) => text
                .parse::<usize>()
                .map(|card| card.checked_sub(1))
                .map_err(|err| format!("mickey-ui answered badly: {err}")),
            window::Outcome::Closed => Ok(None),
            window::Outcome::Failed(err) => Err(err),
        };
        let _ = wake.send(Wake::Pick(pick));
    });
}
