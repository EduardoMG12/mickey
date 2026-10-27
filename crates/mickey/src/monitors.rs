//! This PC's monitors (PRD 0005): where each one sits and its logical size,
//! the space the cursor moves in. Windows asks the system; Linux asks
//! Hyprland over its IPC socket (other desktops come later).

pub use mickey_config::peers::Monitor;

pub fn this_pc() -> Result<Vec<Monitor>, String> {
    #[cfg(windows)]
    return crate::platform::windows::monitors()
        .map_err(|err| format!("cannot list monitors: {err}"));
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return hyprland::monitors();
        }
        // Elsewhere (GNOME, KDE): the RemoteDesktop portal (PRD 0004).
        let monitors = crate::portal::get().map(crate::portal::Portal::monitors);
        monitors
            .filter(|monitors| !monitors.is_empty())
            .ok_or_else(|| {
                "no Hyprland, and the RemoteDesktop portal named no monitors".to_string()
            })
    }
}

#[cfg(target_os = "linux")]
pub mod hyprland {
    use super::Monitor;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    /// Hyprland's request socket for this session.
    pub fn socket() -> Result<PathBuf, String> {
        let signature = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE")
            .ok_or("not running under Hyprland, the only Linux desktop supported for now")?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
        Ok(PathBuf::from(runtime)
            .join("hypr")
            .join(signature)
            .join(".socket.sock"))
    }

    /// One request, the whole answer (Hyprland closes after answering).
    pub fn ask(request: &str) -> Result<String, String> {
        let mut stream = UnixStream::connect(socket()?)
            .map_err(|err| format!("cannot reach Hyprland: {err}"))?;
        stream
            .write_all(request.as_bytes())
            .map_err(|err| err.to_string())?;
        let mut answer = String::new();
        stream
            .read_to_string(&mut answer)
            .map_err(|err| err.to_string())?;
        Ok(answer)
    }

    /// Where the cursor is, in the same coordinates as `monitors`.
    pub fn cursor() -> Result<(i32, i32), String> {
        let answer = ask("cursorpos")?;
        answer
            .trim()
            .split_once(", ")
            .and_then(|(x, y)| Some((x.parse().ok()?, y.parse().ok()?)))
            .ok_or_else(|| format!("Hyprland answered `{answer}`"))
    }

    pub fn monitors() -> Result<Vec<Monitor>, String> {
        parse(&ask("monitors")?)
    }

    /// Reads the plain-text `monitors` answer: per monitor a line
    /// `<w>x<h>@<rate> at <x>x<y>` and a line `scale: <s>`, plus
    /// `transform` (odd values turn it sideways) and `disabled`.
    pub fn parse(text: &str) -> Result<Vec<Monitor>, String> {
        let mut monitors = Vec::new();
        for block in text.split("Monitor ").skip(1) {
            let field = |name: &str| {
                block
                    .lines()
                    .find_map(|line| line.trim().strip_prefix(name).map(str::trim))
            };
            if field("disabled:") == Some("true") {
                continue;
            }
            let geometry = block
                .lines()
                .nth(1)
                .map(str::trim)
                .ok_or("a monitor has no size line")?;
            let (mode, position) = geometry.split_once(" at ").ok_or("bad size line")?;
            let (size, _rate) = mode.split_once('@').unwrap_or((mode, ""));
            let pair = |text: &str| -> Option<(f64, f64)> {
                let (a, b) = text.split_once('x')?;
                Some((a.parse().ok()?, b.parse().ok()?))
            };
            let (w, h) = pair(size).ok_or_else(|| format!("bad size `{size}`"))?;
            let (x, y) = pair(position).ok_or_else(|| format!("bad position `{position}`"))?;
            let scale: f64 = field("scale:").and_then(|s| s.parse().ok()).unwrap_or(1.0);
            let sideways = field("transform:")
                .and_then(|t| t.parse::<u8>().ok())
                .is_some_and(|t| t % 2 == 1);
            let (w, h) = if sideways { (h, w) } else { (w, h) };
            monitors.push(Monitor {
                x: x as i32,
                y: y as i32,
                width: (w / scale).round() as i32,
                height: (h / scale).round() as i32,
            });
        }
        if monitors.is_empty() {
            return Err("Hyprland reported no monitors".into());
        }
        Ok(monitors)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const LAPTOP: &str = "Monitor eDP-1 (ID 0):\n\t1920x1080@165.00000 at 0x0\n\tdescription: BOE\n\tscale: 1.25\n\ttransform: 0\n\tdisabled: false\n\n";

        #[test]
        fn the_owners_laptop_is_1536_by_864() {
            assert_eq!(
                parse(LAPTOP).unwrap(),
                [Monitor {
                    x: 0,
                    y: 0,
                    width: 1536,
                    height: 864
                }]
            );
        }

        #[test]
        fn two_monitors_one_sideways_one_disabled() {
            let text = format!(
                "{LAPTOP}Monitor HDMI-A-1 (ID 1):\n\t2560x1440@60.00000 at 1536x0\n\tscale: 1.00\n\ttransform: 1\n\tdisabled: false\n\nMonitor DP-2 (ID 2):\n\t1920x1080@60.00000 at 4000x0\n\tscale: 1.00\n\tdisabled: true\n"
            );
            assert_eq!(
                parse(&text).unwrap()[1],
                Monitor {
                    x: 1536,
                    y: 0,
                    width: 1440,
                    height: 2560
                }
            );
            assert_eq!(parse(&text).unwrap().len(), 2);
        }

        #[test]
        fn nonsense_is_an_error() {
            assert!(parse("").is_err());
            assert!(parse("Monitor X (ID 0):\n\tgarbage\n").is_err());
        }
    }
}
