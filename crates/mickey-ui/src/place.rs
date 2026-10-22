//! Where the switcher appears (PRD 0007): a corner or the center.
//!
//! On Windows and X11 a program may place its own window. On Wayland only
//! the compositor may, so Mickey asks it: on Hyprland with a window rule
//! for its app id. Other Wayland desktops place it themselves (usually in
//! the center) until they get their own branch here.

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
    false
}

#[cfg(test)]
mod tests {
    use super::*;

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
