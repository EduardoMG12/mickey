//! Mickey's versions (PRD 0013): `0.2.1`, the numbers of `Cargo.toml`.
//! Compared as numbers, so 0.2.10 is newer than 0.2.9, which comparing
//! the text would get wrong.

/// This build's version, the same in every crate of the workspace.
pub const THIS: &str = env!("CARGO_PKG_VERSION");

/// `"0.2.1"` (or `"v0.2.1"`, a release tag) → (0, 2, 1). A pre-release
/// such as `0.3.0-rc1` is not a version an update offers: `None`.
pub fn parse(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim().strip_prefix('v').unwrap_or(text.trim());
    let mut parts = text.split('.');
    let version = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(version)
}

/// Whether `other` is a newer version than `than`; anything that is not a
/// version never is.
pub fn is_newer(other: &str, than: &str) -> bool {
    match (parse(other), parse(than)) {
        (Some(other), Some(than)) => other > than,
        _ => false,
    }
}

/// `text` if it is a version, written as Mickey writes one: `0.3.0`, never
/// a release tag's `v0.3.0` (a download URL adds its own `v`). A peer's
/// word is not trusted, so anything else (too long, odd characters) is
/// dropped.
pub fn checked(text: &str) -> Option<String> {
    let (major, minor, patch) = parse(text)?;
    (text.len() <= 24).then(|| format!("{major}.{minor}.{patch}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.2.10", "0.2.9"));
        assert!(is_newer("v0.3.0", "0.2.1"));
        assert!(!is_newer("0.2.1", "0.2.1"));
        assert!(!is_newer("0.2.0", "0.2.1"));
    }

    #[test]
    fn what_is_not_a_version_is_never_newer() {
        for odd in ["", "1", "1.2", "1.2.3.4", "0.3.0-rc1", "x.y.z"] {
            assert!(!is_newer(odd, "0.0.1"), "{odd:?}");
        }
        assert_eq!(checked("0.2.1"), Some("0.2.1".into()));
        // GitHub's tag, as the update check reads it (`v0.3.0` once made
        // the download URL `.../download/vv0.3.0`).
        assert_eq!(checked("v0.3.0"), Some("0.3.0".into()));
        assert_eq!(checked("0.2.1\u{202e}"), None);
        assert_eq!(checked("99999999999999999999.0.0"), None);
    }
}
