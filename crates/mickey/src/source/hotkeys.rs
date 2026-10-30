//! Hotkeys another PC saved and shared with this one: offered first, and
//! taken only once the person here says yes (#349).

use super::Wake;
use mickey_config as config;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

/// Whether a "use these hotkeys?" question is on screen.
static ASKING: AtomicBool = AtomicBool::new(false);

/// What to do with hotkeys another PC shared.
#[derive(Debug, PartialEq)]
enum Offer {
    /// `[this_pc] share_hotkeys = false`: keep this PC's.
    Off,
    /// They are this PC's already.
    Same,
    Unreadable(String),
    /// Ask, showing what would change.
    Ask(Vec<String>),
}

fn weigh(config: &config::Config, lines: &str) -> Offer {
    if !config.this_pc.share_hotkeys {
        return Offer::Off;
    }
    match config::Hotkeys::from_lines(lines) {
        Ok(theirs) => match config.hotkeys.changes(&theirs) {
            changes if changes.is_empty() => Offer::Same,
            changes => Offer::Ask(changes),
        },
        Err(err) => Offer::Unreadable(err),
    }
}

/// Another PC, `who`, shared its hotkeys (`lines`). Nothing happens with
/// `[this_pc] share_hotkeys = false`, or when they are this PC's already;
/// otherwise the person here is asked what would change, one question at
/// a time, on its own thread. A yes comes back as `Wake::HotkeysAccepted`;
/// anything else keeps this PC's hotkeys.
pub(super) fn offer_hotkeys(lines: String, who: &str, wake: &Sender<Wake>) {
    let config = match config::Config::load() {
        Ok(config) => config.unwrap_or_else(config::Config::empty),
        Err(err) => {
            eprintln!("mickey: {who} shared its hotkeys, but config.toml cannot be read: {err}");
            return;
        }
    };
    let changes = match weigh(&config, &lines) {
        Offer::Off => {
            println!(
                "mickey: {who} shared its hotkeys; this PC keeps its own (share_hotkeys = false)"
            );
            return;
        }
        Offer::Same => return,
        Offer::Unreadable(err) => {
            eprintln!("mickey: {who} shared hotkeys this PC cannot read: {err}");
            return;
        }
        Offer::Ask(changes) => changes,
    };
    if ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let (who, wake) = (who.to_string(), wake.clone());
    crate::threads::spawn("ask hotkeys", move || {
        let answer = crate::window::ui_command(&["hotkeys", &who, &changes.join("\n")])
            .and_then(|mut ui| ui.status());
        match answer.map(|status| status.code()) {
            Ok(Some(0)) => {
                let _ = wake.send(Wake::HotkeysAccepted(lines));
            }
            Ok(Some(1)) => println!("mickey: kept this PC's hotkeys; {who}'s were not taken"),
            _ => eprintln!(
                "mickey: cannot ask whether to take {who}'s hotkeys; this PC keeps its own"
            ),
        }
        ASKING.store(false, Ordering::SeqCst);
    });
}

/// Writes `lines`, another PC's hotkeys, into config.toml and returns the
/// config as it is now. Nothing is written unless the new file still reads
/// as valid hotkeys.
pub(super) fn take_hotkeys(lines: &str) -> Result<config::Config, String> {
    let hotkeys = config::Hotkeys::from_lines(lines)?;
    let path = config::path()?;
    let text = std::fs::read_to_string(&path).map_err(|err| err.to_string())?;
    let new = config::set_hotkeys(&text, &hotkeys)?;
    config::parse(&new)?.hotkeys()?;
    config::write(&path, &new)?;
    config::Config::load()?.ok_or_else(|| "config.toml is gone".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_hotkeys_are_asked_about_only_when_they_change_something() {
        let mine = config::Config::empty();
        assert_eq!(weigh(&mine, "next = \"AltGr+RightShift\""), Offer::Same);
        assert_eq!(
            weigh(&mine, "next = \"LeftCtrl+F12\""),
            Offer::Ask(vec![
                "Next PC / switcher: LeftCtrl+F12 (now AltGr+RightShift)".into()
            ])
        );
        assert!(matches!(weigh(&mine, "next = 12"), Offer::Unreadable(_)));
        let keeps_its_own = config::parse("[this_pc]\nshare_hotkeys = false\n").unwrap();
        assert_eq!(weigh(&keeps_its_own, "next = \"LeftCtrl+F12\""), Offer::Off);
    }
}
