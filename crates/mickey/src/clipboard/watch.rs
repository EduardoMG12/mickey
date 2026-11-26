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

use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

/// Starts watching. Returns how, for the log.
pub fn start(
    changed: Sender<()>,
    text: impl Fn() -> Option<String> + Send + 'static,
) -> &'static str {
    #[cfg(target_os = "linux")]
    if wayland::start(changed.clone(), false).is_ok() {
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
/// It comes from this watcher's own connection, not from the one that
/// reads the copy: a copy read in the instant before the watcher hears of
/// it reads as unmarked. That is the time to dispatch one event, far less
/// than the time from a copy to a crossing.
#[cfg(target_os = "linux")]
pub fn copied_secret() -> bool {
    wayland::SECRET.load(std::sync::atomic::Ordering::Relaxed)
}

/// Follows the copies to know whether each is marked secret: the only
/// watcher that says so, whatever else watches (Mickey's history has its
/// own), so two never disagree. Says so when it cannot: then marks are not
/// seen, and a password copied here can cross.
#[cfg(target_os = "linux")]
pub fn follow_secret_marks() {
    let (changed, _unread) = std::sync::mpsc::channel();
    if let Err(err) = wayland::start(changed, true) {
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::Sender;
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

    struct Watch {
        seat: Option<WlSeat>,
        manager: Option<ZwlrDataControlManagerV1>,
        /// The current selection's offer; the old one is destroyed when a
        /// new copy replaces it, or offers would pile up.
        offer: Option<ZwlrDataControlOfferV1>,
        changed: Sender<()>,
        /// Whether this watcher keeps `SECRET` (only one does).
        marks: bool,
    }

    /// Watches the selection; `marks` says whether this watcher keeps
    /// `SECRET` up to date.
    pub fn start(changed: Sender<()>, marks: bool) -> Result<(), String> {
        let connection = Connection::connect_to_env().map_err(|err| err.to_string())?;
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        let mut watch = Watch {
            seat: None,
            manager: None,
            offer: None,
            changed,
            marks,
        };
        queue.roundtrip(&mut watch).map_err(|err| err.to_string())?;
        let (Some(seat), Some(manager)) = (&watch.seat, &watch.manager) else {
            return Err("the compositor has no data-control".into());
        };
        manager.get_data_device(seat, &handle, ());
        crate::threads::spawn("clip watch", move || {
            while queue.blocking_dispatch(&mut watch).is_ok() {}
            if watch.marks {
                println!(
                    "mickey: stopped following the copies; copies marked secret are shared like any copy from now on"
                );
            }
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
            if let zwlr_data_control_device_v1::Event::Selection { id } = event {
                // Its types came before it became the selection.
                let marked = id
                    .as_ref()
                    .and_then(|offer| offer.data::<AtomicBool>())
                    .is_some_and(|marked| marked.load(Ordering::Relaxed));
                if watch.marks {
                    SECRET.store(marked, Ordering::Relaxed);
                }
                if let Some(old) = std::mem::replace(&mut watch.offer, id) {
                    old.destroy();
                }
                let _ = watch.changed.send(());
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
}
