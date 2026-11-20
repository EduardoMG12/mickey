//! Moving the clipboard between PCs (PRD 0011), over the data link.

use crate::clipboard::wire::{self, Wire};
use crate::clipboard::{Content, Keeper};
use crate::net::secure::Link;
use crate::threads::LockAnyway;
use crate::transfer::file_offer::FileOffer;
use crate::transfer::trigger::Transfers;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Instant;

/// The target's side of a data link, until it closes: `Ask` is answered
/// with what `read` finds copied here (`None` unless the source may read
/// it now, #185), and content sent here is handed to `write`, which puts it
/// in this PC's clipboard, if `may_write` allows it; when it does not, its
/// error names the sending PC for the log (#436). Messages of kind 16 and
/// up (the group map, the shared hotkeys) are not the clipboard's: they go
/// to `other`, with their kind; `TRANSFERS` is answered with the same
/// (`ask_transfers`). With `transfers`, copied files go as an offer to a
/// source that asks for one (`AskOffer`), and an offer sent here is fetched
/// (PRD 0015).
pub fn answer(
    mut link: Link,
    read: impl Fn() -> Option<Content>,
    may_write: impl Fn() -> Result<(), String>,
    write: impl Fn(Content),
    other: impl Fn(u8, &str),
    still_trusted: impl Fn() -> bool,
    transfers: Option<&Arc<Transfers>>,
) -> io::Result<()> {
    loop {
        let bytes = link.recv_bulk()?;
        // Forgotten while connected: nothing more from it.
        if !still_trusted() {
            return Err(io::Error::other("this PC forgot the peer"));
        }
        if let Some((&kind, text)) = bytes.split_first()
            && kind >= crate::net::group::KIND
        {
            if kind == crate::net::group::TRANSFERS {
                link.send_bulk(&crate::net::group::tagged(kind, ""))?;
            } else {
                other(kind, &String::from_utf8_lossy(text));
            }
            continue;
        }
        let message = wire::decode(&bytes).map_err(io::Error::other)?;
        match message {
            Wire::Ask | Wire::AskOffer => {
                let wants_offer = message == Wire::AskOffer;
                // Always what is here, even what the source just sent: on a
                // hop to a third PC, the source only passes it on.
                let reply = match (read(), transfers) {
                    (Some(Content::Files(paths)), Some(transfers)) if wants_offer => {
                        offer_or_files(transfers, paths).into()
                    }
                    (content, _) => content.map_or(Wire::Nothing, Wire::Content),
                };
                link.send_bulk(&encode_or_nothing(&reply))?;
            }
            // Only from the source whose keyboard is on this PC, which
            // brings its clipboard along; files are not even saved
            // otherwise. A refusal logs who sent it, never what.
            Wire::Content(content) => match may_write() {
                Ok(()) => write(content),
                Err(who) => eprintln!(
                    "mickey: a clipboard from {who}, a PC not in control here, was refused"
                ),
            },
            Wire::Files(files) => match may_write() {
                Ok(()) => {
                    // Nothing came (only folders were copied): nothing pasted.
                    if let Some(paths) = save(files).filter(|paths| !paths.is_empty()) {
                        write(Content::Files(paths));
                    }
                }
                Err(who) => refused_files(&who),
            },
            Wire::NotOffered(why) => {
                if may_write().is_ok() {
                    let who = mickey_config::peer_name_or_address(link.peer_ip()?);
                    crate::transfer::show::failed(&who, &why);
                }
            }
            Wire::Offer(offer) => match (may_write(), transfers) {
                (Ok(()), Some(transfers)) => transfers.fetch(offer, link.peer_ip()?),
                (Ok(()), None) => eprintln!("mickey: files offered, but this PC cannot fetch them"),
                (Err(who), _) => refused_files(&who),
            },
            Wire::Nothing => {}
        }
    }
}

/// Logs files refused from `who`, a PC whose keyboard is not here; never
/// what they were.
fn refused_files(who: &str) {
    eprintln!("mickey: files from {who}, a PC not in control here, were refused");
}

/// Whether `content` is what this PC and a peer last exchanged
/// (`shared`), so the peer already has it.
fn already(shared: Option<u64>, content: &Content) -> bool {
    shared.is_some() && content.fingerprint() == shared
}

/// Copied `paths` as an offer from this PC; or why not (too many, or gone),
/// for the other PC to show: a folder never crosses whole, so sending the
/// files whole instead would bring nothing.
fn offer_or_files(transfers: &Transfers, paths: Vec<PathBuf>) -> Carried {
    match transfers.offer(&paths) {
        Ok(offer) => Carried::Offer(offer),
        Err(why) => {
            eprintln!("mickey: files not offered: {why}");
            Carried::NotOffered(why)
        }
    }
}

/// How long after the keyboard leaves this PC its clipboard may be asked
/// for, once: the source asks right after the switch.
const ASK_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a peer may stay silent while answering `Ask`. Per read: a big
/// answer still arriving is not cut short.
const ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// How long a source waits, as a data link opens, for the target to say it
/// knows file transfers: one that does answers at once, an older one never.
const TRANSFERS_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// The source's side of `TRANSFERS`, as the link opens and before anything
/// is asked on it: says this side knows file transfers, and whether the
/// target answered that it does too. Run where the link is opened, so the
/// wait for an older Mickey holds up no clipboard.
pub fn ask_transfers(link: &mut Link) -> io::Result<bool> {
    let transfers = crate::net::group::TRANSFERS;
    link.send_bulk(&crate::net::group::tagged(transfers, ""))?;
    // Nothing within the wait: an older Mickey. Something: the answer, read
    // whole (a target answers nothing else unasked).
    if !link.sends_within(TRANSFERS_WAIT)? {
        return Ok(false);
    }
    // Once begun, an answer still arriving gets as long as any other.
    link.set_read_timeout(Some(ANSWER_WAIT))?;
    Ok(link.recv_bulk()? == [transfers])
}

/// The next clipboard message on `link`, past any tagged one (kind 16 and
/// up): a `TRANSFERS` answer that came after `TRANSFERS_WAIT` must not be
/// read as the answer to `Ask`.
fn recv_clipboard(link: &mut Link) -> io::Result<Vec<u8>> {
    loop {
        let bytes = link.recv_bulk()?;
        if bytes
            .first()
            .is_none_or(|&kind| kind < crate::net::group::KIND)
        {
            return Ok(bytes);
        }
    }
}

/// Grants come at most this often: a peer that says its keyboard came
/// and went, over and over, cannot read the clipboard over and over.
const GRANT_EVERY: std::time::Duration = std::time::Duration::from_secs(3);

/// When a source may read this PC's clipboard (#185): once, within
/// `ASK_WINDOW` after its keyboard left this PC, and only if a key or a
/// button was pressed here meanwhile (something could have been copied),
/// at most once every `GRANT_EVERY`. A paired PC cannot poll it for copied
/// passwords by only saying its keyboard came and went.
#[derive(Debug, Default)]
pub struct ClipGrant {
    /// The source's keyboard is on this PC.
    here: bool,
    /// A key or button was pressed since it came.
    used: bool,
    /// Until when one ask is allowed.
    until: Option<Instant>,
    /// When the last grant was given.
    granted: Option<Instant>,
}

impl ClipGrant {
    /// The source's keyboard came here (`true`) or left (`false`).
    pub fn focus(&mut self, here: bool, now: Instant) {
        let rested = self.granted.is_none_or(|at| now >= at + GRANT_EVERY);
        if !here && self.here && self.used && rested {
            self.until = Some(now + ASK_WINDOW);
            self.granted = Some(now);
        }
        if here && !self.here {
            self.used = false;
        }
        self.here = here;
    }

    /// A key or button went down on this PC from this source.
    pub fn pressed(&mut self) {
        if self.here {
            self.used = true;
        }
    }

    /// Whether the source's keyboard is on this PC now.
    pub fn in_control(&self) -> bool {
        self.here
    }

    /// Whether an ask is allowed now; it uses the permission up.
    pub fn take(&mut self, now: Instant) -> bool {
        self.until.take().is_some_and(|until| now <= until)
    }
}

/// Each source's permission, by address.
pub type Grants = std::sync::Arc<std::sync::Mutex<HashMap<std::net::IpAddr, ClipGrant>>>;

/// Records that the source at `ip` pressed a key or button here.
pub fn pressed(grants: &Grants, ip: std::net::IpAddr) {
    if let Some(grant) = grants.lock_anyway().get_mut(&ip) {
        grant.pressed();
    }
}

/// Records a focus change from the source at `ip`.
pub fn focus_changed(grants: &Grants, ip: std::net::IpAddr, here: bool) {
    let mut grants = grants.lock_anyway();
    grants.entry(ip).or_default().focus(here, Instant::now());
}

/// Whether the source at `ip` may read the clipboard now. The ask and the
/// focus change travel on different links, so the change may still be on
/// its way: it is waited for, briefly.
pub fn may_read(grants: &Grants, ip: std::net::IpAddr) -> bool {
    wait_for(grants, ip, |grant| grant.take(Instant::now()))
}

/// Whether the source at `ip` may put something in this PC's clipboard
/// now: only while its keyboard is here, which is when it brings its
/// clipboard along. Before, any paired PC could replace a copied address or
/// command at any time (found in the security review after 0.3.0). The
/// clipboard and the focus change travel on different links, so the change
/// is waited for, briefly, as for `may_read`.
pub fn may_write(grants: &Grants, ip: std::net::IpAddr) -> bool {
    wait_for(grants, ip, |grant| grant.in_control())
}

/// Whether `check` passes for the source at `ip`, asked again for a short
/// while: the focus change it depends on travels on the other link.
fn wait_for(grants: &Grants, ip: std::net::IpAddr, check: impl Fn(&mut ClipGrant) -> bool) -> bool {
    /// The longest a focus change can trail the clipboard.
    const UP_TO: std::time::Duration = std::time::Duration::from_millis(500);
    const EVERY: std::time::Duration = std::time::Duration::from_millis(25);
    let started = Instant::now();
    loop {
        let allowed = grants.lock_anyway().get_mut(&ip).is_some_and(&check);
        if allowed || started.elapsed() > UP_TO {
            return allowed;
        }
        thread::sleep(EVERY);
    }
}

/// `wire` ready to send; what cannot be sent (files over the limit) goes
/// as `Nothing`, with the reason in the log, so the link stays up.
fn encode_or_nothing(wire: &Wire) -> Vec<u8> {
    wire::encode(wire).unwrap_or_else(|err| {
        eprintln!("mickey: clipboard not shared: {err}");
        wire::encode(&Wire::Nothing).expect("Nothing always encodes")
    })
}

/// Saves files from another PC (see `received`); `None`, logged, if not.
fn save(files: Vec<(String, Vec<u8>)>) -> Option<Vec<std::path::PathBuf>> {
    crate::clipboard::received::save(files)
        .inspect_err(|err| eprintln!("mickey: {err}"))
        .ok()
}

/// The source's side: the clipboard follows the keyboard. When it moves
/// from one PC to another, what is copied where it was is put where it
/// went. On its own thread, so reading, compressing and sending a big
/// image never holds up the keyboard.
#[derive(Clone)]
pub struct DataLinks {
    jobs: Sender<Job>,
    /// How many times the keyboard has moved; each `Job::Moved` carries
    /// its number, so a crossing knows when the keyboard has moved again.
    crossings: Arc<AtomicU64>,
}

enum Job {
    /// A data link to this peer is open; whether the peer knows file
    /// transfers (`ask_transfers`).
    Link(usize, Link, bool),
    /// The group map changed: send it to every peer (PRD 0010).
    Map(Vec<u8>),
    /// Send this to every peer once (the shared hotkeys).
    Broadcast(Vec<u8>),
    /// Who listens changed: send it to every peer, now and as links open.
    Audio(Vec<u8>),
    /// Send this to one peer (the switcher, shown there).
    SendTo(usize, Vec<u8>),
    /// The keyboard went from `from` to `to` (`None`: this PC); the
    /// `crossing`-th move. `asked`: the owner asked for the clipboard
    /// (#437), so it goes even if `to` seems to have it.
    Moved {
        from: Option<usize>,
        to: Option<usize>,
        crossing: u64,
        asked: bool,
    },
}

impl DataLinks {
    /// With `transfers`, copied files cross as offers between PCs that know
    /// them (PRD 0015).
    pub fn start(keeper: Keeper, transfers: Option<Arc<Transfers>>) -> DataLinks {
        let (jobs, todo) = mpsc::channel();
        let crossings = Arc::new(AtomicU64::new(0));
        let latest = Arc::clone(&crossings);
        crate::threads::spawn("clip mover", move || {
            let mut links: HashMap<usize, Link> = HashMap::new();
            // The latest map and listener, for links that open after they
            // were shared.
            let mut map: Option<Vec<u8>> = None;
            let mut audio: Option<Vec<u8>> = None;
            // The peers whose Mickey knows file transfers (PRD 0015): only
            // they are ever sent an offer (#478).
            let mut knows: HashSet<usize> = HashSet::new();
            // What this PC and each peer last exchanged (#424): never
            // sent to it again, as it has it.
            let mut shared: HashMap<usize, u64> = HashMap::new();
            for job in todo {
                match job {
                    Job::Link(peer, mut link, knows_transfers) => {
                        // A peer that goes silent mid-answer (lid closed,
                        // Wi-Fi gone) must not hold this one thread, and
                        // every clipboard after it, for good: a read waits
                        // this long, then the link is dropped.
                        if link.set_read_timeout(Some(ANSWER_WAIT)).is_err() {
                            continue;
                        }
                        let version = crate::net::group::tagged(
                            crate::net::group::VERSION,
                            mickey_config::version::THIS,
                        );
                        let sent = [Some(&version), map.as_ref(), audio.as_ref()]
                            .into_iter()
                            .flatten()
                            .all(|bytes| link.send_bulk(bytes).is_ok());
                        if sent {
                            if knows_transfers {
                                knows.insert(peer);
                            } else {
                                knows.remove(&peer);
                            }
                            // A new link starts knowing nothing: the peer
                            // may have restarted with another clipboard.
                            shared.remove(&peer);
                            links.insert(peer, link);
                        }
                    }
                    Job::Broadcast(bytes) => {
                        links.retain(|_, link| link.send_bulk(&bytes).is_ok());
                    }
                    Job::SendTo(peer, bytes) => {
                        if let Some(link) = links.get_mut(&peer)
                            && link.send_bulk(&bytes).is_err()
                        {
                            links.remove(&peer);
                        }
                    }
                    Job::Map(bytes) => {
                        links.retain(|_, link| link.send_bulk(&bytes).is_ok());
                        map = Some(bytes);
                    }
                    Job::Audio(bytes) => {
                        links.retain(|_, link| link.send_bulk(&bytes).is_ok());
                        audio = Some(bytes);
                    }
                    Job::Moved {
                        from,
                        to,
                        crossing,
                        asked,
                    } => {
                        let takers = OfferTakers {
                            transfers: transfers.as_ref(),
                            knows: &knows,
                        };
                        if asked && let Some(to) = to {
                            shared.remove(&to);
                        }
                        // The keyboard moved again since (a quick bounce,
                        // or this thread was busy): this clipboard would
                        // reach a PC the keyboard already left, which
                        // refuses it (#436). The newer crossing carries.
                        // To this PC it still goes: here nothing refuses
                        // it, and the newer crossing carries it on.
                        let current = || latest.load(Ordering::SeqCst) == crossing;
                        if to.is_none() || current() {
                            let peers = Peers {
                                links: &mut links,
                                shared: &mut shared,
                            };
                            carry(&keeper, peers, takers, (from, to), current);
                        }
                    }
                }
            }
        });
        DataLinks { jobs, crossings }
    }

    pub fn link(&self, peer: usize, link: Link, knows_transfers: bool) {
        let _ = self.jobs.send(Job::Link(peer, link, knows_transfers));
    }

    /// Shares the group map with every peer, now and as links open.
    pub fn share_map(&self, text: &str) {
        let _ = self.jobs.send(Job::Map(crate::net::group::message(text)));
    }

    /// Asks `peer` to show the switcher with these cards.
    pub fn show_switcher(&self, peer: usize, cards: &str) {
        let _ = self.jobs.send(Job::SendTo(
            peer,
            crate::net::group::tagged(crate::net::group::SWITCHER, cards),
        ));
    }

    /// Asks `peer` to show a copy of the switcher, to look at only.
    pub fn mirror_switcher(&self, peer: usize, cards: &str) {
        let _ = self.jobs.send(Job::SendTo(
            peer,
            crate::net::group::tagged(crate::net::group::MIRROR, cards),
        ));
    }

    /// Tells every peer to close the switcher or copy shown for this PC.
    pub fn close_switchers(&self) {
        let _ = self.jobs.send(Job::Broadcast(crate::net::group::tagged(
            crate::net::group::CLOSE,
            "",
        )));
    }

    /// Sends this PC's `[hotkeys]` lines to every peer.
    pub fn share_hotkeys(&self, lines: &str) {
        let _ = self.jobs.send(Job::Broadcast(crate::net::group::tagged(
            crate::net::group::HOTKEYS,
            lines,
        )));
    }

    /// Gives `peer`, the PC that listens, this PC's audio stream key.
    pub fn give_audio_key(&self, peer: usize, hex: &str) {
        let _ = self.jobs.send(Job::SendTo(
            peer,
            crate::net::group::tagged(crate::net::group::AUDIO_KEY, hex),
        ));
    }

    /// Tells every peer who listens (`[audio]`'s shared lines), now and as
    /// links open.
    pub fn share_audio(&self, lines: &str) {
        let _ = self.jobs.send(Job::Audio(crate::net::group::tagged(
            crate::net::group::AUDIO,
            lines,
        )));
    }

    pub fn moved(&self, from: Option<usize>, to: Option<usize>) {
        self.carry(from, to, false);
    }

    /// The owner asked for the clipboard of `from` to come to `to` (#437).
    pub fn bring(&self, from: Option<usize>, to: Option<usize>) {
        self.carry(from, to, true);
    }

    fn carry(&self, from: Option<usize>, to: Option<usize>, asked: bool) {
        let crossing = self.crossings.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = self.jobs.send(Job::Moved {
            from,
            to,
            crossing,
            asked,
        });
    }
}

/// The source's open data links, and what it last exchanged with each
/// peer through them.
struct Peers<'a> {
    links: &'a mut HashMap<usize, Link>,
    shared: &'a mut HashMap<usize, u64>,
}

/// Which PCs take copied files as an offer: this one if it has
/// `transfers`, a peer if it also said it knows them.
#[derive(Clone, Copy)]
struct OfferTakers<'a> {
    transfers: Option<&'a Arc<Transfers>>,
    knows: &'a HashSet<usize>,
}

impl OfferTakers<'_> {
    fn takes(&self, peer: Option<usize>) -> bool {
        self.transfers.is_some() && peer.is_none_or(|peer| self.knows.contains(&peer))
    }
}

/// What a crossing carries: the clipboard itself, or copied files as an
/// offer to fetch.
enum Carried {
    Content(Content),
    Offer(FileOffer),
    /// Copied files that cannot be offered, and why.
    NotOffered(String),
}

impl From<Carried> for Wire {
    fn from(carried: Carried) -> Wire {
        match carried {
            Carried::Content(content) => Wire::Content(content),
            Carried::Offer(offer) => Wire::Offer(offer),
            Carried::NotOffered(why) => Wire::NotOffered(why),
        }
    }
}

/// The keyboard went from `from` to `to` (`None`: this PC): what is
/// copied at `from` is put at `to`. Files go as an offer when both ends
/// take one. It gives up before sending to a peer once `current` says the
/// keyboard has moved again.
fn carry(
    keeper: &Keeper,
    mut peers: Peers,
    takers: OfferTakers,
    (from, to): (Option<usize>, Option<usize>),
    current: impl Fn() -> bool,
) {
    let started = Instant::now();
    let offers = takers.takes(from) && takers.takes(to);
    let carried = match from {
        None => read_here(keeper, takers.transfers.filter(|_| offers && to.is_some())),
        Some(peer) => ask_peer(&mut peers, peer, offers),
    };
    let Some(carried) = carried else {
        return;
    };
    if to.is_some() && !current() {
        return;
    }
    match to {
        None => put_here(keeper, takers.transfers, carried),
        Some(peer) => send_to_peer(&mut peers, peer, carried, started),
    }
}

/// What is copied on this PC; copied files as an offer, with `transfers`.
fn read_here(keeper: &Keeper, transfers: Option<&Arc<Transfers>>) -> Option<Carried> {
    keeper.read().map(|content| match (content, transfers) {
        (Content::Files(paths), Some(transfers)) => offer_or_files(transfers, paths),
        (content, _) => Carried::Content(content),
    })
}

/// What is copied on `peer`, asked over its data link; copied files as an
/// offer, if `offers`. A link that fails is dropped.
fn ask_peer(peers: &mut Peers, peer: usize, offers: bool) -> Option<Carried> {
    let ask = if offers { Wire::AskOffer } else { Wire::Ask };
    let asked = peers.links.get_mut(&peer).map(|link| {
        link.send_bulk(&wire::encode(&ask).map_err(io::Error::other)?)?;
        let answer = wire::decode(&recv_clipboard(link)?).map_err(io::Error::other)?;
        Ok::<_, io::Error>((answer, link.peer_ip()?, *link.peer_key()))
    });
    // What the peer holds is known again only from a text or an image it
    // sent: anything else (files, no grant, an error) and it may hold
    // something new, so the next crossing to it sends, whatever it is.
    match asked.as_ref().and_then(|answer| match answer {
        Ok((Wire::Content(content), _, _)) => content.fingerprint(),
        _ => None,
    }) {
        Some(fingerprint) => peers.shared.insert(peer, fingerprint),
        None => peers.shared.remove(&peer),
    };
    match asked {
        Some(Ok((Wire::Content(content), _, _))) => Some(Carried::Content(content)),
        Some(Ok((Wire::Files(files), _, _))) => {
            save(files).map(|paths| Carried::Content(Content::Files(paths)))
        }
        // A peer offers only its own files, from its own address: the one
        // its data link comes from, whatever it says.
        Some(Ok((Wire::Offer(mut offer), ip, key))) if offer.key == key => {
            offer.ip = Some(ip);
            Some(Carried::Offer(offer))
        }
        Some(Ok((Wire::NotOffered(why), ip, _))) => {
            let who = mickey_config::peer_name_or_address(ip);
            Some(Carried::NotOffered(format!("{who}: {why}")))
        }
        Some(Ok(_)) | None => None,
        Some(Err(_)) => {
            peers.links.remove(&peer);
            None
        }
    }
}

/// Puts `carried` on this PC: in its clipboard, or fetched first.
fn put_here(keeper: &Keeper, transfers: Option<&Arc<Transfers>>, carried: Carried) {
    match carried {
        Carried::Content(content) => keeper.write(content),
        Carried::NotOffered(why) => crate::transfer::show::failed("", &why),
        Carried::Offer(offer) => {
            if let Some(transfers) = transfers
                && let Some(ip) = offer.ip
            {
                transfers.fetch(offer, ip);
            }
        }
    }
}

/// Sends `carried` to `peer`, unless it has it already. A link that fails
/// is dropped.
fn send_to_peer(peers: &mut Peers, peer: usize, carried: Carried, started: Instant) {
    /// Over this, how long a send took is logged.
    const LOG_OVER: usize = 64 << 10;
    let fingerprint = match &carried {
        Carried::Content(content) => {
            // It was sent there, or came from there.
            if already(peers.shared.get(&peer).copied(), content) {
                return;
            }
            content.fingerprint()
        }
        _ => None,
    };
    let Some(link) = peers.links.get_mut(&peer) else {
        return;
    };
    let kind = match &carried {
        Carried::Content(content) => format!("{content:?}"),
        Carried::Offer(_) => "an offer".to_string(),
        Carried::NotOffered(_) => "files not offered".to_string(),
    };
    let bytes = encode_or_nothing(&Wire::from(carried));
    let sent = link.send_bulk(&bytes);
    // Remembered only once it is there; files can change behind a path.
    match (&sent, fingerprint) {
        (Ok(()), Some(fingerprint)) => peers.shared.insert(peer, fingerprint),
        _ => peers.shared.remove(&peer),
    };
    match sent {
        // The kind and size only: the content may be secret.
        Ok(()) if bytes.len() > LOG_OVER => println!(
            "mickey: clipboard ({kind}) sent as {} KB in {} ms",
            bytes.len() >> 10,
            started.elapsed().as_millis()
        ),
        Ok(()) => {}
        Err(_) => {
            peers.links.remove(&peer);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_clipboard_may_be_read_once_right_after_the_keyboard_leaves() {
        let now = Instant::now();
        let second = std::time::Duration::from_secs(1);
        let mut grant = ClipGrant::default();
        assert!(!grant.take(now), "never without a focus change");
        grant.focus(false, now);
        assert!(!grant.take(now), "leaving a PC it never had gives nothing");
        grant.focus(true, now);
        grant.pressed();
        assert!(!grant.take(now), "not while the keyboard is here");
        grant.focus(false, now);
        assert!(grant.take(now + second));
        assert!(!grant.take(now + second), "once");
        let later = now + GRANT_EVERY;
        grant.focus(true, later);
        grant.pressed();
        grant.focus(false, later);
        assert!(!grant.take(later + ASK_WINDOW + second), "too late");
    }

    #[test]
    fn coming_and_going_alone_gives_no_clipboard() {
        let now = Instant::now();
        let mut grant = ClipGrant::default();
        grant.focus(true, now);
        grant.focus(false, now);
        assert!(!grant.take(now), "nothing was pressed here");
        grant.focus(true, now);
        grant.pressed();
        grant.focus(false, now);
        assert!(grant.take(now));
        grant.focus(true, now);
        grant.pressed();
        grant.focus(false, now);
        assert!(!grant.take(now), "a second grant this soon");
    }

    use super::*;
    use crate::test_util::link_pair;
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn only_the_source_in_control_may_write() {
        let grants = Grants::default();
        let ip: std::net::IpAddr = "192.0.2.5".parse().unwrap();
        assert!(!may_write(&grants, ip), "a PC never in control");
        focus_changed(&grants, ip, true);
        assert!(may_write(&grants, ip));
        focus_changed(&grants, ip, false);
        assert!(!may_write(&grants, ip), "its keyboard left");
    }

    #[test]
    fn the_target_answers_and_takes_what_it_is_sent() {
        let (mut link, theirs) = link_pair();
        let written = Arc::new(Mutex::new(Vec::new()));
        let target_written = Arc::clone(&written);
        let target = thread::spawn(move || {
            let link = theirs;
            // The target's clipboard holds a text.
            let _ = answer(
                link,
                // The keyboard just left the target, which holds a text.
                || Some(Content::Text("from the target".into())),
                // The source's keyboard is here.
                || Ok(()),
                |content| target_written.lock().unwrap().push(content),
                |_, _| {},
                || true,
                None,
            );
        });
        let ask = |link: &mut Link, wire: Wire| link.send_bulk(&wire::encode(&wire).unwrap());

        ask(&mut link, Wire::Ask).unwrap();
        let reply = wire::decode(&link.recv_bulk().unwrap()).unwrap();
        assert_eq!(
            reply,
            Wire::Content(Content::Text("from the target".into()))
        );

        ask(
            &mut link,
            Wire::Content(Content::Text("from the source".into())),
        )
        .unwrap();
        drop(link);
        target.join().unwrap();
        assert_eq!(
            *written.lock().unwrap(),
            [Content::Text("from the source".into())]
        );
    }

    #[test]
    fn a_target_says_it_knows_transfers_and_the_clipboard_goes_on() {
        let (mut link, theirs) = link_pair();
        let target = thread::spawn(move || {
            let link = theirs;
            let _ = answer(
                link,
                || Some(Content::Text("copied".into())),
                || Ok(()),
                |_| {},
                |_, _| {},
                || true,
                None,
            );
        });
        assert!(ask_transfers(&mut link).unwrap());
        link.send_bulk(&wire::encode(&Wire::Ask).unwrap()).unwrap();
        let reply = wire::decode(&link.recv_bulk().unwrap()).unwrap();
        assert_eq!(reply, Wire::Content(Content::Text("copied".into())));
        drop(link);
        target.join().unwrap();
    }

    #[test]
    fn an_older_target_that_says_nothing_does_not_know_transfers() {
        let (mut link, theirs) = link_pair();
        let target = thread::spawn(move || {
            let mut link = theirs;
            // As an older Mickey: it reads the message and answers nothing.
            let _ = link.recv_bulk();
            let _ = link.recv_bulk();
        });
        let started = Instant::now();
        assert!(!ask_transfers(&mut link).unwrap());
        assert!(started.elapsed() < TRANSFERS_WAIT * 2);
        drop(link);
        target.join().unwrap();
    }

    #[test]
    fn a_clipboard_from_a_pc_not_in_control_is_refused() {
        let (mut link, theirs) = link_pair();
        let written = Arc::new(Mutex::new(Vec::new()));
        let target_written = Arc::clone(&written);
        let target = thread::spawn(move || {
            let link = theirs;
            let _ = answer(
                link,
                || None,
                // The source's keyboard is elsewhere: refused, named.
                || Err("laptop".into()),
                |content| target_written.lock().unwrap().push(content),
                |_, _| {},
                || true,
                None,
            );
        });
        let sent = Wire::Content(Content::Text("from the source".into()));
        link.send_bulk(&wire::encode(&sent).unwrap()).unwrap();
        drop(link);
        target.join().unwrap();
        assert!(written.lock().unwrap().is_empty());
    }

    #[test]
    fn a_crossing_overtaken_by_another_sends_nothing() {
        let (link, theirs) = link_pair();
        let target = thread::spawn(move || {
            let mut link = theirs;
            wire::decode(&link.recv_bulk().unwrap()).unwrap()
        });
        let mut links = HashMap::from([(0, link)]);
        let mut shared = HashMap::new();
        let keeper = Keeper::in_memory(Some(Content::Text("copied before".into())));
        let knows = HashSet::new();
        let takers = OfferTakers {
            transfers: None,
            knows: &knows,
        };
        // The keyboard already left peer 0: nothing goes.
        carry(
            &keeper,
            Peers {
                links: &mut links,
                shared: &mut shared,
            },
            takers,
            (None, Some(0)),
            || false,
        );
        // It came back and stays, with something new copied: that goes.
        keeper.write(Content::Text("copied here".into()));
        carry(
            &keeper,
            Peers {
                links: &mut links,
                shared: &mut shared,
            },
            takers,
            (None, Some(0)),
            || true,
        );
        assert_eq!(
            target.join().unwrap(),
            Wire::Content(Content::Text("copied here".into())),
            "the first message the target gets is the current crossing's"
        );
    }

    /// The scenario of #424 (found by @wendelmax): copy on A, move the
    /// keyboard to B, back to A and to B again, copying nothing new.
    #[test]
    fn the_same_clipboard_is_not_sent_again_to_the_same_peer() {
        let (link, theirs) = link_pair();
        // What reached B's clipboard, in order; B's clipboard is the last.
        let written = Arc::new(Mutex::new(Vec::new()));
        let b_written = Arc::clone(&written);
        let b = thread::spawn(move || {
            let link = theirs;
            let read_back = Arc::clone(&b_written);
            let _ = answer(
                link,
                move || read_back.lock().unwrap().last().cloned(),
                || Ok(()),
                |content| b_written.lock().unwrap().push(content),
                |_, _| {},
                || true,
                None,
            );
        });
        let mut links = HashMap::from([(0, link)]);
        let mut shared = HashMap::new();
        let keeper = Keeper::in_memory(Some(Content::Text("copied on A".into())));
        let knows = HashSet::new();
        let takers = OfferTakers {
            transfers: None,
            knows: &knows,
        };
        let mut cross = |from, to| {
            let peers = Peers {
                links: &mut links,
                shared: &mut shared,
            };
            carry(&keeper, peers, takers, (from, to), || true);
        };
        cross(None, Some(0));
        cross(Some(0), None);
        cross(None, Some(0));
        keeper.write(Content::Text("copied on A again".into()));
        cross(None, Some(0));
        drop(cross);
        drop(links);
        b.join().unwrap();
        assert_eq!(
            *written.lock().unwrap(),
            [
                Content::Text("copied on A".into()),
                Content::Text("copied on A again".into()),
            ],
            "once each: B already had it on the second crossing"
        );
        assert_eq!(
            keeper.read(),
            Some(Content::Text("copied on A again".into()))
        );
    }

    /// A target that holds `clipboard`, grants every ask, and records what
    /// it is sent: as `answer` runs for one PC.
    fn target(clipboard: Arc<Mutex<Option<Content>>>) -> Link {
        let (ours, theirs) = link_pair();
        thread::spawn(move || {
            let (read, written) = (Arc::clone(&clipboard), clipboard);
            let _ = answer(
                theirs,
                move || read.lock().unwrap().clone(),
                || Ok(()),
                move |content| *written.lock().unwrap() = Some(content),
                |_, _| {},
                || true,
                None,
            );
        });
        ours
    }

    fn cross(
        keeper: &Keeper,
        links: &mut HashMap<usize, Link>,
        shared: &mut HashMap<usize, u64>,
        from: Option<usize>,
        to: Option<usize>,
    ) {
        let knows = HashSet::new();
        let takers = OfferTakers {
            transfers: None,
            knows: &knows,
        };
        let peers = Peers { links, shared };
        carry(keeper, peers, takers, (from, to), || true);
    }

    /// Waits a moment for what reaches `clipboard` over a link.
    fn settled(clipboard: &Arc<Mutex<Option<Content>>>) -> Option<Content> {
        thread::sleep(std::time::Duration::from_millis(200));
        clipboard.lock().unwrap().clone()
    }

    #[test]
    fn a_hop_through_a_pc_that_was_just_sent_it_still_arrives() {
        let (b, c) = (Arc::new(Mutex::new(None)), Arc::new(Mutex::new(None)));
        let mut links = HashMap::from([(0, target(Arc::clone(&b))), (1, target(Arc::clone(&c)))]);
        let mut shared = HashMap::new();
        let keeper = Keeper::in_memory(Some(Content::Text("copied on A".into())));
        // A to B, then B to C: B holds what A sent, and passes it on.
        cross(&keeper, &mut links, &mut shared, None, Some(0));
        assert_eq!(settled(&b), Some(Content::Text("copied on A".into())));
        cross(&keeper, &mut links, &mut shared, Some(0), Some(1));
        assert_eq!(settled(&c), Some(Content::Text("copied on A".into())));
    }

    #[test]
    fn a_peer_that_answers_nothing_may_hold_something_new() {
        let b = Arc::new(Mutex::new(None));
        let mut links = HashMap::from([(0, target(Arc::clone(&b)))]);
        let mut shared = HashMap::new();
        let keeper = Keeper::in_memory(Some(Content::Text("X".into())));
        cross(&keeper, &mut links, &mut shared, None, Some(0));
        assert_eq!(settled(&b), Some(Content::Text("X".into())));
        // Copied on B by B's own keyboard; then B answers nothing (here,
        // an empty clipboard stands for no grant).
        *b.lock().unwrap() = None;
        cross(&keeper, &mut links, &mut shared, Some(0), None);
        *b.lock().unwrap() = Some(Content::Text("Y, B's own".into()));
        // X again to B: B may not have it any more, so it goes.
        cross(&keeper, &mut links, &mut shared, None, Some(0));
        assert_eq!(settled(&b), Some(Content::Text("X".into())));
    }

    #[test]
    fn an_overtaken_crossing_to_this_pc_still_brings_the_clipboard() {
        let b = Arc::new(Mutex::new(Some(Content::Text("copied on B".into()))));
        let mut links = HashMap::from([(0, target(Arc::clone(&b)))]);
        let mut shared = HashMap::new();
        let keeper = Keeper::in_memory(Some(Content::Text("A's old".into())));
        let knows = HashSet::new();
        let takers = OfferTakers {
            transfers: None,
            knows: &knows,
        };
        let peers = Peers {
            links: &mut links,
            shared: &mut shared,
        };
        // B to this PC, already overtaken by a newer crossing to C: it
        // still lands here, so the newer crossing carries it on.
        carry(&keeper, peers, takers, (Some(0), None), || false);
        assert_eq!(keeper.read(), Some(Content::Text("copied on B".into())));
    }
}
