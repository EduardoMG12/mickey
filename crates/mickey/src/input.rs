//! The source's keyboard and mouse, per platform (PRD 0008): evdev devices
//! on Linux, low-level hooks on Windows. `mickey source` only uses this
//! interface, so the focus, hotkey and switcher work the same on both.

use crate::event::InputEvent;
#[cfg(windows)]
use crate::threads::LockAnyway;
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
    use crate::capture;
    use crate::platform::linux::Grab;

    pub struct Input {
        devices: Vec<capture::Found>,
    }

    /// While this lives, the devices are taken from the desktop.
    pub struct Grabbed<'a> {
        _grabs: Vec<Grab<'a>>,
    }

    /// Where the cursor is: a question to Hyprland or the X server,
    /// which may take a few milliseconds (#187).
    pub fn cursor_now() -> Option<(i32, i32)> {
        match crate::desktop::session() {
            crate::desktop::Session::X11 => crate::x11::cursor().ok(),
            _ => crate::monitors::hyprland::cursor().ok(),
        }
    }

    impl Input {
        /// Every keyboard, mouse and touchpad, with a clear message when we
        /// may not read them.
        pub fn open() -> Result<Input, String> {
            let devices = capture::find_devices().map_err(|err| match err.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    "cannot read /dev/input: permission denied; run \
                     `sudo usermod -aG input $USER`, then log in again (or use `newgrp input`)"
                        .to_string()
                }
                _ => format!("cannot list input devices: {err}"),
            })?;
            if devices.is_empty() {
                return Err("no keyboard or mouse found under /dev/input".into());
            }
            for found in &devices {
                let kind = match found.kind {
                    capture::Kind::Keyboard => "keyboard",
                    capture::Kind::Mouse => "mouse",
                    capture::Kind::Touchpad => "touchpad",
                };
                println!("mickey: found {kind}: {}", found.name);
            }
            Ok(Input { devices })
        }

        pub fn start<T: From<InputEvent> + Send + 'static>(
            &self,
            events: Sender<T>,
            touchpad_speed: i32,
        ) -> Result<(), String> {
            capture::spawn_readers(&self.devices, events, touchpad_speed);
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
        pub fn grab(&self) -> Result<Grabbed<'_>, String> {
            let grabs = self
                .devices
                .iter()
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
    use crate::keymap;
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
        pub fn open() -> Result<Input, String> {
            println!("mickey: capturing the keyboard and mouse with low-level hooks");
            Ok(Input {
                anchor: Arc::new(Mutex::new(None)),
            })
        }

        /// Installs the hooks; a thread turns what they see into
        /// `InputEvent`s with evdev codes.
        pub fn start<T: From<InputEvent> + Send + 'static>(
            &self,
            events: Sender<T>,
            _touchpad_speed: i32,
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
