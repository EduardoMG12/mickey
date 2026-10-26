//! The source side (Linux and Windows): capture local input, run the
//! focus state machine, and send input to the focused peer.

use crate::clip_sync::Mover;
use crate::clipboard::Keeper;
use crate::control::{self, Control, Order, Request, Status};
use crate::discovery;
use crate::edges;
use crate::emit;
use crate::event::InputEvent;
use crate::focus::{Action, Focus, Switcher};
use crate::group;
use crate::history;
use crate::hotkey::Hotkey;
use crate::identity::{self, Identity};
use crate::input::Input;
use crate::latency::{self, Latency};
use crate::monitors::{self, Monitor};
use crate::protocol::Message;
use crate::record::{Heard, Recording};
use crate::secure::Link;
use crate::transport;
use crate::tray::{self, Tray};
use crate::trust;
use crate::update;
use crate::window;
use mickey_config as config;
use mickey_config::peers::{PeerInfo, Peers};
use std::net::SocketAddr;
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

const PING_EVERY: Duration = Duration::from_secs(1);
/// With `MICKEY_BENCH` set, while the keyboard is on another PC: a ping
/// every 10 ms, so the latency report (on coming home) holds enough samples
/// for a p99 while input is flowing (#191). Off by default: one a second is
/// plenty to notice a PC gone silent.
const BENCH_PING_EVERY: Duration = Duration::from_millis(10);
const KEYS_UP_POLL: Duration = Duration::from_millis(10);
const RETRY_EVERY: Duration = Duration::from_secs(5);
/// A peer that has answered nothing (no pong) for this long is treated as
/// gone: frozen or unplugged, it must not keep the keyboard and mouse.
const SILENT_LIMIT: Duration = Duration::from_secs(4);

/// What wakes the main loop.
enum Wake {
    Input(InputEvent),
    /// The switcher window closed: the picked peer, `None` for this PC or
    /// Esc, or why the window could not run.
    Pick(Result<Option<usize>, String>),
    Control(Request),
    /// A peer's keyboard and mouse came over to this PC, or left.
    Controlled(bool),
    /// A pick in the tray icon's menu.
    Tray(tray::Click),
    /// The peer at this address placed itself on this PC's desktop.
    Arranged(std::net::IpAddr, [i32; 2]),
    /// The peer at this address runs this Mickey version.
    Version(std::net::IpAddr, String),
    /// A peer's group map (PRD 0010), and who sent it.
    Group(Vec<group::Member>, [u8; 32]),
    /// A peer's hotkeys, to take.
    Hotkeys(String),
    /// Who listens, from a peer.
    Audio(String),
    /// The person here accepted a PC the group introduced (#184).
    Accepted(group::Member, [u8; 32]),
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
        Order::Switcher => match switcher.focus() {
            Focus::Remote(peer) => {
                act(Action::OpenSwitcherOn(peer));
                "ok: showing the switcher on the PC in use".into()
            }
            _ => "error: the focus is moving; try again".into(),
        },
        // The main loop answers these, since it knows the answers.
        Order::Peers
        | Order::Discover
        | Order::History
        | Order::HistoryCopy(_)
        | Order::Quit
        | Order::Arranged
        | Order::Record
        | Order::Forget(_)
        | Order::Connect { .. } => "error: not an order".into(),
        Order::Pairing => {
            trust::open_pairing();
            format!(
                "ok: a new PC may ask to pair for {} s",
                trust::PAIRING_OPEN.as_secs()
            )
        }
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

/// The peer the cursor is pushing onto, and where on its desktop. Never
/// while a mouse button is down: dragging a window to the edge must not
/// throw it to another PC.
fn edge_crossed(
    positions: &[Option<[i32; 2]>],
    peers: &[Peer],
    this_pc: &[Monitor],
    delta: (i32, i32),
    input: &Input,
) -> Option<(usize, (i32, i32))> {
    let placed = |index: usize| positions[index].filter(|_| peers[index].link.is_some());
    // Without a placed peer online, the cursor is not even asked for.
    (0..peers.len()).find(|&index| placed(index).is_some())?;
    let off = edges::beyond(this_pc, input.cursor()?, delta)?;
    let (index, there) = (0..peers.len()).find_map(|index| {
        let there = edges::onto(&peers[index].monitors, placed(index)?, off)?;
        Some((index, there))
    })?;
    (!input.button_down()).then_some((index, there))
}

/// Whether the keyboard and mouse in use control this PC (PRD 0009). Two
/// things change it: this PC's own keyboard going to a peer or coming home,
/// and a peer's keyboard coming over or leaving. The last change wins.
struct InControl {
    here: bool,
    /// Where this PC's own keyboard is: `None` here, or a peer.
    focus: Option<usize>,
}

impl Default for InControl {
    fn default() -> InControl {
        InControl {
            here: true,
            focus: None,
        }
    }
}

impl InControl {
    fn here(&self) -> bool {
        self.here
    }

    /// This PC's keyboard is on `focus` now. If it moved, returns where it
    /// was, so the peers can be told.
    fn focus_is(&mut self, focus: Option<usize>) -> Option<Option<usize>> {
        if focus == self.focus {
            return None;
        }
        let was = std::mem::replace(&mut self.focus, focus);
        self.here = focus.is_none();
        Some(was)
    }

    /// A peer's keyboard came over (`true`) or left (`false`).
    fn peer_says(&mut self, here: bool) {
        self.here = here;
    }
}

/// Opens `mickey-ui <window>` (settings, history), installed next to this
/// program, and leaves it running on its own.
fn open_window(window: &str) {
    let ui = std::env::current_exe()
        .map(|exe| exe.with_file_name(format!("mickey-ui{}", std::env::consts::EXE_SUFFIX)));
    if let Err(err) = ui.and_then(|ui| Command::new(ui).arg(window).spawn()) {
        eprintln!("mickey: cannot open the {window} window: {err}");
    }
}

/// Uses `config`'s hotkeys at once, if they are valid.
fn use_hotkeys(config: &config::Config, switcher: &mut Switcher, window: &mut Duration) {
    match config.hotkeys() {
        Ok(set) => {
            let hotkey = Hotkey::from_config(&set);
            *window = hotkey.double_tap_window();
            switcher.set_hotkeys(hotkey);
        }
        Err(err) => eprintln!("mickey: hotkeys not changed: {err}"),
    }
}

/// After the owner placed PCs in the settings window: reads the positions
/// saved in config.toml, uses them, and tells every placed peer where this
/// PC sits on its desktop (the mirror: if it is at `x, y` here, this PC is
/// at `-x, -y` there), so both PCs agree. Always all of them: telling a peer
/// what it already knows only writes the same line again.
/// Takes config.toml's positions and tells each PC online where this PC
/// sits on its desktop. The PCs offline go into `unsent`, told as soon as
/// they connect (#288): a layout saved while a PC was off must reach it.
fn arrange(
    positions: &mut [Option<[i32; 2]>],
    peers: &mut [Peer],
    unsent: &mut std::collections::HashSet<String>,
) -> String {
    let config = match config::Config::load() {
        Ok(Some(config)) => config,
        Ok(None) => return "error: no config.toml".into(),
        Err(err) => return format!("error: {err}"),
    };
    let mut told = 0;
    for (index, peer) in peers.iter_mut().enumerate() {
        let saved = config.peers.iter().find(|saved| saved.name == peer.name);
        let Some([x, y]) = saved.and_then(|saved| saved.position) else {
            continue;
        };
        positions[index] = Some([x, y]);
        if let Some(link) = &mut peer.link
            && link.send(&Message::Arrange { x: -x, y: -y }).is_ok()
        {
            told += 1;
            unsent.remove(&peer.name);
        } else {
            unsent.insert(peer.name.clone());
        }
    }
    let later = if unsent.is_empty() {
        String::new()
    } else {
        format!("; {} more once online", unsent.len())
    };
    format!("ok: told {told} PC(s) about the layout{later}")
}

/// One PC from a peer's map (already in this PC's coordinates): trusted,
/// because a trusted member vouches for it (the owner's choice, PRD 0010);
/// its place taken if it has one; added to config.toml and to `peers` if
/// this PC did not know it. Returns the new peer's index, to connect to.
/// Writes who listens, as a peer shared it, into config.toml, keeping this
/// PC's own mode; nothing when it is already so (every new data link
/// sends it again).
fn take_audio(lines: &str) -> Result<config::Audio, String> {
    let path = config::path()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mine = config::parse(&text)?.audio;
    let taken = mine.taking(lines)?;
    if taken != mine {
        config::write(&path, &config::set_audio(&text, &taken)?)?;
        println!("mickey: another PC changed who listens");
    }
    Ok(taken)
}

/// Makes this PC the one that listens, in config.toml (the tray's
/// "Listen here").
fn listen_here(me: &[u8; 32]) -> Result<config::Audio, String> {
    let path = config::path()?;
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let audio = config::Audio {
        listener: Some(crate::trust::to_hex(me)),
        since: config::now_seconds(),
        ..config::parse(&text)?.audio
    };
    config::write(&path, &config::set_audio(&text, &audio)?)?;
    println!("mickey: this PC listens now");
    Ok(audio)
}

/// Where this PC's sound should go now (PRD 0012): the listener's audio
/// port and its index, when another PC listens, is online, and this PC's
/// mode sends.
fn audio_target(
    audio: &config::Audio,
    me: &[u8; 32],
    peers: &[Peer],
    online: &[bool],
) -> Option<(usize, SocketAddr)> {
    if audio.mode == config::SoundMode::Here {
        return None;
    }
    let listener = audio.listener.as_deref()?;
    if listener == crate::trust::to_hex(me) {
        return None;
    }
    let index = peers.iter().position(|peer| {
        peer.key
            .is_some_and(|key| crate::trust::to_hex(&key) == listener)
    })?;
    let address = peers[index].address;
    online[index].then(|| {
        (
            index,
            SocketAddr::new(address.ip(), crate::audio::port(address.port())),
        )
    })
}

/// Asks, on its own thread (the main loop must keep moving keys), whether
/// to let `member` join, as `introducer` (who sent the map) says it is in
/// the group. A yes comes back as `Wake::Accepted`; anything else leaves
/// it out.
fn ask_to_join(member: group::Member, by: [u8; 32], introducer: String, wake: &Sender<Wake>) {
    let wake = wake.clone();
    crate::threads::spawn("ask to join", move || {
        let ui = std::env::current_exe()
            .map(|exe| exe.with_file_name(format!("mickey-ui{}", std::env::consts::EXE_SUFFIX)));
        let answer = ui.and_then(|ui| {
            Command::new(ui)
                .args(["join", &member.name, &introducer])
                .status()
        });
        match answer.map(|status| status.code()) {
            Ok(Some(0)) => {
                let _ = wake.send(Wake::Accepted(member, by));
            }
            Ok(Some(1)) => println!("mickey: {} was not let in", member.name),
            _ => eprintln!(
                "mickey: cannot ask whether to let {} in; it stays out",
                member.name
            ),
        }
    });
}

fn join(
    member: group::Member,
    by: &[u8; 32],
    shared: &Shared,
    peers: &mut Vec<Peer>,
    positions: &mut Vec<Option<[i32; 2]>>,
) -> Option<usize> {
    // Only PCs on a local network: a group map cannot make this PC call
    // out to the internet, or to itself (#184).
    if !group::is_local(member.address.ip()) {
        eprintln!(
            "mickey: {} from the group is not on a local network; ignored",
            member.name
        );
        return None;
    }
    {
        let mut trusted = shared.trusted.lock().unwrap_or_else(|p| p.into_inner());
        // Noted with who introduced it, so forgetting that PC forgets it too.
        let note = format!(
            "{} via {}",
            member.address,
            crate::identity::fingerprint(by)
        );
        if !trusted.is_trusted(&member.key)
            && let Err(err) = trusted.add(member.key, &note)
        {
            eprintln!("mickey: {err}");
            return None;
        }
    }
    let known = peers
        .iter()
        .position(|peer| peer.key == Some(member.key) || peer.address == member.address);
    if let Some(index) = known {
        if let Some(position) = member.position
            && positions[index] != Some(position)
        {
            positions[index] = Some(position);
            save_position(&peers[index].name, position);
        }
        return None;
    }
    match add_peer(
        peers,
        positions,
        &member.name,
        member.address,
        member.position,
    ) {
        Ok(index) => {
            peers[index].key = Some(member.key);
            save_key(&peers[index].name, &member.key);
            println!("mickey: {} joined through the group", peers[index].name);
            Some(index)
        }
        Err(err) => {
            eprintln!("mickey: cannot add {} from the group: {err}", member.name);
            None
        }
    }
}

/// A PC new to this one: written into config.toml (a name already taken
/// gets a number) and added to `peers`, not yet connected. Its index.
fn add_peer(
    peers: &mut Vec<Peer>,
    positions: &mut Vec<Option<[i32; 2]>>,
    name: &str,
    address: SocketAddr,
    position: Option<[i32; 2]>,
) -> Result<usize, String> {
    let name = config::path().and_then(|path| {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let (text, name) = config::add_peer(&text, name, address, position)?;
        config::write(&path, &text)?;
        Ok(name)
    })?;
    peers.push(Peer {
        name,
        address,
        link: None,
        heard: Instant::now(),
        key: None,
        latency: Latency::new(),
        monitors: Vec::new(),
        new_key: false,
        version: None,
    });
    positions.push(position);
    Ok(peers.len() - 1)
}

/// Writes `key` into the peer's block in config.toml, the first time it
/// connects (#184).
fn save_key(name: &str, key: &[u8; 32]) {
    let saved = config::path().and_then(|path| {
        let text = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
        let text = config::set_key(&text, name, &trust::to_hex(key))?;
        config::write(&path, &text)
    });
    if let Err(err) = saved {
        eprintln!("mickey: cannot remember {name}'s key: {err}");
    }
}

/// Writes where a peer placed itself into config.toml, so the settings
/// window and the next start see it too.
fn save_position(name: &str, position: [i32; 2]) {
    let saved = config::path().and_then(|path| {
        let text = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
        let text = config::set_position(&text, name, position)?;
        config::write(&path, &text)
    });
    match saved {
        Ok(()) => println!("mickey: {name} placed itself on this desktop at {position:?}"),
        Err(err) => eprintln!("mickey: cannot save where {name} sits: {err}"),
    }
}

/// The answer to `mickey discover`: one PC per line, its name, address
/// and fingerprint, tab-separated.
fn found_text(found: Option<&discovery::Table>) -> String {
    let Some(table) = found else {
        return "error: this Mickey is not looking for other PCs".into();
    };
    let lines: Vec<String> = table
        .list(Instant::now())
        .iter()
        .map(|pc| {
            format!(
                "{}\t{}\t{}",
                pc.name,
                pc.address,
                identity::fingerprint(&pc.key)
            )
        })
        .collect();
    lines.join("\n")
}

/// The answer to `mickey peers`.
fn peers_text(this_pc: &[Monitor], peers: &[Peer], online: &[bool]) -> String {
    Peers {
        this_pc: this_pc.to_vec(),
        peers: peers
            .iter()
            .zip(online)
            .map(|(peer, &online)| PeerInfo {
                name: peer.name.clone(),
                online,
                reinstalled: peer.new_key,
                version: peer.version.clone(),
                monitors: peer.monitors.clone(),
            })
            .collect(),
    }
    .to_text()
}

/// One configured peer and its link, if it is connected.
struct Peer {
    name: String,
    address: SocketAddr,
    link: Option<Link>,
    /// When it last answered (a pong, its monitors…).
    heard: Instant,
    /// Its public key, once it has connected; how the group knows it.
    key: Option<[u8; 32]>,
    latency: Latency,
    /// What the peer reported (PRD 0005); empty until it does.
    monitors: Vec<Monitor>,
    /// It answered with another key than the one this PC knows: refused,
    /// and the settings say why (#289).
    new_key: bool,
    /// The Mickey version it runs, once its data link said (PRD 0013).
    version: Option<String>,
}

/// Everything the background reconnect threads need, shared read-only.
struct Shared {
    identity: Identity,
    /// The trusted PCs. Reconnects never ask, so a peer that is not in
    /// here is not retried.
    trusted: trust::Shared,
}

/// What the target passes on to the source.
pub enum TargetNews {
    /// A peer's keyboard and mouse came over to this PC, or left (PRD 0009).
    Controlled(bool),
    /// The peer at this address placed itself on this PC's desktop.
    Arranged {
        from: std::net::IpAddr,
        position: [i32; 2],
    },
    /// A peer's group map (PRD 0010), in its own coordinates, and the key
    /// of the peer that sent it.
    Group {
        members: Vec<group::Member>,
        by: [u8; 32],
    },
    /// A peer's `[hotkeys]` lines, saved there: the group shares hotkeys.
    Hotkeys(String),
    /// Who listens, as a peer shared it (PRD 0012).
    Audio(String),
    /// The Mickey version the peer at this address runs (PRD 0013).
    Version {
        from: std::net::IpAddr,
        version: String,
    },
}

/// What the rest of Mickey shares with the source. Each is optional:
/// Mickey works without any of them.
pub struct Services {
    /// What the target hears from the peers driving this PC.
    pub controlled: mpsc::Receiver<TargetNews>,
    /// The PCs heard on the network (PRD 0010).
    pub found: Option<discovery::Table>,
    /// The PCs this one trusts, shared with the target.
    pub trusted: trust::Shared,
    /// This PC as the group knows it: its name and the port it listens on.
    pub name: String,
    pub port: u16,
    /// The clipboard, and its history (PRD 0011).
    pub keeper: Option<Keeper>,
    pub history: Option<history::Shared>,
    /// A newer Mickey, found by the daily check or heard from a peer.
    pub updates: Option<update::Updates>,
}

/// Captures local input and sends it to the focused peer. Every peer gets
/// its own secure link; one that is offline is retried in the background
/// and never blocks the others.
pub fn run(
    configured: Vec<config::Peer>,
    hotkeys: Hotkey,
    touchpad_speed: i32,
    mut interface: config::Interface,
    services: Services,
) -> Result<(), String> {
    let Services {
        trusted,
        name: my_name,
        port: my_port,
        controlled,
        found,
        keeper,
        history,
        updates,
    } = services;
    // Two sources would both capture the keyboard and fight over the grab.
    if control::source_running() {
        return Err("another mickey source is already running".into());
    }
    let input = Input::open()?;
    let data_dir = identity::data_dir()?;
    let identity = Identity::load_or_create(&data_dir)?;
    let start = Instant::now();
    let (pong_sender, pongs) = mpsc::channel();
    let (back_sender, back) = mpsc::channel();

    let mut positions: Vec<Option<[i32; 2]>> =
        configured.iter().map(|peer| peer.position).collect();
    let mut peers: Vec<Peer> = configured
        .into_iter()
        .map(|peer| Peer {
            key: peer.key.as_deref().and_then(|hex| {
                let key = trust::from_hex(hex);
                if key.is_none() {
                    eprintln!(
                        "mickey: {}'s key in config.toml is not 64 hex digits; ignored",
                        peer.name
                    );
                }
                key
            }),
            name: peer.name,
            address: peer.address,
            link: None,
            heard: start,
            latency: Latency::new(),
            monitors: Vec::new(),
            new_key: false,
            version: None,
        })
        .collect();
    let shared = Arc::new(Shared { identity, trusted });
    // Every peer is connected on its own thread, asking to pair where
    // needed: one that hangs (frozen, or waiting for its owner to answer
    // a pair window) never holds up the others. Links arrive on `back`.
    for (index, peer) in peers.iter().enumerate() {
        connect_first(
            index,
            peer.name.clone(),
            peer.address,
            &shared,
            &back_sender,
        );
    }
    // The clipboard follows the keyboard (PRD 0011), over data links.
    let mover = keeper.clone().map(Mover::start);
    // Who listens, to every peer as its data link opens (PRD 0012).
    let mut audio = config::Config::load()
        .ok()
        .flatten()
        .map(|config| config.audio)
        .unwrap_or_default();
    if let Some(mover) = &mover {
        mover.share_audio(&audio.shared_lines());
    }
    // This PC's sound on its way to the listener, while it should be.
    let mut sending: Option<(usize, crate::audio::Sending)> = None;
    println!("mickey: AltGr + Right Shift switches focus");

    // One channel wakes the loop for input, a switcher pick or an order
    // from the control socket, so each is handled the moment it happens.
    let (wake, wakes) = mpsc::channel();
    input.start(wake.clone(), touchpad_speed)?;
    let mut switcher_open = false;
    // Local motion between two questions about where the cursor is.
    let mut edge_check = edges::Throttle::new();
    // The copy of a switcher shown on another PC, open on this one.
    let mut mirror: Option<std::process::Child> = None;
    // Whether switchers or copies are open anywhere for this PC.
    let mut copies_out = false;
    // Hotkeys another PC shared while the focus was away, to take back home.
    let mut pending_hotkeys: Option<String> = None;
    // The switcher open on this PC for picking, to close it on a second press.
    let shown_here = window::Slot::default();
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
    {
        let wake = wake.clone();
        crate::threads::spawn("target news", move || {
            for news in controlled {
                let woken = match news {
                    TargetNews::Controlled(here) => Wake::Controlled(here),
                    TargetNews::Arranged { from, position } => Wake::Arranged(from, position),
                    TargetNews::Group { members, by } => Wake::Group(members, by),
                    TargetNews::Hotkeys(lines) => Wake::Hotkeys(lines),
                    TargetNews::Audio(lines) => Wake::Audio(lines),
                    TargetNews::Version { from, version } => Wake::Version(from, version),
                };
                if wake.send(woken).is_err() {
                    return;
                }
            }
        });
    }
    let mut in_control = InControl::default();
    // A hotkey being recorded: what was heard, where to answer, since when.
    let mut recording: Option<(Recording, Sender<String>, Instant)> = None;
    // The tray icon (PRD 0009); Mickey works the same without one.
    let (clicks, picked) = mpsc::channel();
    let first = Status {
        focus: None,
        here: true,
        peers: peers
            .iter()
            .map(|peer| (peer.name.clone(), peer.link.is_some()))
            .collect(),
        update: None,
    };
    let tray = Tray::start(first, clicks)
        .inspect_err(|err| eprintln!("mickey: {err}"))
        .ok();
    {
        let wake = wake.clone();
        crate::threads::spawn("tray clicks", move || {
            for click in picked {
                if wake.send(Wake::Tray(click)).is_err() {
                    return;
                }
            }
        });
    }
    let mut double_tap_window = hotkeys.double_tap_window();
    let mut switcher = Switcher::with_hotkeys(hotkeys);
    // Dropping a `Grab` gives its device back, on every way out of here,
    // including `?` and panics.
    let mut grabbed = None;
    let mut next_ping = start;
    let bench = std::env::var_os("MICKEY_BENCH").is_some();
    // Names never change; which peers are online is refreshed every turn.
    let mut names: Vec<String> = peers.iter().map(|peer| peer.name.clone()).collect();
    // PCs the group introduced that the person here was asked about, so
    // nobody is asked twice in a run, whatever the answer (#184).
    let mut asked: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
    // Which peers were online when the group map was last shared.
    let mut shared_with: Vec<bool> = Vec::new();
    let this_pc = monitors::this_pc().unwrap_or_else(|err| {
        eprintln!("mickey: {err}; `mickey peers` will not show this PC's monitors");
        Vec::new()
    });
    let mut online: Vec<bool> = Vec::with_capacity(peers.len());
    // PCs that were off when the owner saved a layout: told when they
    // connect (#288).
    let mut arrange_unsent: std::collections::HashSet<String> = std::collections::HashSet::new();
    loop {
        for (index, link) in back.try_iter() {
            // A peer is one PC, known by its key: another trusted PC that
            // took over its address gets nothing meant for it (#184).
            match peers[index].key {
                Some(key) if key != *link.peer_key() => {
                    println!(
                        "mickey: {} answered with another PC's key; refused. If it was reinstalled, remove it in the settings and connect again",
                        peers[index].name
                    );
                    peers[index].new_key = true;
                    continue;
                }
                Some(_) => {}
                None => save_key(&peers[index].name, link.peer_key()),
            }
            read_peer(index, &link, &pong_sender, start);
            if let Some(mover) = &mover {
                open_data_link(
                    index,
                    peers[index].address,
                    *link.peer_key(),
                    &shared,
                    mover,
                );
            }
            peers[index].key = Some(*link.peer_key());
            peers[index].new_key = false;
            peers[index].link = Some(link);
            peers[index].heard = Instant::now();
            println!("mickey: {} is online", peers[index].name);
            // A layout saved while it was off reaches it now (#288).
            if arrange_unsent.remove(&peers[index].name)
                && let (Some([x, y]), Some(link)) = (positions[index], &mut peers[index].link)
                && link.send(&Message::Arrange { x: -x, y: -y }).is_ok()
            {
                println!(
                    "mickey: told {} the layout saved while it was off",
                    peers[index].name
                );
            }
        }
        // Where to put the cursor here, coming back from a peer's edge.
        let mut home_at = None;
        // A peer refused the input: bring the focus home.
        let mut refused = false;
        // A card picked in the switcher shown on the focused peer.
        let mut remote_pick = None;
        // Any switcher answered, here or there: the copies can close.
        let mut picked = false;
        let peer_count = peers.len();
        for (index, news) in pongs.try_iter() {
            peers[index].heard = Instant::now();
            match news {
                News::Picked(card) => {
                    picked = true;
                    if switcher.focus() == Focus::Remote(index) {
                        remote_pick = Some(card);
                    }
                }
                News::Refused => {
                    if switcher.focus() == Focus::Remote(index) {
                        println!(
                            "mickey: {} refused the keyboard and mouse (a window running as administrator in front?); they are back here",
                            peers[index].name
                        );
                        refused = true;
                    }
                }
                News::Leave(x, y) => {
                    if switcher.focus() == Focus::Remote(index)
                        && let Some(position) = positions[index]
                    {
                        home_at = edges::back(&this_pc, position, (x, y));
                    }
                }
                News::RoundTrip(micros) => peers[index].latency.add_round_trip(micros),
                News::Monitors(monitors) => {
                    let sizes: Vec<String> = monitors
                        .iter()
                        .map(|m| format!("{}x{} at {},{}", m.width, m.height, m.x, m.y))
                        .collect();
                    println!(
                        "mickey: {} has {} monitor(s): {}",
                        peers[index].name,
                        monitors.len(),
                        sizes.join("; ")
                    );
                    peers[index].monitors = monitors;
                }
            }
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
        if let Some(was) = in_control.focus_is(focus) {
            if let Some(mover) = &mover {
                mover.moved(was, focus);
            }
            // Tell the peers the keyboard left and where it went.
            for (index, now_here) in [(was, false), (focus, true)] {
                if let Some(link) = index.and_then(|index| peers[index].link.as_mut()) {
                    let _ = link.send(&Message::Focus { here: now_here });
                }
            }
        }
        let here = in_control.here();
        let waiting_update = updates.as_ref().and_then(update::Updates::waiting);
        let changed = published.as_ref().is_none_or(|last: &Status| {
            last.focus != focus
                || last.here != here
                || last.update != waiting_update
                // A PC the group added makes the list longer.
                || last.peers.len() != peers.len()
                || last
                    .peers
                    .iter()
                    .zip(&peers)
                    .any(|((_, up), peer)| *up != peer.link.is_some())
        });
        if changed {
            let status = Status {
                focus,
                here,
                peers: peers
                    .iter()
                    .map(|peer| (peer.name.clone(), peer.link.is_some()))
                    .collect(),
                update: waiting_update,
            };
            if let Some(control) = &control {
                control.publish(status.clone());
            }
            if let Some(tray) = &tray {
                tray.show(&status);
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
                let Some(link) = &mut peer.link else {
                    continue;
                };
                if peer.heard.elapsed() > SILENT_LIMIT {
                    println!("mickey: {} stopped answering", peer.name);
                    lost.push(index);
                } else if link.send(&ping).is_err() {
                    lost.push(index);
                }
            }
            let remote = matches!(switcher.focus(), Focus::Remote(_));
            next_ping = now
                + if bench && remote {
                    BENCH_PING_EVERY
                } else {
                    PING_EVERY
                };
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
        let target = audio_target(&audio, shared.identity.public(), &peers, &online);
        if target.map(|(_, to)| to) != sending.as_ref().map(|(_, sending)| sending.to()) {
            // Dropping the old one gives the speakers back.
            sending = None;
            if let (Some((index, to)), Some(mover)) = (target, &mover) {
                let mover = mover.clone();
                let give_key = move |hex: &str| mover.give_audio_key(index, hex);
                match crate::audio::Sending::start(to, audio.mode, give_key) {
                    Ok(started) => sending = Some((index, started)),
                    Err(err) => eprintln!("mickey: {err}"),
                }
            }
        }
        let mut open = false;
        // The switcher shown on the focused peer instead (it is where the
        // owner is looking).
        let mut open_on = None;
        let left = switcher.focus();
        let mut act = |action| match action {
            Action::OpenSwitcher => open = true,
            Action::OpenSwitcherOn(peer) => open_on = Some(peer),
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
                    report_latency(&peers[index]);
                }
            }
        };
        if home_at.is_some() || refused {
            switcher.go_home(&mut act);
        }
        // 0 is this PC; `i + 1` is peer `i`; anything else is "none".
        match remote_pick.map(usize::from) {
            Some(0) => switcher.go_home(&mut act),
            Some(card) if card <= peer_count && switcher.focus() != Focus::Remote(card - 1) => {
                switcher.go_home(&mut act);
                switcher.go_to(Some(card - 1), &online);
            }
            _ => {}
        }
        let mut asked_for_peers = None;
        let mut pushed = None;
        let mut quit = false;
        let mut placed = None;
        let mut heard_map = None;
        let mut accepted = None;
        let mut heard_hotkeys = None;
        let mut heard_audio = None;
        let mut heard_version = None;
        // A PC to connect to from the settings: handled below, like `join`.
        let mut connecting = None;
        let mut asked_to_arrange = None;
        match wakes.recv_timeout(wait) {
            // While recording a hotkey, keys are named, never acted on.
            Ok(Wake::Input(InputEvent::Key { code, pressed })) if recording.is_some() => {
                let answer = recording.as_mut().and_then(|(record, _, _)| {
                    match record.key(code, pressed, Instant::now()) {
                        Heard::Waiting => None,
                        Heard::Chord(text) => Some(format!("ok: {text}")),
                        Heard::DoubleTap(key) => Some(format!("ok: double {key}")),
                        Heard::Mistake(why) => Some(format!("error: {why}")),
                    }
                });
                if let Some(answer) = answer
                    && let Some((_, reply, _)) = recording.take()
                {
                    let _ = reply.send(answer);
                }
            }
            Ok(Wake::Control(Request {
                order: Order::Connect { address, name },
                reply,
            })) => connecting = Some((address, name, reply)),
            Ok(Wake::Control(request)) if request.order == Order::Record => {
                recording = Some((
                    Recording::new(double_tap_window),
                    request.reply,
                    Instant::now(),
                ));
            }
            Ok(Wake::Input(event)) => {
                switcher.handle(event, &online, &mut act);
                if let InputEvent::Motion { dx, dy } = event
                    && switcher.focus() == Focus::Local
                {
                    pushed = edge_check.feed((dx, dy));
                }
            }
            Ok(Wake::Controlled(here)) => in_control.peer_says(here),
            // Both handled below, once `act` no longer borrows the peers.
            Ok(Wake::Arranged(from, position)) => placed = Some((from, position)),
            Ok(Wake::Group(members, by)) => heard_map = Some((members, by)),
            Ok(Wake::Accepted(member, by)) => accepted = Some((member, by)),
            Ok(Wake::Hotkeys(lines)) => heard_hotkeys = Some(lines),
            Ok(Wake::Audio(lines)) => heard_audio = Some(lines),
            Ok(Wake::Version(from, version)) => heard_version = Some((from, version)),
            Ok(Wake::Control(request)) if request.order == Order::Arranged => {
                asked_to_arrange = Some(request.reply);
            }
            Ok(Wake::Tray(tray::Click::Quit)) => quit = true,
            Ok(Wake::Control(request)) if request.order == Order::Quit => {
                let _ = request.reply.send("ok: Mickey is quitting".into());
                quit = true;
            }
            Ok(Wake::Control(control::Request {
                order: Order::Forget(fingerprint),
                reply,
            })) => {
                let forgot = shared
                    .trusted
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .forget(&fingerprint);
                let _ = reply.send(match forgot {
                    Ok(true) => format!("ok: forgot {fingerprint}; it must pair again to connect"),
                    Ok(false) => {
                        format!("error: no trusted peer has the fingerprint {fingerprint}")
                    }
                    Err(err) => format!("error: {err}"),
                });
            }
            Ok(Wake::Tray(tray::Click::Settings)) => open_window("settings"),
            // The settings window says how to update, until it can do it.
            Ok(Wake::Tray(tray::Click::Update)) => open_window("settings"),
            Ok(Wake::Tray(tray::Click::History)) => open_window("history"),
            Ok(Wake::Tray(tray::Click::ListenHere)) => {
                match listen_here(shared.identity.public()) {
                    Ok(taken) => {
                        if let Some(mover) = &mover {
                            mover.share_audio(&taken.shared_lines());
                        }
                        audio = taken;
                    }
                    Err(err) => {
                        eprintln!("mickey: cannot make this PC the one that listens: {err}")
                    }
                }
            }
            Ok(Wake::Tray(tray::Click::Home)) => switcher.go_home(&mut act),
            Ok(Wake::Tray(tray::Click::Go(peer))) => {
                // As from the switcher: only from here, only to a PC online.
                if switcher.focus() == Focus::Local && online[peer] {
                    switcher.go_to(Some(peer), &online);
                }
            }
            Ok(Wake::Pick(pick)) => {
                switcher_open = false;
                picked = true;
                match pick {
                    Ok(peer) => switcher.go_to(peer, &online),
                    Err(err) => {
                        // Without the window, behave like the M0 hotkey.
                        eprintln!("mickey: cannot show the switcher: {err}");
                        switcher.go_to(online.iter().position(|&up| up), &online);
                    }
                }
            }
            // Answered below, once `act` no longer borrows the peers.
            Ok(Wake::Control(request)) if request.order == Order::History => {
                let answer = match (&history, &keeper) {
                    (Some(history), _) => {
                        let history = history.lock().unwrap_or_else(|p| p.into_inner());
                        history::lines(&history, std::time::SystemTime::now())
                    }
                    (None, Some(_)) => {
                        "error: the history is off on this PC; another clipboard manager keeps it ([clipboard] history in config.toml)".to_string()
                    }
                    (None, None) => "error: the clipboard is not shared on this PC".to_string(),
                };
                let _ = request.reply.send(answer);
            }
            Ok(Wake::Control(Request {
                order: Order::HistoryCopy(id),
                reply,
            })) => {
                let entry = history.as_ref().and_then(|history| {
                    let history = history.lock().unwrap_or_else(|p| p.into_inner());
                    history.get(id).and_then(|entry| entry.content())
                });
                let answer = match (entry, &keeper) {
                    (Some(content), Some(keeper)) => {
                        keeper.write(content);
                        "ok: copied".to_string()
                    }
                    _ => format!("error: no history entry {id}"),
                };
                let _ = reply.send(answer);
            }
            Ok(Wake::Control(request)) if request.order == Order::Discover => {
                let _ = request.reply.send(found_text(found.as_ref()));
            }
            Ok(Wake::Control(request)) if request.order == Order::Peers => {
                asked_for_peers = Some(request.reply);
            }
            Ok(Wake::Control(request)) => {
                let answer = obey(request.order, &names, &online, &mut switcher, &mut act);
                let _ = request.reply.send(answer);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        drop(act);
        if recording
            .as_ref()
            .is_some_and(|(_, _, since)| since.elapsed() > control::RECORD_TIMEOUT)
            && let Some((_, reply, _)) = recording.take()
        {
            let _ = reply.send("error: no keys were pressed".into());
        }
        if let Some((from, version)) = heard_version {
            // A peer on a newer Mickey: there is an update for this PC.
            if let Some(updates) = &updates {
                updates.heard(&version);
            }
            for peer in peers.iter_mut().filter(|peer| peer.address.ip() == from) {
                peer.version = Some(version.clone());
            }
        }
        if let Some((from, position)) = placed
            && let Some(index) = peers.iter().position(|peer| peer.address.ip() == from)
        {
            positions[index] = Some(position);
            save_position(&peers[index].name, position);
        }
        if let Some(reply) = asked_to_arrange {
            let _ = reply.send(arrange(&mut positions, &mut peers, &mut arrange_unsent));
            // The hotkeys saved with it: used here at once, and shared.
            if let Ok(Some(config)) = config::Config::load() {
                use_hotkeys(&config, &mut switcher, &mut double_tap_window);
                interface = config.interface.clone();
                if let Some(mover) = &mover {
                    mover.share_hotkeys(&config.hotkeys.to_lines());
                    mover.share_audio(&config.audio.shared_lines());
                }
                audio = config.audio.clone();
            }
            // Everyone gets the new places, not only the PCs moved.
            shared_with.clear();
        }
        if let Some((address, name, reply)) = connecting {
            // The owner is connecting a PC now: it may answer back.
            trust::open_pairing();
            let answer = if peers.iter().any(|peer| peer.address == address) {
                format!("error: {address} is already one of this PC's PCs")
            } else {
                match add_peer(&mut peers, &mut positions, &name, address, None) {
                    Ok(index) => {
                        let name = peers[index].name.clone();
                        names.push(name.clone());
                        // Pairs: the code shows on both PCs.
                        connect_first(index, name.clone(), address, &shared, &back_sender);
                        format!("ok: pairing with {name}: check that both PCs show the same code")
                    }
                    Err(err) => format!("error: {err}"),
                }
            };
            let _ = reply.send(answer);
        }
        if let Some(lines) = heard_hotkeys {
            pending_hotkeys = Some(lines);
        }
        if let Some(lines) = heard_audio {
            match take_audio(&lines) {
                Ok(taken) => audio = taken,
                Err(err) => eprintln!("mickey: cannot take who listens from another PC: {err}"),
            }
        }
        // Never while the keyboard is on another PC: new hotkeys could leave
        // no way home from there (#182). They wait until it is back.
        if switcher.focus() == Focus::Local
            && let Some(lines) = pending_hotkeys.take()
        {
            let taken = config::Hotkeys::from_lines(&lines).and_then(|hotkeys| {
                let path = config::path()?;
                let text = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
                let new = config::set_hotkeys(&text, &hotkeys)?;
                // Checked as hotkeys before anything is written.
                config::parse(&new)?.hotkeys()?;
                config::write(&path, &new)?;
                config::Config::load()?.ok_or_else(|| "config.toml is gone".to_string())
            });
            match taken {
                Ok(config) => {
                    use_hotkeys(&config, &mut switcher, &mut double_tap_window);
                    println!("mickey: took the hotkeys saved on another PC");
                }
                Err(err) => eprintln!("mickey: cannot take another PC's hotkeys: {err}"),
            }
        }
        if let Some((member, by)) = accepted
            && let Some(index) = join(member, &by, &shared, &mut peers, &mut positions)
        {
            names.push(peers[index].name.clone());
            reconnect(index, peers[index].address, &shared, &back_sender);
        }
        if let Some((members, by)) = heard_map {
            let me = *shared.identity.public();
            for member in group::as_seen_by(&members, &me) {
                let trusted = shared
                    .trusted
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_trusted(&member.key);
                // A PC this one never paired with joins only once the
                // person here says so (#184), asked once per run.
                if !trusted {
                    if group::is_local(member.address.ip()) && asked.insert(member.key) {
                        let introducer =
                            peers.iter().find(|peer| peer.key == Some(by)).map_or_else(
                                || crate::identity::fingerprint(&by),
                                |peer| peer.name.clone(),
                            );
                        ask_to_join(member, by, introducer, &wake);
                    }
                    continue;
                }
                if let Some(index) = join(member, &by, &shared, &mut peers, &mut positions) {
                    names.push(peers[index].name.clone());
                    reconnect(index, peers[index].address, &shared, &back_sender);
                }
            }
        }
        // Share the group map when the PCs online changed (one came, or
        // went), so a PC paired with this one reaches every other.
        let now_online: Vec<bool> = peers.iter().map(|peer| peer.link.is_some()).collect();
        if now_online != shared_with
            && let Some(mover) = &mover
        {
            let me = group::Member {
                key: *shared.identity.public(),
                name: my_name.clone(),
                address: SocketAddr::from(([0, 0, 0, 0], my_port)),
                position: Some([0, 0]),
            };
            let others: Vec<group::Member> = peers
                .iter()
                .zip(&positions)
                .filter_map(|(peer, position)| {
                    Some(group::Member {
                        key: peer.key?,
                        name: peer.name.clone(),
                        address: peer.address,
                        position: *position,
                    })
                })
                .collect();
            mover.share_map(&group::encode(&me, &others));
            shared_with = now_online;
        }
        if quit {
            // Returning drops the grab: the devices go back to this PC.
            println!("mickey: quitting, as asked");
            return Ok(());
        }
        if let Some((x, y)) = home_at
            && let Err(err) = emit::place(x, y)
        {
            eprintln!("mickey: cannot place the cursor: {err}");
        }
        let crossed =
            pushed.and_then(|delta| edge_crossed(&positions, &peers, &this_pc, delta, &input));
        if let Some((index, (x, y))) = crossed {
            switcher.go_to(Some(index), &online);
            if let Some(link) = &mut peers[index].link
                && link.send(&Message::Enter { x, y }).is_err()
            {
                lost.push(index);
            }
        }
        if let Some(reply) = asked_for_peers {
            let _ = reply.send(peers_text(&this_pc, &peers, &online));
        }
        if switcher.focus() == Focus::Local && left != Focus::Local {
            println!("mickey: focus is back here");
        }
        // The switcher shows on the PC in use, and a copy on every other
        // online PC, so it is seen wherever the owner looks (PRD 0007).
        // The hotkey again while switchers are open closes them all.
        let toggled_off = copies_out && (open || open_on.is_some());
        let in_use = match open_on {
            _ if toggled_off => None,
            Some(peer) => Some(Some(peer)),
            None if open && !switcher_open => Some(None),
            None => None,
        };
        if let Some(in_use) = in_use {
            let cards = switcher_cards(
                in_use.map_or(0, |peer| peer + 1),
                &my_name,
                &peers,
                &positions,
                &interface,
            );
            if let Some(mover) = &mover {
                for (peer, other) in peers.iter().enumerate() {
                    if Some(peer) == in_use {
                        mover.show_switcher(peer, &cards.join("\n"));
                    } else if other.link.is_some() {
                        mover.mirror_switcher(peer, &cards.join("\n"));
                    }
                }
            }
            match in_use {
                Some(_) => mirror = open_mirror(&cards, &interface).or(mirror),
                None => {
                    switcher_open = true;
                    open_switcher(&cards, &interface, &wake, &shown_here);
                }
            }
            copies_out = true;
        }
        // Closed once a pick is made, or when the focus moved some other way.
        let moved = in_use.is_none() && switcher.focus() != left;
        if copies_out && (picked || toggled_off || moved) {
            copies_out = false;
            window::close(&shown_here);
            if let Some(mover) = &mover {
                mover.close_switchers();
            }
            if let Some(mut window) = mirror.take() {
                let _ = window.kill();
                let _ = window.wait();
            }
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

/// A peer's first connection, on its own thread: pairs if needed, then
/// hands the link to the main loop; if it fails, retries like any peer.
fn connect_first(
    index: usize,
    name: String,
    to: SocketAddr,
    shared: &Arc<Shared>,
    back: &Sender<(usize, Link)>,
) {
    let (shared, back) = (Arc::clone(shared), back.clone());
    crate::threads::spawn("connect", move || {
        match connect_and_pair(&shared.identity, &shared.trusted, &name, to) {
            Ok(link) => {
                let _ = back.send((index, link));
            }
            Err(err) => {
                println!("mickey: {name} ({to}) is offline: {err}");
                reconnect(index, to, &shared, &back);
            }
        }
    });
}

/// Connects, asks to pair if the key is unknown, and waits for the peer to
/// accept us.
fn connect_and_pair(
    identity: &Identity,
    trusted: &trust::Shared,
    name: &str,
    to: SocketAddr,
) -> Result<Link, String> {
    let mut link = transport::connect(to, identity).map_err(|err| err.to_string())?;
    let peer_key = *link.peer_key();
    let code = link.pairing_code();
    if !trust::trust_or_ask(trusted, &peer_key, to, &code, name)? {
        return Err("not paired; nothing was sent".into());
    }
    println!("mickey: waiting for {name} to accept this PC");
    transport::hello(&mut link).map_err(|err| match err.kind() {
        std::io::ErrorKind::UnexpectedEof => "it refused to pair with this PC".to_string(),
        _ => err.to_string(),
    })?;
    Ok(link)
}

/// Opens the data link to a connected peer in the background and hands it
/// to the mover. A peer without one (an older Mickey) only misses the
/// shared clipboard.
/// The data link must answer with `key`, the key of the input link just
/// opened to the same peer: otherwise whoever answers the data port (a PC
/// that took over the address) would get the clipboard and the group map.
fn open_data_link(
    index: usize,
    input: SocketAddr,
    key: [u8; 32],
    shared: &Arc<Shared>,
    mover: &Mover,
) {
    let (shared, mover) = (Arc::clone(shared), mover.clone());
    crate::threads::spawn("open data link", move || {
        match transport::connect_data(input, &shared.identity) {
            Ok(link) if *link.peer_key() == key => mover.link(index, link),
            Ok(_) => eprintln!(
                "mickey: the data port of {input} answered with another PC's key; no shared clipboard with it"
            ),
            Err(err) => eprintln!("mickey: no shared clipboard with {input}: {err}"),
        }
    });
}

/// Retries a trusted peer in the background until it answers, then hands
/// its link to the main loop. Never asks to pair: a peer whose key is not
/// trusted is left alone.
fn reconnect(index: usize, to: SocketAddr, shared: &Arc<Shared>, back: &Sender<(usize, Link)>) {
    let (shared, back) = (Arc::clone(shared), back.clone());
    crate::threads::spawn("reconnect", move || {
        loop {
            thread::sleep(RETRY_EVERY);
            let Ok(mut link) = transport::connect(to, &shared.identity) else {
                continue;
            };
            let trusted = shared
                .trusted
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .is_trusted(link.peer_key());
            if !trusted {
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

/// The switcher's cards: the focused card's id first, then one card per
/// PC, as `id=online:name` or `id=offline:name`, in the order to show them
/// (`Interface::switcher_sort`). Id 0 is this PC, `i + 1` is peer `i`.
fn switcher_cards(
    focused: usize,
    my_name: &str,
    peers: &[Peer],
    positions: &[Option<[i32; 2]>],
    interface: &config::Interface,
) -> Vec<String> {
    let mut all = vec![(my_name, Some([0, 0]), true)];
    for (peer, place) in peers.iter().zip(positions) {
        all.push((peer.name.as_str(), *place, peer.link.is_some()));
    }
    let sortable: Vec<(&str, Option<[i32; 2]>)> =
        all.iter().map(|&(name, place, _)| (name, place)).collect();
    let mut cards = vec![focused.to_string()];
    for id in interface.switcher_sort(&sortable) {
        let (name, _, online) = all[id];
        let state = if online { "online" } else { "offline" };
        cards.push(format!("{id}={state}:{name}"));
    }
    cards
}

/// `mickey-ui` arguments for the switcher (a view-only copy if `mirror`).
fn switcher_args(cards: &[String], interface: &config::Interface, mirror: bool) -> Vec<String> {
    let mut args = vec!["switcher".to_string()];
    if mirror {
        args.push("--mirror".to_string());
    }
    args.extend(["--at".to_string(), interface.switcher_position.clone()]);
    if interface.switcher_vertical {
        args.push("--vertical".to_string());
    }
    args.extend(cards.iter().cloned());
    args
}

/// Opens a view-only copy of the switcher on this PC; the caller kills it
/// once a pick is made.
fn open_mirror(cards: &[String], interface: &config::Interface) -> Option<std::process::Child> {
    std::env::current_exe()
        .map(|exe| exe.with_file_name(format!("mickey-ui{}", std::env::consts::EXE_SUFFIX)))
        .and_then(|ui| {
            Command::new(ui)
                .args(switcher_args(cards, interface, true))
                .stdout(std::process::Stdio::null())
                .spawn()
        })
        .ok()
}

/// Shows `mickey-ui switcher` (installed next to this program) on its own
/// thread, and sends the owner's pick when the window closes: `Some(peer)`,
/// `None` for this PC or Esc, or an error if the window could not run.
/// The GUI is a separate program so this one stays small (PRD 0003).
fn open_switcher(
    cards: &[String],
    interface: &config::Interface,
    wake: &Sender<Wake>,
    slot: &window::Slot,
) {
    let args = switcher_args(cards, interface, false);
    let (wake, slot) = (wake.clone(), slot.clone());
    crate::threads::spawn("switcher", move || {
        let pick = match window::run(&args, &slot) {
            // The window prints the card's id; 0 is this PC.
            window::Outcome::Answered(text) => text
                .parse::<usize>()
                .map(|card| card.checked_sub(1))
                .map_err(|err| format!("mickey-ui answered badly: {err}")),
            window::Outcome::Closed => Ok(None),
            window::Outcome::Failed(err) => Err(err),
        };
        let _ = wake.send(Wake::Pick(pick));
    });
}

/// What a peer sends back on its link.
enum News {
    /// A pong: the round-trip time in microseconds.
    RoundTrip(u32),
    Monitors(Vec<Monitor>),
    /// The cursor is pushing off the peer's desktop onto this point of it.
    Leave(i32, i32),
    /// The peer's system refused the input sent to it.
    Refused,
    /// The owner picked a card in the switcher shown on the peer.
    Picked(u8),
}

/// Reads what a peer sends, on its own thread, and passes it on as
/// `News`: pongs as round-trip times, its monitors, and the rest.
fn read_peer(index: usize, link: &Link, news: &Sender<(usize, News)>, start: Instant) {
    let (Ok(mut reader), news) = (link.try_clone(), news.clone()) else {
        return;
    };
    crate::threads::spawn("peer reader", move || {
        while let Ok(message) = reader.recv() {
            let item = match message {
                Message::Pong { id } => News::RoundTrip(latency::round_trip(start, id)),
                Message::Monitors { count, list } => {
                    News::Monitors(list[..usize::from(count)].to_vec())
                }
                Message::Leave { x, y } => News::Leave(x, y),
                Message::Refused => News::Refused,
                Message::Picked { card } => News::Picked(card),
                _ => continue,
            };
            if news.send((index, item)).is_err() {
                return;
            }
        }
    });
}

fn report_latency(peer: &Peer) {
    if let Some((p50, p99)) = peer.latency.percentiles() {
        println!(
            "mickey: one-way latency to {} over the last {} pings: p50 {p50} µs, p99 {p99} µs",
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
        let (answer, actions) = run(Order::Switcher, &mut switcher, &[true, true]);
        assert!(answer.starts_with("ok"));
        assert_eq!(actions, [Action::OpenSwitcherOn(1)], "on the screen in use");
        let (answer, actions) = run(Order::Home, &mut switcher, &[true, true]);
        assert!(answer.starts_with("ok"));
        assert_eq!(actions, [Action::Ungrab]);
        assert_eq!(switcher.focus(), Focus::Local);
    }

    #[test]
    fn the_last_change_wins() {
        let mut in_control = InControl::default();
        assert!(in_control.here());
        // This PC's keyboard goes to peer 0: not here; peer 0 is told.
        assert_eq!(in_control.focus_is(Some(0)), Some(None));
        assert!(!in_control.here());
        // Another peer's keyboard comes over: here again.
        in_control.peer_says(true);
        assert!(in_control.here());
        // No move of our own keyboard: nothing to tell, still here.
        assert_eq!(in_control.focus_is(Some(0)), None);
        assert!(in_control.here());
        // Our keyboard comes home: here, and peer 0 is told it left.
        assert_eq!(in_control.focus_is(None), Some(Some(0)));
        assert!(in_control.here());
        in_control.peer_says(false);
        assert!(!in_control.here());
    }
}
