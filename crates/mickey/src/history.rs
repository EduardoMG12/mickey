//! Mickey's clipboard history (PRD 0011): the recent copies on this PC,
//! including what arrived from another PC. In memory only, never written
//! to disk: a copy may be a password.

use crate::clipboard::Content;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

const MAX_ENTRIES: usize = 50;
/// What all entries may hold together. Images count as PNG (#190). The
/// vision's budget is 20 MB resident per PC, and a running Mickey uses
/// about 11 MB before any history (10.7 MB measured on the laptop, 0.3.0):
/// 16 MiB of history would have gone over it.
const MAX_BYTES: usize = 8 << 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Stable while the entry lives, so a window can pick it.
    pub id: u64,
    pub at: SystemTime,
    kept: Kept,
}

impl Entry {
    /// The copy as the clipboard takes it (an image decoded again).
    pub fn content(&self) -> Option<Content> {
        match &self.kept {
            Kept::Plain(content) => Some(content.clone()),
            Kept::Png { wire, .. } => match crate::clip_wire::decode(wire) {
                Ok(crate::clip_wire::Wire::Content(content)) => Some(content),
                _ => None,
            },
        }
    }
}

/// How a copy is kept: an image as PNG, which is several times smaller
/// than its pixels (a 4K shot is 33 MB of pixels); the rest as it is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kept {
    Plain(Content),
    /// The image as the data link carries it (PNG behind a kind byte), so
    /// the same code encodes and decodes it.
    Png {
        width: usize,
        height: usize,
        wire: Vec<u8>,
    },
}

impl Kept {
    fn new(content: Content) -> Kept {
        if let Content::Image { width, height, .. } = content {
            let wire = crate::clip_wire::encode(&crate::clip_wire::Wire::Content(content.clone()));
            if let Ok(wire) = wire {
                return Kept::Png {
                    width,
                    height,
                    wire,
                };
            }
        }
        Kept::Plain(content)
    }

    fn size(&self) -> usize {
        match self {
            Kept::Plain(Content::Text(text)) => text.len(),
            Kept::Plain(Content::Image { rgba, .. }) => rgba.len(),
            Kept::Plain(Content::Files(paths)) => {
                paths.iter().map(|path| path.as_os_str().len()).sum()
            }
            Kept::Png { wire, .. } => wire.len(),
        }
    }
}

#[derive(Default)]
pub struct History {
    /// Newest first.
    entries: VecDeque<Entry>,
    next_id: u64,
    bytes: usize,
}

impl History {
    /// Keeps `content` as the newest copy. Copying again what is already
    /// the newest only refreshes it; copying an older entry moves it up.
    pub fn add(&mut self, content: Content, at: SystemTime) {
        let kept = Kept::new(content);
        if let Some(index) = self.entries.iter().position(|entry| entry.kept == kept) {
            let entry = self.entries.remove(index).expect("the index was found");
            self.bytes -= entry.kept.size();
        }
        self.bytes += kept.size();
        self.next_id += 1;
        self.entries.push_front(Entry {
            id: self.next_id,
            at,
            kept,
        });
        // The newest always stays, even if it alone is over the bound.
        while self.entries.len() > 1 && (self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES)
        {
            if let Some(old) = self.entries.pop_back() {
                self.bytes -= old.kept.size();
            }
        }
    }

    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter()
    }

    pub fn get(&self, id: u64) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.id == id)
    }
}

/// The history shared by the recorder and whoever asks.
pub type Shared = Arc<Mutex<History>>;

/// Keeps every copy made on this PC, as `clip_watch` notices them, until
/// Mickey ends. Returns the history to ask.
pub fn record(keeper: crate::clipboard::Keeper) -> Shared {
    let history = Shared::default();
    let (changed, changes) = std::sync::mpsc::channel();
    let text_keeper = keeper.clone();
    let how = crate::clip_watch::start(changed, move || text_keeper.read_text());
    println!("mickey: clipboard history through {how}");
    let kept = Arc::clone(&history);
    crate::threads::spawn("history", move || {
        for () in changes {
            if let Some(content) = keeper.read() {
                kept.lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .add(content, SystemTime::now());
            }
        }
    });
    history
}

/// One line per entry for `mickey history` and the window: id, kind,
/// seconds ago and a one-line preview, tab-separated.
pub fn lines(history: &History, now: SystemTime) -> String {
    let mut out = Vec::new();
    for entry in history.entries() {
        let ago = now.duration_since(entry.at).map_or(0, |ago| ago.as_secs());
        let (kind, preview) = match &entry.kept {
            Kept::Png { width, height, .. } => ("image", format!("{width}x{height}")),
            Kept::Plain(Content::Text(text)) => {
                let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
                ("text", one_line.chars().take(120).collect())
            }
            Kept::Plain(Content::Image { width, height, .. }) => {
                ("image", format!("{width}x{height}"))
            }
            Kept::Plain(Content::Files(paths)) => {
                let names: Vec<String> = paths
                    .iter()
                    .filter_map(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .collect();
                ("files", names.join(", ").chars().take(120).collect())
            }
        };
        out.push(format!("{}\t{kind}\t{ago}\t{preview}", entry.id));
    }
    out.join("\n")
}

/// Another clipboard manager keeping a history on this PC, by name, if
/// one runs: then Mickey leaves the history to it (#257). Read once, when
/// Mickey starts.
pub fn another_manager() -> Option<String> {
    imp::another_manager()
}

/// Programs that keep a clipboard history, by a word of their command line
/// (`wl-paste --watch cliphist store` names cliphist).
#[cfg(target_os = "linux")]
const MANAGERS: [&str; 10] = [
    "cliphist",
    "copyq",
    "gpaste-daemon",
    "clipman",
    "xfce4-clipman",
    "diodon",
    "parcellite",
    "clipit",
    "greenclip",
    "clipmenud",
];

#[cfg(target_os = "linux")]
mod imp {
    /// One of `MANAGERS` among the running programs, or Klipper, which
    /// Plasma runs inside its shell, on by default.
    pub fn another_manager() -> Option<String> {
        let running = std::fs::read_dir("/proc")
            .ok()?
            .flatten()
            .filter_map(|entry| {
                let line = std::fs::read(entry.path().join("cmdline")).ok()?;
                Some(String::from_utf8_lossy(&line).replace('\0', " "))
            });
        let words: Vec<String> = running.collect();
        if let Some(found) = super::MANAGERS
            .iter()
            .find(|name| words.iter().any(|line| mentions(line, name)))
        {
            return Some(found.to_string());
        }
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        desktop
            .split(':')
            .any(|part| part == "KDE")
            .then(|| "Klipper".to_string())
    }

    /// Whether `name` is one of the words of `line`, or the file name of
    /// one (`/usr/bin/copyq`), not part of a longer word.
    pub fn mentions(line: &str, name: &str) -> bool {
        line.split_whitespace()
            .any(|word| word.rsplit('/').next() == Some(name))
    }
}

#[cfg(windows)]
mod imp {
    /// Windows' own history (Win+V), when it is switched on.
    pub fn another_manager() -> Option<String> {
        crate::platform::windows::clipboard_history_on()
            .then(|| "Windows' clipboard history (Win+V)".to_string())
    }
}

#[cfg(test)]
mod tests {
    /// Run by hand: says which clipboard manager this PC runs, if any.
    #[test]
    #[ignore = "depends on the programs running here"]
    fn which_manager_runs_here() {
        println!("manager: {:?}", super::another_manager());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_manager_is_found_by_a_whole_word_of_its_command_line() {
        use super::imp::mentions;
        assert!(mentions("wl-paste --watch cliphist store", "cliphist"));
        assert!(mentions("/usr/bin/copyq --start-server", "copyq"));
        assert!(!mentions("vim notes-about-copyq.txt", "copyq"));
        assert!(!mentions("mickey run", "cliphist"));
    }

    use super::*;

    fn text(text: &str) -> Content {
        Content::Text(text.into())
    }

    #[test]
    fn images_are_kept_as_png_and_come_back_whole() {
        // A 1920x1080 screen of flat colour bands, like a desktop shot.
        let (width, height) = (1920, 1080);
        let rgba: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                let band = (i / width / 60) as u8;
                [band.wrapping_mul(13), 200 - band, band.wrapping_mul(7), 255]
            })
            .collect();
        let image = Content::Image {
            width,
            height,
            rgba,
        };
        let mut history = History::default();
        history.add(image.clone(), SystemTime::now());
        let entry = history.entries().next().unwrap();
        eprintln!(
            "pixels: {} bytes, kept: {} bytes",
            width * height * 4,
            entry.kept.size()
        );
        assert!(
            entry.kept.size() * 10 < width * height * 4,
            "at least 10 times smaller"
        );
        assert_eq!(entry.content(), Some(image.clone()));
        // Copying the same image again does not keep a second one.
        history.add(image, SystemTime::now());
        assert_eq!(history.entries().count(), 1);
    }

    #[test]
    fn newest_first_and_no_duplicates() {
        let mut history = History::default();
        let now = SystemTime::now();
        history.add(text("a"), now);
        history.add(text("b"), now);
        history.add(text("a"), now);
        let order: Vec<_> = history
            .entries()
            .map(|entry| entry.content().unwrap())
            .collect();
        assert_eq!(order, [text("a"), text("b")]);
        let newest = history.entries().next().unwrap().id;
        assert_eq!(history.get(newest).unwrap().content(), Some(text("a")));
    }

    #[test]
    fn lines_are_one_per_entry_on_one_line() {
        let mut history = History::default();
        let then = SystemTime::UNIX_EPOCH;
        history.add(text("two\nlines\there"), then);
        let line = lines(&history, then + std::time::Duration::from_secs(9));
        assert_eq!(line, "1\ttext\t9\ttwo lines here");
    }

    #[test]
    fn the_oldest_go_first_when_full() {
        let mut history = History::default();
        let now = SystemTime::now();
        for i in 0..MAX_ENTRIES + 5 {
            history.add(text(&i.to_string()), now);
        }
        assert_eq!(history.entries().count(), MAX_ENTRIES);
        assert_eq!(history.entries().last().unwrap().content(), Some(text("5")));
        // One image bigger than the bound still stays, alone.
        let huge = Content::Image {
            width: 1,
            height: 1,
            rgba: vec![0; MAX_BYTES + 4],
        };
        history.add(huge.clone(), now);
        assert_eq!(history.entries().count(), 1);
    }
}
