//! The connection from source to target: TCP carrying the secure link.

use crate::edges;
use crate::event::InputEvent;
use crate::held::HeldSet;
use crate::identity::Identity;
use crate::monitors::Monitor;
use crate::protocol::{Message, VERSION};
use crate::secure::Link;
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// How long a send may wait for a target that stopped reading. The kernel
/// buffers megabytes first, so this only fires when the target is gone or
/// stuck, never on a slow network.
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// A peer that is off does not answer at all; give up on it quickly so it
/// never delays the others.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Connects to a target and runs the secure handshake. The caller checks
/// the target's key (pairing) before saying `hello`.
/// `TCP_NODELAY` stops the kernel from holding back tiny frames to batch
/// them (Nagle's algorithm), which would add up to tens of milliseconds.
///
/// Sends fail after `WRITE_TIMEOUT` instead of blocking forever, so a
/// target that stops reading (asleep, crashed, malicious) can never keep
/// the source's keyboard grabbed: the error ends `mickey source`, which
/// drops the grabs.
pub fn connect(addr: SocketAddr, identity: &Identity) -> io::Result<Link> {
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)?;
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    Link::initiate(stream, identity)
}

/// Says hello and waits for the target's hello, which it sends only once it
/// trusts us. Until then nothing else is sent.
pub fn hello(link: &mut Link) -> io::Result<()> {
    link.send(&Message::Hello { version: VERSION })?;
    match link.recv()? {
        Message::Hello { version: VERSION } => Ok(()),
        Message::Hello { version } => Err(invalid(format!(
            "target speaks protocol {version}, this Mickey speaks {VERSION}"
        ))),
        _ => Err(invalid("target did not answer with Hello".into())),
    }
}

/// The target's side of the handshake, for one accepted connection. The
/// caller checks the source's key (pairing) before calling `serve`.
pub fn accept(stream: TcpStream, identity: &Identity) -> io::Result<Link> {
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
    Link::respond(stream, identity)
}

/// The data link (PRD 0011) listens on the port after the input link's.
pub fn data_address(input: SocketAddr) -> SocketAddr {
    SocketAddr::new(input.ip(), input.port().wrapping_add(1))
}

/// A bulk payload of megabytes takes longer to write than a key press.
const DATA_WRITE_TIMEOUT: Duration = Duration::from_secs(10);

/// Opens the data link to the target whose input link is at `input`.
pub fn connect_data(input: SocketAddr, identity: &Identity) -> io::Result<Link> {
    let stream = TcpStream::connect_timeout(&data_address(input), CONNECT_TIMEOUT)?;
    stream.set_write_timeout(Some(DATA_WRITE_TIMEOUT))?;
    Link::initiate(stream, identity)
}

/// The target's side of a data link. Only a PC already paired may open
/// one (`trusted` says): a data link never asks to pair.
pub fn accept_data(
    stream: TcpStream,
    identity: &Identity,
    trusted: impl Fn(&[u8; 32]) -> bool,
) -> io::Result<Link> {
    stream.set_write_timeout(Some(DATA_WRITE_TIMEOUT))?;
    let link = Link::respond(stream, identity)?;
    if !trusted(link.peer_key()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "a PC that is not paired asked for a data link",
        ));
    }
    Ok(link)
}

/// What the target does for the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Input(InputEvent),
    /// Put the cursor at this point of the desktop.
    Place {
        x: i32,
        y: i32,
    },
    /// The source's keyboard and mouse now control this PC, or not any
    /// more (PRD 0009).
    Controlled(bool),
    /// The source sits at this point of this PC's desktop (PRD 0005).
    Arranged([i32; 2]),
}

/// Serves one source until it disconnects, passing every input event to
/// `act`. Whatever is still held when the connection ends, cleanly or not,
/// is released through `act` before returning. `monitors` are this PC's,
/// sent to the source after `Hello` so it can lay the PCs out and cross
/// their edges (PRD 0005). `cursor` says where this PC's cursor is, to
/// notice it pushing off the desktop; `None` when the system cannot say.
pub fn serve(
    link: &mut Link,
    monitors: &[Monitor],
    mut act: impl FnMut(Effect),
    cursor: fn() -> Option<(i32, i32)>,
) -> io::Result<()> {
    let first = link.recv().map_err(|err| match err.kind() {
        io::ErrorKind::UnexpectedEof => {
            invalid("the source left before saying hello (it may have refused to pair)".into())
        }
        _ => err,
    })?;
    match first {
        Message::Hello { version: VERSION } => {
            link.send(&Message::Hello { version: VERSION })?;
            link.send(&Message::monitors(monitors))?;
        }
        Message::Hello { version } => {
            return Err(invalid(format!(
                "peer speaks protocol {version}, this Mickey speaks {VERSION}"
            )));
        }
        _ => return Err(invalid("peer did not start with Hello".into())),
    }

    // The last Leave sent, so pushing against a wall sends it once.
    let mut left = None;
    let mut keys = HeldSet::new();
    let mut buttons = HeldSet::new();
    let result = loop {
        let message = match link.recv() {
            Ok(message) => message,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break Ok(()),
            Err(err) => break Err(err),
        };
        match message {
            Message::Input(event) => {
                match event {
                    InputEvent::Key { code, pressed } => keys.set(code, pressed),
                    InputEvent::Button { code, pressed } => buttons.set(code, pressed),
                    InputEvent::Motion { .. } | InputEvent::Scroll { .. } => {}
                }
                act(Effect::Input(event));
                if let InputEvent::Motion { dx, dy } = event {
                    let off = cursor().and_then(|at| edges::beyond(monitors, at, (dx, dy)));
                    if let Some((x, y)) = off
                        && left != off
                        && let Err(err) = link.send(&Message::Leave { x, y })
                    {
                        break Err(err);
                    }
                    left = off;
                }
            }
            Message::ReleaseAll => release_all(&mut keys, &mut buttons, &mut act),
            Message::Ping { id } => {
                if let Err(err) = link.send(&Message::Pong { id }) {
                    break Err(err);
                }
            }
            Message::Enter { x, y } => {
                left = None;
                act(Effect::Place { x, y });
            }
            Message::Focus { here } => act(Effect::Controlled(here)),
            Message::Arrange { x, y } => act(Effect::Arranged([x, y])),
            Message::Pong { .. } | Message::Monitors { .. } | Message::Leave { .. } => {}
            Message::Hello { .. } => break Err(invalid("second Hello".into())),
        }
    };
    release_all(&mut keys, &mut buttons, &mut act);
    result
}

fn release_all(keys: &mut HeldSet, buttons: &mut HeldSet, act: &mut impl FnMut(Effect)) {
    keys.release_all(|code| {
        act(Effect::Input(InputEvent::Key {
            code,
            pressed: false,
        }))
    });
    buttons.release_all(|code| {
        act(Effect::Input(InputEvent::Button {
            code,
            pressed: false,
        }))
    });
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    /// What the fake target returns: how `serve` ended and what it emitted.
    type Outcome = (io::Result<()>, Vec<InputEvent>);

    fn listener() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a free localhost port");
        let addr = listener
            .local_addr()
            .expect("a bound listener has an address");
        (listener, addr)
    }

    /// Starts a target with a fake emitter that records what it is given.
    fn fake_target() -> (SocketAddr, thread::JoinHandle<Outcome>) {
        let (listener, addr) = listener();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the test connects");
            let identity = Identity::generate().unwrap();
            let mut link = accept(stream, &identity).expect("handshake");
            let mut emitted = Vec::new();
            let result = serve(
                &mut link,
                &[],
                |effect| {
                    if let Effect::Input(event) = effect {
                        emitted.push(event);
                    }
                },
                || None,
            );
            (result, emitted)
        });
        (addr, handle)
    }

    /// A source that has said hello and read the target's monitors, which
    /// the target sends right after its own hello.
    fn source(addr: SocketAddr) -> Link {
        let mut link = connect(addr, &Identity::generate().unwrap()).unwrap();
        hello(&mut link).unwrap();
        assert!(matches!(link.recv().unwrap(), Message::Monitors { .. }));
        link
    }

    const KEY_A: InputEvent = InputEvent::Key {
        code: 30,
        pressed: true,
    };

    #[test]
    fn events_cross_and_disconnect_releases_held_keys() {
        let (addr, target) = fake_target();
        let captured = [
            KEY_A,
            InputEvent::Motion { dx: 5, dy: -3 },
            InputEvent::Button {
                code: 0x110,
                pressed: true,
            },
            InputEvent::Button {
                code: 0x110,
                pressed: false,
            },
            InputEvent::Scroll {
                vertical: 120,
                horizontal: 0,
            },
        ];
        {
            let mut link = source(addr);
            for event in captured {
                link.send(&Message::Input(event)).unwrap();
            }
        } // the link is dropped here while key 30 is still held

        let (result, emitted) = target.join().unwrap();
        assert!(result.is_ok());
        let mut expected = captured.to_vec();
        expected.push(InputEvent::Key {
            code: 30,
            pressed: false,
        });
        assert_eq!(emitted, expected);
    }

    #[test]
    fn the_target_places_on_enter_and_says_once_when_pushed_off() {
        let (listener, addr) = listener();
        let target = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut link = accept(stream, &Identity::generate().unwrap()).unwrap();
            let laptop = [Monitor {
                x: 0,
                y: 0,
                width: 1536,
                height: 864,
            }];
            let mut placed = Vec::new();
            // The cursor sits on the laptop's right edge.
            let result = serve(
                &mut link,
                &laptop,
                |effect| {
                    if let Effect::Place { x, y } = effect {
                        placed.push((x, y));
                    }
                },
                || Some((1535, 284)),
            );
            (result, placed)
        });
        let mut link = source(addr);
        link.send(&Message::Enter { x: 1535, y: 284 }).unwrap();
        let push = Message::Input(InputEvent::Motion { dx: 4, dy: 0 });
        link.send(&push).unwrap();
        link.send(&push).unwrap();
        assert_eq!(link.recv().unwrap(), Message::Leave { x: 1536, y: 284 });
        // Only once: the next message is the pong, not a second Leave.
        link.send(&Message::Ping { id: 7 }).unwrap();
        assert_eq!(link.recv().unwrap(), Message::Pong { id: 7 });
        drop(link);
        let (result, placed) = target.join().unwrap();
        assert!(result.is_ok());
        assert_eq!(placed, [(1535, 284)]);
    }

    /// A data link from a source with `source` to a target that trusts the
    /// keys `trusted` says; returns both ends, or the target's error.
    fn data_link(source: Identity, trust: bool) -> (io::Result<Link>, io::Result<Link>) {
        let (listener, addr) = listener();
        let target = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            accept_data(stream, &Identity::generate().unwrap(), |_| trust)
        });
        // `connect_data` dials the port after `addr`; test the pair directly.
        let stream = TcpStream::connect(addr).unwrap();
        let ours = Link::initiate(stream, &source);
        (ours, target.join().unwrap())
    }

    #[test]
    fn a_big_payload_crosses_the_data_link_both_ways() {
        let (ours, theirs) = data_link(Identity::generate().unwrap(), true);
        let (mut ours, mut theirs) = (ours.unwrap(), theirs.unwrap());
        // Three megabytes that are not all alike, so a lost chunk shows.
        let big: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let sender = thread::spawn(move || {
            ours.send_bulk(&big).unwrap();
            (ours, big)
        });
        let got = theirs.recv_bulk().unwrap();
        let (mut ours, big) = sender.join().unwrap();
        assert_eq!(got, big);
        theirs.send_bulk(b"").unwrap();
        assert_eq!(ours.recv_bulk().unwrap(), b"");
        assert!(
            ours.send_bulk(&vec![0; crate::secure::MAX_BULK + 1])
                .is_err()
        );
    }

    #[test]
    fn a_pc_not_paired_gets_no_data_link() {
        let (_, theirs) = data_link(Identity::generate().unwrap(), false);
        assert!(matches!(theirs, Err(err) if err.kind() == io::ErrorKind::PermissionDenied));
    }

    #[test]
    fn the_data_link_is_on_the_next_port() {
        assert_eq!(
            data_address("192.0.2.7:4242".parse().unwrap()),
            "192.0.2.7:4243".parse().unwrap()
        );
    }

    #[test]
    fn ping_gets_a_pong() {
        let (addr, target) = fake_target();
        let mut link = source(addr);
        link.send(&Message::Ping { id: 42 }).unwrap();
        assert_eq!(link.recv().unwrap(), Message::Pong { id: 42 });
        drop(link);
        assert!(target.join().unwrap().0.is_ok());
    }

    #[test]
    fn a_protocol_error_closes_the_link_and_still_releases() {
        let (addr, target) = fake_target();
        let mut link = source(addr);
        link.send(&Message::Input(KEY_A)).unwrap();
        link.send(&Message::Hello { version: VERSION }).unwrap(); // a second Hello

        let (result, emitted) = target.join().unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            emitted,
            [
                KEY_A,
                InputEvent::Key {
                    code: 30,
                    pressed: false
                }
            ]
        );
    }

    #[test]
    fn a_target_that_stops_reading_makes_send_fail() {
        let (listener, addr) = listener();
        // Answers the handshake, then never reads, until the test ends.
        let stuck = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the test connects");
            accept(stream, &Identity::generate().unwrap())
        });
        let mut link = connect(addr, &Identity::generate().unwrap()).unwrap();
        let event = Message::Input(InputEvent::Motion { dx: 1, dy: 1 });
        let failed = (0..10_000_000).find_map(|_| link.send(&event).err());
        let kind = failed
            .expect("send must fail once the buffers are full")
            .kind();
        // Linux reports a send timeout as WouldBlock, Windows as TimedOut.
        assert!(matches!(
            kind,
            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
        ));
        drop(stuck);
    }

    #[test]
    fn wrong_version_is_refused() {
        let (addr, target) = fake_target();
        let stream = TcpStream::connect(addr).unwrap();
        let mut link = Link::initiate(stream, &Identity::generate().unwrap()).unwrap();
        link.send(&Message::Hello {
            version: VERSION + 1,
        })
        .unwrap();
        assert!(target.join().unwrap().0.is_err());
    }
}
