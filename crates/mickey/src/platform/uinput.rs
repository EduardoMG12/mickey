//! Hand-written uinput bindings: a virtual keyboard and mouse. The kernel
//! hands their events to the compositor exactly like a real device's, so
//! this works on any Linux desktop (PRD 0008).

use super::linux::ioctl;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;

const PATH: &str = "/dev/uinput";
/// The name Mickey gives its virtual device, so capture can skip it.
pub const DEVICE_NAME: &str = "Mickey virtual input";

// ioctl numbers from <linux/uinput.h>, checked against the C headers.
const UI_SET_EVBIT: u64 = 0x4004_5564;
const UI_SET_KEYBIT: u64 = 0x4004_5565;
const UI_SET_RELBIT: u64 = 0x4004_5566;
const UI_DEV_SETUP: u64 = 0x405c_5503;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;

pub const EV_SYN: u16 = 0;
pub const EV_KEY: u16 = 1;
pub const EV_REL: u16 = 2;
const SYN_REPORT: u16 = 0;
const BUS_VIRTUAL: u16 = 6;

/// Every keyboard key (1-255) and the mouse buttons Mickey sends.
const KEYS: std::ops::RangeInclusive<u16> = 1..=255;
const BUTTONS: std::ops::RangeInclusive<u16> = 0x110..=0x117;
/// REL_X, REL_Y, REL_HWHEEL, REL_WHEEL, REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES.
const AXES: [u16; 6] = [0, 1, 6, 8, 11, 12];

/// `struct uinput_setup`: an 8-byte `input_id`, an 80-byte name and a
/// `u32`, 92 bytes in all.
const SETUP_LEN: usize = 92;
const NAME_LEN: usize = 80;

pub struct VirtualDevice {
    file: File,
}

impl VirtualDevice {
    /// Creates the device. Fails with `PermissionDenied` unless the user
    /// may write to `/dev/uinput` (see the README's udev rule).
    pub fn create(name: &str) -> io::Result<VirtualDevice> {
        let file = OpenOptions::new().write(true).open(PATH)?;
        let device = VirtualDevice { file };
        for kind in [EV_SYN, EV_KEY, EV_REL] {
            device.set(UI_SET_EVBIT, kind)?;
        }
        for code in KEYS.chain(BUTTONS) {
            device.set(UI_SET_KEYBIT, code)?;
        }
        for axis in AXES {
            device.set(UI_SET_RELBIT, axis)?;
        }

        let mut setup = [0u8; SETUP_LEN];
        setup[0..2].copy_from_slice(&BUS_VIRTUAL.to_ne_bytes());
        let name = name.as_bytes();
        // Leave room for the terminating zero.
        let len = name.len().min(NAME_LEN - 1);
        setup[8..8 + len].copy_from_slice(&name[..len]);
        // SAFETY: UI_DEV_SETUP reads `sizeof(struct uinput_setup)` (92)
        // bytes from the pointer, and `setup` is exactly that long and lives
        // for the whole call. The fd is open for as long as `device`.
        let result = unsafe { ioctl(device.fd(), UI_DEV_SETUP, setup.as_ptr()) };
        check(result)?;
        // SAFETY: UI_DEV_CREATE takes no argument; the fd is open.
        check(unsafe { ioctl(device.fd(), UI_DEV_CREATE) })?;
        Ok(device)
    }

    /// Queues one event; the compositor sees a batch at `sync`.
    pub fn emit(&mut self, kind: u16, code: u16, value: i32) -> io::Result<()> {
        // `struct input_event` on 64-bit Linux: a 16-byte timeval (zero:
        // the kernel stamps it), type, code, value.
        let mut event = [0u8; 24];
        event[16..18].copy_from_slice(&kind.to_ne_bytes());
        event[18..20].copy_from_slice(&code.to_ne_bytes());
        event[20..24].copy_from_slice(&value.to_ne_bytes());
        self.file.write_all(&event)
    }

    /// Ends a batch of events (`SYN_REPORT`).
    pub fn sync(&mut self) -> io::Result<()> {
        self.emit(EV_SYN, SYN_REPORT, 0)
    }

    fn set(&self, request: u64, code: u16) -> io::Result<()> {
        // SAFETY: the UI_SET_* requests take an int by value, not a
        // pointer; the fd is open for as long as `self`.
        check(unsafe { ioctl(self.fd(), request, i32::from(code)) })
    }

    fn fd(&self) -> i32 {
        self.file.as_raw_fd()
    }
}

/// The kernel would also remove the device when the file closes; destroying
/// it explicitly makes it vanish at once.
impl Drop for VirtualDevice {
    fn drop(&mut self) {
        // SAFETY: UI_DEV_DESTROY takes no argument; the fd is still open
        // because `self.file` is dropped only after this returns.
        let _ = unsafe { ioctl(self.fd(), UI_DEV_DESTROY) };
    }
}

fn check(result: i32) -> io::Result<()> {
    if result < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    /// Spike #69, run by hand: `cargo test -- --ignored uinput_spike`.
    /// It types `a` into the focused window and moves the cursor 200 units
    /// right, so focus a harmless window first.
    #[test]
    #[ignore = "creates a real input device; run by hand"]
    fn uinput_spike_types_and_moves() {
        let mut device = VirtualDevice::create("Mickey spike").expect("needs /dev/uinput");
        // Give the compositor time to notice the new device.
        sleep(Duration::from_secs(1));
        for value in [1, 0] {
            device.emit(EV_KEY, 30, value).unwrap(); // KEY_A
            device.sync().unwrap();
        }
        for _ in 0..20 {
            device.emit(EV_REL, 0, 10).unwrap(); // REL_X
            device.sync().unwrap();
            sleep(Duration::from_millis(10));
        }
        sleep(Duration::from_millis(200));
    }
}
