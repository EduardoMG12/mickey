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

fn stop_signals() -> libc::sigset_t {
    // SAFETY: an all-zero `sigset_t` is a valid value before `sigemptyset`.
    let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
    // SAFETY: `set` is a valid, writable `sigset_t`.
    unsafe {
        libc::sigemptyset(&mut set);
        for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGTERM] {
            libc::sigaddset(&mut set, signal);
        }
    }
    set
}

/// Blocks SIGTERM, SIGINT and SIGHUP on the calling thread so subsequent
/// threads inherit the mask and only the `sigwait` thread receives them.
pub fn block_stop_signals() -> io::Result<()> {
    let set = stop_signals();
    // SAFETY: `set` is initialised above; the old mask is not wanted.
    let error = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut()) };
    if error != 0 {
        return Err(io::Error::from_raw_os_error(error));
    }
    Ok(())
}

/// From now on, SIGTERM, SIGINT and SIGHUP run `cleanup` on a thread of its
/// own, then end the process as the signal would have (exit code 128 + the
/// signal). Call it before starting other threads: they inherit the
/// blocked signals, so only the waiting thread ever takes one. Programs
/// Mickey starts get the usual signals back (`std::process` resets the
/// mask).
pub fn on_stop(cleanup: fn()) -> io::Result<()> {
    block_stop_signals()?;
    let set = stop_signals();
    std::thread::Builder::new()
        .name("signals".into())
        .spawn(move || {
            let mut signal = 0;
            // SAFETY: `set` lives in this closure; `signal` is a valid
            // out-pointer. sigwait returns the signal instead of running a
            // handler.
            if unsafe { libc::sigwait(&set, &mut signal) } == 0 {
                println!("mickey: signal {signal}, ending");
                cleanup();
                std::process::exit(128 + signal);
            }
        })?;
    Ok(())
}
