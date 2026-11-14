//! What the owner chose for the touchpad in the desktop's own settings,
//! read (never written) so that taps and scrolling work on the PC being
//! controlled the way they do on this one.
//!
//! libinput settings live in the compositor, not in the device: GNOME
//! keeps them in GSettings, KDE in `kcminputrc`. Other desktops (sway,
//! Hyprland, Xfce...) keep them in their own files, which Mickey does not
//! read; there `[touchpad]` in config.toml says what to do.

use crate::input::touchpad::ButtonMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// What a desktop said; `None` for what it did not say.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Detected {
    pub tap: Option<bool>,
    pub tap_and_drag: Option<bool>,
    pub button_map: Option<ButtonMap>,
    pub natural_scroll: Option<bool>,
    pub two_finger_scroll: Option<bool>,
    pub disable_while_typing: Option<bool>,
    /// Where it was read, for the message at start-up.
    pub source: Option<&'static str>,
}

/// Reads the settings of the desktop this session runs. `device` is the
/// touchpad's name, which KDE keys its settings by.
pub fn detect(device: &str) -> Detected {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    let desktop = desktop.to_ascii_lowercase();
    let has = |name: &str| desktop.split(':').any(|part| part == name);
    // These share GNOME's setting names, under their own schema.
    let schema = if has("gnome") || has("unity") || has("budgie") || has("pantheon") {
        Some("org.gnome.desktop.peripherals.touchpad")
    } else if has("x-cinnamon") || has("cinnamon") {
        Some("org.cinnamon.desktop.peripherals.touchpad")
    } else {
        None
    };
    if let Some(schema) = schema {
        let text = run("gsettings", &["list-recursively", schema]);
        return Detected {
            source: Some("GSettings"),
            ..parse_gsettings(&text.unwrap_or_default())
        };
    }
    if has("kde") {
        let text = kcminputrc().and_then(|path| std::fs::read_to_string(path).ok());
        return Detected {
            source: Some("kcminputrc"),
            ..parse_kcminputrc(&text.unwrap_or_default(), device)
        };
    }
    Detected::default()
}

fn kcminputrc() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })?;
    Some(base.join("kcminputrc"))
}

/// Runs a program and returns what it printed, or `None` when it is not
/// installed, fails, or takes over a second (a session bus that does not
/// answer must not hold up the keyboard).
fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait().ok()? {
            Some(status) if status.success() => break,
            Some(_) => return None,
            None if started.elapsed() > Duration::from_secs(1) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            None => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
    Some(text)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// `gsettings list-recursively <schema>`: one `schema key value` per line.
/// `tap-button-map` is `'default'` (the device's own, left-right-middle
/// for libinput), `'lrm'` or `'lmr'`.
pub fn parse_gsettings(text: &str) -> Detected {
    let mut found = Detected::default();
    for line in text.lines() {
        let mut parts = line.splitn(3, ' ');
        let (Some(_schema), Some(key), Some(value)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        match key {
            "tap-to-click" => found.tap = parse_bool(value),
            "tap-and-drag" => found.tap_and_drag = parse_bool(value),
            "natural-scroll" => found.natural_scroll = parse_bool(value),
            "two-finger-scrolling-enabled" => found.two_finger_scroll = parse_bool(value),
            "disable-while-typing" => found.disable_while_typing = parse_bool(value),
            "tap-button-map" => {
                found.button_map = match value.trim().trim_matches('\'') {
                    "default" | "lrm" => Some(ButtonMap::Lrm),
                    "lmr" => Some(ButtonMap::Lmr),
                    _ => None,
                }
            }
            _ => {}
        }
    }
    found
}

/// KDE's `kcminputrc`: a group per device, `[Libinput][vendor][product][name]`.
/// The group of `device` is used; if none has its name and exactly one
/// belongs to a touchpad, that one. Keys the file does not have are not
/// guessed (their defaults depend on the Plasma version).
pub fn parse_kcminputrc(text: &str, device: &str) -> Detected {
    // (group name, its lines' key=value pairs)
    let mut groups: Vec<(String, Vec<(&str, &str)>)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            groups.push((line.to_string(), Vec::new()));
        } else if let Some((key, value)) = line.split_once('=')
            && let Some((_, pairs)) = groups.last_mut()
        {
            pairs.push((key.trim(), value.trim()));
        }
    }
    let libinput: Vec<&(String, Vec<(&str, &str)>)> = groups
        .iter()
        .filter(|(name, _)| name.starts_with("[Libinput]"))
        .collect();
    let named = libinput
        .iter()
        .find(|(name, _)| name.ends_with(&format!("[{device}]")));
    let touchpads: Vec<_> = libinput
        .iter()
        .filter(|(name, _)| name.to_ascii_lowercase().contains("touchpad"))
        .collect();
    let chosen = named.or(match touchpads[..] {
        [only] => Some(only),
        _ => None,
    });
    let Some((_, pairs)) = chosen else {
        return Detected::default();
    };
    let get = |wanted: &str| {
        pairs
            .iter()
            .find(|(key, _)| *key == wanted)
            .map(|(_, value)| *value)
    };
    Detected {
        tap: get("TapToClick").and_then(parse_bool),
        tap_and_drag: get("TapAndDrag").and_then(parse_bool),
        natural_scroll: get("NaturalScroll").and_then(parse_bool),
        disable_while_typing: get("DisableWhileTyping").and_then(parse_bool),
        // `LmrTapButtonMap=true`: two fingers middle, three right.
        button_map: get("LmrTapButtonMap")
            .and_then(parse_bool)
            .map(|lmr| if lmr { ButtonMap::Lmr } else { ButtonMap::Lrm }),
        two_finger_scroll: None,
        source: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gnome_settings_are_read_as_they_are() {
        let text = "\
org.gnome.desktop.peripherals.touchpad click-method 'fingers'
org.gnome.desktop.peripherals.touchpad natural-scroll true
org.gnome.desktop.peripherals.touchpad tap-to-click true
org.gnome.desktop.peripherals.touchpad tap-and-drag false
org.gnome.desktop.peripherals.touchpad tap-button-map 'lmr'
org.gnome.desktop.peripherals.touchpad two-finger-scrolling-enabled true
org.gnome.desktop.peripherals.touchpad disable-while-typing false
";
        let found = parse_gsettings(text);
        assert_eq!(found.tap, Some(true));
        assert_eq!(found.tap_and_drag, Some(false));
        assert_eq!(found.natural_scroll, Some(true));
        assert_eq!(found.button_map, Some(ButtonMap::Lmr));
        assert_eq!(found.two_finger_scroll, Some(true));
        assert_eq!(found.disable_while_typing, Some(false));
    }

    #[test]
    fn gnomes_default_button_map_is_left_right_middle() {
        let found = parse_gsettings("s tap-button-map 'default'\n");
        assert_eq!(found.button_map, Some(ButtonMap::Lrm));
        assert_eq!(parse_gsettings("nonsense\n\n"), Detected::default());
    }

    const KDE: &str = "\
[General]
x=1

[Libinput][1739][52620][SYNA2BA6:00 06CB:CD8C Touchpad]
NaturalScroll=true
TapToClick=true
DisableWhileTyping=false

[Libinput][1][2][Some Mouse]
NaturalScroll=false
TapToClick=false
";

    #[test]
    fn kde_settings_are_found_by_the_devices_name() {
        let found = parse_kcminputrc(KDE, "SYNA2BA6:00 06CB:CD8C Touchpad");
        assert_eq!(found.tap, Some(true));
        assert_eq!(found.natural_scroll, Some(true));
        assert_eq!(found.disable_while_typing, Some(false));
        assert_eq!(
            found.tap_and_drag, None,
            "what the file lacks is not guessed"
        );
        assert_eq!(found.button_map, None);
    }

    #[test]
    fn kde_falls_back_to_the_only_touchpad_but_never_to_a_mouse() {
        let found = parse_kcminputrc(KDE, "Another name");
        assert_eq!(found.tap, Some(true), "the one touchpad group");
        let mouse_only = "[Libinput][1][2][Some Mouse]\nTapToClick=true\n";
        assert_eq!(parse_kcminputrc(mouse_only, "Pad"), Detected::default());
        assert_eq!(parse_kcminputrc("", "Pad"), Detected::default());
        let two = format!("{KDE}\n[Libinput][3][4][Other Touchpad]\nTapToClick=false\n");
        assert_eq!(parse_kcminputrc(&two, "Neither"), Detected::default());
    }

    #[test]
    fn kdes_button_map_is_a_flag() {
        let text = "[Libinput][1][2][Pad Touchpad]\nLmrTapButtonMap=true\n";
        assert_eq!(
            parse_kcminputrc(text, "Pad Touchpad").button_map,
            Some(ButtonMap::Lmr)
        );
    }
}
