//! What became of each offer fetched here, and one lane per PC files come
//! from (PRD 0015): an offer that crosses again is fetched once, one that
//! broke goes on from where it stopped (#475, #502), and transfers from
//! one PC take turns.

use crate::threads::LockAnyway;
use crate::transfer::receive::Fetched;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The most broken transfers kept to go on: a PC that started and broke
/// off many (by design or not) cannot keep their folders for ever.
const MAX_BROKEN: usize = 8;

/// Each offer fetched here, by the offering PC's key and the offer's id.
#[derive(Default)]
pub struct Fetches(Mutex<HashMap<Key, Fetch>>);

/// An offer: the offering PC's key and the offer's id.
pub type Key = ([u8; 32], u64);

/// What became of one offer.
enum Fetch {
    /// Being fetched now, or waiting its turn; into this folder once it
    /// has one.
    Running(Option<PathBuf>),
    /// Stopped (the network, a sleeping PC, a full disk, Cancel): what it
    /// had done, and where, to go on; since when.
    Broken {
        into: PathBuf,
        fetched: Box<Fetched>,
        at: Instant,
    },
    /// All there: where its top-level files and folders landed.
    Done(Vec<PathBuf>),
}

/// What to do with an offer that arrives.
pub enum Begin {
    /// Nothing: it is being fetched.
    Skip,
    /// It is all here already, at these paths: pasted again, not fetched
    /// again (copied again on the other PC, or crossing again).
    Here(Vec<PathBuf>),
    /// Go on with it, into the same folder.
    Resume(PathBuf, Box<Fetched>),
    /// Fetch it for the first time.
    Start,
}

impl Fetches {
    /// Says what to do with offer `key`, and marks it as being fetched
    /// unless it is running or here already. Files deleted since they
    /// arrived are fetched again.
    pub fn begin(&self, key: Key) -> Begin {
        let mut fetches = self.0.lock_anyway();
        match fetches.get(&key) {
            Some(Fetch::Running(_)) => return Begin::Skip,
            // All skipped by the other PC, it is fetched again: nothing
            // was pasted, and the files may be steady now.
            Some(Fetch::Done(landed))
                if !landed.is_empty() && landed.iter().all(|path| path.exists()) =>
            {
                return Begin::Here(landed.clone());
            }
            _ => {}
        }
        match fetches.insert(key, Fetch::Running(None)) {
            Some(Fetch::Broken { into, fetched, .. }) => {
                fetches.insert(key, Fetch::Running(Some(into.clone())));
                Begin::Resume(into, fetched)
            }
            _ => Begin::Start,
        }
    }

    /// Records the folder `key`, now running, writes into.
    pub fn running_in(&self, key: Key, into: PathBuf) {
        self.0.lock_anyway().insert(key, Fetch::Running(Some(into)));
    }

    /// Records that `key` is all here, at `landed`.
    pub fn done(&self, key: Key, landed: Vec<PathBuf>) {
        self.0.lock_anyway().insert(key, Fetch::Done(landed));
    }

    /// Records that `key` broke, into `into`, with what it had done.
    /// The oldest beyond `MAX_BROKEN` are forgotten: they would start over.
    pub fn broke(&self, key: Key, into: PathBuf, fetched: Fetched) {
        let mut fetches = self.0.lock_anyway();
        let fetched = Box::new(fetched);
        let at = Instant::now();
        fetches.insert(key, Fetch::Broken { into, fetched, at });
        let mut broken: Vec<(Instant, Key)> = fetches
            .iter()
            .filter_map(|(key, fetch)| match fetch {
                Fetch::Broken { at, .. } => Some((*at, *key)),
                _ => None,
            })
            .collect();
        broken.sort();
        let extra = broken.len().saturating_sub(MAX_BROKEN);
        for (_, key) in broken.into_iter().take(extra) {
            fetches.remove(&key);
        }
    }

    /// The folders of transfers running, waiting their turn, or broken and
    /// waiting to go on: never cleaned away, or they would lose files or
    /// start over.
    pub fn waiting(&self) -> Vec<PathBuf> {
        self.0
            .lock_anyway()
            .values()
            .filter_map(|fetch| match fetch {
                Fetch::Running(into) => into.clone(),
                Fetch::Broken { into, .. } => Some(into.clone()),
                Fetch::Done(_) => None,
            })
            .collect()
    }

    /// Forgets `key`: the owner said Ignore, or it could not start.
    pub fn forget(&self, key: Key) {
        self.0.lock_anyway().remove(&key);
    }
}

/// A lock per PC, made on first use (PRD 0015's queue).
#[derive(Default)]
pub struct Lanes(Mutex<HashMap<[u8; 32], Arc<Mutex<()>>>>);

impl Lanes {
    /// The lane for the PC with `key`.
    pub fn of(&self, key: &[u8; 32]) -> Arc<Mutex<()>> {
        Arc::clone(self.0.lock_anyway().entry(*key).or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_offer_is_fetched_once_and_a_broken_one_goes_on() {
        let fetches = Fetches::default();
        let key = ([5; 32], 42);
        assert!(matches!(fetches.begin(key), Begin::Start));
        assert!(
            matches!(fetches.begin(key), Begin::Skip),
            "offered again while it runs"
        );
        let mut fetched = Fetched::default();
        fetched.summary.files = 7;
        fetches.broke(key, "/somewhere".into(), fetched);
        let Begin::Resume(into, fetched) = fetches.begin(key) else {
            panic!("a broken fetch goes on");
        };
        assert_eq!(
            (into, fetched.summary.files),
            (PathBuf::from("/somewhere"), 7)
        );
        // All here: pasted again, not fetched again; fetched again once
        // the files are gone.
        let here = std::env::temp_dir();
        fetches.done(key, vec![here.clone()]);
        assert!(matches!(fetches.begin(key), Begin::Here(landed) if landed == [here]));
        fetches.done(key, vec![PathBuf::from("/no/such/file/for/mickey")]);
        assert!(matches!(fetches.begin(key), Begin::Start), "deleted since");
        fetches.forget(key);
        assert!(matches!(fetches.begin(key), Begin::Start), "forgotten");
    }

    #[test]
    fn transfers_from_one_pc_take_turns() {
        let lanes = Lanes::default();
        let (a, b) = (lanes.of(&[1; 32]), lanes.of(&[2; 32]));
        let _turn = a.lock_anyway();
        assert!(lanes.of(&[1; 32]).try_lock().is_err(), "the same PC waits");
        assert!(b.try_lock().is_ok(), "another PC goes on");
    }

    #[test]
    fn a_running_transfers_folder_is_never_cleaned_away() {
        let fetches = Fetches::default();
        let key = ([1; 32], 1);
        assert!(matches!(fetches.begin(key), Begin::Start));
        assert!(fetches.waiting().is_empty(), "no folder yet");
        fetches.running_in(key, "/received/1".into());
        assert_eq!(fetches.waiting(), [PathBuf::from("/received/1")]);
        fetches.done(key, vec![PathBuf::from("/received/1/a.txt")]);
        assert!(fetches.waiting().is_empty(), "done: like any other");
    }

    #[test]
    fn only_the_newest_broken_transfers_are_kept() {
        let fetches = Fetches::default();
        for id in 0..MAX_BROKEN as u64 + 3 {
            let key = ([2; 32], id);
            fetches.begin(key);
            fetches.broke(key, format!("/received/{id}").into(), Fetched::default());
        }
        let waiting = fetches.waiting();
        assert_eq!(waiting.len(), MAX_BROKEN);
        assert!(
            !waiting.contains(&PathBuf::from("/received/0")),
            "the oldest goes"
        );
        assert!(waiting.contains(&PathBuf::from(format!("/received/{}", MAX_BROKEN + 2))));
    }

    #[test]
    fn an_offer_all_skipped_is_fetched_again_never_pasted_empty() {
        let fetches = Fetches::default();
        let key = ([3; 32], 1);
        fetches.begin(key);
        fetches.done(key, Vec::new());
        assert!(matches!(fetches.begin(key), Begin::Start));
    }
}
