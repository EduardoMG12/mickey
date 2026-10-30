//! Fuzzing every decoder that reads bytes from the network (the security
//! battery after 0.2.0). Each one gets valid messages, mangled in every
//! way a hostile or broken peer could (bytes changed, inserted, cut,
//! repeated, lengths set to 0 or the maximum), and plain random bytes. The
//! rule: never a panic, never a slow answer, whatever arrives.
//!
//! `cargo test fuzz` runs a short pass on every build; the long pass is
//! `MICKEY_FUZZ=2000000 cargo test --release fuzz -- --ignored`.
//! No crate needed: a small xorshift generator, seeded, so a failure
//! repeats.

use crate::audio::jitter::Jitter;
use crate::audio::wire::{Opener, PCM_BYTES, Sealer, StreamKey};
use crate::clipboard::Content;
use crate::clipboard::wire::Wire;
use crate::net::discovery::Announce;
use crate::net::group::{self, Member};
use crate::net::protocol;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

/// The slowest one decode may take before it counts as a finding.
const SLOW: Duration = Duration::from_millis(250);

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

/// `seed`, changed in one to four ways.
fn mangle(rng: &mut Rng, seed: &[u8]) -> Vec<u8> {
    let mut data = seed.to_vec();
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(data.len() + 1);
        // An index inside the data, when there is any.
        let inside = at.min(data.len().saturating_sub(1));
        let op = rng.below(8);
        match op {
            0 if !data.is_empty() => data[inside] ^= 1 << rng.below(8),
            1 if !data.is_empty() => data[inside] = rng.next() as u8,
            2 => data.insert(at, rng.next() as u8),
            3 if !data.is_empty() => {
                data.remove(inside);
            }
            4 => data.truncate(at),
            5 => {
                // A run of 0x00 or 0xFF: lengths and counts at their extremes.
                let fill = if rng.below(2) == 0 { 0 } else { 0xff };
                let run = 1 + rng.below(8);
                for i in 0..run {
                    if let Some(byte) = data.get_mut(at + i) {
                        *byte = fill;
                    }
                }
            }
            6 if !data.is_empty() => {
                let from = rng.below(data.len());
                let end = (from + 1 + rng.below(16)).min(data.len());
                let piece = data[from..end].to_vec();
                let to = rng.below(data.len() + 1);
                data.splice(to..to, piece);
            }
            _ => {
                let n = rng.below(32);
                data.extend(rng.bytes(n));
            }
        }
    }
    data
}

/// Runs `decode` on mangled seeds and on random bytes, `rounds` times, and
/// fails on a panic or a slow answer, naming the input.
fn hammer(name: &str, rounds: usize, seeds: &[Vec<u8>], mut decode: impl FnMut(&[u8])) {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ name.len() as u64);
    let mut slowest = Duration::ZERO;
    for round in 0..rounds {
        let input = if round % 4 == 0 || seeds.is_empty() {
            let n = rng.below(1100);
            rng.bytes(n)
        } else {
            let seed = &seeds[rng.below(seeds.len())];
            mangle(&mut rng, seed)
        };
        let started = Instant::now();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode(&input)));
        let took = started.elapsed();
        assert!(
            outcome.is_ok(),
            "{name}: panicked on round {round}, input {input:02x?}"
        );
        assert!(
            took < SLOW,
            "{name}: {took:?} on round {round}, input of {} bytes",
            input.len()
        );
        slowest = slowest.max(took);
    }
    println!("{name}: {rounds} inputs, slowest {slowest:?}");
}

fn rounds(short: usize) -> usize {
    std::env::var("MICKEY_FUZZ")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(short)
}

fn input_link() {
    let mut frame = [0u8; protocol::MAX_FRAME];
    let seeds: Vec<Vec<u8>> = [
        protocol::Message::Ping { id: 7 },
        protocol::Message::ReleaseAll,
    ]
    .iter()
    .map(|message| {
        let len = protocol::encode(message, &mut frame);
        frame[..len].to_vec()
    })
    .chain((0..32u8).map(|kind| vec![kind; 1 + kind as usize]))
    .collect();
    hammer(
        "input link (protocol::decode)",
        rounds(20_000),
        &seeds,
        |bytes| {
            let _ = protocol::decode(bytes);
        },
    );
}

fn clipboard() {
    let wires = [
        Wire::Ask,
        Wire::Content(Content::Text("see you at 3pm".into())),
        Wire::Content(Content::Image {
            width: 2,
            height: 2,
            rgba: vec![9; 16],
        }),
        Wire::Files(vec![
            ("a.txt".into(), b"hello".to_vec()),
            ("b".into(), vec![]),
        ]),
    ];
    let seeds: Vec<Vec<u8>> = wires
        .iter()
        .map(|wire| crate::clipboard::wire::encode(wire).unwrap())
        .collect();
    hammer(
        "clipboard (clipboard::wire::decode)",
        rounds(20_000),
        &seeds,
        |bytes| {
            let _ = crate::clipboard::wire::decode(bytes);
        },
    );
}

fn group_map() {
    let member = |n: u8, port: u16| Member {
        key: [n; 32],
        name: format!("PC {n}"),
        address: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 0, 2, n)), port),
        position: Some([-1920, 0]),
    };
    let text = group::encode(&member(1, 4242), &[member(2, 4242), member(3, 4300)]);
    let from = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
    hammer(
        "group map (group::decode)",
        rounds(20_000),
        &[text.into_bytes()],
        |bytes| {
            let _ = group::decode(&String::from_utf8_lossy(bytes), from);
        },
    );
}

fn discovery() {
    let seed = Announce {
        name: "Desk".into(),
        port: 4242,
        key: [5; 32],
    }
    .encode();
    hammer(
        "discovery (Announce::decode)",
        rounds(20_000),
        &[seed],
        |bytes| {
            let _ = Announce::decode(bytes);
        },
    );
}

fn audio() {
    let stream = StreamKey::new().unwrap();
    let mut sealer = Sealer::new(&stream);
    let seeds: Vec<Vec<u8>> = (0..4).map(|n| sealer.seal(&[n; PCM_BYTES])).collect();
    let mut opener = Opener::new(&stream);
    let mut jitter = Jitter::default();
    hammer(
        "audio (Opener::open, then the jitter buffer)",
        rounds(20_000),
        &seeds,
        |bytes| {
            if let Some((counter, pcm)) = opener.open(bytes) {
                jitter.push(counter, pcm);
            }
            let _ = jitter.pull();
        },
    );
    // Counters from a peer that lies: far ahead, far behind, the extremes.
    let mut jitter = Jitter::default();
    let mut rng = Rng(42);
    for _ in 0..rounds(20_000) {
        let counter = match rng.below(4) {
            0 => u64::MAX - rng.below(4) as u64,
            1 => rng.below(4) as u64,
            _ => rng.next(),
        };
        jitter.push(counter, [0; PCM_BYTES]);
        let _ = jitter.pull();
    }
}

#[test]
fn every_decoder_survives_mangled_and_random_bytes() {
    input_link();
    clipboard();
    group_map();
    discovery();
    audio();
}

/// The long pass: MICKEY_FUZZ sets the inputs per decoder.
#[test]
#[ignore = "long: run with MICKEY_FUZZ=2000000 cargo test --release fuzz -- --ignored"]
fn every_decoder_survives_a_long_run() {
    every_decoder_survives_mangled_and_random_bytes();
}
