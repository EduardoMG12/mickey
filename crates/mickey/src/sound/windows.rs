//! Windows: WASAPI in shared mode, through the `wasapi` crate. A
//! `Recorder` is a loopback capture of the default output (what this PC
//! plays, even muted); it gets nothing while nothing plays, so then nothing
//! is sent. A `Player` is one render stream per sending PC; its writes wait
//! while the device's buffer is full, which paces the jitter buffer. Each
//! is made on the thread that uses it (COM).

use super::at_volume;
use crate::audio_wire::{CHANNELS, FRAMES, PCM_BYTES, RATE};
use std::collections::VecDeque;
use std::time::Duration;
use wasapi::{
    AudioCaptureClient, AudioClient, AudioRenderClient, Direction, SampleType, StreamMode,
    WaveFormat, get_default_device, initialize_mta,
};

/// The device buffer, in 100 ns units: 20 ms.
const BUFFER: i64 = 200_000;

fn client(direction: Direction) -> Result<AudioClient, String> {
    let _ = initialize_mta();
    let format = WaveFormat::new(16, 16, &SampleType::Int, RATE as usize, CHANNELS, None);
    let device = get_default_device(&Direction::Render).map_err(|err| err.to_string())?;
    let mut client = device.get_iaudioclient().map_err(|err| err.to_string())?;
    let mode = StreamMode::PollingShared {
        autoconvert: true,
        buffer_duration_hns: BUFFER,
    };
    // Capture on a render device is loopback: what it plays.
    client
        .initialize_client(&format, &direction, &mode)
        .map_err(|err| err.to_string())?;
    Ok(client)
}

/// What this PC plays, 5 ms at a time.
pub struct Recorder {
    client: AudioClient,
    capture: AudioCaptureClient,
    queue: VecDeque<u8>,
}

impl Recorder {
    /// Records what the default output plays.
    pub fn open() -> Result<Recorder, String> {
        let client = client(Direction::Capture)
            .map_err(|err| format!("cannot record this PC's sound: {err}"))?;
        let capture = client
            .get_audiocaptureclient()
            .map_err(|err| err.to_string())?;
        client.start_stream().map_err(|err| err.to_string())?;
        Ok(Recorder {
            client,
            capture,
            queue: VecDeque::with_capacity(8 * PCM_BYTES),
        })
    }

    /// The next 5 ms; waits for them (while nothing plays, that is until
    /// something does).
    pub fn read(&mut self) -> Result<[u8; PCM_BYTES], String> {
        while self.queue.len() < PCM_BYTES {
            self.capture
                .read_from_device_to_deque(&mut self.queue)
                .map_err(|err| format!("recording stopped: {err}"))?;
            if self.queue.len() < PCM_BYTES {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        let mut pcm = [0u8; PCM_BYTES];
        for (byte, queued) in pcm.iter_mut().zip(self.queue.drain(..PCM_BYTES)) {
            *byte = queued;
        }
        Ok(pcm)
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
}

impl Player {
    /// A stream on the default output. (Windows names it after Mickey.)
    pub fn open(pc: &str) -> Result<Player, String> {
        let client =
            client(Direction::Render).map_err(|err| format!("cannot play {pc}'s sound: {err}"))?;
        let render = client
            .get_audiorenderclient()
            .map_err(|err| err.to_string())?;
        client.start_stream().map_err(|err| err.to_string())?;
        Ok(Player { client, render })
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
            std::thread::sleep(Duration::from_millis(1));
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
    use crate::sound::samples;

    /// Run by hand on Windows, in the owner's session: mutes the speakers
    /// the way Send only does, plays a quiet tone through a Player while a
    /// Recorder captures the output, and prints the mute state, how loud
    /// the capture was and how long the first sound took to come back.
    #[test]
    #[ignore = "mutes this PC's speakers and plays a tone for one second"]
    fn the_tone_is_captured_while_the_speakers_are_muted() {
        let muted = || crate::platform::output::muted().unwrap();
        println!("muted before: {}", muted());
        let diverted = crate::speakers::divert().unwrap();
        println!("muted while sending: {}", muted());
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
        for _ in 0..200 {
            let pcm = recorder.read().unwrap();
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
}
