//! How a sound stream goes (PRD 0012), told once a minute: how far apart
//! the two PCs' sound clocks run, and how much the buffer holds.
//!
//! Each datagram's counter moves at the sending PC's clock; the player
//! takes one datagram's worth (5 ms) at this PC's sound card's clock. Each
//! is set against this PC's system clock from the first turn that played,
//! so what piled up while the sound card opened is not counted. The two
//! apart is the drift: at 50 ppm the buffer gains or loses 3 ms a minute.

use std::time::{Duration, Instant};

/// How often a stream says how it goes.
pub const EVERY: Duration = Duration::from_secs(60);
/// One datagram's worth of sound.
const DATAGRAM: Duration = Duration::from_millis(5);

#[derive(Debug, Default)]
pub struct Drift {
    /// When the first turn played, and the counter playing then; the
    /// newest counter heard.
    start: Option<(Instant, u64)>,
    newest: Option<u64>,
    /// Turns the player took since the first sound.
    turns: u64,
    /// What the buffer held at each turn this minute, in ms.
    held: Held,
    said: Option<Instant>,
}

#[derive(Debug, Default)]
struct Held {
    least: Option<usize>,
    most: usize,
    sum: u64,
    turns: u64,
}

/// A minute's news.
#[derive(Debug, PartialEq)]
pub struct Report {
    /// The sending PC's clock, and this PC's sound card's, against this
    /// PC's system clock, in parts per million (`None` before a minute).
    pub sender_ppm: Option<f64>,
    pub card_ppm: Option<f64>,
    /// What the buffer held this minute, in ms: least, average, most.
    pub held: (usize, usize, usize),
}

impl Report {
    /// The two clocks apart: how fast the buffer fills (+) or empties (-).
    pub fn drift_ppm(&self) -> Option<f64> {
        Some(self.sender_ppm? - self.card_ppm?)
    }
}

impl Drift {
    pub fn heard(&mut self, counter: u64) {
        self.newest = Some(self.newest.map_or(counter, |newest| newest.max(counter)));
    }

    /// The player took a turn at `now`; the buffer held `held_ms`.
    pub fn turn(&mut self, held_ms: usize, now: Instant) {
        let Some(newest) = self.newest else {
            return;
        };
        // Playing now: the newest, less what is held.
        self.start
            .get_or_insert((now, newest.saturating_sub(held_ms as u64 / 5)));
        self.turns += 1;
        let held = &mut self.held;
        held.least = Some(held.least.map_or(held_ms, |least| least.min(held_ms)));
        held.most = held.most.max(held_ms);
        held.sum += held_ms as u64;
        held.turns += 1;
    }

    /// The news, once every `EVERY`; it starts the next minute's.
    pub fn report(&mut self, now: Instant) -> Option<Report> {
        let (start, first) = self.start?;
        let due = self.said.is_none_or(|said| now >= said + EVERY);
        if !due || self.held.turns == 0 {
            return None;
        }
        self.said = Some(now);
        let elapsed = now.duration_since(start);
        let ppm = |count: u64| {
            (elapsed >= EVERY).then(|| {
                ((DATAGRAM * count as u32).as_secs_f64() / elapsed.as_secs_f64() - 1.0) * 1e6
            })
        };
        let held = std::mem::take(&mut self.held);
        Some(Report {
            sender_ppm: ppm(self.newest.unwrap_or(first).saturating_sub(first)),
            card_ppm: ppm(self.turns),
            held: (
                held.least.unwrap_or(0),
                (held.sum / held.turns) as usize,
                held.most,
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_clocks_set_against_this_pcs_give_the_drift() {
        let start = Instant::now();
        let mut drift = Drift::default();
        drift.heard(1000);
        drift.turn(0, start);
        // A hundred minutes: 1 200 000 datagrams' worth at the system
        // clock, so each ppm is whole datagrams.
        let nominal = 1_200_000u64;
        // The sender ran 50 ppm fast, the sound card 10 ppm slow.
        drift.heard(1000 + nominal + nominal * 50 / 1_000_000);
        for _ in 1..nominal - nominal * 10 / 1_000_000 {
            drift.turn(0, start);
        }
        let report = drift.report(start + Duration::from_secs(6000)).unwrap();
        let close = |got: Option<f64>, want: f64| (got.unwrap() - want).abs() < 1.0;
        assert!(close(report.sender_ppm, 50.0), "{report:?}");
        assert!(close(report.card_ppm, -10.0), "{report:?}");
        assert!(close(report.drift_ppm(), 60.0), "{report:?}");
        assert_eq!(report.held, (0, 0, 0));
    }

    #[test]
    fn the_first_minute_says_only_what_the_buffer_held() {
        let start = Instant::now();
        let mut drift = Drift::default();
        drift.heard(0);
        drift.turn(10, start);
        drift.turn(30, start);
        let report = drift.report(start + Duration::from_secs(1)).unwrap();
        assert_eq!(report.sender_ppm, None);
        assert_eq!(report.held, (10, 20, 30));
        assert!(
            drift.report(start + Duration::from_secs(2)).is_none(),
            "once a minute"
        );
    }
}
