//! `mickey-ui`, the program that draws every window, and the windows Mickey
//! may have to close before they answer: the switcher closes when its
//! hotkey is pressed again (PRD 0007).

use crate::threads::LockAnyway;
use mickey_config as config;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The window open, if any, shared by the thread waiting for its answer and
/// whoever may close it.
pub type Slot = Arc<Mutex<Option<Child>>>;

/// `mickey-ui` with `args`. It is installed next to this program: the
/// windows live there, so the long-running mickey never links the GUI
/// (PRD 0003).
pub fn ui_command<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> std::io::Result<Command> {
    let exe = std::env::current_exe()?;
    let mut command =
        Command::new(exe.with_file_name(format!("mickey-ui{}", std::env::consts::EXE_SUFFIX)));
    command.args(args);
    Ok(command)
}

/// `mickey-ui` arguments for the switcher (a view-only copy if `mirror`):
/// this PC's corner and direction, then one card per PC.
pub fn switcher_args(cards: &[String], interface: &config::Interface, mirror: bool) -> Vec<String> {
    let mut args = vec!["switcher".to_string()];
    if mirror {
        args.push("--mirror".to_string());
    }
    args.extend(["--at".to_string(), interface.switcher_position.clone()]);
    if interface.switcher_vertical {
        args.push("--vertical".to_string());
    }
    args.extend(cards.iter().cloned());
    args
}

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
    let child = ui_command(args)
        .and_then(|mut ui| ui.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn());
    let child = match child {
        Ok(child) => child,
        Err(err) => return Outcome::Failed(format!("mickey-ui: {err}")),
    };
    close(slot);
    *slot.lock_anyway() = Some(child);
    // Polls without holding the slot, so `close` is never kept waiting.
    let child = loop {
        std::thread::sleep(Duration::from_millis(50));
        let mut slot = slot.lock_anyway();
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
    if let Some(mut child) = slot.lock_anyway().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switcher_arguments_put_this_pc_first_then_the_cards() {
        let interface = config::Interface {
            switcher_position: "top-left".into(),
            switcher_vertical: true,
            ..config::Interface::default()
        };
        let cards = ["1".to_string(), "online:Desk".to_string()];
        assert_eq!(
            switcher_args(&cards, &interface, true),
            [
                "switcher",
                "--mirror",
                "--at",
                "top-left",
                "--vertical",
                "1",
                "online:Desk"
            ]
        );
        let plain = config::Interface::default();
        assert_eq!(
            switcher_args(&cards, &plain, false)[..2],
            ["switcher", "--at"]
        );
    }
}
