//! Hand-written bindings for `SendInput`.

use crate::keymap::MouseButton;
use std::io;
use std::mem::size_of;

const INPUT_MOUSE: u32 = 0;
const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_EXTENDEDKEY: u32 = 0x0001;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const KEYEVENTF_SCANCODE: u32 = 0x0008;
const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
const MOUSEEVENTF_MIDDLEDOWN: u32 = 0x0020;
const MOUSEEVENTF_MIDDLEUP: u32 = 0x0040;
const MOUSEEVENTF_XDOWN: u32 = 0x0080;
const MOUSEEVENTF_XUP: u32 = 0x0100;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const MOUSEEVENTF_HWHEEL: u32 = 0x1000;
const XBUTTON1: u32 = 0x0001;
const XBUTTON2: u32 = 0x0002;

#[repr(C)]
#[derive(Clone, Copy)]
struct MouseInput {
    dx: i32,
    dy: i32,
    mouse_data: u32,
    flags: u32,
    time: u32,
    extra_info: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct KeybdInput {
    virtual_key: u16,
    scan_code: u16,
    flags: u32,
    time: u32,
    extra_info: usize,
}

// HARDWAREINPUT is smaller than MouseInput, so leaving it out keeps the size.
#[repr(C)]
#[derive(Clone, Copy)]
union InputUnion {
    mouse: MouseInput,
    keyboard: KeybdInput,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Input {
    kind: u32,
    data: InputUnion,
}

// sizeof(INPUT) on 64-bit Windows.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Input>() == 40);

#[link(name = "user32")]
unsafe extern "system" {
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
}

/// Puts the cursor at a point of the virtual desktop.
pub fn set_cursor_pos(x: i32, y: i32) -> io::Result<()> {
    // SAFETY: takes two plain integers; Windows clamps them to the desktop.
    match unsafe { SetCursorPos(x, y) } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

/// Sends a scan code, so the target's keyboard layout decides the character.
/// `extended` keys (arrows, AltGr, ...) carry the E0 prefix.
pub fn send_key(scan_code: u16, extended: bool, pressed: bool) -> io::Result<()> {
    let mut flags = KEYEVENTF_SCANCODE;
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    if !pressed {
        flags |= KEYEVENTF_KEYUP;
    }
    send(&[Input {
        kind: INPUT_KEYBOARD,
        data: InputUnion {
            keyboard: KeybdInput {
                virtual_key: 0,
                scan_code,
                flags,
                time: 0,
                extra_info: 0,
            },
        },
    }])
}

pub fn move_mouse(dx: i32, dy: i32) -> io::Result<()> {
    send_mouse(dx, dy, 0, MOUSEEVENTF_MOVE)
}

pub fn send_button(button: MouseButton, pressed: bool) -> io::Result<()> {
    let (down, up, data) = match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, 0),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, 0),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, 0),
        MouseButton::X1 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON1),
        MouseButton::X2 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, XBUTTON2),
    };
    send_mouse(0, 0, data, if pressed { down } else { up })
}

/// Wheel amounts are in 120ths of a notch; positive is up or right.
pub fn scroll(vertical: i32, horizontal: i32) -> io::Result<()> {
    // `mouseData` is a DWORD that Windows reads as signed, so a negative
    // amount is passed as its two's-complement bits (`as u32`).
    if vertical != 0 {
        send_mouse(0, 0, vertical as u32, MOUSEEVENTF_WHEEL)?;
    }
    if horizontal != 0 {
        send_mouse(0, 0, horizontal as u32, MOUSEEVENTF_HWHEEL)?;
    }
    Ok(())
}

fn send_mouse(dx: i32, dy: i32, mouse_data: u32, flags: u32) -> io::Result<()> {
    send(&[Input {
        kind: INPUT_MOUSE,
        data: InputUnion {
            mouse: MouseInput {
                dx,
                dy,
                mouse_data,
                flags,
                time: 0,
                extra_info: 0,
            },
        },
    }])
}

fn send(inputs: &[Input]) -> io::Result<()> {
    let count = u32::try_from(inputs.len()).map_err(io::Error::other)?;
    // SAFETY: `inputs` is a live slice of `count` initialized values, each
    // `kind` matches the union field written, and `Input` has the C layout
    // of `INPUT` (size checked above).
    let injected = unsafe { SendInput(count, inputs.as_ptr(), size_of::<Input>() as i32) };
    if injected == count {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// `RECT` from `windef.h`.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

type MonitorEnumProc = unsafe extern "system" fn(isize, isize, *mut Rect, isize) -> i32;

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumDisplayMonitors(
        hdc: isize,
        clip: *const Rect,
        proc: MonitorEnumProc,
        data: isize,
    ) -> i32;
}

/// Every monitor's rectangle on the virtual desktop.
pub fn monitors() -> io::Result<Vec<crate::monitors::Monitor>> {
    unsafe extern "system" fn add(
        _monitor: isize,
        _hdc: isize,
        rect: *mut Rect,
        data: isize,
    ) -> i32 {
        // SAFETY: `data` is the `&mut Vec` passed below, alive for the whole
        // EnumDisplayMonitors call, and only this callback touches it;
        // `rect` points to the monitor's rectangle for this call.
        let (found, rect) = unsafe { (&mut *(data as *mut Vec<crate::monitors::Monitor>), *rect) };
        found.push(crate::monitors::Monitor {
            x: rect.left,
            y: rect.top,
            width: rect.right - rect.left,
            height: rect.bottom - rect.top,
        });
        1 // keep going
    }
    let mut found: Vec<crate::monitors::Monitor> = Vec::new();
    // SAFETY: no device context and no clip rectangle means "every monitor";
    // `add` matches `MonitorEnumProc`; the pointer to `found` outlives the
    // call, which is synchronous.
    let ok =
        unsafe { EnumDisplayMonitors(0, std::ptr::null(), add, &mut found as *mut _ as isize) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(found)
}
