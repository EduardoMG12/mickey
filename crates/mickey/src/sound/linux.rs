//! Linux: libpulse's blocking ("simple") API, which PipeWire also answers.
//! A `Recorder` reads the default output's monitor: `mickey.monitor` once
//! `speakers` made the silent sink the default (so divert first), or the
//! speakers' own when this PC keeps its sound too. A `Player` is one stream
//! per sending PC, named after it, so the system's own volume control shows
//! each one; its writes block while the sound card's buffer is full, which
//! is what paces the jitter buffer.

use super::at_volume;
use crate::audio_wire::{CHANNELS, PCM_BYTES, RATE};
use libpulse_binding::def::BufferAttr;
use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;

/// The default output's monitor, whatever the default is.
const DEFAULT_MONITOR: &str = "@DEFAULT_MONITOR@";
/// How much the player keeps queued for the sound card: 20 ms, enough to
/// ride over a late wake-up, little enough not to be heard as delay.
const PLAY_AHEAD: u32 = 4 * PCM_BYTES as u32;

fn spec() -> Spec {
    Spec {
        format: Format::S16le,
        rate: RATE,
        channels: CHANNELS as u8,
    }
}

/// Small fragments: the server's default held a second of audio back.
fn attr(play_ahead: u32, fragment: u32) -> BufferAttr {
    BufferAttr {
        maxlength: u32::MAX,
        tlength: play_ahead,
        prebuf: u32::MAX,
        minreq: u32::MAX,
        fragsize: fragment,
    }
}

/// What this PC plays, 5 ms at a time.
pub struct Recorder(Simple);

impl Recorder {
    /// Records what the default output plays.
    pub fn open() -> Result<Recorder, String> {
        let source = DEFAULT_MONITOR;
        Simple::new(
            None,
            "Mickey",
            Direction::Record,
            Some(source),
            "What this PC plays, for the PC that listens",
            &spec(),
            None,
            Some(&attr(u32::MAX, PCM_BYTES as u32)),
        )
        .map(Recorder)
        .map_err(|err| format!("cannot record {source}: {err}"))
    }

    /// The next 5 ms; waits for them.
    pub fn read(&mut self) -> Result<[u8; PCM_BYTES], String> {
        let mut pcm = [0u8; PCM_BYTES];
        self.0
            .read(&mut pcm)
            .map_err(|err| format!("recording stopped: {err}"))?;
        Ok(pcm)
    }
}

/// Another PC's sound, played here.
pub struct Player(Simple);

impl Player {
    /// A stream named after the PC it plays, on the default output.
    pub fn open(pc: &str) -> Result<Player, String> {
        Simple::new(
            None,
            "Mickey",
            Direction::Playback,
            None,
            &format!("Mickey: {pc}"),
            &spec(),
            None,
            Some(&attr(PLAY_AHEAD, u32::MAX)),
        )
        .map(Player)
        .map_err(|err| format!("cannot play {pc}'s sound: {err}"))
    }

    /// Plays 5 ms at `volume` (0.0 silent, 1.0 as sent); waits while the
    /// sound card's buffer is full.
    pub fn write(&mut self, pcm: &[u8; PCM_BYTES], volume: f32) -> Result<(), String> {
        self.0
            .write(&at_volume(pcm, volume))
            .map_err(|err| format!("playing stopped: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sound::samples;

    /// Run by hand on a Linux desktop: plays a quiet 440 Hz tone for one
    /// second on the default output while recording that output's monitor,
    /// and prints how loud the recording was, and how long the first
    /// sound took to come back.
    #[test]
    #[ignore = "plays a tone on this PC for one second"]
    fn a_tone_played_comes_back_recorded() {
        let mut recorder = Recorder::open().unwrap();
        let player = std::thread::spawn(|| {
            let mut player = Player::open("test").unwrap();
            let mut phase = 0f32;
            for _ in 0..200 {
                let mut pcm = [0u8; PCM_BYTES];
                for frame in pcm.as_chunks_mut::<4>().0 {
                    let s = ((phase * std::f32::consts::TAU).sin() * 6000.0) as i16;
                    phase = (phase + 440.0 / RATE as f32) % 1.0;
                    frame[..2].copy_from_slice(&s.to_le_bytes());
                    frame[2..].copy_from_slice(&s.to_le_bytes());
                }
                player.write(&pcm, 1.0).unwrap();
            }
        });
        let started = std::time::Instant::now();
        let (mut first, mut sum, mut n) = (None, 0f64, 0usize);
        for _ in 0..240 {
            let pcm = recorder.read().unwrap();
            let loud = samples(&pcm).iter().any(|s| s.unsigned_abs() > 1000);
            if loud && first.is_none() {
                first = Some(started.elapsed());
            }
            for s in samples(&pcm) {
                sum += f64::from(s) * f64::from(s);
                n += 1;
            }
        }
        player.join().unwrap();
        println!(
            "rms {:.3}, first sound back after {:?}",
            (sum / n as f64).sqrt() / 32768.0,
            first
        );
    }
}
