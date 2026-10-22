//! Hand-written evdev bindings: read input events and grab a device.

use std::fs::File;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::Path;

// ioctl numbers from <linux/input.h>, expanded from the _IOW/_IOR macros.
// `_IOR` puts the buffer size in bits 16..30, hence the `<< 16`.
const EVIOCGRAB: u64 = 0x4004_4590;
const BITS_BYTES: usize = 96; // KEY_MAX (0x2ff) + 1 bits, the largest bitmap
const EVIOCGKEY: u64 = 0x8000_4518 | ((BITS_BYTES as u64) << 16);
const NAME_BYTES: usize = 256;
const EVIOCGNAME: u64 = 0x8000_4506 | ((NAME_BYTES as u64) << 16);
/// `EVIOCGABS(axis)`: one axis' `struct input_absinfo`, six `i32`s.
const ABSINFO_BYTES: usize = 24;
const fn eviocgabs(axis: u16) -> u64 {
    (0x8000_4540 + axis as u64) | ((ABSINFO_BYTES as u64) << 16)
}
/// `EVIOCGBIT(ev_type)`: which codes of one event type the device has.
const fn eviocgbit(ev_type: u16) -> u64 {
    (0x8000_4520 + ev_type as u64) | ((BITS_BYTES as u64) << 16)
}

// `struct input_event` on 64-bit Linux: a 16-byte timeval, then type, code, value.
const EVENT_SIZE: usize = 24;

unsafe extern "C" {
    /// Shared with `uinput.rs`.
    pub(super) fn ioctl(fd: i32, request: u64, ...) -> i32;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEvent {
    pub kind: u16,
    pub code: u16,
    pub value: i32,
}

pub struct Device {
    file: File,
}

impl Device {
    pub fn open(path: &Path) -> io::Result<Device> {
        Ok(Device {
            file: File::open(path)?,
        })
    }

    /// Blocks until the device has an event.
    pub fn read_event(&self) -> io::Result<RawEvent> {
        let mut buf = [0u8; EVENT_SIZE];
        // The kernel only hands out whole events, never part of one.
        (&self.file).read_exact(&mut buf)?;
        Ok(RawEvent {
            kind: u16::from_ne_bytes([buf[16], buf[17]]),
            code: u16::from_ne_bytes([buf[18], buf[19]]),
            value: i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]),
        })
    }

    pub fn any_key_down(&self) -> io::Result<bool> {
        let state = self.bitmap(EVIOCGKEY)?;
        Ok(state.iter().any(|&byte| byte != 0))
    }

    /// Whether the device can send `code` of `ev_type`, e.g. `EV_KEY`, `KEY_A`.
    pub fn supports(&self, ev_type: u16, code: u16) -> io::Result<bool> {
        let bits = self.bitmap(eviocgbit(ev_type))?;
        let code = usize::from(code);
        Ok(bits
            .get(code / 8)
            .is_some_and(|byte| byte & (1 << (code % 8)) != 0))
    }

    /// Device units per millimetre on an absolute axis; 0 when unknown.
    pub fn abs_resolution(&self, axis: u16) -> io::Result<i32> {
        let mut info = [0u8; ABSINFO_BYTES];
        self.ioctl_read(eviocgabs(axis), &mut info)?;
        // Fields: value, minimum, maximum, fuzz, flat, resolution.
        Ok(i32::from_ne_bytes([info[20], info[21], info[22], info[23]]))
    }

    pub fn name(&self) -> io::Result<String> {
        let mut name = [0u8; NAME_BYTES];
        let len = self.ioctl_read(EVIOCGNAME, &mut name)?;
        let name = name[..len].split(|&byte| byte == 0).next().unwrap_or(&[]);
        Ok(String::from_utf8_lossy(name).into_owned())
    }

    fn bitmap(&self, request: u64) -> io::Result<[u8; BITS_BYTES]> {
        let mut bits = [0u8; BITS_BYTES];
        self.ioctl_read(request, &mut bits)?;
        Ok(bits)
    }

    /// Runs a read ioctl whose size field equals `buf.len()`; returns the
    /// number of bytes the kernel wrote.
    fn ioctl_read(&self, request: u64, buf: &mut [u8]) -> io::Result<usize> {
        debug_assert_eq!((request >> 16) & 0x3fff, buf.len() as u64);
        // SAFETY: every caller passes a request whose size field is
        // `buf.len()`, so the kernel writes at most that many bytes into
        // `buf`, which lives for the whole call. The fd is open as long as
        // `self`.
        let written = unsafe { ioctl(self.file.as_raw_fd(), request, buf.as_mut_ptr()) };
        if written < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(written as usize)
    }

    /// Takes the device away from the desktop until the guard is dropped.
    pub fn grab(&self) -> io::Result<Grab<'_>> {
        set_grab(&self.file, true)?;
        Ok(Grab { file: &self.file })
    }
}

/// Releases the grab when dropped, including during a panic. If the process
/// is killed, the kernel releases it when the file is closed.
pub struct Grab<'a> {
    file: &'a File,
}

impl Drop for Grab<'_> {
    fn drop(&mut self) {
        if let Err(err) = set_grab(self.file, false) {
            eprintln!("mickey: failed to release the grab: {err}");
        }
    }
}

fn set_grab(file: &File, on: bool) -> io::Result<()> {
    // SAFETY: EVIOCGRAB takes an int by value, not a pointer, and the fd is
    // open because `file` is borrowed for the whole call.
    let result = unsafe { ioctl(file.as_raw_fd(), EVIOCGRAB, i32::from(on)) };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
