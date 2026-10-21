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

pub struct Latency {
    /// One-way estimates, in microseconds.
    samples: Vec<u32>,
}

impl Latency {
    pub fn new() -> Latency {
        Latency {
            samples: Vec::new(),
        }
    }

    pub fn add_round_trip(&mut self, micros: u32) {
        self.samples.push(micros / 2);
    }

    /// p50 and p99 one-way latency, in microseconds; `None` with no samples.
    pub fn percentiles(&mut self) -> Option<(u32, u32)> {
        Some((self.percentile(50)?, self.percentile(99)?))
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// The value with `percent`% of the samples at or below it
    /// (nearest-rank method).
    fn percentile(&mut self, percent: usize) -> Option<u32> {
        if self.samples.is_empty() {
            return None;
        }
        let rank = (self.samples.len() * percent).div_ceil(100).max(1);
        let (_, value, _) = self.samples.select_nth_unstable(rank - 1);
        Some(*value)
    }
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
    fn one_sample_is_every_percentile() {
        let mut latency = Latency::new();
        latency.add_round_trip(800);
        assert_eq!(latency.percentiles(), Some((400, 400)));
    }
}
