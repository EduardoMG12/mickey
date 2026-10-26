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
///
/// A monitor gives silence too, without end: reading it 5 ms at a time
/// cost 2.8% of a core with nothing playing, over the 1% budget. After a
/// second of silence the recorder turns drowsy and reads 50 ms at a time;
/// the first sound after silence wakes it, and those 50 ms are still sent,
/// once, up to 50 ms late.
pub struct Recorder {
    simple: Simple,
    source: &'static str,
    /// Silent 5 ms frames in a row, while awake.
    quiet: u32,
    drowsy: bool,
    /// Sound read while drowsy, given out 5 ms at a time.
    waiting: std::collections::VecDeque<[u8; PCM_BYTES]>,
}

/// Silent frames before the recorder turns drowsy: one second.
const QUIET_FRAMES: u32 = 200;
/// Frames read at once while drowsy: 50 ms.
const DROWSY_FRAMES: usize = 10;

impl Recorder {
    /// Records what this PC plays: Mickey's silent sink by name once
    /// `diverted` (asking for the default's monitor right after switching
    /// the default still gave the speakers', a race the two-PC test found),
    /// else the default output's.
    pub fn open(diverted: bool) -> Result<Recorder, String> {
        let source = if diverted {
            "mickey.monitor"
        } else {
            DEFAULT_MONITOR
        };
        Ok(Recorder {
            simple: record(source, PCM_BYTES)?,
            source,
            quiet: 0,
            drowsy: false,
            waiting: std::collections::VecDeque::new(),
        })
    }

    /// The next 5 ms, waiting for them; `None` while drowsy and silent.
    pub fn read(&mut self) -> Result<Option<[u8; PCM_BYTES]>, String> {
        if let Some(pcm) = self.waiting.pop_front() {
            return Ok(Some(pcm));
        }
        if self.drowsy {
            let mut long = [0u8; PCM_BYTES * DROWSY_FRAMES];
            self.simple
                .read(&mut long)
                .map_err(|err| format!("recording stopped: {err}"))?;
            if long.iter().all(|&byte| byte == 0) {
                return Ok(None);
            }
            // Sound: awake again, 5 ms at a time, and these 50 ms go out.
            self.simple = record(self.source, PCM_BYTES)?;
            self.drowsy = false;
            self.quiet = 0;
            let (frames, _) = long.as_chunks::<PCM_BYTES>();
            self.waiting.extend(frames.iter().copied());
            return Ok(self.waiting.pop_front());
        }
        let mut pcm = [0u8; PCM_BYTES];
        self.simple
            .read(&mut pcm)
            .map_err(|err| format!("recording stopped: {err}"))?;
        if pcm.iter().all(|&byte| byte == 0) {
            self.quiet += 1;
            if self.quiet >= QUIET_FRAMES {
                self.simple = record(self.source, PCM_BYTES * DROWSY_FRAMES)?;
                self.drowsy = true;
            }
        } else {
            self.quiet = 0;
        }
        Ok(Some(pcm))
    }
}

/// A recording stream of `source`, handing over `fragment` bytes at a time.
fn record(source: &str, fragment: usize) -> Result<Simple, String> {
    Simple::new(
        None,
        "Mickey",
        Direction::Record,
        Some(source),
        "What this PC plays, for the PC that listens",
        &spec(),
        None,
        Some(&attr(u32::MAX, fragment as u32)),
    )
    .map_err(|err| format!("cannot record {source}: {err}"))
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
        let mut recorder = Recorder::open(false).unwrap();
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
            let pcm = recorder
                .read()
                .unwrap()
                .expect("a monitor always gives sound");
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

/// CPU time this process has used so far, in clock ticks (100 a second).
#[cfg(test)]
fn cpu_ticks() -> u64 {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let after = stat.rsplit_once(')').map_or("", |(_, rest)| rest);
    let fields: Vec<&str> = after.split_whitespace().collect();
    // utime and stime are fields 14 and 15; after "pid (comm)" they are 11 and 12.
    fields.get(11).zip(fields.get(12)).map_or(0, |(u, s)| {
        u.parse::<u64>().unwrap_or(0) + s.parse::<u64>().unwrap_or(0)
    })
}

#[cfg(test)]
mod idle {
    /// The recorder's CPU while nothing plays (second performance review):
    /// `cargo test --release recorder_idle_cost -- --ignored --nocapture`
    /// with no sound playing.
    #[test]
    #[ignore = "a measurement; needs PulseAudio or PipeWire"]
    fn recorder_idle_cost() {
        let mut recorder = super::Recorder::open(false).expect("a monitor to record");
        let started = std::time::Instant::now();
        let ticks = super::cpu_ticks();
        let (mut reads, mut sound) = (0, 0);
        while started.elapsed() < std::time::Duration::from_secs(10) {
            if let Some(pcm) = recorder.read().unwrap() {
                reads += 1;
                if pcm.iter().any(|&b| b != 0) {
                    sound += 1;
                }
            }
        }
        let used = (super::cpu_ticks() - ticks) as f64 / 100.0;
        println!(
            "recorder, 10 s: {:.2}% of one core, {reads} frames ({sound} with sound)",
            used / started.elapsed().as_secs_f64() * 100.0
        );
    }

    /// After the recorder turns drowsy, sound wakes it and is recorded.
    /// The "sound" is a sample value of 1: not zero, and not audible, since
    /// it may reach the PC that listens.
    #[test]
    #[ignore = "needs PulseAudio or PipeWire"]
    fn sound_wakes_a_drowsy_recorder() {
        let mut recorder = super::Recorder::open(false).expect("a monitor to record");
        // Over a second of silence: drowsy.
        for _ in 0..(super::QUIET_FRAMES + 5) {
            recorder.read().unwrap();
        }
        assert!(recorder.drowsy);
        let player = std::thread::spawn(|| {
            let mut player = super::Player::open("a test").unwrap();
            let mut faint = [0u8; crate::audio_wire::PCM_BYTES];
            for sample in faint.as_chunks_mut::<2>().0 {
                *sample = 1i16.to_le_bytes();
            }
            for _ in 0..100 {
                player.write(&faint, 1.0).unwrap();
            }
        });
        let started = std::time::Instant::now();
        let mut heard = None;
        while heard.is_none() && started.elapsed() < std::time::Duration::from_secs(3) {
            if let Some(pcm) = recorder.read().unwrap()
                && pcm.iter().any(|&b| b != 0)
            {
                heard = Some(started.elapsed());
            }
        }
        player.join().unwrap();
        let heard = heard.expect("the sound is recorded");
        assert!(!recorder.drowsy, "awake again");
        println!("sound heard {} ms after it started", heard.as_millis());
    }
}
