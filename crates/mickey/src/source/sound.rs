//! Who listens: the one PC whose headphones play every PC's sound
//! (PRD 0012), kept the same on every PC.

use super::Peer;
use mickey_config as config;
use std::net::SocketAddr;

/// Writes who listens, as a peer shared it, into config.toml, keeping this
/// PC's own mode; nothing when it is already so (every new data link
/// sends it again).
pub(super) fn take_audio(lines: &str) -> Result<config::Audio, String> {
    let path = config::path()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mine = config::parse(&text)?.audio;
    let taken = mine.taking(lines)?;
    if taken != mine {
        config::write(&path, &config::set_audio(&text, &taken)?)?;
        println!("mickey: another PC changed who listens");
    }
    Ok(taken)
}

/// Makes this PC the one that listens, in config.toml (the tray's
/// "Listen here").
pub(super) fn listen_here(me: &[u8; 32]) -> Result<config::Audio, String> {
    let path = config::path()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let audio = config::Audio {
        listener: Some(crate::hex::encode(me)),
        since: config::now_seconds(),
        ..config::parse(&text)?.audio
    };
    config::write(&path, &config::set_audio(&text, &audio)?)?;
    println!("mickey: this PC listens now");
    Ok(audio)
}

/// Where this PC's sound should go now (PRD 0012): the listener's audio
/// port and its index, when another PC listens, is online, and this PC's
/// mode sends.
pub(super) fn audio_target(
    audio: &config::Audio,
    me: &[u8; 32],
    peers: &[Peer],
    online: &[bool],
) -> Option<(usize, SocketAddr)> {
    if audio.mode == config::SoundMode::Here {
        return None;
    }
    // Compared as bytes: this runs on every input event, and writing two
    // keys as hex text cost 66 allocations and 7 us each time.
    let listener = crate::hex::decode_key(audio.listener.as_deref()?)?;
    if listener == *me {
        return None;
    }
    let index = peers.iter().position(|peer| peer.key == Some(listener))?;
    let address = peers[index].address;
    online[index].then(|| {
        (
            index,
            SocketAddr::new(address.ip(), crate::audio::port(address.port())),
        )
    })
}
