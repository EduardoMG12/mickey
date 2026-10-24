//! Finding the other PCs on the local network (PRD 0010). Every running
//! Mickey broadcasts a small packet every few seconds, saying its name, the
//! port it listens on and its public key, and keeps a table of the packets
//! it hears. Nothing in it is secret, and nothing is trusted because of it:
//! trust still comes from pairing, which checks the key.
//!
//! A PC that hears another also answers it directly, at most every few
//! seconds: a broadcast can leave by the wrong network adapter (a VPN
//! adapter first in line, found on the owner's Windows PC), and then only
//! one side would hear the other.
//!
//! Each announcement goes to every local network's own broadcast address
//! as well as to 255.255.255.255: Linux sends that one only through the
//! default route's network, so a PC on two networks (Ethernet and Wi-Fi,
//! or a VM bridge) was heard on one only (#217).
//!
//! ```text
//! "MKY1" | port: u16 | public key: 32 bytes | name length: u8 | name
//! ```

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

pub const PORT: u16 = 4243;
const MAGIC: &[u8; 4] = b"MKY1";
const EVERY: Duration = Duration::from_secs(3);
/// A PC not heard from for this long has left the list.
const FORGET_AFTER: Duration = Duration::from_secs(10);
const MAX_NAME: usize = 64;
/// PCs remembered at once: a LAN has a handful; a flood of made-up keys
/// must not grow memory without bound (#186).
const MAX_HEARD: usize = 64;

/// What one PC says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announce {
    pub name: String,
    pub port: u16,
    pub key: [u8; 32],
}

impl Announce {
    pub fn encode(&self) -> Vec<u8> {
        let name = &self.name.as_bytes()[..self.name.len().min(MAX_NAME)];
        let mut packet = MAGIC.to_vec();
        packet.extend_from_slice(&self.port.to_be_bytes());
        packet.extend_from_slice(&self.key);
        packet.push(name.len() as u8);
        packet.extend_from_slice(name);
        packet
    }

    /// Anything from the network may be garbage: `None` for whatever is
    /// not exactly one of our packets.
    pub fn decode(packet: &[u8]) -> Option<Announce> {
        let rest = packet.strip_prefix(MAGIC)?;
        let (port, rest) = rest.split_first_chunk::<2>()?;
        let (key, rest) = rest.split_first_chunk::<32>()?;
        let (&len, name) = rest.split_first()?;
        if name.len() != usize::from(len) || name.len() > MAX_NAME {
            return None;
        }
        let name = std::str::from_utf8(name).ok()?;
        if name.is_empty() || name.contains(['\t', '\n']) {
            return None;
        }
        Some(Announce {
            name: name.to_string(),
            port: u16::from_be_bytes(*port),
            key: *key,
        })
    }
}

/// A PC heard on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    /// Where its Mickey listens: the packet's sender with the port it gave.
    pub address: SocketAddr,
    pub key: [u8; 32],
}

/// Each PC heard, by key, with when it was last heard.
type Heard = HashMap<[u8; 32], (Found, Instant)>;

/// The PCs heard lately, shared between the listener and whoever asks.
#[derive(Clone, Default)]
pub struct Table(Arc<Mutex<Heard>>);

impl Table {
    fn heard(&self, from: IpAddr, announce: Announce, at: Instant) {
        let found = Found {
            name: announce.name,
            address: SocketAddr::new(from, announce.port),
            key: announce.key,
        };
        let mut table = self.0.lock().unwrap_or_else(|p| p.into_inner());
        table.insert(found.key, (found, at));
        if table.len() > MAX_HEARD {
            table.retain(|_, (_, heard)| at.duration_since(*heard) < FORGET_AFTER);
        }
        // Still full of fresh entries: the oldest goes.
        while table.len() > MAX_HEARD {
            let oldest = table
                .iter()
                .min_by_key(|(_, (_, heard))| *heard)
                .map(|(key, _)| *key);
            match oldest {
                Some(key) => table.remove(&key),
                None => break,
            };
        }
    }

    /// The name a PC announced, if it was heard.
    pub fn name_of(&self, key: &[u8; 32]) -> Option<String> {
        let table = self.0.lock().unwrap_or_else(|p| p.into_inner());
        table.get(key).map(|(found, _)| found.name.clone())
    }

    /// The PCs heard in the last `FORGET_AFTER`, by name.
    pub fn list(&self, now: Instant) -> Vec<Found> {
        let mut table = self.0.lock().unwrap_or_else(|p| p.into_inner());
        table.retain(|_, (_, at)| now.duration_since(*at) < FORGET_AFTER);
        let mut found: Vec<Found> = table.values().map(|(found, _)| found.clone()).collect();
        found.sort_by(|a, b| a.name.cmp(&b.name));
        found
    }
}

/// Starts announcing `me` and listening for the others; their packets go
/// into the returned table. Our own packets are left out by key.
pub fn start(me: Announce) -> Result<Table, String> {
    let table = Table::default();
    let listener = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, PORT))
        .map_err(|err| format!("cannot listen for other PCs on UDP {PORT}: {err}"))?;
    let sender = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|err| err.to_string())?;
    sender.set_broadcast(true).map_err(|err| err.to_string())?;

    let packet = me.encode();
    let answer = packet.clone();
    thread::spawn(move || {
        loop {
            // A network that is down now may be up in a moment: keep
            // trying, and look at the networks again every time.
            for to in targets() {
                let _ = sender.send_to(&packet, (to, PORT));
            }
            thread::sleep(EVERY);
        }
    });
    let heard = table.clone();
    thread::spawn(move || {
        let mut buf = [0u8; 128];
        // When each address was last answered, so two PCs answering each
        // other send one packet every `EVERY`, not a storm. By address, not
        // by key: a made-up key per packet must not get an answer each (#186).
        let mut answered: HashMap<IpAddr, Instant> = HashMap::new();
        loop {
            let Ok((len, from)) = listener.recv_from(&mut buf) else {
                continue;
            };
            let Some(announce) = Announce::decode(&buf[..len]) else {
                continue;
            };
            // Our own packets, and anything from outside the local network.
            if announce.key == me.key || !crate::group::is_local(from.ip()) {
                continue;
            }
            let now = Instant::now();
            if answered
                .get(&from.ip())
                .is_none_or(|at| now.duration_since(*at) >= EVERY)
            {
                let _ = listener.send_to(&answer, (from.ip(), PORT));
                answered.insert(from.ip(), now);
                answered.retain(|_, at| now.duration_since(*at) < EVERY);
            }
            heard.heard(from.ip(), announce, now);
        }
    });
    Ok(table)
}

/// Where to announce: each local network's broadcast address, then the
/// limited broadcast as before. Only private networks, the same rule that
/// decides whose packets are heard: a VPN that joins strangers (Radmin,
/// Hamachi) never gets this PC's name and key.
fn targets() -> Vec<Ipv4Addr> {
    let mut all: Vec<Ipv4Addr> = networks()
        .into_iter()
        .filter(|network| crate::group::is_local((*network).into()))
        .collect();
    all.push(Ipv4Addr::BROADCAST);
    all
}

#[cfg(target_os = "linux")]
fn networks() -> Vec<Ipv4Addr> {
    std::fs::read_to_string("/proc/net/route")
        .map(|table| broadcasts(&table))
        .unwrap_or_default()
}

/// Windows: from each adapter's address and mask.
#[cfg(windows)]
fn networks() -> Vec<Ipv4Addr> {
    broadcasts_of(&crate::platform::netif::networks())
}

/// The broadcast address of each address and mask, leaving out loopback
/// and masks that make no network (0, or all ones).
#[cfg_attr(not(windows), allow(dead_code))]
fn broadcasts_of(networks: &[(Ipv4Addr, Ipv4Addr)]) -> Vec<Ipv4Addr> {
    let mut found = Vec::new();
    for &(address, mask) in networks {
        let mask = u32::from(mask);
        if address.is_loopback() || mask == 0 || mask == u32::MAX {
            continue;
        }
        let broadcast = Ipv4Addr::from(u32::from(address) | !mask);
        if !found.contains(&broadcast) {
            found.push(broadcast);
        }
    }
    found
}

/// The broadcast address of each network this PC sits on, from the text
/// of `/proc/net/route`: every route that is up and reaches a network
/// directly, leaving out the default route (mask 0), routes to a single
/// host (mask all ones) and routes through a gateway (another network,
/// whose routers drop broadcasts anyway).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn broadcasts(table: &str) -> Vec<Ipv4Addr> {
    const UP: u32 = 0x1;
    const GATEWAY: u32 = 0x2;
    let mut found = Vec::new();
    for line in table.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let hex = |at: usize| fields.get(at).and_then(|f| u32::from_str_radix(f, 16).ok());
        let (Some(network), Some(flags), Some(mask)) = (hex(1), hex(3), hex(7)) else {
            continue;
        };
        if flags & UP == 0 || flags & GATEWAY != 0 || mask == 0 || mask == u32::MAX {
            continue;
        }
        // The kernel prints an address as its four bytes read as one
        // number in this machine's byte order; `|` and `!` work byte by
        // byte, so the bytes come out right on any machine.
        let broadcast = Ipv4Addr::from((network | !mask).to_ne_bytes());
        if !broadcast.is_loopback() && !found.contains(&broadcast) {
            found.push(broadcast);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_flood_of_made_up_pcs_keeps_the_table_small() {
        let table = Table::default();
        let now = Instant::now();
        for n in 0..1000u32 {
            let mut key = [0; 32];
            key[..4].copy_from_slice(&n.to_be_bytes());
            let from: IpAddr = "192.0.2.9".parse().unwrap();
            table.heard(
                from,
                Announce {
                    name: format!("pc{n}"),
                    port: 4242,
                    key,
                },
                now,
            );
        }
        assert_eq!(table.list(now).len(), MAX_HEARD);
    }

    use super::*;

    fn laptop() -> Announce {
        Announce {
            name: "Notebook".into(),
            port: 4242,
            key: [7; 32],
        }
    }

    #[test]
    fn a_packet_round_trips() {
        assert_eq!(Announce::decode(&laptop().encode()), Some(laptop()));
    }

    #[test]
    fn garbage_is_refused() {
        let good = laptop().encode();
        assert_eq!(
            Announce::decode(&good[..good.len() - 1]),
            None,
            "short name"
        );
        assert_eq!(
            Announce::decode(&[good.as_slice(), b"x"].concat()),
            None,
            "extra byte"
        );
        assert_eq!(Announce::decode(b"MKY2"), None);
        assert_eq!(Announce::decode(&[]), None);
        let tab = Announce {
            name: "a\tb".into(),
            ..laptop()
        };
        assert_eq!(Announce::decode(&tab.encode()), None);
    }

    /// A `/proc/net/route` line, the way the kernel writes it.
    fn route(iface: &str, network: [u8; 4], flags: u32, mask: [u8; 4]) -> String {
        let hex = |bytes: [u8; 4]| format!("{:08X}", u32::from_ne_bytes(bytes));
        format!(
            "{iface}\t{}\t00000000\t{flags:04X}\t0\t0\t100\t{}\t0\t0\t0",
            hex(network),
            hex(mask)
        )
    }

    #[test]
    fn every_network_gets_its_own_broadcast() {
        let table = [
            "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\tMTU\tWindow\tIRTT"
                .to_string(),
            // The default route, through a NAT network: left out.
            route("enp0s2", [0, 0, 0, 0], 0x3, [0, 0, 0, 0]),
            route("enp0s2", [10, 0, 2, 0], 0x1, [255, 255, 255, 0]),
            // The shared LAN, not the default route: the one #217 missed.
            route("enp0s1", [192, 0, 2, 0], 0x1, [255, 255, 255, 0]),
            route("br0", [198, 51, 100, 0], 0x1, [255, 255, 254, 0]),
            // A host route, a route through a gateway and one that is down.
            route("enp0s1", [192, 0, 2, 9], 0x1, [255, 255, 255, 255]),
            route("tun0", [203, 0, 113, 0], 0x3, [255, 255, 255, 0]),
            route("enp0s3", [172, 16, 0, 0], 0x0, [255, 255, 0, 0]),
            // The same network twice, and a line that is not one.
            route("enp0s1", [192, 0, 2, 0], 0x1, [255, 255, 255, 0]),
            "garbage".to_string(),
        ]
        .join("\n");
        assert_eq!(
            broadcasts(&table),
            [
                Ipv4Addr::new(10, 0, 2, 255),
                Ipv4Addr::new(192, 0, 2, 255),
                Ipv4Addr::new(198, 51, 101, 255),
            ]
        );
    }

    #[test]
    fn each_windows_address_gives_its_network_broadcast() {
        let net = |a: [u8; 4], m: [u8; 4]| (Ipv4Addr::from(a), Ipv4Addr::from(m));
        let found = broadcasts_of(&[
            net([192, 0, 2, 10], [255, 255, 255, 0]),
            net([198, 51, 100, 7], [255, 255, 254, 0]),
            net([127, 0, 0, 1], [255, 0, 0, 0]),
            net([203, 0, 113, 9], [255, 255, 255, 255]),
            net([192, 0, 2, 11], [255, 255, 255, 0]),
        ]);
        assert_eq!(
            found,
            [
                Ipv4Addr::new(192, 0, 2, 255),
                Ipv4Addr::new(198, 51, 101, 255)
            ]
        );
    }

    #[test]
    fn the_table_forgets_pcs_gone_quiet() {
        let table = Table::default();
        let start = Instant::now();
        table.heard(IpAddr::from([192, 0, 2, 7]), laptop(), start);
        let found = table.list(start + Duration::from_secs(1));
        assert_eq!(found[0].address, "192.0.2.7:4242".parse().unwrap());
        assert!(table.list(start + FORGET_AFTER).is_empty());
    }
}
