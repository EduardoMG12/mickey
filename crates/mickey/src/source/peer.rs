//! One paired PC as the source sees it: its address, its link, what it
//! said last, and the threads that connect to it, pair with it and read it.

use super::RETRY_EVERY;
use crate::clip_sync::Mover;
use crate::identity::Identity;
use crate::latency::{self, Latency};
use crate::monitors::Monitor;
use crate::protocol::Message;
use crate::secure::Link;
use crate::threads::LockAnyway;
use crate::transport;
use crate::trust;
use mickey_config as config;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Instant;

/// One configured peer and its link, if it is connected.
pub(super) struct Peer {
    pub(super) name: String,
    pub(super) address: SocketAddr,
    pub(super) link: Option<Link>,
    /// When it last answered (a pong, its monitors…).
    pub(super) heard: Instant,
    /// Its public key, once it has connected; how the group knows it.
    pub(super) key: Option<[u8; 32]>,
    pub(super) latency: Latency,
    /// What the peer reported (PRD 0005); empty until it does.
    pub(super) monitors: Vec<Monitor>,
    /// It answered with another key than the one this PC knows: refused,
    /// and the settings say why (#289).
    pub(super) new_key: bool,
    /// The Mickey version it runs, once its data link said (PRD 0013).
    pub(super) version: Option<String>,
}

impl Peer {
    /// A peer not connected yet, last heard of at `heard`.
    pub(super) fn new(
        name: String,
        address: SocketAddr,
        key: Option<[u8; 32]>,
        heard: Instant,
    ) -> Peer {
        Peer {
            name,
            address,
            link: None,
            heard,
            key,
            latency: Latency::new(),
            monitors: Vec::new(),
            new_key: false,
            version: None,
        }
    }
}

/// Everything the background reconnect threads need, shared read-only.
pub(super) struct Shared {
    pub(super) identity: Identity,
    /// The trusted PCs. Reconnects never ask, so a peer that is not in
    /// here is not retried.
    pub(super) trusted: trust::Shared,
}

/// A peer's first connection, on its own thread: pairs if needed, then
/// hands the link to the main loop; if it fails, retries like any peer.
pub(super) fn connect_first(
    index: usize,
    name: String,
    to: SocketAddr,
    shared: &Arc<Shared>,
    back: &Sender<(usize, Link)>,
) {
    let (shared, back) = (Arc::clone(shared), back.clone());
    crate::threads::spawn("connect", move || {
        match connect_and_pair(&shared.identity, &shared.trusted, &name, to) {
            Ok(link) => {
                let _ = back.send((index, link));
            }
            Err(err) => {
                println!("mickey: {name} ({to}) is offline: {err}");
                reconnect(index, to, &shared, &back);
            }
        }
    });
}

/// Connects, asks to pair if the key is unknown, and waits for the peer to
/// accept us.
pub(super) fn connect_and_pair(
    identity: &Identity,
    trusted: &trust::Shared,
    name: &str,
    to: SocketAddr,
) -> Result<Link, String> {
    let mut link = transport::connect(to, identity).map_err(|err| err.to_string())?;
    let peer_key = *link.peer_key();
    let code = link.pairing_code();
    if !trust::trust_or_ask(trusted, &peer_key, to, &code, name)? {
        return Err("not paired; nothing was sent".into());
    }
    println!("mickey: waiting for {name} to accept this PC");
    transport::hello(&mut link).map_err(|err| match err.kind() {
        std::io::ErrorKind::UnexpectedEof => "it refused to pair with this PC".to_string(),
        _ => err.to_string(),
    })?;
    Ok(link)
}

/// Opens the data link to a connected peer in the background and hands it
/// to the mover. A peer without one (an older Mickey) only misses the
/// shared clipboard.
/// The data link must answer with `key`, the key of the input link just
/// opened to the same peer: otherwise whoever answers the data port (a PC
/// that took over the address) would get the clipboard and the group map.
pub(super) fn open_data_link(
    index: usize,
    input: SocketAddr,
    key: [u8; 32],
    shared: &Arc<Shared>,
    mover: &Mover,
) {
    let (shared, mover) = (Arc::clone(shared), mover.clone());
    crate::threads::spawn("open data link", move || {
        match transport::connect_data(input, &shared.identity) {
            Ok(link) if *link.peer_key() == key => mover.link(index, link),
            Ok(_) => eprintln!(
                "mickey: the data port of {input} answered with another PC's key; no shared clipboard with it"
            ),
            Err(err) => eprintln!("mickey: no shared clipboard with {input}: {err}"),
        }
    });
}

/// Retries a trusted peer in the background until it answers, then hands
/// its link to the main loop. Never asks to pair: a peer whose key is not
/// trusted is left alone.
pub(super) fn reconnect(
    index: usize,
    to: SocketAddr,
    shared: &Arc<Shared>,
    back: &Sender<(usize, Link)>,
) {
    let (shared, back) = (Arc::clone(shared), back.clone());
    crate::threads::spawn("reconnect", move || {
        loop {
            thread::sleep(RETRY_EVERY);
            let Ok(mut link) = transport::connect(to, &shared.identity) else {
                continue;
            };
            let trusted = shared.trusted.lock_anyway().is_trusted(link.peer_key());
            if !trusted {
                println!(
                    "mickey: {to} has a key this PC never paired with; `mickey connect {to} <name>` pairs again"
                );
                return;
            }
            if transport::hello(&mut link).is_ok() {
                let _ = back.send((index, link));
                return;
            }
        }
    });
}

/// What a peer sends back on its link.
pub(super) enum News {
    /// A pong: the round-trip time in microseconds.
    RoundTrip(u32),
    Monitors(Vec<Monitor>),
    /// The cursor is pushing off the peer's desktop onto this point of it.
    Leave(i32, i32),
    /// The peer's system refused the input sent to it.
    Refused,
    /// The owner picked a card in the switcher shown on the peer.
    Picked(u8),
}

/// Reads what a peer sends, on its own thread, and passes it on as
/// `News`: pongs as round-trip times, its monitors, and the rest.
pub(super) fn read_peer(index: usize, link: &Link, news: &Sender<(usize, News)>, start: Instant) {
    let (Ok(mut reader), news) = (link.try_clone(), news.clone()) else {
        return;
    };
    crate::threads::spawn("peer reader", move || {
        while let Ok(message) = reader.recv() {
            let item = match message {
                Message::Pong { id } => News::RoundTrip(latency::round_trip(start, id)),
                Message::Monitors { count, list } => {
                    News::Monitors(list[..usize::from(count)].to_vec())
                }
                Message::Leave { x, y } => News::Leave(x, y),
                Message::Refused => News::Refused,
                Message::Picked { card } => News::Picked(card),
                _ => continue,
            };
            if news.send((index, item)).is_err() {
                return;
            }
        }
    });
}

pub(super) fn report_latency(peer: &Peer) {
    if let Some((p50, p99)) = peer.latency.percentiles() {
        println!(
            "mickey: one-way latency to {} over the last {} pings: p50 {p50} µs, p99 {p99} µs",
            peer.name,
            peer.latency.len()
        );
    }
}

/// Writes `key` into the peer's block in config.toml, the first time it
/// connects (#184).
pub(super) fn save_key(name: &str, key: &[u8; 32]) {
    let saved = config::path().and_then(|path| {
        let text = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
        let text = config::set_key(&text, name, &crate::hex::encode(key))?;
        config::write(&path, &text)
    });
    if let Err(err) = saved {
        eprintln!("mickey: cannot remember {name}'s key: {err}");
    }
}
