//! The jitter buffer (PRD 0012): audio datagrams arrive when the network
//! lets them, early, late or out of order; the sound card wants one every
//! 5 ms, exactly. The buffer holds a few datagrams in hand so a late one is
//! still there in time, as few as the network allows: it starts at 2
//! (10 ms), grows by one each time one was missing when its turn came, and
//! gives one back after a calm stretch, so the delay stays low.

use crate::audio_wire::PCM_BYTES;

/// Datagrams the ring holds: 320 ms, more than any target.
const SLOTS: usize = 64;
/// The fewest datagrams in hand (10 ms).
const MIN_TARGET: usize = 2;
/// The most (80 ms): past that, waiting longer costs more than a gap.
const MAX_TARGET: usize = 16;
/// Turns with nothing missing before the buffer gives one back (10 s):
/// shorter, and a jittery network makes it shrink, miss, and grow again,
/// a crackle every few seconds.
const CALM: u32 = 2000;

/// What to play for one 5 ms turn.
#[derive(Debug, PartialEq, Eq)]
pub enum Turn {
    Sound(Box<[u8; PCM_BYTES]>),
    /// Nothing to play: before the first datagrams, or one missing.
    Silence,
}

pub struct Jitter {
    /// `slots[counter % SLOTS]` holds that datagram until it plays.
    slots: Vec<Option<(u64, Box<[u8; PCM_BYTES]>)>>,
    /// The counter to play next; `None` until the buffer first fills.
    next: Option<u64>,
    /// How many datagrams to hold in hand.
    target: usize,
    /// Turns since one was missing.
    calm: u32,
}

impl Default for Jitter {
    fn default() -> Jitter {
        Jitter {
            slots: (0..SLOTS).map(|_| None).collect(),
            next: None,
            target: MIN_TARGET,
            calm: 0,
        }
    }
}

impl Jitter {
    /// A datagram from the network, as `audio_wire` opened it.
    pub fn push(&mut self, counter: u64, pcm: [u8; PCM_BYTES]) {
        if let Some(next) = self.next {
            // Its turn is gone.
            if counter < next {
                return;
            }
            // Far ahead (the sender started over, or a long gap): start
            // again from it rather than wait through the gap.
            if counter >= next + SLOTS as u64 {
                self.restart();
            }
        }
        self.slots[counter as usize % SLOTS] = Some((counter, Box::new(pcm)));
    }

    /// The next 5 ms to play; called by the sound card's clock.
    pub fn pull(&mut self) -> Turn {
        let Some(next) = self.next.or_else(|| self.ready()) else {
            return Turn::Silence;
        };
        self.next = Some(next + 1);
        match self.take(next) {
            Some(pcm) => {
                self.calm += 1;
                if self.calm >= CALM && self.target > MIN_TARGET && self.held() >= self.target {
                    // Calm: hold one fewer, by skipping one ahead.
                    self.target -= 1;
                    self.calm = 0;
                    self.take(next + 1);
                    self.next = Some(next + 2);
                }
                Turn::Sound(pcm)
            }
            None => {
                // Missing when its turn came: hold one more from now on.
                self.target = (self.target + 1).min(MAX_TARGET);
                self.calm = 0;
                if self.held() == 0 {
                    // Nothing left at all: wait to fill up again.
                    self.next = None;
                } else if self.held() < self.target {
                    // Later ones are few: it is likely still on its way.
                    // Wait for it: the sound slips 5 ms later, which is how
                    // the buffer comes to hold one more.
                    self.next = Some(next);
                }
                // Else enough later ones are here: it is lost, play on.
                Turn::Silence
            }
        }
    }

    /// The delay the buffer holds, for the log and the mixer.
    pub fn delay_ms(&self) -> usize {
        self.target * 5
    }

    /// Once `target` datagrams are in hand, the oldest is the first to play.
    fn ready(&self) -> Option<u64> {
        (self.held() >= self.target)
            .then(|| self.oldest())
            .flatten()
    }

    fn held(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    fn oldest(&self) -> Option<u64> {
        self.slots
            .iter()
            .flatten()
            .map(|(counter, _)| *counter)
            .min()
    }

    fn take(&mut self, counter: u64) -> Option<Box<[u8; PCM_BYTES]>> {
        let slot = &mut self.slots[counter as usize % SLOTS];
        match slot {
            Some((held, _)) if *held == counter => slot.take().map(|(_, pcm)| pcm),
            _ => None,
        }
    }

    fn restart(&mut self) {
        self.slots.iter_mut().for_each(|slot| *slot = None);
        self.next = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 5 ms of PCM that says which datagram it is.
    fn pcm(counter: u64) -> [u8; PCM_BYTES] {
        [counter as u8; PCM_BYTES]
    }

    fn heard(turn: Turn) -> Option<u8> {
        match turn {
            Turn::Sound(pcm) => Some(pcm[0]),
            Turn::Silence => None,
        }
    }

    #[test]
    fn a_steady_stream_plays_in_order_after_two_in_hand() {
        let mut jitter = Jitter::default();
        jitter.push(0, pcm(0));
        assert_eq!(heard(jitter.pull()), None, "one in hand is not enough");
        let mut played = Vec::new();
        for counter in 1..50 {
            jitter.push(counter, pcm(counter));
            played.push(heard(jitter.pull()));
        }
        assert!(played.iter().all(Option::is_some), "no gap: {played:?}");
        assert_eq!(played[..3], [Some(0), Some(1), Some(2)]);
        assert_eq!(jitter.delay_ms(), 10);
    }

    #[test]
    fn out_of_order_datagrams_play_in_order() {
        let mut jitter = Jitter::default();
        for counter in [1, 0, 3, 2, 5, 4] {
            jitter.push(counter, pcm(counter));
        }
        let played: Vec<_> = (0..6).map(|_| heard(jitter.pull())).collect();
        assert_eq!(played, [0, 1, 2, 3, 4, 5].map(Some));
    }

    #[test]
    fn a_lost_datagram_is_5_ms_of_silence_and_the_buffer_grows() {
        let mut jitter = Jitter::default();
        for counter in (0..10).filter(|&c| c != 4) {
            jitter.push(counter, pcm(counter));
        }
        let played: Vec<_> = (0..10).map(|_| heard(jitter.pull())).collect();
        assert_eq!(played[3], Some(3));
        assert_eq!(played[4], None, "the lost one");
        assert_eq!(played[5], Some(5), "and on at once");
        assert_eq!(jitter.delay_ms(), 15);
    }

    #[test]
    fn jitter_makes_it_hold_more_until_nothing_is_missing() {
        let mut jitter = Jitter::default();
        // Datagram n leaves at turn n and arrives 0 to 4 turns (20 ms)
        // later, in a fixed, uneven pattern, as on a busy Wi-Fi.
        let late = [0, 3, 1, 0, 4, 2, 0, 1, 3, 0, 2, 4, 1, 0, 0, 3];
        let arrives = |n: u64| n + late[n as usize % late.len()];
        let mut gaps_per_second = Vec::new();
        for second in 0..4u64 {
            let mut gaps = 0;
            for turn in second * 200..(second + 1) * 200 {
                for n in turn.saturating_sub(4)..=turn {
                    if arrives(n) == turn {
                        jitter.push(n, pcm(n));
                    }
                }
                gaps += usize::from(heard(jitter.pull()).is_none());
            }
            gaps_per_second.push(gaps);
        }
        assert!(gaps_per_second[0] > 0, "{gaps_per_second:?}");
        assert_eq!(gaps_per_second[3], 0, "{gaps_per_second:?}");
        assert!(jitter.delay_ms() <= 40, "{} ms", jitter.delay_ms());
    }

    #[test]
    fn a_calm_network_gives_the_delay_back() {
        let mut jitter = Jitter {
            target: 8,
            ..Jitter::default()
        };
        let mut counter = 0;
        for _ in 0..8 {
            jitter.push(counter, pcm(counter));
            counter += 1;
        }
        for _ in 0..(CALM as usize * 3) {
            jitter.push(counter, pcm(counter));
            counter += 1;
            jitter.pull();
        }
        assert_eq!(
            jitter.delay_ms(),
            25,
            "8 → 5 datagrams after three calm stretches"
        );
    }

    #[test]
    fn a_sender_that_starts_over_is_followed() {
        let mut jitter = Jitter::default();
        for counter in 1000..1010 {
            jitter.push(counter, pcm(counter));
            jitter.pull();
        }
        jitter.push(0, pcm(0));
        jitter.push(5000, pcm(5000));
        jitter.push(5001, pcm(5001));
        let played: Vec<_> = (0..3).map(|_| heard(jitter.pull())).collect();
        assert!(played.contains(&Some((5000 % 256) as u8)), "{played:?}");
    }
}
