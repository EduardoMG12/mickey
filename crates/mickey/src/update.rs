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
    let mut curl = Command::new("curl");
    curl.args(["--silent", "--show-error", "--fail", "--location"])
        .args(["--max-time", "20", "--proto", "=https"])
        .args(["--user-agent", &format!("mickey/{}", version::THIS)])
        .args(["--header", "Accept: application/vnd.github+json"])
        .arg(LATEST);
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
    let json = String::from_utf8_lossy(&answer.stdout);
    let tag = tag_name(&json).ok_or("GitHub's answer names no release")?;
    version::checked(tag).ok_or_else(|| format!("the latest release, {tag:?}, is not a version"))
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
