//! The data link a target serves: the clipboard, and the small messages
//! peers send beside it (the group map, hotkeys, the switcher, sound).

use super::slots::{SharedSlots, Slot};
use super::{Shown, Sources, show_switcher_here};
use crate::threads::LockAnyway;
use crate::{
    audio, clipboard, net::group, net::identity, net::transport, net::trust, source, window,
};
use mickey_config as config;
use std::net::SocketAddr;

/// Answers paired PCs' data links (PRD 0011) on the port after `listen`,
/// each on its own thread, for as long as Mickey runs.
pub(super) fn serve_clipboard(
    listen: SocketAddr,
    identity: &std::sync::Arc<identity::Identity>,
    trusted: &std::sync::Arc<std::sync::Mutex<trust::TrustedPeers>>,
    keeper: clipboard::Keeper,
    controlled: &std::sync::mpsc::Sender<source::TargetNews>,
    sources: &Sources,
    grants: &clipboard::sync::Grants,
) {
    let data = transport::data_address(listen);
    let listener = match std::net::TcpListener::bind(data) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("mickey: no shared clipboard: cannot listen on {data}: {err}");
            return;
        }
    };
    let (identity, trusted, controlled) = (identity.clone(), trusted.clone(), controlled.clone());
    let (sources, grants) = (sources.clone(), grants.clone());
    let shown = Shown::default();
    crate::threads::spawn("data accept", move || {
        let slots = SharedSlots::default();
        for stream in listener.incoming().flatten() {
            let Ok(from) = stream.peer_addr() else {
                continue;
            };
            let Some(slot) = Slot::take(&slots, from.ip()) else {
                continue;
            };
            let (identity, trusted, keeper) = (identity.clone(), trusted.clone(), keeper.clone());
            let (controlled, sources, shown) = (controlled.clone(), sources.clone(), shown.clone());
            let grants = grants.clone();
            let from = from.ip();
            crate::threads::try_spawn("data link", move || {
                let _slot = slot;
                let is_trusted = |key: &[u8; 32]| trusted.lock_anyway().is_trusted(key);
                match transport::accept_data(stream, &identity, is_trusted) {
                    Ok(link) => {
                        let by = *link.peer_key();
                        let payload = |kind: u8, text: &str| {
                            on_payload(kind, text, from, by, &controlled, &sources, &shown)
                        };
                        let _ = clipboard::sync::answer(
                            link,
                            || clipboard::sync::may_read(&grants, from),
                            || keeper.read(),
                            || clipboard::sync::may_write(&grants, from),
                            |content| keeper.write(content),
                            payload,
                        );
                    }
                    Err(err) => eprintln!("mickey: data link refused: {err}"),
                }
            });
        }
    });
}

/// One small message a peer sent on its data link, by `kind`: the group
/// map, hotkeys and who listens go to the source; the switcher opens or
/// closes here; a peer's sound key and version are noted.
fn on_payload(
    kind: u8,
    text: &str,
    from: std::net::IpAddr,
    by: [u8; 32],
    controlled: &std::sync::mpsc::Sender<source::TargetNews>,
    sources: &Sources,
    shown: &Shown,
) {
    match kind {
        group::KIND => match group::decode(text, from) {
            Ok(members) => {
                let _ = controlled.send(source::TargetNews::Group { members, by });
            }
            Err(err) => eprintln!("mickey: {err}"),
        },
        group::SWITCHER | group::MIRROR => {
            let (text, sources, shown) = (text.to_string(), sources.clone(), shown.clone());
            let mirror = kind == group::MIRROR;
            crate::threads::spawn("switcher here", move || {
                show_switcher_here(&text, from, &sources, &shown, mirror)
            });
        }
        group::CLOSE => {
            let shown = shown.clone();
            crate::threads::spawn("switcher close", move || window::close(&shown));
        }
        group::HOTKEYS => {
            let _ = controlled.send(source::TargetNews::Hotkeys(text.to_string()));
        }
        group::AUDIO_KEY => {
            // Named after the PC, as config.toml knows it.
            let name = config::Config::load()
                .ok()
                .flatten()
                .and_then(|config| {
                    config
                        .peers
                        .into_iter()
                        .find(|peer| peer.address.ip() == from)
                })
                .map_or_else(|| from.to_string(), |peer| peer.name);
            audio::heard_key(text, from, &name);
        }
        group::AUDIO => {
            let _ = controlled.send(source::TargetNews::Audio(text.to_string()));
        }
        group::VERSION => {
            // A peer's word: shown only if it is a version.
            if let Some(version) = config::version::checked(text) {
                let _ = controlled.send(source::TargetNews::Version { from, version });
            }
        }
        _ => {}
    }
}
