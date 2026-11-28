//! Whether the app that copied marked the copy as secret (#425), on
//! Windows: password managers (KeePassXC, Bitwarden's desktop app) add a
//! clipboard format saying "keep this out of the clipboard history and the
//! cloud", which Windows' own history (Win+V) obeys. Mickey obeys it too.
//!
//! Two marks, either one is enough:
//! - `ExcludeClipboardContentFromMonitorProcessing`, there at all;
//! - `CanIncludeInClipboardHistory`, a 4-byte number, set to 0.

use std::ffi::c_void;

#[link(name = "user32")]
unsafe extern "system" {
    fn RegisterClipboardFormatW(name: *const u16) -> u32;
    fn IsClipboardFormatAvailable(format: u32) -> i32;
    fn OpenClipboard(owner: isize) -> i32;
    fn CloseClipboard() -> i32;
    fn GetClipboardData(format: u32) -> *mut c_void;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GlobalLock(memory: *mut c_void) -> *mut c_void;
    fn GlobalUnlock(memory: *mut c_void) -> i32;
    fn GlobalSize(memory: *mut c_void) -> usize;
}

/// Whether what is copied now carries either mark.
pub fn copied_secret() -> bool {
    let excluded = format("ExcludeClipboardContentFromMonitorProcessing");
    // SAFETY: takes a plain format number; only looks.
    if excluded != 0 && unsafe { IsClipboardFormatAvailable(excluded) } != 0 {
        return true;
    }
    let history = format("CanIncludeInClipboardHistory");
    // SAFETY: as above.
    if history == 0 || unsafe { IsClipboardFormatAvailable(history) } == 0 {
        return false;
    }
    // Marked, but only 0 keeps it out. One that cannot be read (another
    // program holds the clipboard this instant) counts as secret: a copy
    // kept here by mistake costs less than a password sent by mistake.
    number_in(history).is_none_or(|allowed| allowed == 0)
}

/// The number Windows gives the clipboard format called `name` (the same
/// in every program); 0 if it will not.
fn format(name: &str) -> u32 {
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that lives through
    // the call.
    unsafe { RegisterClipboardFormatW(wide.as_ptr()) }
}

/// The 4-byte number held in `format`, if the clipboard can be opened and
/// holds one.
fn number_in(format: u32) -> Option<u32> {
    // SAFETY: no owner window; closed below on every path.
    if unsafe { OpenClipboard(0) } == 0 {
        return None;
    }
    // SAFETY: the clipboard is open; the handle stays the clipboard's (it
    // is never freed here).
    let data = unsafe { GetClipboardData(format) };
    let number = if data.is_null() {
        None
    } else {
        read_number(data)
    };
    // SAFETY: this thread opened it above.
    unsafe { CloseClipboard() };
    number
}

/// The first 4 bytes of the global memory `data`, if it holds as many.
fn read_number(data: *mut c_void) -> Option<u32> {
    // SAFETY: `data` is a clipboard handle, valid while the clipboard is
    // open; GlobalSize only reads its size.
    if unsafe { GlobalSize(data) } < 4 {
        return None;
    }
    // SAFETY: as above; locked until the unlock below.
    let bytes = unsafe { GlobalLock(data) } as *const u32;
    if bytes.is_null() {
        return None;
    }
    // SAFETY: at least 4 bytes, checked above; read unaligned, as global
    // memory makes no promise to a u32.
    let number = unsafe { bytes.read_unaligned() };
    // SAFETY: locked just above.
    unsafe { GlobalUnlock(data) };
    Some(number)
}

#[cfg(test)]
mod tests {
    /// Run by hand on Windows, after copying something: says whether the
    /// copy carries a secret mark.
    #[test]
    #[ignore = "reads this session's clipboard"]
    fn says_whether_what_is_copied_is_marked_secret() {
        println!("marked secret: {}", super::copied_secret());
    }
}
