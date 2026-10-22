//! The wire format. TCP delivers a stream of bytes, not messages, so every
//! message is a frame: one length byte, then a body of that many bytes.
//!
//! ```text
//! [len] [type] [payload...]     len = 1 + payload length
//! ```
//!
//! Numbers are big-endian. Every byte from the network is untrusted, so
//! decoding returns an error for anything unexpected and never panics.

use crate::event::InputEvent;
use crate::monitors::Monitor;
use std::fmt;

/// 2: messages travel inside the secure link (PRD 0002).
/// 3: `Monitors` (PRD 0005).
pub const VERSION: u8 = 3;

/// Longest frame: length byte, type byte and an 8-byte payload.
pub const MAX_FRAME: usize = 3 + 16 * MAX_MONITORS;
/// The most monitors one `Monitors` message carries.
pub const MAX_MONITORS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message {
    Hello {
        version: u8,
    },
    Input(InputEvent),
    ReleaseAll,
    Ping {
        id: u32,
    },
    Pong {
        id: u32,
    },
    /// The sender's monitors (PRD 0005); a fixed array keeps `Message`
    /// `Copy` and the per-event path free of allocations.
    Monitors {
        count: u8,
        list: [Monitor; MAX_MONITORS],
    },
}

const HELLO: u8 = 1;
const KEY: u8 = 2;
const BUTTON: u8 = 3;
const MOTION: u8 = 4;
const SCROLL: u8 = 5;
const RELEASE_ALL: u8 = 6;
const PING: u8 = 7;
const PONG: u8 = 8;
const MONITORS: u8 = 9;

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    Empty,
    UnknownType(u8),
    WrongLength { kind: u8, len: usize },
    BadBool(u8),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DecodeError::Empty => write!(f, "empty frame"),
            DecodeError::UnknownType(kind) => write!(f, "unknown message type {kind}"),
            DecodeError::WrongLength { kind, len } => {
                write!(f, "message type {kind} has a {len}-byte payload")
            }
            DecodeError::BadBool(byte) => write!(f, "{byte} is not 0 or 1"),
        }
    }
}

/// Writes `message` as a whole frame into `frame` and returns its length.
/// Uses a caller-owned buffer so the per-event path does not allocate.
pub fn encode(message: &Message, frame: &mut [u8; MAX_FRAME]) -> usize {
    // Byte 0 is the length; it is known only at the end.
    let mut len = 1;
    let mut put = |bytes: &[u8]| {
        frame[len..len + bytes.len()].copy_from_slice(bytes);
        len += bytes.len();
    };
    match *message {
        Message::Hello { version } => put(&[HELLO, version]),
        Message::Input(InputEvent::Key { code, pressed }) => {
            put(&[KEY]);
            put(&code.to_be_bytes());
            put(&[u8::from(pressed)]);
        }
        Message::Input(InputEvent::Button { code, pressed }) => {
            put(&[BUTTON]);
            put(&code.to_be_bytes());
            put(&[u8::from(pressed)]);
        }
        Message::Input(InputEvent::Motion { dx, dy }) => {
            put(&[MOTION]);
            put(&dx.to_be_bytes());
            put(&dy.to_be_bytes());
        }
        Message::Input(InputEvent::Scroll {
            vertical,
            horizontal,
        }) => {
            put(&[SCROLL]);
            put(&vertical.to_be_bytes());
            put(&horizontal.to_be_bytes());
        }
        Message::ReleaseAll => put(&[RELEASE_ALL]),
        Message::Ping { id } => {
            put(&[PING]);
            put(&id.to_be_bytes());
        }
        Message::Pong { id } => {
            put(&[PONG]);
            put(&id.to_be_bytes());
        }
        Message::Monitors { count, list } => {
            put(&[MONITORS, count]);
            for monitor in &list[..usize::from(count).min(MAX_MONITORS)] {
                for value in [monitor.x, monitor.y, monitor.width, monitor.height] {
                    put(&value.to_be_bytes());
                }
            }
        }
    }
    // At most MAX_FRAME (131), so it always fits in the length byte.
    frame[0] = (len - 1) as u8;
    len
}

impl Message {
    /// The `Monitors` message for up to `MAX_MONITORS` of `found`.
    pub fn monitors(found: &[Monitor]) -> Message {
        let mut list = [Monitor::default(); MAX_MONITORS];
        let count = found.len().min(MAX_MONITORS);
        list[..count].copy_from_slice(&found[..count]);
        Message::Monitors {
            count: count as u8,
            list,
        }
    }
}

/// Decodes a frame body: everything after the length byte.
pub fn decode(body: &[u8]) -> Result<Message, DecodeError> {
    let (&kind, payload) = body.split_first().ok_or(DecodeError::Empty)?;
    let message = match kind {
        HELLO => {
            let [version] = exact(kind, payload)?;
            Message::Hello { version }
        }
        KEY => {
            let (code, pressed) = press(kind, payload)?;
            Message::Input(InputEvent::Key { code, pressed })
        }
        BUTTON => {
            let (code, pressed) = press(kind, payload)?;
            Message::Input(InputEvent::Button { code, pressed })
        }
        MOTION => {
            let (dx, dy) = two_i32(kind, payload)?;
            Message::Input(InputEvent::Motion { dx, dy })
        }
        SCROLL => {
            let (vertical, horizontal) = two_i32(kind, payload)?;
            Message::Input(InputEvent::Scroll {
                vertical,
                horizontal,
            })
        }
        RELEASE_ALL => {
            let [] = exact(kind, payload)?;
            Message::ReleaseAll
        }
        PING => Message::Ping {
            id: u32::from_be_bytes(exact(kind, payload)?),
        },
        PONG => Message::Pong {
            id: u32::from_be_bytes(exact(kind, payload)?),
        },
        MONITORS => {
            let (&count, rects) = payload
                .split_first()
                .ok_or(DecodeError::WrongLength { kind, len: 0 })?;
            if usize::from(count) > MAX_MONITORS || rects.len() != 16 * usize::from(count) {
                return Err(DecodeError::WrongLength {
                    kind,
                    len: payload.len(),
                });
            }
            let mut list = [Monitor::default(); MAX_MONITORS];
            let (rects, _) = rects.as_chunks::<16>();
            for (slot, bytes) in list.iter_mut().zip(rects) {
                let at = |i: usize| {
                    i32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]])
                };
                *slot = Monitor {
                    x: at(0),
                    y: at(4),
                    width: at(8),
                    height: at(12),
                };
            }
            Message::Monitors { count, list }
        }
        other => return Err(DecodeError::UnknownType(other)),
    };
    Ok(message)
}

/// The payload as an array of exactly `N` bytes, or a length error.
fn exact<const N: usize>(kind: u8, payload: &[u8]) -> Result<[u8; N], DecodeError> {
    payload.try_into().map_err(|_| DecodeError::WrongLength {
        kind,
        len: payload.len(),
    })
}

fn press(kind: u8, payload: &[u8]) -> Result<(u16, bool), DecodeError> {
    let [hi, lo, pressed] = exact(kind, payload)?;
    let pressed = match pressed {
        0 => false,
        1 => true,
        other => return Err(DecodeError::BadBool(other)),
    };
    Ok((u16::from_be_bytes([hi, lo]), pressed))
}

fn two_i32(kind: u8, payload: &[u8]) -> Result<(i32, i32), DecodeError> {
    let [a0, a1, a2, a3, b0, b1, b2, b3] = exact(kind, payload)?;
    Ok((
        i32::from_be_bytes([a0, a1, a2, a3]),
        i32::from_be_bytes([b0, b1, b2, b3]),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(message: Message) {
        let mut frame = [0u8; MAX_FRAME];
        let len = encode(&message, &mut frame);
        assert_eq!(usize::from(frame[0]), len - 1, "length byte of {message:?}");
        assert_eq!(decode(&frame[1..len]), Ok(message));
    }

    #[test]
    fn every_message_survives_a_round_trip() {
        round_trip(Message::Hello { version: VERSION });
        round_trip(Message::Input(InputEvent::Key {
            code: 30,
            pressed: true,
        }));
        round_trip(Message::Input(InputEvent::Button {
            code: 0x110,
            pressed: false,
        }));
        round_trip(Message::Input(InputEvent::Motion {
            dx: i32::MIN,
            dy: i32::MAX,
        }));
        round_trip(Message::Input(InputEvent::Scroll {
            vertical: -120,
            horizontal: 240,
        }));
        round_trip(Message::ReleaseAll);
        round_trip(Message::Ping { id: 7 });
        round_trip(Message::Pong { id: u32::MAX });
        round_trip(monitors(&[
            Monitor {
                x: 0,
                y: 0,
                width: 2560,
                height: 1080,
            },
            Monitor {
                x: 2560,
                y: -845,
                width: 1080,
                height: 1920,
            },
        ]));
        round_trip(monitors(&[]));
    }

    #[test]
    fn a_monitors_count_that_disagrees_with_the_length_is_refused() {
        assert!(decode(&[MONITORS, 1]).is_err());
        assert!(decode(&[MONITORS, 9]).is_err());
        assert!(decode(&[MONITORS]).is_err());
    }

    #[test]
    fn numbers_are_big_endian() {
        let mut frame = [0u8; MAX_FRAME];
        let message = Message::Input(InputEvent::Key {
            code: 0x0102,
            pressed: true,
        });
        let len = encode(&message, &mut frame);
        assert_eq!(&frame[..len], &[4, KEY, 0x01, 0x02, 1]);
    }

    #[test]
    fn malformed_bodies_are_errors() {
        assert_eq!(decode(&[]), Err(DecodeError::Empty));
        assert_eq!(decode(&[99]), Err(DecodeError::UnknownType(99)));
        assert_eq!(
            decode(&[KEY, 0, 30]),
            Err(DecodeError::WrongLength { kind: KEY, len: 2 })
        );
        assert_eq!(
            decode(&[RELEASE_ALL, 0]),
            Err(DecodeError::WrongLength {
                kind: RELEASE_ALL,
                len: 1
            })
        );
        assert_eq!(decode(&[BUTTON, 1, 16, 2]), Err(DecodeError::BadBool(2)));
    }

    fn monitors(found: &[Monitor]) -> Message {
        Message::monitors(found)
    }
}
