//! One paired PC as the source sees it: its address, its link, what it
//! said last, and the threads that connect to it, pair with it and read it.

use super::RETRY_EVERY;
use crate::clipboard::sync::DataLinks;
use crate::monitors::Monitor;
use crate::net::identity::Identity;
use crate::net::latency::{self, Latency};
use crate::net::protocol::{Message, NotPairedWhy};
use crate::net::secure::Link;
use crate::net::transport;
use crate::net::trust;
use crate::threads::LockAnyway;
use mickey_config as config;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
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
    /// The addresses a first connection is pairing with right now: a second
    /// Connect while one waits (up to a minute and a half for the other PC's
    /// answer) must not open a second pair window.
    pub(super) pairing: Pairing,
}

/// The addresses being paired with, shared with the threads doing it.
pub(super) type Pairing = Arc<std::sync::Mutex<std::collections::HashSet<SocketAddr>>>;

/// Marks `address` as being paired with until dropped, whether the pairing
/// worked or not.
struct PairingWith(Pairing, SocketAddr);

impl PairingWith {
    fn start(pairing: &Pairing, address: SocketAddr) -> PairingWith {
        pairing.lock_anyway().insert(address);
        PairingWith(Arc::clone(pairing), address)
    }
}

impl Drop for PairingWith {
    fn drop(&mut self) {
        self.0.lock_anyway().remove(&self.1);
    }
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
    // Marked before the thread starts, so the main loop sees it at once.
    let pairing = PairingWith::start(&shared.pairing, to);
    crate::threads::spawn("connect", move || {
        let result = connect_and_pair(&shared.identity, &shared.trusted, &name, to);
        drop(pairing);
        match result {
            Ok(link) => {
                let _ = back.send((index, link));
            }
            Err(err) => {
                println!("mickey: {name} ({to}) not connected: {err}");
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
    if trusted.lock_anyway().is_trusted(&peer_key) {
        transport::hello(&mut link).map_err(|err| refusal(&err, false, name))?;
        return Ok(link);
    }
    // While this PC shows the code, the other PC may already say no (its
    // pairing closed, say): its first message is read meanwhile, and closes
    // the window, so a code never shows on one screen only (#420).
    let first = first_message(&link)?;
    let early = std::cell::RefCell::new(None);
    let stopped = || {
        if early.borrow().is_none()
            && let Ok(got) = first.try_recv()
        {
            *early.borrow_mut() = Some(got);
        }
        early.borrow().is_some()
    };
    let code = link.pairing_code();
    let answer = trust::trust_or_ask(trusted, &peer_key, to, &code, name, &stopped);
    // Closing it also ends the thread waiting for the first message.
    let stop = |link: &Link| {
        let _ = link.shutdown();
    };
    // Only the person here pairs: when the other PC closed the window, what
    // it said can only be a no, even if it reads as a yes (a Hello sent
    // before this PC asked would otherwise pair without anyone's consent).
    let consented = answer == Ok(trust::Answer::Paired);
    match answer {
        Ok(trust::Answer::Paired | trust::Answer::Stopped) => {}
        Ok(trust::Answer::NotPaired(_)) => {
            stop(&link);
            return Err("not paired; nothing was sent".into());
        }
        Err(err) => {
            stop(&link);
            return Err(err);
        }
    }
    let got = match early.into_inner() {
        Some(got) => got,
        None => {
            println!("mickey: waiting for {name} to accept this PC");
            transport::send_hello(&mut link).map_err(|err| err.to_string())?;
            first
                .recv_timeout(transport::PAIR_WAIT)
                .unwrap_or_else(|_| {
                    stop(&link);
                    Err(std::io::ErrorKind::TimedOut.into())
                })
        }
    };
    let answered = answered(got, name, consented);
    if let Err((why, word)) = &answered {
        stop(&link);
        tell_not_paired(name, why, word);
    }
    answered.map_err(|(why, _)| why)?;
    // Only now: had the other PC refused, this one would have trusted a PC
    // that never paired with it (#420).
    trust::remember(trusted, &peer_key, to, name)?;
    Ok(link)
}

/// What the first message from `name`, which this PC asked to pair with,
/// means: paired, or why not, as the log says it and as a word for the
/// window (`mickey-ui not-paired`), which puts it in the owner's language.
/// Never paired unless the person here `consented`: an answer that came
/// before (it closed the window) can only be a no.
fn answered(
    got: std::io::Result<Message>,
    name: &str,
    consented: bool,
) -> Result<(), (String, &'static str)> {
    match got {
        Ok(Message::NotPaired { why }) => Err((not_paired(why, name), why_word(why))),
        Ok(_) if !consented => Err((
            format!("{name} answered before this PC paired"),
            "no-answer",
        )),
        Ok(message) => transport::hello_answer(message).map_err(|err| (err.to_string(), "version")),
        Err(err) => Err((refusal(&err, true, name), "no-answer")),
    }
}

/// The first message `link`'s peer sends, read on a thread of its own.
fn first_message(link: &Link) -> Result<mpsc::Receiver<std::io::Result<Message>>, String> {
    let mut reader = link.try_clone().map_err(|err| err.to_string())?;
    let (sent, first) = mpsc::channel();
    crate::threads::spawn("pairing answer", move || {
        let _ = sent.send(reader.recv());
    });
    Ok(first)
}

/// What to tell the person when `name` said it did not pair, and why.
fn not_paired(why: NotPairedWhy, name: &str) -> String {
    match why {
        NotPairedWhy::Closed => {
            format!("{name}'s pairing is closed: open it there (Settings → PCs), then try again")
        }
        NotPairedWhy::Busy => {
            format!(
                "{name} is answering another PC, or was asked a moment ago: try again in a minute"
            )
        }
        NotPairedWhy::Declined => format!("{name} said no to pairing with this PC"),
    }
}

/// The word `mickey-ui not-paired` turns into the owner's language.
fn why_word(why: NotPairedWhy) -> &'static str {
    match why {
        NotPairedWhy::Closed => "closed",
        NotPairedWhy::Busy => "busy",
        NotPairedWhy::Declined => "declined",
    }
}

/// Shows why pairing with `name` did not happen: `why` in the log, `word`
/// for the window.
fn tell_not_paired(name: &str, why: &str, word: &str) {
    println!("mickey: not paired with {name}: {why}");
    let _ = crate::window::open(&["not-paired", name, word]);
}

/// What to tell the person when `name` did not answer the handshake with
/// Hello. `new`: this PC had just asked to pair with it.
fn refusal(err: &std::io::Error, new: bool, name: &str) -> String {
    use std::io::ErrorKind::{TimedOut, UnexpectedEof, WouldBlock};
    match (err.kind(), new) {
        // It closed the connection: its pairing was closed, another PC was
        // asking there, or its owner said no.
        // A Mickey speaking another protocol version closes it the same way.
        (UnexpectedEof, true) => format!(
            "{name} did not pair: its pairing is closed, it said no, or it runs another Mickey version. Open pairing on {name} (Settings → PCs) and try again"
        ),
        (UnexpectedEof, false) => format!(
            "{name} closed the connection: it may have forgotten this PC (pair them again from its settings) or run another Mickey version"
        ),
        (TimedOut | WouldBlock, true) => {
            format!("{name} did not answer the pairing in time; try again")
        }
        _ => err.to_string(),
    }
}

/// Opens the data link to a connected peer in the background and hands it
/// to `DataLinks`. A peer without one (an older Mickey) only misses the
/// shared clipboard.
/// The data link must answer with `key`, the key of the input link just
/// opened to the same peer: otherwise whoever answers the data port (a PC
/// that took over the address) would get the clipboard and the group map.
pub(super) fn open_data_link(
    index: usize,
    input: SocketAddr,
    key: [u8; 32],
    shared: &Arc<Shared>,
    data_links: &DataLinks,
) {
    let (shared, data_links) = (Arc::clone(shared), data_links.clone());
    crate::threads::spawn("open data link", move || {
        match transport::connect_data(input, &shared.identity) {
            Ok(mut link) if *link.peer_key() == key => {
                match crate::clipboard::sync::ask_transfers(&mut link) {
                    Ok(knows) => data_links.link(index, link, knows),
                    Err(err) => eprintln!("mickey: no shared clipboard with {input}: {err}"),
                }
            }
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

/// Writes `key` into the peer's block in config.toml.
pub(super) fn save_key(name: &str, key: &[u8; 32]) -> bool {
    let saved = config::path().and_then(|path| {
        let text = config::read_text(&path)?;
        let text = config::set_key(&text, name, &crate::hex::encode(key))?;
        config::write(&path, &text)
    });
    if let Err(err) = saved {
        eprintln!("mickey: cannot remember {name}'s key: {err}");
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error, ErrorKind};

    #[test]
    fn a_refused_pairing_says_what_to_do_on_the_other_pc() {
        let closed = refusal(&Error::from(ErrorKind::UnexpectedEof), true, "Desk");
        assert!(closed.contains("Open pairing on Desk"), "{closed}");
        let late = refusal(&Error::from(ErrorKind::TimedOut), true, "Desk");
        assert!(late.contains("in time"), "{late}");
    }

    #[test]
    fn a_paired_pc_that_refuses_may_have_forgotten_this_one() {
        let text = refusal(&Error::from(ErrorKind::UnexpectedEof), false, "Desk");
        assert!(text.contains("forgotten"), "{text}");
    }

    #[test]
    fn a_pairing_in_progress_is_marked_until_it_ends() {
        let pairing = Pairing::default();
        let to: SocketAddr = "192.0.2.7:4242".parse().unwrap();
        let marked = PairingWith::start(&pairing, to);
        assert!(
            pairing.lock_anyway().contains(&to),
            "a second Connect sees it"
        );
        drop(marked);
        assert!(
            !pairing.lock_anyway().contains(&to),
            "and may pair again once it ended"
        );
    }

    #[test]
    fn other_errors_keep_their_own_words() {
        let text = refusal(&Error::other("target speaks protocol 9"), true, "Desk");
        assert_eq!(text, "target speaks protocol 9");
    }

    #[test]
    fn the_first_answer_says_whether_and_why_not() {
        let hello = Message::Hello {
            version: crate::net::protocol::VERSION,
        };
        assert_eq!(answered(Ok(hello), "Desk", true), Ok(()));
        let closed = Message::NotPaired {
            why: NotPairedWhy::Closed,
        };
        let (why, word) = answered(Ok(closed), "Desk", true).unwrap_err();
        assert_eq!(word, "closed");
        assert!(why.contains("Desk's pairing is closed"), "{why}");
        let declined = Message::NotPaired {
            why: NotPairedWhy::Declined,
        };
        assert_eq!(
            answered(Ok(declined), "Desk", true).unwrap_err().1,
            "declined"
        );
        let gone = Error::from(std::io::ErrorKind::UnexpectedEof);
        assert_eq!(
            answered(Err(gone), "Desk", true).unwrap_err().1,
            "no-answer"
        );
        let old = Message::Hello { version: 2 };
        assert_eq!(answered(Ok(old), "Desk", true).unwrap_err().1, "version");
    }

    #[test]
    fn a_hello_before_the_person_here_paired_is_not_a_yes() {
        let hello = Message::Hello {
            version: crate::net::protocol::VERSION,
        };
        assert!(answered(Ok(hello), "Desk", false).is_err());
    }
}
