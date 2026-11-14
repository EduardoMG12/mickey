//! The source's keyboard and mouse, per platform (PRD 0008): evdev devices
//! on Linux, low-level hooks on Windows. `mickey source` only uses this
//! interface, so the focus, hotkey and switcher work the same on both.

#[cfg(target_os = "linux")]
pub mod capture;
pub mod emit;
pub mod event;
pub mod held;
pub mod hotkey;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod keymap;
pub mod record;
#[cfg(target_os = "linux")]
pub mod touchpad;
#[cfg(target_os = "linux")]
pub mod touchpad_prefs;

use crate::input::event::InputEvent;
#[cfg(windows)]
use crate::threads::LockAnyway;
use mickey_config as config;
use std::sync::Arc;
use std::sync::mpsc::Sender;

#[cfg(target_os = "linux")]
pub use linux::Input;
#[cfg(target_os = "linux")]
pub use linux::cursor_now;
#[cfg(windows)]
pub use windows::Input;
#[cfg(windows)]
pub use windows::cursor_now;

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use crate::input::capture;
    use crate::platform::linux::{Device, Grab};
    use mickey_config::i18n;

    pub struct Input {
        devices: Vec<capture::Found>,
    }

    /// While this lives, the devices are taken from the desktop. A touchpad
    /// the owner kept local (`forward = false` in `config.toml`) is not in
    /// the list: it keeps moving this PC's own cursor.
    pub struct Grabbed<'a> {
        _grabs: Vec<Grab<'a>>,
    }

    /// Where the cursor is: a question to Hyprland or the X server,
    /// which may take a few milliseconds (#187).
    pub fn cursor_now() -> Option<(i32, i32)> {
        match crate::desktop::session() {
            crate::desktop::Session::X11 => crate::desktop::x11::cursor().ok(),
            _ => crate::desktop::hyprland::cursor().ok(),
        }
    }

    impl Input {
        /// Every keyboard, mouse and touchpad, with a clear message when we
        /// may not read them. `touchpad` is the `[touchpad]` block of
        /// `config.toml`: its `forward = false` keeps a touchpad on this PC.
        pub fn open(touchpad: &config::Touchpad) -> Result<Input, String> {
            let mut devices = capture::find_devices().map_err(|err| match err.kind() {
                std::io::ErrorKind::PermissionDenied => i18n::t("capture.permission_denied").into(),
                _ => i18n::tf("capture.cannot_list", &[("err", &err.to_string())]),
            })?;
            if devices.is_empty() {
                return Err(i18n::t("capture.no_keyboard").into());
            }
            capture::set_forward(&mut devices, touchpad);
            for found in &devices {
                let line = match found.kind {
                    capture::Kind::Keyboard => {
                        i18n::tf("messages.found_keyboard", &[("name", &found.name)])
                    }
                    capture::Kind::Mouse => {
                        i18n::tf("messages.found_mouse", &[("name", &found.name)])
                    }
                    capture::Kind::Touchpad => {
                        i18n::tf("messages.found_touchpad", &[("name", &found.name)])
                    }
                };
                println!("{line}");
            }
            Ok(Input { devices })
        }

        pub fn start<T: From<InputEvent> + Send + 'static>(
            &self,
            events: Sender<T>,
            touchpad: &config::Touchpad,
        ) -> Result<(), String> {
            // A keyboard the touchpad's `disable while typing` can ask
            // about: the first keyboard among the devices. None when this
            // PC has no keyboard (a tablet with a touchpad, say).
            let keyboard: Option<Arc<Device>> = self
                .devices
                .iter()
                .find(|found| found.kind == capture::Kind::Keyboard)
                .map(|found| Arc::clone(&found.device));
            capture::spawn_readers(&self.devices, events, touchpad, keyboard);
            Ok(())
        }

        pub fn any_key_down(&self) -> bool {
            self.devices
                .iter()
                .any(|found| found.device.any_key_down().unwrap_or(true))
        }

        pub fn button_down(&self) -> bool {
            self.devices
                .iter()
                .any(|found| found.device.button_down().unwrap_or(true))
        }

        /// Where the cursor is on this desktop, if the compositor says.
        /// A touchpad kept local (`forward = false`) is not grabbed: its
        /// events keep reaching this PC's own cursor while the keyboard
        /// and mouse go to another PC.
        pub fn grab(&self) -> Result<Grabbed<'_>, String> {
            let grabs = self
                .devices
                .iter()
                .filter(|found| match found.kind {
                    capture::Kind::Touchpad => found.forward,
                    _ => true,
                })
                .map(|found| found.device.grab())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|err| format!("cannot grab the input devices: {err}"))?;
            Ok(Grabbed { _grabs: grabs })
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use crate::input::keymap;
    use crate::platform::hooks::{self, HookEvent, Point};
    use std::marker::PhantomData;
    use std::sync::{Arc, Mutex};

    /// evdev codes for left, right, middle, back and forward.
    const BUTTONS: [u16; 5] = [0x110, 0x111, 0x112, 0x113, 0x114];

    pub struct Input {
        /// Where the cursor was pinned when the focus left; `None` while
        /// local. Motion is measured from it.
        anchor: Arc<Mutex<Option<Point>>>,
    }

    /// While this lives, the hooks swallow every event.
    pub struct Grabbed<'a> {
        anchor: Arc<Mutex<Option<Point>>>,
        _input: PhantomData<&'a Input>,
    }

    impl Drop for Grabbed<'_> {
        fn drop(&mut self) {
            hooks::set_swallow(false);
            *self.anchor.lock_anyway() = None;
        }
    }

    /// Where the cursor is: a local call, cheap.
    pub fn cursor_now() -> Option<(i32, i32)> {
        let point = hooks::cursor_pos();
        Some((point.x, point.y))
    }

    impl Input {
        pub fn open(_touchpad: &config::Touchpad) -> Result<Input, String> {
            println!("{}", mickey_config::i18n::t("capture.capturing_hooks"));
            Ok(Input {
                anchor: Arc::new(Mutex::new(None)),
            })
        }

        /// Installs the hooks; a thread turns what they see into
        /// `InputEvent`s with evdev codes.
        pub fn start<T: From<InputEvent> + Send + 'static>(
            &self,
            events: Sender<T>,
            _touchpad: &config::Touchpad,
        ) -> Result<(), String> {
            let (raw_sender, raw) = std::sync::mpsc::channel();
            hooks::start(raw_sender).map_err(|err| format!("cannot install the hooks: {err}"))?;
            let anchor = Arc::clone(&self.anchor);
            crate::threads::spawn("input translate", move || {
                for event in raw {
                    let anchor = *anchor.lock_anyway();
                    if let Some(event) = translate(event, anchor)
                        && events.send(T::from(event)).is_err()
                    {
                        return;
                    }
                }
            });
            Ok(())
        }

        pub fn any_key_down(&self) -> bool {
            hooks::any_key_down()
        }

        pub fn button_down(&self) -> bool {
            hooks::button_down()
        }

        /// Pins the cursor where it is and starts swallowing.
        pub fn grab(&self) -> Result<Grabbed<'_>, String> {
            *self.anchor.lock_anyway() = Some(hooks::cursor_pos());
            hooks::set_swallow(true);
            Ok(Grabbed {
                anchor: Arc::clone(&self.anchor),
                _input: PhantomData,
            })
        }
    }

    /// Hook events to Mickey's events. Motion only counts while the cursor
    /// is pinned: swallowed moves leave it at `anchor`, so each reported
    /// position minus the anchor is exactly one movement.
    fn translate(event: HookEvent, anchor: Option<Point>) -> Option<InputEvent> {
        match event {
            HookEvent::Key { scan, pressed } => {
                keymap::evdev_code(scan).map(|code| InputEvent::Key { code, pressed })
            }
            HookEvent::Button { button, pressed } => BUTTONS
                .get(usize::from(button))
                .map(|&code| InputEvent::Button { code, pressed }),
            HookEvent::Delta { dx, dy } => Some(InputEvent::Motion { dx, dy }),
            // With raw input, positions are not motion (a game may pin the
            // cursor); without it, motion is measured from the anchor.
            HookEvent::Move { .. } if hooks::raw_mouse_active() => None,
            HookEvent::Move { x, y } => {
                let anchor = anchor?;
                let (dx, dy) = (x - anchor.x, y - anchor.y);
                (dx != 0 || dy != 0).then_some(InputEvent::Motion { dx, dy })
            }
            HookEvent::Wheel {
                vertical,
                horizontal,
            } => Some(InputEvent::Scroll {
                vertical,
                horizontal,
            }),
        }
    }
}
