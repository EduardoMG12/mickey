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
"#;

// `deny_unknown_fields`: a typo like `adress` is an error, not ignored.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, rename = "peer")]
    pub peers: Vec<Peer>,
    #[serde(default)]
    pub touchpad: Touchpad,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub name: String,
    pub address: SocketAddr,
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
    }

    #[test]
    fn errors_point_at_the_line() {
        let err = parse("\n\n[touchpad]\nspeed = \"fast\"\n").unwrap_err();
        assert!(err.contains("line 4"), "{err}");
    }
}
