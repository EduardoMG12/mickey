//! macOS shared audio playback (`AudioQueue`) and capture placeholder
//! (system audio capture via CoreAudio Process Taps follows in a separate PR).

use super::at_volume;
use crate::audio::wire::PCM_BYTES;
use crate::platform::macos_audio::AudioQueuePlayer;

pub struct Recorder;

impl Recorder {
    pub fn open(_diverted: bool) -> Result<Recorder, String> {
        Err("shared audio capture on macOS is not implemented yet".into())
    }

    pub fn read(&mut self) -> Result<Option<[u8; PCM_BYTES]>, String> {
        Err("shared audio capture on macOS is not implemented yet".into())
    }
}

pub struct Player(AudioQueuePlayer);

impl Player {
    pub fn open(pc: &str) -> Result<Player, String> {
        AudioQueuePlayer::open(pc).map(Player)
    }

    pub fn write(&mut self, pcm: &[u8; PCM_BYTES], volume: f32) -> Result<(), String> {
        self.0.write(&at_volume(pcm, volume))
    }
}
