//! Finding a newer Mickey (PRD 0013). Once a day Mickey asks GitHub which
//! release is the latest; a paired PC that already runs a newer one says so
//! too (payload kind 23). What is found waits here for the tray and the
//! settings window to show.
//!
//! The request goes through the system's `curl` (every Linux desktop and
//! Windows 10 1803 or newer have it), so Mickey carries no HTTP or TLS code.
//! It sends nothing about the owner: only what any HTTPS request sends.

use mickey_config::version;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const LATEST: &str = "https://api.github.com/repos/EduardoMG12/mickey/releases/latest";

/// The first check waits this long after Mickey starts: the network may
/// not be up yet, and starting stays quick.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The newest version heard of that is newer than this one, if any.
#[derive(Clone, Default)]
pub struct Updates(Arc<Mutex<Option<String>>>);

impl Updates {
    /// Remembers `found` if it is newer than this build and than what was
    /// heard before.
    pub fn heard(&self, found: &str) {
        let mut waiting = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let newest = waiting.as_deref().unwrap_or(version::THIS);
        if version::is_newer(found, newest) {
            *waiting = Some(found.trim_start_matches('v').to_string());
        }
    }

    /// The version to update to, if one is waiting.
    pub fn waiting(&self) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// Checks now, then every day, while `[updates] check` is on (read each
/// time, so turning it off in the settings takes effect at once).
pub fn start(updates: Updates) {
    crate::threads::spawn("update check", move || {
        std::thread::sleep(FIRST_CHECK);
        loop {
            let wanted = mickey_config::Config::load()
                .ok()
                .flatten()
                .is_none_or(|config| config.updates.check);
            if wanted {
                match latest() {
                    Ok(found) => updates.heard(&found),
                    Err(err) => eprintln!("mickey: could not check for an update: {err}"),
                }
            }
            std::thread::sleep(EVERY);
        }
    });
}

/// The latest release's version, as GitHub tags it (`v0.2.2`).
pub fn latest() -> Result<String, String> {
    let json = fetch(LATEST, &["--max-time", "20"])?;
    let json = String::from_utf8_lossy(&json);
    let tag = tag_name(&json).ok_or("GitHub's answer names no release")?;
    version::checked(tag).ok_or_else(|| format!("the latest release, {tag:?}, is not a version"))
}

/// The body at `url`, through curl: HTTPS only, redirects followed (a
/// release's files are served from another GitHub host).
fn fetch(url: &str, limits: &[&str]) -> Result<Vec<u8>, String> {
    // Windows: System32's own curl, by its full path, never one found first
    // elsewhere.
    #[cfg(windows)]
    let mut curl = Command::new(crate::autostart::system_tool("curl.exe"));
    #[cfg(not(windows))]
    let mut curl = Command::new("curl");
    curl.args(["--silent", "--show-error", "--fail", "--location"])
        .args(["--proto", "=https", "--proto-redir", "=https"])
        .args(limits)
        .args(["--user-agent", &format!("mickey/{}", version::THIS)])
        .args(["--header", "Accept: application/vnd.github+json"])
        .arg(url);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: no console flashing up.
        curl.creation_flags(0x0800_0000);
    }
    let answer = curl
        .output()
        .map_err(|err| format!("cannot run curl: {err}"))?;
    if !answer.status.success() {
        return Err(String::from_utf8_lossy(&answer.stderr).trim().to_string());
    }
    Ok(answer.stdout)
}

/// The public key that signs releases, built in (`packaging/release-key.pub`,
/// made by `tools/release-key/new-key.sh`).
const RELEASE_KEY: &str = include_str!("../../../packaging/release-key.pub");

/// The release key's 32 bytes; `None` until the owner made one.
fn release_key(text: &str) -> Option<[u8; 32]> {
    let hex = text
        .lines()
        .find(|line| !line.starts_with('#') && !line.trim().is_empty())?;
    let hex = hex.trim();
    if hex.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(key)
}

/// This system's file in `version`'s release: an exact name, so nothing
/// listed can point elsewhere.
fn asset(version: &str) -> String {
    if cfg!(windows) {
        format!("Mickey-Setup-{version}.exe")
    } else {
        "mickey-linux-x86_64.tar.gz".into()
    }
}

/// Whether `file`, called `name`, is `version` as the owner released it:
/// `sums` (a release's SHA256SUMS) carries a valid Ed25519 `signature` from
/// `key`, lists `name` with `file`'s SHA-256, and is `version`'s list (it
/// names `version`'s Windows installer), so an older release, signed too,
/// cannot be served as a newer one. Nothing else is ever installed.
fn verified(
    sums: &[u8],
    signature: &[u8],
    key: &[u8; 32],
    version: &str,
    name: &str,
    file: &[u8],
) -> Result<(), String> {
    use ed25519_dalek::{Signature, VerifyingKey};
    use sha2::{Digest, Sha256};
    let key = VerifyingKey::from_bytes(key).map_err(|_| "the release key is not a valid key")?;
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| "the signature is not 64 bytes")?;
    key.verify_strict(sums, &Signature::from_bytes(&signature))
        .map_err(|_| "SHA256SUMS is not signed by the release key")?;
    let sums = std::str::from_utf8(sums).map_err(|_| "SHA256SUMS is not text")?;
    let names_version = sums
        .lines()
        .filter_map(|line| line.split_once("  "))
        .any(|(_, listed)| listed == format!("Mickey-Setup-{version}.exe"));
    if !names_version {
        return Err(format!("this SHA256SUMS is not Mickey {version}'s"));
    }
    let listed = sums
        .lines()
        .filter_map(|line| line.split_once("  "))
        .find(|(_, listed)| *listed == name)
        .map(|(hash, _)| hash)
        .ok_or_else(|| format!("{name} is not in SHA256SUMS"))?;
    let hash: String = Sha256::digest(file)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if hash != listed {
        return Err(format!("{name} is not the file that was released"));
    }
    Ok(())
}

/// Where a verified release waits to be installed. On Windows, a Mickey
/// running as administrator starts the installer without a prompt, so the
/// file must sit where only administrators can change it (`state`, next to
/// mickey.exe): in %APPDATA% any program of the user could swap it between
/// the check and the start, and run as administrator.
fn updates_dir() -> Result<std::path::PathBuf, String> {
    #[cfg(windows)]
    if crate::platform::acl::elevated() {
        let state = crate::trust::dir()?;
        if !crate::platform::acl::admin_only(&state).unwrap_or(false) {
            return Err("Mickey runs as administrator without a folder only administrators can change: install the update by hand".into());
        }
        return Ok(state.join("updates"));
    }
    Ok(crate::identity::data_dir()?.join("updates"))
}

/// Downloads `version`'s file for this system into Mickey's data folder and
/// checks it (`verified`). Its path, once it may be installed.
pub fn download(version: &str) -> Result<std::path::PathBuf, String> {
    let key = release_key(RELEASE_KEY).ok_or(
        "this Mickey has no release key, so it cannot check an update: install the new one by hand",
    )?;
    let version = version::checked(version).ok_or("not a version")?;
    let base = format!("https://github.com/EduardoMG12/mickey/releases/download/v{version}");
    // Small files: a short limit. The release itself: a few minutes.
    let small = ["--max-time", "30", "--max-filesize", "65536"];
    let sums = fetch(&format!("{base}/SHA256SUMS"), &small)?;
    let signature = fetch(&format!("{base}/SHA256SUMS.sig"), &small)?;
    let name = asset(&version);
    let file = fetch(
        &format!("{base}/{name}"),
        &["--max-time", "600", "--max-filesize", "209715200"],
    )?;
    verified(&sums, &signature, &key, &version, &name, &file)?;
    let dir = updates_dir()?.join(&version);
    std::fs::create_dir_all(&dir).map_err(|err| format!("cannot make {}: {err}", dir.display()))?;
    let path = dir.join(&name);
    std::fs::write(&path, &file)
        .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    Ok(path)
}

/// Downloads `version`, checks it (`download`) and installs it. The
/// running Mickey is then stopped and started again by the installer, the
/// normal way, which releases every grab (hard rule 8).
pub fn apply(version: &str) -> Result<(), String> {
    let path = download(version)?;
    install(&path)
}

/// Linux: unpacks the tarball and runs its `install.sh --update` (no sudo)
/// in a unit of its own, so restarting Mickey's service does not stop it.
#[cfg(target_os = "linux")]
fn install(tarball: &std::path::Path) -> Result<(), String> {
    let script = unpack(tarball)?;
    let started = Command::new("systemd-run")
        .args(["--user", "--collect", "--quiet", "--unit", "mickey-update"])
        .arg(&script)
        .arg("--update")
        .status()
        .map_err(|err| format!("cannot run systemd-run: {err}"))?;
    if !started.success() {
        return Err("cannot start the installer".into());
    }
    Ok(())
}

/// Unpacks a verified Linux release next to it; its `install.sh`.
#[cfg(target_os = "linux")]
fn unpack(tarball: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let dir = tarball.parent().ok_or("the download has no folder")?;
    let unpacked = Command::new("tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(dir)
        .status()
        .map_err(|err| format!("cannot run tar: {err}"))?;
    if !unpacked.success() {
        return Err("cannot unpack the release".into());
    }
    Ok(dir.join("mickey-linux-x86_64").join("install.sh"))
}

/// A first install on a PC (the MCP server's `install`): the latest
/// release, downloaded and checked like an update. What happened, for the
/// person to read.
pub fn install_latest() -> Result<String, String> {
    let latest = latest()?;
    let path = download(&latest)?;
    #[cfg(target_os = "linux")]
    {
        // The first install asks for sudo once (input devices, firewall):
        // that needs the person at a terminal, never an agent.
        let script = unpack(&path)?;
        Ok(format!(
            "Mickey {latest} is downloaded and its signature checked. To install it, run this in a terminal (it asks for your password once):\n{}",
            script.display()
        ))
    }
    #[cfg(windows)]
    {
        install(&path)?;
        Ok(format!(
            "Installing Mickey {latest}: confirm Windows' administrator prompt. Mickey then starts by itself."
        ))
    }
}

/// Windows: runs the installer silently (`/S`) as administrator; Windows
/// asks the owner once. The installer stops Mickey, replaces it and starts
/// it again.
#[cfg(windows)]
fn install(installer: &std::path::Path) -> Result<(), String> {
    crate::platform::elevate::run_as_admin(installer, "/S").map_err(|err| err.to_string())
}

/// The value of `"tag_name"` in GitHub's answer. A full JSON parser for one
/// field would be a dependency for nothing; the tag never holds a quote.
fn tag_name(json: &str) -> Option<&str> {
    let after = &json[json.find("\"tag_name\"")? + "\"tag_name\"".len()..];
    let value = after.trim_start().strip_prefix(':')?.trim_start();
    let value = value.strip_prefix('"')?;
    Some(&value[..value.find('"')?])
}

#[cfg(test)]
mod tests {
    use super::*;

    use ed25519_dalek::{Signer, SigningKey};

    /// Version 0.3.0's release signed by `signer`: its SHA256SUMS (which,
    /// as release.yml writes it, lists the Windows installer by version)
    /// and the signature.
    fn release(signer: &SigningKey, file: &[u8]) -> (Vec<u8>, Vec<u8>) {
        use sha2::{Digest, Sha256};
        let hash: String = Sha256::digest(file)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let sums = format!("{hash}  mickey-linux-x86_64.tar.gz\n{hash}  Mickey-Setup-0.3.0.exe\n")
            .into_bytes();
        let signature = signer.sign(&sums).to_bytes().to_vec();
        (sums, signature)
    }

    #[test]
    fn a_signed_release_is_installed_and_any_change_is_refused() {
        let owner = SigningKey::from_bytes(&[7; 32]);
        let key = owner.verifying_key().to_bytes();
        let file = b"the release";
        let name = "mickey-linux-x86_64.tar.gz";
        let (sums, signature) = release(&owner, file);
        assert_eq!(
            verified(&sums, &signature, &key, "0.3.0", name, file),
            Ok(())
        );
        // Another file than the one released.
        assert!(verified(&sums, &signature, &key, "0.3.0", name, b"the release!").is_err());
        // SHA256SUMS changed after signing (to list another file's hash).
        let mut changed = sums.clone();
        changed[0] ^= 1;
        assert!(verified(&changed, &signature, &key, "0.3.0", name, file).is_err());
        // Signed by someone else.
        let stranger = SigningKey::from_bytes(&[8; 32]);
        let (sums, signature) = release(&stranger, file);
        assert!(verified(&sums, &signature, &key, "0.3.0", name, file).is_err());
        // Cut short, or a name not in the list.
        assert!(verified(&sums, &signature[..63], &key, "0.3.0", name, file).is_err());
        let (sums, signature) = release(&owner, file);
        assert!(verified(&sums, &signature, &key, "0.3.0", "other.tar.gz", file).is_err());
        // 0.3.0's real, signed list served as 0.3.5's: a downgrade.
        assert!(verified(&sums, &signature, &key, "0.3.5", name, file).is_err());
    }

    #[test]
    fn the_release_key_is_read_from_its_file() {
        assert_eq!(release_key("# not made yet\n"), None);
        assert_eq!(
            release_key(&format!("# key\n{}\n", "ab".repeat(32))),
            Some([0xab; 32])
        );
        assert_eq!(release_key("abcd\n"), None);
        assert_eq!(release_key(&"zz".repeat(32)), None);
    }

    #[test]
    fn the_tag_is_read_from_githubs_answer() {
        let json = r#"{"url":"x","tag_name": "v0.2.2","name":"Mickey v0.2.2"}"#;
        assert_eq!(tag_name(json), Some("v0.2.2"));
        assert_eq!(tag_name(r#"{"message":"Not Found"}"#), None);
        assert_eq!(tag_name(r#"{"tag_name":5}"#), None);
        assert_eq!(tag_name(r#"{"tag_name":"v1"#), None);
    }

    #[test]
    fn only_a_newer_version_waits() {
        let updates = Updates::default();
        updates.heard("0.0.1");
        assert_eq!(updates.waiting(), None);
        updates.heard("v99.0.0");
        updates.heard("98.0.0");
        assert_eq!(updates.waiting().as_deref(), Some("99.0.0"));
    }
}
