//! A log file for Mickey on Windows. Started at logon by `start-hidden.vbs`,
//! it has no console, so everything it prints was lost: a problem could not
//! be looked into (#287). On Linux the systemd journal keeps it
//! (`journalctl --user -u mickey`).
//!
//! With no console anyone can see (none, or the hidden one `start-hidden.vbs`
//! gets: window style 0 still makes a console, only invisible), Mickey
//! points its standard output and error at
//! `%LOCALAPPDATA%\mickey\mickey.log`; the previous run's stays as
//! `mickey.log.old`. Rust's `println!` asks Windows for the standard handle
//! on every write, so it follows. The log holds what Mickey prints, never a
//! key pressed (hard rule 7).

use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;

const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const STD_ERROR_HANDLE: u32 = -12i32 as u32;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetConsoleWindow() -> *mut c_void;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn IsWindowVisible(window: *mut c_void) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetStdHandle(which: u32, handle: *mut c_void) -> i32;
}

/// When no console is shown, sends what Mickey prints to its log file.
/// Errors are ignored: Mickey runs the same without a log.
pub fn when_hidden() {
    // SAFETY: no arguments; null when the process has no console.
    let console = unsafe { GetConsoleWindow() };
    // SAFETY: `console` is null or this process's console window.
    if !console.is_null() && unsafe { IsWindowVisible(console) } != 0 {
        return;
    }
    let Some(dir) = std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("mickey"))
    else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    let log = dir.join("mickey.log");
    let _ = std::fs::rename(&log, dir.join("mickey.log.old"));
    let Ok(file) = std::fs::File::create(&log) else {
        return;
    };
    let handle = file.as_raw_handle();
    // SAFETY: `handle` is a valid file handle; it is kept open for the rest
    // of the process by leaking `file` below, so the standard handles never
    // point at a closed one.
    unsafe {
        SetStdHandle(STD_OUTPUT_HANDLE, handle);
        SetStdHandle(STD_ERROR_HANDLE, handle);
    }
    std::mem::forget(file);
}
