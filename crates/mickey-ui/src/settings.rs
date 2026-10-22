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
    pub position: String,
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
            position: config.interface.switcher_position.clone(),
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
        out += "\n[interface]\n";
        out += &format!("switcher_position = {}\n", quote(&self.position));
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
    /// The mascot in the header, loaded on the first frame.
    mascot: Option<egui::TextureHandle>,
    /// When config.toml last changed as far as this window knows, and when
    /// it last looked: another PC may place itself and rewrite the file.
    config_changed: Option<std::time::SystemTime>,
    looked: Instant,
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
            fingerprint: ask_mickey("id").unwrap_or_else(|_| "(run `mickey id`)".into()),
            peers: load_peers(),
            asked: Instant::now(),
            drag: layout::Drag::default(),
            tab: Tab::ThisPc,
            mascot: None,
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
        let written = config::path().and_then(|path| {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
            }
            std::fs::write(&path, text).map_err(|err| format!("{}: {err}", path.display()))
        });
        self.message = match written {
            Ok(()) => {
                self.saved = self.form.clone();
                // Our own write is not news.
                self.config_changed = config_time();
                // The layout is used at once and the other PCs follow it;
                // other settings still wait for a restart.
                match ask_mickey("arranged") {
                    Ok(_) => {
                        "Saved. The other PCs follow the new layout; restart Mickey for the rest."
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
fn ask_mickey(argument: &str) -> Result<String, String> {
    std::env::current_exe()
        .map(|exe| exe.with_file_name(format!("mickey{}", std::env::consts::EXE_SUFFIX)))
        .and_then(|mickey| {
            let mut command = Command::new(mickey);
            command.arg(argument);
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

fn load_peers() -> Result<Peers, String> {
    let text = ask_mickey("peers")?;
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
                theme::tab(ui, &mut self.tab, Tab::Look, icon::PALETTE, "Appearance");
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
                            ui.label(format!("{}  Listen on", icon::LINK));
                            ui.add(egui::TextEdit::singleline(&mut form.listen).desired_width(260.0));
                            ui.end_row();
                        });
                    ui.add_space(10.0);
                    ui.checkbox(
                        &mut form.controls_others,
                        "Control other PCs with this keyboard and mouse",
                    );
                    ui.checkbox(&mut form.controlled, "Let paired PCs control this one");
                }
                Tab::Pcs => {
                    let mut remove = None;
                    egui::Grid::new("pcs")
                        .striped(true)
                        .num_columns(4)
                        .min_col_width(150.0)
                        .show(ui, |ui| {
                            ui.strong("Name");
                            ui.strong("Address (ip:port)");
                            ui.strong("Hotkey (optional)");
                            ui.end_row();
                            for (i, pc) in form.pcs.iter_mut().enumerate() {
                                ui.add(
                                    egui::TextEdit::singleline(&mut pc.name).desired_width(150.0),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut pc.address)
                                        .desired_width(170.0),
                                );
                                ui.add(
                                    egui::TextEdit::singleline(&mut pc.hotkey).desired_width(170.0),
                                );
                                if ui.button(format!("{}  Remove", icon::TRASH)).clicked() {
                                    remove = Some(i);
                                }
                                ui.end_row();
                            }
                        });
                    if let Some(i) = remove {
                        form.pcs.remove(i);
                    }
                    if ui.button(format!("{}  Add a PC", icon::PLUS)).clicked() {
                        form.pcs.push(PcRow {
                            name: "New PC".into(),
                            address: format!("192.168.0.2:{}", config::DEFAULT_PORT),
                            hotkey: String::new(),
                            position: None,
                        });
                    }
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
                    ui.weak("e.g. AltGr+RightShift or AltGr+RightShift+H");
                    ui.weak(
                        "Empty: AltGr+RightShift (next), +0 (home), +1 to +9 (each PC in order)",
                    );
                    egui::Grid::new("hotkeys")
                        .min_col_width(150.0)
                        .show(ui, |ui| {
                            for (label, value) in [
                                ("Next PC / switcher", &mut form.next),
                                ("Back to this PC", &mut form.home),
                                ("Open the switcher", &mut form.switcher),
                                ("Next PC (one hand)", &mut form.forward),
                                ("Previous PC (one hand)", &mut form.back),
                                ("Double tap (empty: off)", &mut form.double_tap),
                            ] {
                                ui.label(label);
                                ui.add_sized([240.0, 20.0], egui::TextEdit::singleline(value));
                                ui.end_row();
                            }
                        });
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
                    ui.add(egui::Slider::new(&mut form.speed, 1..=200).text("Touchpad speed"));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_form_writes_parses_back_to_the_same_form() {
        let original = config::parse(
            "[[peer]]\nname = \"Windows \\\"main\\\"\"\naddress = \"192.0.2.10:4242\"\nhotkey = \"AltGr+RightShift+1\"\nposition = [-1920, 0]\n\n[hotkeys]\nhome = \"AltGr+RightShift+H\"\n\n[interface]\nswitcher_position = \"top-left\"\n\n[this_pc]\ncontrolled = false\nname = \"Desk\"\n",
        )
        .unwrap();
        let form = Form::from_config(&original);
        let reread = config::parse(&form.to_toml()).unwrap();
        assert_eq!(Form::from_config(&reread), form);
    }

    #[test]
    fn a_bad_field_is_caught_by_the_same_rules_as_mickey() {
        let mut form = Form::from_config(&empty());
        form.pcs.push(PcRow {
            name: "A".into(),
            address: "nowhere".into(),
            hotkey: String::new(),
            position: None,
        });
        assert!(config::parse(&form.to_toml()).is_err());
    }
}
