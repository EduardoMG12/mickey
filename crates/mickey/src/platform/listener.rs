//! Which program listens on a localhost TCP port, on Windows (#349). The
//! control file says where the running Mickey listens, but any program of
//! the user can write it: before believing another Mickey runs, Mickey
//! asks Windows who owns that port and compares the program with itself.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

/// `MIB_TCPROW_OWNER_PID`: one TCP socket and the process that owns it.
#[repr(C)]
#[derive(Clone, Copy)]
struct TcpRow {
    state: u32,
    /// The address and port in network order (the port in the low 16 bits).
    local_address: u32,
    local_port: u32,
    remote_address: u32,
    remote_port: u32,
    pid: u32,
}

const AF_INET: u32 = 2;
const TCP_TABLE_OWNER_PID_LISTENER: u32 = 3;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

#[link(name = "iphlpapi")]
unsafe extern "system" {
    fn GetExtendedTcpTable(
        table: *mut u8,
        size: *mut u32,
        order: i32,
        family: u32,
        class: u32,
        reserved: u32,
    ) -> u32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
    fn QueryFullProcessImageNameW(
        process: *mut std::ffi::c_void,
        flags: u32,
        name: *mut u16,
        size: *mut u32,
    ) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}

/// The program listening on 127.0.0.1:`port`, if Windows says.
pub fn program_on(port: u16) -> Option<PathBuf> {
    exe_of(pid_on(port)?)
}

fn pid_on(port: u16) -> Option<u32> {
    // Sockets may open between the two calls: ask a few times.
    let mut size = 0u32;
    for _ in 0..3 {
        // u32s, so the table (a u32 count, then rows of u32s) is aligned.
        let mut buffer = vec![0u32; (size as usize).div_ceil(4).max(1)];
        // SAFETY: `buffer` holds at least `size` bytes and lives through
        // the call; Windows writes the count, then that many rows, within
        // it, or only the size it needs.
        let answer = unsafe {
            GetExtendedTcpTable(
                buffer.as_mut_ptr().cast(),
                &mut size,
                0,
                AF_INET,
                TCP_TABLE_OWNER_PID_LISTENER,
                0,
            )
        };
        if answer == ERROR_INSUFFICIENT_BUFFER {
            continue;
        }
        if answer != 0 {
            return None;
        }
        let count = buffer[0] as usize;
        let row_words = size_of::<TcpRow>() / 4;
        let rows = &buffer[1..];
        if rows.len() < count * row_words {
            return None;
        }
        let localhost = u32::from_ne_bytes([127, 0, 0, 1]);
        return (0..count)
            .map(|i| {
                // SAFETY: row `i` lies inside `rows` (checked just above),
                // which is u32-aligned, as TcpRow needs.
                unsafe { *(rows[i * row_words..].as_ptr() as *const TcpRow) }
            })
            .find(|row| {
                row.local_address == localhost && u16::from_be(row.local_port as u16) == port
            })
            .map(|row| row.pid);
    }
    None
}

fn exe_of(pid: u32) -> Option<PathBuf> {
    // SAFETY: no pointers; null when the process is gone or not ours to ask.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut name = vec![0u16; 32_768];
    let mut size = name.len() as u32;
    // SAFETY: `process` is open; `name` holds `size` u16s, and Windows
    // writes at most that many and sets `size` to the length written.
    let found = unsafe { QueryFullProcessImageNameW(process, 0, name.as_mut_ptr(), &mut size) };
    // SAFETY: `process` was opened above and is closed once.
    unsafe { CloseHandle(process) };
    (found != 0).then(|| PathBuf::from(OsString::from_wide(&name[..size as usize])))
}
