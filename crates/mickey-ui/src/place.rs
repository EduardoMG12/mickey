//! Where the switcher appears (PRD 0007): a corner or the center.
//!
//! On Windows and X11 a program may place its own window. On Wayland only
//! the compositor may, so Mickey asks it: on Hyprland with a window rule
//! for its app id, on Plasma with a KWin script (#218). Other Wayland
//! desktops, GNOME among them, place it themselves (usually in the center).

use std::process::Command;

/// Distance from the screen edges, in logical pixels.
const MARGIN: f32 = 24.0;
/// Room for a bar at the top (Waybar) or the taskbar at the bottom.
const BAR: f32 = 48.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub fn parse(text: &str) -> Option<Corner> {
        Some(match text {
            "center" => Corner::Center,
            "top-left" => Corner::TopLeft,
            "top-right" => Corner::TopRight,
            "bottom-left" => Corner::BottomLeft,
            "bottom-right" => Corner::BottomRight,
            _ => return None,
        })
    }

    /// The window's top-left point on a screen of `screen` size.
    pub fn position(self, screen: [f32; 2], window: [f32; 2]) -> [f32; 2] {
        let left = MARGIN;
        let right = screen[0] - window[0] - MARGIN;
        let top = BAR;
        let bottom = screen[1] - window[1] - BAR;
        match self {
            Corner::Center => [(screen[0] - window[0]) / 2.0, (screen[1] - window[1]) / 2.0],
            Corner::TopLeft => [left, top],
            Corner::TopRight => [right, top],
            Corner::BottomLeft => [left, bottom],
            Corner::BottomRight => [right, bottom],
        }
    }

    /// A KWin script (Plasma 6) that places windows with this app id in the
    /// corner of their screen's usable area (panels left out) when they
    /// appear, keeps them above the rest, takes their title bar away and
    /// keeps them out of the taskbar and Alt+Tab (Wayland ignores the
    /// window's own wish for that).
    pub fn kwin_script(self, app_id: &str) -> String {
        let (x, y) = match self {
            Corner::Center => (
                "a.x + (a.width - g.width) / 2".to_string(),
                "a.y + (a.height - g.height) / 2".to_string(),
            ),
            corner => {
                let x = match corner {
                    Corner::TopLeft | Corner::BottomLeft => format!("a.x + {MARGIN}"),
                    _ => format!("a.x + a.width - g.width - {MARGIN}"),
                };
                let y = match corner {
                    Corner::TopLeft | Corner::TopRight => format!("a.y + {MARGIN}"),
                    _ => format!("a.y + a.height - g.height - {MARGIN}"),
                };
                (x, y)
            }
        };
        format!(
            "function place(w) {{\n\
             \x20   if (w.resourceClass != \"{app_id}\") return;\n\
             \x20   const a = workspace.clientArea(KWin.MaximizeArea, w);\n\
             \x20   w.noBorder = true;\n\
             \x20   w.keepAbove = true;\n\
             \x20   w.skipTaskbar = true;\n\
             \x20   w.skipSwitcher = true;\n\
             \x20   const g = w.frameGeometry;\n\
             \x20   w.frameGeometry = {{ x: {x}, y: {y}, width: g.width, height: g.height }};\n\
             }}\n\
             workspace.windowAdded.connect(place);\n\
             workspace.windowList().forEach(place);\n"
        )
    }

    /// A Hyprland (0.50+, Lua config) window rule that floats, pins and
    /// places windows with this app id. Adding a rule with the same name
    /// replaces the previous one.
    pub fn hyprland_rule(self, app_id: &str) -> String {
        let place = match self {
            Corner::Center => "center = true".to_string(),
            corner => {
                let x = match corner {
                    Corner::TopLeft | Corner::BottomLeft => format!("({MARGIN})"),
                    _ => format!("(monitor_w-window_w-{MARGIN})"),
                };
                let y = match corner {
                    Corner::TopLeft | Corner::TopRight => format!("({BAR})"),
                    _ => format!("(monitor_h-window_h-{BAR})"),
                };
                format!("move = \"{x} {y}\"")
            }
        };
        format!(
            "hl.window_rule({{ name = \"{app_id}\", match = {{ class = \"^({app_id})$\" }}, \
             float = true, pin = true, {place} }})"
        )
    }
}

/// Asks the compositor to place the window, where that is the only way.
/// Returns `true` if it did, so the window should not try itself.
pub fn ask_compositor(corner: Corner, app_id: &str) -> bool {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Command::new("hyprctl")
            .args(["eval", &corner.hyprland_rule(app_id)])
            .output()
            .is_ok_and(|output| output.status.success());
    }
    let plasma = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| desktop.contains("KDE"));
    if plasma && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return kwin_place(&corner.kwin_script(app_id)).is_some();
    }
    false
}

/// Loads `script` into KWin (Plasma 6) through D-Bus and runs it, in place
/// of the one an earlier switcher left: a script stays loaded until
/// replaced, so it places every later window with that app id too.
fn kwin_place(script: &str) -> Option<()> {
    const NAME: &str = "mickey-switcher";
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, Into::into);
    let path = std::path::Path::new(&dir).join("mickey-switcher.js");
    std::fs::write(&path, script).ok()?;
    let call = |object: &str, method: &str, args: &[&str]| {
        Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.kde.KWin",
                "--object-path",
                object,
                "--method",
                method,
            ])
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let _ = call("/Scripting", "org.kde.kwin.Scripting.unloadScript", &[NAME]);
    let path = path.to_str()?;
    // Answers "(7,)": the new script's number, -1 if it was refused.
    let answer = call(
        "/Scripting",
        "org.kde.kwin.Scripting.loadScript",
        &[path, NAME],
    )?;
    let id: i32 = answer
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(",)")
        .parse()
        .ok()?;
    if id < 0 {
        return None;
    }
    call(
        &format!("/Scripting/Script{id}"),
        "org.kde.kwin.Script.run",
        &[],
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kwin_script_places_only_mickeys_window() {
        let script = Corner::BottomRight.kwin_script("mickey-ui");
        assert!(script.contains("w.resourceClass != \"mickey-ui\""));
        assert!(script.contains("x: a.x + a.width - g.width - 24"));
        assert!(script.contains("y: a.y + a.height - g.height - 24"));
        assert!(script.contains("w.noBorder = true"));
        assert!(script.contains("w.skipTaskbar = true"));
        assert!(
            Corner::Center
                .kwin_script("x")
                .contains("(a.width - g.width) / 2")
        );
    }

    #[test]
    fn corners_keep_their_margins() {
        let screen = [1536.0, 864.0];
        let window = [380.0, 200.0];
        assert_eq!(
            Corner::BottomRight.position(screen, window),
            [1132.0, 616.0]
        );
        assert_eq!(Corner::TopLeft.position(screen, window), [24.0, 48.0]);
        assert_eq!(Corner::Center.position(screen, window), [578.0, 332.0]);
    }

    #[test]
    fn names_parse_and_unknown_ones_do_not() {
        assert_eq!(Corner::parse("bottom-right"), Some(Corner::BottomRight));
        assert_eq!(Corner::parse("middle"), None);
    }

    #[test]
    fn the_hyprland_rule_uses_the_window_size() {
        let rule = Corner::BottomRight.hyprland_rule("mickey-ui");
        assert!(
            rule.contains(r#"move = "(monitor_w-window_w-24) (monitor_h-window_h-48)""#),
            "{rule}"
        );
        assert!(
            Corner::Center
                .hyprland_rule("mickey-ui")
                .contains("center = true")
        );
    }
}
