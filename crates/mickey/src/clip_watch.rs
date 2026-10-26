//! Noticing every copy on this PC (PRD 0011), so the history can keep it,
//! without reading the clipboard over and over:
//!
//! - Wayland (Hyprland, Sway, KDE): the compositor tells data-control
//!   clients about every new selection;
//! - Windows: a counter the system bumps on every change, cheap to read;
//! - elsewhere (X11, GNOME): the text is compared once a second.
//!
//! Each change is a `()` on the channel; the caller reads the clipboard.

use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

/// Starts watching. Returns how, for the log.
pub fn start(
    changed: Sender<()>,
    text: impl Fn() -> Option<String> + Send + 'static,
) -> &'static str {
    #[cfg(target_os = "linux")]
    if wayland::start(changed.clone()).is_ok() {
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
    use std::sync::mpsc::Sender;
    use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
    use wayland_client::{Connection, Dispatch, QueueHandle, event_created_child};
    use wayland_protocols_wlr::data_control::v1::client::{
        zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1},
        zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
        zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
    };

    struct Watch {
        seat: Option<WlSeat>,
        manager: Option<ZwlrDataControlManagerV1>,
        /// The current selection's offer; the old one is destroyed when a
        /// new copy replaces it, or offers would pile up.
        offer: Option<ZwlrDataControlOfferV1>,
        changed: Sender<()>,
    }

    pub fn start(changed: Sender<()>) -> Result<(), String> {
        let connection = Connection::connect_to_env().map_err(|err| err.to_string())?;
        let mut queue = connection.new_event_queue();
        let handle = queue.handle();
        connection.display().get_registry(&handle, ());
        let mut watch = Watch {
            seat: None,
            manager: None,
            offer: None,
            changed,
        };
        queue.roundtrip(&mut watch).map_err(|err| err.to_string())?;
        let (Some(seat), Some(manager)) = (&watch.seat, &watch.manager) else {
            return Err("the compositor has no data-control".into());
        };
        manager.get_data_device(seat, &handle, ());
        crate::threads::spawn("clip watch", move || {
            while queue.blocking_dispatch(&mut watch).is_ok() {}
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
                if let Some(old) = std::mem::replace(&mut watch.offer, id) {
                    old.destroy();
                }
                let _ = watch.changed.send(());
            }
        }

        event_created_child!(Watch, ZwlrDataControlDeviceV1, [
            zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ZwlrDataControlOfferV1, ()),
        ]);
    }

    /// Nothing to do with these objects' own events.
    macro_rules! ignore {
        ($($interface:ty),*) => {$(
            impl Dispatch<$interface, ()> for Watch {
                fn event(_: &mut Watch, _: &$interface, _: <$interface as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Watch>) {}
            }
        )*};
    }
    ignore!(WlSeat, ZwlrDataControlManagerV1, ZwlrDataControlOfferV1);
}
