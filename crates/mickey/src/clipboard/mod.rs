//! The system clipboard (PRD 0011): what is copied on this PC, and putting
//! something there. Wayland through the data-control protocol (Hyprland,
//! Sway, KDE), X11 otherwise, and Windows, all through `arboard`.
//!
//! What is copied may be a password: it is never printed or logged, only
//! its kind and size.

pub mod history;
pub mod received;
pub mod sync;
pub mod watch;
pub mod wire;

use std::fmt;
use std::path::PathBuf;

#[derive(Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// Pixels row by row, 4 bytes each (red, green, blue, alpha).
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
    /// Files copied in a file manager, by their paths on this PC.
    Files(Vec<PathBuf>),
}

/// Only the kind and size: the content itself may be secret.
impl fmt::Debug for Content {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Content::Text(text) => write!(f, "text, {} bytes", text.len()),
            Content::Image { width, height, .. } => write!(f, "image, {width}x{height}"),
            Content::Files(paths) => write!(f, "{} file(s)", paths.len()),
        }
    }
}

/// While this lives, it can read and write the clipboard. On Wayland and
/// X11 what we write stays available only while this lives: the clipboard
/// asks its owner for the data when someone pastes.
pub struct Clipboard(arboard::Clipboard);

impl Clipboard {
    pub fn open() -> Result<Clipboard, String> {
        arboard::Clipboard::new()
            .map(Clipboard)
            .map_err(|err| format!("cannot reach the clipboard: {err}"))
    }

    /// What is copied now: files if a file manager copied some (it also
    /// offers their paths as text), else text, else an image, else nothing.
    pub fn read(&mut self) -> Option<Content> {
        if let Ok(paths) = self.0.get().file_list()
            && !paths.is_empty()
        {
            return Some(Content::Files(paths));
        }
        if let Some(text) = text(&mut self.0) {
            return Some(Content::Text(text));
        }
        let image = self.0.get_image().ok()?;
        Some(Content::Image {
            width: image.width,
            height: image.height,
            rgba: image.bytes.into_owned(),
        })
    }

    pub fn write(&mut self, content: &Content) -> Result<(), String> {
        let done = match content {
            Content::Text(text) => self.0.set_text(text.as_str()),
            Content::Files(paths) => self.0.set().file_list(paths),
            Content::Image {
                width,
                height,
                rgba,
            } => self.0.set_image(arboard::ImageData {
                width: *width,
                height: *height,
                bytes: rgba.as_slice().into(),
            }),
        };
        done.map_err(|err| format!("cannot write the clipboard: {err}"))
    }
}

/// The one clipboard of this process, on its own thread. What Mickey
/// writes stays pasteable on Wayland and X11 only while its owner lives,
/// so the owner never goes away; the rest of Mickey asks it.
#[derive(Clone)]
pub struct Keeper(std::sync::mpsc::Sender<Job>);

enum Job {
    Read(std::sync::mpsc::Sender<Option<Content>>),
    ReadText(std::sync::mpsc::Sender<Option<String>>),
    Write(Content),
}

impl Keeper {
    pub fn start() -> Result<Keeper, String> {
        let mut board = Clipboard::open()?;
        let (jobs, todo) = std::sync::mpsc::channel();
        crate::threads::spawn("clipboard", move || {
            for job in todo {
                match job {
                    Job::Read(answer) => {
                        let _ = answer.send(board.read());
                    }
                    Job::ReadText(answer) => {
                        let _ = answer.send(text(&mut board.0));
                    }
                    Job::Write(content) => {
                        if let Err(err) = board.write(&content) {
                            eprintln!("mickey: {err}");
                        }
                    }
                }
            }
        });
        Ok(Keeper(jobs))
    }

    /// What is copied now; `None` also if the keeper is gone.
    pub fn read(&self) -> Option<Content> {
        let (answer, wait) = std::sync::mpsc::channel();
        self.0.send(Job::Read(answer)).ok()?;
        wait.recv().ok().flatten()
    }

    /// Only the text, if any: cheap, never an image.
    pub fn read_text(&self) -> Option<String> {
        let (answer, wait) = std::sync::mpsc::channel();
        self.0.send(Job::ReadText(answer)).ok()?;
        wait.recv().ok().flatten()
    }

    pub fn write(&self, content: Content) {
        let _ = self.0.send(Job::Write(content));
    }
}

/// The copied text: from GPaste on GNOME Wayland when it runs, else
/// through `arboard`.
fn text(board: &mut arboard::Clipboard) -> Option<String> {
    #[cfg(target_os = "linux")]
    if let Some(text) = gpaste::latest() {
        return Some(text);
    }
    board.get_text().ok()
}

/// GNOME on Wayland lets no program read the clipboard in the background:
/// `arboard` only sees what X11 apps copy. GPaste, with its GNOME Shell
/// extension, does follow it, and hands out its latest copy over D-Bus
/// (#258). Asked only on GNOME Wayland, and only while GPaste runs (never
/// started by asking).
#[cfg(target_os = "linux")]
mod gpaste {
    use std::sync::OnceLock;
    use zbus::blocking::{Connection, fdo::DBusProxy};

    const NAME: &str = "org.gnome.GPaste";

    /// The session bus, opened once, on GNOME Wayland only.
    fn bus() -> Option<&'static Connection> {
        static BUS: OnceLock<Option<Connection>> = OnceLock::new();
        BUS.get_or_init(|| {
            let gnome = std::env::var("XDG_CURRENT_DESKTOP")
                .is_ok_and(|desktop| desktop.split(':').any(|part| part == "GNOME"));
            let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
            (gnome && wayland).then(Connection::session)?.ok()
        })
        .as_ref()
    }

    /// GPaste's newest copy, if GPaste runs and has one.
    pub fn latest() -> Option<String> {
        let bus = bus()?;
        let running = DBusProxy::new(bus)
            .ok()?
            .name_has_owner(NAME.try_into().ok()?)
            .ok()?;
        if !running {
            return None;
        }
        let reply = bus
            .call_method(
                Some(NAME),
                "/org/gnome/GPaste",
                Some("org.gnome.GPaste2"),
                "GetElementAtIndex",
                &(0u64,),
            )
            .ok()?;
        // (uuid, text)
        let (_, text): (String, String) = reply.body().deserialize().ok()?;
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    /// Run by hand on GNOME Wayland with GPaste: prints its newest copy.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a GNOME Wayland session with GPaste"]
    fn gpaste_hands_out_its_newest_copy() {
        println!("gpaste: {:?}", super::gpaste::latest());
    }

    use super::*;

    #[test]
    fn debug_never_shows_the_content() {
        let secret = Content::Text("hunter2".into());
        assert_eq!(format!("{secret:?}"), "text, 7 bytes");
        let image = Content::Image {
            width: 2,
            height: 1,
            rgba: vec![0; 8],
        };
        assert_eq!(format!("{image:?}"), "image, 2x1");
    }
}
