//! Hand-written evdev bindings: read input events and grab a device.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const O_NONBLOCK: i32 = 0o4000;

// ioctl numbers from <linux/input.h>, expanded from the _IOW/_IOR macros.
const EVIOCGRAB: u64 = 0x4004_4590;
const KEY_STATE_BYTES: usize = 96; // KEY_MAX (0x2ff) + 1 bits
const EVIOCGKEY: u64 = 0x8000_4518 | ((KEY_STATE_BYTES as u64) << 16);

// `struct input_event` on 64-bit Linux: a 16-byte timeval, then type, code, value.
const EVENT_SIZE: usize = 24;

unsafe extern "C" {
    fn ioctl(fd: i32, request: u64, ...) -> i32;
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
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(O_NONBLOCK)
            .open(path)?;
        Ok(Device { file })
    }

    /// Returns `None` when no event is waiting (the device is non-blocking).
    pub fn read_event(&self) -> io::Result<Option<RawEvent>> {
        let mut buf = [0u8; EVENT_SIZE];
        // The kernel only hands out whole events, never part of one.
        match (&self.file).read_exact(&mut buf) {
            Ok(()) => Ok(Some(RawEvent {
                kind: u16::from_ne_bytes([buf[16], buf[17]]),
                code: u16::from_ne_bytes([buf[18], buf[19]]),
                value: i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]),
            })),
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(None),
            Err(err) => Err(err),
        }
    }

    pub fn any_key_down(&self) -> io::Result<bool> {
        let mut state = [0u8; KEY_STATE_BYTES];
        // SAFETY: EVIOCGKEY writes at most KEY_STATE_BYTES bytes (the size
        // encoded in the request) into `state`, which is that long and lives
        // for the whole call. The fd is open for as long as `self`.
        let result = unsafe { ioctl(self.file.as_raw_fd(), EVIOCGKEY, state.as_mut_ptr()) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(state.iter().any(|&byte| byte != 0))
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
