//! Where files copied on another PC land on this one (PRD 0011), so the
//! clipboard can point at them: a fresh folder per transfer under Mickey's
//! own cache folder. Only the last few transfers are kept.
//!
//! The names come from the network: a name that could leave the folder
//! (`..`, a slash, a drive letter) is refused, never "cleaned up".

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Transfers kept; older folders are removed.
const KEEP: usize = 5;

/// `name` if it is a plain file name, safe to join to a folder.
pub fn safe_name(name: &str) -> Option<&str> {
    let bad = name.is_empty()
        || name.len() > 255
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':' | '\0') || c.is_control());
    (!bad).then_some(name)
}

/// `~/.cache/mickey/received` on Linux, `%LOCALAPPDATA%\mickey\received`
/// on Windows.
fn base() -> Result<PathBuf, String> {
    let cache = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".cache")))
    };
    cache
        .map(|dir| dir.join("mickey").join("received"))
        .ok_or_else(|| "no folder to put received files in".to_string())
}

/// Writes `files` (name, bytes) into a new folder and returns their paths.
pub fn save(files: Vec<(String, Vec<u8>)>) -> Result<Vec<PathBuf>, String> {
    save_in(&base()?, files)
}

fn save_in(base: &Path, files: Vec<(String, Vec<u8>)>) -> Result<Vec<PathBuf>, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let folder = base.join(stamp.to_string());
    std::fs::create_dir_all(&folder)
        .map_err(|err| format!("cannot create {}: {err}", folder.display()))?;
    let mut paths = Vec::new();
    for (name, bytes) in files {
        let name = safe_name(&name)
            .ok_or_else(|| format!("refused a file name from the network: {name:?}"))?;
        let path = folder.join(name);
        std::fs::write(&path, bytes)
            .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
        paths.push(path);
    }
    prune(base);
    Ok(paths)
}

/// Removes all but the newest `KEEP` transfer folders.
fn prune(base: &Path) {
    let Ok(read) = std::fs::read_dir(base) else {
        return;
    };
    let mut folders: Vec<(u128, PathBuf)> = read
        .flatten()
        .filter_map(|entry| Some((entry.file_name().to_str()?.parse().ok()?, entry.path())))
        .collect();
    folders.sort();
    let extra = folders.len().saturating_sub(KEEP);
    for (_, folder) in folders.into_iter().take(extra) {
        let _ = std::fs::remove_dir_all(folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_names_are_safe() {
        assert_eq!(safe_name("report.pdf"), Some("report.pdf"));
        assert_eq!(safe_name("foto de férias.png"), Some("foto de férias.png"));
        for bad in [
            "",
            ".",
            "..",
            "../etc/passwd",
            "a/b",
            "a\\b",
            "C:evil",
            "x\0y",
            "tab\there",
        ] {
            assert_eq!(safe_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn files_land_in_a_fresh_folder_and_old_ones_go() {
        let base = std::env::temp_dir().join(format!("mickey-received-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let paths = save_in(&base, vec![("a.txt".into(), b"hello".to_vec())]).unwrap();
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"hello");
        assert!(paths[0].starts_with(&base));
        assert!(save_in(&base, vec![("../x".into(), vec![])]).is_err());
        for _ in 0..KEEP + 2 {
            save_in(&base, vec![("b".into(), vec![])]).unwrap();
        }
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), KEEP);
        std::fs::remove_dir_all(&base).unwrap();
    }
}
