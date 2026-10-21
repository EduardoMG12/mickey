//! Hand-written bindings for the one Win32 call Mickey needs: `SendInput`.
//!
//! Spike 0b chose hand-written bindings over the `windows-sys` crate: the
//! surface is one function and three structs, and writing them out shows
//! exactly what crosses the boundary. See PRD 0001, *Discoveries*.

use std::io;
use std::mem::size_of;

// Values from `winuser.h`.
const INPUT_MOUSE: u32 = 0;
const INPUT_KEYBOARD: u32 = 1;
const KEYEVENTF_KEYUP: u32 = 0x0002;
const KEYEVENTF_SCANCODE: u32 = 0x0008;
const MOUSEEVENTF_MOVE: u32 = 0x0001;

/// `MOUSEINPUT` from `winuser.h`. Field names follow Rust style; order and
/// types must match the C struct exactly, which `#[repr(C)]` guarantees.
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

/// `KEYBDINPUT` from `winuser.h`.
#[repr(C)]
#[derive(Clone, Copy)]
struct KeybdInput {
    virtual_key: u16,
    scan_code: u16,
    flags: u32,
    time: u32,
    extra_info: usize,
}

/// The anonymous union inside `INPUT`. `HARDWAREINPUT` is left out: it is
/// smaller than `MOUSEINPUT`, so the union's size does not change.
#[repr(C)]
#[derive(Clone, Copy)]
union InputUnion {
    mouse: MouseInput,
    keyboard: KeybdInput,
}

/// `INPUT` from `winuser.h`: a tag (`kind`) saying which union field is valid.
#[repr(C)]
#[derive(Clone, Copy)]
struct Input {
    kind: u32,
    data: InputUnion,
}

// Windows checks `cbSize` and rejects the call if our layout is wrong, so
// catch a mismatch at compile time instead. 40 bytes is `sizeof(INPUT)` on
// 64-bit Windows.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Input>() == 40);

#[link(name = "user32")]
unsafe extern "system" {
    /// Returns how many inputs were injected; 0 means blocked or failed.
    fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
}

/// Presses or releases one key by its scan code (the physical key position),
/// so Windows applies its own keyboard layout.
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

/// Moves the cursor by a relative amount, in mickeys (raw mouse units).
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
    // A slice longer than u32::MAX is impossible here (callers pass one
    // element), but convert instead of casting so it can never truncate.
    let count = u32::try_from(inputs.len()).map_err(io::Error::other)?;
    // SAFETY: `inputs` points to `count` valid, initialized `Input` values
    // that live until the call returns, and every one has `kind` matching
    // the union field we wrote. `Input` is `#[repr(C)]` with the same layout
    // as the C `INPUT` (checked by the size assertion above), and `size` is
    // its exact size, which Windows uses to validate the layout.
    let injected = unsafe { SendInput(count, inputs.as_ptr(), size_of::<Input>() as i32) };
    if injected == count {
        Ok(())
    } else {
        // Windows gives no error code when another program blocks the input
        // (UIPI, secure desktop), so `last_os_error` may just say "success".
        Err(io::Error::last_os_error())
    }
}
