//! Audio datagrams (PRD 0012): 5 ms of 48 kHz stereo 16-bit PCM each, sent
//! over UDP and sealed with a key the sending PC picked and handed to the
//! listener over the data link, which is already encrypted and
//! authenticated.
//!
//! A datagram is `[version | stream id (4) | counter (8)]`, in the clear
//! and authenticated, then the PCM sealed with ChaCha20-Poly1305, the
//! counter as the nonce. Each datagram opens on its own, so a lost or
//! late one costs 5 ms of sound and never the next ones; a replayed or
//! too old one is dropped.

use snow::params::CipherChoice;
use snow::resolvers::{CryptoResolver, DefaultResolver};
use snow::types::Cipher;

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;
/// Frames in one datagram: 5 ms.
pub const FRAMES: usize = 240;
/// PCM bytes in one datagram (960): small enough for any network's MTU.
pub const PCM_BYTES: usize = FRAMES * CHANNELS * 2;
const VERSION: u8 = 1;
const HEADER: usize = 1 + 4 + 8;
/// ChaCha20-Poly1305's key and tag.
const KEY: usize = 32;
const TAG: usize = 16;
pub const DATAGRAM: usize = HEADER + PCM_BYTES + TAG;
// Under any network's MTU, with room for IP and UDP headers.
const _: () = assert!(DATAGRAM <= 1200);
/// How far behind the newest datagram one may be and still play (320 ms).
const WINDOW: u64 = 64;

/// What a sending PC hands the listener over the data link: which stream,
/// and the key that opens it.
#[derive(Clone, PartialEq, Eq)]
pub struct StreamKey {
    pub id: u32,
    pub key: [u8; KEY],
}

impl StreamKey {
    /// A new stream: a random id and key, from the generator the identity
    /// keys use.
    pub fn new() -> Result<StreamKey, String> {
        let mut bytes = [0u8; 4 + KEY];
        DefaultResolver
            .resolve_rng()
            .ok_or("no random number generator")?
            .try_fill_bytes(&mut bytes)
            .map_err(|err| err.to_string())?;
        let (id, key) = bytes.split_at(4);
        Ok(StreamKey {
            id: u32::from_be_bytes(id.try_into().expect("4 bytes")),
            key: key.try_into().expect("32 bytes"),
        })
    }

    /// As sent on the data link.
    pub fn to_bytes(&self) -> Vec<u8> {
        [&self.id.to_be_bytes()[..], &self.key].concat()
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<StreamKey> {
        let (id, key) = bytes.split_first_chunk::<4>()?;
        Some(StreamKey {
            id: u32::from_be_bytes(*id),
            key: key.try_into().ok()?,
        })
    }
}

fn cipher(key: &[u8; KEY]) -> Box<dyn Cipher> {
    let mut cipher = DefaultResolver
        .resolve_cipher(&CipherChoice::ChaChaPoly)
        .expect("ChaChaPoly is built into snow");
    cipher.set(key);
    cipher
}

/// The sending side of one stream.
pub struct Sealer {
    id: u32,
    cipher: Box<dyn Cipher>,
    next: u64,
}

impl Sealer {
    pub fn new(stream: &StreamKey) -> Sealer {
        Sealer {
            id: stream.id,
            cipher: cipher(&stream.key),
            next: 0,
        }
    }

    /// The next datagram, carrying `pcm`.
    pub fn seal(&mut self, pcm: &[u8; PCM_BYTES]) -> Vec<u8> {
        let mut datagram = vec![0u8; DATAGRAM];
        datagram[0] = VERSION;
        datagram[1..5].copy_from_slice(&self.id.to_be_bytes());
        datagram[5..HEADER].copy_from_slice(&self.next.to_be_bytes());
        let (header, body) = datagram.split_at_mut(HEADER);
        self.cipher.encrypt(self.next, header, pcm, body);
        self.next += 1;
        datagram
    }
}

/// The listening side of one stream: opens its datagrams and refuses
/// replays.
pub struct Opener {
    id: u32,
    cipher: Box<dyn Cipher>,
    /// The highest counter opened so far.
    newest: Option<u64>,
    /// Bit `i` set: counter `newest - i` was already opened.
    seen: u64,
}

impl Opener {
    pub fn new(stream: &StreamKey) -> Opener {
        Opener {
            id: stream.id,
            cipher: cipher(&stream.key),
            newest: None,
            seen: 0,
        }
    }

    /// The stream id a datagram claims, to find its opener.
    pub fn stream_of(datagram: &[u8]) -> Option<u32> {
        let id = datagram.get(1..5)?;
        Some(u32::from_be_bytes(id.try_into().ok()?))
    }

    /// The counter and the PCM, or `None` for anything else: a short or
    /// forged datagram, another stream, a replay, or one too old to play.
    pub fn open(&mut self, datagram: &[u8]) -> Option<(u64, [u8; PCM_BYTES])> {
        if datagram.len() != DATAGRAM || datagram[0] != VERSION {
            return None;
        }
        if Opener::stream_of(datagram)? != self.id {
            return None;
        }
        let counter = u64::from_be_bytes(datagram[5..HEADER].try_into().ok()?);
        if !self.fresh(counter) {
            return None;
        }
        let mut pcm = [0u8; PCM_BYTES];
        let (header, body) = datagram.split_at(HEADER);
        self.cipher.decrypt(counter, header, body, &mut pcm).ok()?;
        // Only an authentic datagram moves the window: forged counters
        // cannot push real ones out.
        self.mark(counter);
        Some((counter, pcm))
    }

    fn fresh(&self, counter: u64) -> bool {
        match self.newest {
            None => true,
            Some(newest) if counter > newest => true,
            Some(newest) => {
                let behind = newest - counter;
                behind < WINDOW && self.seen & (1 << behind) == 0
            }
        }
    }

    fn mark(&mut self, counter: u64) {
        match self.newest {
            Some(newest) if counter <= newest => self.seen |= 1 << (newest - counter),
            Some(newest) => {
                let ahead = counter - newest;
                self.seen = if ahead < WINDOW {
                    self.seen << ahead
                } else {
                    0
                } | 1;
                self.newest = Some(counter);
            }
            None => {
                self.seen = 1;
                self.newest = Some(counter);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Sealer, Opener) {
        let stream = StreamKey::new().unwrap();
        (Sealer::new(&stream), Opener::new(&stream))
    }

    fn pcm(byte: u8) -> [u8; PCM_BYTES] {
        [byte; PCM_BYTES]
    }

    #[test]
    fn a_datagram_opens_to_its_pcm() {
        let (mut sealer, mut opener) = pair();
        let datagram = sealer.seal(&pcm(7));
        assert_eq!(datagram.len(), DATAGRAM);
        assert_eq!(opener.open(&datagram), Some((0, pcm(7))));
    }

    #[test]
    fn a_key_survives_the_data_link() {
        let stream = StreamKey::new().unwrap();
        assert!(StreamKey::from_bytes(&stream.to_bytes()) == Some(stream));
        assert!(StreamKey::from_bytes(&[1, 2, 3]).is_none());
    }

    #[test]
    fn another_key_or_a_changed_byte_is_refused() {
        let (mut sealer, _) = pair();
        let datagram = sealer.seal(&pcm(1));
        // Same stream id, another key: someone guessing.
        let mut other = Opener::new(&StreamKey {
            id: sealer.id,
            key: [9; KEY],
        });
        assert_eq!(other.open(&datagram), None);
        let (mut sealer, mut opener) = pair();
        for at in [0, 3, HEADER + 10, DATAGRAM - 1] {
            let mut forged = sealer.seal(&pcm(1));
            forged[at] ^= 1;
            assert_eq!(opener.open(&forged), None, "byte {at} changed");
        }
        assert_eq!(opener.open(&datagram[..DATAGRAM - 1]), None);
    }

    #[test]
    fn a_replay_is_refused_and_late_ones_still_play() {
        let (mut sealer, mut opener) = pair();
        let sent: Vec<Vec<u8>> = (0..5).map(|n| sealer.seal(&pcm(n))).collect();
        assert!(opener.open(&sent[0]).is_some());
        assert!(opener.open(&sent[3]).is_some());
        // 1 and 2 arrive late, out of order: both play.
        assert_eq!(opener.open(&sent[2]).map(|(n, _)| n), Some(2));
        assert_eq!(opener.open(&sent[1]).map(|(n, _)| n), Some(1));
        // Every one of them again: refused.
        for datagram in &sent[..4] {
            assert_eq!(opener.open(datagram), None);
        }
        assert!(opener.open(&sent[4]).is_some());
    }

    #[test]
    fn one_too_old_to_play_is_refused() {
        let (mut sealer, mut opener) = pair();
        let first = sealer.seal(&pcm(0));
        for _ in 0..WINDOW {
            sealer.seal(&pcm(0));
        }
        assert!(opener.open(&sealer.seal(&pcm(0))).is_some());
        assert_eq!(opener.open(&first), None, "64 datagrams behind");
    }
}
