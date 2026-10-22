//! Moving the clipboard between PCs (PRD 0011), over the data link.

use crate::clip_wire::{self, Wire};
use crate::clipboard::{Content, Keeper};
use crate::secure::Link;
use std::collections::HashMap;
use std::io;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Instant;

/// The target's side of a data link, until it closes: `Ask` is answered
/// with what `read` finds copied here, and content sent here is handed to
/// `write`, which puts it in this PC's clipboard.
/// A group map (PRD 0010) goes to `group` instead.
pub fn answer(
    mut link: Link,
    read: impl Fn() -> Option<Content>,
    write: impl Fn(Content),
    group: impl Fn(&str),
) -> io::Result<()> {
    loop {
        let bytes = link.recv_bulk()?;
        if let Some((&crate::group::KIND, text)) = bytes.split_first() {
            group(&String::from_utf8_lossy(text));
            continue;
        }
        let message = clip_wire::decode(&bytes).map_err(io::Error::other)?;
        match message {
            Wire::Ask => {
                let reply = read().map_or(Wire::Nothing, Wire::Content);
                link.send_bulk(&encode_or_nothing(&reply))?;
            }
            Wire::Content(content) => write(content),
            Wire::Files(files) => {
                if let Some(paths) = save(files) {
                    write(Content::Files(paths));
                }
            }
            Wire::Nothing => {}
        }
    }
}

/// `wire` ready to send; what cannot be sent (files over the limit) goes
/// as `Nothing`, with the reason in the log, so the link stays up.
fn encode_or_nothing(wire: &Wire) -> Vec<u8> {
    clip_wire::encode(wire).unwrap_or_else(|err| {
        eprintln!("mickey: clipboard not shared: {err}");
        clip_wire::encode(&Wire::Nothing).expect("Nothing always encodes")
    })
}

/// Saves files from another PC (see `received`); `None`, logged, if not.
fn save(files: Vec<(String, Vec<u8>)>) -> Option<Vec<std::path::PathBuf>> {
    crate::received::save(files)
        .inspect_err(|err| eprintln!("mickey: {err}"))
        .ok()
}

/// The source's side: the clipboard follows the keyboard. When it moves
/// from one PC to another, what is copied where it was is put where it
/// went. On its own thread, so reading, compressing and sending a big
/// image never holds up the keyboard.
#[derive(Clone)]
pub struct Mover(Sender<Job>);

enum Job {
    /// A data link to this peer is open.
    Link(usize, Link),
    /// The group map changed: send it to every peer (PRD 0010).
    Map(Vec<u8>),
    /// The keyboard went from `from` to `to` (`None`: this PC).
    Moved {
        from: Option<usize>,
        to: Option<usize>,
    },
}

impl Mover {
    pub fn start(keeper: Keeper) -> Mover {
        let (jobs, todo) = mpsc::channel();
        thread::spawn(move || {
            let mut links: HashMap<usize, Link> = HashMap::new();
            // The latest map, for links that open after it was shared.
            let mut map: Option<Vec<u8>> = None;
            for job in todo {
                match job {
                    Job::Link(peer, mut link) => {
                        if map.as_ref().is_none_or(|map| link.send_bulk(map).is_ok()) {
                            links.insert(peer, link);
                        }
                    }
                    Job::Map(bytes) => {
                        links.retain(|_, link| link.send_bulk(&bytes).is_ok());
                        map = Some(bytes);
                    }
                    Job::Moved { from, to } => carry(&keeper, &mut links, from, to),
                }
            }
        });
        Mover(jobs)
    }

    pub fn link(&self, peer: usize, link: Link) {
        let _ = self.0.send(Job::Link(peer, link));
    }

    /// Shares the group map with every peer, now and as links open.
    pub fn share_map(&self, text: &str) {
        let _ = self.0.send(Job::Map(crate::group::message(text)));
    }

    pub fn moved(&self, from: Option<usize>, to: Option<usize>) {
        let _ = self.0.send(Job::Moved { from, to });
    }
}

/// Takes what is copied on `from` and puts it on `to`. A link that fails
/// is dropped; the next connection brings a new one.
fn carry(
    keeper: &Keeper,
    links: &mut HashMap<usize, Link>,
    from: Option<usize>,
    to: Option<usize>,
) {
    let started = Instant::now();
    let content = match from {
        None => keeper.read(),
        Some(peer) => {
            let asked = links.get_mut(&peer).map(|link| {
                link.send_bulk(&clip_wire::encode(&Wire::Ask).map_err(io::Error::other)?)?;
                clip_wire::decode(&link.recv_bulk()?).map_err(io::Error::other)
            });
            match asked {
                Some(Ok(Wire::Content(content))) => Some(content),
                Some(Ok(Wire::Files(files))) => save(files).map(Content::Files),
                Some(Ok(_)) | None => None,
                Some(Err(_)) => {
                    links.remove(&peer);
                    None
                }
            }
        }
    };
    let Some(content) = content else {
        return;
    };
    match to {
        None => keeper.write(content),
        Some(peer) => {
            let Some(link) = links.get_mut(&peer) else {
                return;
            };
            let kind = format!("{content:?}");
            let bytes = encode_or_nothing(&Wire::Content(content));
            let sent = link.send_bulk(&bytes).map(|()| bytes.len());
            match sent {
                // The kind and size only: the content may be secret.
                Ok(len) if len > 64 << 10 => println!(
                    "mickey: clipboard ({kind}) sent as {} KB in {} ms",
                    len >> 10,
                    started.elapsed().as_millis()
                ),
                Ok(_) => {}
                Err(_) => {
                    links.remove(&peer);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    #[test]
    fn the_target_answers_and_takes_what_it_is_sent() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let written = Arc::new(Mutex::new(Vec::new()));
        let target_written = Arc::clone(&written);
        let target = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let link = Link::respond(stream, &Identity::generate().unwrap()).unwrap();
            // The target's clipboard holds a text.
            let _ = answer(
                link,
                || Some(Content::Text("from the target".into())),
                |content| target_written.lock().unwrap().push(content),
                |_| {},
            );
        });
        let stream = TcpStream::connect(addr).unwrap();
        let mut link = Link::initiate(stream, &Identity::generate().unwrap()).unwrap();
        let ask = |link: &mut Link, wire: Wire| link.send_bulk(&clip_wire::encode(&wire).unwrap());

        ask(&mut link, Wire::Ask).unwrap();
        let reply = clip_wire::decode(&link.recv_bulk().unwrap()).unwrap();
        assert_eq!(
            reply,
            Wire::Content(Content::Text("from the target".into()))
        );

        ask(
            &mut link,
            Wire::Content(Content::Text("from the source".into())),
        )
        .unwrap();
        drop(link);
        target.join().unwrap();
        assert_eq!(
            *written.lock().unwrap(),
            [Content::Text("from the source".into())]
        );
    }
}
