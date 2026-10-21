//! A peer's identity: its long-term X25519 key pair (PRD 0002). The private
//! key is stored in the user's data directory and never printed or sent;
//! the public key is shown to people as a fingerprint.

use snow::params::{DHChoice, HashChoice};
use snow::resolvers::{CryptoResolver, DefaultResolver};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The Noise protocol the secure link speaks.
pub const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_BLAKE2s";
const KEY_LEN: usize = 32;
const FILE_NAME: &str = "identity";
/// 160 bits of the hash: 32 base32 characters.
const FINGERPRINT_BYTES: usize = 20;
const BASE32: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

pub struct Identity {
    // ponytail: read by the Noise handshake in the next PR (#36).
    #[allow(dead_code)]
    private: [u8; KEY_LEN],
    public: [u8; KEY_LEN],
}

impl Identity {
    /// Loads this peer's identity from `dir`, creating it on first run.
    pub fn load_or_create(dir: &Path) -> Result<Identity, String> {
        let path = dir.join(FILE_NAME);
        if path.exists() {
            return load(&path);
        }
        let builder = snow::Builder::new(NOISE_PARAMS.parse().map_err(|err| format!("{err}"))?);
        let keypair = builder
            .generate_keypair()
            .map_err(|err| format!("cannot generate a key: {err}"))?;
        let private: [u8; KEY_LEN] = keypair
            .private
            .try_into()
            .map_err(|_| "unexpected private key length".to_string())?;
        save(dir, &path, &private)?;
        Identity::from_private(private)
    }

    fn from_private(private: [u8; KEY_LEN]) -> Result<Identity, String> {
        let mut dh = DefaultResolver
            .resolve_dh(&DHChoice::Curve25519)
            .ok_or("X25519 is not available")?;
        dh.set(&private);
        let public = dh
            .pubkey()
            .try_into()
            .map_err(|_| "unexpected public key length".to_string())?;
        Ok(Identity { private, public })
    }

    pub fn fingerprint(&self) -> String {
        fingerprint(&self.public)
    }
}

/// Where identities and trusted peers live: `$XDG_DATA_HOME/mickey` (or
/// `~/.local/share/mickey`) on Linux, `%APPDATA%\mickey` on Windows.
pub fn data_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".local/share")));
    base.map(|dir| dir.join("mickey"))
        .ok_or_else(|| "cannot find the user's data directory".to_string())
}

/// A hash of `public`, short enough for people to compare on two screens.
pub fn fingerprint(public: &[u8]) -> String {
    let mut hash = DefaultResolver
        .resolve_hash(&HashChoice::Blake2s)
        .expect("BLAKE2s is enabled in Cargo.toml");
    let mut digest = [0u8; 32];
    hash.input(public);
    hash.result(&mut digest);
    let text = base32(&digest[..FINGERPRINT_BYTES]);
    let groups: Vec<&str> = text
        .as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).expect("base32 is ASCII"))
        .collect();
    groups.join("-")
}

/// RFC 4648 base32 without padding: every 5 bits become one character.
fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(BASE32[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(BASE32[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

fn load(path: &Path) -> Result<Identity, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map_err(|err| format!("cannot read {}: {err}", path.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(format!(
                "{} can be read by other users; run `chmod 600 {}`",
                path.display(),
                path.display()
            ));
        }
    }
    let bytes = fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let private = bytes.try_into().map_err(|_| {
        format!(
            "{} is damaged; delete it to make a new identity",
            path.display()
        )
    })?;
    Identity::from_private(private)
}

/// Writes the private key, readable only by this user. On Windows the file
/// inherits the user profile's permissions, which already exclude others.
fn save(dir: &Path, path: &Path, private: &[u8; KEY_LEN]) -> Result<(), String> {
    let error = |err: std::io::Error| format!("cannot save {}: {err}", path.display());
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(error)?;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    fs::create_dir_all(dir).map_err(error)?;
    options
        .open(path)
        .and_then(|mut file| file.write_all(private))
        .map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty directory for one test.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mickey-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn base32_matches_rfc_4648() {
        assert_eq!(base32(b""), "");
        assert_eq!(base32(b"f"), "MY");
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
    }

    #[test]
    fn fingerprints_are_eight_groups_of_four() {
        let fp = fingerprint(&[7; KEY_LEN]);
        assert_eq!(fp.len(), 32 + 7);
        assert!(fp.split('-').all(|group| group.len() == 4));
        assert_eq!(fp, fingerprint(&[7; KEY_LEN]));
        assert_ne!(fp, fingerprint(&[8; KEY_LEN]));
    }

    #[test]
    fn identity_is_created_once_and_loaded_after() {
        let dir = temp_dir("create");
        let first = Identity::load_or_create(&dir).unwrap();
        let second = Identity::load_or_create(&dir).unwrap();
        assert_eq!(first.public, second.public);
        assert_eq!(first.private, second.private);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_key_readable_by_others_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("perms");
        Identity::load_or_create(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let err = Identity::load_or_create(&dir).err().unwrap();
        assert!(err.contains("chmod 600"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_file_is_an_error() {
        let dir = temp_dir("damaged");
        Identity::load_or_create(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), b"short").unwrap();
        let err = Identity::load_or_create(&dir).err().unwrap();
        assert!(err.contains("damaged"), "{err}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
