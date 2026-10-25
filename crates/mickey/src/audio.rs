//! Shared audio's engine (PRD 0012): a PC sends what it plays to the PC
//! that listens, which plays each sender as its own stream.
//!
//! - **Listening** is always ready: one UDP socket on the input port + 2.
//!   A datagram plays only if its stream's key came over a trusted data
//!   link (`heard_key`), from the same address. Each sending PC gets a
//!   thread with a jitter buffer and a player; it ends after a short
//!   silence (nothing plays there) and starts afresh with the next sound,
//!   so the buffer never grows through a pause.
//! - **Sending** (`Sending`) runs while another PC listens, is online and
//!   this PC's mode is not *Here*: it takes the sound off the speakers
//!   (mode *Listener*), records, seals and sends. Silence is not sent.

use crate::audio_wire::{Opener, PCM_BYTES, Sealer, StreamKey};
use crate::jitter::{Jitter, Turn};
use crate::sound::{Player, Recorder};
use mickey_config::SoundMode;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

/// The audio port is the input port + 2 (the data link's is + 1).
pub fn port(input: u16) -> u16 {
    input.wrapping_add(2)
}

/// A player with nothing new for this long ends (nothing plays there).
const IDLE: Duration = Duration::from_millis(250);
/// How often a sender gives the listener its key again, in case the data
/// link was not open the first time.
const KEY_AGAIN: Duration = Duration::from_secs(2);

/// A stream the listener may play: who sends it, and its key.
struct Stream {
    from: IpAddr,
    name: String,
    opener: Opener,
    player: Option<Sender<(u64, [u8; PCM_BYTES])>>,
}

static KEYS: OnceLock<Sender<(StreamKey, IpAddr, String)>> = OnceLock::new();

/// Starts listening on `port`. Once per process.
pub fn listen(port: u16) -> std::io::Result<()> {
    let socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], port)))?;
    let (keys, heard) = mpsc::channel();
    let _ = KEYS.set(keys);
    thread::Builder::new()
        .name("audio in".into())
        .spawn(move || receive(&socket, &heard))?;
    Ok(())
}

/// A stream's key, from the data link of the trusted PC at `from`.
pub fn heard_key(hex: &str, from: IpAddr, name: &str) {
    let key = unhex(hex).and_then(|bytes| StreamKey::from_bytes(&bytes));
    match (key, KEYS.get()) {
        (Some(key), Some(keys)) => {
            let _ = keys.send((key, from, name.to_string()));
        }
        (None, _) => eprintln!("mickey: a bad audio key from {name}"),
        (_, None) => {}
    }
}

/// Hex digits back to bytes; `None` for anything else.
fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn receive(socket: &UdpSocket, heard: &Receiver<(StreamKey, IpAddr, String)>) {
    let mut streams: HashMap<u32, Stream> = HashMap::new();
    let mut datagram = [0u8; 2048];
    loop {
        let Ok((len, from)) = socket.recv_from(&mut datagram) else {
            continue;
        };
        // Keys first: the first datagrams of a stream follow its key.
        while let Ok((key, ip, name)) = heard.try_recv() {
            streams.insert(
                key.id,
                Stream {
                    from: ip,
                    name,
                    opener: Opener::new(&key),
                    player: None,
                },
            );
        }
        let datagram = &datagram[..len];
        let Some(stream) = Opener::stream_of(datagram).and_then(|id| streams.get_mut(&id)) else {
            continue;
        };
        if stream.from != from.ip() {
            continue;
        }
        let Some(opened) = stream.opener.open(datagram) else {
            continue;
        };
        let alive = stream
            .player
            .as_ref()
            .is_some_and(|player| player.send(opened).is_ok());
        if !alive {
            let (player, sound) = mpsc::channel();
            let _ = player.send(opened);
            let (name, from) = (stream.name.clone(), stream.from);
            if thread::Builder::new()
                .name("audio out".into())
                .spawn(move || play(&name, from, &sound))
                .is_ok()
            {
                stream.player = Some(player);
            }
        }
    }
}

/// How loud the PC at `from` plays here: its volume in config.toml, found
/// by its key (the volume is shared by key, PRD 0012).
fn volume_of(from: IpAddr) -> f32 {
    let Ok(Some(config)) = mickey_config::Config::load() else {
        return 1.0;
    };
    config
        .peers
        .iter()
        .find(|peer| peer.address.ip() == from)
        .and_then(|peer| peer.key.as_deref())
        .map_or(1.0, |key| config.audio.volume_of(key))
}

/// How often a player looks at its PC's volume again: a change in the
/// settings is heard within this.
const VOLUME_AGAIN: Duration = Duration::from_secs(1);

/// Plays one sending PC until it goes quiet.
fn play(name: &str, from: IpAddr, sound: &Receiver<(u64, [u8; PCM_BYTES])>) {
    let mut player = match Player::open(name) {
        Ok(player) => player,
        Err(err) => return eprintln!("mickey: {err}"),
    };
    let mut jitter = Jitter::default();
    let mut heard = Instant::now();
    let mut volume = volume_of(from);
    let mut volume_read = Instant::now();
    let silence = [0u8; PCM_BYTES];
    loop {
        loop {
            match sound.try_recv() {
                Ok((counter, pcm)) => {
                    jitter.push(counter, pcm);
                    heard = Instant::now();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if heard.elapsed() > IDLE {
            println!(
                "mickey: {name} went quiet (buffer was {} ms)",
                jitter.delay_ms()
            );
            return;
        }
        let pcm = match jitter.pull() {
            Turn::Sound(pcm) => *pcm,
            Turn::Silence => silence,
        };
        // Waits for room: the sound card's clock paces this loop.
        if volume_read.elapsed() > VOLUME_AGAIN {
            volume = volume_of(from);
            volume_read = Instant::now();
        }
        if let Err(err) = player.write(&pcm, volume) {
            return eprintln!("mickey: {err}");
        }
    }
}

/// This PC sending its sound to the listener; stops when dropped.
pub struct Sending {
    stop: Arc<AtomicBool>,
    to: SocketAddr,
}

impl Sending {
    /// Starts sending to `to` (the listener's audio port). `give_key` hands
    /// the stream's key to the listener over the data link.
    pub fn start(
        to: SocketAddr,
        mode: SoundMode,
        give_key: impl Fn(&str) + Send + 'static,
    ) -> Result<Sending, String> {
        let stream = StreamKey::new()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        thread::Builder::new()
            .name("audio out".into())
            .spawn(move || {
                if let Err(err) = send(&stream, to, mode, &stopped, &give_key) {
                    eprintln!("mickey: no sound sent: {err}");
                }
            })
            .map_err(|err| err.to_string())?;
        println!("mickey: sending this PC's sound to {to}");
        Ok(Sending { stop, to })
    }

    pub fn to(&self) -> SocketAddr {
        self.to
    }
}

impl Drop for Sending {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn send(
    stream: &StreamKey,
    to: SocketAddr,
    mode: SoundMode,
    stop: &AtomicBool,
    give_key: &dyn Fn(&str),
) -> Result<(), String> {
    let key = crate::trust::to_hex(&stream.to_bytes());
    give_key(&key);
    let mut key_given = Instant::now();
    // Off the speakers first, then record: on Linux the recorder follows
    // the default output, which `divert` changes.
    let _diverted = match mode {
        SoundMode::Listener => Some(crate::speakers::divert()?),
        _ => None,
    };
    let mut recorder = Recorder::open()?;
    let socket =
        UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).map_err(|err| err.to_string())?;
    let mut sealer = Sealer::new(stream);
    while !stop.load(Ordering::Relaxed) {
        if key_given.elapsed() > KEY_AGAIN {
            give_key(&key);
            key_given = Instant::now();
        }
        let Some(pcm) = recorder.read()? else {
            continue;
        };
        if pcm.iter().all(|&byte| byte == 0) {
            continue;
        }
        let _ = socket.send_to(&sealer.seal(&pcm), to);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_key_survives_hex() {
        let key = super::StreamKey::new().unwrap();
        let hex = crate::trust::to_hex(&key.to_bytes());
        assert!(super::unhex(&hex).and_then(|b| super::StreamKey::from_bytes(&b)) == Some(key));
        assert_eq!(super::unhex("abc"), None);
        assert_eq!(super::unhex("zz"), None);
    }
}
