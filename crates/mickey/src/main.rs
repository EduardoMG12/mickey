//! Mickey: one keyboard and mouse shared across computers on the LAN.

#[cfg(target_os = "linux")]
mod capture;
mod cli;
mod control;
mod emit;
mod event;
mod focus;
mod held;
mod hotkey;
mod identity;
mod input;
mod latency;
mod monitors;
// The Linux build never translates keys for Windows.
#[cfg_attr(not(windows), allow(dead_code))]
mod keymap;
mod platform;
mod protocol;
mod secure;
mod source;
mod transport;
mod trust;

use cli::Command;
use mickey_config as config;
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
        Command::Order(order) => {
            let answer = control::request(&order)?
                .next()
                .ok_or("mickey source closed the connection")?
                .map_err(|err| err.to_string())?;
            match answer.strip_prefix("ok: ") {
                Some(done) => {
                    println!("mickey: {done}");
                    Ok(())
                }
                None => Err(answer.trim_start_matches("error: ").to_string()),
            }
        }
        Command::Status { waybar: true } => follow_for_waybar(),
        Command::Status { waybar: false } => {
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
            let switcher_position = config.as_ref().map_or_else(
                || config::Interface::default().switcher_position,
                |config| config.interface.switcher_position.clone(),
            );
            let config_hotkeys = config.as_ref().map(config::Config::hotkeys).transpose()?;
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
            // `--to` has no config: the defaults, for one peer.
            let set = match &config_hotkeys {
                Some(set) if to.is_none() => set.clone(),
                // Same defaults as an empty config.toml, for these peers.
                _ => config::HotkeySet {
                    peers: vec![None; peers.len()],
                    ..config::parse("")
                        .and_then(|empty| empty.hotkeys())
                        .expect("the defaults are valid")
                },
            };
            source::run(
                peers,
                hotkey::Hotkey::from_config(&set),
                touchpad_speed,
                switcher_position,
            )
        }
        Command::Target { listen } => target(listen),
        Command::Run => run_from_config(),
        Command::Monitors => {
            for (i, m) in monitors::this_pc()?.iter().enumerate() {
                println!("monitor {i}: {}x{} at {},{}", m.width, m.height, m.x, m.y);
            }
            Ok(())
        }
        Command::Settings => {
            // The window lives in mickey-ui, next to this program, so the
            // long-running mickey never links the GUI (PRD 0003).
            let ui = std::env::current_exe()
                .map(|exe| exe.with_file_name(format!("mickey-ui{}", std::env::consts::EXE_SUFFIX)))
                .map_err(|err| err.to_string())?;
            std::process::Command::new(&ui)
                .arg("settings")
                .status()
                .map_err(|err| format!("cannot start {}: {err}", ui.display()))?;
            Ok(())
        }
    }
}

/// `mickey run`: the roles `[this_pc]` asks for, in one process. The target
/// waits for peers on its own thread; the source runs here.
fn run_from_config() -> Result<(), String> {
    // Right after installing there is no config yet: with no peers, this PC
    // just waits to be controlled until some are added.
    let config = match config::Config::load()? {
        Some(config) => config,
        None => config::parse("").expect("an empty config is valid"),
    };
    let this_pc = &config.this_pc;
    let source_wanted = this_pc.controls_others && !config.peers.is_empty();
    if !source_wanted && !this_pc.controlled {
        return Err("[this_pc] says this PC neither controls others nor is controlled".into());
    }
    let target = this_pc.controlled.then(|| {
        let listen = this_pc.listen;
        std::thread::spawn(move || target(listen))
    });
    if source_wanted {
        let hotkeys = hotkey::Hotkey::from_config(&config.hotkeys()?);
        source::run(
            config.peers,
            hotkeys,
            config.touchpad.speed,
            config.interface.switcher_position,
        )
    } else {
        println!("mickey: this PC only waits to be controlled");
        target
            .expect("controlled is true here")
            .join()
            .map_err(|_| "the target stopped unexpectedly".to_string())?
    }
}

/// Prints a Waybar line now and after every change, for as long as Waybar
/// keeps it running. While no source runs it shows "off" and retries, so
/// the bar never shows an error.
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
fn target(listen: SocketAddr) -> Result<(), String> {
    use std::io::ErrorKind;
    use std::net::TcpListener;

    let data_dir = identity::data_dir()?;
    let identity = identity::Identity::load_or_create(&data_dir)?;
    let mut trusted = trust::TrustedPeers::load(&data_dir)?;
    let mut emitter = emit::Emitter::new()?;
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
        // Asked per connection: monitors may have changed since the last one.
        let monitors = monitors::this_pc().unwrap_or_else(|err| {
            eprintln!("mickey: {err}; the source will not see this PC's monitors");
            Vec::new()
        });
        let result = transport::serve(&mut link, &monitors, |event| {
            if let Err(err) = emitter.emit(event)
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
