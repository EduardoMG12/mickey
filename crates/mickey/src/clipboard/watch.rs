//! Noticing every copy on this PC (PRD 0011), so the history can keep it,
//! without reading the clipboard over and over:
//!
//! - Wayland (Hyprland, Sway, KDE): the compositor tells data-control
//!   clients about every new selection;
//! - Windows: a counter the system bumps on every change, cheap to read;
//! - elsewhere (X11, GNOME): the text is compared once a second.
//!
//! Each change is a `()` on the channel; the caller reads the clipboard.
//!
//! On Wayland it also notes whether the copy carries the secret mark
//! password managers add there (#425): `x-kde-passwordManagerHint` among
//! the types it offers (KeePassXC, KDE Wallet). `arboard` never shows
//! those types, so `copied_secret` is the only place it is known.
//!
//! One Wayland connection does both: it stores the mark, then tells every
//! watcher about the copy. With two connections (one for the mark, one
//! for the history) nothing ordered them, and the history could read the
//! previous copy's mark and keep a password.

use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

/// Starts watching. Returns how, for the log.
pub fn start(
    changed: Sender<()>,
    text: impl Fn() -> Option<String> + Send + 'static,
) -> &'static str {
    #[cfg(target_os = "linux")]
    if wayland::start().is_ok() {
        wayland::listen(changed);
        return "Wayland selection events";
    }
    #[cfg(windows)]
    {
        let _ = text;
        windows_counter(changed);
        "the Windows clipboard counter"
    }
    #[cfg(not(windows))]
    {
        poll_text(changed, text);
        "the text, once a second"
    }
}

/// Whether the current copy carries the secret mark (#425), on Wayland
/// with data-control (Hyprland, Sway, KDE); `false` elsewhere (X11,
/// GNOME), where Mickey cannot see the mark.
///
/// It comes from the watcher's connection, not from the one that reads
/// the copy: a copy read in the instant before the watcher hears of it
/// reads as unmarked. That is the time to dispatch one event, far less
/// than the time from a copy to a crossing; `selections` lets a reader
/// notice a copy that arrived while it read.
#[cfg(target_os = "linux")]
pub fn copied_secret() -> bool {
    wayland::SECRET.load(std::sync::atomic::Ordering::SeqCst)
}

/// How many copies the watcher has heard of; 0 when it does not run.
#[cfg(target_os = "linux")]
pub fn selections() -> u32 {
    wayland::SELECTIONS.load(std::sync::atomic::Ordering::SeqCst)
}

/// Starts following the copies, to know whether each is marked secret.
/// Says so when it cannot: then marks are not seen, and a password copied
/// here can cross.
#[cfg(target_os = "linux")]
pub fn follow_secret_marks() {
    if let Err(err) = wayland::start() {
        println!(
            "mickey: copies marked secret cannot be told apart on this desktop ({err}); they are shared like any copy"
        );
    }
}

#[cfg(windows)]
fn windows_counter(changed: Sender<()>) {
    crate::threads::spawn("clip watch", move || {
        let mut last = crate::platform::windows::clipboard_sequence();
        loop {
            thread::sleep(Duration::from_millis(250));
            let now = crate::platform::windows::clipboard_sequence();
            if now != last {
                last = now;
                if changed.send(()).is_err() {
                    return;
                }
            }
        }
    });
}

#[cfg(not(windows))]
fn poll_text(changed: Sender<()>, text: impl Fn() -> Option<String> + Send + 'static) {
    crate::threads::spawn("clip watch", move || {
        let mut last = text();
        loop {
            thread::sleep(Duration::from_secs(1));
            let now = text();
            if now != last {
                last = now;
                if changed.send(()).is_err() {
                    return;
                }
            }
        }
    });
}

#[cfg(target_os = "linux")]
mod wayland {
    use crate::threads::LockAnyway;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::mpsc::Sender;
    use std::sync::{Mutex, OnceLock};
    use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
    use wayland_protocols_wlr::data_control::v1::client::{
        zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
        zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
        zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1},
    };

    /// The type password managers offer with a password they copy.
    const SECRET_HINT: &str = "x-kde-passwordManagerHint";

    /// Whether the current selection offered `SECRET_HINT`.
    pub static SECRET: AtomicBool = AtomicBool::new(false);

    /// Bumped after `SECRET` on every new selection.
    pub static SELECTIONS: AtomicU32 = AtomicU32::new(0);

    /// Told about every copy, after `SECRET` holds its mark.
    static LISTENERS: Mutex<Vec<Sender<()>>> = Mutex::new(Vec::new());

    /// The one connection's outcome: it is opened on the first `start`.
    static STARTED: OnceLock<Result<(), String>> = OnceLock::new();

    struct Watch {
        seat: Option<WlSeat>,
        manager: Option<ZwlrDataControlManagerV1>,
        /// The current selection's offer; the old one is destroyed when a
        /// new copy replaces it, or offers would pile up.
        offer: Option<ZwlrDataControlOfferV1>,
        /// The compositor ended the data-control device (`finished`).
        finished: bool,
    }

    /// Follows the selection, on one connection however many ask.
    pub fn start() -> Result<(), String> {
        STARTED.get_or_init(connect).clone()
    }

    /// `changed` gets a `()` for every copy from now on.
    pub fn listen(changed: Sender<()>) {
        LISTENERS.lock_anyway().push(changed);
    }

    fn connect() -> Result<(), String> {
        let connection = Connection::connect_to_env().map_err(|err| err.to_string())?;
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        let mut watch = Watch {
            seat: None,
            manager: None,
            offer: None,
            finished: false,
        };
        queue.roundtrip(&mut watch).map_err(|err| err.to_string())?;
        let (Some(seat), Some(manager)) = (&watch.seat, &watch.manager) else {
            return Err("the compositor has no data-control".into());
        };
        manager.get_data_device(seat, &handle, ());
        crate::threads::spawn("clip watch", move || {
            while !watch.finished && queue.blocking_dispatch(&mut watch).is_ok() {}
            // Whatever it last held no longer says anything.
            SECRET.store(false, Ordering::SeqCst);
            println!(
                "mickey: stopped following the copies; copies marked secret are shared like any copy from now on"
            );
        });
        Ok(())
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for Watch {
        fn event(
            watch: &mut Watch,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            handle: &QueueHandle<Watch>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                match interface.as_str() {
                    "wl_seat" if watch.seat.is_none() => {
                        watch.seat = Some(registry.bind(name, version.min(1), handle, ()));
                    }
                    "zwlr_data_control_manager_v1" => {
                        watch.manager = Some(registry.bind(name, version.min(1), handle, ()));
                    }
                    _ => {}
                }
            }
        }
    }

    impl Dispatch<ZwlrDataControlDeviceV1, ()> for Watch {
        fn event(
            watch: &mut Watch,
            _: &ZwlrDataControlDeviceV1,
            event: zwlr_data_control_device_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Watch>,
        ) {
            match event {
                zwlr_data_control_device_v1::Event::Selection { id } => {
                    // Its types came before it became the selection.
                    let marked = id
                        .as_ref()
                        .and_then(|offer| offer.data::<AtomicBool>())
                        .is_some_and(|marked| marked.load(Ordering::Relaxed));
                    // The mark first, then the news: whoever hears of the
                    // copy reads this copy's mark, never the previous one.
                    SECRET.store(marked, Ordering::SeqCst);
                    SELECTIONS.fetch_add(1, Ordering::SeqCst);
                    if let Some(old) = std::mem::replace(&mut watch.offer, id) {
                        old.destroy();
                    }
                    LISTENERS
                        .lock_anyway()
                        .retain(|changed| changed.send(()).is_ok());
                }
                zwlr_data_control_device_v1::Event::Finished => watch.finished = true,
                _ => {}
            }
        }

        // Each offer carries whether it offered `SECRET_HINT`.
        event_created_child!(Watch, ZwlrDataControlDeviceV1, [
            zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, AtomicBool::new(false)),
        ]);
    }

    impl Dispatch<ZwlrDataControlOfferV1, AtomicBool> for Watch {
        fn event(
            _: &mut Watch,
            _: &ZwlrDataControlOfferV1,
            event: zwlr_data_control_offer_v1::Event,
            marked: &AtomicBool,
            _: &Connection,
            _: &QueueHandle<Watch>,
        ) {
            // The mark's value ("secret") is not read: only password
            // managers offer this type, and reading it would take a pipe.
            if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event
                && mime_type == SECRET_HINT
            {
                marked.store(true, Ordering::Relaxed);
            }
        }
    }

    // Nothing to do with these objects' own events.
    wayland_client::delegate_noop!(Watch: ignore WlSeat);
    wayland_client::delegate_noop!(Watch: ignore ZwlrDataControlManagerV1);
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    /// Run by hand on Hyprland, Sway or KDE, after copying a password in
    /// KeePassXC: says whether the copy carries the secret mark.
    #[test]
    #[ignore = "reads this session's clipboard"]
    fn says_whether_the_copy_is_marked_secret() {
        super::follow_secret_marks();
        std::thread::sleep(std::time::Duration::from_millis(300));
        println!("marked secret: {}", super::copied_secret());
    }

    /// Run by hand on Hyprland or Sway (it replaces this session's copy,
    /// with `wl-copy`): whoever hears of a copy already reads its mark.
    /// Before #566's follow-up the history had its own connection and
    /// could read the previous copy's mark.
    #[test]
    #[ignore = "replaces this session's clipboard"]
    fn a_listener_reads_the_mark_of_the_copy_it_hears_of() {
        use std::process::Command;
        use std::time::Duration;
        super::follow_secret_marks();
        let (changed, changes) = std::sync::mpsc::channel();
        super::wayland::listen(changed);
        let copy = |args: &[&str]| {
            // The selection already there when the watcher starts is news
            // too; only this copy's counts.
            std::thread::sleep(Duration::from_millis(100));
            while changes.try_recv().is_ok() {}
            let done = Command::new("wl-copy").args(args).status().unwrap();
            assert!(done.success());
            changes.recv_timeout(Duration::from_secs(2)).unwrap();
            super::copied_secret()
        };
        for _ in 0..20 {
            assert!(copy(&[
                "--type",
                "x-kde-passwordManagerHint",
                "mickey test"
            ]));
            assert!(!copy(&["mickey test, not secret"]));
        }
    }
}
