//! The system clipboard (PRD 0011): what is copied on this PC, and putting
//! something there. Wayland through the data-control protocol (Hyprland,
//! Sway, KDE), X11 otherwise, and Windows, all through `arboard`.
//!
//! What is copied may be a password: it is never printed or logged, only
//! its kind and size.

use std::fmt;

#[derive(Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// Pixels row by row, 4 bytes each (red, green, blue, alpha).
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}

/// Only the kind and size: the content itself may be secret.
impl fmt::Debug for Content {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Content::Text(text) => write!(f, "text, {} bytes", text.len()),
            Content::Image { width, height, .. } => write!(f, "image, {width}x{height}"),
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

    /// What is copied now: text if there is text, else an image, else
    /// nothing (empty, or something Mickey does not carry yet).
    pub fn read(&mut self) -> Option<Content> {
        if let Ok(text) = self.0.get_text() {
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

#[cfg(test)]
mod tests {
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
