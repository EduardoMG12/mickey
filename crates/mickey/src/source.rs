//! The source side (Linux): capture local input, run the focus state
//! machine, and send input to the focused peer.

use crate::config;
use crate::control::{self, Control, Order, Request, Status};
use crate::event::InputEvent;
use crate::focus::{Action, Focus, Switcher};
use crate::identity::{self, Identity};
use crate::input::Input;
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

const PING_EVERY: Duration = Duration::from_secs(1);
const KEYS_UP_POLL: Duration = Duration::from_millis(10);
const RETRY_EVERY: Duration = Duration::from_secs(5);

/// What wakes the main loop.
enum Wake {
    Input(InputEvent),
    /// The switcher window closed: the picked peer, `None` for this PC or
    /// Esc, or why the window could not run.
    Pick(Result<Option<usize>, String>),
    Control(Request),
}

impl From<InputEvent> for Wake {
    fn from(event: InputEvent) -> Wake {
        Wake::Input(event)
    }
}

impl From<Request> for Wake {
    fn from(request: Request) -> Wake {
        Wake::Control(request)
    }
}

/// Carries out an order from the control socket and returns the answer
/// line. Orders only move the focus.
fn obey(
    order: Order,
    names: &[String],
    online: &[bool],
    switcher: &mut Switcher,
    act: &mut impl FnMut(Action),
) -> String {
    match order {
        Order::Home => {
            switcher.go_home(act);
            "ok: the focus is on this PC".into()
        }
        Order::Switcher if switcher.focus() == Focus::Local => {
            act(Action::OpenSwitcher);
            "ok: showing the switcher".into()
        }
        Order::Switcher => "error: the focus is on another PC; press the hotkey".into(),
        Order::Go(name) => {
            let Some(peer) = names.iter().position(|known| *known == name) else {
                return format!("error: no PC named `{name}` in config.toml");
            };
            if !online[peer] {
                return format!("error: {name} is offline");
            }
            if switcher.focus() != Focus::Local {
                return "error: the focus is on another PC; use the hotkey or `mickey home`".into();
            }
            switcher.go_to(Some(peer), online);
            format!("ok: going to {name} once every key is released")
        }
    }
}

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
    let input = Input::open()?;
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

    // One channel wakes the loop for input, a switcher pick or an order
    // from the control socket, so each is handled the moment it happens.
    let (wake, wakes) = mpsc::channel();
    input.start(wake.clone(), touchpad_speed)?;
    let mut switcher_open = false;
    // The status for bars and commands; without a runtime directory Mickey
    // still works, just without them.
    let control = match Control::start(wake.clone()) {
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
    let mut grabbed = None;
    let mut next_ping = start;
    // Names never change; which peers are online is refreshed every turn.
    let names: Vec<String> = peers.iter().map(|peer| peer.name.clone()).collect();
    let mut online: Vec<bool> = Vec::with_capacity(peers.len());
    loop {
        for (index, link) in back.try_iter() {
            read_pongs(index, &link, &pong_sender, start);
            peers[index].link = Some(link);
            println!("mickey: {} is back online", peers[index].name);
        }
        for (index, round_trip) in pongs.try_iter() {
            peers[index].latency.add_round_trip(round_trip);
        }
        if let Focus::WaitingForKeysUp(index) = switcher.focus()
            && !input.any_key_down()
        {
            grabbed = Some(input.grab()?);
            switcher.grabbed();
            println!("mickey: focus is on {}", peers[index].name);
        }

        // Publish only when the focus or a peer's state changed; comparing
        // first keeps the per-event path free of allocations.
        let focus = match switcher.focus() {
            Focus::Remote(peer) => Some(peer),
            Focus::Local | Focus::WaitingForKeysUp(_) => None,
        };
        let changed = published.as_ref().is_none_or(|last: &Status| {
            last.focus != focus
                || last
                    .peers
                    .iter()
                    .zip(&peers)
                    .any(|((_, up), peer)| *up != peer.link.is_some())
        });
        if changed {
            let status = Status {
                focus,
                peers: peers
                    .iter()
                    .map(|peer| (peer.name.clone(), peer.link.is_some()))
                    .collect(),
            };
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

        // Sleep until something wakes us or the next ping is due; poll often
        // only while waiting for keys to go up, so an idle source wakes once
        // a second.
        let mut wait = next_ping.saturating_duration_since(now);
        if matches!(switcher.focus(), Focus::WaitingForKeysUp(_)) {
            wait = wait.min(KEYS_UP_POLL);
        }
        // Reuses one buffer: no allocation per input event.
        online.clear();
        online.extend(peers.iter().map(|peer| peer.link.is_some()));
        let mut open = false;
        let left = switcher.focus();
        let mut act = |action| match action {
            Action::OpenSwitcher => open = true,
            Action::Send(index, event) => {
                if let Some(link) = &mut peers[index].link
                    && link.send(&Message::Input(event)).is_err()
                {
                    lost.push(index);
                }
            }
            Action::Ungrab => {
                grabbed = None;
                if let Focus::Remote(index) = left {
                    report_latency(&mut peers[index]);
                }
            }
        };
        match wakes.recv_timeout(wait) {
            Ok(Wake::Input(event)) => switcher.handle(event, &online, &mut act),
            Ok(Wake::Pick(pick)) => {
                switcher_open = false;
                match pick {
                    Ok(peer) => switcher.go_to(peer, &online),
                    Err(err) => {
                        // Without the window, behave like the M0 hotkey.
                        eprintln!("mickey: cannot show the switcher: {err}");
                        switcher.go_to(online.iter().position(|&up| up), &online);
                    }
                }
            }
            Ok(Wake::Control(request)) => {
                let answer = obey(request.order, &names, &online, &mut switcher, &mut act);
                let _ = request.reply.send(answer);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        drop(act);
        if switcher.focus() == Focus::Local && left != Focus::Local {
            println!("mickey: focus is back here");
        }
        if open && !switcher_open {
            switcher_open = true;
            open_switcher(&peers, &wake);
        }

        lost.sort_unstable();
        lost.dedup();
        for index in lost {
            peers[index].link = None;
            println!("mickey: lost {}; retrying every 5 s", peers[index].name);
            switcher.peer_lost(index, |_ungrab| {
                grabbed = None;
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
fn open_switcher(peers: &[Peer], wake: &Sender<Wake>) {
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
    let wake = wake.clone();
    thread::spawn(move || {
        let pick = std::env::current_exe()
            .map(|exe| exe.with_file_name(format!("mickey-ui{}", std::env::consts::EXE_SUFFIX)))
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
        let _ = wake.send(Wake::Pick(pick));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run(order: Order, switcher: &mut Switcher, online: &[bool]) -> (String, Vec<Action>) {
        let names = ["Desktop".to_string(), "Windows".to_string()];
        let mut actions = Vec::new();
        let answer = obey(order, &names, online, switcher, &mut |action| {
            actions.push(action)
        });
        (answer, actions)
    }

    #[test]
    fn go_moves_the_focus_to_an_online_peer_only() {
        let mut switcher = Switcher::new();
        let (answer, _) = run(Order::Go("Desktop".into()), &mut switcher, &[false, true]);
        assert_eq!(answer, "error: Desktop is offline");
        let (answer, _) = run(Order::Go("Nope".into()), &mut switcher, &[false, true]);
        assert!(answer.starts_with("error: no PC named"));
        let (answer, _) = run(Order::Go("Windows".into()), &mut switcher, &[false, true]);
        assert!(answer.starts_with("ok"));
        assert_eq!(switcher.focus(), Focus::WaitingForKeysUp(1));
    }

    #[test]
    fn home_and_switcher_from_a_remote_peer() {
        let mut switcher = Switcher::new();
        switcher.go_to(Some(1), &[true, true]);
        switcher.grabbed();
        let (answer, _) = run(Order::Switcher, &mut switcher, &[true, true]);
        assert!(
            answer.starts_with("error"),
            "the window would open on the wrong screen"
        );
        let (answer, actions) = run(Order::Home, &mut switcher, &[true, true]);
        assert!(answer.starts_with("ok"));
        assert_eq!(actions, [Action::Ungrab]);
        assert_eq!(switcher.focus(), Focus::Local);
    }
}
