//! The secure link (PRD 0002): a Noise `XX` handshake, then every message
//! encrypted and authenticated.
//!
//! On the wire, each Noise message is framed by a 2-byte big-endian length:
//!
//! ```text
//! [len_hi] [len_lo] [noise message...]
//! ```
//!
//! Inside an encrypted message is one protocol frame body (type and
//! payload, without the old length byte).
//!
//! A data link (PRD 0011) carries bulk payloads instead: a message holding
//! the payload's length (4 bytes), then the payload in messages of up to
//! `BULK_CHUNK` bytes each.

use crate::identity::{Identity, NOISE_PARAMS};
use crate::protocol::{self, MAX_FRAME, Message};
use snow::{HandshakeState, TransportState};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Noise adds a 16-byte authentication tag to every encrypted message.
const TAG_LEN: usize = 16;
/// The largest Noise message we accept. Handshake messages are under 100
/// bytes and transport messages under 30, so anything bigger is hostile.
const MAX_NOISE_MESSAGE: usize = 256;
// ponytail: the clipboard sync uses it (#131).
#[allow(dead_code)]
/// The largest bulk payload: a big screenshot fits, and a hostile peer
/// cannot make us hold gigabytes.
pub const MAX_BULK: usize = 64 << 20;
// ponytail: the clipboard sync uses it (#131).
#[allow(dead_code)]
/// The most plaintext one Noise message carries (65 535 with its tag).
const BULK_CHUNK: usize = u16::MAX as usize - TAG_LEN;
/// A peer that stalls in the middle of the handshake is dropped.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// One end of an encrypted connection. Clones share the same cipher state,
/// so one thread can send while another receives. (A poisoned lock is used
/// anyway: if a panic left the state half-updated, the next message fails to
/// decrypt and the link closes, which is the safe outcome.)
pub struct Link {
    stream: TcpStream,
    noise: Arc<Mutex<TransportState>>,
    peer: [u8; 32],
    /// Six digits both sides compute alike (PRD 0010).
    code: u32,
}

impl Link {
    /// Runs the handshake as the side that connected (the source).
    pub fn initiate(stream: TcpStream, identity: &Identity) -> io::Result<Link> {
        let handshake = builder(identity)?.build_initiator().map_err(noise_error)?;
        // XX: -> e, <- e ee s es, -> s se
        Link::handshake(stream, handshake, &[true, false, true])
    }

    /// Runs the handshake as the side that listened (the target).
    pub fn respond(stream: TcpStream, identity: &Identity) -> io::Result<Link> {
        let handshake = builder(identity)?.build_responder().map_err(noise_error)?;
        Link::handshake(stream, handshake, &[false, true, false])
    }

    /// The other side's public key, proven by the handshake.
    pub fn peer_key(&self) -> &[u8; 32] {
        &self.peer
    }

    /// The pairing code, e.g. "042 917": the same on both PCs only if no
    /// machine sits in between (PRD 0010).
    pub fn pairing_code(&self) -> String {
        format!("{:03} {:03}", self.code / 1000, self.code % 1000)
    }

    // Only the source clones a link (for its pong reader), so the Windows
    // build never calls this in M0.
    #[cfg_attr(windows, allow(dead_code))]
    pub fn try_clone(&self) -> io::Result<Link> {
        Ok(Link {
            stream: self.stream.try_clone()?,
            noise: Arc::clone(&self.noise),
            peer: self.peer,
            code: self.code,
        })
    }

    pub fn send(&mut self, message: &Message) -> io::Result<()> {
        let mut frame = [0u8; MAX_FRAME];
        let len = protocol::encode(message, &mut frame);
        let mut sealed = [0u8; MAX_FRAME + TAG_LEN];
        // Hold the lock until the bytes are written: messages must reach the
        // wire in the order they were encrypted, or the peer's nonces break.
        let mut noise = self
            .noise
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let sealed_len = noise
            .write_message(&frame[1..len], &mut sealed)
            .map_err(noise_error)?;
        write_noise(&mut self.stream, &sealed[..sealed_len])
    }

    /// Reads one message. A frame that was changed, replayed, dropped or
    /// reordered fails to decrypt, which is an error.
    pub fn recv(&mut self) -> io::Result<Message> {
        let mut sealed = [0u8; MAX_NOISE_MESSAGE];
        let len = read_noise(&mut self.stream, &mut sealed)?;
        let mut body = [0u8; MAX_NOISE_MESSAGE];
        // Only the decryption holds the lock, never the blocking read.
        let body_len = self
            .noise
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .read_message(&sealed[..len], &mut body)
            .map_err(noise_error)?;
        protocol::decode(&body[..body_len]).map_err(|err| invalid(err.to_string()))
    }

    // ponytail: the clipboard sync uses it (#131).
    #[allow(dead_code)]
    /// Sends `bytes` whole, on a data link (PRD 0011): never on the input
    /// link, where it would hold key presses behind it.
    pub fn send_bulk(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_BULK {
            return Err(invalid(format!(
                "{} bytes is more than a bulk payload may hold",
                bytes.len()
            )));
        }
        let length = (bytes.len() as u32).to_be_bytes();
        let mut sealed = vec![0u8; BULK_CHUNK + TAG_LEN];
        let mut noise = self
            .noise
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for piece in std::iter::once(&length[..]).chain(bytes.chunks(BULK_CHUNK)) {
            let len = noise
                .write_message(piece, &mut sealed)
                .map_err(noise_error)?;
            write_noise(&mut self.stream, &sealed[..len])?;
        }
        Ok(())
    }

    // ponytail: the clipboard sync uses it (#131).
    #[allow(dead_code)]
    /// Reads one payload sent with `send_bulk`.
    pub fn recv_bulk(&mut self) -> io::Result<Vec<u8>> {
        let mut sealed = vec![0u8; BULK_CHUNK + TAG_LEN];
        let mut plain = vec![0u8; BULK_CHUNK + TAG_LEN];
        let mut open = |stream: &mut TcpStream, plain: &mut [u8]| -> io::Result<usize> {
            let len = read_noise(stream, &mut sealed)?;
            self.noise
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .read_message(&sealed[..len], plain)
                .map_err(noise_error)
        };
        let mut stream = self.stream.try_clone()?;
        let header = open(&mut stream, &mut plain)?;
        let length: [u8; 4] = plain[..header]
            .try_into()
            .map_err(|_| invalid("a bulk payload must start with its length".into()))?;
        let length = u32::from_be_bytes(length) as usize;
        if length > MAX_BULK {
            return Err(invalid(format!(
                "the peer announced {length} bytes, too many"
            )));
        }
        let mut payload = Vec::with_capacity(length);
        while payload.len() < length {
            let got = open(&mut stream, &mut plain)?;
            if got == 0 || payload.len() + got > length {
                return Err(invalid("a bulk payload does not match its length".into()));
            }
            payload.extend_from_slice(&plain[..got]);
        }
        Ok(payload)
    }

    /// `writes[i]` says whether this side sends handshake message `i`.
    fn handshake(
        mut stream: TcpStream,
        mut handshake: HandshakeState,
        writes: &[bool],
    ) -> io::Result<Link> {
        stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
        let mut message = [0u8; MAX_NOISE_MESSAGE];
        let mut payload = [0u8; MAX_NOISE_MESSAGE];
        for &write in writes {
            if write {
                let len = handshake
                    .write_message(&[], &mut message)
                    .map_err(noise_error)?;
                write_noise(&mut stream, &message[..len])?;
            } else {
                let len = read_noise(&mut stream, &mut message)?;
                handshake
                    .read_message(&message[..len], &mut payload)
                    .map_err(noise_error)?;
            }
        }
        stream.set_read_timeout(None)?;
        let peer = handshake
            .get_remote_static()
            .and_then(|key| <[u8; 32]>::try_from(key).ok())
            .ok_or_else(|| invalid("peer sent no identity".into()))?;
        let code = code_from(handshake.get_handshake_hash());
        let noise = handshake.into_transport_mode().map_err(noise_error)?;
        Ok(Link {
            stream,
            noise: Arc::new(Mutex::new(noise)),
            peer,
            code,
        })
    }
}

/// Six digits from the handshake hash. Both sides end the handshake with
/// the same hash, which covers both public keys and both ephemeral keys:
/// a machine in the middle runs two handshakes with different keys, so the
/// two PCs would show different codes. The leftover bias of `% 1_000_000`
/// on 32 bits is under 0.03 %, too little to help an attacker guess.
fn code_from(hash: &[u8]) -> u32 {
    let first = hash.first_chunk::<4>().copied().unwrap_or_default();
    u32::from_be_bytes(first) % 1_000_000
}

fn builder(identity: &Identity) -> io::Result<snow::Builder<'_>> {
    let params = NOISE_PARAMS.parse().map_err(noise_error)?;
    snow::Builder::new(params)
        .local_private_key(identity.private())
        .map_err(noise_error)
}

fn write_noise(stream: &mut TcpStream, message: &[u8]) -> io::Result<()> {
    // At most 65 535 bytes (a bulk chunk with its tag): fits in two bytes.
    let len = (message.len() as u16).to_be_bytes();
    stream.write_all(&len)?;
    stream.write_all(message)
}

/// Reads one Noise message into `buf`; longer than `buf` is an error, so
/// the input link keeps refusing anything over `MAX_NOISE_MESSAGE`.
fn read_noise(stream: &mut TcpStream, buf: &mut [u8]) -> io::Result<usize> {
    let mut len = [0u8; 2];
    stream.read_exact(&mut len)?;
    let len = usize::from(u16::from_be_bytes(len));
    if len == 0 || len > buf.len() {
        return Err(invalid(format!(
            "noise message length {len} is out of range"
        )));
    }
    stream.read_exact(&mut buf[..len])?;
    Ok(len)
}

/// snow's errors carry no key material, only what went wrong.
fn noise_error(err: impl std::fmt::Display) -> io::Error {
    invalid(format!("secure link: {err}"))
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::InputEvent;
    use std::net::{SocketAddr, TcpListener};
    use std::thread;

    const KEY: Message = Message::Input(InputEvent::Key {
        code: 30,
        pressed: true,
    });

    fn listener() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a free localhost port");
        let addr = listener
            .local_addr()
            .expect("a bound listener has an address");
        (listener, addr)
    }

    /// A target thread that answers the handshake and collects messages
    /// until the link fails or closes.
    fn target(identity: Identity) -> (SocketAddr, thread::JoinHandle<(Vec<Message>, io::Error)>) {
        let (listener, addr) = listener();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the test connects");
            let mut link = Link::respond(stream, &identity).expect("handshake");
            let mut got = Vec::new();
            loop {
                match link.recv() {
                    Ok(message) => got.push(message),
                    Err(err) => return (got, err),
                }
            }
        });
        (addr, handle)
    }

    /// Relays source → target frame by frame, letting `mangle` change the
    /// frames after the handshake (index 0 and 1 are handshake messages).
    fn proxy(target: SocketAddr, mangle: fn(usize, Vec<u8>) -> Vec<Vec<u8>>) -> SocketAddr {
        let (listener, addr) = listener();
        thread::spawn(move || {
            let (mut source, _) = listener.accept().expect("the test connects");
            let mut target = TcpStream::connect(target).expect("target is listening");
            let (mut back_from, mut back_to) =
                (target.try_clone().unwrap(), source.try_clone().unwrap());
            thread::spawn(move || io::copy(&mut back_from, &mut back_to));
            let mut buf = [0u8; MAX_NOISE_MESSAGE];
            for index in 0.. {
                let Ok(len) = read_noise(&mut source, &mut buf) else {
                    return;
                };
                for frame in mangle(index, buf[..len].to_vec()) {
                    if write_noise(&mut target, &frame).is_err() {
                        return;
                    }
                }
            }
        });
        addr
    }

    #[test]
    fn both_sides_learn_each_others_key_and_messages_cross() {
        let (source_id, target_id) = (Identity::generate().unwrap(), Identity::generate().unwrap());
        let target_key = *target_id.public();
        let (addr, handle) = target(target_id);
        let mut link = Link::initiate(TcpStream::connect(addr).unwrap(), &source_id).unwrap();
        assert_eq!(link.peer_key(), &target_key);
        link.send(&KEY).unwrap();
        link.send(&Message::Ping { id: 9 }).unwrap();
        drop(link);
        let (got, err) = handle.join().unwrap();
        assert_eq!(got, [KEY, Message::Ping { id: 9 }]);
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn both_sides_show_the_same_code_and_each_connection_a_new_one() {
        let (listener, addr) = listener();
        let target = thread::spawn(move || {
            let identity = Identity::generate().unwrap();
            (0..2)
                .map(|_| {
                    let (stream, _) = listener.accept().unwrap();
                    Link::respond(stream, &identity).unwrap().pairing_code()
                })
                .collect::<Vec<_>>()
        });
        let source = Identity::generate().unwrap();
        let first = Link::initiate(TcpStream::connect(addr).unwrap(), &source).unwrap();
        let second = Link::initiate(TcpStream::connect(addr).unwrap(), &source).unwrap();
        let theirs = target.join().unwrap();
        assert_eq!(first.pairing_code(), theirs[0]);
        assert_eq!(second.pairing_code(), theirs[1]);
        assert_ne!(theirs[0], theirs[1], "fresh ephemeral keys, fresh code");
        assert!(first.pairing_code().len() == 7 && first.pairing_code().as_bytes()[3] == b' ');
    }

    #[test]
    fn a_changed_byte_closes_the_link() {
        let (addr, handle) = target(Identity::generate().unwrap());
        let flip = |index, mut frame: Vec<u8>| {
            if index == 3 {
                frame[0] ^= 1;
            }
            vec![frame]
        };
        let addr = proxy(addr, flip);
        let mut link = Link::initiate(
            TcpStream::connect(addr).unwrap(),
            &Identity::generate().unwrap(),
        )
        .unwrap();
        for _ in 0..3 {
            link.send(&KEY).unwrap();
        }
        let (got, err) = handle.join().unwrap();
        assert_eq!(got, [KEY], "only the message before the changed one");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_replayed_frame_closes_the_link() {
        let (addr, handle) = target(Identity::generate().unwrap());
        let replay = |index, frame: Vec<u8>| {
            if index == 2 {
                vec![frame.clone(), frame]
            } else {
                vec![frame]
            }
        };
        let addr = proxy(addr, replay);
        let mut link = Link::initiate(
            TcpStream::connect(addr).unwrap(),
            &Identity::generate().unwrap(),
        )
        .unwrap();
        link.send(&KEY).unwrap();
        let (got, err) = handle.join().unwrap();
        assert_eq!(got, [KEY]);
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }
}
