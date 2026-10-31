//! What each command does, and the long-running Mickey started from
//! config.toml.

use crate::cli::Command;
use crate::target::target;
use mickey_config as config;

use crate::{
    audio, audio::speakers, autostart, cli, clipboard, clipboard::history, control, control::mcp,
    input::hotkey, monitors, net::discovery, net::identity, net::trust, platform, source, update,
    window,
};
#[cfg(target_os = "linux")]
use crate::{desktop, desktop::portal};

pub fn run(command: Command) -> Result<(), String> {
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
        Command::Start => autostart::start(),
        Command::UpdateStatus => {
            let waiting = control::request("update")?
                .next()
                .and_then(Result::ok)
                .ok_or("mickey is not running")?;
            println!("{waiting}");
            Ok(())
        }
        Command::Mcp => mcp::serve(),
        Command::UpdateApply => {
            let latest = update::latest()?;
            if !config::version::is_newer(&latest, config::version::THIS) {
                println!("up to date: Mickey {}", config::version::THIS);
                return Ok(());
            }
            update::apply(&latest)?;
            println!("installing: Mickey {latest}; Mickey restarts by itself");
            Ok(())
        }
        Command::UpdateDownload => {
            let latest = update::latest()?;
            if !config::version::is_newer(&latest, config::version::THIS) {
                println!("up to date: Mickey {}", config::version::THIS);
                return Ok(());
            }
            let path = update::download(&latest)?;
            println!(
                "verified: Mickey {latest}, signed by the release key: {}",
                path.display()
            );
            Ok(())
        }
        Command::UpdateCheck => {
            let latest = update::latest()?;
            let this = config::version::THIS;
            if config::version::is_newer(&latest, this) {
                println!("update: Mickey {latest} is out; this PC runs {this}");
            } else {
                println!("up to date: Mickey {this}");
            }
            Ok(())
        }
        Command::Id => {
            let identity = identity::Identity::load_or_create(&identity::data_dir()?)?;
            println!("{}", identity.fingerprint());
            Ok(())
        }
        Command::IdKey => {
            let identity = identity::Identity::load_or_create(&identity::data_dir()?)?;
            println!("{}", crate::hex::encode(identity.public()));
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
            // The running Mickey changes it, so it stops trusting that PC at
            // once (its list is in memory); and only it may when the list is
            // one only administrators can change.
            if control::source_running() || !trust::can_change() {
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
            // `--to` has no config: the defaults, for one peer.
            let next = match (&config, to) {
                (Some(config), None) => config.hotkeys.next_or_default().to_string(),
                _ => config::DEFAULT_NEXT.to_string(),
            };
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
            let set = match &config_hotkeys {
                Some(set) if to.is_none() => set.clone(),
                // Same defaults as an empty config.toml, for these peers.
                _ => config::HotkeySet {
                    peers: vec![None; peers.len()],
                    ..config::Config::empty()
                        .hotkeys()
                        .expect("the defaults are valid")
                },
            };
            println!("mickey: {next} switches focus");
            // Alone, no target tells this source it is controlled.
            let (_, target_news) = std::sync::mpsc::channel();
            source::run(
                peers,
                hotkey::Hotkey::from_config(&set),
                touchpad_speed,
                interface,
                source::Services {
                    trusted: trust::shared(&trust::dir()?)?,
                    name: config::Config::empty().this_pc.name(),
                    port: config::DEFAULT_PORT,
                    target_news,
                    found: None,
                    keeper: clipboard::Keeper::start().ok(),
                    history: None,
                    updates: None,
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
            window::ui_command(&["settings"])
                .and_then(|mut ui| ui.status())
                .map_err(|err| format!("cannot start mickey-ui: {err}"))?;
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

/// Gives the sound back to the speakers when `mickey run` returns, however
/// it returns (Quit in the tray, an error). The thread sending the sound
/// holds its `Diverted`, but the process ends with `main` and never drops
/// it: without this, quitting left the sound on Mickey's silent sink.
struct SoundBack;

impl Drop for SoundBack {
    fn drop(&mut self) {
        if let Err(err) = speakers::clean_up() {
            eprintln!("mickey: cannot give the sound back to the speakers: {err}");
        }
    }
}

/// `mickey run`: the roles `[this_pc]` asks for, in one process. The target
/// waits for peers on its own thread; the source runs here.
fn run_from_config() -> Result<(), String> {
    // The sound back on the speakers first, if a Mickey before this one
    // ended while sending it (PRD 0012); on Linux, on a stop signal too;
    // and when this one returns.
    if let Err(err) = speakers::clean_up() {
        eprintln!("mickey: {err}");
    }
    let _sound_back = SoundBack;
    // Before any thread starts: a thread started earlier would not block
    // the stop signals, and one sent to it would end Mickey at once,
    // leaving the sound off the speakers.
    #[cfg(target_os = "linux")]
    platform::signals::on_stop(|| {
        let _ = speakers::clean_up();
    })
    .map_err(|err| format!("cannot wait for stop signals: {err}"))?;
    // Outside Hyprland, the RemoteDesktop portal's session (PRD 0004),
    // opened now: its one question comes at login, not mid-switch.
    #[cfg(target_os = "linux")]
    if desktop::session() == desktop::Session::Portal {
        crate::threads::spawn("portal start", || {
            // The desktop's portal may come up after Mickey at login.
            for _ in 0..24 {
                if portal::get().is_some() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        });
    }
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
        None => config::Config::empty(),
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
    // Ready for other PCs' sound, should this PC be the one that listens
    // (PRD 0012).
    if let Err(err) = audio::listen(audio::port(this_pc.listen.port())) {
        eprintln!(
            "mickey: no shared audio: cannot listen on UDP {}: {err}",
            audio::port(this_pc.listen.port())
        );
    }
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
    let (news_sender, target_news) = std::sync::mpsc::channel();
    // One list of trusted PCs for the source and the target.
    let trusted = trust::shared(&trust::dir()?)?;
    trust::follow(&trusted, &trust::dir()?);
    // One clipboard for the whole process (PRD 0011).
    let keeper = clipboard::Keeper::start()
        .inspect_err(|err| eprintln!("mickey: {err}; the clipboard will not be shared"))
        .ok();
    let target = this_pc.controlled.then(|| {
        let (listen, found, keeper) = (this_pc.listen, found.clone(), keeper.clone());
        let trusted = trusted.clone();
        crate::threads::spawn("target", move || {
            target(listen, trusted, news_sender, found, keeper)
        })
    });
    if source_wanted {
        let hotkeys = hotkey::Hotkey::from_config(&config.hotkeys()?);
        println!(
            "mickey: {} switches focus",
            config.hotkeys.next_or_default()
        );
        // A newer Mickey: checked once a day, and heard from peers (PRD 0013).
        let updates = update::Updates::default();
        update::start(updates.clone());
        let result = source::run(
            config.peers,
            hotkeys,
            config.touchpad.speed,
            config.interface,
            source::Services {
                trusted,
                name: this_pc.name(),
                port: this_pc.listen.port(),
                target_news,
                found,
                // Off: another clipboard manager keeps the history.
                history: keeper
                    .clone()
                    .filter(|_| keep_history(config.clipboard.history))
                    .map(history::record),
                keeper,
                updates: Some(updates),
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
