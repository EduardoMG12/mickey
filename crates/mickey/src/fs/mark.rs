//! The Mark of the Web: Windows' note that a file came from elsewhere, and
//! from where (PRD 0015).
//!
//! A file from another PC says it came from Mickey (`HostUrl`), and from
//! the local network: Defender scans it all the same, but SmartScreen and
//! Office's Protected View leave the owner's own files alone. The owner may
//! have them marked as from the Internet instead (`[clipboard]
//! warn_received`), and then those warn too. Nothing on other systems.

use std::path::Path;

/// How files from one PC are marked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mark {
    /// The PC they came from, as this one names it; empty if not known.
    pub from: String,
    /// Marked as from the Internet, not the local network.
    pub internet: bool,
}

impl Mark {
    /// Files from `from`, marked as the owner chose in config.toml.
    pub fn configured(from: &str) -> Mark {
        let internet = mickey_config::Config::load()
            .ok()
            .flatten()
            .is_some_and(|config| config.clipboard.warn_received);
        Mark {
            from: from.to_string(),
            internet,
        }
    }

    /// The `Zone.Identifier` text: ZoneId 1 is the local network (intranet),
    /// 3 the Internet.
    #[cfg(any(windows, test))]
    fn text(&self) -> String {
        let zone = if self.internet { 3 } else { 1 };
        let from: String = self
            .from
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        format!("[ZoneTransfer]\r\nZoneId={zone}\r\nHostUrl=mickey://{from}\r\n")
    }

    /// Marks the file at `path`; on Windows only, where NTFS keeps the mark
    /// in a stream beside the file.
    pub fn put_on(&self, path: &Path) {
        #[cfg(windows)]
        {
            let mut stream = path.as_os_str().to_owned();
            stream.push(":Zone.Identifier");
            // Said once per file, with no name (names never reach the
            // log): a drive with no streams (FAT, exFAT) cannot keep it.
            if let Err(err) = std::fs::write(stream, self.text()) {
                eprintln!("mickey: a received file could not be marked as from another PC: {err}");
            }
        }
        #[cfg(not(windows))]
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_from_another_pc_says_where_from() {
        let local = Mark {
            from: "DESK-01".into(),
            internet: false,
        };
        assert_eq!(
            local.text(),
            "[ZoneTransfer]\r\nZoneId=1\r\nHostUrl=mickey://DESK-01\r\n"
        );
        let internet = Mark {
            from: "Notebook da Ana\r\nZoneId=0".into(),
            internet: true,
        };
        assert_eq!(
            internet.text(),
            "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=mickey://NotebookdaAnaZoneId0\r\n",
            "a name never adds a line"
        );
    }
}
