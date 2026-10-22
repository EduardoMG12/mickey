//! The settings window (PRD 0006, step 3 of the plan): edits config.toml.
//! What it saves is checked with `mickey_config::parse`, the same rules
//! `mickey` applies, before anything is written.

use eframe::egui;
use mickey_config::{self as config, Config, SWITCHER_POSITIONS};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PcRow {
    pub name: String,
    pub address: String,
    pub hotkey: String,
}

/// Everything editable, as the text the fields show.
#[derive(Debug, Clone, PartialEq)]
pub struct Form {
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
    tab: Tab,
    message: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    ThisPc,
    Pcs,
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
            fingerprint: fingerprint(),
            tab: Tab::ThisPc,
            message,
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
                "Saved. Restart Mickey for it to take effect.".into()
            }
            Err(err) => format!("Not saved: {err}"),
        };
    }
}

fn empty() -> Config {
    config::parse("").expect("an empty config is valid")
}

/// This PC's fingerprint, from `mickey id` next to this program.
fn fingerprint() -> String {
    std::env::current_exe()
        .map(|exe| exe.with_file_name(format!("mickey{}", std::env::consts::EXE_SUFFIX)))
        .and_then(|mickey| {
            let mut command = Command::new(mickey);
            command.arg("id");
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
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "(run `mickey id`)".into())
}

impl eframe::App for Settings {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::ThisPc, "This PC");
                ui.selectable_value(&mut self.tab, Tab::Pcs, "PCs");
                ui.selectable_value(&mut self.tab, Tab::Hotkeys, "Hotkeys");
                ui.selectable_value(&mut self.tab, Tab::Look, "Appearance");
            });
            ui.separator();
            let form = &mut self.form;
            match self.tab {
                Tab::ThisPc => {
                    ui.label(format!("Fingerprint: {}", self.fingerprint));
                    ui.checkbox(
                        &mut form.controls_others,
                        "Control other PCs with this keyboard and mouse",
                    );
                    ui.checkbox(&mut form.controlled, "Let paired PCs control this one");
                    ui.horizontal(|ui| {
                        ui.label("Listen on");
                        ui.text_edit_singleline(&mut form.listen);
                    });
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
                                if ui.button("Remove").clicked() {
                                    remove = Some(i);
                                }
                                ui.end_row();
                            }
                        });
                    if let Some(i) = remove {
                        form.pcs.remove(i);
                    }
                    if ui.button("Add a PC").clicked() {
                        form.pcs.push(PcRow {
                            name: "New PC".into(),
                            address: format!("192.168.0.2:{}", config::DEFAULT_PORT),
                            hotkey: String::new(),
                        });
                    }
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
            ui.separator();
            ui.horizontal(|ui| {
                let changed = self.form != self.saved;
                if ui.add_enabled(changed, egui::Button::new("Save")).clicked() {
                    self.save();
                }
                if ui.add_enabled(changed, egui::Button::new("Undo")).clicked() {
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
            "[[peer]]\nname = \"Windows \\\"main\\\"\"\naddress = \"192.0.2.10:4242\"\nhotkey = \"AltGr+RightShift+1\"\n\n[hotkeys]\nhome = \"AltGr+RightShift+H\"\n\n[interface]\nswitcher_position = \"top-left\"\n\n[this_pc]\ncontrolled = false\n",
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
        });
        assert!(config::parse(&form.to_toml()).is_err());
    }
}
