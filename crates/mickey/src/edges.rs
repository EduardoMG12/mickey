//! Screen edges (PRD 0005). Each peer's desktop sits at `position` on this
//! one: its point `p` is this desktop's `p + position`. The cursor crosses
//! where it would step off one desktop onto the other.

use crate::monitors::Monitor;

pub fn on_screen(monitors: &[Monitor], (x, y): (i32, i32)) -> bool {
    monitors
        .iter()
        .any(|m| (m.x..m.x + m.width).contains(&x) && (m.y..m.y + m.height).contains(&y))
}

/// The pixel the cursor at `at` would step onto with a motion of `delta`,
/// if that pixel is off every monitor of `here`: the cursor is pushing
/// Asking where the cursor is costs a round trip to the compositor on
/// Linux (Hyprland's socket), so motion is summed and the edge checked at
/// most once per `EDGE_CHECK` instead of on every mouse report (#187): at
/// 1000 Hz, 125 questions a second instead of 1000, a crossing at most
/// 8 ms later.
pub const EDGE_CHECK: std::time::Duration = std::time::Duration::from_millis(8);

/// Asks where the cursor is on a thread of its own, so the thread that
/// injects the mouse never waits for the compositor (#187): one answer took
/// 2.7 ms at p99 on Hyprland, and every event behind it waited too. Motion
/// goes in with `push`; whether it pushed off this desktop comes back, a
/// moment later, from `answer`. The thread ends with this.
pub struct EdgeWatch {
    pushed: std::sync::mpsc::Sender<(i32, i32)>,
    answers: std::sync::mpsc::Receiver<Option<(i32, i32)>>,
}

impl EdgeWatch {
    pub fn start(monitors: Vec<Monitor>, cursor: fn() -> Option<(i32, i32)>) -> EdgeWatch {
        let (pushed, motion) = std::sync::mpsc::channel::<(i32, i32)>();
        let (answer, answers) = std::sync::mpsc::channel();
        crate::threads::spawn("edge check", move || {
            while let Ok(mut delta) = motion.recv() {
                // Motion that came while the last question was out joins in.
                for (dx, dy) in motion.try_iter() {
                    delta = (delta.0.saturating_add(dx), delta.1.saturating_add(dy));
                }
                let off = cursor().and_then(|at| beyond(&monitors, at, delta));
                if answer.send(off).is_err() {
                    break;
                }
            }
        });
        EdgeWatch { pushed, answers }
    }

    /// Hands motion over, without waiting.
    pub fn push(&self, delta: (i32, i32)) {
        let _ = self.pushed.send(delta);
    }

    /// The newest answer, if one came since the last call: the pixel off
    /// this desktop the cursor is pushing onto, or `Some(None)` when it is
    /// not pushing off.
    pub fn answer(&self) -> Option<Option<(i32, i32)>> {
        self.answers.try_iter().last()
    }
}

/// Motion summed between two edge checks.
#[derive(Debug)]
pub struct Throttle {
    checked: std::time::Instant,
    sum: (i32, i32),
}

impl Throttle {
    pub fn new() -> Throttle {
        Throttle {
            checked: std::time::Instant::now(),
            sum: (0, 0),
        }
    }

    /// Adds a move; returns the motion summed since the last check when it
    /// is time to check again (and there was motion), then starts over.
    pub fn feed(&mut self, delta: (i32, i32)) -> Option<(i32, i32)> {
        self.feed_at(delta, std::time::Instant::now())
    }

    fn feed_at(&mut self, (dx, dy): (i32, i32), now: std::time::Instant) -> Option<(i32, i32)> {
        self.sum = (self.sum.0.saturating_add(dx), self.sum.1.saturating_add(dy));
        if now.duration_since(self.checked) < EDGE_CHECK || self.sum == (0, 0) {
            return None;
        }
        self.checked = now;
        Some(std::mem::take(&mut self.sum))
    }
}

/// against the edge of its desktop there.
pub fn beyond(here: &[Monitor], at: (i32, i32), (dx, dy): (i32, i32)) -> Option<(i32, i32)> {
    if !on_screen(here, at) {
        return None;
    }
    // One axis at a time, so pushing along an edge (down a left edge, say)
    // is not mistaken for leaving through it.
    let steps = [(dx.signum(), 0), (0, dy.signum())];
    steps
        .into_iter()
        .filter(|&step| step != (0, 0))
        .map(|(sx, sy)| (at.0 + sx, at.1 + sy))
        .find(|&next| !on_screen(here, next))
}

/// `point` of this desktop as a point of the peer at `position`, if one
/// of the peer's monitors is there.
pub fn onto(peer: &[Monitor], [x, y]: [i32; 2], point: (i32, i32)) -> Option<(i32, i32)> {
    let there = (point.0 - x, point.1 - y);
    on_screen(peer, there).then_some(there)
}

/// `point` of the peer at `position` as a point of this desktop, if one of
/// this PC's monitors is there.
pub fn back(home: &[Monitor], [x, y]: [i32; 2], point: (i32, i32)) -> Option<(i32, i32)> {
    let here = (point.0 + x, point.1 + y);
    on_screen(home, here).then_some(here)
}

#[cfg(test)]
mod tests {
    /// #187's measurement, on a real Hyprland: how long the thread that
    /// injects the mouse waits per event at 1000 Hz, when the edge check
    /// runs on its own thread. `cargo test --release edge_watch_cost --
    /// --ignored --nocapture`, in a Hyprland session.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a Hyprland session"]
    fn edge_watch_cost() {
        let screen = [Monitor {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        }];
        let cursor = || crate::monitors::hyprland::cursor().ok();
        let watch = EdgeWatch::start(screen.to_vec(), cursor);
        let mut throttle = Throttle::new();
        let mut waits = Vec::new();
        for _ in 0..8000 {
            let started = std::time::Instant::now();
            if let Some(delta) = throttle.feed((1, 0)) {
                watch.push(delta);
            }
            let _ = watch.answer();
            waits.push(started.elapsed().as_micros());
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        waits.sort();
        let at = |q: f64| waits[((waits.len() as f64 * q) as usize).min(waits.len() - 1)];
        println!(
            "an event waits: p99 {} us, p99.9 {} us, max {} us",
            at(0.99),
            at(0.999),
            waits[waits.len() - 1]
        );
    }

    #[test]
    fn motion_is_summed_and_checked_at_most_every_8_ms() {
        let start = std::time::Instant::now();
        let mut throttle = Throttle {
            checked: start,
            sum: (0, 0),
        };
        let ms = |n| start + std::time::Duration::from_millis(n);
        // A 1000 Hz mouse moving right: no question for the first 7 reports.
        for n in 1..8 {
            assert_eq!(throttle.feed_at((1, 0), ms(n)), None);
        }
        assert_eq!(
            throttle.feed_at((1, 0), ms(8)),
            Some((8, 0)),
            "summed, then asked"
        );
        assert_eq!(throttle.feed_at((0, 2), ms(9)), None);
        assert_eq!(throttle.feed_at((0, 0), ms(20)), Some((0, 2)));
        assert_eq!(
            throttle.feed_at((0, 0), ms(40)),
            None,
            "no motion, no question"
        );
    }

    use super::*;

    const LAPTOP: [Monitor; 1] = [Monitor {
        x: 0,
        y: 0,
        width: 1536,
        height: 864,
    }];
    /// The owner's Windows PC, with the laptop where they placed it: left
    /// of the main monitor, bottoms lined up.
    const WINDOWS: [Monitor; 3] = [
        Monitor {
            x: 0,
            y: 0,
            width: 2560,
            height: 1080,
        },
        Monitor {
            x: 246,
            y: -1080,
            width: 1920,
            height: 1080,
        },
        Monitor {
            x: 2560,
            y: -845,
            width: 1080,
            height: 1920,
        },
    ];
    const LAPTOP_AT: [i32; 2] = [-1536, 216];

    #[test]
    fn only_a_push_off_the_desktop_counts() {
        assert_eq!(beyond(&WINDOWS, (0, 500), (-3, 0)), Some((-1, 500)));
        assert_eq!(beyond(&WINDOWS, (0, 500), (3, 0)), None);
        assert_eq!(beyond(&WINDOWS, (1, 500), (-3, 0)), None);
        // Down along the left edge is not leaving through it.
        assert_eq!(beyond(&WINDOWS, (0, 500), (0, 4)), None);
        // The main monitor's right edge touches the sideways monitor.
        assert_eq!(beyond(&WINDOWS, (2559, 500), (5, 0)), None);
    }

    #[test]
    fn across_where_the_desktops_touch_and_back() {
        // Windows' left edge at y = 500 is the laptop's right edge, 284 down.
        let there = onto(&LAPTOP, LAPTOP_AT, (-1, 500)).unwrap();
        assert_eq!(there, (1535, 284));
        // Pushing right from there comes back to the same place.
        let off = beyond(&LAPTOP, there, (2, 0)).unwrap();
        assert_eq!(back(&WINDOWS, LAPTOP_AT, off), Some((0, 500)));
    }

    #[test]
    fn where_the_desktops_do_not_touch_nothing_happens() {
        // Left of the main monitor but above the laptop's top (y = 216).
        assert_eq!(onto(&LAPTOP, LAPTOP_AT, (-1, 100)), None);
        // Pushing up off the laptop: nothing of Windows is above it.
        let off = beyond(&LAPTOP, (100, 0), (0, -1)).unwrap();
        assert_eq!(back(&WINDOWS, LAPTOP_AT, off), None);
    }
}
