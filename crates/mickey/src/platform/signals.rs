//! Ending cleanly on SIGTERM, SIGINT and SIGHUP (Linux). Until shared audio
//! (PRD 0012) Mickey left these to their default, ending at once: the kernel
//! releases a grabbed device when its file closes. Taking the sound off the
//! speakers is different: the sound server keeps that after Mickey is gone,
//! so `systemctl --user stop mickey` or Ctrl+C must give it back first.
//!
//! The signals are blocked, then one thread waits for them with `sigwait`
//! and runs the cleanup there: an ordinary thread, so it may talk to the
//! sound server, which a signal handler may not.

use std::io;

/// `sigset_t` on 64-bit glibc: 1024 bits.
#[repr(C)]
struct SigSet([u64; 16]);

const SIG_BLOCK: i32 = 0;
const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

unsafe extern "C" {
    fn sigemptyset(set: *mut SigSet) -> i32;
    fn sigaddset(set: *mut SigSet, signal: i32) -> i32;
    fn pthread_sigmask(how: i32, set: *const SigSet, old: *mut SigSet) -> i32;
    fn sigwait(set: *const SigSet, signal: *mut i32) -> i32;
}

fn stop_signals() -> SigSet {
    let mut set = SigSet([0; 16]);
    // SAFETY: `set` is a valid, writable sigset_t-sized buffer.
    unsafe {
        sigemptyset(&mut set);
        for signal in [SIGHUP, SIGINT, SIGTERM] {
            sigaddset(&mut set, signal);
        }
    }
    set
}

/// From now on, SIGTERM, SIGINT and SIGHUP run `cleanup` on a thread of its
/// own, then end the process as the signal would have (exit code 128 + the
/// signal). Call it before starting other threads: they inherit the
/// blocked signals, so only the waiting thread ever takes one. Programs
/// Mickey starts get the usual signals back (`std::process` resets the
/// mask).
pub fn on_stop(cleanup: fn()) -> io::Result<()> {
    let set = stop_signals();
    // SAFETY: `set` is initialised above; the old mask is not wanted.
    let error = unsafe { pthread_sigmask(SIG_BLOCK, &set, std::ptr::null_mut()) };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error));
    }
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            let mut signal = 0;
            // SAFETY: `set` lives in this closure; `signal` is a valid
            // out-pointer. sigwait returns the signal instead of running a
            // handler.
            if unsafe { sigwait(&set, &mut signal) } == 0 {
                println!("mickey: signal {signal}, ending");
                cleanup();
                std::process::exit(128 + signal);
            }
        })?;
    Ok(())
}
