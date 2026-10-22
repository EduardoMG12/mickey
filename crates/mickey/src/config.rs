//! `config.toml` (PRD 0003): the peers this machine drives and its
//! settings.
//!
//! ```toml
//! [[peer]]
//! name = "Desktop"
//! address = "192.0.2.10:4242"
//!
//! [touchpad]
//! speed = 16
//! ```

use crate::chord::{self, Chord};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;

const FILE_NAME: &str = "config.toml";
/// What the M0 touchpad used, in mouse counts per millimetre.
const DEFAULT_TOUCHPAD_SPEED: i32 = 16;
const TOUCHPAD_SPEEDS: std::ops::RangeInclusive<i32> = 1..=200;

pub const EXAMPLE: &str = r#"[[peer]]
name = "Desktop"
address = "192.0.2.10:4242"

[touchpad]
speed = 16

[interface]
switcher_position = "bottom-right"   # or center, top-left, top-right, bottom-left

[this_pc]
controls_others = true     # send this PC's keyboard and mouse to the peers
controlled = true          # let paired peers drive this PC
listen = "0.0.0.0:4242"    # where peers connect to this PC
"#;

// `deny_unknown_fields`: a typo like `adress` is an error, not ignored.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, rename = "peer")]
    pub peers: Vec<Peer>,
    #[serde(default)]
    pub touchpad: Touchpad,
    #[serde(default)]
    pub hotkeys: Hotkeys,
    #[serde(default)]
    pub interface: Interface,
    #[serde(default)]
    pub this_pc: ThisPc,
}

/// `[this_pc]`: which roles `mickey run` plays on this machine.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ThisPc {
    /// Send this PC's keyboard and mouse to the peers (the source).
    #[serde(default = "yes")]
    pub controls_others: bool,
    /// Let paired peers drive this PC (the target).
    #[serde(default = "yes")]
    pub controlled: bool,
    /// Where the target waits for peers. All interfaces by default: only
    /// paired machines can do anything (PRD 0002), so this needs no IP.
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,
}

impl Default for ThisPc {
    fn default() -> ThisPc {
        ThisPc {
            controls_others: true,
            controlled: true,
            listen: default_listen(),
        }
    }
}

fn yes() -> bool {
    true
}

pub const DEFAULT_PORT: u16 = 4242;

fn default_listen() -> SocketAddr {
    SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT))
}

/// `[interface]`: how Mickey shows itself.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Interface {
    /// Where the switcher appears: a corner or `center`.
    #[serde(default = "default_switcher_position")]
    pub switcher_position: String,
}

impl Default for Interface {
    fn default() -> Interface {
        Interface {
            switcher_position: default_switcher_position(),
        }
    }
}

const SWITCHER_POSITIONS: [&str; 5] = [
    "center",
    "top-left",
    "top-right",
    "bottom-left",
    "bottom-right",
];

fn default_switcher_position() -> String {
    "bottom-right".into()
}

/// `[hotkeys]`, as written; `Config::hotkeys` parses and checks them.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Hotkeys {
    pub next: Option<String>,
    pub home: Option<String>,
    pub switcher: Option<String>,
}

/// Every hotkey, parsed. `peers[i]` is peer `i`'s own hotkey, if any.
#[derive(Debug, PartialEq)]
pub struct HotkeySet {
    pub next: Chord,
    pub home: Option<Chord>,
    pub switcher: Option<Chord>,
    pub peers: Vec<Option<Chord>>,
}

/// The M0 hotkey, still the default for `next`.
pub const DEFAULT_NEXT: &str = "AltGr+RightShift";

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub name: String,
    pub address: SocketAddr,
    /// Goes straight to this peer, e.g. `"AltGr+RightShift+1"`.
    #[serde(default)]
    pub hotkey: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Touchpad {
    #[serde(default = "default_speed")]
    pub speed: i32,
}

impl Default for Touchpad {
    fn default() -> Touchpad {
        Touchpad {
            speed: DEFAULT_TOUCHPAD_SPEED,
        }
    }
}

fn default_speed() -> i32 {
    DEFAULT_TOUCHPAD_SPEED
}

impl Config {
    /// Parses every hotkey; an error names the setting and the bad part,
    /// and two settings with the same keys are refused.
    pub fn hotkeys(&self) -> Result<HotkeySet, String> {
        let one = |setting: &str, text: &Option<String>| {
            text.as_deref()
                .map(chord::parse)
                .transpose()
                .map_err(|err| format!("{setting}: {err}"))
        };
        let set = HotkeySet {
            next: one("hotkeys.next", &self.hotkeys.next)?
                .unwrap_or_else(|| chord::parse(DEFAULT_NEXT).expect("the default parses")),
            home: one("hotkeys.home", &self.hotkeys.home)?,
            switcher: one("hotkeys.switcher", &self.hotkeys.switcher)?,
            peers: self
                .peers
                .iter()
                .map(|peer| one(&format!("hotkey of {}", peer.name), &peer.hotkey))
                .collect::<Result<_, _>>()?,
        };
        let all: Vec<&Chord> = [Some(&set.next), set.home.as_ref(), set.switcher.as_ref()]
            .into_iter()
            .flatten()
            .chain(set.peers.iter().flatten())
            .collect();
        for (i, chord) in all.iter().enumerate() {
            if all[..i].contains(chord) {
                return Err(format!("two hotkeys use the same keys: {chord:?}"));
            }
        }
        Ok(set)
    }

    /// Reads `config.toml`; `None` when the file does not exist.
    pub fn load() -> Result<Option<Config>, String> {
        let path = path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => parse(&text)
                .map(Some)
                .map_err(|err| format!("{}: {err}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(format!("cannot read {}: {err}", path.display())),
        }
    }
}

/// `$XDG_CONFIG_HOME/mickey/config.toml` (or `~/.config/...`) on Linux,
/// `%APPDATA%\mickey\config.toml` on Windows.
pub fn path() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    base.map(|dir| dir.join("mickey").join(FILE_NAME))
        .ok_or_else(|| "cannot find the user's config directory".to_string())
}

pub fn parse(text: &str) -> Result<Config, String> {
    // toml's message already names the line and shows the bad text.
    let config: Config = toml::from_str(text).map_err(|err| err.to_string())?;
    config.hotkeys()?;
    // Names travel in tab-separated status lines and as window labels.
    if let Some(peer) = config
        .peers
        .iter()
        .find(|peer| peer.name.contains(['\t', '\n']))
    {
        return Err(format!(
            "peer name {:?} may not contain tabs or line breaks",
            peer.name
        ));
    }
    if !SWITCHER_POSITIONS.contains(&config.interface.switcher_position.as_str()) {
        return Err(format!(
            "switcher_position {:?} is not one of {}",
            config.interface.switcher_position,
            SWITCHER_POSITIONS.join(", ")
        ));
    }
    if !TOUCHPAD_SPEEDS.contains(&config.touchpad.speed) {
        return Err(format!(
            "touchpad speed {} is not between {} and {}",
            config.touchpad.speed,
            TOUCHPAD_SPEEDS.start(),
            TOUCHPAD_SPEEDS.end()
        ));
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_parses() {
        let config = parse(EXAMPLE).unwrap();
        assert_eq!(
            config.peers,
            [Peer {
                name: "Desktop".into(),
                address: "192.0.2.10:4242".parse().unwrap(),
                hotkey: None,
            }]
        );
        assert_eq!(config.touchpad.speed, 16);
    }

    #[test]
    fn an_empty_file_has_no_peers_and_default_settings() {
        assert_eq!(
            parse("").unwrap(),
            Config {
                peers: vec![],
                touchpad: Touchpad::default(),
                hotkeys: Hotkeys::default(),
                interface: Interface::default(),
                this_pc: ThisPc::default(),
            }
        );
    }

    #[test]
    fn a_typo_in_a_key_is_an_error_that_names_it() {
        let err = parse("[[peer]]\nname = \"A\"\nadress = \"192.0.2.10:4242\"\n").unwrap_err();
        assert!(err.contains("adress"), "{err}");
    }

    #[test]
    fn bad_values_are_errors_not_panics() {
        assert!(parse("[[peer]]\nname = \"A\"\naddress = \"nowhere\"\n").is_err());
        assert!(parse("[touchpad]\nspeed = 0\n").is_err());
        assert!(parse("[[peer]\n").is_err());
        assert!(parse("[interface]\nswitcher_position = \"middle\"\n").is_err());
        assert!(parse("[[peer]]\nname = \"a\\tb\"\naddress = \"192.0.2.10:1\"\n").is_err());
    }

    #[test]
    fn hotkeys_default_to_the_m0_one_and_parse_per_peer() {
        let set = parse(EXAMPLE).unwrap().hotkeys().unwrap();
        assert_eq!(set.next, chord::parse(DEFAULT_NEXT).unwrap());
        assert_eq!((set.home, set.switcher), (None, None));

        let text = "[hotkeys]\nhome = \"AltGr+RightShift+H\"\n\n[[peer]]\nname = \"W\"\naddress = \"192.0.2.10:1\"\nhotkey = \"AltGr+RightShift+1\"\n";
        let set = parse(text).unwrap().hotkeys().unwrap();
        assert_eq!(set.home.unwrap().key, Some(35));
        assert_eq!(set.peers[0].as_ref().unwrap().key, Some(2));
    }

    #[test]
    fn bad_or_clashing_hotkeys_are_errors() {
        let err = parse("[hotkeys]\nhome = \"AltGr+Nope\"\n").unwrap_err();
        assert!(
            err.contains("hotkeys.home") && err.contains("Nope"),
            "{err}"
        );
        let err = parse("[hotkeys]\nhome = \"RightShift+AltGr\"\n").unwrap_err();
        assert!(err.contains("same keys"), "{err}");
    }

    #[test]
    fn this_pc_plays_both_roles_by_default() {
        let config = parse("").unwrap();
        assert!(config.this_pc.controls_others && config.this_pc.controlled);
        assert_eq!(config.this_pc.listen.port(), DEFAULT_PORT);
        let config = parse("[this_pc]\ncontrolled = false\n").unwrap();
        assert!(!config.this_pc.controlled);
    }

    #[test]
    fn errors_point_at_the_line() {
        let err = parse("\n\n[touchpad]\nspeed = \"fast\"\n").unwrap_err();
        assert!(err.contains("line 4"), "{err}");
    }
}
