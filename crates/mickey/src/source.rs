//! The source side (Linux): capture local input, run the focus state
//! machine, and send input to the focused peer.

use crate::capture;
use crate::focus::{Action, Focus, Switcher};
use crate::identity;
use crate::latency::{self, Latency};
use crate::protocol::Message;
use crate::transport;
use crate::trust;
use std::net::SocketAddr;

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

/// Captures local input and sends it to `to` while focus is remote.
pub fn run(to: SocketAddr, touchpad_speed: i32) -> Result<(), String> {
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
    if !trust::trust_or_ask(&identity, &mut trusted, &peer_key, to)? {
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
        if matches!(switcher.focus(), Focus::WaitingForKeysUp(_))
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
        if matches!(switcher.focus(), Focus::WaitingForKeysUp(_)) {
            wait = wait.min(KEYS_UP_POLL);
        }
        let event = match events.recv_timeout(wait) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };

        let mut sent = Ok(());
        // ponytail: one peer, always online, until several PCs land (#46).
        switcher.handle(event, &[true], |action| match action {
            Action::Send(_, event) => {
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
