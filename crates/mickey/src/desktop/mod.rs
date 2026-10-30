//! Which Linux desktop this session runs: each answers for the cursor and
//! the monitors its own way (Hyprland and Sway over their IPC sockets, the
//! X server, or the portals on GNOME and KDE, PRD 0004).

pub mod hyprland;
pub mod portal;
pub mod sway;
pub mod x11;

use std::ffi::OsString;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Session {
    Hyprland,
    X11,
    Sway,
    /// GNOME or KDE on Wayland: the RemoteDesktop and InputCapture portals.
    Portal,
}

/// This session, read once: the variables it comes from do not change
/// while Mickey runs.
pub fn session() -> Session {
    static SESSION: OnceLock<Session> = OnceLock::new();
    *SESSION.get_or_init(|| Session::detect(|name| std::env::var_os(name)))
}

impl Session {
    /// From the session's variables, in this order: Hyprland (whose
    /// XWayland sets `DISPLAY` too); X11, when `XDG_SESSION_TYPE` says so
    /// or there is a `DISPLAY` and no Wayland compositor; Sway
    /// (`SWAYSOCK`); else the portals.
    fn detect(var: impl Fn(&str) -> Option<OsString>) -> Session {
        if var("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
            return Session::Hyprland;
        }
        let kind = var("XDG_SESSION_TYPE").and_then(|kind| kind.into_string().ok());
        let x11 = match kind.as_deref() {
            Some("x11") => true,
            Some("wayland") => false,
            _ => var("DISPLAY").is_some() && var("WAYLAND_DISPLAY").is_none(),
        };
        if x11 {
            return Session::X11;
        }
        if var("SWAYSOCK").is_some() {
            return Session::Sway;
        }
        Session::Portal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(vars: &[(&str, &str)]) -> Session {
        Session::detect(|name| {
            vars.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
    }

    #[test]
    fn hyprland_wins_over_its_own_xwayland() {
        let vars = [
            ("HYPRLAND_INSTANCE_SIGNATURE", "abc"),
            ("DISPLAY", ":0"),
            ("XDG_SESSION_TYPE", "x11"),
        ];
        assert_eq!(detect(&vars), Session::Hyprland);
    }

    #[test]
    fn x11_from_the_session_type_or_a_display_alone() {
        assert_eq!(
            detect(&[
                ("XDG_SESSION_TYPE", "x11"),
                ("WAYLAND_DISPLAY", "wayland-0")
            ]),
            Session::X11
        );
        assert_eq!(detect(&[("DISPLAY", ":0")]), Session::X11);
        assert_eq!(
            detect(&[("DISPLAY", ":0"), ("WAYLAND_DISPLAY", "wayland-0")]),
            Session::Portal
        );
    }

    #[test]
    fn sway_then_the_portals() {
        let sway = [
            ("XDG_SESSION_TYPE", "wayland"),
            ("DISPLAY", ":0"),
            ("SWAYSOCK", "/run/sway.sock"),
        ];
        assert_eq!(detect(&sway), Session::Sway);
        assert_eq!(detect(&[("XDG_SESSION_TYPE", "wayland")]), Session::Portal);
        assert_eq!(detect(&[]), Session::Portal);
    }
}
