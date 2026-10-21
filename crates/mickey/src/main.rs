//! Mickey: one keyboard and mouse shared across computers on the LAN.

#[cfg(target_os = "linux")]
mod capture;
// ponytail: parsed and checked, but used only by tests until the hotkey
// matcher learns several chords (#61, next PR).
#[allow(dead_code)]
mod chord;
mod cli;
mod config;
#[cfg(target_os = "linux")]
mod control;
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
#[cfg(target_os = "linux")]
mod source;
#[allow(dead_code)]
mod transport;
mod trust;

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
        Command::Id => {
            let identity = identity::Identity::load_or_create(&identity::data_dir()?)?;
            println!("{}", identity.fingerprint());
            Ok(())
        }
        Command::Status { waybar: true } => {
            #[cfg(target_os = "linux")]
            follow_for_waybar();
            #[cfg(not(target_os = "linux"))]
            Err("mickey status needs the source, which runs only on Linux in M0".into())
        }
        Command::Status { waybar: false } => {
            #[cfg(target_os = "linux")]
            {
                let mut answer = control::request("status")?;
                let line = answer
                    .next()
                    .ok_or("mickey source has no status yet")?
                    .map_err(|err| err.to_string())?;
                let status = control::Status::from_line(&line)
                    .ok_or_else(|| format!("mickey source answered `{line}`"))?;
                print!("{}", status.describe());
                Ok(())
            }
            #[cfg(not(target_os = "linux"))]
            Err("mickey status needs the source, which runs only on Linux in M0".into())
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
            let peers = match (to, config) {
                (Some(to), _) => vec![config::Peer {
                    name: to.to_string(),
                    address: to,
                    hotkey: None,
                }],
                (None, Some(config)) if !config.peers.is_empty() => config.peers,
                (None, _) => {
                    return Err(format!(
                        "no peer to connect to: pass --to <ip:port>, or list peers in {}, e.g.\n\n{}",
                        config::path()?.display(),
                        config::EXAMPLE
                    ));
                }
            };
            #[cfg(target_os = "linux")]
            return source::run(peers, touchpad_speed);
            #[cfg(not(target_os = "linux"))]
            {
                let _ = (peers, touchpad_speed);
                Err("in M0 the source runs only on Linux (Windows sources come in M2)".into())
            }
        }
        Command::Target { listen } => target(listen),
    }
}

/// Prints a Waybar line now and after every change, for as long as Waybar
/// keeps it running. While no source runs it shows "off" and retries, so
/// the bar never shows an error.
#[cfg(target_os = "linux")]
fn follow_for_waybar() -> ! {
    use std::io::Write;
    let mut last = None;
    loop {
        let mut show = |line: String| {
            if last.as_ref() != Some(&line) {
                println!("{line}");
                // Waybar reads a pipe, which Rust buffers; push each line out.
                let _ = std::io::stdout().flush();
                last = Some(line);
            }
        };
        if let Ok(lines) = control::request("follow") {
            for line in lines.map_while(Result::ok) {
                if let Some(status) = control::Status::from_line(&line) {
                    show(control::waybar_line(Some(&status)));
                }
            }
        }
        show(control::waybar_line(None));
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
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
        match trust::trust_or_ask(&identity, &mut trusted, &peer_key, peer) {
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
