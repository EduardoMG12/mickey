//! The settings form: config.toml's values as the window edits them, and
//! the config.toml text they make when saved.

use mickey_config::{self as config, Audio, Config, SoundMode};

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
    /// Keep a clipboard history here: always, never (another manager keeps
    /// it), or `None`, only when no other manager runs.
    pub history: Option<bool>,
    /// Ask GitHub once a day for a newer Mickey (PRD 0013).
    pub check_updates: bool,
    /// Who listens (PRD 0012): a PC's public key; `None`, nobody.
    pub listener: Option<String>,
    /// Where this PC's own sound goes.
    pub mode: SoundMode,
    /// Each PC's volume on the listener (0–100) and the muted PCs, by key.
    pub volumes: std::collections::BTreeMap<String, u8>,
    pub muted: Vec<String>,
    /// When what every PC shares last changed (`Audio::since`).
    pub since: u64,
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
            check_updates: config.updates.check,
            listener: config.audio.listener.clone(),
            mode: config.audio.mode,
            volumes: config.audio.volumes.clone(),
            muted: config.audio.muted.clone(),
            since: config.audio.since,
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
                config::toml_string(self.name.trim())
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
            config::toml_string(&self.listen)
        );
        for pc in &self.pcs {
            out += "\n[[peer]]\n";
            out += &format!("name = {}\n", config::toml_string(&pc.name));
            out += &format!("address = {}\n", config::toml_string(&pc.address));
            if !pc.hotkey.trim().is_empty() {
                out += &format!(
                    "hotkey = {}    # straight to this PC\n",
                    config::toml_string(pc.hotkey.trim())
                );
            }
            if let Some([x, y]) = pc.position {
                out += &format!("position = [{x}, {y}]    # where it sits next to this PC\n");
            }
            if let Some(key) = &pc.key {
                out += &format!(
                    "key = {}    # this PC's identity, set by Mickey\n",
                    config::toml_string(key)
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
            out += &format!("{key} = {}\n", config::toml_string(value));
        }
        // Written even when empty: "" is how the double tap is turned off.
        out += &format!(
            "double_tap = {}    # a key tapped twice quickly: next PC\n",
            config::toml_string(self.double_tap.trim())
        );
        out += &format!(
            "double_tap_ms = {}    # how close the two taps must be\n",
            self.double_tap_ms
        );
        out += "\n[interface]\n";
        out += &format!(
            "switcher_position = {}\n",
            config::toml_string(&self.position)
        );
        if self.vertical {
            out += "switcher_vertical = true\n";
        }
        if !self.order.is_empty() {
            let names: Vec<String> = self
                .order
                .iter()
                .map(|name| config::toml_string(name))
                .collect();
            out += &format!("switcher_order = [{}]\n", names.join(", "));
        }
        let audio = Audio {
            listener: self.listener.clone(),
            mode: self.mode,
            volumes: self.volumes.clone(),
            muted: self.muted.clone(),
            since: self.since,
        };
        if audio != Audio::default() {
            out += "\n[audio]\n";
            out += &audio.to_lines();
        }
        match self.history {
            None => {}
            Some(true) => {
                out += "\n[clipboard]\nhistory = true    # even with another clipboard manager\n"
            }
            Some(false) => {
                out += "\n[clipboard]\nhistory = false    # another clipboard manager keeps the history\n"
            }
        }
        if !self.check_updates {
            out += "\n[updates]\ncheck = false    # never ask GitHub for a newer Mickey\n";
        }
        out += "\n[touchpad]\n";
        out += &format!("speed = {}    # higher is faster\n", self.speed);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_form_writes_parses_back_to_the_same_form() {
        let original = config::parse(
            "[[peer]]\nname = \"Windows \\\"main\\\"\"\naddress = \"192.0.2.10:4242\"\nhotkey = \"AltGr+RightShift+1\"\nposition = [-1920, 0]\nkey = \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n\n[hotkeys]\nhome = \"AltGr+RightShift+H\"\n\n[interface]\nswitcher_position = \"top-left\"\n\n[clipboard]\nhistory = false\n\n[updates]\ncheck = false\n\n[audio]\nlistener = \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\nmode = \"both\"\n\n[this_pc]\ncontrolled = false\nname = \"Desk\"\n",
        )
        .unwrap();
        let form = Form::from_config(&original);
        let reread = config::parse(&form.to_toml()).unwrap();
        assert_eq!(Form::from_config(&reread), form);
        assert!(
            reread.clipboard.history == Some(false),
            "the history setting survives a save"
        );
        assert!(
            !reread.updates.check,
            "turning the update check off survives a save"
        );
        assert_eq!(
            reread.audio, original.audio,
            "who listens and the mode survive a save"
        );
        assert_eq!(reread.audio.mode, config::SoundMode::Both);
    }

    #[test]
    fn a_bad_field_is_caught_by_the_same_rules_as_mickey() {
        let mut form = Form::from_config(&Config::empty());
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
