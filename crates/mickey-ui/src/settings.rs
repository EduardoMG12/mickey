//! The settings window (PRD 0006, step 3 of the plan): edits config.toml.
//! What it saves is checked with `mickey_config::parse`, the same rules
//! `mickey` applies, before anything is written.

use crate::layout;
use eframe::egui;
use mickey_config::peers::Peers;
use mickey_config::{self as config, Config, SWITCHER_POSITIONS};
use std::process::Command;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcRow {
    pub name: String,
    pub address: String,
    pub hotkey: String,
    pub position: Option<[i32; 2]>,
    /// Its public key, written by Mickey when it first connects; kept
    /// as it is, never shown or edited.
    pub key: Option<String>,
}

/// Everything editable, as the text the fields show.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
    /// Empty: the host name.
    pub name: String,
    pub controls_others: bool,
    pub controlled: bool,
    pub listen: String,
    pub pcs: Vec<PcRow>,
    pub next: String,
    pub home: String,
    pub switcher: String,
    pub forward: String,
    pub back: String,
    /// A key name; empty turns the double tap off.
    pub double_tap: String,
    /// How close the two taps must be, in milliseconds.
    pub double_tap_ms: u64,
    pub position: String,
    /// The switcher as a column.
    pub vertical: bool,
    /// Keep a clipboard history here (off: another manager keeps it).
    pub history: bool,
    /// The switcher's custom order, by name; empty: as the layout goes.
    pub order: Vec<String>,
    pub speed: i32,
}

impl Form {
    pub fn from_config(config: &Config) -> Form {
        let text = |value: &Option<String>| value.clone().unwrap_or_default();
        Form {
            name: text(&config.this_pc.name),
            controls_others: config.this_pc.controls_others,
            controlled: config.this_pc.controlled,
            listen: config.this_pc.listen.to_string(),
            pcs: config
                .peers
                .iter()
                .map(|peer| PcRow {
                    name: peer.name.clone(),
                    address: peer.address.to_string(),
                    hotkey: text(&peer.hotkey),
                    position: peer.position,
                    key: peer.key.clone(),
                })
                .collect(),
            next: text(&config.hotkeys.next),
            home: text(&config.hotkeys.home),
            switcher: text(&config.hotkeys.switcher),
            forward: text(&config.hotkeys.forward),
            back: text(&config.hotkeys.back),
            double_tap: config
                .hotkeys
                .double_tap
                .clone()
                .unwrap_or_else(|| config::DEFAULT_DOUBLE_TAP.into()),
            double_tap_ms: config
                .hotkeys
                .double_tap_ms
                .unwrap_or(config::DEFAULT_DOUBLE_TAP_MS),
            position: config.interface.switcher_position.clone(),
            vertical: config.interface.switcher_vertical,
            history: config.clipboard.history,
            order: config.interface.switcher_order.clone(),
            speed: config.touchpad.speed,
        }
    }

    /// The form as config.toml, with a comment on every setting. Empty
    /// optional fields are left out.
    pub fn to_toml(&self) -> String {
        let mut out =
            String::from("# Mickey settings; the settings window rewrites this file.\n\n");
        out += "[this_pc]\n";
        if !self.name.trim().is_empty() {
            out += &format!(
                "name = {}    # how the other PCs see this one\n",
                quote(self.name.trim())
            );
        }
        out += &format!(
            "controls_others = {}    # send this PC's keyboard and mouse to the PCs below\n",
            self.controls_others
        );
        out += &format!(
            "controlled = {}         # let paired PCs drive this one\n",
            self.controlled
        );
        out += &format!(
            "listen = {}    # where the other PCs connect\n",
            quote(&self.listen)
        );
        for pc in &self.pcs {
            out += "\n[[peer]]\n";
            out += &format!("name = {}\n", quote(&pc.name));
            out += &format!("address = {}\n", quote(&pc.address));
            if !pc.hotkey.trim().is_empty() {
                out += &format!(
                    "hotkey = {}    # straight to this PC\n",
                    quote(pc.hotkey.trim())
                );
            }
            if let Some([x, y]) = pc.position {
                out += &format!("position = [{x}, {y}]    # where it sits next to this PC\n");
            }
            if let Some(key) = &pc.key {
                out += &format!(
                    "key = {}    # this PC's identity, set by Mickey\n",
                    quote(key)
                );
            }
        }
        let hotkeys: Vec<(&str, &str)> = [
            ("next", &self.next),
            ("home", &self.home),
            ("switcher", &self.switcher),
            ("forward", &self.forward),
            ("back", &self.back),
        ]
        .into_iter()
        .map(|(key, value)| (key, value.trim()))
        .filter(|(_, value)| !value.is_empty())
        .collect();
        out += "\n[hotkeys]\n";
        for (key, value) in hotkeys {
            out += &format!("{key} = {}\n", quote(value));
        }
        // Written even when empty: "" is how the double tap is turned off.
        out += &format!(
            "double_tap = {}    # a key tapped twice quickly: next PC\n",
            quote(self.double_tap.trim())
        );
        out += &format!(
            "double_tap_ms = {}    # how close the two taps must be\n",
            self.double_tap_ms
        );
        out += "\n[interface]\n";
        out += &format!("switcher_position = {}\n", quote(&self.position));
        if self.vertical {
            out += "switcher_vertical = true\n";
        }
        if !self.order.is_empty() {
            let names: Vec<String> = self.order.iter().map(|name| quote(name)).collect();
            out += &format!("switcher_order = [{}]\n", names.join(", "));
        }
        if !self.history {
            out += "\n[clipboard]\n";
            out += "history = false    # another clipboard manager keeps the history\n";
        }
        out += "\n[touchpad]\n";
        out += &format!("speed = {}    # higher is faster\n", self.speed);
        out
    }
}

/// A TOML basic string: quotes and backslashes escaped.
fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

pub struct Settings {
    form: Form,
    saved: Form,
    fingerprint: String,
    /// What the running Mickey said about the PCs; `Err` says why not.
    peers: Result<Peers, String>,
    asked: Instant,
    drag: layout::Drag,
    tab: Tab,
    message: String,
    /// A hotkey being recorded: its row, and where `mickey record` answers.
    recording: Option<(usize, std::sync::mpsc::Receiver<Result<String, String>>)>,
    /// The hotkey row being typed into, if any.
    typing: Option<usize>,
    /// The PCs heard on the network, from `mickey discover`.
    nearby: Vec<Nearby>,
    /// Whether Mickey starts with this PC, as `mickey autostart` says.
    autostart: bool,
    /// The answer of a change to it, still on its way (Windows may ask for
    /// an administrator first).
    autostart_change: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// The mascot in the header, loaded on the first frame.
    mascot: Option<egui::TextureHandle>,
    /// When config.toml last changed as far as this window knows, and when
    /// it last looked: another PC may place itself and rewrite the file.
    config_changed: Option<std::time::SystemTime>,
    looked: Instant,
}

/// The Hotkeys tab's rows: what each is for, and what empty means.
const HOTKEY_ROWS: [(&str, &str); 6] = [
    ("Next PC / switcher", config::DEFAULT_NEXT),
    ("Back to this PC", "the hotkey above + 0"),
    ("Open the switcher", config::DEFAULT_SWITCHER),
    ("Next PC (one hand)", config::DEFAULT_FORWARD),
    ("Previous PC (one hand)", config::DEFAULT_BACK),
    ("Double tap (empty: off)", ""),
];
const DOUBLE_TAP_ROW: usize = 5;

fn hotkey_field(form: &mut Form, row: usize) -> &mut String {
    match row {
        0 => &mut form.next,
        1 => &mut form.home,
        2 => &mut form.switcher,
        3 => &mut form.forward,
        4 => &mut form.back,
        _ => &mut form.double_tap,
    }
}

/// Puts what `mickey record` heard into the row, and says how it went.
fn recorded(form: &mut Form, row: usize, result: Result<String, String>) -> String {
    let answer = match result {
        Ok(answer) => answer,
        Err(err) => return format!("Could not record: {err}"),
    };
    let heard = answer.trim().strip_prefix("ok: ").map(str::to_string);
    match (heard, row == DOUBLE_TAP_ROW) {
        (Some(heard), true) => match heard.strip_prefix("double ") {
            Some(key) => {
                *hotkey_field(form, row) = key.to_string();
                format!("Recorded: double tap {key}")
            }
            None => "That was keys together; tap one key twice here.".into(),
        },
        (Some(heard), false) if heard.starts_with("double ") => {
            "That was a double tap; it belongs in the Double tap row.".into()
        }
        (Some(heard), false) => {
            *hotkey_field(form, row) = heard.clone();
            format!("Recorded: {heard}")
        }
        (None, _) => answer.trim().trim_start_matches("error: ").to_string(),
    }
}

/// When config.toml was last written, if it exists.
fn config_time() -> Option<std::time::SystemTime> {
    let path = config::path().ok()?;
    std::fs::metadata(path).ok()?.modified().ok()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    ThisPc,
    Pcs,
    Layout,
    Hotkeys,
    Look,
}

impl Settings {
    pub fn load() -> Settings {
        let (config, message) = match Config::load() {
            Ok(Some(config)) => (config, String::new()),
            Ok(None) => (empty(), "No config.toml yet; Save creates one.".into()),
            Err(err) => (
                empty(),
                format!("config.toml has an error ({err}); Save replaces it."),
            ),
        };
        let form = Form::from_config(&config);
        Settings {
            saved: form.clone(),
            form,
            fingerprint: ask_mickey(&["id"]).unwrap_or_else(|_| "(run `mickey id`)".into()),
            peers: load_peers(),
            asked: Instant::now(),
            drag: layout::Drag::default(),
            tab: Tab::ThisPc,
            mascot: None,
            recording: None,
            typing: None,
            nearby: load_nearby(),
            autostart: ask_mickey(&["autostart", "status"])
                .is_ok_and(|answer| answer.trim() == "on"),
            autostart_change: None,
            message,
            config_changed: config_time(),
            looked: Instant::now(),
        }
    }

    /// Once a second: follows config.toml when something else rewrote it
    /// (another PC placing itself, PRD 0005), unless the owner has changes
    /// not saved yet; and every 5 s asks Mickey again about the PCs.
    fn follow_changes(&mut self, ctx: &egui::Context) {
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.looked.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.looked = Instant::now();
        let now = config_time();
        if now != self.config_changed {
            self.config_changed = now;
            if self.form == self.saved {
                if let Ok(Some(config)) = Config::load() {
                    self.form = Form::from_config(&config);
                    self.saved = self.form.clone();
                    self.message = "Updated: config.toml changed (another PC moved).".into();
                }
            } else {
                self.message = "config.toml changed elsewhere; Save would replace it.".into();
            }
        }
        if self.asked.elapsed() > Duration::from_secs(5) {
            self.peers = load_peers();
            self.nearby = load_nearby();
            self.asked = Instant::now();
        }
    }

    fn save(&mut self) {
        let text = self.form.to_toml();
        // The same check `mickey` makes when it reads the file.
        if let Err(err) = config::parse(&text) {
            self.message = format!("Not saved: {err}");
            return;
        }
        let written = config::path().and_then(|path| config::write(&path, &text));
        self.message = match written {
            Ok(()) => {
                self.saved = self.form.clone();
                // Our own write is not news.
                self.config_changed = config_time();
                // The layout is used at once and the other PCs follow it;
                // other settings still wait for a restart.
                match ask_mickey(&["arranged"]) {
                    Ok(_) => {
                        "Saved. Layout and hotkeys apply on every PC now; the rest after a restart."
                            .into()
                    }
                    Err(_) => "Saved. Restart Mickey for it to take effect.".into(),
                }
            }
            Err(err) => format!("Not saved: {err}"),
        };
    }
}

fn empty() -> Config {
    config::parse("").expect("an empty config is valid")
}

/// Runs `mickey <argument>` from next to this program: its output, or
/// what it said went wrong.
fn ask_mickey(arguments: &[&str]) -> Result<String, String> {
    std::env::current_exe()
        .map(|exe| exe.with_file_name(format!("mickey{}", std::env::consts::EXE_SUFFIX)))
        .and_then(|mickey| {
            let mut command = Command::new(mickey);
            command.args(arguments);
            // mickey.exe is a console program; without this, Windows would
            // flash a console window over the settings.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                command.creation_flags(CREATE_NO_WINDOW);
            }
            command.output()
        })
        .map_err(|err| err.to_string())
        .and_then(|output| match output.status.success() {
            true => Ok(String::from_utf8_lossy(&output.stdout).trim().to_string()),
            false => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        })
}

/// A PC running Mickey heard on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Nearby {
    name: String,
    address: String,
    fingerprint: String,
}

/// `mickey discover --lines`: name, address, fingerprint per line. Empty
/// when Mickey cannot say.
fn load_nearby() -> Vec<Nearby> {
    parse_nearby(&ask_mickey(&["discover", "--lines"]).unwrap_or_default())
}

fn parse_nearby(text: &str) -> Vec<Nearby> {
    text.lines()
        .filter_map(|line| {
            let [name, address, fingerprint] = line.split('\t').collect::<Vec<_>>()[..] else {
                return None;
            };
            Some(Nearby {
                name: name.to_string(),
                address: address.to_string(),
                fingerprint: fingerprint.to_string(),
            })
        })
        .collect()
}

fn load_peers() -> Result<Peers, String> {
    let text = ask_mickey(&["peers"])?;
    Peers::from_text(&text).ok_or_else(|| format!("mickey peers answered `{text}`"))
}

impl eframe::App for Settings {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        use crate::theme::{self, icon};
        if self.mascot.is_none() {
            self.mascot = theme::mascot(ui.ctx());
        }
        self.follow_changes(ui.ctx());
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(mascot) = &self.mascot {
                    ui.add(egui::Image::new(mascot).fit_to_exact_size(egui::vec2(52.0, 52.0)));
                }
                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    ui.label(egui::RichText::new("Mickey").size(24.0).strong());
                    ui.weak("One keyboard and mouse for all your PCs");
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                theme::tab(ui, &mut self.tab, Tab::ThisPc, icon::LAPTOP, "This PC");
                theme::tab(ui, &mut self.tab, Tab::Pcs, icon::DESKTOP, "PCs");
                theme::tab(ui, &mut self.tab, Tab::Layout, icon::MONITOR, "Layout");
                theme::tab(ui, &mut self.tab, Tab::Hotkeys, icon::KEYBOARD, "Hotkeys");
                theme::tab(ui, &mut self.tab, Tab::Look, icon::PALETTE, "General");
            });
            ui.add_space(4.0);
            let form = &mut self.form;
            // Room below the card for the Save row.
            let height = (ui.available_height() - 52.0).max(120.0);
            theme::card(ui, |ui| {
            ui.set_min_size(egui::vec2(ui.available_width(), height - 24.0));
            ui.set_max_height(height - 24.0);
            let canvas = egui::vec2(ui.available_width(), height - 70.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            match self.tab {
                Tab::ThisPc => {
                    egui::Grid::new("this-pc")
                        .num_columns(2)
                        .spacing([16.0, 12.0])
                        .show(ui, |ui| {
                            ui.label(format!("{}  Name", icon::LAPTOP));
                            ui.add(
                                egui::TextEdit::singleline(&mut form.name)
                                    .hint_text("the computer's name")
                                    .desired_width(260.0),
                            );
                            ui.end_row();
                            ui.label(format!("{}  Fingerprint", icon::SHIELD));
                            ui.label(egui::RichText::new(&self.fingerprint).monospace().weak());
                            ui.end_row();
                        });
                    ui.add_space(10.0);
                    ui.checkbox(
                        &mut form.controls_others,
                        "Control other PCs with this keyboard and mouse",
                    );
                    ui.checkbox(&mut form.controlled, "Let paired PCs control this one");
                    let mut wanted = self.autostart;
                    let busy = self.autostart_change.is_some();
                    let changed = ui
                        .add_enabled(!busy, egui::Checkbox::new(&mut wanted, "Start Mickey when this PC starts"))
                        .changed();
                    if changed {
                        // Applied at once, not on Save; on a thread, since
                        // Windows may show its administrator prompt.
                        let (send, answers) = std::sync::mpsc::channel();
                        std::thread::spawn(move || {
                            let word = if wanted { "on" } else { "off" };
                            let _ = send.send(ask_mickey(&["autostart", word]));
                        });
                        self.autostart_change = Some(answers);
                    }
                    if let Some(answer) = self.autostart_change.as_ref().and_then(|answers| answers.try_recv().ok()) {
                        self.autostart_change = None;
                        self.autostart = ask_mickey(&["autostart", "status"]).is_ok_and(|answer| answer.trim() == "on");
                        if let Err(err) = answer {
                            self.message = format!("Could not change starting with this PC: {err}");
                        }
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().slider_width = 220.0;
                        ui.label(format!("{}  Touchpad speed", icon::LAPTOP));
                        ui.add(egui::Slider::new(&mut form.speed, 1..=200));
                    });
                    ui.add_space(10.0);
                    egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(format!("{}  Listen on", icon::LINK));
                            ui.add(egui::TextEdit::singleline(&mut form.listen).desired_width(200.0));
                        });
                        ui.weak("Where the other PCs connect. 0.0.0.0:4242 (every network of this PC) suits almost everyone.");
                    });
                }
                Tab::Pcs => {
                    // What the window last heard: who is online, their monitors.
                    let online = |name: &str| {
                        self.peers.as_ref().ok().and_then(|peers| peers.peers.iter().find(|peer| peer.name == name))
                    };
                    ui.label(egui::RichText::new(format!("{}  Your PCs", icon::DESKTOP)).size(16.0).strong());
                    if form.pcs.is_empty() {
                        ui.weak("None yet: connect one from the list below.");
                    }
                    let mut remove = None;
                    egui::Grid::new("pcs").num_columns(5).spacing([10.0, 8.0]).show(ui, |ui| {
                        for (i, pc) in form.pcs.iter_mut().enumerate() {
                            let info = online(&pc.name);
                            let up = info.is_some_and(|info| info.online);
                            ui.horizontal(|ui| {
                                let (spot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                                let colour = if up { theme::palette::HERE } else { theme::palette::FUR };
                                ui.painter().circle_filled(spot.center(), 4.0, colour);
                                ui.add_sized([140.0, 24.0], egui::TextEdit::singleline(&mut pc.name));
                            });
                            ui.weak(format!("{}  {}", icon::LINK, pc.address));
                            ui.weak(match info.map(|info| info.monitors.len()) {
                                Some(0) | None => "monitors unknown".to_string(),
                                Some(1) => format!("{}  1 monitor", icon::MONITOR),
                                Some(n) => format!("{}  {n} monitors", icon::MONITOR),
                            });
                            ui.add_sized([170.0, 24.0], egui::TextEdit::singleline(&mut pc.hotkey).hint_text("own hotkey"));
                            if ui.button(format!("{}  Remove", icon::TRASH)).clicked() {
                                remove = Some(i);
                            }
                            ui.end_row();
                        }
                    });
                    if let Some(i) = remove {
                        form.pcs.remove(i);
                    }

                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(format!("{}  On this network", icon::SEARCH)).size(16.0).strong());
                    ui.horizontal(|ui| {
                        if ui.button(format!("{}  Let a new PC pair (2 min)", icon::LINK)).clicked() {
                            self.message = match ask_mickey(&["pairing"]) {
                                Ok(_) => "For 2 minutes, a new PC may ask to pair with this one. Check the code on both.".into(),
                                Err(err) => format!("Could not open pairing: {err}"),
                            };
                        }
                        ui.weak("Needed on this PC when the new PC is the one that connects.");
                    });
                    let known: Vec<&str> = form.pcs.iter().map(|pc| pc.address.as_str()).collect();
                    let nearby: Vec<&Nearby> = self
                        .nearby
                        .iter()
                        .filter(|pc| !known.contains(&pc.address.as_str()))
                        .collect();
                    if nearby.is_empty() {
                        ui.weak("No other PC running Mickey heard yet. Start Mickey on it; it shows up here within seconds.");
                    }
                    let mut connect = None;
                    egui::Grid::new("nearby").num_columns(4).spacing([10.0, 8.0]).show(ui, |ui| {
                        for pc in nearby {
                            ui.label(egui::RichText::new(format!("{}  {}", icon::LAPTOP, pc.name)).strong());
                            ui.weak(&pc.address);
                            ui.weak(pc.fingerprint.get(..9).unwrap_or(&pc.fingerprint));
                            if ui.add(theme::primary(icon::LINK, "Connect")).clicked() {
                                connect = Some((pc.address.clone(), pc.name.clone()));
                            }
                            ui.end_row();
                        }
                    });
                    if let Some((address, name)) = connect {
                        self.message = match ask_mickey(&["connect", &address, &name]) {
                            Ok(answer) => answer.trim().trim_start_matches("mickey: ").to_string(),
                            Err(err) => format!("Could not connect: {err}"),
                        };
                    }

                    ui.add_space(12.0);
                    egui::CollapsingHeader::new("Add by address…").show(ui, |ui| {
                        ui.weak("Only if the PC does not show up above (another network, say).");
                        if ui.button(format!("{}  Add a PC", icon::PLUS)).clicked() {
                            form.pcs.push(PcRow {
                                name: "New PC".into(),
                                address: format!("192.168.0.2:{}", config::DEFAULT_PORT),
                                hotkey: String::new(),
                                position: None,
                                key: None,
                            });
                        }
                        ui.weak("Then edit its address in Your PCs, and Save.");
                    });
                }
                Tab::Layout => {
                    // Mickey may still be starting: ask again every 2 s.
                    if self.peers.is_err() && self.asked.elapsed() > Duration::from_secs(2) {
                        self.peers = load_peers();
                        self.asked = Instant::now();
                    }
                    if self.peers.is_err() {
                        ui.ctx().request_repaint_after(Duration::from_secs(2));
                    }
                    match &self.peers {
                        Ok(_) => ui.weak("Drag a PC to where it sits; it snaps against the nearest monitor. Then Save."),
                        Err(err) => ui.colored_label(
                            ui.visuals().warn_fg_color,
                            format!("Waiting for Mickey to list the monitors ({err})"),
                        ),
                    };
                    let empty = Peers::default();
                    let peers = self.peers.as_ref().unwrap_or(&empty);
                    layout::show(ui, canvas, peers, &mut form.pcs, &mut self.drag);
                }
                Tab::Hotkeys => {
                    ui.weak("Record, then press the keys together, or tap one key twice for the double tap. You can also type the names.");
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("{}  Always home:", crate::theme::icon::SHIELD)).strong());
                        ui.weak("hold");
                        crate::theme::keycaps(ui, "RightShift", false, false);
                        ui.weak("alone 3 s, or with");
                        crate::theme::keycaps(ui, "AltGr", false, false);
                        ui.weak("2 s, and let go: works whatever is set here");
                    });
                    let held = game_keys(form);
                    if !held.is_empty() {
                        ui.colored_label(
                            crate::theme::palette::AWAY,
                            format!(
                                "{}  {} in a hotkey: while another PC has the keyboard, Mickey holds {} back until the next key, so games that use {} (crouch, walk, jump) may feel late there.",
                                crate::theme::icon::SHIELD,
                                held.join(", "),
                                if held.len() == 1 { "it" } else { "them" },
                                if held.len() == 1 { "it" } else { "them" },
                            ),
                        );
                    }
                    ui.add_space(4.0);
                    // What the recording in progress gave, if it is done.
                    if let Some((row, answer)) = &self.recording
                        && let Ok(result) = answer.try_recv()
                    {
                        let row = *row;
                        self.recording = None;
                        self.message = recorded(form, row, result);
                    }
                    let busy = self.recording.as_ref().map(|(row, _)| *row);
                    egui::Grid::new("hotkeys")
                        .num_columns(5)
                        .min_col_width(60.0)
                        .spacing([10.0, 12.0])
                        .show(ui, |ui| {
                            for (row, (label, default)) in HOTKEY_ROWS.iter().enumerate() {
                                ui.label(egui::RichText::new(*label).strong());
                                let double = row == DOUBLE_TAP_ROW;
                                let value = hotkey_field(form, row);
                                if self.typing == Some(row) {
                                    let hint = if default.is_empty() { "none" } else { default };
                                    let field = ui.add(
                                        egui::TextEdit::singleline(value).hint_text(hint).desired_width(260.0),
                                    );
                                    field.request_focus();
                                    if ui.button(format!("{}  Done", icon::CHECK)).clicked()
                                        || ui.input(|input| input.key_pressed(egui::Key::Enter))
                                    {
                                        self.typing = None;
                                    }
                                } else {
                                    // The keys as keys; an empty setting shows its default, faded.
                                    let shown = if value.trim().is_empty() && !double { default } else { value.trim() };
                                    theme::keycaps(ui, shown, double, value.trim().is_empty());
                                    if ui.button("Type").clicked() {
                                        self.typing = Some(row);
                                    }
                                }
                                if busy == Some(row) {
                                    ui.colored_label(theme::palette::BRUISE, "Press the keys…");
                                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                                } else if ui
                                    .add_enabled(busy.is_none(), egui::Button::new(format!("{}  Record", icon::KEYBOARD)))
                                    .clicked()
                                {
                                    let (send, answer) = std::sync::mpsc::channel();
                                    std::thread::spawn(move || {
                                        let _ = send.send(ask_mickey(&["record"]));
                                    });
                                    self.recording = Some((row, answer));
                                }
                                if ui.button(format!("{}  Default", icon::UNDO)).clicked() {
                                    // Empty means the default, except the double tap,
                                    // where empty turns it off.
                                    *hotkey_field(form, row) = if row == DOUBLE_TAP_ROW {
                                        config::DEFAULT_DOUBLE_TAP.to_string()
                                    } else {
                                        String::new()
                                    };
                                }
                                ui.end_row();
                            }
                        });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().slider_width = 220.0;
                        ui.label("Double tap: both taps within");
                        ui.add(
                            egui::Slider::new(&mut form.double_tap_ms, config::DOUBLE_TAP_MS)
                                .suffix(" ms")
                                .step_by(50.0),
                        );
                    });
                    ui.weak("Only keys nobody types alone can be a double tap: Shift, Ctrl, Alt, AltGr, Super, F1–F12.");
                    ui.add_space(6.0);
                    ui.weak("Each PC also gets the next number: the hotkey + 1, + 2… goes straight to it.");
                }
                Tab::Look => {
                    egui::ComboBox::from_label("Switcher position")
                        .selected_text(form.position.clone())
                        .show_ui(ui, |ui| {
                            for position in SWITCHER_POSITIONS {
                                ui.selectable_value(
                                    &mut form.position,
                                    position.to_string(),
                                    position,
                                );
                            }
                        });
                    ui.weak("Where the switcher window appears when the hotkey opens it.");
                    ui.add_space(8.0);
                    ui.checkbox(&mut form.vertical, "Show the PCs in a column");
                    ui.add_space(8.0);
                    switcher_order(ui, form);
                    ui.add_space(12.0);
                    ui.checkbox(&mut form.history, "Keep a history of copies on this PC");
                    ui.weak("Off: the clipboard is still shared, and another manager (cliphist, CopyQ, Klipper) keeps the history. Applies after a restart.");
                }
            }
            });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let changed = self.form != self.saved;
                if ui.add_enabled(changed, theme::primary(icon::SAVE, "Save")).clicked() {
                    self.save();
                }
                if ui
                    .add_enabled(changed, egui::Button::new(format!("{}  Undo", icon::UNDO)))
                    .clicked()
                {
                    self.form = self.saved.clone();
                }
                ui.label(&self.message);
            });
        });
    }
}

/// The switcher's order: every PC with buttons to move it. Moving one saves
/// the whole list as the custom order; "Like the layout" empties it.
fn switcher_order(ui: &mut egui::Ui, form: &mut Form) {
    let this_pc = if form.name.trim().is_empty() {
        config::ThisPc::default().name()
    } else {
        form.name.trim().to_string()
    };
    let mut cards = vec![(this_pc.as_str(), Some([0, 0]))];
    cards.extend(form.pcs.iter().map(|pc| (pc.name.as_str(), pc.position)));
    let interface = config::Interface {
        switcher_vertical: form.vertical,
        switcher_order: form.order.clone(),
        ..config::Interface::default()
    };
    let shown: Vec<String> = interface
        .switcher_sort(&cards)
        .into_iter()
        .map(|index| cards[index].0.to_string())
        .collect();
    ui.horizontal(|ui| {
        ui.label("Order in the switcher");
        if form.order.is_empty() {
            ui.weak("(as the layout places them)");
        } else if ui.button("Like the layout").clicked() {
            form.order.clear();
        }
    });
    let mut moved = None;
    for (i, name) in shown.iter().enumerate() {
        ui.horizontal(|ui| {
            let up = ui.add_enabled(
                i > 0,
                egui::Button::new(crate::theme::icon::ARROW_UP).small(),
            );
            let down = ui.add_enabled(
                i + 1 < shown.len(),
                egui::Button::new(crate::theme::icon::ARROW_DOWN).small(),
            );
            if up.clicked() {
                moved = Some((i, i - 1));
            }
            if down.clicked() {
                moved = Some((i, i + 1));
            }
            ui.label(format!("{i}  {name}"));
        });
    }
    ui.add_space(8.0);
    ui.weak("Preview: how the switcher will look.");
    preview(ui, &shown, form.vertical);
    if let Some((from, to)) = moved {
        let mut order = shown;
        order.swap(from, to);
        form.order = order;
    }
}

/// The switcher as it will show, drawn by the same code as the real one
/// (`crate::cards`), with the first PC in use and the next one selected.
fn preview(ui: &mut egui::Ui, names: &[String], vertical: bool) {
    let choice = crate::choice::Choice {
        corner: crate::place::Corner::BottomRight,
        vertical,
        pcs: names
            .iter()
            .enumerate()
            .map(|(id, name)| crate::choice::Pc {
                id,
                name: name.clone(),
                online: true,
            })
            .collect(),
        focused: 0,
        selected: usize::from(names.len() > 1),
    };
    // Measuring lays the cards out off screen, so it is done again only
    // when the names or the direction change.
    static WIDTH: std::sync::Mutex<Option<(Vec<String>, bool, f32)>> = std::sync::Mutex::new(None);
    let mut cached = WIDTH.lock().unwrap_or_else(|p| p.into_inner());
    let width = match cached.as_ref() {
        Some((known, was_vertical, width)) if known == names && *was_vertical == vertical => *width,
        _ => {
            let width = crate::measure(&choice).1;
            *cached = Some((names.to_vec(), vertical, width));
            width
        }
    };
    egui::Frame::NONE
        .fill(crate::theme::palette::INK)
        .corner_radius(10.0)
        .inner_margin(8.0)
        .show(ui, |ui| {
            crate::cards(ui, &choice, width);
        });
}

/// Keys games use all the time that appear in a hotkey (#188). A key that
/// may start a hotkey is held back on the way to another PC until the
/// next key shows whether the hotkey is meant, which games feel.
fn game_keys(form: &Form) -> Vec<&'static str> {
    const GAME_KEYS: [&str; 5] = ["LeftCtrl", "LeftShift", "LeftAlt", "Space", "Tab"];
    let mut texts = vec![
        &form.next,
        &form.home,
        &form.switcher,
        &form.forward,
        &form.back,
        &form.double_tap,
    ];
    texts.extend(form.pcs.iter().map(|pc| &pc.hotkey));
    GAME_KEYS
        .into_iter()
        .filter(|key| {
            texts.iter().any(|text| {
                text.split('+')
                    .any(|part| part.trim().eq_ignore_ascii_case(key))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn keys_games_use_are_found_in_hotkeys() {
        let mut form = Form::from_config(&config::parse("").unwrap());
        assert!(
            game_keys(&form).is_empty(),
            "the defaults hold back nothing games use"
        );
        form.double_tap = "LeftCtrl".into();
        form.home = "LeftShift+AltGr+H".into();
        assert_eq!(game_keys(&form), ["LeftCtrl", "LeftShift"]);
    }

    use super::*;

    #[test]
    fn what_the_form_writes_parses_back_to_the_same_form() {
        let original = config::parse(
            "[[peer]]\nname = \"Windows \\\"main\\\"\"\naddress = \"192.0.2.10:4242\"\nhotkey = \"AltGr+RightShift+1\"\nposition = [-1920, 0]\nkey = \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n\n[hotkeys]\nhome = \"AltGr+RightShift+H\"\n\n[interface]\nswitcher_position = \"top-left\"\n\n[clipboard]\nhistory = false\n\n[this_pc]\ncontrolled = false\nname = \"Desk\"\n",
        )
        .unwrap();
        let form = Form::from_config(&original);
        let reread = config::parse(&form.to_toml()).unwrap();
        assert_eq!(Form::from_config(&reread), form);
        assert!(
            !reread.clipboard.history,
            "the history setting survives a save"
        );
    }

    #[test]
    fn a_recording_lands_in_the_right_row_or_says_why_not() {
        let mut form = Form::from_config(&empty());
        recorded(&mut form, 3, Ok("ok: AltGr+Right\n".into()));
        assert_eq!(form.forward, "AltGr+Right");
        recorded(&mut form, DOUBLE_TAP_ROW, Ok("ok: double LeftShift".into()));
        assert_eq!(form.double_tap, "LeftShift");
        let before = form.next.clone();
        let said = recorded(&mut form, 0, Ok("ok: double LeftShift".into()));
        assert!(said.contains("Double tap row"));
        assert_eq!(
            form.next, before,
            "a double tap does not go into a chord row"
        );
        assert_eq!(
            recorded(&mut form, 0, Ok("error: no keys were pressed".into())),
            "no keys were pressed"
        );
    }

    #[test]
    fn nearby_lines_become_pcs() {
        let nearby = parse_nearby("Desk\t192.0.2.64:4242\tVK2A-L4MJ\nnonsense\n");
        assert_eq!(nearby.len(), 1);
        assert_eq!(nearby[0].address, "192.0.2.64:4242");
    }

    #[test]
    fn a_bad_field_is_caught_by_the_same_rules_as_mickey() {
        let mut form = Form::from_config(&empty());
        form.pcs.push(PcRow {
            name: "A".into(),
            address: "nowhere".into(),
            hotkey: String::new(),
            position: None,
            key: None,
        });
        assert!(config::parse(&form.to_toml()).is_err());
    }
}
