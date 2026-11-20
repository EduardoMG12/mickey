//! The PC the files go to: fetches every file of an offer into a folder
//! (PRD 0015).
//!
//! Each file is written as `name.part` and gets its real name only once
//! its hash matches the one the other PC sent. Nothing is written past
//! the size the offer announced, and every path was checked when the
//! offer was decoded (`wire`), and is checked again here. The disk never
//! fills: see `room`.
//!
//! A transfer that breaks (the network, a PC asleep, a full disk) resumes
//! where it stopped when fetched again with the same `Fetched` (#475):
//! finished files are not fetched again, and a `.part` this transfer wrote
//! continues from its last byte.

use crate::fs::mark::Mark;
use crate::net::invalid;
use crate::net::secure::Link;
use crate::platform::space;
use crate::threads::LockAnyway;
use crate::transfer::hash::{self, HashWrite};
use crate::transfer::offer::{Kind, Offer};
use crate::transfer::path;
use crate::transfer::progress::{self, Counted, Progress, Stage};
use crate::transfer::room::{self, Guarded};
use crate::transfer::wire::{self, Message, Why};
use std::collections::HashSet;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What a transfer brought, for the owner to see.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// Where each copied file or folder (the offer's top level) landed.
    pub landed: Vec<PathBuf>,
    pub files: usize,
    pub bytes: u64,
    /// Files not brought, by path in the offer, and why.
    pub skipped: Vec<(String, Why)>,
}

/// One offer's transfer so far, kept between attempts so a broken one
/// resumes instead of starting over. It lives as long as Mickey runs.
#[derive(Debug, Default)]
pub struct Fetched {
    /// The name each top-level entry got here, chosen once.
    tops: Vec<(String, PathBuf)>,
    /// Entries done with: folders made, files arrived or skipped.
    finished: HashSet<usize>,
    /// Files with a `.part` this transfer wrote, to continue.
    parts: HashSet<usize>,
    /// Top-level names this transfer started writing under: anything else
    /// that appears at one of its names meanwhile is the owner's.
    claimed: HashSet<PathBuf>,
    pub summary: Summary,
    /// For the window to show while it runs.
    pub progress: progress::Shared,
    /// How arriving files are marked on Windows: from which PC, and
    /// whether as from the Internet.
    pub mark: Mark,
}

impl Fetched {
    /// A transfer not started yet, whose files will carry `mark`.
    pub fn marked(mark: Mark) -> Fetched {
        Fetched {
            mark,
            ..Default::default()
        }
    }
}

/// Fetches every file of `offer` into the folder `into`, or what is left of
/// it if `fetched` comes from an attempt that broke. A copied file or
/// folder whose name is taken there gets `name (2)`, never overwriting.
pub fn fetch(link: &mut Link, offer: &Offer, into: &Path, fetched: &mut Fetched) -> io::Result<()> {
    let result = fetch_all(link, offer, into, fetched);
    fetched.progress.lock_anyway().stage = match result {
        Ok(()) => Stage::Done,
        Err(_) => Stage::Paused,
    };
    result
}

fn fetch_all(link: &mut Link, offer: &Offer, into: &Path, fetched: &mut Fetched) -> io::Result<()> {
    fetched.progress.lock_anyway().stage = Stage::CheckingSpace;
    let places = offer
        .entries
        .iter()
        .map(|entry| place(&entry.path, entry.kind, into, &mut fetched.tops))
        .collect::<io::Result<Vec<(PathBuf, PathBuf)>>>()?;
    // Nothing more is written unless what is left fits, with the margin.
    let (mut need, mut total) = (0u64, 0u64);
    for (index, entry) in offer.entries.iter().enumerate() {
        if let Kind::File { size } = entry.kind {
            total = total.saturating_add(size);
            if !fetched.finished.contains(&index) {
                let kept = kept(fetched, index, &places[index].1, size).unwrap_or(0);
                need = need.saturating_add(size - kept);
            }
        }
    }
    {
        let mut progress = fetched.progress.lock_anyway();
        *progress = Progress::new(total);
        progress.advance(total - need, std::time::Instant::now());
        progress.stage = Stage::CheckingSpace;
    }
    let missing = room::missing(need, space::of(into)?);
    if missing > 0 {
        return Err(room::full(missing));
    }
    fetched.progress.lock_anyway().stage = Stage::Transferring;
    for (index, entry) in offer.entries.iter().enumerate() {
        if fetched.finished.contains(&index) {
            continue;
        }
        let (top, target) = &places[index];
        if !fetched.claimed.contains(top) {
            if top.symlink_metadata().is_ok() {
                // Taken meanwhile (by the owner, or another transfer): the
                // name is picked again next time, never failing for ever.
                fetched.tops.retain(|(_, here)| here != top);
                return Err(taken());
            }
            fetched.claimed.insert(top.clone());
        }
        if let Kind::File { size } = entry.kind {
            let wire_index = u32::try_from(index).map_err(|_| invalid("too many files".into()))?;
            let kept = kept(fetched, index, target, size);
            // Where the bar stood before this file: its bytes may count
            // while they come, and then the other PC skip it after all.
            let before = fetched.progress.lock_anyway().done;
            match fetch_file(link, offer.id, wire_index, size, kept, target, fetched) {
                Ok(None) => {
                    fetched.summary.files += 1;
                    // Sizes come from the other PC: never past u64.
                    fetched.summary.bytes = fetched.summary.bytes.saturating_add(size);
                }
                Ok(Some(why)) => {
                    // Dealt with: what it would have taken counts as done.
                    let mut progress = fetched.progress.lock_anyway();
                    let done = before.saturating_add(size - kept.unwrap_or(0));
                    progress.advance(done, std::time::Instant::now());
                    drop(progress);
                    fetched.summary.skipped.push((entry.path.clone(), why));
                }
                Err(err) => {
                    // Kept for next time unless it was thrown away (damaged,
                    // or not this transfer's).
                    let ours = !matches!(
                        err.kind(),
                        io::ErrorKind::InvalidData | io::ErrorKind::AlreadyExists
                    );
                    if ours && part_of(target).is_file() {
                        fetched.parts.insert(index);
                    } else {
                        fetched.parts.remove(&index);
                    }
                    // A copied file whose name was taken meanwhile gets
                    // another name next time, never failing for ever.
                    if err.kind() == io::ErrorKind::AlreadyExists && target == top {
                        fetched.claimed.remove(top);
                        fetched.tops.retain(|(_, here)| here != top);
                    }
                    return Err(err);
                }
            }
            fetched.parts.remove(&index);
        } else {
            std::fs::create_dir_all(target)?;
        }
        fetched.finished.insert(index);
    }
    // Only what is there: a copied file the other PC skipped never landed.
    fetched.summary.landed = fetched
        .tops
        .iter()
        .map(|(_, here)| here.clone())
        .filter(|here| here.symlink_metadata().is_ok())
        .collect();
    Ok(())
}

/// Where the entry at `path` goes under `into`: its top-level entry's
/// place, and its own. The first entry of each top-level name picks a name
/// for it that is free on disk and not given to another top-level entry
/// already, kept in `tops`.
fn place(
    path: &str,
    kind: Kind,
    into: &Path,
    tops: &mut Vec<(String, PathBuf)>,
) -> io::Result<(PathBuf, PathBuf)> {
    let relative = path::checked(path).map_err(invalid)?;
    let top = path.split('/').next().unwrap_or_default();
    let here = match tops.iter().find(|(name, _)| name == top) {
        Some((_, here)) => here.clone(),
        None => {
            let is_file = top == path && matches!(kind, Kind::File { .. });
            let given: Vec<&Path> = tops.iter().map(|(_, here)| here.as_path()).collect();
            let here = free_name(&into.join(top), is_file, &given);
            tops.push((top.to_string(), here.clone()));
            here
        }
    };
    // The path below the top level, under the name the top level got
    // (joining an empty path would add a trailing slash).
    let below: PathBuf = relative.components().skip(1).collect();
    let target = if below.as_os_str().is_empty() {
        here.clone()
    } else {
        here.join(below)
    };
    Ok((here, target))
}

/// How many bytes of the file at `index` this transfer already has in its
/// own `.part`, if it wrote one (possibly empty); never more than `size`.
fn kept(fetched: &Fetched, index: usize, target: &Path, size: u64) -> Option<u64> {
    if !fetched.parts.contains(&index) {
        return None;
    }
    std::fs::symlink_metadata(part_of(target))
        .ok()
        .filter(|metadata| metadata.is_file() && metadata.len() <= size)
        .map(|metadata| metadata.len())
}

/// Fetches one file into `target`: into a new `.part`, or, if `kept` says
/// this transfer has one with that many bytes, continuing it. `Some(why)`
/// if the other PC skipped it.
fn fetch_file(
    link: &mut Link,
    offer: u64,
    index: u32,
    size: u64,
    kept: Option<u64>,
    target: &Path,
    fetched: &Fetched,
) -> io::Result<Option<Why>> {
    let progress = &*fetched.progress;
    if let Some(folder) = target.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let part = part_of(target);
    let mut hasher = blake3::Hasher::new();
    let (file, from) = match kept {
        // Created new: a `.part` already there (the owner's, or a link) is
        // not this transfer's to write over or to remove.
        None => {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&part)?;
            (file, 0)
        }
        // Opened once, without following a link put in its place, and
        // used for everything. The other PC sends only the rest, but the
        // hash covers the whole file: what is here is read again first.
        Some(from) => {
            let file = crate::fs::open::to_continue(&part)?;
            progress.lock_anyway().stage = Stage::Rereading;
            hash::hash_start(&file, from, &mut hasher, link)?;
            progress.lock_anyway().stage = Stage::Transferring;
            file.set_len(from)?;
            (file, from)
        }
    };
    wire::send(link, &Message::Fetch { offer, index, from })?;
    let folder = part.parent().unwrap_or(&part);
    let result = write_verified(
        link,
        index,
        size - from,
        (file, from),
        hasher,
        folder,
        progress,
    );
    // Thrown away if it was skipped or arrived damaged; kept (to resume)
    // if the network or the disk stopped it.
    if matches!(result, Ok(Some(_)))
        || result
            .as_ref()
            .is_err_and(|err| err.kind() == io::ErrorKind::InvalidData)
    {
        let _ = std::fs::remove_file(&part);
    }
    let why = result?;
    if why.is_some() {
        return Ok(why);
    }
    // Something put at its name meanwhile (by the owner, a browser) is never
    // replaced: renaming would.
    if target.symlink_metadata().is_ok() {
        let _ = std::fs::remove_file(&part);
        return Err(taken());
    }
    fetched.mark.put_on(&part);
    std::fs::rename(&part, target)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&part);
        })
        .map(|()| None)
}

/// Receives the last `rest` bytes of the file at `index` into `file` (at
/// byte `from`) while hashing them after what `hasher` has, counting them
/// in `progress`, and while the disk holding `folder` keeps its margin;
/// then checks the hash the other PC sends after them.
fn write_verified(
    link: &mut Link,
    index: u32,
    rest: u64,
    (mut file, from): (std::fs::File, u64),
    mut hasher: blake3::Hasher,
    folder: &Path,
    progress: &Mutex<Progress>,
) -> io::Result<Option<Why>> {
    match wire::recv(link)? {
        Message::Skip { index: got, why } if got == index => return Ok(Some(why)),
        Message::File { index: got, size } if got == index && size == rest => {}
        _ => return Err(invalid(format!("not the file {index} that was asked for"))),
    }
    io::Seek::seek(&mut file, io::SeekFrom::Start(from))?;
    let mut writer = HashWrite {
        inner: Counted {
            inner: Guarded::new(BufWriter::new(file), || space::of(folder)),
            progress,
        },
        hasher: &mut hasher,
    };
    link.recv_stream(&mut writer, rest)?;
    writer.flush()?;
    match wire::recv(link)? {
        Message::Skip { index: got, why } if got == index => Ok(Some(why)),
        Message::Hash { index: got, hash } if got == index => {
            if hasher.finalize() == blake3::Hash::from_bytes(hash) {
                Ok(None)
            } else {
                Err(invalid(format!("file {index} arrived damaged")))
            }
        }
        _ => Err(invalid(format!("no hash after file {index}"))),
    }
}

/// `path`, or `path (2)`, `path (3)`… for the first that is free: not in
/// `given`, nothing there, and for a file no `.part` beside it either. A
/// file keeps its extension last (`foto (2).jpg`); a folder's dot is part
/// of its name.
fn free_name(path: &Path, is_file: bool, given: &[&Path]) -> PathBuf {
    let (stem, extension) = match (is_file, path.extension()) {
        (true, Some(ext)) => (path.file_stem(), Some(ext.to_string_lossy())),
        _ => (path.file_name(), None),
    };
    let stem = stem.unwrap_or_default().to_string_lossy();
    let taken = |candidate: &Path| {
        given.contains(&candidate)
            || candidate.symlink_metadata().is_ok()
            || (is_file && part_of(candidate).symlink_metadata().is_ok())
    };
    let mut candidate = path.to_path_buf();
    let mut n = 2;
    while taken(&candidate) {
        let name = match &extension {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        candidate.set_file_name(name);
        n += 1;
    }
    candidate
}

/// `target` while it is being received (`path::RECEIVING` added).
fn part_of(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(path::RECEIVING);
    target.with_file_name(name)
}

/// Something appeared at `path` while the transfer waited to write there.
/// No name in it: errors reach the log, where copied names never go.
fn taken() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "something appeared at a file's name while the transfer waited",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::{Scratch, link_pair};
    use crate::transfer::offer::{self, Entry, Listing};
    use crate::transfer::send;
    use std::thread;

    /// A scratch folder with `from/`, where the copied files are, and
    /// `into/`, where they land.
    fn sides(name: &str) -> Scratch {
        let scratch = Scratch::new(name);
        std::fs::create_dir_all(scratch.0.join("from")).unwrap();
        std::fs::create_dir_all(scratch.0.join("into")).unwrap();
        scratch
    }

    /// What `send::answer_fetches` finds for a PC that handed out only
    /// `listing`, as offer `id`.
    fn only(listing: Listing, id: u64) -> impl Fn(u64) -> Option<std::sync::Arc<Listing>> {
        let listing = std::sync::Arc::new(listing);
        move |asked| (asked == id).then(|| std::sync::Arc::clone(&listing))
    }

    /// Serves `listing` on one end and fetches it into `into` on the other.
    fn transfer(listing: Listing, offer: &Offer, into: &Path) -> io::Result<Summary> {
        let (mut fetching, mut serving) = link_pair();
        let id = offer.id;
        let server = thread::spawn(move || send::answer_fetches(&mut serving, only(listing, id)));
        let mut fetched = Fetched::default();
        let result = fetch(&mut fetching, offer, into, &mut fetched);
        drop(fetching);
        server.join().unwrap().unwrap();
        result.map(|()| fetched.summary)
    }

    #[test]
    fn a_tree_arrives_whole_and_verified() {
        let scratch = sides("tree");
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
        scratch.file("from/fotos/praia.jpg", &big);
        scratch.file("from/fotos/2026/a.txt", b"hello");
        scratch.file("from/fotos/empty.txt", b"");
        std::fs::create_dir(scratch.0.join("from/fotos/nothing")).unwrap();
        let listing = offer::list(&[scratch.0.join("from/fotos")]).unwrap();
        let offer = listing.offer(7);
        let summary = transfer(listing, &offer, &scratch.0.join("into")).unwrap();
        let fotos = scratch.0.join("into").join("fotos");
        assert_eq!(std::fs::read(fotos.join("praia.jpg")).unwrap(), big);
        assert_eq!(std::fs::read(fotos.join("2026/a.txt")).unwrap(), b"hello");
        assert!(fotos.join("empty.txt").is_file());
        assert!(fotos.join("nothing").is_dir());
        assert_eq!(summary.landed, [fotos]);
        assert_eq!((summary.files, summary.bytes), (3, big.len() as u64 + 5));
        assert!(summary.skipped.is_empty());
    }

    #[test]
    fn a_name_already_taken_gets_a_number() {
        let scratch = sides("taken");
        scratch.file("from/notes.txt", b"new");
        std::fs::write(scratch.0.join("into").join("notes.txt"), b"old").unwrap();
        let listing = offer::list(&[scratch.0.join("from/notes.txt")]).unwrap();
        let offer = listing.offer(7);
        let summary = transfer(listing, &offer, &scratch.0.join("into")).unwrap();
        assert_eq!(
            std::fs::read(scratch.0.join("into").join("notes.txt")).unwrap(),
            b"old"
        );
        assert_eq!(
            std::fs::read(scratch.0.join("into").join("notes (2).txt")).unwrap(),
            b"new"
        );
        assert_eq!(
            summary.landed,
            [scratch.0.join("into").join("notes (2).txt")]
        );
    }

    #[test]
    fn the_owners_part_file_and_dotted_folders_are_left_alone() {
        let scratch = sides("owners");
        scratch.file("from/notes.txt", b"new");
        scratch.file("from/my.project/a.txt", b"a");
        std::fs::write(
            scratch.0.join("into").join("notes.txt.mickey-part"),
            b"mine",
        )
        .unwrap();
        std::fs::create_dir(scratch.0.join("into").join("my.project")).unwrap();
        let roots = [
            scratch.0.join("from/notes.txt"),
            scratch.0.join("from/my.project"),
        ];
        let listing = offer::list(&roots).unwrap();
        let offer = listing.offer(7);
        let summary = transfer(listing, &offer, &scratch.0.join("into")).unwrap();
        let dest = scratch.0.join("into");
        assert_eq!(
            std::fs::read(dest.join("notes.txt.mickey-part")).unwrap(),
            b"mine"
        );
        assert_eq!(std::fs::read(dest.join("notes (2).txt")).unwrap(), b"new");
        assert_eq!(
            std::fs::read(dest.join("my.project (2)/a.txt")).unwrap(),
            b"a"
        );
        assert_eq!(
            summary.landed,
            [dest.join("notes (2).txt"), dest.join("my.project (2)")]
        );
    }

    #[test]
    fn a_skipped_file_did_not_land() {
        let scratch = sides("skipped");
        scratch.file("from/gone.txt", b"x");
        let listing = offer::list(&[scratch.0.join("from/gone.txt")]).unwrap();
        let offer = listing.offer(7);
        std::fs::remove_file(scratch.0.join("from/gone.txt")).unwrap();
        let summary = transfer(listing, &offer, &scratch.0.join("into")).unwrap();
        assert!(summary.landed.is_empty());
        assert_eq!(summary.skipped, [("gone.txt".to_string(), Why::Gone)]);
    }

    #[test]
    fn a_file_changed_or_deleted_since_the_offer_is_skipped() {
        let scratch = sides("changed");
        scratch.file("from/docs/changes.txt", b"before");
        scratch.file("from/docs/goes.txt", b"here");
        scratch.file("from/docs/stays.txt", b"same");
        let listing = offer::list(&[scratch.0.join("from/docs")]).unwrap();
        let offer = listing.offer(7);
        scratch.file("from/docs/changes.txt", b"after, longer");
        std::fs::remove_file(scratch.0.join("from/docs/goes.txt")).unwrap();
        let summary = transfer(listing, &offer, &scratch.0.join("into")).unwrap();
        let docs = scratch.0.join("into").join("docs");
        assert_eq!(
            summary.skipped,
            [
                ("docs/changes.txt".to_string(), Why::Changed),
                ("docs/goes.txt".to_string(), Why::Gone)
            ]
        );
        assert!(
            !docs.join("changes.txt").exists() && !docs.join("changes.txt.mickey-part").exists()
        );
        assert_eq!(std::fs::read(docs.join("stays.txt")).unwrap(), b"same");
    }

    #[cfg(unix)]
    #[test]
    fn a_file_swapped_for_a_link_after_the_offer_is_not_followed() {
        let scratch = sides("swapped");
        scratch.file("from/secret", b"key");
        scratch.file("from/shared/a.txt", b"ok");
        let listing = offer::list(&[scratch.0.join("from/shared")]).unwrap();
        let offer = listing.offer(7);
        let a = scratch.0.join("from/shared/a.txt");
        std::fs::remove_file(&a).unwrap();
        std::os::unix::fs::symlink(scratch.0.join("from/secret"), &a).unwrap();
        let summary = transfer(listing, &offer, &scratch.0.join("into")).unwrap();
        assert_eq!(summary.skipped, [("shared/a.txt".to_string(), Why::Gone)]);
        assert!(!scratch.0.join("into").join("shared/a.txt").exists());
    }

    #[test]
    fn a_broken_transfer_resumes_where_it_stopped() {
        let scratch = sides("resume");
        let big: Vec<u8> = (0..1_000_000u32).map(|i| (i % 241) as u8).collect();
        scratch.file("from/docs/first.txt", b"first");
        scratch.file("from/docs/big.bin", &big);
        let listing = offer::list(&[scratch.0.join("from/docs")]).unwrap();
        let offer = listing.offer(7);
        let mut fetched = Fetched::default();
        // The first attempt: `big.bin` (index 1) breaks after 300 000
        // bytes, as when the network goes.
        let (mut fetching, mut serving) = link_pair();
        let id = offer.id;
        let first = big[..300_000].to_vec();
        let breaking = thread::spawn(move || {
            serving.recv_bulk().unwrap();
            let file = Message::File {
                index: 1,
                size: 1_000_000,
            };
            serving.send_bulk(&wire::encode(&file)).unwrap();
            serving.send_stream(&first[..], 300_000).unwrap();
        });
        // `big.bin` comes first in name order, `first.txt` after.
        assert_eq!(offer.entries[1].path, "docs/big.bin");
        assert!(fetch(&mut fetching, &offer, &scratch.0.join("into"), &mut fetched).is_err());
        breaking.join().unwrap();
        let part = scratch.0.join("into").join("docs/big.bin.mickey-part");
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 300_000, "kept");
        {
            let progress = fetched.progress.lock_anyway();
            assert_eq!(progress.stage, Stage::Paused);
            assert_eq!((progress.done, progress.total), (300_000, 1_000_005));
        }
        // The second attempt asks only for the rest (starting over would
        // fail: a new `.part` must not exist yet), and checks the whole.
        let (mut fetching, mut serving) = link_pair();
        let server =
            thread::spawn(move || send::answer_fetches(&mut serving, only(listing, id)).unwrap());
        fetch(&mut fetching, &offer, &scratch.0.join("into"), &mut fetched).unwrap();
        drop(fetching);
        server.join().unwrap();
        let docs = scratch.0.join("into").join("docs");
        assert_eq!(std::fs::read(docs.join("big.bin")).unwrap(), big);
        assert_eq!(std::fs::read(docs.join("first.txt")).unwrap(), b"first");
        assert!(!part.exists());
        assert_eq!(fetched.summary.files, 2);
        assert_eq!(fetched.summary.landed, [docs]);
        let progress = fetched.progress.lock_anyway();
        assert_eq!(progress.stage, Stage::Done);
        assert_eq!(progress.done, progress.total);
    }

    #[test]
    fn two_copied_files_never_get_the_same_name() {
        let scratch = sides("same-name");
        scratch.file("from/photo (2).jpg", b"two");
        scratch.file("from/photo.jpg", b"one");
        std::fs::write(scratch.0.join("into").join("photo.jpg"), b"owner's").unwrap();
        let roots = [
            scratch.0.join("from/photo (2).jpg"),
            scratch.0.join("from/photo.jpg"),
        ];
        let listing = offer::list(&roots).unwrap();
        let offer = listing.offer(7);
        transfer(listing, &offer, &scratch.0.join("into")).unwrap();
        let dest = scratch.0.join("into");
        assert_eq!(std::fs::read(dest.join("photo.jpg")).unwrap(), b"owner's");
        assert_eq!(std::fs::read(dest.join("photo (2).jpg")).unwrap(), b"two");
        assert_eq!(std::fs::read(dest.join("photo (3).jpg")).unwrap(), b"one");
    }

    /// A first attempt at `offer` whose server reads the fetch and hangs up
    /// at once, before any byte.
    fn break_at_once(offer: &Offer, into: &Path, fetched: &mut Fetched) {
        let (mut fetching, mut serving) = link_pair();
        let breaking = thread::spawn(move || {
            serving.recv_bulk().unwrap();
        });
        assert!(fetch(&mut fetching, offer, into, fetched).is_err());
        breaking.join().unwrap();
    }

    #[test]
    fn a_transfer_broken_before_its_first_byte_resumes() {
        let scratch = sides("first-byte");
        scratch.file("from/a.txt", b"hello");
        let listing = offer::list(&[scratch.0.join("from/a.txt")]).unwrap();
        let offer = listing.offer(7);
        let mut fetched = Fetched::default();
        break_at_once(&offer, &scratch.0.join("into"), &mut fetched);
        assert!(
            scratch.0.join("into").join("a.txt.mickey-part").is_file(),
            "an empty .part"
        );
        let (mut fetching, mut serving) = link_pair();
        let id = offer.id;
        let server =
            thread::spawn(move || send::answer_fetches(&mut serving, only(listing, id)).unwrap());
        fetch(&mut fetching, &offer, &scratch.0.join("into"), &mut fetched).unwrap();
        drop(fetching);
        server.join().unwrap();
        assert_eq!(
            std::fs::read(scratch.0.join("into").join("a.txt")).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn what_the_owner_puts_at_a_name_while_waiting_is_kept() {
        let scratch = sides("owner-meanwhile");
        scratch.file("from/a.txt", b"from the other PC");
        let listing = offer::list(&[scratch.0.join("from/a.txt")]).unwrap();
        let offer = listing.offer(7);
        let mut fetched = Fetched::default();
        break_at_once(&offer, &scratch.0.join("into"), &mut fetched);
        std::fs::write(scratch.0.join("into").join("a.txt"), b"the owner's").unwrap();
        let (mut fetching, mut serving) = link_pair();
        let id = offer.id;
        let server =
            thread::spawn(move || send::answer_fetches(&mut serving, only(listing, id)).unwrap());
        let err = fetch(&mut fetching, &offer, &scratch.0.join("into"), &mut fetched).unwrap_err();
        drop(fetching);
        server.join().unwrap();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read(scratch.0.join("into").join("a.txt")).unwrap(),
            b"the owner's"
        );
        assert!(!scratch.0.join("into").join("a.txt.mickey-part").exists());
        // Next time it picks another name, and arrives: never stuck.
        let listing = offer::list(&[scratch.0.join("from/a.txt")]).unwrap();
        let (mut fetching, mut serving) = link_pair();
        let server =
            thread::spawn(move || send::answer_fetches(&mut serving, only(listing, id)).unwrap());
        fetch(&mut fetching, &offer, &scratch.0.join("into"), &mut fetched).unwrap();
        drop(fetching);
        server.join().unwrap();
        assert_eq!(
            std::fs::read(scratch.0.join("into").join("a (2).txt")).unwrap(),
            b"from the other PC"
        );
        assert_eq!(
            std::fs::read(scratch.0.join("into").join("a.txt")).unwrap(),
            b"the owner's"
        );
    }

    #[test]
    fn bytes_of_a_file_skipped_after_all_count_once() {
        let scratch = sides("skipped-late");
        let (mut fetching, mut serving) = link_pair();
        let file = |path: &str| Entry {
            path: path.into(),
            kind: Kind::File { size: 4 },
        };
        let offer = Offer {
            id: 1,
            entries: vec![file("a.bin"), file("b.bin")],
            left_out: Vec::new(),
        };
        // Sends a.bin's bytes, then says it changed meanwhile, then hangs
        // up before b.bin.
        let sender = thread::spawn(move || {
            serving.recv_bulk().unwrap();
            let send = |link: &mut Link, message| link.send_bulk(&wire::encode(&message)).unwrap();
            send(&mut serving, Message::File { index: 0, size: 4 });
            serving.send_stream(&b"abcd"[..], 4).unwrap();
            send(
                &mut serving,
                Message::Skip {
                    index: 0,
                    why: Why::Changed,
                },
            );
        });
        let mut fetched = Fetched::default();
        assert!(fetch(&mut fetching, &offer, &scratch.0.join("into"), &mut fetched).is_err());
        sender.join().unwrap();
        assert_eq!(
            fetched.progress.lock_anyway().done,
            4,
            "a.bin once, not twice"
        );
    }

    #[test]
    fn an_offer_that_does_not_fit_writes_nothing() {
        let scratch = sides("too-big");
        let (mut fetching, _serving) = link_pair();
        let offer = Offer {
            id: 1,
            entries: vec![Entry {
                path: "huge.bin".into(),
                kind: Kind::File { size: 1 << 50 },
            }],
            left_out: Vec::new(),
        };
        let err = fetch(
            &mut fetching,
            &offer,
            &scratch.0.join("into"),
            &mut Fetched::default(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::StorageFull);
        assert_eq!(
            std::fs::read_dir(scratch.0.join("into")).unwrap().count(),
            0
        );
    }

    #[test]
    fn a_sender_that_lies_about_the_size_leaves_nothing() {
        let scratch = sides("lies");
        let (mut fetching, mut serving) = link_pair();
        let offer = Offer {
            id: 1,
            entries: vec![Entry {
                path: "x.bin".into(),
                kind: Kind::File { size: 4 },
            }],
            left_out: Vec::new(),
        };
        let liar = thread::spawn(move || {
            serving.recv_bulk().unwrap();
            let file = Message::File {
                index: 0,
                size: 1 << 30,
            };
            serving.send_bulk(&wire::encode(&file)).unwrap();
        });
        assert!(
            fetch(
                &mut fetching,
                &offer,
                &scratch.0.join("into"),
                &mut Fetched::default()
            )
            .is_err()
        );
        liar.join().unwrap();
        assert_eq!(
            std::fs::read_dir(scratch.0.join("into")).unwrap().count(),
            0
        );
    }
}
