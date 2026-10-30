//! Windows: WASAPI in shared mode, through the `wasapi` crate. A
//! `Recorder` is a loopback capture of the default output (what this PC
//! plays, even muted); it gets nothing while nothing plays, so then nothing
//! is sent. A `Player` is one render stream per sending PC; its writes wait
//! while the device's buffer is full, which paces the jitter buffer. Each
//! is made on the thread that uses it (COM).
//!
//! Both wait on the event WASAPI signals once per device period, not on
//! sleeps: polling woke Mickey about 500 times a second even in silence
//! (#349). A loopback capture signals only while something plays, so a
//! quiet PC sleeps until the 50 ms timeout.

use super::at_volume;
use crate::audio::wire::{CHANNELS, FRAMES, PCM_BYTES, RATE};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use wasapi::{
    AudioCaptureClient, AudioClient, AudioRenderClient, Direction, Handle, SampleType, StreamMode,
    WaveFormat, get_default_device, initialize_mta,
};

/// The device buffer, in 100 ns units: 20 ms.
const BUFFER: i64 = 200_000;

/// How long a read waits for sound before saying there is none.
const QUIET: Duration = Duration::from_millis(50);

/// A shared-mode client and the event it signals once per device period.
fn client(direction: Direction) -> Result<(AudioClient, Handle), String> {
    let _ = initialize_mta();
    let format = WaveFormat::new(16, 16, &SampleType::Int, RATE as usize, CHANNELS, None);
    let device = get_default_device(&Direction::Render).map_err(|err| err.to_string())?;
    let mut client = device.get_iaudioclient().map_err(|err| err.to_string())?;
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: BUFFER,
    };
    // Capture on a render device is loopback: what it plays.
    client
        .initialize_client(&format, &direction, &mode)
        .map_err(|err| err.to_string())?;
    // Asked for after initializing and before starting, as WASAPI wants.
    let wake = client
        .set_get_eventhandle()
        .map_err(|err| err.to_string())?;
    Ok((client, wake))
}

/// What this PC plays, 5 ms at a time.
pub struct Recorder {
    client: AudioClient,
    capture: AudioCaptureClient,
    wake: Handle,
    queue: VecDeque<u8>,
}

impl Recorder {
    /// Records what the default output plays (muted or not: `diverted`
    /// changes nothing here).
    pub fn open(_diverted: bool) -> Result<Recorder, String> {
        let (client, wake) = client(Direction::Capture)
            .map_err(|err| format!("cannot record this PC's sound: {err}"))?;
        let capture = client
            .get_audiocaptureclient()
            .map_err(|err| err.to_string())?;
        client.start_stream().map_err(|err| err.to_string())?;
        Ok(Recorder {
            client,
            capture,
            wake,
            queue: VecDeque::with_capacity(8 * PCM_BYTES),
        })
    }

    /// The next 5 ms; `None` after 50 ms with nothing, which is what a
    /// loopback capture gives while nothing plays (so the caller can stop).
    pub fn read(&mut self) -> Result<Option<[u8; PCM_BYTES]>, String> {
        let started = Instant::now();
        loop {
            self.capture
                .read_from_device_to_deque(&mut self.queue)
                .map_err(|err| format!("recording stopped: {err}"))?;
            if self.queue.len() >= PCM_BYTES {
                break;
            }
            let left = QUIET.saturating_sub(started.elapsed());
            // A timeout: nothing played for 50 ms.
            if left.is_zero() || self.wake.wait_for_event(left.as_millis() as u32).is_err() {
                return Ok(None);
            }
        }
        let mut pcm = [0u8; PCM_BYTES];
        for (byte, queued) in pcm.iter_mut().zip(self.queue.drain(..PCM_BYTES)) {
            *byte = queued;
        }
        Ok(Some(pcm))
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

/// Another PC's sound, played here.
pub struct Player {
    client: AudioClient,
    render: AudioRenderClient,
    wake: Handle,
}

impl Player {
    /// A stream on the default output. (Windows names it after Mickey.)
    pub fn open(pc: &str) -> Result<Player, String> {
        let (client, wake) =
            client(Direction::Render).map_err(|err| format!("cannot play {pc}'s sound: {err}"))?;
        let render = client
            .get_audiorenderclient()
            .map_err(|err| err.to_string())?;
        client.start_stream().map_err(|err| err.to_string())?;
        Ok(Player {
            client,
            render,
            wake,
        })
    }

    /// Plays 5 ms at `volume` (0.0 silent, 1.0 as sent); waits while the
    /// device's buffer is full.
    pub fn write(&mut self, pcm: &[u8; PCM_BYTES], volume: f32) -> Result<(), String> {
        loop {
            let space = self
                .client
                .get_available_space_in_frames()
                .map_err(|err| format!("playing stopped: {err}"))?;
            if space as usize >= FRAMES {
                break;
            }
            // Signalled once per device period, when room was made. A
            // timeout (a device that stopped) asks again, and the next
            // question says what went wrong.
            let _ = self.wake.wait_for_event(QUIET.as_millis() as u32);
        }
        self.render
            .write_to_device(FRAMES, &at_volume(pcm, volume), None)
            .map_err(|err| format!("playing stopped: {err}"))
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.client.stop_stream();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::sound::samples;

    /// Run by hand on Windows, in the owner's session: mutes the speakers
    /// the way Send only does, plays a quiet tone through a Player while a
    /// Recorder captures the output, and prints the mute state, how loud
    /// the capture was and how long the first sound took to come back.
    #[test]
    #[ignore = "mutes this PC's speakers and plays a tone for one second"]
    fn the_tone_is_captured_while_the_speakers_are_muted() {
        let muted = || crate::platform::output::muted().unwrap();
        println!("muted before: {}", muted());
        let diverted = crate::audio::speakers::divert().unwrap();
        println!("muted while sending: {}", muted());
        let mut recorder = Recorder::open(true).unwrap();
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
        for _ in 0..200 {
            let Some(pcm) = recorder.read().unwrap() else {
                continue;
            };
            let all = samples(&pcm);
            if first.is_none() && all.iter().any(|s| s.unsigned_abs() > 1000) {
                first = Some(started.elapsed());
            }
            for s in all {
                sum += f64::from(s) * f64::from(s);
                n += 1;
            }
        }
        player.join().unwrap();
        println!(
            "captured rms {:.3}, first sound back after {first:?}",
            (sum / n as f64).sqrt() / 32768.0
        );
        drop(diverted);
        println!("muted after: {}", muted());
    }

    /// Run by hand on Windows to measure wake-ups (#349): records for 60 s,
    /// for something outside to count this process's CPU time and context
    /// switches. Quiet unless something plays.
    #[test]
    #[ignore = "records this PC's sound for 60 seconds"]
    fn a_recorder_for_a_minute() {
        let mut recorder = Recorder::open(false).unwrap();
        let started = std::time::Instant::now();
        let mut got = 0;
        while started.elapsed() < std::time::Duration::from_secs(60) {
            got += usize::from(recorder.read().unwrap().is_some());
        }
        println!("5 ms blocks recorded: {got}");
    }

    /// The same for a Player given silence: its writes pace themselves.
    #[test]
    #[ignore = "plays silence for 60 seconds"]
    fn a_player_for_a_minute() {
        let mut player = Player::open("test").unwrap();
        let started = std::time::Instant::now();
        while started.elapsed() < std::time::Duration::from_secs(60) {
            player.write(&[0u8; PCM_BYTES], 1.0).unwrap();
        }
    }
}
