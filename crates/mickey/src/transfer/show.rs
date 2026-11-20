//! Showing a transfer while it runs (PRD 0015, #477): a window in the
//! corner (`mickey-ui progress`) once it has taken more than a moment,
//! told how it goes ten times a second, and told how it ended. Its Cancel
//! closes the transfer's connection; what arrived stays, to resume. A
//! transfer waiting its turn behind another from the same PC shows too.

use crate::net::secure::Link;
use crate::threads::LockAnyway;
use crate::transfer::offer::{self, Skipped};
use crate::transfer::progress::{self, Progress, Stage};
use crate::transfer::wire::Why;
use std::io::{BufRead, Write};
use std::process::{ChildStdin, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A transfer shorter than this shows no window.
const QUIET_FOR: Duration = Duration::from_secs(2);
/// How often the window hears how it goes.
const EVERY: Duration = Duration::from_millis(100);

/// How a transfer ended.
pub enum Ended {
    Done {
        files: usize,
        bytes: u64,
        folder: String,
        /// Files the other PC did not send (changed or gone there), by
        /// name: shown in the window only, never logged.
        skipped: Vec<(String, Why)>,
        /// What was copied there but never offered (links, names Windows
        /// refuses...), as the offer says.
        left_out: Vec<Skipped>,
    },
    Failed(String),
}

/// What Cancel stops: the transfer's connection once it has one. Until
/// then (it waits its turn), it remembers the owner cancelled.
#[derive(Clone, Default)]
pub struct Stop(Arc<Mutex<Stopping>>);

#[derive(Default)]
struct Stopping {
    link: Option<Link>,
    cancelled: bool,
}

impl Stop {
    /// The transfer is on `link` now: closed at once if already cancelled.
    pub fn on(&self, link: &Link) {
        let mut stopping = self.0.lock_anyway();
        if let Ok(link) = link.try_clone() {
            if stopping.cancelled {
                let _ = link.shutdown();
            }
            stopping.link = Some(link);
        }
    }

    /// Whether the owner cancelled.
    pub fn cancelled(&self) -> bool {
        self.0.lock_anyway().cancelled
    }

    /// What the window's Cancel does.
    pub fn cancel(&self) {
        let mut stopping = self.0.lock_anyway();
        stopping.cancelled = true;
        if let Some(link) = &stopping.link {
            let _ = link.shutdown();
        }
    }
}

/// Watches one transfer; `end` says how it ended.
pub struct Watcher {
    ended: Sender<Ended>,
    pub stop: Stop,
}

impl Watcher {
    /// Starts watching `progress`, a transfer of files from `who`. Its
    /// `stop` is what the window's Cancel stops.
    pub fn start(who: String, progress: progress::Shared) -> Watcher {
        let (ended, end) = mpsc::channel();
        let stop = Stop::default();
        let window_stop = stop.clone();
        crate::threads::spawn("transfer shown", move || {
            let stop = window_stop;
            let started = Instant::now();
            let mut window: Option<ChildStdin> = None;
            // The owner closed it: not opened again while it runs. How it
            // ends, if it failed or files did not come, still shows.
            let mut dismissed = false;
            loop {
                match end.recv_timeout(EVERY) {
                    Ok(how) => {
                        // A failure, or files that did not come, always
                        // show, however soon it ended.
                        if window.is_none() && worth_showing(&how) {
                            window = open(&who, Some(stop.clone()));
                        }
                        if let Some(mut input) = window {
                            let _ = writeln!(input, "{}", ended_line(&how));
                        }
                        return;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                if window.is_none() && !dismissed && started.elapsed() >= QUIET_FOR {
                    window = open(&who, Some(stop.clone()));
                }
                if let Some(input) = &mut window {
                    let line = progress_line(&progress.lock_anyway());
                    // The owner closed the window: the transfer goes on.
                    if writeln!(input, "{line}").is_err() {
                        window = None;
                        dismissed = true;
                    }
                }
            }
        });
        Watcher { ended, stop }
    }

    pub fn end(self, how: Ended) {
        let _ = self.ended.send(how);
    }
}

/// Says a transfer from `who` could not even start (the other PC out of
/// reach): the window, with why, and nothing to cancel.
pub fn failed(who: &str, why: &str) {
    if let Some(mut input) = open(who, None) {
        let _ = writeln!(input, "{}", ended_line(&Ended::Failed(why.to_string())));
    }
}

/// Opens the window, and closes `stop` when it says the owner cancelled;
/// its input, to tell it how the transfer goes.
fn open(who: &str, stop: Option<Stop>) -> Option<ChildStdin> {
    let mut child = crate::window::ui_command(&["progress", who])
        .and_then(|mut ui| ui.stdin(Stdio::piped()).stdout(Stdio::piped()).spawn())
        .inspect_err(|err| eprintln!("mickey: cannot show the transfer: {err}"))
        .ok()?;
    let (input, stdout) = (child.stdin.take()?, child.stdout.take()?);
    crate::threads::spawn("transfer cancel", move || {
        let cancelled = std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .any(|line| line == "cancel");
        if cancelled && let Some(stop) = stop {
            stop.cancel();
        }
        let _ = child.wait();
    });
    Some(input)
}

/// `<stage>\t<done>\t<total>\t<bytes a second>\t<seconds left>`, the last
/// two empty while not known.
fn progress_line(progress: &Progress) -> String {
    let stage = match progress.stage {
        Stage::Queued => "queued",
        Stage::CheckingSpace => "space",
        Stage::Rereading => "rereading",
        Stage::Paused => "paused",
        Stage::Transferring | Stage::Done => "transferring",
    };
    let speed = progress
        .speed()
        .map_or_else(String::new, |speed| format!("{speed:.0}"));
    let left = progress
        .time_left()
        .map_or_else(String::new, |left| left.as_secs().to_string());
    format!(
        "{stage}\t{}\t{}\t{speed}\t{left}",
        progress.done, progress.total
    )
}

/// Whether how a transfer ended shows even if it was quick: it failed, or
/// some files did not come.
fn worth_showing(how: &Ended) -> bool {
    match how {
        Ended::Failed(_) => true,
        Ended::Done {
            skipped, left_out, ..
        } => !skipped.is_empty() || !left_out.is_empty(),
    }
}

/// `done\t<files>\t<bytes>\t<folder>`, after one `skipped\t<why>\t<name>`
/// line per file that did not come; or `failed\t<why>`. A left-out name
/// comes from the other PC unchecked: it is cleaned to be shown.
fn ended_line(how: &Ended) -> String {
    let flat = |text: &str| text.replace(['\t', '\n', '\r'], " ");
    match how {
        Ended::Done {
            files,
            bytes,
            folder,
            skipped,
            left_out,
        } => {
            let sent = skipped.iter().map(|(name, why)| {
                let why = match why {
                    Why::Changed => "changed",
                    Why::Gone => "gone",
                };
                (why, flat(name))
            });
            let never_offered = left_out.iter().map(|skipped| {
                let why = match skipped.why {
                    offer::Why::Link => "link",
                    offer::Why::BadName => "name",
                    offer::Why::SameNameOtherCase => "case",
                    offer::Why::NotAFile => "not-a-file",
                    offer::Why::Unreadable => "unreadable",
                };
                (why, crate::fs::names::shown(&skipped.path))
            });
            let mut lines: Vec<String> = sent
                .chain(never_offered)
                .map(|(why, name)| format!("skipped\t{why}\t{name}"))
                .collect();
            lines.push(format!("done\t{files}\t{bytes}\t{}", flat(folder)));
            lines.join("\n")
        }
        Ended::Failed(why) => format!("failed\t{}", flat(why)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_hears_the_stage_bytes_speed_and_time_left() {
        let start = Instant::now();
        let mut progress = Progress::new(1000);
        progress.stage = Stage::Transferring;
        progress.advance(0, start);
        assert_eq!(progress_line(&progress), "transferring\t0\t1000\t\t");
        progress.advance(100, start + Duration::from_secs(2));
        assert_eq!(progress_line(&progress), "transferring\t100\t1000\t50\t18");
    }

    #[test]
    fn how_it_ended_is_one_line() {
        let done = Ended::Done {
            files: 3,
            bytes: 42,
            folder: "/home/ana/Downloads/Mickey".into(),
            skipped: vec![],
            left_out: vec![],
        };
        assert!(
            !worth_showing(&done),
            "a quick transfer that brought everything shows nothing"
        );
        assert_eq!(ended_line(&done), "done\t3\t42\t/home/ana/Downloads/Mickey");
        let some_skipped = Ended::Done {
            files: 1,
            bytes: 5,
            folder: "/d".into(),
            skipped: vec![
                ("fotos/a.jpg".into(), Why::Changed),
                ("b.txt".into(), Why::Gone),
            ],
            left_out: vec![Skipped {
                path: "atalho\u{202E}gpj.exe".into(),
                why: offer::Why::Link,
            }],
        };
        assert_eq!(
            ended_line(&some_skipped),
            "skipped\tchanged\tfotos/a.jpg\nskipped\tgone\tb.txt\n\
             skipped\tlink\tatalho?gpj.exe\ndone\t1\t5\t/d"
        );
        assert!(worth_showing(&some_skipped), "shown even if quick");
        let failed = Ended::Failed("cannot reach\n192.0.2.7".into());
        assert_eq!(ended_line(&failed), "failed\tcannot reach 192.0.2.7");
    }

    #[test]
    fn cancelled_before_the_link_exists_closes_it_once_it_does() {
        let (ours, mut theirs) = crate::test_util::link_pair();
        let stop = Stop::default();
        stop.cancel();
        assert!(stop.cancelled());
        stop.on(&ours);
        assert!(theirs.recv_bulk().is_err(), "the link is closed");
    }
}
