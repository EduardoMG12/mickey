//! The data link a target serves: the clipboard, and the small messages
//! peers send beside it (the group map, hotkeys, the switcher, sound).

use super::slots::{SharedSlots, Slot};
use super::{Shown, Sources, show_switcher_here};
use crate::threads::LockAnyway;
use crate::{
    audio, clipboard, net::group, net::identity, net::secure, net::transport, net::trust, source,
    window,
};
use mickey_config as config;
use std::net::SocketAddr;

/// Each source's newest data link, by key, with the number it was opened
/// as: a source has one at a time, so a new one closes the one before.
type Newest =
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<[u8; 32], (u64, secure::Link)>>>;

/// This PC's clipboard, and what it needs to fetch files offered to it
/// (PRD 0015).
#[derive(Clone)]
pub(super) struct Clip {
    pub keeper: clipboard::Keeper,
    pub transfers: Option<std::sync::Arc<crate::transfer::trigger::Transfers>>,
}

/// Answers paired PCs' data links (PRD 0011) on the port after `listen`,
/// each on its own thread, for as long as Mickey runs.
pub(super) fn serve_clipboard(
    listen: SocketAddr,
    identity: &std::sync::Arc<identity::Identity>,
    trusted: &std::sync::Arc<std::sync::Mutex<trust::TrustedPeers>>,
    clip: Clip,
    target_news: &std::sync::mpsc::Sender<source::TargetNews>,
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
    let (identity, trusted, target_news) = (identity.clone(), trusted.clone(), target_news.clone());
    let (sources, grants) = (sources.clone(), grants.clone());
    let shown = Shown::default();
    crate::threads::spawn("data accept", move || {
        let slots = SharedSlots::default();
        let refused_said = std::sync::Mutex::new(None);
        // Each source's newest data link, by key: a source has one at a
        // time, so a new one closes the one before (#452). A data link
        // carries no pings, so an old one from a PC that vanished would
        // otherwise hold its slot for ever.
        let newest = Newest::default();
        let mut opened: u64 = 0;
        for stream in listener.incoming().flatten() {
            let Ok(from) = stream.peer_addr() else {
                continue;
            };
            let Some(slot) = Slot::take(&slots, from.ip()) else {
                super::slots::say_refused(&refused_said, from.ip(), super::slots::MAX_PER_ADDRESS);
                continue;
            };
            let newest = newest.clone();
            opened += 1;
            let this = opened;
            let (identity, trusted, clip) = (identity.clone(), trusted.clone(), clip.clone());
            let (target_news, sources, shown) =
                (target_news.clone(), sources.clone(), shown.clone());
            let grants = grants.clone();
            let from = from.ip();
            crate::threads::try_spawn("data link", move || {
                let _slot = slot;
                let is_trusted = |key: &[u8; 32]| trusted.lock_anyway().is_trusted(key);
                match transport::accept_data(stream, &identity, is_trusted) {
                    Ok(link) => {
                        let by = *link.peer_key();
                        if let Ok(handle) = link.try_clone() {
                            let mut map = newest.lock_anyway();
                            // Two links of one PC may finish their handshakes
                            // out of order: the older one gives way, never
                            // the newer.
                            if map.get(&by).is_some_and(|(id, _)| *id > this) {
                                return;
                            }
                            if let Some((_, older)) = map.insert(by, (this, handle)) {
                                let _ = older.shutdown();
                            }
                        }
                        let payload = |kind: u8, text: &str| {
                            on_payload(kind, text, from, by, &target_news, &sources, &shown)
                        };
                        let keeper = &clip.keeper;
                        let _ = clipboard::sync::answer(
                            link,
                            // Only right after the keyboard left this PC (#185).
                            || {
                                clipboard::sync::may_read(&grants, from)
                                    .then(|| keeper.read_to_share())
                                    .flatten()
                            },
                            || {
                                if clipboard::sync::may_write(&grants, from) {
                                    Ok(())
                                } else {
                                    Err(config::peer_name_or_address(from))
                                }
                            },
                            |content| keeper.write(content),
                            payload,
                            || trusted.lock_anyway().is_trusted(&by),
                            clip.transfers.as_ref(),
                        );
                        // Ended: let go of its handle, unless a newer link
                        // from the same PC took its place already.
                        let mut newest = newest.lock_anyway();
                        if newest.get(&by).is_some_and(|(id, _)| *id == this) {
                            newest.remove(&by);
                        }
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
    target_news: &std::sync::mpsc::Sender<source::TargetNews>,
    sources: &Sources,
    shown: &Shown,
) {
    match kind {
        group::KIND => match group::decode(text, from) {
            Ok(members) => {
                let _ = target_news.send(source::TargetNews::Group { members, by });
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
            let lines = text.to_string();
            let _ = target_news.send(source::TargetNews::Hotkeys { lines, from });
        }
        group::AUDIO_KEY => audio::heard_key(text, from, &config::peer_name_or_address(from)),
        group::AUDIO => {
            let _ = target_news.send(source::TargetNews::Audio(text.to_string()));
        }
        group::VERSION => {
            // A peer's word: shown only if it is a version.
            if let Some(version) = config::version::checked(text) {
                let _ = target_news.send(source::TargetNews::Version { from, version });
            }
        }
        _ => {}
    }
}
