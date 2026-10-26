//! Every thread Mickey starts has a name: `top -H`, `htop`, gdb and the
//! Windows debuggers show it, so one can tell which part of Mickey is busy
//! or stuck (the performance battery could not: 40 threads all called
//! "mickey"). Linux keeps the first 15 bytes of a name.

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
