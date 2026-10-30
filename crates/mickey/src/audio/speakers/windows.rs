//! Windows: WASAPI's loopback capture ignores the output's mute (PRD 0012
//! Discoveries), so Mickey just mutes the default output while it sends.
//! A file, `muted-by-mickey` in Mickey's data folder, says Mickey did it:
//! `clean_up` unmutes only then, so an output the owner had muted stays
//! muted, and a Mickey that ended while sending is undone at the next start.

use crate::platform::output;
use std::path::PathBuf;

fn marker() -> Result<PathBuf, String> {
    Ok(crate::net::identity::data_dir()?.join("muted-by-mickey"))
}

/// The sound taken off the speakers; given back when dropped.
pub struct Diverted;

/// Mutes the default output, unless the owner already had.
pub fn divert() -> Result<Diverted, String> {
    clean_up()?;
    if !output::muted().map_err(|err| format!("cannot read the sound output: {err}"))? {
        let marker = marker()?;
        if let Some(dir) = marker.parent() {
            std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
        }
        // Written first: if Mickey ends right after muting, the next start
        // still knows to unmute.
        std::fs::write(&marker, "")
            .map_err(|err| format!("cannot write {}: {err}", marker.display()))?;
        output::set_muted(true).map_err(|err| format!("cannot mute the sound output: {err}"))?;
    }
    Ok(Diverted)
}

impl Drop for Diverted {
    fn drop(&mut self) {
        if let Err(err) = clean_up() {
            eprintln!("mickey: cannot give the sound back to the speakers: {err}");
        }
    }
}

/// Unmutes the default output if Mickey muted it.
pub fn clean_up() -> Result<(), String> {
    let marker = marker()?;
    if marker.exists() {
        output::set_muted(false).map_err(|err| format!("cannot unmute the sound output: {err}"))?;
        std::fs::remove_file(&marker)
            .map_err(|err| format!("cannot remove {}: {err}", marker.display()))?;
    }
    Ok(())
}
