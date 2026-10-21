//! Hand-written bindings for `SendInput`.

use std::io;
use std::mem::size_of;

const INPUT_MOUSE: u32 = 0;
const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const KEYEVENTF_SCANCODE: u32 = 0x0008;
const MOUSEEVENTF_MOVE: u32 = 0x0001;

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
}

/// Sends a scan code, so the target's keyboard layout decides the character.
pub fn send_key(scan_code: u16, pressed: bool) -> io::Result<()> {
    let mut flags = KEYEVENTF_SCANCODE;
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
    send(&[Input {
        kind: INPUT_MOUSE,
        data: InputUnion {
            mouse: MouseInput {
                dx,
                dy,
                mouse_data: 0,
                flags: MOUSEEVENTF_MOVE,
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
