//! Every thread Mickey starts has a name: `top -H`, `htop`, gdb and the
//! Windows debuggers show it, so one can tell which part of Mickey is busy
//! or stuck (the performance battery could not: 40 threads all called
//! "mickey"). Linux keeps the first 15 bytes of a name.

use std::sync::{Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

/// Starts `work` on a thread called `name`. Like `thread::spawn`, it
/// panics if the system cannot start one.
pub fn spawn<T: Send + 'static>(
    name: &str,
    work: impl FnOnce() -> T + Send + 'static,
) -> JoinHandle<T> {
    thread::Builder::new()
        .name(name.to_string())
        .spawn(work)
        .expect("the system starts a thread")
}

/// Like `spawn`, but says so instead of panicking when the system cannot
/// start a thread: a loop that starts one per connection keeps running.
pub fn try_spawn(name: &str, work: impl FnOnce() + Send + 'static) {
    if let Err(err) = thread::Builder::new().name(name.to_string()).spawn(work) {
        eprintln!("mickey: cannot start a thread for a connection: {err}");
    }
}

/// Locking that goes on after another thread panicked while holding the
/// lock. Mickey's shared state stays usable then (a list of peers, a
/// clipboard), and stopping every thread over one panic would leave the
/// keyboard stuck on another PC.
pub trait LockAnyway<T> {
    fn lock_anyway(&self) -> MutexGuard<'_, T>;
}

impl<T> LockAnyway<T> for Mutex<T> {
    fn lock_anyway(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_thread_carries_its_name() {
        let name = super::spawn("clip mover", || {
            std::thread::current().name().map(str::to_string)
        });
        assert_eq!(name.join().unwrap().as_deref(), Some("clip mover"));
    }
}
