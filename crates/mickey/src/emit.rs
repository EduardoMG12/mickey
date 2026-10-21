//! Turns Mickey events into real input on Windows.

use crate::event::InputEvent;
use crate::keymap;
use crate::platform::windows;
use std::io;

/// Injects one event. Keys and buttons with no Windows equivalent are
/// dropped.
// ponytail: dropped silently; the PRD wants each unknown code logged once,
// which waits for a logger that keeps codes at trace level.
pub fn emit(event: InputEvent) -> io::Result<()> {
    match event {
        InputEvent::Key { code, pressed } => match keymap::scan_code(code) {
            Some(scan) => windows::send_key(u16::from(scan.code), scan.extended, pressed),
            None => Ok(()),
        },
        InputEvent::Button { code, pressed } => match keymap::mouse_button(code) {
            Some(button) => windows::send_button(button, pressed),
            None => Ok(()),
        },
        InputEvent::Motion { dx, dy } => windows::move_mouse(dx, dy),
        InputEvent::Scroll {
            vertical,
            horizontal,
        } => windows::scroll(vertical, horizontal),
    }
}
