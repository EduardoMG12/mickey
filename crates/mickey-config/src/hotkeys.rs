//! `[hotkeys]`: the chords that move the keyboard and mouse between PCs.

use crate::chord::{self, Chord};
use crate::{Config, toml_string};
use serde::Deserialize;

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

impl Hotkeys {
    /// The hotkey to the next PC, as written (the default if unset).
    pub fn next_or_default(&self) -> &str {
        self.next.as_deref().unwrap_or(DEFAULT_NEXT)
    }

    /// What taking `theirs` would change here, one line per hotkey as the
    /// settings name it, e.g. `Next PC / switcher: LeftCtrl+F12 (now
    /// AltGr+RightShift)`. Unset hotkeys count as their defaults, so two PCs
    /// that only write them differently change nothing. Empty when equal.
    pub fn changes(&self, theirs: &Hotkeys) -> Vec<String> {
        // Spaces and line breaks collapsed: a peer's value is shown on one
        // line, whatever it was padded with.
        let shown = |text: &Option<String>, default: &str| {
            let text = text.as_deref().unwrap_or(default);
            match text.split_whitespace().collect::<Vec<_>>().join(" ") {
                text if text.is_empty() => "off".to_string(),
                text => text,
            }
        };
        let rows = [
            ("Next PC / switcher", &self.next, &theirs.next, DEFAULT_NEXT),
            (
                "Back to this PC",
                &self.home,
                &theirs.home,
                "the hotkey above + 0",
            ),
            (
                "Open the switcher",
                &self.switcher,
                &theirs.switcher,
                DEFAULT_SWITCHER,
            ),
            (
                "Next PC (one hand)",
                &self.forward,
                &theirs.forward,
                DEFAULT_FORWARD,
            ),
            (
                "Previous PC (one hand)",
                &self.back,
                &theirs.back,
                DEFAULT_BACK,
            ),
            (
                "Double tap",
                &self.double_tap,
                &theirs.double_tap,
                DEFAULT_DOUBLE_TAP,
            ),
        ];
        let mut lines: Vec<String> = rows
            .iter()
            .filter_map(|(label, mine, their, default)| {
                let (mine, their) = (shown(mine, default), shown(their, default));
                (mine != their).then(|| format!("{label}: {their} (now {mine})"))
            })
            .collect();
        let window = |ms: Option<u64>| ms.unwrap_or(DEFAULT_DOUBLE_TAP_MS);
        let (mine, their) = (window(self.double_tap_ms), window(theirs.double_tap_ms));
        if mine != their {
            lines.push(format!("Double tap window: {their} ms (now {mine} ms)"));
        }
        lines
    }
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
}
