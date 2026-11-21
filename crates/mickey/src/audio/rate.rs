//! Playing a sound a hair faster or slower than it came (PRD 0012), so the
//! buffer holds what it aims at whatever the two PCs' clocks do: they
//! never agree exactly (tens of ppm apart, and more), and the buffer would
//! otherwise fill (delay) or run dry (a click). PipeWire's ROC and RTP
//! modules do the same.
//!
//! The speed follows how much the buffer holds against its aim, at most
//! 0.2% either way: a delay 100 ms too long goes in under a minute, and a
//! change of pitch that small is not heard.

use crate::audio::wire::{CHANNELS, FRAMES, PCM_BYTES, RATE};
use std::collections::VecDeque;

/// How much faster per ms held over the aim, in parts per million: clocks
/// 300 ppm apart then settle 3 ms off the aim.
const PPM_PER_MS: f64 = 100.0;
/// The most it plays faster or slower, in parts per million (0.2%).
const MOST_PPM: f64 = 2000.0;
/// How quickly the held time it follows moves: it changes by 5 ms each
/// time a datagram comes, so it is averaged over about half a second.
const SMOOTHING: f64 = 0.01;

/// The speed for a buffer holding `held_ms`, aiming at `aim_ms`.
pub fn speed(held_ms: f64, aim_ms: f64) -> f64 {
    1.0 + ((held_ms - aim_ms) * PPM_PER_MS).clamp(-MOST_PPM, MOST_PPM) / 1e6
}

/// Sound in hand, played at a speed.
#[derive(Default)]
pub struct Rate {
    /// Frames not played yet; the next one played is at `at` among them.
    frames: VecDeque<[f32; CHANNELS]>,
    at: f64,
    /// The held time followed, averaged.
    held_ms: Option<f64>,
}

impl Rate {
    /// Adds 5 ms of sound.
    pub fn feed(&mut self, pcm: &[u8; PCM_BYTES]) {
        for frame in pcm.as_chunks::<{ 2 * CHANNELS }>().0 {
            let sample = |i: usize| f32::from(i16::from_le_bytes([frame[2 * i], frame[2 * i + 1]]));
            self.frames.push_back(std::array::from_fn(sample));
        }
    }

    /// What it holds, in ms of sound.
    pub fn held_ms(&self) -> f64 {
        (self.frames.len() as f64 - self.at) * 1000.0 / f64::from(RATE)
    }

    /// Whether it holds enough for 5 ms more at `speed`.
    pub fn ready(&self, speed: f64) -> bool {
        self.frames.len() as f64 >= self.at + FRAMES as f64 * speed + 2.0
    }

    /// The speed to play at, the buffer holding `held_ms` (with this) and
    /// aiming at `aim_ms`.
    pub fn speed_for(&mut self, held_ms: f64, aim_ms: f64) -> f64 {
        let averaged = match self.held_ms {
            Some(before) => before + (held_ms - before) * SMOOTHING,
            None => held_ms,
        };
        self.held_ms = Some(averaged);
        speed(averaged, aim_ms)
    }

    /// The next 5 ms, read at `speed` (1.0: as it came). Call when
    /// `ready(speed)`.
    pub fn take(&mut self, speed: f64) -> [u8; PCM_BYTES] {
        let mut out = [0u8; PCM_BYTES];
        for frame in out.as_chunks_mut::<{ 2 * CHANNELS }>().0 {
            let whole = self.at.floor();
            let part = (self.at - whole) as f32;
            let (a, b) = (whole as usize, whole as usize + 1);
            for (channel, bytes) in frame.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let get = |i: usize| self.frames.get(i).map_or(0.0, |frame| frame[channel]);
                let value = get(a) + (get(b) - get(a)) * part;
                bytes.copy_from_slice(&(value.round() as i16).to_le_bytes());
            }
            self.at += speed;
        }
        let played = self.at.floor();
        self.frames
            .drain(..(played as usize).min(self.frames.len()));
        self.at -= played;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5 ms whose frame `i` holds `start + i` on both channels.
    fn ramp(start: i16) -> [u8; PCM_BYTES] {
        let mut pcm = [0u8; PCM_BYTES];
        for (i, frame) in pcm
            .as_chunks_mut::<{ 2 * CHANNELS }>()
            .0
            .iter_mut()
            .enumerate()
        {
            let value = (start + i as i16).to_le_bytes();
            frame[..2].copy_from_slice(&value);
            frame[2..].copy_from_slice(&value);
        }
        pcm
    }

    #[test]
    fn at_speed_one_it_plays_what_came() {
        let mut rate = Rate::default();
        rate.feed(&ramp(0));
        rate.feed(&ramp(240));
        assert!(rate.ready(1.0));
        assert_eq!(rate.take(1.0), ramp(0));
        assert_eq!(rate.held_ms(), 5.0);
    }

    #[test]
    fn faster_it_plays_more_of_what_came() {
        let mut rate = Rate::default();
        for start in [0, 240, 480] {
            rate.feed(&ramp(start));
        }
        rate.take(1.01);
        // 1% faster: 242.4 frames read for 240 played.
        assert!((rate.held_ms() - (720.0 - 242.4) / 48.0).abs() < 0.01);
    }

    #[test]
    fn the_speed_follows_the_buffer_within_a_fifth_of_a_percent() {
        assert_eq!(speed(20.0, 20.0), 1.0);
        assert!(
            (speed(25.0, 20.0) - 1.0005).abs() < 1e-9,
            "5 ms over: 500 ppm"
        );
        assert_eq!(speed(500.0, 20.0), 1.002, "never more than 0.2%");
        assert_eq!(speed(0.0, 300.0), 0.998);
    }

    /// Four minutes of a sender whose clock runs `ppm` apart from this
    /// PC's sound card: the most and least held once it settled.
    fn follow(ppm: f64) -> (f64, f64) {
        let mut rate = Rate::default();
        let aim = 20.0;
        let mut owed = 0.0f64;
        for _ in 0..4 {
            rate.feed(&ramp(0));
        }
        let (mut most, mut least) = (0.0f64, f64::MAX);
        for turn in 0..48_000u64 {
            owed += 1.0 + ppm / 1e6;
            while owed >= 1.0 {
                rate.feed(&ramp(0));
                owed -= 1.0;
            }
            let speed = rate.speed_for(rate.held_ms(), aim);
            assert!(rate.ready(speed), "ran dry at turn {turn}");
            rate.take(speed);
            if turn > 12_000 {
                most = most.max(rate.held_ms());
                least = least.min(rate.held_ms());
            }
        }
        (most, least)
    }

    #[test]
    fn a_fast_or_slow_sender_is_followed_and_the_buffer_holds_steady() {
        let (most, _) = follow(300.0);
        assert!(most < 45.0, "a fast sender: held at most {most} ms");
        let (_, least) = follow(-300.0);
        assert!(least > 5.0, "a slow sender: held at least {least} ms");
    }
}
