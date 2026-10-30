//! Moving the clipboard between PCs (PRD 0011), over the data link.

use crate::clip_wire::{self, Wire};
use crate::clipboard::{Content, Keeper};
use crate::secure::Link;
use crate::threads::LockAnyway;
use std::collections::HashMap;
use std::io;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Instant;

/// The target's side of a data link, until it closes: `Ask` is answered
/// with what `read` finds copied here, and content sent here is handed to
/// `write`, which puts it in this PC's clipboard.
/// Messages of kind 16 and up (the group map, the shared hotkeys) are not
/// the clipboard's: they go to `other`, with their kind.
pub fn answer(
    mut link: Link,
    may_read: impl Fn() -> bool,
    read: impl Fn() -> Option<Content>,
    may_write: impl Fn() -> bool,
    write: impl Fn(Content),
    other: impl Fn(u8, &str),
) -> io::Result<()> {
    loop {
        let bytes = link.recv_bulk()?;
        if let Some((&kind, text)) = bytes.split_first()
            && kind >= crate::group::KIND
        {
            other(kind, &String::from_utf8_lossy(text));
            continue;
        }
        let message = clip_wire::decode(&bytes).map_err(io::Error::other)?;
        match message {
            Wire::Ask => {
                // Only right after the keyboard left this PC (#185).
                let reply = if may_read() {
                    read().map_or(Wire::Nothing, Wire::Content)
                } else {
                    Wire::Nothing
                };
                link.send_bulk(&encode_or_nothing(&reply))?;
            }
            // Only from the source whose keyboard is on this PC, which
            // brings its clipboard along; files are not even saved
            // otherwise.
            Wire::Content(content) => {
                if may_write() {
                    write(content);
                } else {
                    eprintln!("mickey: a clipboard from a PC not in control here was refused");
                }
            }
            Wire::Files(files) => {
                if !may_write() {
                    eprintln!("mickey: files from a PC not in control here were refused");
                } else if let Some(paths) = save(files) {
                    write(Content::Files(paths));
                }
            }
            Wire::Nothing => {}
        }
    }
}

/// How long after the keyboard leaves this PC its clipboard may be asked
/// for, once: the source asks right after the switch.
const ASK_WINDOW: std::time::Duration = std::time::Duration::from_secs(5);

/// How long a peer may stay silent while answering `Ask`. Per read: a big
/// answer still arriving is not cut short.
const ANSWER_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// When a source may read this PC's clipboard (#185): once, within
/// `ASK_WINDOW` after its keyboard left this PC, never at any other time,
/// so a paired PC cannot poll it for copied passwords.
#[derive(Debug, Default)]
pub struct ClipGrant {
    /// The source's keyboard is on this PC.
    here: bool,
    /// Until when one ask is allowed.
    until: Option<Instant>,
}

impl ClipGrant {
    /// The source's keyboard came here (`true`) or left (`false`).
    pub fn focus(&mut self, here: bool, now: Instant) {
        if !here && self.here {
            self.until = Some(now + ASK_WINDOW);
        }
        self.here = here;
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

/// Records a focus change from the source at `ip`.
pub fn focus_changed(grants: &Grants, ip: std::net::IpAddr, here: bool) {
    let mut grants = grants.lock_anyway();
    grants.entry(ip).or_default().focus(here, Instant::now());
}

/// Whether the source at `ip` may read the clipboard now. The ask and the
/// focus change travel on different links, so the change may still be on
/// its way: it is waited for, briefly.
pub fn may_read(grants: &Grants, ip: std::net::IpAddr) -> bool {
    let started = Instant::now();
    loop {
        let allowed = grants
            .lock_anyway()
            .get_mut(&ip)
            .is_some_and(|grant| grant.take(Instant::now()));
        if allowed || started.elapsed() > std::time::Duration::from_millis(500) {
            return allowed;
        }
        thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// Whether the source at `ip` may put something in this PC's clipboard
/// now: only while its keyboard is here, which is when it brings its
/// clipboard along. Before, any paired PC could replace a copied address or
/// command at any time (found in the security review after 0.3.0). The
/// clipboard and the focus change travel on different links, so the change
/// is waited for, briefly, as for `may_read`.
pub fn may_write(grants: &Grants, ip: std::net::IpAddr) -> bool {
    let started = Instant::now();
    loop {
        let allowed = grants
            .lock_anyway()
            .get(&ip)
            .is_some_and(ClipGrant::in_control);
        if allowed || started.elapsed() > std::time::Duration::from_millis(500) {
            return allowed;
        }
        thread::sleep(std::time::Duration::from_millis(25));
    }
}

/// `wire` ready to send; what cannot be sent (files over the limit) goes
/// as `Nothing`, with the reason in the log, so the link stays up.
fn encode_or_nothing(wire: &Wire) -> Vec<u8> {
    clip_wire::encode(wire).unwrap_or_else(|err| {
        eprintln!("mickey: clipboard not shared: {err}");
        clip_wire::encode(&Wire::Nothing).expect("Nothing always encodes")
    })
}

/// Saves files from another PC (see `received`); `None`, logged, if not.
fn save(files: Vec<(String, Vec<u8>)>) -> Option<Vec<std::path::PathBuf>> {
    crate::received::save(files)
        .inspect_err(|err| eprintln!("mickey: {err}"))
        .ok()
}

/// The source's side: the clipboard follows the keyboard. When it moves
/// from one PC to another, what is copied where it was is put where it
/// went. On its own thread, so reading, compressing and sending a big
/// image never holds up the keyboard.
#[derive(Clone)]
pub struct Mover(Sender<Job>);

enum Job {
    /// A data link to this peer is open.
    Link(usize, Link),
    /// The group map changed: send it to every peer (PRD 0010).
    Map(Vec<u8>),
    /// Send this to every peer once (the shared hotkeys).
    Broadcast(Vec<u8>),
    /// Who listens changed: send it to every peer, now and as links open.
    Audio(Vec<u8>),
    /// Send this to one peer (the switcher, shown there).
    SendTo(usize, Vec<u8>),
    /// The keyboard went from `from` to `to` (`None`: this PC).
    Moved {
        from: Option<usize>,
        to: Option<usize>,
    },
}

impl Mover {
    pub fn start(keeper: Keeper) -> Mover {
        let (jobs, todo) = mpsc::channel();
        crate::threads::spawn("clip mover", move || {
            let mut links: HashMap<usize, Link> = HashMap::new();
            // The latest map and listener, for links that open after they
            // were shared.
            let mut map: Option<Vec<u8>> = None;
            let mut audio: Option<Vec<u8>> = None;
            for job in todo {
                match job {
                    Job::Link(peer, mut link) => {
                        // A peer that goes silent mid-answer (lid closed,
                        // Wi-Fi gone) must not hold this one thread, and
                        // every clipboard after it, for good: a read waits
                        // this long, then the link is dropped.
                        if link.set_read_timeout(Some(ANSWER_WAIT)).is_err() {
                            continue;
                        }
                        let version = crate::group::tagged(
                            crate::group::VERSION,
                            mickey_config::version::THIS,
                        );
                        let sent = [Some(&version), map.as_ref(), audio.as_ref()]
                            .into_iter()
                            .flatten()
                            .all(|bytes| link.send_bulk(bytes).is_ok());
                        if sent {
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
                    Job::Moved { from, to } => carry(&keeper, &mut links, from, to),
                }
            }
        });
        Mover(jobs)
    }

    pub fn link(&self, peer: usize, link: Link) {
        let _ = self.0.send(Job::Link(peer, link));
    }

    /// Shares the group map with every peer, now and as links open.
    pub fn share_map(&self, text: &str) {
        let _ = self.0.send(Job::Map(crate::group::message(text)));
    }

    /// Asks `peer` to show the switcher with these cards.
    pub fn show_switcher(&self, peer: usize, cards: &str) {
        let _ = self.0.send(Job::SendTo(
            peer,
            crate::group::tagged(crate::group::SWITCHER, cards),
        ));
    }

    /// Asks `peer` to show a copy of the switcher, to look at only.
    pub fn mirror_switcher(&self, peer: usize, cards: &str) {
        let _ = self.0.send(Job::SendTo(
            peer,
            crate::group::tagged(crate::group::MIRROR, cards),
        ));
    }

    /// Tells every peer to close the switcher or copy shown for this PC.
    pub fn close_switchers(&self) {
        let _ = self.0.send(Job::Broadcast(crate::group::tagged(
            crate::group::CLOSE,
            "",
        )));
    }

    /// Sends this PC's `[hotkeys]` lines to every peer.
    pub fn share_hotkeys(&self, lines: &str) {
        let _ = self.0.send(Job::Broadcast(crate::group::tagged(
            crate::group::HOTKEYS,
            lines,
        )));
    }

    /// Gives `peer`, the PC that listens, this PC's audio stream key.
    pub fn give_audio_key(&self, peer: usize, hex: &str) {
        let _ = self.0.send(Job::SendTo(
            peer,
            crate::group::tagged(crate::group::AUDIO_KEY, hex),
        ));
    }

    /// Tells every peer who listens (`[audio]`'s shared lines), now and as
    /// links open.
    pub fn share_audio(&self, lines: &str) {
        let _ = self
            .0
            .send(Job::Audio(crate::group::tagged(crate::group::AUDIO, lines)));
    }

    pub fn moved(&self, from: Option<usize>, to: Option<usize>) {
        let _ = self.0.send(Job::Moved { from, to });
    }
}

/// Takes what is copied on `from` and puts it on `to`. A link that fails
/// is dropped; the next connection brings a new one.
fn carry(
    keeper: &Keeper,
    links: &mut HashMap<usize, Link>,
    from: Option<usize>,
    to: Option<usize>,
) {
    let started = Instant::now();
    let content = match from {
        None => keeper.read(),
        Some(peer) => {
            let asked = links.get_mut(&peer).map(|link| {
                link.send_bulk(&clip_wire::encode(&Wire::Ask).map_err(io::Error::other)?)?;
                clip_wire::decode(&link.recv_bulk()?).map_err(io::Error::other)
            });
            match asked {
                Some(Ok(Wire::Content(content))) => Some(content),
                Some(Ok(Wire::Files(files))) => save(files).map(Content::Files),
                Some(Ok(_)) | None => None,
                Some(Err(_)) => {
                    links.remove(&peer);
                    None
                }
            }
        }
    };
    let Some(content) = content else {
        return;
    };
    match to {
        None => keeper.write(content),
        Some(peer) => {
            let Some(link) = links.get_mut(&peer) else {
                return;
            };
            let kind = format!("{content:?}");
            let bytes = encode_or_nothing(&Wire::Content(content));
            let sent = link.send_bulk(&bytes).map(|()| bytes.len());
            match sent {
                // The kind and size only: the content may be secret.
                Ok(len) if len > 64 << 10 => println!(
                    "mickey: clipboard ({kind}) sent as {} KB in {} ms",
                    len >> 10,
                    started.elapsed().as_millis()
                ),
                Ok(_) => {}
                Err(_) => {
                    links.remove(&peer);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_clipboard_may_be_read_once_right_after_the_keyboard_leaves() {
        let now = Instant::now();
        let mut grant = ClipGrant::default();
        assert!(!grant.take(now), "never without a focus change");
        grant.focus(false, now);
        assert!(!grant.take(now), "leaving a PC it never had gives nothing");
        grant.focus(true, now);
        assert!(!grant.take(now), "not while the keyboard is here");
        grant.focus(false, now);
        assert!(grant.take(now + std::time::Duration::from_secs(1)));
        assert!(!grant.take(now + std::time::Duration::from_secs(1)), "once");
        grant.focus(true, now);
        grant.focus(false, now);
        assert!(
            !grant.take(now + ASK_WINDOW + std::time::Duration::from_millis(1)),
            "too late"
        );
    }

    use super::*;
    use crate::identity::Identity;
    use std::net::{TcpListener, TcpStream};
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
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let written = Arc::new(Mutex::new(Vec::new()));
        let target_written = Arc::clone(&written);
        let target = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let link = Link::respond(stream, &Identity::generate().unwrap()).unwrap();
            // The target's clipboard holds a text.
            let _ = answer(
                link,
                // The keyboard just left the target.
                || true,
                || Some(Content::Text("from the target".into())),
                // The source's keyboard is here.
                || true,
                |content| target_written.lock().unwrap().push(content),
                |_, _| {},
            );
        });
        let stream = TcpStream::connect(addr).unwrap();
        let mut link = Link::initiate(stream, &Identity::generate().unwrap()).unwrap();
        let ask = |link: &mut Link, wire: Wire| link.send_bulk(&clip_wire::encode(&wire).unwrap());

        ask(&mut link, Wire::Ask).unwrap();
        let reply = clip_wire::decode(&link.recv_bulk().unwrap()).unwrap();
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
}
