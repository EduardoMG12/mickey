//! Finding a newer Mickey (PRD 0013): asking GitHub which release is the
//! latest (`mickey update check`).
//!
//! The request goes through the system's `curl` (every Linux desktop and
//! Windows 10 1803 or newer have it), so Mickey carries no HTTP or TLS code.
//! It sends nothing about the owner: only what any HTTPS request sends.

use mickey_config::version;
use std::process::Command;

const LATEST: &str = "https://api.github.com/repos/EduardoMG12/mickey/releases/latest";

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
}
