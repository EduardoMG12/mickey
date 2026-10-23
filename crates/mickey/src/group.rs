//! The group map (PRD 0005, 0010): every PC a running Mickey knows, so a
//! PC paired with one member, or moved in one PC's Layout, is known to all.
//! The owner decided that a PC the group introduces is trusted (a member
//! already trusted vouches for it).
//!
//! A map is a sender's view: positions are in its desktop's coordinates,
//! and it is in the map itself at 0,0. One line per PC, tab-separated:
//!
//! ```text
//! <public key, 64 hex> <name> <ip:port, or :port for the sender> <x,y or ->
//! ```

use std::net::{IpAddr, SocketAddr};

/// The first byte of a group map on the data link; the clipboard's kinds
/// are below 16.
pub const KIND: u8 = 16;

/// The first byte of the shared hotkeys on the data link: the `[hotkeys]`
/// lines of the PC where the owner saved them.
pub const HOTKEYS: u8 = 17;

/// The first byte of "show the switcher here": the focused card's id, then
/// one card per line (`<id>=online:<name>` or `<id>=offline:<name>`), in
/// the order to show them.
pub const SWITCHER: u8 = 18;
/// The same cards, to show only: a copy on the PCs not in use.
pub const MIRROR: u8 = 19;
/// Close the switcher (or its copy) shown for the source: a pick was made.
pub const CLOSE: u8 = 20;

/// Whether `ip` is on a local network: private IPv4 ranges, link-local,
/// the 100.64.0.0/10 range VPNs such as Tailscale use, IPv6 unique-local
/// and link-local. Never loopback or a public address.
pub fn is_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            ip.is_private() || ip.is_link_local() || (a == 100 && (64..128).contains(&b))
        }
        std::net::IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// A map ready for the data link.
pub fn message(text: &str) -> Vec<u8> {
    tagged(KIND, text)
}

/// `text` behind its kind byte, for the data link.
pub fn tagged(kind: u8, text: &str) -> Vec<u8> {
    [&[kind], text.as_bytes()].concat()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub key: [u8; 32],
    pub name: String,
    /// Where its Mickey listens; for the sender only the port is sent (the
    /// receiver knows the IP it came from).
    pub address: SocketAddr,
    /// Where its point 0,0 sits on the sender's desktop, if placed.
    pub position: Option<[i32; 2]>,
}

fn hex(key: &[u8; 32]) -> String {
    key.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    let mut key = [0u8; 32];
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(key)
}

/// `sender` first (its address goes as `:port`), then the others.
pub fn encode(sender: &Member, others: &[Member]) -> String {
    let line = |member: &Member, address: String| {
        let position = member
            .position
            .map_or("-".to_string(), |[x, y]| format!("{x},{y}"));
        format!(
            "{}\t{}\t{address}\t{position}",
            hex(&member.key),
            member.name
        )
    };
    let mut lines = vec![line(sender, format!(":{}", sender.address.port()))];
    lines.extend(
        others
            .iter()
            .map(|member| line(member, member.address.to_string())),
    );
    lines.join("\n")
}

/// Reads a map that came from `from`. Anything from the network may be
/// garbage: a bad line makes the whole map an error.
pub fn decode(text: &str, from: IpAddr) -> Result<Vec<Member>, String> {
    text.lines()
        .map(|line| {
            let bad = || format!("a bad group map line: {line:?}");
            let [key, name, address, position] = line.split('\t').collect::<Vec<_>>()[..] else {
                return Err(bad());
            };
            let key = unhex(key).ok_or_else(bad)?;
            if name.is_empty() || name.len() > 64 || name.contains(['\n', '\r']) {
                return Err(bad());
            }
            let address = match address.strip_prefix(':') {
                Some(port) => SocketAddr::new(from, port.parse().map_err(|_| bad())?),
                None => address.parse().map_err(|_| bad())?,
            };
            let position = match position {
                "-" => None,
                xy => {
                    let (x, y) = xy.split_once(',').ok_or_else(bad)?;
                    Some([x.parse().map_err(|_| bad())?, y.parse().map_err(|_| bad())?])
                }
            };
            Ok(Member {
                key,
                name: name.to_string(),
                address,
                position,
            })
        })
        .collect()
}

/// The members other than `me`, with positions moved from the sender's
/// desktop to mine: if I sit at `m` on the sender's, a PC at `p` there is
/// at `p - m` here. Without my place there, nobody's place can be moved.
pub fn as_seen_by(members: &[Member], me: &[u8; 32]) -> Vec<Member> {
    let mine = members
        .iter()
        .find(|member| &member.key == me)
        .and_then(|member| member.position);
    members
        .iter()
        .filter(|member| &member.key != me)
        .map(|member| Member {
            position: match (member.position, mine) {
                (Some([x, y]), Some([mx, my])) => Some([x - mx, y - my]),
                _ => None,
            },
            ..member.clone()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_local_addresses_may_join() {
        for local in [
            "192.168.0.4",
            "10.1.2.3",
            "172.16.0.9",
            "169.254.1.1",
            "100.100.1.1",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(is_local(local.parse().unwrap()), "{local}");
        }
        for outside in [
            "8.8.8.8",
            "127.0.0.1",
            "100.128.0.1",
            "::1",
            "2001:db8::1",
            "0.0.0.0",
        ] {
            assert!(!is_local(outside.parse().unwrap()), "{outside}");
        }
    }

    use super::*;

    fn member(byte: u8, name: &str, address: &str, position: Option<[i32; 2]>) -> Member {
        Member {
            key: [byte; 32],
            name: name.into(),
            address: address.parse().unwrap(),
            position,
        }
    }

    /// The owner's group as the main Windows PC sees it.
    fn windows_view() -> (Member, Vec<Member>) {
        (
            member(1, "Windows", "0.0.0.0:4242", Some([0, 0])),
            vec![
                member(2, "Notebook", "192.0.2.74:4242", Some([-1536, 216])),
                member(3, "Notebook 2", "192.0.2.64:4242", Some([3640, -845])),
            ],
        )
    }

    #[test]
    fn a_map_round_trips_and_the_sender_gets_its_ip() {
        let (sender, others) = windows_view();
        let read = decode(&encode(&sender, &others), "192.0.2.7".parse().unwrap()).unwrap();
        assert_eq!(read[0].address, "192.0.2.7:4242".parse().unwrap());
        assert_eq!(read[1..], others[..]);
    }

    #[test]
    fn the_laptop_sees_everyone_from_where_it_sits() {
        let (sender, others) = windows_view();
        let map = decode(&encode(&sender, &others), "192.0.2.7".parse().unwrap()).unwrap();
        let seen = as_seen_by(&map, &[2; 32]);
        // Windows is the mirror of the laptop's place on it.
        assert_eq!(seen[0].name, "Windows");
        assert_eq!(seen[0].position, Some([1536, -216]));
        // Notebook 2, right of the sideways monitor on Windows.
        assert_eq!(seen[1].position, Some([3640 + 1536, -845 - 216]));
    }

    #[test]
    fn garbage_is_an_error_and_unplaced_stays_unplaced() {
        let ip = "192.0.2.7".parse().unwrap();
        assert!(decode("nonsense", ip).is_err());
        assert!(decode(&format!("{}\tA\t:x\t-", "00".repeat(32)), ip).is_err());
        let (sender, mut others) = windows_view();
        others[0].position = None;
        let map = decode(&encode(&sender, &others), ip).unwrap();
        assert_eq!(
            as_seen_by(&map, &[3; 32])[1].position,
            None,
            "the laptop has no place"
        );
    }
}
