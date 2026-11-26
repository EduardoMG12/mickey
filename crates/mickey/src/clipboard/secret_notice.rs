//! When a copy marked secret stays on this PC (#425), a small window says
//! so: otherwise the owner crosses, pastes, and finds nothing there, with
//! no idea why. At most once while Mickey runs, and never again once the
//! owner ticks "Don't show again", which the window keeps as a file in
//! Mickey's data folder.

use std::sync::atomic::{AtomicBool, Ordering};

/// The file the window writes when the owner ticks "Don't show again".
const FILE: &str = "secret-notice-off";

/// Whether the window was shown since Mickey started.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Opens `mickey-ui secret-kept <file>` unless it was shown already while
/// Mickey runs, or the owner said not to show it again. Failing to open is
/// only logged: the notice is a nicety, the secret stays here either way.
pub fn maybe_show() {
    let Ok(data_dir) = crate::net::identity::data_dir() else {
        return;
    };
    let off = data_dir.join(FILE);
    if !should_show(off.exists(), &SHOWN) {
        return;
    }
    if let Err(err) = crate::window::open(&["secret-kept".as_ref(), off.as_os_str()]) {
        eprintln!("mickey: cannot say a secret stayed here: {err}");
    }
}

/// Whether to show it: not turned off, and not shown yet (`shown` is then
/// set, so the next secret while Mickey runs shows nothing).
fn should_show(turned_off: bool, shown: &AtomicBool) -> bool {
    !turned_off && !shown.swap(true, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_notice_shows_once_a_run_until_turned_off() {
        let shown = AtomicBool::new(false);
        assert!(should_show(false, &shown), "the first secret");
        assert!(!should_show(false, &shown), "not again while it runs");
        assert!(!should_show(true, &AtomicBool::new(false)), "turned off");
    }
}
