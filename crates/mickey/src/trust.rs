//! Trusted peers (PRD 0002): the public keys this machine has paired with.
//! A peer is trusted by its key, never by its address.
//!
//! The file `trusted_peers` sits next to the identity, one peer per line:
//!
//! ```text
//! <public key, 64 hex digits> <where it was paired from>
//! ```

use crate::identity::fingerprint;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "trusted_peers";

pub struct TrustedPeers {
    path: PathBuf,
    peers: Vec<([u8; 32], String)>,
}

impl TrustedPeers {
    /// Reads the list; a missing file means nobody is trusted yet. A damaged
    /// line is an error rather than skipped, so a typo never goes unnoticed.
    pub fn load(dir: &Path) -> Result<TrustedPeers, String> {
        let path = dir.join(FILE_NAME);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
        };
        let mut peers = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (key, note) = line.split_once(' ').unwrap_or((line, ""));
            let key = from_hex(key)
                .ok_or_else(|| format!("{} line {} is damaged", path.display(), number + 1))?;
            peers.push((key, note.to_string()));
        }
        Ok(TrustedPeers { path, peers })
    }

    pub fn is_trusted(&self, key: &[u8; 32]) -> bool {
        self.peers.iter().any(|(trusted, _)| trusted == key)
    }

    /// Adds `key` and saves the list. `note` is informational, e.g. the
    /// address the peer was paired from.
    pub fn add(&mut self, key: [u8; 32], note: &str) -> Result<(), String> {
        if !self.is_trusted(&key) {
            self.peers.push((key, note.to_string()));
        }
        self.save()
    }

    /// Removes the peer with this fingerprint (dashes and case ignored).
    /// Returns whether one was found.
    pub fn forget(&mut self, wanted: &str) -> Result<bool, String> {
        let normalize = |text: &str| text.replace('-', "").to_ascii_uppercase();
        let wanted = normalize(wanted);
        let before = self.peers.len();
        self.peers
            .retain(|(key, _)| normalize(&fingerprint(key)) != wanted);
        let found = self.peers.len() != before;
        if found {
            self.save()?;
        }
        Ok(found)
    }

    fn save(&self) -> Result<(), String> {
        let error = |err: std::io::Error| format!("cannot save {}: {err}", self.path.display());
        let mut text = String::new();
        for (key, note) in &self.peers {
            text.push_str(&to_hex(key));
            text.push(' ');
            text.push_str(note);
            text.push('\n');
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(&self.path)
            .and_then(|mut file| file.write_all(text.as_bytes()))
            .map_err(error)
    }
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut key = [0u8; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mickey-trust-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn nobody_is_trusted_at_first_and_added_peers_persist() {
        let dir = temp_dir("persist");
        let mut trusted = TrustedPeers::load(&dir).unwrap();
        assert!(!trusted.is_trusted(&[1; 32]));
        trusted.add([1; 32], "192.0.2.10").unwrap();
        let reloaded = TrustedPeers::load(&dir).unwrap();
        assert!(reloaded.is_trusted(&[1; 32]));
        assert!(!reloaded.is_trusted(&[2; 32]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn forget_takes_a_fingerprint_with_or_without_dashes() {
        let dir = temp_dir("forget");
        let mut trusted = TrustedPeers::load(&dir).unwrap();
        trusted.add([1; 32], "").unwrap();
        trusted.add([2; 32], "").unwrap();
        let plain = fingerprint(&[1; 32]).replace('-', "").to_lowercase();
        assert!(trusted.forget(&plain).unwrap());
        assert!(!trusted.forget(&plain).unwrap());
        let reloaded = TrustedPeers::load(&dir).unwrap();
        assert!(!reloaded.is_trusted(&[1; 32]));
        assert!(reloaded.is_trusted(&[2; 32]));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_line_is_an_error() {
        let dir = temp_dir("damaged");
        fs::write(dir.join(FILE_NAME), "not-a-key somewhere\n").unwrap();
        let err = TrustedPeers::load(&dir).err().unwrap();
        assert!(err.contains("line 1 is damaged"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hex_round_trips() {
        let key = [0xab; 32];
        assert_eq!(from_hex(&to_hex(&key)), Some(key));
        assert_eq!(from_hex("zz"), None);
    }
}
