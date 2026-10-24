//! Recording what this PC plays, and playing what another PC sends (PRD
//! 0012), in the datagrams' unit: 5 ms of 48 kHz stereo 16-bit PCM. One
//! `Recorder` and one `Player` per OS, with the same calls.

use crate::audio_wire::PCM_BYTES;

#[cfg(target_os = "linux")]
mod linux;
#[allow(unused_imports)] // all used once streams start (#270)
#[cfg(target_os = "linux")]
pub use linux::{Player, Recorder};
#[cfg(windows)]
mod windows;
#[allow(unused_imports)] // all used once streams start (#270)
#[cfg(windows)]
pub use windows::{Player, Recorder};

/// `pcm` with every sample scaled by `volume` (0.0 to 1.0): each PC's own
/// volume, applied here so it is the same on Linux and Windows.
pub fn at_volume(pcm: &[u8; PCM_BYTES], volume: f32) -> [u8; PCM_BYTES] {
    let volume = volume.clamp(0.0, 1.0);
    let mut out = [0u8; PCM_BYTES];
    for (from, to) in pcm
        .as_chunks::<2>()
        .0
        .iter()
        .zip(out.as_chunks_mut::<2>().0)
    {
        let sample = i16::from_le_bytes(*from);
        let scaled = (f32::from(sample) * volume).round() as i16;
        to.copy_from_slice(&scaled.to_le_bytes());
    }
    out
}

/// The samples of 5 ms of PCM.
pub fn samples(pcm: &[u8; PCM_BYTES]) -> Vec<i16> {
    pcm.as_chunks::<2>()
        .0
        .iter()
        .map(|b| i16::from_le_bytes(*b))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_scales_every_sample() {
        let mut pcm = [0u8; PCM_BYTES];
        for (i, chunk) in pcm.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let sample: i16 = if i % 2 == 0 { 20_000 } else { -32_768 };
            chunk.copy_from_slice(&sample.to_le_bytes());
        }
        assert_eq!(samples(&at_volume(&pcm, 1.0))[..2], [20_000, -32_768]);
        assert_eq!(samples(&at_volume(&pcm, 0.5))[..2], [10_000, -16_384]);
        assert!(
            samples(&at_volume(&pcm, 0.0)).iter().all(|&s| s == 0),
            "muted"
        );
        assert_eq!(
            samples(&at_volume(&pcm, 7.0))[..2],
            [20_000, -32_768],
            "never louder"
        );
    }
}
