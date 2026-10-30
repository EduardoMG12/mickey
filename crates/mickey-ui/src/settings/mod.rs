//! The settings window (PRD 0006, step 3 of the plan): edits config.toml.
//! What it saves is checked with `mickey_config::parse`, the same rules
//! `mickey` applies, before anything is written.

mod form;
mod general;
mod hotkeys;
mod pcs;
mod sound;
mod this_pc;

use form::Form;
pub use form::PcRow;
use pcs::{Nearby, load_nearby, load_peers};

use crate::layout;
use eframe::egui;
use mickey_config::peers::Peers;
use mickey_config::{self as config, Config};
use std::time::{Duration, Instant};

pub struct Settings {
    form: Form,
    saved: Form,
    fingerprint: String,
    /// This PC's public key, to mark it in "Who listens".
    own_key: Option<String>,
    /// A newer Mickey the running one knows of (PRD 0013).
    update: Option<String>,
    /// `mickey update apply`, running: its answer, once it ends.
    updating: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// The PC whose Remove was clicked once: a second click removes it.
    confirm_remove: Option<String>,
    /// What the running Mickey said about the PCs; `Err` says why not.
    peers: Result<Peers, String>,
    asked: Instant,
    /// The PC being paired from the Connect button, and since when: once it
    /// is online, the Layout tab opens to place it (#119).
    pairing: Option<(String, Instant)>,
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
            Ok(None) => (
                Config::empty(),
                "No config.toml yet; Save creates one.".into(),
            ),
            Err(err) => (
                Config::empty(),
                format!("config.toml has an error ({err}); Save replaces it."),
            ),
        };
        let form = Form::from_config(&config);
        Settings {
            saved: form.clone(),
            form,
            fingerprint: crate::mickey::ask(&["id"]).unwrap_or_else(|_| "(run `mickey id`)".into()),
            own_key: crate::mickey::ask(&["id", "--key"])
                .ok()
                .map(|key| key.trim().to_string()),
            confirm_remove: None,
            peers: load_peers(),
            asked: Instant::now(),
            pairing: None,
            drag: layout::Drag::default(),
            tab: Tab::ThisPc,
            mascot: None,
            recording: None,
            typing: None,
            nearby: load_nearby(),
            autostart: crate::mickey::ask(&["autostart", "status"])
                .is_ok_and(|answer| answer.trim() == "on"),
            autostart_change: None,
            update: crate::mickey::ask(&["update", "status"])
                .ok()
                .and_then(|answer| mickey_config::version::checked(answer.trim())),
            updating: None,
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
        // Every second while a pairing is on, so the Layout tab opens soon
        // after the owner accepts; every 5 s otherwise.
        let every = if self.pairing.is_some() { 1 } else { 5 };
        if self.asked.elapsed() > Duration::from_secs(every) {
            self.peers = load_peers();
            self.nearby = load_nearby();
            self.asked = Instant::now();
            self.follow_pairing();
        }
    }

    /// Opens the Layout tab once the PC being paired is online; gives up
    /// after the two minutes a pairing stays open.
    fn follow_pairing(&mut self) {
        let Some((name, since)) = &self.pairing else {
            return;
        };
        let online = self.peers.as_ref().is_ok_and(|peers| {
            peers
                .peers
                .iter()
                .any(|peer| peer.name == *name && peer.online)
        });
        if online {
            self.message = format!("Paired with {name}. Place it next to this PC's screens.");
            self.tab = Tab::Layout;
            self.pairing = None;
        } else if since.elapsed() > Duration::from_secs(120) {
            self.pairing = None;
        }
    }

    fn save(&mut self) {
        // What every PC shares changed here: newer than anyone's, so the
        // other PCs take it (and an older one never undoes it).
        let shared = |form: &Form| {
            (
                form.listener.clone(),
                form.volumes.clone(),
                form.muted.clone(),
            )
        };
        if shared(&self.form) != shared(&self.saved) {
            self.form.since = config::now_seconds();
        }
        // PCs removed since the last save: no longer trusted either, once
        // this is written (Undo before then forgets nothing).
        let kept: Vec<Option<String>> = self.form.pcs.iter().map(|pc| pc.key.clone()).collect();
        let removed: Vec<String> = self
            .saved
            .pcs
            .iter()
            .filter_map(|pc| pc.key.clone())
            .filter(|key| !kept.contains(&Some(key.clone())))
            .collect();
        let text = self.form.to_toml();
        // The same check `mickey` makes when it reads the file.
        if let Err(err) = config::parse(&text) {
            self.message = format!("Not saved: {err}");
            return;
        }
        let written = config::path().and_then(|path| config::write(&path, &text));
        self.message = match written {
            Ok(()) => {
                for key in &removed {
                    if let Err(err) = crate::mickey::ask(&["forget", key]) {
                        eprintln!("mickey-ui: could not stop trusting a removed PC: {err}");
                    }
                }
                self.saved = self.form.clone();
                // Our own write is not news.
                self.config_changed = config_time();
                // The layout is used at once and the other PCs follow it;
                // other settings still wait for a restart.
                match crate::mickey::ask(&["arranged"]) {
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
            // Hard to miss (PRD 0013): most releases fix security or speed.
            if let Some(version) = &self.update {
                ui.horizontal(|ui| {
                    ui.colored_label(
                        theme::palette::AWAY,
                        format!(
                            "Mickey {version} is out; this PC runs {}.",
                            mickey_config::version::THIS
                        ),
                    );
                    if self.updating.is_some() {
                        ui.spinner();
                        ui.weak("Downloading and checking it…");
                    } else if ui.button("Update now").clicked() {
                        // Minutes, maybe, and Windows' administrator
                        // prompt: on a thread.
                        let (send, answer) = std::sync::mpsc::channel();
                        std::thread::spawn(move || {
                            let _ = send.send(crate::mickey::ask(&["update", "apply"]));
                        });
                        self.updating = Some(answer);
                    }
                    ui.hyperlink_to(
                        "What's new",
                        "https://github.com/EduardoMG12/mickey/releases/latest",
                    );
                });
                if let Some(answer) = self
                    .updating
                    .as_ref()
                    .and_then(|answer| answer.try_recv().ok())
                {
                    self.message = answer.unwrap_or_else(|err| format!("The update failed: {err}"));
                    self.updating = None;
                }
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(250));
                ui.add_space(4.0);
            }
            // Room below the card for the Save row.
            let height = (ui.available_height() - 52.0).max(120.0);
            theme::card(ui, |ui| {
                ui.set_min_size(egui::vec2(ui.available_width(), height - 24.0));
                ui.set_max_height(height - 24.0);
                let canvas = egui::vec2(ui.available_width(), height - 70.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.tab {
                        Tab::ThisPc => self.this_pc_tab(ui),
                        Tab::Pcs => self.pcs_tab(ui),
                        Tab::Layout => self.layout_tab(ui, canvas),
                        Tab::Hotkeys => self.hotkeys_tab(ui),
                        Tab::Look => self.general_tab(ui),
                    });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let changed = self.form != self.saved;
                if ui
                    .add_enabled(changed, theme::primary(icon::SAVE, "Save"))
                    .clicked()
                {
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

/// Each tab of the settings window, drawn inside its card.
impl Settings {
    /// Layout: where each PC's screens sit.
    fn layout_tab(&mut self, ui: &mut egui::Ui, canvas: egui::Vec2) {
        let form = &mut self.form;
        // Mickey may still be starting: ask again every 2 s.
        if self.peers.is_err() && self.asked.elapsed() > Duration::from_secs(2) {
            self.peers = load_peers();
            self.asked = Instant::now();
        }
        if self.peers.is_err() {
            ui.ctx().request_repaint_after(Duration::from_secs(2));
        }
        match &self.peers {
            Ok(_) => ui.weak(
                "Drag a PC to where it sits; it snaps against the nearest monitor. Then Save.",
            ),
            Err(err) => ui.colored_label(
                ui.visuals().warn_fg_color,
                format!("Waiting for Mickey to list the monitors ({err})"),
            ),
        };
        let empty = Peers::default();
        let peers = self.peers.as_ref().unwrap_or(&empty);
        layout::show(ui, canvas, peers, &mut form.pcs, &mut self.drag);
    }
}
