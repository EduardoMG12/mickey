//! Mickey: one keyboard and mouse shared across computers on the LAN.

#[cfg(target_os = "linux")]
mod capture;
mod cli;
mod clip_sync;
mod clip_watch;
mod clip_wire;
mod clipboard;
mod control;
mod discovery;
mod edges;
mod emit;
mod event;
mod focus;
mod group;
mod held;
mod history;
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
mod received;
mod record;
mod secure;
mod source;
mod transport;
mod tray;
mod tray_picture;
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
        Command::Clipboard { write_test } => {
            let mut board = clipboard::Clipboard::open()?;
            if write_test {
                board.write(&clipboard::Content::Text("Mickey clipboard test".into()))?;
                // On Wayland and X11 the text lives only while we do: give
                // a moment to paste it somewhere.
                println!("mickey: copied a test text; paste it within 30 s");
                std::thread::sleep(std::time::Duration::from_secs(30));
                return Ok(());
            }
            match board.read() {
                Some(content) => println!("mickey: the clipboard holds {content:?}"),
                None => println!("mickey: the clipboard holds nothing Mickey carries"),
            }
            Ok(())
        }
        Command::History { lines } => {
            for line in control::request("history")? {
                let line = line.map_err(|err| err.to_string())?;
                if let Some(err) = line.strip_prefix("error: ") {
                    return Err(err.to_string());
                }
                if lines {
                    println!("{line}");
                } else if let [id, kind, ago, preview] =
                    line.splitn(4, '\t').collect::<Vec<_>>()[..]
                {
                    println!("{id:>4}  {ago:>5} s ago  {kind:<5}  {preview}");
                }
            }
            Ok(())
        }
        Command::Discover { lines } => {
            let mut any = false;
            for line in control::request("discover")? {
                let line = line.map_err(|err| err.to_string())?;
                if let Some(err) = line.strip_prefix("error: ") {
                    return Err(err.to_string());
                }
                if lines {
                    println!("{line}");
                    any = true;
                    continue;
                }
                let fields: Vec<&str> = line.split('\t').collect();
                if let [name, address, fingerprint] = fields.as_slice() {
                    println!("{name}  {address}  {fingerprint}");
                    any = true;
                }
            }
            if !any && !lines {
                println!("mickey: no other PC heard yet (they announce every 3 s)");
            }
            Ok(())
        }
        Command::Peers => {
            for line in control::request("peers")? {
                println!("{}", line.map_err(|err| err.to_string())?);
            }
            Ok(())
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
                    position: None,
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
            // Alone, no target tells this source it is controlled.
            let (_, controlled) = std::sync::mpsc::channel();
            source::run(
                peers,
                hotkey::Hotkey::from_config(&set),
                touchpad_speed,
                switcher_position,
                source::Services {
                    trusted: trust::shared(&identity::data_dir()?)?,
                    name: config::parse("")
                        .map(|empty| empty.this_pc.name())
                        .unwrap_or_default(),
                    port: config::DEFAULT_PORT,
                    controlled,
                    found: None,
                    keeper: clipboard::Keeper::start().ok(),
                    history: None,
                },
            )
        }
        Command::Target { listen } => target(
            listen,
            trust::shared(&identity::data_dir()?)?,
            std::sync::mpsc::channel().0,
            None,
            clipboard::Keeper::start().ok(),
        ),
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
    // Let the other PCs find this one, and hear them (PRD 0010). Without
    // it Mickey still works with the addresses in config.toml.
    let identity = identity::Identity::load_or_create(&identity::data_dir()?)?;
    let me = discovery::Announce {
        name: this_pc.name(),
        port: this_pc.listen.port(),
        key: *identity.public(),
    };
    let found = discovery::start(me)
        .inspect_err(|err| eprintln!("mickey: {err}; other PCs will not show up by themselves"))
        .ok();
    let source_wanted = this_pc.controls_others && !config.peers.is_empty();
    if !source_wanted && !this_pc.controlled {
        return Err("[this_pc] says this PC neither controls others nor is controlled".into());
    }
    // The target tells the source when a peer's keyboard comes over, for
    // the status and the tray (PRD 0009).
    let (controlled_sender, controlled) = std::sync::mpsc::channel();
    // One list of trusted PCs for the source and the target.
    let trusted = trust::shared(&identity::data_dir()?)?;
    // One clipboard for the whole process (PRD 0011).
    let keeper = clipboard::Keeper::start()
        .inspect_err(|err| eprintln!("mickey: {err}; the clipboard will not be shared"))
        .ok();
    let target = this_pc.controlled.then(|| {
        let (listen, found, keeper) = (this_pc.listen, found.clone(), keeper.clone());
        let trusted = trusted.clone();
        std::thread::spawn(move || target(listen, trusted, controlled_sender, found, keeper))
    });
    if source_wanted {
        let hotkeys = hotkey::Hotkey::from_config(&config.hotkeys()?);
        source::run(
            config.peers,
            hotkeys,
            config.touchpad.speed,
            config.interface.switcher_position,
            source::Services {
                trusted,
                name: this_pc.name(),
                port: this_pc.listen.port(),
                controlled,
                found,
                history: keeper.clone().map(history::record),
                keeper,
            },
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
/// `controlled` hears whether a source's keyboard and mouse control this PC;
/// `found`, when discovery runs, names a PC asking to pair; `keeper`, when
/// the clipboard is reachable, lets paired PCs read and write it.
fn target(
    listen: SocketAddr,
    trusted: trust::Shared,
    controlled: std::sync::mpsc::Sender<source::TargetNews>,
    found: Option<discovery::Table>,
    keeper: Option<clipboard::Keeper>,
) -> Result<(), String> {
    use std::io::ErrorKind;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    let data_dir = identity::data_dir()?;
    let identity = Arc::new(identity::Identity::load_or_create(&data_dir)?);
    let sources = Sources::default();
    // One virtual keyboard and mouse for every source that connects.
    let emitter = Arc::new(Mutex::new(emit::Emitter::new()?));
    let listener = TcpListener::bind(listen).map_err(|err| match err.kind() {
        ErrorKind::AddrInUse => format!("{listen} is already in use; is another Mickey running?"),
        ErrorKind::AddrNotAvailable => format!("{listen} is not an address of this machine"),
        _ => format!("cannot listen on {listen}: {err}"),
    })?;
    println!("mickey: listening on {listen}");
    if let Some(keeper) = keeper {
        serve_clipboard(listen, &identity, &trusted, keeper, &controlled, &sources);
    }
    loop {
        let (stream, peer) = match listener.accept() {
            Ok(connection) => connection,
            Err(err) => {
                eprintln!("mickey: accept failed: {err}");
                continue;
            }
        };
        // Each source on its own thread: a PC asking to pair, its window
        // still open, must not hold up a paired PC that reconnects.
        let (identity, trusted, emitter) = (identity.clone(), trusted.clone(), emitter.clone());
        let around = Around {
            controlled: controlled.clone(),
            found: found.clone(),
            sources: sources.clone(),
        };
        std::thread::spawn(move || {
            serve_source(stream, peer, &identity, &trusted, &emitter, &around);
        });
    }
}

/// Answers paired PCs' data links (PRD 0011) on the port after `listen`,
/// each on its own thread, for as long as Mickey runs.
fn serve_clipboard(
    listen: SocketAddr,
    identity: &std::sync::Arc<identity::Identity>,
    trusted: &std::sync::Arc<std::sync::Mutex<trust::TrustedPeers>>,
    keeper: clipboard::Keeper,
    controlled: &std::sync::mpsc::Sender<source::TargetNews>,
    sources: &Sources,
) {
    let data = transport::data_address(listen);
    let listener = match std::net::TcpListener::bind(data) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("mickey: no shared clipboard: cannot listen on {data}: {err}");
            return;
        }
    };
    let (identity, trusted, controlled) = (identity.clone(), trusted.clone(), controlled.clone());
    let sources = sources.clone();
    let shown = Shown::default();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (identity, trusted, keeper) = (identity.clone(), trusted.clone(), keeper.clone());
            let (controlled, sources, shown) = (controlled.clone(), sources.clone(), shown.clone());
            let Ok(from) = stream.peer_addr().map(|address| address.ip()) else {
                continue;
            };
            std::thread::spawn(move || {
                let is_trusted = |key: &[u8; 32]| {
                    trusted
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .is_trusted(key)
                };
                match transport::accept_data(stream, &identity, is_trusted) {
                    Ok(link) => {
                        // The group map and shared hotkeys from a paired PC go to the source.
                        let group = |kind: u8, text: &str| match kind {
                            group::KIND => match group::decode(text, from) {
                                Ok(members) => {
                                    let _ = controlled.send(source::TargetNews::Group { members });
                                }
                                Err(err) => eprintln!("mickey: {err}"),
                            },
                            group::SWITCHER | group::MIRROR => {
                                let (text, sources, shown) =
                                    (text.to_string(), sources.clone(), shown.clone());
                                let mirror = kind == group::MIRROR;
                                std::thread::spawn(move || {
                                    show_switcher_here(&text, from, &sources, &shown, mirror)
                                });
                            }
                            group::CLOSE => {
                                let shown = shown.clone();
                                std::thread::spawn(move || close_switcher_here(&shown));
                            }
                            group::HOTKEYS => {
                                let _ =
                                    controlled.send(source::TargetNews::Hotkeys(text.to_string()));
                            }
                            _ => {}
                        };
                        let _ = clip_sync::answer(
                            link,
                            || keeper.read(),
                            |content| keeper.write(content),
                            group,
                        );
                    }
                    Err(err) => eprintln!("mickey: data link refused: {err}"),
                }
            });
        }
    });
}

/// The sources driving this PC, by address: their input links, for what
/// goes back to them outside `serve` (a pick in the switcher shown here).
type Sources =
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<std::net::IpAddr, secure::Link>>>;

/// What every source's thread on the target shares.
struct Around {
    controlled: std::sync::mpsc::Sender<source::TargetNews>,
    found: Option<discovery::Table>,
    sources: Sources,
}

/// The switcher window open on this PC for a source, if any, so a later
/// "close" from the source can close it.
type Shown = std::sync::Arc<std::sync::Mutex<Option<std::process::Child>>>;

/// Runs `mickey-ui switcher` with the cards a source sent (PRD 0007), in
/// this PC's own corner: the first line is the focused card, then one card
/// per line. A `mirror` only shows; otherwise this is the PC in use, and
/// the card picked goes back to the source.
fn show_switcher_here(
    text: &str,
    from: std::net::IpAddr,
    sources: &Sources,
    shown: &Shown,
    mirror: bool,
) {
    let mut lines = text.lines();
    let Some(focused) = lines.next() else {
        return;
    };
    let corner = config::Config::load().ok().flatten().map_or_else(
        || "bottom-right".to_string(),
        |config| config.interface.switcher_position,
    );
    let mut args = vec!["switcher".to_string()];
    if mirror {
        args.push("--mirror".to_string());
    }
    args.extend(["--at".to_string(), corner, focused.to_string()]);
    args.extend(lines.map(str::to_string));
    let child = std::env::current_exe()
        .map(|exe| exe.with_file_name(format!("mickey-ui{}", std::env::consts::EXE_SUFFIX)))
        .and_then(|ui| {
            std::process::Command::new(ui)
                .args(&args)
                .stdout(std::process::Stdio::piped())
                .spawn()
        });
    let Ok(child) = child else {
        return;
    };
    close_switcher_here(shown);
    *shown.lock().unwrap_or_else(|p| p.into_inner()) = Some(child);
    if mirror {
        return;
    }
    // Waits without holding the slot, so "close" can still kill the window.
    let status = loop {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut slot = shown.lock().unwrap_or_else(|p| p.into_inner());
        let Some(child) = slot.as_mut() else {
            break None;
        };
        match child.try_wait() {
            Ok(None) => continue,
            Ok(Some(status)) => break slot.take().map(|child| (status, child)),
            Err(_) => break None,
        }
    };
    // 255: closed without a pick.
    let card = status
        .filter(|(status, _)| status.success())
        .and_then(|(_, child)| {
            let mut out = String::new();
            std::io::Read::read_to_string(&mut child.stdout?, &mut out).ok()?;
            out.trim().parse::<u8>().ok()
        })
        .unwrap_or(u8::MAX);
    let mut sources = sources.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(link) = sources.get_mut(&from) {
        let _ = link.send(&protocol::Message::Picked { card });
    }
}

/// Closes the switcher window open here for a source, if any.
fn close_switcher_here(shown: &Shown) {
    if let Some(mut child) = shown.lock().unwrap_or_else(|p| p.into_inner()).take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Pairs with one source if needed, then plays what it sends until it
/// leaves.
fn serve_source(
    stream: std::net::TcpStream,
    peer: SocketAddr,
    identity: &identity::Identity,
    trusted: &std::sync::Mutex<trust::TrustedPeers>,
    emitter: &std::sync::Mutex<emit::Emitter>,
    around: &Around,
) {
    let Around {
        controlled,
        found,
        sources,
    } = around;
    let found = found.as_ref();
    let mut link = match transport::accept(stream, identity) {
        Ok(link) => link,
        Err(err) => {
            eprintln!("mickey: {peer} failed the secure handshake: {err}");
            return;
        }
    };
    let peer_key = *link.peer_key();
    let lock = || trusted.lock().unwrap_or_else(|p| p.into_inner());
    // The list is not held while the person decides.
    if !lock().is_trusted(&peer_key) {
        let who = found
            .and_then(|table| table.name_of(&peer_key))
            .unwrap_or_else(|| peer.ip().to_string());
        match trust::ask(&peer_key, peer, &link.pairing_code(), &who) {
            Ok(true) => {
                if let Err(err) = lock().add(peer_key, &peer.to_string()) {
                    eprintln!("mickey: {err}");
                    return;
                }
                println!("mickey: paired with {who}");
            }
            Ok(false) => {
                println!("mickey: refused {peer}");
                return;
            }
            Err(err) => {
                eprintln!("mickey: {err}");
                return;
            }
        }
    }
    println!("mickey: source {peer} connected");
    // Kept so a switcher shown here can send the pick back to this source.
    if let Ok(clone) = link.try_clone() {
        sources
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(peer.ip(), clone);
    }
    let mut reported = false;
    // Asked per connection: monitors may have changed since the last one.
    let monitors = monitors::this_pc().unwrap_or_else(|err| {
        eprintln!("mickey: {err}; the source will not see this PC's monitors");
        Vec::new()
    });
    let output = |effect| {
        let mut emitter = emitter.lock().unwrap_or_else(|p| p.into_inner());
        match effect {
            transport::Effect::Input(event) => emitter.emit(event),
            transport::Effect::Place { x, y } => emitter.place(x, y),
            // Nobody listens when this PC only waits to be controlled.
            transport::Effect::Controlled(here) => {
                let _ = controlled.send(source::TargetNews::Controlled(here));
                Ok(())
            }
            transport::Effect::Arranged(position) => {
                let from = peer.ip();
                let _ = controlled.send(source::TargetNews::Arranged { from, position });
                Ok(())
            }
        }
    };
    let result = transport::serve(
        &mut link,
        &monitors,
        |effect| match output(effect) {
            Ok(()) => true,
            Err(err) => {
                if !reported {
                    eprintln!("mickey: input injection failed: {err} (not reported again)");
                    reported = true;
                }
                false
            }
        },
        emit::cursor,
    );
    sources
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(&peer.ip());
    match result {
        Ok(()) => println!("mickey: source {peer} disconnected"),
        Err(err) => eprintln!("mickey: source {peer} dropped: {err}"),
    }
}
