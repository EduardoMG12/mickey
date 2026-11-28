//! What was copied, as an offer: walking the copied files and folders on
//! the PC where they were copied.
//!
//! Links are never followed (a link to `~/.ssh` would otherwise send the
//! keys), and anything the other PC could not write the same way is left
//! out and listed, never sent half right.

use crate::transfer::path;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The most entries (files and folders) one offer may hold. ponytail: a
/// folder with more (a big `node_modules`) is refused; the upgrade path is
/// sending the offer in pages.
pub const MAX_ENTRIES: usize = 100_000;
/// The most left-out names an offer carries: enough to say what is
/// missing, never a second listing.
pub const MAX_LEFT_OUT: usize = 100;

/// What crosses to the other PC: the copied tree, without content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// Names this offer: random, and known only to the PC it was carried
    /// to, which sends it back to fetch (`Offers`).
    pub id: u64,
    /// Every folder comes before what is inside it.
    pub entries: Vec<Entry>,
    /// What was copied but does not cross, and why (the first
    /// `MAX_LEFT_OUT`), so the receiving PC can say so.
    pub left_out: Vec<Skipped>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Relative, `/` between folders, every part a safe name.
    pub path: String,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File { size: u64 },
}

/// What stays on this PC: where each entry is, and what was left out.
#[derive(Debug, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// `local[i]` is where `entries[i]` is on this PC, and when it was last
    /// changed: a file that changes while it is sent is sent again.
    pub local: Vec<(PathBuf, Option<SystemTime>)>,
    pub skipped: Vec<Skipped>,
}

/// Something copied that does not cross, and why: shown to the owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: String,
    pub why: Why,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// A symlink, or a junction on Windows.
    Link,
    /// A name Windows refuses, or a path too deep or too long.
    BadName,
    /// Only differs in case from one already listed: the same file on
    /// Windows.
    SameNameOtherCase,
    /// Not a file or a folder (a socket, a device).
    NotAFile,
    /// Could not be read.
    Unreadable,
}

impl Listing {
    /// The file at `index` of the offer: where it is here, its size and
    /// when it was last changed, as listed. `None` for a folder or an index
    /// past the end.
    pub fn file(&self, index: u32) -> Option<(&Path, u64, Option<SystemTime>)> {
        let index = usize::try_from(index).ok()?;
        let Kind::File { size } = self.entries.get(index)?.kind else {
            return None;
        };
        let (path, modified) = &self.local[index];
        Some((path, size, *modified))
    }

    /// The offer for this listing, under `id` (`Offers` picks it).
    pub fn offer(&self, id: u64) -> Offer {
        Offer {
            id,
            entries: self.entries.clone(),
            left_out: self.skipped.iter().take(MAX_LEFT_OUT).cloned().collect(),
        }
    }
}

/// Walks the copied `roots` (files and folders, as the clipboard lists
/// them). Too many entries is an error: the offer would be refused.
pub fn list(roots: &[PathBuf]) -> Result<Listing, String> {
    let mut listing = Listing::default();
    let mut seen = HashSet::new();
    // Last in, first out: pushed in reverse to come out in order.
    let mut pending: Vec<(PathBuf, Result<String, String>)> = roots
        .iter()
        .rev()
        .map(|root| (root.clone(), name_of(root)))
        .collect();
    while let Some((local, path)) = pending.pop() {
        if listing.entries.len() == MAX_ENTRIES {
            return Err(format!("more than {MAX_ENTRIES} files and folders"));
        }
        let path = match path {
            Ok(path) if path::checked(&path).is_ok() => path,
            Ok(shown) | Err(shown) => {
                listing.skipped.push(Skipped {
                    path: shown,
                    why: Why::BadName,
                });
                continue;
            }
        };
        let skip = |why| Skipped {
            path: path.clone(),
            why,
        };
        let Ok(metadata) = std::fs::symlink_metadata(&local) else {
            listing.skipped.push(skip(Why::Unreadable));
            continue;
        };
        // On Windows, junctions count as symlinks here too.
        let kind = if metadata.is_symlink() {
            listing.skipped.push(skip(Why::Link));
            continue;
        } else if metadata.is_dir() {
            Kind::Folder
        } else if metadata.is_file() {
            Kind::File {
                size: metadata.len(),
            }
        } else {
            listing.skipped.push(skip(Why::NotAFile));
            continue;
        };
        if !seen.insert(path.to_lowercase()) {
            listing.skipped.push(skip(Why::SameNameOtherCase));
            continue;
        }
        if kind == Kind::Folder {
            let Some(names) = children(&local) else {
                listing.skipped.push(skip(Why::Unreadable));
                continue;
            };
            let below = names.into_iter().rev();
            pending.extend(below.map(|(child, name)| {
                let under = |name| format!("{path}/{name}");
                (child, name.map(under).map_err(under))
            }));
        }
        listing.local.push((local, metadata.modified().ok()));
        listing.entries.push(Entry { path, kind });
    }
    Ok(listing)
}

/// A folder's entries, sorted by name so offers come out the same way
/// every time.
fn children(folder: &Path) -> Option<Vec<(PathBuf, Result<String, String>)>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    Some(
        paths
            .into_iter()
            .map(|path| {
                let name = name_of(&path);
                (path, name)
            })
            .collect(),
    )
}

/// The last part of `path`, or, if it is not UTF-8 (possible on Linux),
/// the error with a readable version to show the owner: such a name
/// cannot be written the same way on the other PC.
fn name_of(path: &Path) -> Result<String, String> {
    let name = path.file_name().unwrap_or_default();
    name.to_str()
        .map(str::to_string)
        .ok_or_else(|| name.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::Scratch;

    fn paths(listing: &Listing) -> Vec<&str> {
        listing
            .entries
            .iter()
            .map(|entry| entry.path.as_str())
            .collect()
    }

    #[test]
    fn a_tree_is_listed_folders_first_in_name_order() {
        let scratch = Scratch::new("tree");
        scratch.file("fotos/b.jpg", b"bb");
        scratch.file("fotos/a.jpg", b"a");
        scratch.file("fotos/2026/c.jpg", b"");
        std::fs::create_dir(scratch.0.join("fotos/empty")).unwrap();
        scratch.file("notes.txt", b"hi");
        let roots = [scratch.0.join("fotos"), scratch.0.join("notes.txt")];
        let listing = list(&roots).unwrap();
        assert_eq!(
            paths(&listing),
            [
                "fotos",
                "fotos/2026",
                "fotos/2026/c.jpg",
                "fotos/a.jpg",
                "fotos/b.jpg",
                "fotos/empty",
                "notes.txt"
            ]
        );
        assert_eq!(listing.entries[4].kind, Kind::File { size: 2 });
        assert_eq!(listing.entries[5].kind, Kind::Folder);
        assert_eq!(listing.local[6].0, scratch.0.join("notes.txt"));
        assert!(listing.skipped.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_listed_as_skipped_never_followed() {
        let scratch = Scratch::new("links");
        scratch.file("secret/id_rsa", b"key");
        scratch.file("shared/ok.txt", b"ok");
        std::os::unix::fs::symlink(scratch.0.join("secret"), scratch.0.join("shared/keys"))
            .unwrap();
        let listing = list(&[scratch.0.join("shared")]).unwrap();
        assert_eq!(paths(&listing), ["shared", "shared/ok.txt"]);
        assert_eq!(
            listing.skipped,
            [Skipped {
                path: "shared/keys".into(),
                why: Why::Link
            }]
        );
        // The offer says so, for the receiving PC to show.
        assert_eq!(listing.offer(1).left_out, listing.skipped);
    }

    #[test]
    fn an_offer_carries_only_the_first_left_out_names() {
        let listing = Listing {
            skipped: (0..MAX_LEFT_OUT + 5)
                .map(|n| Skipped {
                    path: format!("{n}.lnk"),
                    why: Why::Link,
                })
                .collect(),
            ..Default::default()
        };
        assert_eq!(listing.offer(1).left_out.len(), MAX_LEFT_OUT);
    }

    // Needs a case-sensitive filesystem (APFS on macOS folds Foto.jpg and foto.jpg).
    #[cfg(target_os = "linux")]
    #[test]
    fn names_windows_refuses_and_case_twins_are_skipped() {
        let scratch = Scratch::new("names");
        scratch.file("docs/a:b.txt", b"");
        scratch.file("docs/Foto.jpg", b"");
        scratch.file("docs/foto.jpg", b"");
        let listing = list(&[scratch.0.join("docs")]).unwrap();
        assert_eq!(paths(&listing), ["docs", "docs/Foto.jpg"]);
        let why: Vec<Why> = listing.skipped.iter().map(|skipped| skipped.why).collect();
        // In name order: `Foto.jpg`, `a:b.txt`, `foto.jpg`.
        assert_eq!(why, [Why::BadName, Why::SameNameOtherCase]);
    }

    // Needs a filesystem that permits non-UTF-8 filenames (APFS rejects them with EILSEQ).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_name_that_is_not_utf8_is_skipped() {
        use std::os::unix::ffi::OsStrExt;
        let scratch = Scratch::new("utf8");
        scratch.file("docs/ok.txt", b"");
        let odd = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
        std::fs::write(scratch.0.join("docs").join(odd), b"").unwrap();
        let listing = list(&[scratch.0.join("docs")]).unwrap();
        assert_eq!(paths(&listing), ["docs", "docs/ok.txt"]);
        assert_eq!(
            listing.skipped,
            [Skipped {
                path: "docs/caf\u{FFFD}.txt".into(),
                why: Why::BadName
            }]
        );
    }
}
