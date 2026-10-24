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

pub mod chord;
pub mod peers;

use chord::Chord;
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
    /// How the other PCs see this one (PRD 0010); the host name when unset.
    #[serde(default)]
    pub name: Option<String>,
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
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Interface {
    /// Where the switcher appears: a corner or `center`.
    #[serde(default = "default_switcher_position")]
    pub switcher_position: String,
    /// The switcher's cards in a column instead of a row.
    #[serde(default)]
    pub switcher_vertical: bool,
    /// PC names in the order the switcher shows them; PCs left out follow,
    /// as the layout places them. Empty: all as the layout places them.
    #[serde(default)]
    pub switcher_order: Vec<String>,
}

impl Default for Interface {
    fn default() -> Interface {
        Interface {
            switcher_position: default_switcher_position(),
            switcher_vertical: false,
            switcher_order: Vec::new(),
        }
    }
}

impl Interface {
    /// The switcher's cards in the order to show them: each card's name and
    /// its place in the layout (`None` if not placed). The custom order goes
    /// first; then left to right as the layout places them (top to bottom
    /// in a column); unplaced PCs last. Returns indexes into `cards`.
    pub fn switcher_sort(&self, cards: &[(&str, Option<[i32; 2]>)]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..cards.len()).collect();
        order.sort_by_key(|&index| {
            let (name, place) = cards[index];
            let custom = self.switcher_order.iter().position(|wanted| wanted == name);
            let place = place.map(|[x, y]| {
                if self.switcher_vertical {
                    (y, x)
                } else {
                    (x, y)
                }
            });
            (custom.unwrap_or(usize::MAX), place.is_none(), place, index)
        });
        order
    }
}

pub const SWITCHER_POSITIONS: [&str; 5] = [
    "center",
    "top-left",
    "top-right",
    "bottom-left",
    "bottom-right",
];

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


fn default_switcher_position() -> String {
    "bottom-right".into()
}

/// `[hotkeys]`, as written; `Config::hotkeys` parses and checks them.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Hotkeys {
    pub next: Option<String>,
    pub home: Option<String>,
    pub switcher: Option<String>,
    /// One hand: the next and the previous PC in the list.
    pub forward: Option<String>,
    pub back: Option<String>,
    /// A key tapped twice quickly also goes to the next PC; `""` turns it
    /// off.
    pub double_tap: Option<String>,
    /// How close the two taps must be, in milliseconds.
    pub double_tap_ms: Option<u64>,
}

/// Every hotkey, parsed. `peers[i]` is peer `i`'s own hotkey, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct HotkeySet {
    pub next: Chord,
    pub home: Option<Chord>,
    pub switcher: Chord,
    pub forward: Chord,
    pub back: Chord,
    /// `None` when turned off.
    pub double_tap: Option<u16>,
    /// Two taps closer than this are a double tap.
    pub double_tap_window: std::time::Duration,
    pub peers: Vec<Option<Chord>>,
}

/// How close two taps must be by default, and the range allowed.
pub const DEFAULT_DOUBLE_TAP_MS: u64 = 400;
pub const DOUBLE_TAP_MS: std::ops::RangeInclusive<u64> = 150..=1000;

/// The M0 hotkey, still the default for `next`.
pub const DEFAULT_NEXT: &str = "AltGr+RightShift";
/// One-hand defaults, asked for by the owner: AltGr and the arrows next to
/// it, and a double tap on Right Shift.
pub const DEFAULT_FORWARD: &str = "AltGr+Right";
pub const DEFAULT_BACK: &str = "AltGr+Left";
/// The switcher with one hand too: AltGr and the arrow above the others.
pub const DEFAULT_SWITCHER: &str = "AltGr+Up";
pub const DEFAULT_DOUBLE_TAP: &str = "RightShift";

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
    /// Parses every hotkey; an error names the setting and the bad part,
    /// and two settings with the same keys are refused.
    pub fn hotkeys(&self) -> Result<HotkeySet, String> {
        let one = |setting: &str, text: &Option<String>| {
            text.as_deref()
                .map(chord::parse)
                .transpose()
                .map_err(|err| format!("{setting}: {err}"))
        };
        let default = |text| chord::parse(text).expect("the defaults parse");
        let double_tap = match self.hotkeys.double_tap.as_deref().map(str::trim) {
            Some("") => None,
            text => {
                let text = text.unwrap_or(DEFAULT_DOUBLE_TAP);
                let code =
                    chord::parse_key(text).map_err(|err| format!("hotkeys.double_tap: {err}"))?;
                if !chord::fits_a_double_tap(code) {
                    return Err(format!(
                        "hotkeys.double_tap: `{text}` is typed alone, so a double tap on it would get in the way; use Shift, Ctrl, Alt, AltGr, Super or an F key"
                    ));
                }
                Some(code)
            }
        };
        let window = self.hotkeys.double_tap_ms.unwrap_or(DEFAULT_DOUBLE_TAP_MS);
        if !DOUBLE_TAP_MS.contains(&window) {
            return Err(format!(
                "hotkeys.double_tap_ms: {window} is outside {} to {}",
                DOUBLE_TAP_MS.start(),
                DOUBLE_TAP_MS.end()
            ));
        }
        let set = HotkeySet {
            next: one("hotkeys.next", &self.hotkeys.next)?.unwrap_or_else(|| default(DEFAULT_NEXT)),
            home: one("hotkeys.home", &self.hotkeys.home)?,
            switcher: one("hotkeys.switcher", &self.hotkeys.switcher)?
                .unwrap_or_else(|| default(DEFAULT_SWITCHER)),
            forward: one("hotkeys.forward", &self.hotkeys.forward)?
                .unwrap_or_else(|| default(DEFAULT_FORWARD)),
            back: one("hotkeys.back", &self.hotkeys.back)?.unwrap_or_else(|| default(DEFAULT_BACK)),
            double_tap,
            double_tap_window: std::time::Duration::from_millis(window),
            peers: self
                .peers
                .iter()
                .map(|peer| one(&format!("hotkey of {}", peer.name), &peer.hotkey))
                .collect::<Result<_, _>>()?,
        };
        let all: Vec<&Chord> = [
            Some(&set.next),
            set.home.as_ref(),
            Some(&set.switcher),
            Some(&set.forward),
            Some(&set.back),
        ]
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

/// `text` (a config.toml) with the peer named `name` placed at `position`:
/// its `position` line replaced, or added under its `name`. Every other
/// line, comments included, is kept as it was. The result is checked with
/// the same rules as `parse`.
pub fn set_position(text: &str, name: &str, position: [i32; 2]) -> Result<String, String> {
    let wanted = format!("position = [{}, {}]", position[0], position[1]);
    set_peer_field(text, name, "position", wanted)
}

/// `text` with the peer named `name` bound to the public key `key` (64 hex
/// digits): Mickey then talks to that PC only with that key (#184).
pub fn set_key(text: &str, name: &str, key: &str) -> Result<String, String> {
    set_peer_field(text, name, "key", format!("key = {}", toml_string(key)))
}

/// `text` with the `field` line of the peer named `name` replaced by
/// `wanted`, or `wanted` added under its `name`. Every other line,
/// comments included, is kept; the result is checked like `parse`.
fn set_peer_field(text: &str, name: &str, field: &str, wanted: String) -> Result<String, String> {
    let is_field = |line: &str| {
        line.strip_prefix(field)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    };
    let is_our_name = |line: &str| {
        line.starts_with("name")
            && toml::from_str::<toml::Table>(line)
                .ok()
                .and_then(|table| table.get("name")?.as_str().map(|value| value == name))
                == Some(true)
    };
    let mut out: Vec<String> = Vec::new();
    let mut in_peer = false;
    // Where our peer's name line is in `out`, while inside its block.
    let mut ours: Option<usize> = None;
    let mut done = false;
    for line in text.lines().chain(std::iter::once("[end]")) {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            // Leaving our block without a position line: add one under its name.
            if let Some(at) = ours.take()
                && !done
            {
                out.insert(at + 1, wanted.clone());
                done = true;
            }
            in_peer = trimmed.starts_with("[[peer]]");
        }
        if in_peer && !done && is_our_name(trimmed) {
            ours = Some(out.len());
        }
        if ours.is_some() && !done && is_field(trimmed) {
            out.push(wanted.clone());
            done = true;
            continue;
        }
        out.push(line.to_string());
    }
    out.pop(); // the "[end]" sentinel
    if !done {
        return Err(format!("no PC named {name:?} in config.toml"));
    }
    let mut text = out.join("\n");
    text.push('\n');
    parse(&text)?;
    Ok(text)
}

impl Hotkeys {
    /// The section's lines, as config.toml takes them (no header). This is
    /// what one PC sends the others so all share the same hotkeys.
    pub fn to_lines(&self) -> String {
        let mut lines = Vec::new();
        for (key, value) in [
            ("next", &self.next),
            ("home", &self.home),
            ("switcher", &self.switcher),
            ("forward", &self.forward),
            ("back", &self.back),
            ("double_tap", &self.double_tap),
        ] {
            if let Some(value) = value {
                lines.push(format!("{key} = {}", toml_string(value)));
            }
        }
        if let Some(ms) = self.double_tap_ms {
            lines.push(format!("double_tap_ms = {ms}"));
        }
        lines.join("\n")
    }

    /// Reads lines made by `to_lines`, e.g. from another PC.
    pub fn from_lines(lines: &str) -> Result<Hotkeys, String> {
        toml::from_str(lines).map_err(|err| err.to_string())
    }
}

/// `text` (a config.toml) with its `[hotkeys]` section replaced by
/// `hotkeys`, in place, or added at the end. Other lines are kept, and the
/// result is checked with `parse`.
pub fn set_hotkeys(text: &str, hotkeys: &Hotkeys) -> Result<String, String> {
    let section = format!("[hotkeys]\n{}", hotkeys.to_lines());
    let mut out: Vec<String> = Vec::new();
    let (mut skipping, mut done) = (false, false);
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            skipping = trimmed.starts_with("[hotkeys]");
            if skipping {
                out.push(section.clone());
                out.push(String::new());
                done = true;
            }
        }
        if !skipping {
            out.push(line.to_string());
        }
    }
    if !done {
        out.push(String::new());
        out.push(section);
    }
    let mut text = out.join("\n").trim_end().to_string();
    text.push('\n');
    parse(&text)?;
    Ok(text)
}

/// `text` (a config.toml) with a new peer at its end. A name already
/// taken gets " (2)", " (3)"… so two PCs never share one. Checked with
/// `parse`; returns the text and the name used.
pub fn add_peer(
    text: &str,
    name: &str,
    address: SocketAddr,
    position: Option<[i32; 2]>,
) -> Result<(String, String), String> {
    let taken = parse(text)?
        .peers
        .into_iter()
        .map(|peer| peer.name)
        .collect::<Vec<_>>();
    let name = (1..)
        .map(|n| {
            if n == 1 {
                name.to_string()
            } else {
                format!("{name} ({n})")
            }
        })
        .find(|candidate| !taken.contains(candidate))
        .expect("an unused name exists");
    let mut block = format!(
        "\n[[peer]]\nname = {}\naddress = \"{address}\"\n",
        toml_string(&name)
    );
    if let Some([x, y]) = position {
        block += &format!("position = [{x}, {y}]\n");
    }
    let mut text = text.trim_end().to_string();
    text += "\n";
    text += &block;
    parse(&text)?;
    Ok((text, name))
}

/// A TOML basic string: quotes and backslashes escaped.
fn toml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
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
    fn an_empty_file_has_no_peers_and_default_settings() {
        assert_eq!(
            parse("").unwrap(),
            Config {
                peers: vec![],
                touchpad: Touchpad::default(),
                hotkeys: Hotkeys::default(),
                interface: Interface::default(),
                clipboard: Clipboard::default(),
                this_pc: ThisPc::default(),
            }
        );
        assert_eq!(
            parse("").unwrap().clipboard.history,
            None,
            "automatic unless set"
        );
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
