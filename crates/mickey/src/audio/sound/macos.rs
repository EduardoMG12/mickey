//! macOS shared audio playback (`AudioQueue`) and system-audio capture
//! (`AudioHardwareCreateProcessTap`, macOS 14.2+).

use super::at_volume;
use crate::audio::wire::PCM_BYTES;
use crate::platform::macos_audio::{AudioQueuePlayer, TapRecorder};

pub struct Recorder(TapRecorder);

impl Recorder {
    pub fn open(diverted: bool) -> Result<Recorder, String> {
        TapRecorder::open(diverted).map(Recorder)
    }

    pub fn read(&mut self) -> Result<Option<[u8; PCM_BYTES]>, String> {
        self.0.read()
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
