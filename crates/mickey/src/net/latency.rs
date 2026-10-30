//! Network latency from `Ping`/`Pong`. The two machines' clocks are not in
//! sync, so one-way latency is estimated as half the round trip.

use std::time::Instant;

/// A ping's id is the time it was sent, in microseconds since `start`.
/// It wraps after about 71 minutes, which `round_trip` handles.
pub fn ping_id(start: Instant) -> u32 {
    // Truncating to u32 on purpose: only differences between ids matter.
    start.elapsed().as_micros() as u32
}

/// Microseconds from sending the ping `id` until now.
pub fn round_trip(start: Instant, id: u32) -> u32 {
    ping_id(start).wrapping_sub(id)
}

/// Samples kept: the last hour, at one ping a second. Older ones are
/// written over, so a Mickey left running for weeks still holds 14 KB per
/// peer instead of growing by 345 KB a day (#190).
const KEPT: usize = 3600;

/// The latest one-way estimates, in a ring: once full, each new sample
/// replaces the oldest, at `next`.
pub struct Latency {
    /// One-way estimates, in microseconds, at most `KEPT`.
    samples: Vec<u32>,
    /// Where the next sample goes once the ring is full: the oldest one.
    next: usize,
}

impl Latency {
    pub fn new() -> Latency {
        Latency {
            samples: Vec::with_capacity(KEPT),
            next: 0,
        }
    }

    pub fn add_round_trip(&mut self, micros: u32) {
        if self.samples.len() < KEPT {
            self.samples.push(micros / 2);
        } else {
            self.samples[self.next] = micros / 2;
        }
        self.next = (self.next + 1) % KEPT;
    }

    /// p50 and p99 one-way latency over the samples kept, in
    /// microseconds; `None` with no samples.
    pub fn percentiles(&self) -> Option<(u32, u32)> {
        // Selecting reorders what it works on: a copy, so the ring keeps
        // its order and `next` still points at the oldest sample.
        let mut copy = self.samples.clone();
        Some((percentile(&mut copy, 50)?, percentile(&mut copy, 99)?))
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }
}

/// The value with `percent`% of `samples` at or below it (nearest-rank
/// method). Reorders `samples`.
fn percentile(samples: &mut [u32], percent: usize) -> Option<u32> {
    if samples.is_empty() {
        return None;
    }
    let rank = (samples.len() * percent).div_ceil(100).max(1);
    let (_, value, _) = samples.select_nth_unstable(rank - 1);
    Some(*value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_ping_has_a_short_round_trip() {
        let start = Instant::now();
        let id = ping_id(start);
        assert!(round_trip(start, id) < 1_000_000);
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let mut latency = Latency::new();
        assert_eq!(latency.percentiles(), None);
        // Round trips of 2, 4, ..., 200 µs: one-way 1..=100 µs, shuffled.
        for i in (1..=100).rev() {
            latency.add_round_trip(i * 2);
        }
        assert_eq!(latency.percentiles(), Some((50, 99)));
    }

    #[test]
    fn only_the_latest_hour_is_kept() {
        let mut latency = Latency::new();
        // An hour at 1000 µs one way, then an hour at 100 µs: the first
        // hour is written over entirely.
        for _ in 0..KEPT {
            latency.add_round_trip(2000);
        }
        for _ in 0..KEPT {
            latency.add_round_trip(200);
        }
        assert_eq!(latency.len(), KEPT);
        assert_eq!(latency.percentiles(), Some((100, 100)));
        // Half an hour more at 1000 µs: half the ring, the oldest half.
        for _ in 0..KEPT / 2 {
            latency.add_round_trip(2000);
        }
        assert_eq!(latency.percentiles(), Some((100, 1000)));
        assert_eq!(latency.samples.capacity(), KEPT, "the ring never grows");
    }

    #[test]
    fn one_sample_is_every_percentile() {
        let mut latency = Latency::new();
        latency.add_round_trip(800);
        assert_eq!(latency.percentiles(), Some((400, 400)));
    }
}
