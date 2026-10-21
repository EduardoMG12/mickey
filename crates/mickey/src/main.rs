//! Mickey: one keyboard and mouse shared across computers on the LAN.

#[cfg(target_os = "linux")]
mod capture;
mod cli;
mod config;
#[cfg(windows)]
mod emit;
mod event;
#[cfg(target_os = "linux")]
mod focus;
mod held;
#[cfg(target_os = "linux")]
mod hotkey;
mod identity;
#[cfg(target_os = "linux")]
mod latency;
// The source never translates keys for Windows; only the target does.
#[cfg_attr(not(windows), allow(dead_code))]
mod keymap;
mod platform;
// ponytail: in M0 Linux is only a source and Windows only a target, so each
// build uses half of these. Both halves run on Linux in M1a (`uinput`).
#[allow(dead_code)]
mod protocol;
mod secure;
#[allow(dead_code)]
mod transport;
mod trust;

use cli::Command;
#[cfg(target_os = "linux")]
use protocol::Message;
use std::net::SocketAddr;

fn main() {
    let result = std::env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map_err(|arg| format!("argument is not valid UTF-8: {}", arg.display()))
        })
        .collect::<Result<Vec<String>, String>>()
        .and_then(|args| cli::parse(&args))
        .and_then(run);
    if let Err(err) = result {
        eprintln!("mickey: {err}");
        std::process::exit(1);
    }
}

fn run(command: Command) -> Result<(), String> {
    match command {
        Command::Help => {
            print!("{}", cli::USAGE);
            Ok(())
        }
        Command::Id => {
            let identity = identity::Identity::load_or_create(&identity::data_dir()?)?;
            println!("{}", identity.fingerprint());
            Ok(())
        }
        Command::Forget { fingerprint } => {
            let mut trusted = trust::TrustedPeers::load(&identity::data_dir()?)?;
            if trusted.forget(&fingerprint)? {
                println!("mickey: forgot {fingerprint}; it must pair again to connect");
                Ok(())
            } else {
                Err(format!("no trusted peer has the fingerprint {fingerprint}"))
            }
        }
        Command::Source { to } => {
            let config = config::Config::load()?;
            let touchpad_speed = config
                .as_ref()
                .map_or(config::Touchpad::default().speed, |config| {
                    config.touchpad.speed
                });
            // ponytail: only the first peer until several PCs land (#46).
            let to = match (to, config.as_ref().and_then(|config| config.peers.first())) {
                (Some(to), _) => to,
                (None, Some(peer)) => peer.address,
                (None, None) => {
                    return Err(format!(
                        "no peer to connect to: pass --to <ip:port>, or list peers in {}, e.g.\n\n{}",
                        config::path()?.display(),
                        config::EXAMPLE
                    ));
                }
            };
            source(to, touchpad_speed)
        }
        Command::Target { listen } => target(listen),
    }
}

/// Whether to talk to the peer whose key is `peer_key`. A trusted key is
/// accepted at once; an unknown one only after the person at this machine
/// compares fingerprints and types `y`. Anything else (including no
/// terminal to ask on) refuses.
fn trust_peer(
    identity: &identity::Identity,
    trusted: &mut trust::TrustedPeers,
    peer_key: &[u8; 32],
    address: SocketAddr,
) -> Result<bool, String> {
    if trusted.is_trusted(peer_key) {
        return Ok(true);
    }
    println!("mickey: new peer at {address}");
    println!("mickey:   this PC:   {}", identity.fingerprint());
    println!("mickey:   other PC:  {}", identity::fingerprint(peer_key));
    println!("mickey: pair with it? Check that \"other PC\" here matches \"this PC\" there.");
    print!("mickey: type y to pair, anything else to refuse: ");
    std::io::Write::flush(&mut std::io::stdout()).map_err(|err| err.to_string())?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|err| format!("cannot read the answer: {err}"))?;
    if !matches!(answer.trim(), "y" | "Y" | "yes") {
        return Ok(false);
    }
    trusted.add(*peer_key, &address.to_string())?;
    println!("mickey: paired; it will connect without asking from now on");
    Ok(true)
}

/// Serves one source at a time, forever, injecting what it sends.
#[cfg(windows)]
fn target(listen: SocketAddr) -> Result<(), String> {
    use std::io::ErrorKind;
    use std::net::TcpListener;

    let data_dir = identity::data_dir()?;
    let identity = identity::Identity::load_or_create(&data_dir)?;
    let mut trusted = trust::TrustedPeers::load(&data_dir)?;
    let listener = TcpListener::bind(listen).map_err(|err| match err.kind() {
        ErrorKind::AddrInUse => format!("{listen} is already in use; is another Mickey running?"),
        ErrorKind::AddrNotAvailable => format!("{listen} is not an address of this machine"),
        _ => format!("cannot listen on {listen}: {err}"),
    })?;
    println!("mickey: listening on {listen}");
    loop {
        let (stream, peer) = match listener.accept() {
            Ok(connection) => connection,
            Err(err) => {
                eprintln!("mickey: accept failed: {err}");
                continue;
            }
        };
        let mut link = match transport::accept(stream, &identity) {
            Ok(link) => link,
            Err(err) => {
                eprintln!("mickey: {peer} failed the secure handshake: {err}");
                continue;
            }
        };
        let peer_key = *link.peer_key();
        match trust_peer(&identity, &mut trusted, &peer_key, peer) {
            Ok(true) => println!("mickey: source {peer} connected"),
            Ok(false) => {
                println!("mickey: refused {peer}");
                continue;
            }
            Err(err) => {
                eprintln!("mickey: {err}");
                continue;
            }
        }
        let mut reported = false;
        let result = transport::serve(&mut link, |event| {
            if let Err(err) = emit::emit(event)
                && !reported
            {
                eprintln!("mickey: input injection failed: {err} (not reported again)");
                reported = true;
            }
        });
        match result {
            Ok(()) => println!("mickey: source {peer} disconnected"),
            Err(err) => eprintln!("mickey: source {peer} dropped: {err}"),
        }
    }
}

#[cfg(not(windows))]
fn target(_listen: SocketAddr) -> Result<(), String> {
    Err("in M0 the target runs only on Windows (Linux targets come in M1a)".into())
}

/// Every keyboard and mouse, with a clear message when we may not read them.
#[cfg(target_os = "linux")]
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

/// Captures local input and sends it to `to` while focus is remote.
#[cfg(target_os = "linux")]
fn source(to: SocketAddr, touchpad_speed: i32) -> Result<(), String> {
    use focus::{Action, Focus, Switcher};
    use latency::Latency;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::{Duration, Instant};

    const PING_EVERY: Duration = Duration::from_secs(1);
    const KEYS_UP_POLL: Duration = Duration::from_millis(10);

    let devices = open_devices()?;
    let data_dir = identity::data_dir()?;
    let identity = identity::Identity::load_or_create(&data_dir)?;
    let mut trusted = trust::TrustedPeers::load(&data_dir)?;
    let mut link = transport::connect(to, &identity)
        .map_err(|err| format!("cannot connect to {to}: {err}"))?;
    let peer_key = *link.peer_key();
    if !trust_peer(&identity, &mut trusted, &peer_key, to)? {
        return Err(format!("not paired with {to}; nothing was sent"));
    }
    println!("mickey: waiting for {to} to accept this PC");
    transport::hello(&mut link).map_err(|err| match err.kind() {
        std::io::ErrorKind::UnexpectedEof => format!("{to} refused to pair with this PC"),
        _ => format!("lost the connection to {to}: {err}"),
    })?;
    println!("mickey: connected to {to}; AltGr + Right Shift switches focus");

    // Pongs come back on their own thread, as round-trip times.
    let start = Instant::now();
    let mut reader = link
        .try_clone()
        .map_err(|err| format!("cannot read from {to}: {err}"))?;
    let (pong_sender, pongs) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(message) = reader.recv() {
            if let Message::Pong { id } = message
                && pong_sender.send(latency::round_trip(start, id)).is_err()
            {
                return;
            }
        }
    });
    let mut latency = Latency::new();
    let mut next_ping = start;

    let (sender, events) = mpsc::channel();
    capture::spawn_readers(&devices, sender, touchpad_speed);
    let mut switcher = Switcher::new();
    // Dropping a `Grab` gives its device back, on every way out of here,
    // including `?` and panics.
    let mut grabs = Vec::new();
    loop {
        if switcher.focus() == Focus::WaitingForKeysUp
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
            println!("mickey: focus is on {to}");
        }

        let now = Instant::now();
        if now >= next_ping {
            let ping = Message::Ping {
                id: latency::ping_id(start),
            };
            link.send(&ping)
                .map_err(|err| format!("lost the connection to {to}: {err}"))?;
            next_ping = now + PING_EVERY;
        }

        // Sleep until there is input or the next ping is due; poll often only
        // while waiting for keys to go up, so an idle source wakes once a second.
        let mut wait = next_ping.saturating_duration_since(now);
        if switcher.focus() == Focus::WaitingForKeysUp {
            wait = wait.min(KEYS_UP_POLL);
        }
        let event = match events.recv_timeout(wait) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        let mut sent = Ok(());
        switcher.handle(event, |action| match action {
            Action::Send(event) => {
                if sent.is_ok() {
                    sent = link.send(&Message::Input(event));
                }
            }
            Action::Ungrab => {
                grabs.clear();
                println!("mickey: focus is back here");
                for round_trip in pongs.try_iter() {
                    latency.add_round_trip(round_trip);
                }
                if let Some((p50, p99)) = latency.percentiles() {
                    println!(
                        "mickey: one-way latency over {} pings: p50 {p50} µs, p99 {p99} µs",
                        latency.len()
                    );
                }
            }
        });
        sent.map_err(|err| format!("lost the connection to {to}: {err}"))?;
    }
    Err("every input device is gone".into())
}

#[cfg(not(target_os = "linux"))]
fn source(_to: SocketAddr, _touchpad_speed: i32) -> Result<(), String> {
    Err("in M0 the source runs only on Linux (Windows sources come in M2)".into())
}
