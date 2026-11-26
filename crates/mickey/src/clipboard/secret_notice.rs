//! The first time a copy marked secret stays on this PC (#425), a small
//! window says so, once: otherwise the owner crosses, pastes, and finds
//! nothing there, with no idea why. Mickey keeps a file in its data folder
//! once the window was shown; deleting it shows it again.

use std::path::Path;

const FILE: &str = "secret-notice-shown";

/// Opens `mickey-ui secret-kept` if it was never shown on this PC, and
/// remembers it. Failing to read, write or open is only logged: the
/// notice is a nicety, the secret stays here either way.
pub fn once() {
    let Ok(data_dir) = crate::net::identity::data_dir() else {
        return;
    };
    if !first_time(&data_dir) {
        return;
    }
    if let Err(err) = crate::window::open(&["secret-kept"]) {
        eprintln!("mickey: cannot say a secret stayed here: {err}");
    }
}

/// Whether the notice was never shown in `data_dir`; marks it shown.
fn first_time(data_dir: &Path) -> bool {
    let path = data_dir.join(FILE);
    if path.exists() {
        return false;
    }
    if let Err(err) = std::fs::create_dir_all(data_dir).and_then(|()| std::fs::write(&path, "")) {
        eprintln!("mickey: cannot remember the secret notice was shown: {err}");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::Scratch;

    #[test]
    fn the_notice_shows_once_per_pc() {
        let scratch = Scratch::new("secret-notice");
        assert!(first_time(&scratch.0), "never shown");
        assert!(!first_time(&scratch.0), "shown already");
    }
}
