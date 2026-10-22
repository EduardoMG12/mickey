//! `mickey-ui` windows that Mickey may have to close before they answer:
//! the switcher closes when its hotkey is pressed again (PRD 0007).

use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The window open, if any, shared by the thread waiting for its answer and
/// whoever may close it.
pub type Slot = Arc<Mutex<Option<Child>>>;

/// How a window ended.
pub enum Outcome {
    /// It printed this (the switcher: the id of the PC picked).
    Answered(String),
    /// Closed without an answer: by the owner (Esc) or by `close`.
    Closed,
    Failed(String),
}

/// Runs `mickey-ui` with `args`, kept in `slot` (replacing and closing any
/// window there), and waits for it.
pub fn run(args: &[String], slot: &Slot) -> Outcome {
    let child = std::env::current_exe()
        .map(|exe| exe.with_file_name(format!("mickey-ui{}", std::env::consts::EXE_SUFFIX)))
        .and_then(|ui| {
            Command::new(ui)
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
        });
    let child = match child {
        Ok(child) => child,
        Err(err) => return Outcome::Failed(format!("mickey-ui: {err}")),
    };
    close(slot);
    *lock(slot) = Some(child);
    // Polls without holding the slot, so `close` is never kept waiting.
    let child = loop {
        std::thread::sleep(Duration::from_millis(50));
        let mut slot = lock(slot);
        let Some(child) = slot.as_mut() else {
            return Outcome::Closed;
        };
        match child.try_wait() {
            Ok(None) => continue,
            Ok(Some(_)) => break slot.take(),
            Err(err) => return Outcome::Failed(err.to_string()),
        }
    };
    let Some(child) = child else {
        return Outcome::Closed;
    };
    match child.wait_with_output() {
        // 1: closed with Esc; 2: no display, with the reason on stderr.
        Ok(output) => match output.status.code() {
            Some(0) => {
                Outcome::Answered(String::from_utf8_lossy(&output.stdout).trim().to_string())
            }
            Some(1) => Outcome::Closed,
            _ => Outcome::Failed(String::from_utf8_lossy(&output.stderr).trim().to_string()),
        },
        Err(err) => Outcome::Failed(err.to_string()),
    }
}

/// Closes the window in `slot`, if any; its `run` returns `Closed`.
pub fn close(slot: &Slot) {
    if let Some(mut child) = lock(slot).take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn lock(slot: &Slot) -> std::sync::MutexGuard<'_, Option<Child>> {
    slot.lock().unwrap_or_else(|p| p.into_inner())
}
