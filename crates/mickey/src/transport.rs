//! One TCP connection from source to target, carrying protocol frames.

use crate::event::InputEvent;
use crate::held::HeldSet;
use crate::protocol::{self, MAX_FRAME, Message, VERSION};
use std::io::{self, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};

/// Connects to a target and says hello. `TCP_NODELAY` stops the kernel from
/// holding back tiny frames to batch them (Nagle's algorithm), which would
/// add up to tens of milliseconds of latency.
pub fn connect(addr: SocketAddr) -> io::Result<TcpStream> {
    let mut stream = TcpStream::connect(addr)?;
    stream.set_nodelay(true)?;
    send(&mut stream, &Message::Hello { version: VERSION })?;
    Ok(stream)
}

pub fn send(writer: &mut impl Write, message: &Message) -> io::Result<()> {
    let mut frame = [0u8; MAX_FRAME];
    let len = protocol::encode(message, &mut frame);
    writer.write_all(&frame[..len])
}

/// Reads exactly one frame. A clean end of stream is `UnexpectedEof`.
pub fn read_message(reader: &mut impl Read) -> io::Result<Message> {
    let mut len = [0u8; 1];
    reader.read_exact(&mut len)?;
    let len = usize::from(len[0]);
    if len == 0 || len > MAX_FRAME - 1 {
        return Err(invalid(format!("frame length {len} is out of range")));
    }
    let mut body = [0u8; MAX_FRAME - 1];
    reader.read_exact(&mut body[..len])?;
    protocol::decode(&body[..len]).map_err(|err| invalid(err.to_string()))
}

/// Serves one source until it disconnects, passing every input event to
/// `emit`. Whatever is still held when the connection ends, cleanly or not,
/// is released through `emit` before returning.
pub fn serve(stream: &TcpStream, mut emit: impl FnMut(InputEvent)) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let mut reader = BufReader::new(stream);
    let mut writer = stream;

    match read_message(&mut reader)? {
        Message::Hello { version: VERSION } => {}
        Message::Hello { version } => {
            return Err(invalid(format!(
                "peer speaks protocol {version}, this Mickey speaks {VERSION}"
            )));
        }
        _ => return Err(invalid("peer did not start with Hello".into())),
    }

    let mut keys = HeldSet::new();
    let mut buttons = HeldSet::new();
    let result = loop {
        let message = match read_message(&mut reader) {
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
                emit(event);
            }
            Message::ReleaseAll => release_all(&mut keys, &mut buttons, &mut emit),
            Message::Ping { id } => {
                if let Err(err) = send(&mut writer, &Message::Pong { id }) {
                    break Err(err);
                }
            }
            Message::Pong { .. } => {}
            Message::Hello { .. } => break Err(invalid("second Hello".into())),
        }
    };
    release_all(&mut keys, &mut buttons, &mut emit);
    result
}

fn release_all(keys: &mut HeldSet, buttons: &mut HeldSet, emit: &mut impl FnMut(InputEvent)) {
    keys.release_all(|code| {
        emit(InputEvent::Key {
            code,
            pressed: false,
        })
    });
    buttons.release_all(|code| {
        emit(InputEvent::Button {
            code,
            pressed: false,
        })
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

    /// Starts a target with a fake emitter that records what it is given.
    fn fake_target() -> (SocketAddr, thread::JoinHandle<Outcome>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a free localhost port");
        let addr = listener
            .local_addr()
            .expect("a bound listener has an address");
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the test connects");
            let mut emitted = Vec::new();
            let result = serve(&stream, |event| emitted.push(event));
            (result, emitted)
        });
        (addr, handle)
    }

    #[test]
    fn events_cross_and_disconnect_releases_held_keys() {
        let (addr, target) = fake_target();
        let captured = [
            InputEvent::Key {
                code: 30,
                pressed: true,
            },
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
            let mut source = connect(addr).unwrap();
            for event in captured {
                send(&mut source, &Message::Input(event)).unwrap();
            }
        } // `source` is dropped here while key 30 is still held

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
    fn ping_gets_a_pong() {
        let (addr, target) = fake_target();
        let mut source = connect(addr).unwrap();
        send(&mut source, &Message::Ping { id: 42 }).unwrap();
        assert_eq!(read_message(&mut source).unwrap(), Message::Pong { id: 42 });
        drop(source);
        assert!(target.join().unwrap().0.is_ok());
    }

    #[test]
    fn garbage_closes_the_connection_and_still_releases() {
        let (addr, target) = fake_target();
        let mut source = connect(addr).unwrap();
        send(
            &mut source,
            &Message::Input(InputEvent::Key {
                code: 42,
                pressed: true,
            }),
        )
        .unwrap();
        source.write_all(&[2, 99, 0]).unwrap(); // message type 99 does not exist

        let (result, emitted) = target.join().unwrap();
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(
            emitted.last(),
            Some(&InputEvent::Key {
                code: 42,
                pressed: false
            })
        );
    }

    #[test]
    fn wrong_version_is_refused() {
        let (addr, target) = fake_target();
        let mut source = TcpStream::connect(addr).unwrap();
        send(
            &mut source,
            &Message::Hello {
                version: VERSION + 1,
            },
        )
        .unwrap();
        assert!(target.join().unwrap().0.is_err());
    }
}
