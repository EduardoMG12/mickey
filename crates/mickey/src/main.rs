//! Mickey: one keyboard and mouse shared across computers on the LAN.

#[cfg(target_os = "linux")]
mod capture;
mod cli;
#[cfg(windows)]
mod emit;
// ponytail: on Linux these are used only by tests until the source is wired
// (M0 steps 6 and 7, #13 and #14).
#[allow(dead_code)]
mod event;
#[allow(dead_code)]
mod held;
#[allow(dead_code)]
mod keymap;
mod platform;
#[allow(dead_code)]
mod protocol;
#[allow(dead_code)]
mod transport;

use cli::Command;
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
        Command::Source { to } => {
            warn_unencrypted();
            Err(format!(
                "sending to {to} is not implemented yet (M0 step 3)"
            ))
        }
        Command::Target { listen } => {
            warn_unencrypted();
            target(listen)
        }
        Command::Spike => spike(),
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

#[cfg(target_os = "linux")]
fn spike() -> Result<(), String> {
    capture_demo()
}

#[cfg(windows)]
fn spike() -> Result<(), String> {
    type_demo().map_err(|err| format!("input injection failed: {err}"))
}

/// Finds and grabs every keyboard and mouse, then counts what the readers
/// translate until the hotkey is released or 10 seconds pass.
#[cfg(target_os = "linux")]
fn capture_demo() -> Result<(), String> {
    use event::InputEvent;
    use std::sync::mpsc;
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    const KEY_RIGHTSHIFT: u16 = 54;
    const KEY_RIGHTALT: u16 = 100; // AltGr
    const GRAB_LIMIT: Duration = Duration::from_secs(10);

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
        let kind = if found.is_keyboard {
            "keyboard"
        } else {
            "mouse"
        };
        println!("mickey: found {kind}: {}", found.name);
    }

    // Grabbing while a key is held leaves it stuck down on the desktop.
    println!("mickey: waiting until no key is held");
    let os_err = |err: std::io::Error| err.to_string();
    while devices
        .iter()
        .any(|found| found.device.any_key_down().unwrap_or(false))
    {
        sleep(Duration::from_millis(10));
    }
    let grabs = devices
        .iter()
        .map(|found| found.device.grab())
        .collect::<Result<Vec<_>, _>>()
        .map_err(os_err)?;
    println!("mickey: keyboard and mouse grabbed for at most 10 s");
    println!("mickey: press and release AltGr + Right Shift to get them back");

    let (sender, events) = mpsc::channel();
    capture::spawn_readers(&devices, sender);

    let deadline = Instant::now() + GRAB_LIMIT;
    let (mut alt, mut shift, mut both_held) = (false, false, false);
    let (mut keys, mut motions, mut others) = (0u32, 0u32, 0u32);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Ok(event) = events.recv_timeout(left) else {
            break;
        };
        match event {
            InputEvent::Key { code, pressed } => {
                keys += 1;
                match code {
                    KEY_RIGHTALT => alt = pressed,
                    KEY_RIGHTSHIFT => shift = pressed,
                    _ => {}
                }
            }
            InputEvent::Motion { .. } => motions += 1,
            InputEvent::Button { .. } | InputEvent::Scroll { .. } => others += 1,
        }
        both_held |= alt && shift;
        if both_held && !alt && !shift {
            println!("mickey: hotkey released");
            break;
        }
    }

    drop(grabs);
    println!(
        "mickey: released; seen {keys} key, {motions} motion and {others} button/scroll events"
    );
    Ok(())
}

#[cfg(windows)]
fn type_demo() -> std::io::Result<()> {
    use platform::windows::{move_mouse, send_key};
    use std::thread::sleep;
    use std::time::Duration;

    const SCAN_H: u16 = 0x23;
    const SCAN_E: u16 = 0x12;
    const SCAN_L: u16 = 0x26;
    const SCAN_O: u16 = 0x18;

    println!("mickey: click on a text window now, starting in 8 seconds");
    sleep(Duration::from_secs(8));

    for (dx, dy) in [(10, 0), (0, 10), (-10, 0), (0, -10)] {
        for _ in 0..50 {
            move_mouse(dx, dy)?;
            sleep(Duration::from_millis(20));
        }
    }

    for scan_code in [SCAN_H, SCAN_E, SCAN_L, SCAN_L, SCAN_O] {
        // Release even if the press failed, so no key stays held.
        let pressed = send_key(scan_code, false, true);
        let released = send_key(scan_code, false, false);
        pressed?;
        released?;
        sleep(Duration::from_millis(50));
    }

    println!("mickey: done");
    Ok(())
}
