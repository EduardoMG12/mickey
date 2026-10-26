//! `[audio]` (PRD 0012): which PC listens, each PC's volume, and where
//! this PC's own sound goes. Shared between PCs over the data link (kind
//! 21), newest choice first.

use crate::toml_string;
use serde::Deserialize;

/// `[audio]` (PRD 0012): which PC listens, and where this PC's sound goes.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Audio {
    /// The public key (64 hex digits) of the PC whose speakers or
    /// headphones play every PC's sound; not set, nobody: each PC plays its
    /// own. A key, not a name: each PC may name the others its own way.
    /// The same on every PC: picked on one, it is sent to the others.
    #[serde(default)]
    pub listener: Option<String>,
    /// Where this PC's own sound goes. This PC's alone, never sent.
    #[serde(default)]
    pub mode: SoundMode,
    /// Each PC's volume on the listener, 0 to 100, by public key; a PC not
    /// here plays at 100. Shared, like `listener`.
    #[serde(default)]
    pub volumes: std::collections::BTreeMap<String, u8>,
    /// The PCs muted on the listener, by public key. Shared.
    #[serde(default)]
    pub muted: Vec<String>,
    /// When the owner last changed what is shared (seconds since 1970):
    /// a PC takes another's only if it is newer, so one that starts with
    /// an old config.toml does not undo a newer choice.
    #[serde(default)]
    pub since: u64,
}

/// Where a PC's sound goes while another PC listens.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SoundMode {
    /// To the listener only: this PC's speakers stay silent.
    #[default]
    Listener,
    /// To the listener, and on this PC's speakers too.
    Both,
    /// On this PC's speakers only: nothing is sent.
    Here,
}

impl SoundMode {
    pub fn word(self) -> &'static str {
        match self {
            SoundMode::Listener => "listener",
            SoundMode::Both => "both",
            SoundMode::Here => "here",
        }
    }
}

impl Audio {
    /// The section's lines, as config.toml takes them (no header).
    pub fn to_lines(&self) -> String {
        let mut lines = String::new();
        if let Some(listener) = &self.listener {
            lines += &format!("listener = {}\n", toml_string(listener));
        }
        if self.mode != SoundMode::default() {
            lines += &format!("mode = {}\n", toml_string(self.mode.word()));
        }
        let volumes: Vec<String> = self
            .volumes
            .iter()
            .filter(|(_, volume)| **volume != 100)
            .map(|(key, volume)| format!("{} = {volume}", toml_string(key)))
            .collect();
        if !volumes.is_empty() {
            lines += &format!("volumes = {{ {} }}\n", volumes.join(", "));
        }
        if !self.muted.is_empty() {
            let muted: Vec<String> = self.muted.iter().map(|key| toml_string(key)).collect();
            lines += &format!("muted = [{}]\n", muted.join(", "));
        }
        if self.since != 0 {
            lines += &format!("since = {}\n", self.since);
        }
        lines
    }

    /// How loud the PC with public key `key` plays on the listener, 0.0 to
    /// 1.0.
    pub fn volume_of(&self, key: &str) -> f32 {
        if self.muted.iter().any(|muted| muted == key) {
            return 0.0;
        }
        f32::from(self.volumes.get(key).copied().unwrap_or(100).min(100)) / 100.0
    }

    /// What every PC shares: who listens, and each PC's volume. (Each
    /// PC's mode is its own.)
    pub fn shared_lines(&self) -> String {
        Audio {
            mode: SoundMode::default(),
            ..self.clone()
        }
        .to_lines()
    }

    /// This PC's audio with what another PC sent (`shared_lines`), if that
    /// is newer than this PC's; else this PC's, unchanged.
    pub fn taking(&self, lines: &str) -> Result<Audio, String> {
        let heard: Audio = toml::from_str(lines).map_err(|err| err.to_string())?;
        let keys = heard
            .listener
            .iter()
            .chain(heard.volumes.keys())
            .chain(&heard.muted);
        for key in keys {
            if !is_key(key) {
                return Err("a PC named by something that is not a public key".into());
            }
        }
        if heard.since <= self.since {
            return Ok(self.clone());
        }
        Ok(Audio {
            mode: self.mode,
            ..heard
        })
    }
}

/// Seconds since 1970, for `Audio::since`.
pub fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// 64 hex digits: a public key as config.toml keeps it.
pub(crate) fn is_key(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_heard_listener_keeps_this_pcs_mode() {
        let mine = Audio {
            mode: SoundMode::Here,
            ..Audio::default()
        };
        let sent = Audio {
            listener: Some(KEY_A.into()),
            mode: SoundMode::Both,
            since: 5,
            ..Audio::default()
        };
        // Only who listens travels.
        assert_eq!(
            sent.shared_lines(),
            format!("listener = \"{KEY_A}\"\nsince = 5\n")
        );
        let taken = mine.taking(&sent.shared_lines()).unwrap();
        assert_eq!(taken.listener.as_deref(), Some(KEY_A));
        assert_eq!(taken.mode, SoundMode::Here);
        assert!(
            mine.taking("listener = \"not-a-key\"\nsince = 9\n")
                .is_err()
        );
        // Nobody, chosen later.
        assert_eq!(taken.taking("since = 9\n").unwrap().listener, None);
    }

    #[test]
    fn each_pc_has_a_volume_and_a_mute_shared_with_the_others() {
        let other = "f".repeat(64);
        let audio = Audio {
            volumes: [(KEY_A.to_string(), 40), (other.clone(), 100)].into(),
            muted: vec![other.clone()],
            since: 1,
            ..Audio::default()
        };
        assert_eq!(audio.volume_of(KEY_A), 0.4);
        assert_eq!(audio.volume_of(&other), 0.0, "muted");
        assert_eq!(audio.volume_of(&"e".repeat(64)), 1.0, "not set: full");
        // 100 is not written; volumes and mutes travel.
        let lines = audio.shared_lines();
        assert_eq!(
            lines,
            format!("volumes = {{ \"{KEY_A}\" = 40 }}\nmuted = [\"{other}\"]\nsince = 1\n")
        );
        let taken = Audio::default().taking(&lines).unwrap();
        assert_eq!(taken.volume_of(KEY_A), 0.4);
        assert!(
            Audio::default()
                .taking("muted = [\"nope\"]\nsince = 1\n")
                .is_err()
        );
    }

    #[test]
    fn only_a_newer_choice_is_taken() {
        let mine = Audio {
            listener: Some(KEY_A.into()),
            since: 200,
            ..Audio::default()
        };
        let older = Audio {
            since: 100,
            ..Audio::default()
        };
        assert_eq!(
            mine.taking(&older.shared_lines()).unwrap(),
            mine,
            "an old config.toml undoes nothing"
        );
        let newer = Audio {
            since: 300,
            ..Audio::default()
        };
        assert_eq!(mine.taking(&newer.shared_lines()).unwrap().listener, None);
    }
}
