//! Mickey: one keyboard and mouse shared across computers on the LAN.

#[cfg(target_os = "linux")]
mod capture;
mod cli;
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
// ponytail: used only by tests until source and target switch to it (#36).
#[allow(dead_code)]
mod secure;
#[allow(dead_code)]
mod transport;

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
        Command::Source { to } => {
            warn_unencrypted();
            source(to)
        }
        Command::Target { listen } => {
            warn_unencrypted();
            target(listen)
        }
    }
}

fn warn_unencrypted() {
    eprintln!("mickey: WARNING: traffic is not encrypted; use only on a trusted network");
    eprintln!("mickey: do not type passwords through Mickey until encryption lands (M1a)");
}

/// Serves one source at a time, forever, injecting what it sends.
#[cfg(windows)]
fn target(listen: SocketAddr) -> Result<(), String> {
    use std::io::ErrorKind;
    use std::net::TcpListener;

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
        println!("mickey: source {peer} connected");
        let mut reported = false;
        let result = transport::serve(&stream, |event| {
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
fn source(to: SocketAddr) -> Result<(), String> {
    use focus::{Action, Focus, Switcher};
    use latency::Latency;
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::time::{Duration, Instant};

    const PING_EVERY: Duration = Duration::from_secs(1);
    const KEYS_UP_POLL: Duration = Duration::from_millis(10);

    let devices = open_devices()?;
    let mut stream =
        transport::connect(to).map_err(|err| format!("cannot connect to {to}: {err}"))?;
    println!("mickey: connected to {to}; AltGr + Right Shift switches focus");

    // Pongs come back on their own thread, as round-trip times.
    let start = Instant::now();
    let mut reader = stream
        .try_clone()
        .map_err(|err| format!("cannot read from {to}: {err}"))?;
    let (pong_sender, pongs) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(message) = transport::read_message(&mut reader) {
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
    capture::spawn_readers(&devices, sender);
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
            transport::send(&mut stream, &ping)
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
                    sent = transport::send(&mut stream, &Message::Input(event));
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
fn source(_to: SocketAddr) -> Result<(), String> {
    Err("in M0 the source runs only on Linux (Windows sources come in M2)".into())
}
