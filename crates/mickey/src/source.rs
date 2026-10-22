//! The source side (Linux): capture local input, run the focus state
//! machine, and send input to the focused peer.

use crate::capture;
use crate::config;
use crate::control::{self, Control, Status};
use crate::focus::{Action, Focus, Switcher};
use crate::identity::{self, Identity};
use crate::latency::{self, Latency};
use crate::protocol::Message;
use crate::secure::Link;
use crate::transport;
use crate::trust::{self, TrustedPeers};
use std::net::SocketAddr;
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

/// Every keyboard and mouse, with a clear message when we may not read them.
fn open_devices() -> Result<Vec<capture::Found>, String> {
    let devices = capture::find_devices().map_err(|err| match err.kind() {
        std::io::ErrorKind::PermissionDenied => "cannot read /dev/input: permission denied; run \
             `sudo usermod -aG input $USER`, then log in again (or use `newgrp input`)"
            .to_string(),
        _ => format!("cannot list input devices: {err}"),
    })?;
    if devices.is_empty() {
        return Err("no keyboard or mouse found under /dev/input".into());
    }
    for found in &devices {
        let kind = match found.kind {
            capture::Kind::Keyboard => "keyboard",
            capture::Kind::Mouse => "mouse",
            capture::Kind::Touchpad => "touchpad",
        };
        println!("mickey: found {kind}: {}", found.name);
    }
    Ok(devices)
}

const PING_EVERY: Duration = Duration::from_secs(1);
const KEYS_UP_POLL: Duration = Duration::from_millis(10);
const RETRY_EVERY: Duration = Duration::from_secs(5);

/// One configured peer and its link, if it is connected.
struct Peer {
    name: String,
    address: SocketAddr,
    link: Option<Link>,
    latency: Latency,
}

/// Everything the background reconnect threads need, shared read-only.
struct Shared {
    identity: Identity,
    /// Who was trusted at start. Reconnects never ask, so a peer that is
    /// not in here is not retried.
    trusted: TrustedPeers,
}

/// Captures local input and sends it to the focused peer. Every peer gets
/// its own secure link; one that is offline is retried in the background
/// and never blocks the others.
pub fn run(configured: Vec<config::Peer>, touchpad_speed: i32) -> Result<(), String> {
    // Two sources would both capture the keyboard and fight over the grab.
    if control::source_running() {
        return Err("another mickey source is already running".into());
    }
    let devices = open_devices()?;
    let data_dir = identity::data_dir()?;
    let identity = Identity::load_or_create(&data_dir)?;
    let mut trusted = TrustedPeers::load(&data_dir)?;
    let start = Instant::now();
    let (pong_sender, pongs) = mpsc::channel();
    let (back_sender, back) = mpsc::channel();

    // Connect at start, asking to pair where needed (devices are not grabbed
    // yet, so the terminal still gets the keyboard).
    let mut peers = Vec::new();
    for (index, peer) in configured.into_iter().enumerate() {
        let link = match connect_and_pair(&identity, &mut trusted, &peer) {
            Ok(link) => {
                println!("mickey: {} ({}) is connected", peer.name, peer.address);
                read_pongs(index, &link, &pong_sender, start);
                Some(link)
            }
            Err(err) => {
                println!("mickey: {} ({}) is offline: {err}", peer.name, peer.address);
                None
            }
        };
        peers.push(Peer {
            name: peer.name,
            address: peer.address,
            link,
            latency: Latency::new(),
        });
    }
    let shared = Arc::new(Shared { identity, trusted });
    for (index, peer) in peers.iter().enumerate() {
        if peer.link.is_none() {
            reconnect(index, peer.address, &shared, &back_sender);
        }
    }
    println!("mickey: AltGr + Right Shift switches focus");

    let (sender, events) = mpsc::channel();
    capture::spawn_readers(&devices, sender, touchpad_speed);
    let (pick_sender, picks) = mpsc::channel();
    let mut switcher_open = false;
    // The status for bars and commands; without a runtime directory Mickey
    // still works, just without them.
    let control = match Control::start() {
        Ok(control) => Some(control),
        Err(err) => {
            eprintln!("mickey: no control socket: {err}");
            None
        }
    };
    let mut published = None;
    let mut switcher = Switcher::new();
    // Dropping a `Grab` gives its device back, on every way out of here,
    // including `?` and panics.
    let mut grabs = Vec::new();
    let mut next_ping = start;
    loop {
        for (index, link) in back.try_iter() {
            read_pongs(index, &link, &pong_sender, start);
            peers[index].link = Some(link);
            println!("mickey: {} is back online", peers[index].name);
        }
        for (index, round_trip) in pongs.try_iter() {
            peers[index].latency.add_round_trip(round_trip);
        }
        for pick in picks.try_iter() {
            switcher_open = false;
            let online: Vec<bool> = peers.iter().map(|peer| peer.link.is_some()).collect();
            match pick {
                Ok(peer) => switcher.go_to(peer, &online),
                Err(err) => {
                    // Without the window, behave like the M0 hotkey.
                    eprintln!("mickey: cannot show the switcher: {err}");
                    switcher.go_to(online.iter().position(|&up| up), &online);
                }
            }
        }

        if let Focus::WaitingForKeysUp(index) = switcher.focus()
            && !devices
                .iter()
                .any(|found| found.device.any_key_down().unwrap_or(true))
        {
            grabs = devices
                .iter()
                .map(|found| found.device.grab())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| format!("cannot grab the input devices: {err}"))?;
            switcher.grabbed();
            println!("mickey: focus is on {}", peers[index].name);
        }

        let status = Status {
            focus: match switcher.focus() {
                Focus::Remote(peer) => Some(peer),
                Focus::Local | Focus::WaitingForKeysUp(_) => None,
            },
            peers: peers
                .iter()
                .map(|peer| (peer.name.clone(), peer.link.is_some()))
                .collect(),
        };
        if published.as_ref() != Some(&status) {
            if let Some(control) = &control {
                control.publish(status.clone());
            }
            published = Some(status);
        }

        let mut lost = Vec::new();
        let now = Instant::now();
        if now >= next_ping {
            let ping = Message::Ping {
                id: latency::ping_id(start),
            };
            for (index, peer) in peers.iter_mut().enumerate() {
                if let Some(link) = &mut peer.link
                    && link.send(&ping).is_err()
                {
                    lost.push(index);
                }
            }
            next_ping = now + PING_EVERY;
        }

        // Sleep until there is input or the next ping is due; poll often only
        // while waiting for keys to go up, so an idle source wakes once a second.
        let mut wait = next_ping.saturating_duration_since(now);
        if switcher_open || matches!(switcher.focus(), Focus::WaitingForKeysUp(_)) {
            wait = wait.min(KEYS_UP_POLL);
        }
        match events.recv_timeout(wait) {
            Ok(event) => {
                let online: Vec<bool> = peers.iter().map(|peer| peer.link.is_some()).collect();
                let left = switcher.focus();
                let mut open = false;
                switcher.handle(event, &online, |action| match action {
                    Action::OpenSwitcher => open = true,
                    Action::Send(index, event) => {
                        if let Some(link) = &mut peers[index].link
                            && link.send(&Message::Input(event)).is_err()
                        {
                            lost.push(index);
                        }
                    }
                    Action::Ungrab => {
                        grabs.clear();
                        if let Focus::Remote(index) = left {
                            report_latency(&mut peers[index]);
                        }
                    }
                });
                if switcher.focus() == Focus::Local && left != Focus::Local {
                    println!("mickey: focus is back here");
                }
                if open && !switcher_open {
                    switcher_open = true;
                    open_switcher(&peers, &pick_sender);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        lost.sort_unstable();
        lost.dedup();
        for index in lost {
            peers[index].link = None;
            println!("mickey: lost {}; retrying every 5 s", peers[index].name);
            switcher.peer_lost(index, |_ungrab| {
                grabs.clear();
                println!("mickey: focus is back here");
            });
            reconnect(index, peers[index].address, &shared, &back_sender);
        }
    }
    Err("every input device is gone".into())
}

/// Connects, asks to pair if the key is unknown, and waits for the peer to
/// accept us.
fn connect_and_pair(
    identity: &Identity,
    trusted: &mut TrustedPeers,
    peer: &config::Peer,
) -> Result<Link, String> {
    let to = peer.address;
    let mut link = transport::connect(to, identity).map_err(|err| err.to_string())?;
    let peer_key = *link.peer_key();
    if !trust::trust_or_ask(identity, trusted, &peer_key, to)? {
        return Err("not paired; nothing was sent".into());
    }
    println!("mickey: waiting for {} to accept this PC", peer.name);
    transport::hello(&mut link).map_err(|err| match err.kind() {
        std::io::ErrorKind::UnexpectedEof => "it refused to pair with this PC".to_string(),
        _ => err.to_string(),
    })?;
    Ok(link)
}

/// Retries a trusted peer in the background until it answers, then hands
/// its link to the main loop. Never asks to pair: a peer whose key is not
/// trusted is left alone.
fn reconnect(index: usize, to: SocketAddr, shared: &Arc<Shared>, back: &Sender<(usize, Link)>) {
    let (shared, back) = (Arc::clone(shared), back.clone());
    thread::spawn(move || {
        loop {
            thread::sleep(RETRY_EVERY);
            let Ok(mut link) = transport::connect(to, &shared.identity) else {
                continue;
            };
            if !shared.trusted.is_trusted(link.peer_key()) {
                println!("mickey: {to} has a key this PC never paired with; restart to pair");
                return;
            }
            if transport::hello(&mut link).is_ok() {
                let _ = back.send((index, link));
                return;
            }
        }
    });
}

/// Shows `mickey-ui switcher` (installed next to this program) on its own
/// thread, and sends the owner's pick when the window closes: `Some(peer)`,
/// `None` for this PC or Esc, or an error if the window could not run.
/// The GUI is a separate program so this one stays small (PRD 0003).
fn open_switcher(peers: &[Peer], picks: &Sender<Result<Option<usize>, String>>) {
    let mut args = vec![
        "switcher".to_string(),
        "0".to_string(),
        "online:This PC".to_string(),
    ];
    for peer in peers {
        let state = if peer.link.is_some() {
            "online"
        } else {
            "offline"
        };
        args.push(format!("{state}:{}", peer.name));
    }
    let picks = picks.clone();
    thread::spawn(move || {
        let pick = std::env::current_exe()
            .map(|exe| exe.with_file_name("mickey-ui"))
            .and_then(|ui| Command::new(ui).args(&args).output())
            .map_err(|err| format!("mickey-ui: {err}"))
            .and_then(|output| match output.status.code() {
                // The window prints the card index; card 0 is this PC.
                Some(0) => String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse::<usize>()
                    .map(|card| card.checked_sub(1))
                    .map_err(|err| format!("mickey-ui answered badly: {err}")),
                Some(1) => Ok(None),
                _ => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
            });
        let _ = picks.send(pick);
    });
}

/// Pongs come back on their own thread, as round-trip times per peer.
fn read_pongs(index: usize, link: &Link, pongs: &Sender<(usize, u32)>, start: Instant) {
    let (Ok(mut reader), pongs) = (link.try_clone(), pongs.clone()) else {
        return;
    };
    thread::spawn(move || {
        while let Ok(message) = reader.recv() {
            if let Message::Pong { id } = message
                && pongs.send((index, latency::round_trip(start, id))).is_err()
            {
                return;
            }
        }
    });
}

fn report_latency(peer: &mut Peer) {
    if let Some((p50, p99)) = peer.latency.percentiles() {
        println!(
            "mickey: one-way latency to {} over {} pings: p50 {p50} µs, p99 {p99} µs",
            peer.name,
            peer.latency.len()
        );
    }
}
