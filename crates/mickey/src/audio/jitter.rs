//! The jitter buffer (PRD 0012): audio datagrams arrive when the network
//! lets them, early, late or out of order; the sound card wants one every
//! 5 ms, exactly. The buffer holds a few datagrams in hand so a late one is
//! still there in time, as few as the network allows: it starts at 2
//! (10 ms), grows by one each time one was missing when its turn came, and
//! gives one back after a calm stretch, so the delay stays low.

use crate::audio::wire::PCM_BYTES;

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
/// Datagrams it must always have held over a calm stretch to aim at one
/// fewer: a margin left, not a stretch where it nearly ran dry.
const SPARE: usize = 2;
/// At a start, what is kept beyond the target: a burst of the network is
/// played, a pile-up is not (6 datagrams, 30 ms).
const START_SLACK: usize = 6;

/// What to play for one 5 ms turn.
#[derive(Debug, PartialEq, Eq)]
pub enum Turn {
    Sound(Box<[u8; PCM_BYTES]>),
    /// Nothing to play: before the first datagrams, or one missing.
    Silence,
}

pub struct Jitter {
    /// Turns that found nothing though sound was going (clicks), and
    /// datagrams thrown away to catch up, since it began (for `Drift`).
    pub gaps: u64,
    pub dropped: u64,
    /// Whether it ever started: only the first start cuts a pile-up.
    started: bool,
    /// The least it held, in datagrams, since the calm stretch began.
    low: usize,
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
            gaps: 0,
            dropped: 0,
            started: false,
            low: usize::MAX,
            slots: (0..SLOTS).map(|_| None).collect(),
            next: None,
            target: MIN_TARGET,
            calm: 0,
        }
    }
}

impl Jitter {
    /// A datagram from the network, as `audio::wire` opened it.
    pub fn push(&mut self, counter: u64, pcm: [u8; PCM_BYTES]) {
        if let Some(next) = self.next {
            // Its turn is gone.
            if counter < next {
                return;
            }
            // Far ahead (the sender started over, or a long gap): start
            // again from it rather than wait through the gap.
            // Counters come from the network: sums never past u64.
            if counter >= next.saturating_add(SLOTS as u64) {
                self.dropped += self.held() as u64;
                self.restart();
            }
        }
        self.slots[counter as usize % SLOTS] = Some((counter, Box::new(pcm)));
    }

    /// The next 5 ms to play; called by the sound card's clock.
    pub fn pull(&mut self) -> Turn {
        let next = match self.next {
            Some(next) => Some(next),
            None => self.start(),
        };
        let Some(next) = next else {
            return Turn::Silence;
        };
        self.next = Some(next.saturating_add(1));
        match self.take(next) {
            Some(pcm) => {
                self.calm += 1;
                self.low = self.low.min(self.held());
                if self.calm >= CALM {
                    // Calm, and never near empty: aim at one fewer. Shrinking
                    // into where datagrams run late only made it miss, grow
                    // and shrink again, a click every few seconds. Nothing
                    // is skipped either (a click): the player plays a hair
                    // faster until it holds that (`rate`).
                    if self.target > MIN_TARGET && self.low >= SPARE {
                        self.target -= 1;
                    }
                    self.calm = 0;
                    self.low = usize::MAX;
                }
                Turn::Sound(pcm)
            }
            None => {
                self.gaps += 1;
                self.low = usize::MAX;
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

    /// What it holds now, in ms of sound.
    pub fn held_ms(&self) -> usize {
        self.held() * 5
    }

    /// Once `target` datagrams are in hand, where to start. The first time:
    /// the newest `target + START_SLACK` of them, as older ones piled up
    /// while the sound card was opening (a third of a second on Windows)
    /// would only be delay; they go before anything plays, so no click.
    fn start(&mut self) -> Option<u64> {
        if self.held() < self.target {
            return None;
        }
        // Only the stream's first start: later, after the buffer ran dry
        // (a sound card that asks in bursts, a late network), what came is
        // just in time, and cutting it would lose sound.
        if std::mem::replace(&mut self.started, true) {
            return self.oldest();
        }
        let newest = self.newest()?;
        let keep = (self.target + START_SLACK) as u64;
        // Counters come from the network: never past u64.
        let first = newest.saturating_add(1).saturating_sub(keep);
        for slot in &mut self.slots {
            if slot.as_ref().is_some_and(|(counter, _)| *counter < first) {
                *slot = None;
                self.dropped += 1;
            }
        }
        self.oldest()
    }

    fn newest(&self) -> Option<u64> {
        self.slots
            .iter()
            .flatten()
            .map(|(counter, _)| *counter)
            .max()
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
        // Two in hand, then one comes for each one played, as in a stream.
        let mut sent = (0..12).filter(|&c| c != 4);
        for counter in sent.by_ref().take(2) {
            jitter.push(counter, pcm(counter));
        }
        let played: Vec<_> = (0..10)
            .map(|_| {
                if let Some(counter) = sent.next() {
                    jitter.push(counter, pcm(counter));
                }
                heard(jitter.pull())
            })
            .collect();
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

    #[test]
    fn what_piled_up_before_the_first_sound_is_never_delay() {
        let mut jitter = Jitter::default();
        // 60 datagrams (300 ms) arrived while the sound card opened.
        for counter in 0..60 {
            jitter.push(counter, [counter as u8; PCM_BYTES]);
        }
        // It starts with the newest eight (40 ms), not 300 ms behind.
        assert_eq!(jitter.pull(), Turn::Sound(Box::new([52; PCM_BYTES])));
        assert_eq!(jitter.held_ms(), 35);
        assert_eq!(jitter.dropped, 52);
    }

    #[test]
    fn running_dry_later_cuts_nothing() {
        let mut jitter = Jitter::default();
        jitter.push(0, [0; PCM_BYTES]);
        jitter.push(1, [1; PCM_BYTES]);
        jitter.pull();
        jitter.pull();
        // Dry; then a burst comes in, all just in time: all of it plays.
        jitter.pull();
        for counter in 2..14 {
            jitter.push(counter, [counter as u8; PCM_BYTES]);
        }
        jitter.pull();
        assert_eq!(jitter.dropped, 0);
        assert_eq!(jitter.gaps, 1, "the turn that found nothing");
    }

    #[test]
    fn a_calm_stretch_that_ran_near_empty_keeps_the_aim() {
        let mut jitter = Jitter {
            target: 4,
            ..Jitter::default()
        };
        // Barely enough all along: one in hand after each turn.
        let mut counter = 0;
        for _ in 0..4 {
            jitter.push(counter, pcm(counter));
            counter += 1;
        }
        for _ in 0..2 {
            jitter.pull();
        }
        for _ in 0..(CALM as usize * 3) {
            jitter.pull();
            jitter.push(counter, pcm(counter));
            counter += 1;
        }
        assert_eq!(jitter.delay_ms(), 20, "never lower than it held");
    }
}
