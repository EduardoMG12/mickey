//! This PC's monitors (PRD 0005): where each one sits and its logical size,
//! the space the cursor moves in. Windows asks the system; on Linux,
//! Hyprland or Sway over their IPC sockets, the X server in an X11
//! session, and the RemoteDesktop portal elsewhere (GNOME and KDE on
//! Wayland).

use crate::threads::LockAnyway;
pub use mickey_config::peers::Monitor;

pub fn this_pc() -> Result<Vec<Monitor>, String> {
    #[cfg(windows)]
    return crate::platform::windows::monitors()
        .map_err(|err| format!("cannot list monitors: {err}"));
    #[cfg(target_os = "macos")]
    return crate::platform::macos::monitors()
        .map_err(|err| format!("cannot list monitors: {err}"));
    #[cfg(target_os = "linux")]
    {
        use crate::desktop::Session;
        match crate::desktop::session() {
            Session::Hyprland => return crate::desktop::hyprland::monitors(),
            Session::X11 => return crate::desktop::x11::monitors(),
            Session::Sway => return crate::desktop::sway::monitors(),
            Session::Portal => {}
        }
        let monitors = crate::desktop::portal::get().map(crate::desktop::portal::Portal::monitors);
        if let Some(monitors) = monitors.filter(|monitors| !monitors.is_empty()) {
            return Ok(monitors);
        }
        // GNOME and KDE always have the RemoteDesktop portal, whose monitors
        // match libei's regions; with none yet (the person has not answered
        // its dialog), wait for it rather than take another layout.
        if portal_desktop() {
            return Err("the RemoteDesktop portal named no monitors yet".to_string());
        }
        // No RemoteDesktop portal (COSMIC, wlroots desktops other than Sway):
        // Wayland's own xdg-output names them, in the same logical layout.
        match crate::desktop::outputs::outputs() {
            Ok(named) if !named.is_empty() => {
                Ok(named.into_iter().map(|(_, monitor)| monitor).collect())
            }
            Ok(_) => Err(
                "no Hyprland, and neither the RemoteDesktop portal nor xdg-output named a monitor"
                    .to_string(),
            ),
            Err(err) => Err(format!(
                "no Hyprland, the RemoteDesktop portal named no monitors, and xdg-output: {err}"
            )),
        }
    }
}

/// Whether this is GNOME or KDE, whose RemoteDesktop portal names the
/// monitors (`XDG_CURRENT_DESKTOP` lists the desktop, e.g. `ubuntu:GNOME`).
#[cfg(target_os = "linux")]
pub fn portal_desktop() -> bool {
    is_portal_desktop(&std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default())
}

#[cfg(any(target_os = "linux", test))]
fn is_portal_desktop(desktops: &str) -> bool {
    desktops
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("GNOME") || desktop.eq_ignore_ascii_case("KDE"))
}

/// Reads the monitors again every `every` on a thread of its own, and sends
/// them when they differ from the last ones: moving a monitor in the
/// display settings moves its edges too, without restarting Mickey. Off
/// the main loop because on Hyprland one read is a round trip to the
/// compositor (#187). A failed read changes nothing. Once the receiver is
/// dropped, the thread ends at the next change.
pub fn watch(
    mut last: Vec<Monitor>,
    every: std::time::Duration,
    read: fn() -> Result<Vec<Monitor>, String>,
) -> std::sync::mpsc::Receiver<Vec<Monitor>> {
    let (changed, changes) = std::sync::mpsc::channel();
    crate::threads::spawn("monitor watch", move || {
        loop {
            std::thread::sleep(every);
            let Ok(found) = read() else { continue };
            if found == last {
                continue;
            }
            if changed.send(found.clone()).is_err() {
                break;
            }
            last = found;
        }
    });
    changes
}

/// Reads the monitors every `every` on a thread of its own until a read
/// names at least one, sends them once and stops. For GNOME and KDE, where
/// the first read waits for the person to answer the RemoteDesktop dialog:
/// the source's main loop must keep pinging meanwhile, or the other PCs
/// let it go after 10 s of silence (#457).
#[cfg(target_os = "linux")]
pub fn first_found(
    every: std::time::Duration,
    read: fn() -> Result<Vec<Monitor>, String>,
) -> std::sync::mpsc::Receiver<Vec<Monitor>> {
    let (found, first) = std::sync::mpsc::channel();
    crate::threads::spawn("monitors at last", move || {
        loop {
            if let Ok(monitors) = read()
                && !monitors.is_empty()
            {
                let _ = found.send(monitors);
                return;
            }
            std::thread::sleep(every);
        }
    });
    first
}

/// Whether this PC's monitors can be read again while Mickey runs: on
/// Windows, Hyprland, Sway and X11. Not on GNOME and KDE: each read goes through the
/// RemoteDesktop portal, which reports them once per session, and reading
/// every second could ask for permission again.
pub fn follows_changes() -> bool {
    #[cfg(target_os = "linux")]
    return crate::desktop::session() != crate::desktop::Session::Portal;
    #[cfg(any(windows, target_os = "macos"))]
    return true;
}

/// This PC's monitors as last read, shared by every source this PC serves.
pub type Latest = std::sync::Arc<std::sync::Mutex<Vec<Monitor>>>;

/// One `watch` for the whole target, not one per connection: a source
/// that reconnects would leave a thread reading the monitors behind.
/// `None` where they do not follow a change (`follows_changes`).
pub fn keep_latest(every: std::time::Duration) -> Option<Latest> {
    if !follows_changes() {
        return None;
    }
    let latest = Latest::default();
    let first = this_pc().unwrap_or_default();
    *latest.lock_anyway() = first.clone();
    let changes = watch(first, every, this_pc);
    let kept = latest.clone();
    crate::threads::spawn("monitor share", move || {
        for found in changes {
            *kept.lock_anyway() = found;
        }
    });
    Some(latest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn gnome_and_kde_wait_for_their_portal_others_ask_wayland() {
        assert!(is_portal_desktop("GNOME"));
        assert!(is_portal_desktop("ubuntu:GNOME"));
        assert!(is_portal_desktop("KDE"));
        assert!(!is_portal_desktop("COSMIC"));
        assert!(!is_portal_desktop("river"));
        assert!(!is_portal_desktop(""));
    }
    use std::time::Duration;

    /// The owner's vertical monitor before and after it was moved down
    /// 845 pixels in the display settings (#365).
    const BEFORE: Monitor = Monitor {
        x: 2560,
        y: -845,
        width: 1080,
        height: 1920,
    };
    const AFTER: Monitor = Monitor { y: 0, ..BEFORE };

    static READS: AtomicUsize = AtomicUsize::new(0);

    /// Unchanged, then a failed read, then moved, then moved still.
    fn moved_once() -> Result<Vec<Monitor>, String> {
        match READS.fetch_add(1, Ordering::SeqCst) {
            0 => Ok(vec![BEFORE]),
            1 => Err("the compositor did not answer".into()),
            _ => Ok(vec![AFTER]),
        }
    }

    #[cfg(target_os = "linux")]
    static SLOW_READS: AtomicUsize = AtomicUsize::new(0);

    /// The portal not up yet, then up with no monitors, then with one.
    #[cfg(target_os = "linux")]
    fn portal_answers_late() -> Result<Vec<Monitor>, String> {
        match SLOW_READS.fetch_add(1, Ordering::SeqCst) {
            0 => Err("the RemoteDesktop portal named no monitors yet".into()),
            1 => Ok(Vec::new()),
            _ => Ok(vec![BEFORE]),
        }
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_first_monitors_found_are_sent_once_then_reading_stops() {
        let first = first_found(Duration::from_millis(1), portal_answers_late);
        assert_eq!(first.recv_timeout(Duration::from_secs(1)), Ok(vec![BEFORE]));
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            SLOW_READS.load(Ordering::SeqCst),
            3,
            "no read after the first found"
        );
        assert!(first.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn a_moved_monitor_is_sent_once() {
        let changes = watch(vec![BEFORE], Duration::from_millis(1), moved_once);
        let wait = Duration::from_secs(1);
        assert_eq!(changes.recv_timeout(wait), Ok(vec![AFTER]));
        // Still moved, read after read: nothing more to say.
        assert!(changes.recv_timeout(Duration::from_millis(50)).is_err());
        assert!(READS.load(Ordering::SeqCst) > 3, "it kept reading");
    }
}
