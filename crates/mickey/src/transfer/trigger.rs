//! Copy and paste triggers a transfer (PRD 0015, #478): a copied file list
//! crosses as an offer from the PC where the files are, and whoever ends up
//! with the clipboard fetches them from that PC, then pastes their paths.
//!
//! The clipboard carries a `FileOffer` (`file_offer`); what became of each
//! is in `fetches`, and the question before a big one in `bring`.

use crate::clipboard::{Content, Keeper};
use crate::net::identity::Identity;
use crate::net::secure::Link;
use crate::net::transport;
use crate::net::trust;
use crate::threads::LockAnyway;
use crate::transfer::bring;
use crate::transfer::fetches::{Begin, Fetches, Lanes};
use crate::transfer::file_offer::FileOffer;
use crate::transfer::offer;
use crate::transfer::offers::Offers;
use crate::transfer::progress::Stage;
use crate::transfer::receive::{self, Fetched};
use crate::transfer::show::{Ended, Stop, Watcher};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What a PC needs to offer the files copied on it and to fetch others'.
pub struct Transfers {
    pub offers: Arc<Offers>,
    pub identity: Arc<Identity>,
    /// This PC's input port, which its offers name.
    pub port: u16,
    /// Where fetched files go, a fresh folder per transfer: `transfers/`
    /// in the received files' folder (`clipboard::received::base`).
    pub received: PathBuf,
    /// Files are fetched only from a paired PC.
    pub trusted: trust::Shared,
    /// This PC's clipboard, where fetched files are pasted.
    keeper: Keeper,
    /// Each file or folder transfers brought here, and the PC it came from:
    /// they are never sent back there (#424). As long as Mickey runs.
    came_from: std::sync::Mutex<Vec<(PathBuf, [u8; 32])>>,
    /// One lane per PC files are fetched from: a transfer from a PC waits
    /// for the one before it from that PC to end, instead of both sharing
    /// the network and the disk.
    lanes: Lanes,
    /// What became of each offer fetched here, while Mickey runs: one
    /// offered again (the keyboard crossed again) is fetched once, and one
    /// that broke goes on from where it stopped (#475, #502).
    fetches: Fetches,
}

impl Transfers {
    /// Offers this PC's copied files from `offers`, as `identity`, naming
    /// `port`; fetches others' into `received`, from paired PCs only, and
    /// pastes them in `keeper`'s clipboard.
    pub fn new(
        offers: Arc<Offers>,
        identity: Arc<Identity>,
        port: u16,
        received: PathBuf,
        trusted: trust::Shared,
        keeper: Keeper,
    ) -> Transfers {
        Transfers {
            offers,
            identity,
            port,
            received,
            trusted,
            keeper,
            came_from: Default::default(),
            lanes: Default::default(),
            fetches: Default::default(),
        }
    }

    /// An offer of the files at `paths`, handed out by this PC (the last
    /// few stay fetchable). The IP is left out: the receiving PC knows it.
    pub fn offer(&self, paths: &[PathBuf]) -> Result<FileOffer, String> {
        let listing = offer::list(paths)?;
        // Nothing left to offer (the copied files were deleted or moved):
        // the other PC is told why.
        if listing.entries.is_empty() {
            return Err("none of the copied files is there any more".into());
        }
        // How many, never their names: what is copied stays out of the log
        // (CONTRIBUTING, the clipboard); the names are for the window.
        if !listing.skipped.is_empty() {
            println!(
                "mickey: {} copied files or folders not offered (links, names Windows refuses, unreadable)",
                listing.skipped.len()
            );
        }
        Ok(FileOffer {
            ip: None,
            port: self.port,
            key: *self.identity.public(),
            origin: self.origin_of(paths),
            offer: self.offers.hand_out(listing)?,
        })
    }

    /// The PC `paths` came from, if they all arrived here from one PC.
    fn origin_of(&self, paths: &[PathBuf]) -> Option<[u8; 32]> {
        let came_from = self.came_from.lock_anyway();
        let origin_of = |path: &PathBuf| {
            came_from
                .iter()
                .find(|(folder, _)| path.starts_with(folder))
                .map(|(_, key)| *key)
        };
        let first = origin_of(paths.first()?)?;
        paths
            .iter()
            .all(|path| origin_of(path) == Some(first))
            .then_some(first)
    }

    /// Fetches `offer` (which came from the PC at `from`) on a thread of its
    /// own, and once every file is there and verified puts them on the
    /// clipboard, unless something else was copied meanwhile.
    pub fn fetch(self: &Arc<Self>, offer: FileOffer, from: IpAddr) {
        // Files that came from here: this PC has them already, and its
        // clipboard keeps what it holds.
        if offer.origin == Some(*self.identity.public()) {
            return;
        }
        // Never from a PC that is not paired: before any folder is made or
        // any question asked, which such a PC could otherwise cause at will.
        if !self.trusted.lock_anyway().is_trusted(&offer.key) {
            return eprintln!("mickey: files offered by a PC that is not paired; not fetched");
        }
        let this = Arc::clone(self);
        crate::threads::spawn("transfer fetch", move || {
            let key = (offer.key, offer.offer.id);
            let who = mickey_config::peer_name_or_address(offer.ip.unwrap_or(from));
            let (into, mut fetched) = match this.fetches.begin(key) {
                Begin::Skip => return,
                Begin::Here(landed) => return this.keeper.write(Content::Files(landed)),
                Begin::Resume(into, fetched) => (into, *fetched),
                Begin::Start => match this.landing(&offer, &who) {
                    Ok(Some(into)) => {
                        this.fetches.running_in(key, into.clone());
                        let mark = crate::fs::mark::Mark::configured(&who);
                        (into, Fetched::marked(mark))
                    }
                    Ok(None) => return this.fetches.forget(key),
                    Err(err) => {
                        this.fetches.forget(key);
                        return eprintln!("mickey: files not fetched: {err}");
                    }
                },
            };
            let before = this.keeper.read();
            let result = this.fetch_shown(&offer, from, &who, &into, &mut fetched);
            match &result {
                // Everything skipped by the other PC: nothing to paste.
                Ok(landed) if landed.is_empty() => {}
                Ok(landed) if this.keeper.read() == before => {
                    this.keeper.write(Content::Files(landed.clone()))
                }
                Ok(_) => println!(
                    "mickey: files arrived in {}, not pasted: something else was copied meanwhile",
                    into.display()
                ),
                Err(err) => eprintln!(
                    "mickey: files not fetched: {err}; offered again, they go on from where they stopped"
                ),
            }
            match result {
                Ok(landed) => this.fetches.done(key, landed),
                // Nothing arrived (out of reach, a firewall, an offer the
                // other PC no longer has): nothing to go on with, and the
                // empty folder goes. Downloads/Mickey itself stays.
                Err(_) if !fetched.started() => {
                    this.fetches.forget(key);
                    if into.starts_with(&this.received) {
                        let _ = std::fs::remove_dir(&into);
                    }
                }
                Err(_) => this.fetches.broke(key, into, fetched),
            }
        });
    }

    /// Fetches `offer` into `into`, going on from what `fetched` says was
    /// done before, and showing how it goes once it takes more than a
    /// moment (`show`); where its top-level files and folders landed.
    fn fetch_shown(
        &self,
        offer: &FileOffer,
        from: IpAddr,
        who: &str,
        into: &Path,
        fetched: &mut Fetched,
    ) -> Result<Vec<PathBuf>, String> {
        // Shown from the start, so one waiting its turn says so.
        let shown = Watcher::start(who.to_string(), fetched.progress.clone());
        let landed = self.fetch_in_turn(offer, from, into, fetched, &shown.stop);
        shown.end(match &landed {
            Ok(_) => Ended::Done {
                files: fetched.summary.files,
                bytes: fetched.summary.bytes,
                folder: into.display().to_string(),
                skipped: fetched.summary.skipped.clone(),
                left_out: offer.offer.left_out.clone(),
            },
            Err(err) => Ended::Failed(err.clone()),
        });
        landed
    }

    /// Waits its turn behind another transfer from the same PC, then
    /// fetches `offer`, unless the owner cancelled meanwhile (`stop`).
    fn fetch_in_turn(
        &self,
        offer: &FileOffer,
        from: IpAddr,
        into: &Path,
        fetched: &mut Fetched,
        stop: &Stop,
    ) -> Result<Vec<PathBuf>, String> {
        let lane = self.lanes.of(&offer.key);
        let _turn = match lane.try_lock() {
            Ok(turn) => turn,
            Err(_) => {
                fetched.progress.lock_anyway().stage = Stage::Queued;
                lane.lock_anyway()
            }
        };
        if stop.cancelled() {
            return Err("cancelled while it waited its turn".into());
        }
        let mut link = self.connect(offer, from)?;
        stop.on(&link);
        self.receive(offer, &mut link, into, fetched)
    }

    /// Where `offer`'s files go: a fresh folder among the received ones; or,
    /// over `bring::ask_over()`, `Downloads/Mickey` if the owner says Bring, and
    /// `None` if they say Ignore.
    fn landing(&self, offer: &FileOffer, who: &str) -> Result<Option<PathBuf>, String> {
        let size = bring::total(&offer.offer);
        if size <= bring::ask_over() {
            let waiting = self.fetches.waiting();
            return crate::clipboard::received::new_folder(&self.received, &waiting).map(Some);
        }
        let folder = crate::fs::downloads::mickey_folder()?;
        if !bring::ask(&offer.offer, who, size, &folder) {
            println!(
                "mickey: {:.1} GB of files offered, not brought",
                size as f64 / 1e9
            );
            return Ok(None);
        }
        // Only Mickey's own leftovers go, never what the owner brought.
        crate::transfer::cleanup::remove_abandoned(&folder, std::time::SystemTime::now());
        Ok(Some(folder))
    }

    /// A transfer link to the PC that made `offer`: only a paired one, and
    /// only if it answers with that PC's key.
    fn connect(&self, offer: &FileOffer, from: IpAddr) -> Result<Link, String> {
        if !self.trusted.lock_anyway().is_trusted(&offer.key) {
            return Err("files offered by a PC that is not paired".into());
        }
        let at = offer.address(from);
        let link = transport::connect_transfer(at, &self.identity).map_err(|err| {
            format!(
                "cannot reach {} on TCP {} (a firewall?): {err}",
                at.ip(),
                transport::transfer_address(at).port()
            )
        })?;
        // Whoever took over that address gets nothing.
        if *link.peer_key() != offer.key {
            return Err(format!("{} answered with another PC's key", at.ip()));
        }
        Ok(link)
    }

    /// Receives `offer` on `link` into the folder `into`; where its
    /// top-level files and folders landed.
    fn receive(
        &self,
        offer: &FileOffer,
        link: &mut Link,
        into: &Path,
        fetched: &mut Fetched,
    ) -> Result<Vec<PathBuf>, String> {
        receive::fetch(link, &offer.offer, into, fetched).map_err(|err| err.to_string())?;
        let summary = &fetched.summary;
        // Never sent back there (#424).
        let landed = summary.landed.iter().map(|path| (path.clone(), offer.key));
        self.came_from.lock_anyway().extend(landed);
        println!(
            "mickey: {} files, {:.1} MB, verified, in {}",
            summary.files,
            summary.bytes as f64 / 1e6,
            into.display()
        );
        Ok(summary.landed.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{Scratch, free_input_port};

    /// A PC called `name`, its files under `scratch`, offering on `port`,
    /// trusting the PC with `key`, if given, and with `copied` in its
    /// clipboard.
    fn pc(
        scratch: &Scratch,
        name: &str,
        port: u16,
        key: Option<[u8; 32]>,
        copied: Option<Content>,
    ) -> Arc<Transfers> {
        let dir = scratch.0.join(name);
        std::fs::create_dir_all(dir.join("trust")).unwrap();
        let mut peers = trust::TrustedPeers::load(&dir.join("trust")).unwrap();
        if let Some(key) = key {
            peers.add(key, "test").unwrap();
        }
        Arc::new(Transfers::new(
            Arc::new(Offers::default()),
            Arc::new(Identity::generate().unwrap()),
            port,
            dir.join("received"),
            Arc::new(std::sync::Mutex::new(peers)),
            Keeper::in_memory(copied),
        ))
    }

    #[test]
    fn copied_files_that_are_gone_are_not_offered() {
        let scratch = Scratch::new("gone");
        let transfers = pc(&scratch, "pc", 1, None, None);
        // Copied, then deleted before the keyboard crossed.
        assert!(
            transfers
                .offer(&[scratch.0.join("deleted folder")])
                .is_err()
        );
    }

    #[test]
    fn files_offered_by_a_pc_not_paired_are_not_fetched() {
        let scratch = Scratch::new("unpaired");
        let b = pc(&scratch, "pc", 1, None, None);
        // Refused before any connection: nothing listens at that address.
        let Err(err) = b.connect(
            &crate::transfer::file_offer::example(None),
            "127.0.0.1".parse().unwrap(),
        ) else {
            panic!("a PC not paired must be refused");
        };
        assert!(err.contains("not paired"), "{err}");
    }

    #[test]
    fn copying_again_while_files_come_cuts_nothing_short() {
        let scratch = Scratch::new("turns");
        let from = scratch.0.join("from");
        std::fs::create_dir_all(&from).unwrap();
        std::fs::write(from.join("x.bin"), vec![1u8; 3_000_000]).unwrap();
        std::fs::write(from.join("y.txt"), b"y").unwrap();
        // PC A serves what it hands out.
        let listen = free_input_port();
        let a = pc(&scratch, "a", listen.port(), None, None);
        let offers = Arc::clone(&a.offers);
        crate::transfer::listen::listen(
            listen,
            Arc::clone(&a.identity),
            |_| true,
            move |mut link| {
                let _ = crate::transfer::send::answer_fetches(&mut link, |id| offers.find(id));
            },
        )
        .unwrap();
        // X is copied and crosses; then Y, before X has arrived.
        let x = a.offer(&[from.join("x.bin")]).unwrap();
        let y = a.offer(&[from.join("y.txt")]).unwrap();
        let b = pc(&scratch, "b", 1, Some(*a.identity.public()), None);
        let fetch = |offer: FileOffer, name: &'static str| {
            let (b, into) = (Arc::clone(&b), scratch.0.join(name));
            std::fs::create_dir_all(&into).unwrap();
            std::thread::spawn(move || {
                b.fetch_shown(&offer, listen.ip(), "A", &into, &mut Fetched::default())
            })
        };
        let (first, second) = (fetch(x, "x"), fetch(y, "y"));
        assert!(first.join().unwrap().is_ok(), "X arrives whole");
        assert!(second.join().unwrap().is_ok(), "and Y after it");
        assert_eq!(
            std::fs::read(scratch.0.join("x/x.bin")).unwrap().len(),
            3_000_000
        );
        assert_eq!(std::fs::read(scratch.0.join("y/y.txt")).unwrap(), b"y");
    }

    #[test]
    fn copied_files_cross_as_an_offer_and_are_pasted_once_verified() {
        use crate::clipboard::wire::{self as clip, Wire};
        use crate::net::secure::Link;
        let scratch = Scratch::new("trigger");
        let fotos = scratch.0.join("from/fotos");
        std::fs::create_dir_all(&fotos).unwrap();
        std::fs::write(fotos.join("a.txt"), b"hello").unwrap();
        // PC A, where the folder is copied: it serves its offers.
        let listen = free_input_port();
        let a_copied = Content::Files(vec![fotos.clone()]);
        let a = pc(&scratch, "a", listen.port(), None, Some(a_copied));
        let offers = Arc::clone(&a.offers);
        crate::transfer::listen::listen(
            listen,
            Arc::clone(&a.identity),
            |_| true,
            move |mut link| {
                let _ = crate::transfer::send::answer_fetches(&mut link, |id| offers.find(id));
            },
        )
        .unwrap();
        // The source asks A's clipboard for an offer, over A's data link.
        let data = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let data_at = data.local_addr().unwrap();
        let a_side = Arc::clone(&a);
        std::thread::spawn(move || {
            let (stream, _) = data.accept().unwrap();
            let link = Link::respond(stream, &a_side.identity).unwrap();
            let read = || a_side.keeper.read();
            let _ = crate::clipboard::sync::answer(
                link,
                read,
                || Ok(()),
                |_| {},
                |_, _| {},
                || true,
                Some(&a_side),
            );
        });
        let source = Identity::generate().unwrap();
        let mut link =
            Link::initiate(std::net::TcpStream::connect(data_at).unwrap(), &source).unwrap();
        link.send_bulk(&clip::encode(&Wire::AskOffer).unwrap())
            .unwrap();
        let Wire::Offer(offer) = clip::decode(&link.recv_bulk().unwrap()).unwrap() else {
            panic!("copied files should come as an offer");
        };
        assert_eq!(
            (offer.ip, offer.port, offer.key),
            (None, listen.port(), *a.identity.public())
        );
        // PC B fetches it from A, and pastes it once it is all there.
        let b_copied = Content::Text("copied before".into());
        let b = pc(&scratch, "b", 1, Some(*a.identity.public()), Some(b_copied));
        b.fetch(offer, listen.ip());
        let pasted = (0..100)
            .find_map(|_| {
                std::thread::sleep(std::time::Duration::from_millis(50));
                match b.keeper.read() {
                    Some(Content::Files(paths)) => Some(paths),
                    _ => None,
                }
            })
            .expect("the files reach the clipboard");
        assert_eq!(pasted.len(), 1);
        assert!(pasted[0].starts_with(scratch.0.join("b/received")));
        assert_eq!(std::fs::read(pasted[0].join("a.txt")).unwrap(), b"hello");
        // Copied again on B, the folder says it came from A, and A never
        // fetches it back (#424): its clipboard keeps what it holds.
        let back = b.offer(&pasted).unwrap();
        assert_eq!(back.origin, Some(*a.identity.public()));
        a.keeper.write(Content::Text("A's own".into()));
        // Refused before any thread starts: nothing to wait for.
        a.fetch(back, listen.ip());
        assert_eq!(a.keeper.read(), Some(Content::Text("A's own".into())));
        assert!(!scratch.0.join("a/received").exists(), "nothing fetched");
    }

    #[test]
    fn a_transfer_waiting_its_turn_says_so_and_can_be_cancelled() {
        let scratch = Scratch::new("queued");
        let a = pc(&scratch, "a", 1, None, None);
        let offer = crate::transfer::file_offer::example(None);
        // Another transfer from the same PC is running.
        let lane = a.lanes.of(&offer.key);
        let turn = lane.lock_anyway();
        // This one broke before, so it is not `Queued` yet.
        let mut fetched = Fetched::default();
        fetched.progress.lock_anyway().stage = Stage::Paused;
        let progress = fetched.progress.clone();
        let stop = Stop::default();
        let waiting = {
            let (a, stop) = (Arc::clone(&a), stop.clone());
            std::thread::spawn(move || {
                let into = std::path::Path::new("/nowhere");
                a.fetch_in_turn(
                    &offer,
                    "127.0.0.1".parse().unwrap(),
                    into,
                    &mut fetched,
                    &stop,
                )
            })
        };
        let queued = (0..200).any(|_| {
            std::thread::sleep(std::time::Duration::from_millis(10));
            progress.lock_anyway().stage == Stage::Queued
        });
        assert!(queued, "the window says it waits its turn");
        stop.cancel();
        drop(turn);
        let err = waiting.join().unwrap().unwrap_err();
        assert!(err.contains("cancelled"), "{err}");
    }

    #[test]
    fn an_offer_from_a_pc_not_paired_makes_no_folder_and_asks_nothing() {
        let scratch = Scratch::new("unpaired-offer");
        let b = pc(&scratch, "b", 1, None, None);
        // Refused before any thread starts: nothing to wait for.
        b.fetch(
            crate::transfer::file_offer::example(None),
            "127.0.0.1".parse().unwrap(),
        );
        assert!(!scratch.0.join("b/received").exists(), "no folder");
        assert!(b.fetches.waiting().is_empty(), "nothing kept");
    }
}
