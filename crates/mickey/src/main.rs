//! Mickey: one keyboard and mouse shared across computers on the LAN.

// Not used yet: the audio streams use them from Audio 6 (#270) on.
#[allow(dead_code)]
mod audio_wire;
mod autostart;
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
#[allow(dead_code)]
mod jitter;
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
// Not used yet: the audio streams use it from Audio 6 (#270) on.
#[allow(dead_code)]
mod sound;
mod source;
// `divert` is used once streams start (#270); `clean_up` already is.
#[allow(dead_code)]
mod speakers;
mod transport;
mod tray;
mod tray_picture;
mod trust;
mod window;

use cli::Command;
use mickey_config as config;
use std::net::SocketAddr;

fn main() {
    // Before anything is written: as administrator, never through a
    // junction a user made (see `refuse_user_junctions`).
    #[cfg(windows)]
    if platform::acl::elevated()
        && let Err(err) = platform::acl::refuse_user_junctions()
    {
        eprintln!("mickey: cannot refuse users' junctions: {err}");
    }
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
        Command::Version => {
            println!("mickey {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Help => {
            print!("{}", cli::USAGE);
            Ok(())
        }
        Command::Autostart(Some(on)) => {
            autostart::set(on)?;
            let answer = if on { "yes" } else { "no" };
            println!("mickey: starts with this PC: {answer}");
            Ok(())
        }
        Command::Autostart(None) => {
            println!("{}", if autostart::is_on() { "on" } else { "off" });
            Ok(())
        }
        Command::Id => {
            let identity = identity::Identity::load_or_create(&identity::data_dir()?)?;
            println!("{}", identity.fingerprint());
            Ok(())
        }
        Command::IdKey => {
            let identity = identity::Identity::load_or_create(&identity::data_dir()?)?;
            println!("{}", trust::to_hex(identity.public()));
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
            // The list only administrators can change: the running Mickey
            // changes it.
            if !trust::can_change() {
                let answer = control::request(&format!("forget {fingerprint}"))?
                    .next()
                    .ok_or("Mickey did not answer")?
                    .map_err(|err| err.to_string())?;
                println!("{answer}");
                return if answer.starts_with("ok") {
                    Ok(())
                } else {
                    Err(answer)
                };
            }
            let mut trusted = trust::TrustedPeers::load(&trust::dir()?)?;
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
            let interface = config
                .as_ref()
                .map_or_else(config::Interface::default, |config| {
                    config.interface.clone()
                });
            let config_hotkeys = config.as_ref().map(config::Config::hotkeys).transpose()?;
            let peers = match (to, config) {
                (Some(to), _) => vec![config::Peer {
                    name: to.to_string(),
                    address: to,
                    hotkey: None,
                    position: None,
                    key: None,
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
                interface,
                source::Services {
                    trusted: trust::shared(&trust::dir()?)?,
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
            trust::shared(&trust::dir()?)?,
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

/// Whether Mickey keeps the clipboard history: as `[clipboard] history`
/// says, or, not set, only when no other manager keeps one (#257).
fn keep_history(setting: Option<bool>) -> bool {
    setting.unwrap_or_else(|| match history::another_manager() {
        Some(manager) => {
            println!(
                "mickey: {manager} keeps the clipboard history; Mickey only shares the clipboard"
            );
            false
        }
        None => true,
    })
}

/// `mickey run`: the roles `[this_pc]` asks for, in one process. The target
/// waits for peers on its own thread; the source runs here.
fn run_from_config() -> Result<(), String> {
    // The sound back on the speakers first, if a Mickey before this one
    // ended while sending it (PRD 0012); on Linux, on a stop signal too.
    if let Err(err) = speakers::clean_up() {
        eprintln!("mickey: {err}");
    }
    #[cfg(target_os = "linux")]
    platform::signals::on_stop(|| {
        let _ = speakers::clean_up();
    })
    .map_err(|err| format!("cannot wait for stop signals: {err}"))?;
    // As administrator only where nobody else can change this Mickey: else
    // a program could have changed what it runs or trusts (#247).
    #[cfg(windows)]
    if platform::acl::elevated()
        && (!autostart::install_is_safe() || !platform::acl::user_junctions_refused())
    {
        autostart::step_down()?;
        return Err("Mickey ran as administrator from a folder other users can change, or on a Windows too old to refuse their junctions; it starts again without administrator rights".into());
    }
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
    // Even with no PCs listed yet: the group or a pairing brings them, and
    // the tray shows (peers are symmetric, PRD 0010).
    let source_wanted = this_pc.controls_others;
    if !source_wanted && !this_pc.controlled {
        return Err("[this_pc] says this PC neither controls others nor is controlled".into());
    }
    // The target tells the source when a peer's keyboard comes over, for
    // the status and the tray (PRD 0009).
    let (controlled_sender, controlled) = std::sync::mpsc::channel();
    // One list of trusted PCs for the source and the target.
    let trusted = trust::shared(&trust::dir()?)?;
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
        let result = source::run(
            config.peers,
            hotkeys,
            config.touchpad.speed,
            config.interface,
            source::Services {
                trusted,
                name: this_pc.name(),
                port: this_pc.listen.port(),
                controlled,
                found,
                // Off: another clipboard manager keeps the history.
                history: keeper
                    .clone()
                    .filter(|_| keep_history(config.clipboard.history))
                    .map(history::record),
                keeper,
            },
        );
        match (result, target) {
            // Say why, and still be controlled (say, no access to the
            // keyboard on Linux yet).
            (Err(err), Some(target)) => {
                eprintln!(
                    "mickey: cannot control other PCs: {err}; this PC can still be controlled"
                );
                target
                    .join()
                    .map_err(|_| "the target stopped unexpectedly".to_string())?
            }
            (result, _) => result,
        }
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
    let grants = clip_sync::Grants::default();
    // One virtual keyboard and mouse for every source that connects.
    let emitter = Arc::new(Mutex::new(emit::Emitter::new()?));
    let listener = TcpListener::bind(listen).map_err(|err| match err.kind() {
        ErrorKind::AddrInUse => format!("{listen} is already in use; is another Mickey running?"),
        ErrorKind::AddrNotAvailable => format!("{listen} is not an address of this machine"),
        _ => format!("cannot listen on {listen}: {err}"),
    })?;
    println!("mickey: listening on {listen}");
    if let Some(keeper) = keeper {
        serve_clipboard(
            listen,
            &identity,
            &trusted,
            keeper,
            &controlled,
            &sources,
            &grants,
        );
    }
    let input_slots = std::sync::Arc::default();
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
            grants: grants.clone(),
        };
        let Some(slot) = Slot::take(&input_slots) else {
            continue;
        };
        on_its_thread(move || {
            let _slot = slot;
            serve_source(stream, peer, &identity, &trusted, &emitter, &around);
        });
    }
}

/// At most this many connections on a port at once, paired or not: a few
/// PCs need a few. Without a cap, anyone on the network could open
/// connections until no thread could be made.
const MAX_CONNECTIONS: usize = 32;

/// One of a port's connections, given back when dropped.
struct Slot(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl Slot {
    /// `None` when the port already has `MAX_CONNECTIONS`: the caller
    /// drops the stream, which closes it.
    fn take(count: &std::sync::Arc<std::sync::atomic::AtomicUsize>) -> Option<Slot> {
        use std::sync::atomic::Ordering;
        count
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_CONNECTIONS).then_some(n + 1)
            })
            .ok()
            .map(|_| Slot(count.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

/// Runs `work` on a new thread; when none can be made, says so instead
/// of panicking (`thread::spawn` would end the accept loop).
fn on_its_thread(work: impl FnOnce() + Send + 'static) {
    if let Err(err) = std::thread::Builder::new().spawn(work) {
        eprintln!("mickey: cannot start a thread for a connection: {err}");
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
    grants: &clip_sync::Grants,
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
    let (sources, grants) = (sources.clone(), grants.clone());
    let shown = Shown::default();
    std::thread::spawn(move || {
        let slots = std::sync::Arc::default();
        for stream in listener.incoming().flatten() {
            let Some(slot) = Slot::take(&slots) else {
                continue;
            };
            let (identity, trusted, keeper) = (identity.clone(), trusted.clone(), keeper.clone());
            let (controlled, sources, shown) = (controlled.clone(), sources.clone(), shown.clone());
            let grants = grants.clone();
            let Ok(from) = stream.peer_addr().map(|address| address.ip()) else {
                continue;
            };
            on_its_thread(move || {
                let _slot = slot;
                let is_trusted = |key: &[u8; 32]| {
                    trusted
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .is_trusted(key)
                };
                match transport::accept_data(stream, &identity, is_trusted) {
                    Ok(link) => {
                        let by = *link.peer_key();
                        // The group map and shared hotkeys from a paired PC go to the source.
                        let group = |kind: u8, text: &str| match kind {
                            group::KIND => match group::decode(text, from) {
                                Ok(members) => {
                                    let _ =
                                        controlled.send(source::TargetNews::Group { members, by });
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
                                std::thread::spawn(move || window::close(&shown));
                            }
                            group::HOTKEYS => {
                                let _ =
                                    controlled.send(source::TargetNews::Hotkeys(text.to_string()));
                            }
                            group::AUDIO => {
                                let _ =
                                    controlled.send(source::TargetNews::Audio(text.to_string()));
                            }
                            _ => {}
                        };
                        let _ = clip_sync::answer(
                            link,
                            || clip_sync::may_read(&grants, from),
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
    /// When each source may read the clipboard (#185).
    grants: clip_sync::Grants,
}

/// The switcher window open on this PC for a source, if any, so a later
/// "close" from the source can close it.
type Shown = window::Slot;

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
    let interface = config::Config::load()
        .ok()
        .flatten()
        .map_or_else(config::Interface::default, |config| config.interface);
    let mut args = vec!["switcher".to_string()];
    if mirror {
        args.push("--mirror".to_string());
    }
    // This PC's own corner and direction; the order is the source's.
    args.extend(["--at".to_string(), interface.switcher_position]);
    if interface.switcher_vertical {
        args.push("--vertical".to_string());
    }
    args.push(focused.to_string());
    args.extend(lines.map(str::to_string));
    if mirror {
        // A copy only shows: nobody waits for an answer.
        let slot = shown.clone();
        std::thread::spawn(move || window::run(&args, &slot));
        return;
    }
    // 255: closed without a pick.
    let card = match window::run(&args, shown) {
        window::Outcome::Answered(text) => text.parse::<u8>().unwrap_or(u8::MAX),
        window::Outcome::Closed | window::Outcome::Failed(_) => u8::MAX,
    };
    let mut sources = sources.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(link) = sources.get_mut(&from) {
        let _ = link.send(&protocol::Message::Picked { card });
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
        grants,
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
        if !trust::pairing_open(trusted) {
            println!(
                "mickey: {peer} asked to pair while pairing is closed; refused (open it in the settings' PCs tab, or `mickey pairing`)"
            );
            return;
        }
        let who = found
            .and_then(|table| table.name_of(&peer_key))
            .unwrap_or_else(|| peer.ip().to_string());
        match trust::ask_once(&peer_key, peer, &link.pairing_code(), &who) {
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
                clip_sync::focus_changed(grants, peer.ip(), here);
                // The keyboard and mouse went elsewhere: the cursor too.
                if here {
                    emit::unpark();
                } else {
                    emit::park();
                }
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
