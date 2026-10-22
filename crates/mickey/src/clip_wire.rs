//! What crosses the data link for the clipboard (PRD 0011): one bulk
//! payload per message, a kind byte and its body.
//!
//! ```text
//! 0  Ask      send me what is copied on your PC   (no body)
//! 1  Text     UTF-8
//! 2  Image    PNG, 8-bit RGBA
//! 3  Nothing  nothing Mickey carries is copied     (no body)
//! ```
//!
//! Images cross as PNG: a screenshot's raw pixels are several times larger.

use crate::clipboard::Content;
use std::io::Cursor;

const ASK: u8 = 0;
const TEXT: u8 = 1;
const IMAGE: u8 = 2;
const NOTHING: u8 = 3;
/// The most pixel bytes a received PNG may unpack to (an 8K screenshot is
/// about 130 MB): a small PNG must not make us allocate gigabytes.
const MAX_PIXELS: usize = 256 << 20;

#[derive(Debug, PartialEq, Eq)]
pub enum Wire {
    Ask,
    Content(Content),
    Nothing,
}

pub fn encode(wire: &Wire) -> Result<Vec<u8>, String> {
    Ok(match wire {
        Wire::Ask => vec![ASK],
        Wire::Nothing => vec![NOTHING],
        Wire::Content(Content::Text(text)) => [&[TEXT], text.as_bytes()].concat(),
        Wire::Content(Content::Image {
            width,
            height,
            rgba,
        }) => {
            let mut out = vec![IMAGE];
            let (width, height) = (u32::try_from(*width), u32::try_from(*height));
            let (Ok(width), Ok(height)) = (width, height) else {
                return Err("the image is too large to send".into());
            };
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            // Fast: the time to compress is spent on every crossing.
            encoder.set_compression(png::Compression::Fast);
            let mut writer = encoder.write_header().map_err(|err| err.to_string())?;
            writer
                .write_image_data(rgba)
                .map_err(|err| err.to_string())?;
            writer.finish().map_err(|err| err.to_string())?;
            out
        }
    })
}

/// Anything from the network may be garbage: an error, never a panic.
pub fn decode(bytes: &[u8]) -> Result<Wire, String> {
    let (&kind, body) = bytes.split_first().ok_or("an empty clipboard message")?;
    match kind {
        ASK if body.is_empty() => Ok(Wire::Ask),
        NOTHING if body.is_empty() => Ok(Wire::Nothing),
        TEXT => String::from_utf8(body.to_vec())
            .map(|text| Wire::Content(Content::Text(text)))
            .map_err(|_| "clipboard text that is not UTF-8".to_string()),
        IMAGE => image(body).map(Wire::Content),
        _ => Err(format!("unknown clipboard message {kind}")),
    }
}

fn image(png_bytes: &[u8]) -> Result<Content, String> {
    let limits = png::Limits { bytes: MAX_PIXELS };
    let decoder = png::Decoder::new_with_limits(Cursor::new(png_bytes), limits);
    let mut reader = decoder.read_info().map_err(|err| err.to_string())?;
    let info = reader.info();
    if (info.color_type, info.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
        return Err("a clipboard image must be 8-bit RGBA".into());
    }
    let (width, height) = (info.width as usize, info.height as usize);
    let size = reader
        .output_buffer_size()
        .ok_or("the image is too large")?;
    let mut rgba = vec![0; size];
    reader
        .next_frame(&mut rgba)
        .map_err(|err| err.to_string())?;
    rgba.truncate(width * height * 4);
    Ok(Content::Image {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(wire: Wire) {
        assert_eq!(decode(&encode(&wire).unwrap()), Ok(wire));
    }

    #[test]
    fn every_message_round_trips() {
        round_trip(Wire::Ask);
        round_trip(Wire::Nothing);
        round_trip(Wire::Content(Content::Text("olá, Mickey".into())));
        round_trip(Wire::Content(Content::Text(String::new())));
        let rgba: Vec<u8> = (0..3 * 2 * 4).map(|i| i as u8 * 9).collect();
        round_trip(Wire::Content(Content::Image {
            width: 3,
            height: 2,
            rgba,
        }));
    }

    #[test]
    fn a_screenshot_shrinks_as_png() {
        // Mostly one colour, like most screenshots.
        let (width, height) = (1920, 1080);
        let rgba = vec![40u8; width * height * 4];
        let sent = encode(&Wire::Content(Content::Image {
            width,
            height,
            rgba,
        }))
        .unwrap();
        assert!(sent.len() < width * height * 4 / 50, "{} bytes", sent.len());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode(&[]).is_err());
        assert!(decode(&[ASK, 1]).is_err());
        assert!(decode(&[TEXT, 0xff]).is_err());
        assert!(decode(&[IMAGE, 1, 2, 3]).is_err());
        assert!(decode(&[9]).is_err());
    }
}
