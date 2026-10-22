//! The tray icon (PRD 0009): the rat with a green, red or grey dot, a
//! tooltip naming the PC in control, and a menu to move the keyboard, open
//! the settings or quit. On Linux it is a StatusNotifierItem (Waybar's
//! tray, KDE, GNOME with the AppIndicator extension), through `ksni`.

use crate::control::Status;
#[cfg(target_os = "linux")]
use crate::tray_picture;
use crate::tray_picture::Dot;
use std::sync::mpsc::Sender;

/// What the owner picked in the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Click {
    /// Bring the keyboard and mouse to this PC.
    Home,
    /// Send them to this peer.
    Go(usize),
    Settings,
    Quit,
}

/// The dot and the tooltip for a status.
pub fn looks(status: &Status) -> (Dot, String) {
    if !status.peers.iter().any(|(_, online)| *online) {
        return (Dot::Alone, "No other PC connected".into());
    }
    if status.here {
        return (Dot::Here, "In control: this PC".into());
    }
    let there = status
        .focus
        .map_or("another PC", |peer| status.peers[peer].0.as_str());
    (Dot::Elsewhere, format!("In control: {there}"))
}

#[cfg(target_os = "linux")]
struct Icon {
    status: Status,
    clicks: Sender<Click>,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for Icon {
    fn id(&self) -> String {
        "mickey".into()
    }

    fn title(&self) -> String {
        "Mickey".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let (dot, _) = looks(&self.status);
        let rgba = tray_picture::picture(dot, false);
        // The protocol wants ARGB, byte by byte.
        let (pixels, _) = rgba.as_chunks::<4>();
        let argb = pixels
            .iter()
            .flat_map(|&[r, g, b, a]| [a, r, g, b])
            .collect();
        let size = tray_picture::SIZE as i32;
        vec![ksni::Icon {
            width: size,
            height: size,
            data: argb,
        }]
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Mickey".into(),
            description: looks(&self.status).1,
            ..Default::default()
        }
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        let item = |label: String, enabled: bool, click: Click| {
            StandardItem {
                label,
                enabled,
                // Only a send: the menu must not wait on Mickey's loop.
                activate: Box::new(move |icon: &mut Icon| {
                    let _ = icon.clicks.send(click);
                }),
                ..Default::default()
            }
            .into()
        };
        let mut menu = vec![item("This PC".into(), true, Click::Home)];
        for (index, (name, online)) in self.status.peers.iter().enumerate() {
            menu.push(item(name.clone(), *online, Click::Go(index)));
        }
        menu.push(ksni::MenuItem::Separator);
        menu.push(item("Settings".into(), true, Click::Settings));
        menu.push(item("Quit Mickey".into(), true, Click::Quit));
        menu
    }
}

/// The icon in the tray, while this lives.
#[cfg(target_os = "linux")]
pub struct Tray(ksni::blocking::Handle<Icon>);

#[cfg(target_os = "linux")]
impl Tray {
    /// Shows the icon; picks in its menu arrive on `clicks`. Fails when the
    /// desktop has no tray.
    pub fn start(status: Status, clicks: Sender<Click>) -> Result<Tray, String> {
        use ksni::blocking::TrayMethods;
        Icon { status, clicks }
            .spawn()
            .map(Tray)
            .map_err(|err| format!("no tray to show the icon in: {err}"))
    }

    pub fn show(&self, status: &Status) {
        self.0.update(|icon| icon.status = status.clone());
    }
}

// ponytail: the Windows tray is the next ticket (#111).
#[cfg(not(target_os = "linux"))]
pub struct Tray;

#[cfg(not(target_os = "linux"))]
impl Tray {
    pub fn start(_status: Status, _clicks: Sender<Click>) -> Result<Tray, String> {
        Err("no tray icon on this system yet".into())
    }

    pub fn show(&self, _status: &Status) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(focus: Option<usize>, here: bool, online: bool) -> Status {
        Status {
            focus,
            here,
            peers: vec![("Windows".into(), online)],
        }
    }

    #[test]
    fn the_dot_follows_the_keyboard() {
        assert_eq!(looks(&status(None, true, true)).0, Dot::Here);
        assert_eq!(
            looks(&status(Some(0), false, true)),
            (Dot::Elsewhere, "In control: Windows".into())
        );
        // Another PC's keyboard left for a third PC.
        assert_eq!(
            looks(&status(None, false, true)).1,
            "In control: another PC"
        );
        assert_eq!(looks(&status(None, true, false)).0, Dot::Alone);
    }
}
