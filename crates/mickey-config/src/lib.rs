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

pub mod audio;
pub mod chord;
mod edit;
mod hotkeys;
mod interface;
pub mod peers;
pub mod version;

pub use audio::{Audio, SoundMode, now_seconds};
pub use edit::{add_peer, set_audio, set_hotkeys, set_key, set_position, toml_string};
pub use hotkeys::{
    DEFAULT_BACK, DEFAULT_DOUBLE_TAP, DEFAULT_DOUBLE_TAP_MS, DEFAULT_FORWARD, DEFAULT_NEXT,
    DEFAULT_SWITCHER, DOUBLE_TAP_MS, HotkeySet, Hotkeys,
};
pub use interface::{Interface, SWITCHER_POSITIONS};

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
    pub clipboard: Clipboard,
    #[serde(default)]
    pub audio: Audio,
    #[serde(default)]
    pub updates: Updates,
    #[serde(default)]
    pub this_pc: ThisPc,
}

/// `[updates]` (PRD 0013).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Updates {
    /// Ask GitHub once a day whether a newer Mickey is out. Off, Mickey
    /// never contacts the internet.
    #[serde(default = "yes")]
    pub check: bool,
}

impl Default for Updates {
    fn default() -> Updates {
        Updates { check: true }
    }
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
    /// How the other PCs see this one (PRD 0010); the host name when unset.
    #[serde(default)]
    pub name: Option<String>,
    /// Share `[hotkeys]` with the other PCs, and be asked when one shares
    /// its own (#349). Off, this PC keeps its hotkeys to itself: one with a
    /// different keyboard may want others.
    #[serde(default = "yes")]
    pub share_hotkeys: bool,
}

impl ThisPc {
    /// `name`, or the host name, or "Mickey" when even that is unknown.
    pub fn name(&self) -> String {
        self.name
            .clone()
            .or_else(host_name)
            .unwrap_or_else(|| "Mickey".into())
    }
}

/// The machine's host name, without any crate: Linux keeps it in /proc,
/// Windows in the COMPUTERNAME variable.
fn host_name() -> Option<String> {
    let name = if cfg!(windows) {
        std::env::var("COMPUTERNAME").ok()?
    } else {
        std::fs::read_to_string("/proc/sys/kernel/hostname").ok()?
    };
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_string())
}

impl Default for ThisPc {
    fn default() -> ThisPc {
        ThisPc {
            controls_others: true,
            controlled: true,
            listen: default_listen(),
            name: None,
            share_hotkeys: true,
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

/// `[clipboard]`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Clipboard {
    /// Keep a history of the copies on this PC. `false` leaves it to
    /// another clipboard manager (cliphist, CopyQ, Klipper, Windows'
    /// Win+V): the clipboard is still shared, and what Mickey puts in it
    /// reaches that manager. Not set: Mickey keeps one only when no other
    /// manager runs (#257).
    #[serde(default)]
    pub history: Option<bool>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub name: String,
    pub address: SocketAddr,
    /// Goes straight to this peer, e.g. `"AltGr+RightShift+1"`.
    #[serde(default)]
    pub hotkey: Option<String>,
    /// Where the peer's desktop sits next to this one (PRD 0005): its point
    /// 0,0 is at `[x, y]` of this PC's desktop, so its monitors are its own
    /// positions plus this. The cursor crosses wherever the two desktops
    /// touch. None: no edges, only hotkeys.
    #[serde(default)]
    pub position: Option<[i32; 2]>,
    /// The peer's public key (64 hex digits), written when it first
    /// connects: from then on Mickey talks to this address only with this
    /// key, so another trusted PC that takes over the address gets nothing
    /// meant for this one (#184).
    #[serde(default)]
    pub key: Option<String>,
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
    /// Every setting at its default: what an empty config.toml gives.
    pub fn empty() -> Config {
        parse("").expect("an empty config is valid")
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

/// Writes `text` to the config file at `path`, creating its folder first:
/// on a fresh install `~/.config/mickey` does not exist yet, and the first
/// PC connected must not fail on that.
pub fn write(path: &std::path::Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    }
    std::fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))
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
    // It travels in discovery packets, tab-separated lines, one length byte.
    if let Some(name) = &config.this_pc.name
        && (name.contains(['\t', '\n']) || name.is_empty() || name.len() > 64)
    {
        return Err(format!(
            "this PC's name {name:?} must be 1 to 64 bytes, without tabs or line breaks"
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

    #[test]
    fn the_next_hotkey_names_the_default_when_unset() {
        assert_eq!(Config::empty().hotkeys.next_or_default(), DEFAULT_NEXT);
        let set = parse("[hotkeys]\nnext = \"AltGr+Down\"\n").unwrap();
        assert_eq!(set.hotkeys.next_or_default(), "AltGr+Down");
    }
    #[test]
    fn a_peer_gets_its_key_in_place() {
        let text =
            "[[peer]]\nname = \"Windows\"\naddress = \"192.0.2.7:4242\"\nkeyboard_note = 1\n";
        let text = text.replace("keyboard_note = 1\n", "");
        let key = "ab".repeat(32);
        let with = set_key(&text, "Windows", &key).unwrap();
        assert_eq!(
            parse(&with).unwrap().peers[0].key.as_deref(),
            Some(key.as_str())
        );
        let again = set_key(&with, "Windows", &"cd".repeat(32)).unwrap();
        assert_eq!(
            again.matches("key =").count(),
            1,
            "replaced, not added twice"
        );
    }

    use super::*;

    #[test]
    fn shared_hotkeys_are_on_unless_this_pc_says_no() {
        assert!(Config::empty().this_pc.share_hotkeys);
        let config = parse("[this_pc]\nshare_hotkeys = false\n").unwrap();
        assert!(!config.this_pc.share_hotkeys);
    }

    #[test]
    fn changes_name_each_hotkey_that_would_change() {
        let mine = Hotkeys::default();
        let same = Hotkeys {
            next: Some(DEFAULT_NEXT.into()),
            ..Hotkeys::default()
        };
        assert!(
            mine.changes(&same).is_empty(),
            "written or default, the same"
        );
        let theirs = Hotkeys {
            next: Some("LeftCtrl+F12".into()),
            double_tap: Some(String::new()),
            double_tap_ms: Some(250),
            ..Hotkeys::default()
        };
        assert_eq!(
            mine.changes(&theirs),
            [
                "Next PC / switcher: LeftCtrl+F12 (now AltGr+RightShift)",
                "Double tap: off (now RightShift)",
                "Double tap window: 250 ms (now 400 ms)",
            ]
        );
    }

    #[test]
    fn write_creates_the_missing_folder() {
        // A fresh install: neither the folder nor the file exists.
        let base = std::env::temp_dir().join(format!("mickey-config-test-{}", std::process::id()));
        let path = base.join("mickey").join(FILE_NAME);
        let _ = std::fs::remove_dir_all(&base);
        write(&path, "name = \"desk\"\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "name = \"desk\"\n");
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn a_double_tap_needs_a_key_nobody_types_alone_and_a_sane_window() {
        let hotkeys =
            |body: &str| parse(&format!("[hotkeys]\n{body}\n")).and_then(|config| config.hotkeys());
        assert_eq!(
            hotkeys("double_tap = \"LeftCtrl\"").unwrap().double_tap,
            Some(29)
        );
        assert_eq!(hotkeys("double_tap = \"F9\"").unwrap().double_tap, Some(67));
        let letter = hotkeys("double_tap = \"W\"").unwrap_err();
        assert!(letter.contains("typed alone"), "{letter}");
        let window = hotkeys("double_tap_ms = 600").unwrap().double_tap_window;
        assert_eq!(window, std::time::Duration::from_millis(600));
        assert!(hotkeys("double_tap_ms = 50").is_err());
        assert_eq!(hotkeys("").unwrap().double_tap_window.as_millis(), 400);
    }

    #[test]
    fn hotkeys_travel_as_lines_and_replace_the_section_in_place() {
        let config = "[[peer]]\nname = \"Windows\"\naddress = \"192.0.2.7:4242\"\n\n[hotkeys]\nnext = \"AltGr+RightShift\"\n\n[touchpad]\nspeed = 16\n";
        let mine = Hotkeys {
            forward: Some("RightCtrl+Right".into()),
            double_tap: Some("LeftCtrl".into()),
            double_tap_ms: Some(500),
            ..Hotkeys::default()
        };
        let read = Hotkeys::from_lines(&mine.to_lines()).unwrap();
        assert_eq!(read, mine);
        let text = set_hotkeys(config, &mine).unwrap();
        let parsed = parse(&text).unwrap();
        assert_eq!(parsed.hotkeys, mine);
        assert_eq!(parsed.touchpad.speed, 16);
        assert!(
            text.find("[hotkeys]") < text.find("[touchpad]"),
            "kept in place"
        );
        let added = set_hotkeys("[touchpad]\nspeed = 3\n", &mine).unwrap();
        assert_eq!(parse(&added).unwrap().hotkeys, mine);
    }

    #[test]
    fn the_switcher_follows_the_layout_unless_told_otherwise() {
        let cards = [
            ("Laptop", Some([0, 0])),
            ("Windows", Some([-1920, 0])),
            ("Spare", None),
            ("Above", Some([200, -900])),
        ];
        let mut interface = Interface::default();
        assert_eq!(
            interface.switcher_sort(&cards),
            [1, 0, 3, 2],
            "left to right"
        );
        interface.switcher_vertical = true;
        assert_eq!(
            interface.switcher_sort(&cards),
            [3, 1, 0, 2],
            "top to bottom"
        );
        interface.switcher_order = vec!["Spare".into(), "Laptop".into()];
        assert_eq!(
            interface.switcher_sort(&cards),
            [2, 0, 3, 1],
            "custom first"
        );
    }

    #[test]
    fn a_peer_is_added_at_the_end_with_a_free_name() {
        let config = "[[peer]]\nname = \"Windows\"\naddress = \"192.0.2.7:4242\"\n\n[touchpad]\nspeed = 16\n";
        let address = "192.0.2.64:4242".parse().unwrap();
        let (text, name) = add_peer(config, "Notebook 2", address, Some([5, -6])).unwrap();
        assert_eq!(name, "Notebook 2");
        let parsed = parse(&text).unwrap();
        assert_eq!(parsed.peers[1].position, Some([5, -6]));
        assert_eq!(parsed.touchpad.speed, 16);
        let (_, again) = add_peer(&text, "Windows", address, None).unwrap();
        assert_eq!(again, "Windows (2)");
    }

    #[test]
    fn a_position_is_set_without_touching_the_rest() {
        let config = "# mine\n[[peer]]\nname = \"Windows\"\naddress = \"192.0.2.10:4242\"\n\n[[peer]]\nname = \"Desk\"\naddress = \"192.0.2.11:4242\"\nposition = [1, 2]    # old\n\n[touchpad]\nspeed = 16\n";
        let added = set_position(config, "Windows", [1536, -216]).unwrap();
        assert!(
            added.starts_with(
                "# mine\n[[peer]]\nname = \"Windows\"\nposition = [1536, -216]\naddress"
            ),
            "{added}"
        );
        let parsed = parse(&added).unwrap();
        assert_eq!(parsed.peers[0].position, Some([1536, -216]));
        assert_eq!(parsed.peers[1].position, Some([1, 2]));
        let replaced = set_position(&added, "Desk", [-5, 6]).unwrap();
        assert_eq!(parse(&replaced).unwrap().peers[1].position, Some([-5, 6]));
        assert!(replaced.contains("speed = 16"));
        assert!(set_position(config, "Nobody", [0, 0]).is_err());
    }

    #[test]
    fn a_position_is_two_numbers() {
        let peer = |position: &str| {
            format!(
                "[[peer]]\nname = \"A\"\naddress = \"192.0.2.10:4242\"\nposition = {position}\n"
            )
        };
        assert_eq!(
            parse(&peer("[-1536, 216]")).unwrap().peers[0].position,
            Some([-1536, 216])
        );
        assert!(parse(&peer("[1]")).is_err());
        assert!(parse(&peer("\"left\"")).is_err());
    }

    #[test]
    fn the_example_parses() {
        let config = parse(EXAMPLE).unwrap();
        assert_eq!(
            config.peers,
            [Peer {
                name: "Desktop".into(),
                address: "192.0.2.10:4242".parse().unwrap(),
                hotkey: None,
                position: None,
                key: None,
            }]
        );
        assert_eq!(config.touchpad.speed, 16);
    }

    #[test]
    fn the_update_check_is_on_unless_turned_off() {
        assert!(parse("").unwrap().updates.check);
        assert!(!parse("[updates]\ncheck = false\n").unwrap().updates.check);
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
                clipboard: Clipboard::default(),
                audio: Audio::default(),
                updates: Updates { check: true },
                this_pc: ThisPc::default(),
            }
        );
        assert_eq!(
            parse("").unwrap().clipboard.history,
            None,
            "automatic unless set"
        );
    }

    const KEY_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn audio_is_nobody_listening_unless_set() {
        assert_eq!(parse("").unwrap().audio, Audio::default());
        let config = parse(&format!(
            "[audio]\nlistener = \"{KEY_A}\"\nmode = \"both\"\n"
        ))
        .unwrap();
        assert_eq!(config.audio.listener.as_deref(), Some(KEY_A));
        assert_eq!(config.audio.mode, SoundMode::Both);
        assert!(parse("[audio]\nmode = \"loud\"\n").is_err());
    }

    #[test]
    fn the_audio_section_is_replaced_in_place() {
        let text = "[clipboard]\nhistory = false\n\n[audio]\nmode = \"here\"\n\n[this_pc]\nname = \"Desk\"\n";
        let audio = Audio {
            listener: Some(KEY_A.into()),
            mode: SoundMode::Here,
            volumes: [(KEY_A.to_string(), 40)].into(),
            muted: vec![KEY_A.into()],
            since: 7,
        };
        let new = set_audio(text, &audio).unwrap();
        let config = parse(&new).unwrap();
        assert_eq!(config.audio, audio);
        assert_eq!(config.this_pc.name(), "Desk", "other sections kept");
        assert_eq!(new.matches("[audio]").count(), 1);
    }

    #[test]
    fn the_history_can_be_left_to_another_manager() {
        let config = parse("[clipboard]\nhistory = false\n").unwrap();
        assert_eq!(config.clipboard.history, Some(false));
        let config = parse("[clipboard]\nhistory = true\n").unwrap();
        assert_eq!(config.clipboard.history, Some(true));
        assert!(
            parse("[clipboard]\nhistroy = false\n").is_err(),
            "a typo is an error"
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
        assert_eq!(set.home, None);
        assert_eq!(set.switcher, chord::parse(DEFAULT_SWITCHER).unwrap());

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
        assert!(
            !config.this_pc.name().is_empty(),
            "the host name, or Mickey"
        );
        let named = parse("[this_pc]\nname = \"Desk\"\n").unwrap();
        assert_eq!(named.this_pc.name(), "Desk");
    }

    #[test]
    fn errors_point_at_the_line() {
        let err = parse("\n\n[touchpad]\nspeed = \"fast\"\n").unwrap_err();
        assert!(err.contains("line 4"), "{err}");
    }
}
