//! What crosses the data link for the clipboard (PRD 0011): one bulk
//! payload per message, a kind byte and its body.
//!
//! ```text
//! 0  Ask      send me what is copied on your PC   (no body)
//! 1  Text     UTF-8
//! 2  Image    PNG, 8-bit RGBA
//! 3  Nothing  nothing Mickey carries is copied     (no body)
//! 4  Files    count: u32, then per file name length: u16, name (UTF-8),
//!             size: u64, bytes
//! ```
//!
//! Files cross whole, at most `MAX_FILES` bytes together; folders are left
//! out for now.
//!
//! Images cross as PNG: a screenshot's raw pixels are several times larger.

use crate::clipboard::Content;
use std::io::Cursor;

const ASK: u8 = 0;
const TEXT: u8 = 1;
const IMAGE: u8 = 2;
const NOTHING: u8 = 3;
const FILES: u8 = 4;
/// Room for the names under the data link's 64 MB.
pub const MAX_FILES: u64 = 60 << 20;
/// The most files one copy may carry.
pub const MAX_FILE_COUNT: u32 = 1000;
/// The most pixel bytes a received PNG may unpack to (an 8K screenshot is
/// about 130 MB): a small PNG must not make us allocate gigabytes.
const MAX_PIXELS: usize = 256 << 20;

#[derive(Debug, PartialEq, Eq)]
pub enum Wire {
    Ask,
    Content(Content),
    Nothing,
    /// Files as received: names (checked safe) and bytes, not yet saved.
    Files(Vec<(String, Vec<u8>)>),
}

pub fn encode(wire: &Wire) -> Result<Vec<u8>, String> {
    Ok(match wire {
        Wire::Ask => vec![ASK],
        Wire::Nothing => vec![NOTHING],
        Wire::Content(Content::Text(text)) => [&[TEXT], text.as_bytes()].concat(),
        Wire::Content(Content::Files(paths)) => {
            let files = read_files(paths)?;
            encode(&Wire::Files(files))?
        }
        Wire::Files(files) => {
            let mut out = vec![FILES];
            out.extend_from_slice(&(files.len() as u32).to_be_bytes());
            for (name, bytes) in files {
                out.extend_from_slice(&(name.len() as u16).to_be_bytes());
                out.extend_from_slice(name.as_bytes());
                out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
                out.extend_from_slice(bytes);
            }
            out
        }
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
        FILES => files(body).map(Wire::Files),
        _ => Err(format!("unknown clipboard message {kind}")),
    }
}

/// The copied files' names and bytes; folders are skipped, and too much
/// in total is an error rather than a slow crossing.
fn read_files(paths: &[std::path::PathBuf]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut files = Vec::new();
    let mut total = 0;
    for path in paths.iter().filter(|path| path.is_file()) {
        total += path.metadata().map_err(|err| err.to_string())?.len();
        if total > MAX_FILES {
            return Err(format!("the copied files are over {} MB", MAX_FILES >> 20));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("{} has no usable name", path.display()))?;
        let bytes = std::fs::read(path).map_err(|err| format!("{}: {err}", path.display()))?;
        files.push((name.to_string(), bytes));
    }
    Ok(files)
}

fn files(mut body: &[u8]) -> Result<Vec<(String, Vec<u8>)>, String> {
    let short = || "a files message that ends too soon".to_string();
    let mut take = |len: usize| -> Result<&[u8], String> {
        let (head, rest) = body.split_at_checked(len).ok_or_else(short)?;
        body = rest;
        Ok(head)
    };
    let count = u32::from_be_bytes(take(4)?.try_into().map_err(|_| short())?);
    // Each file becomes a file on disk: millions of empty ones would fit
    // in MAX_FILES bytes.
    if count > MAX_FILE_COUNT {
        return Err(format!("{count} files at once, more than {MAX_FILE_COUNT}"));
    }
    let mut files = Vec::new();
    let mut total = 0u64;
    for _ in 0..count {
        let name_len = u16::from_be_bytes(take(2)?.try_into().map_err(|_| short())?);
        let name = std::str::from_utf8(take(name_len.into())?)
            .map_err(|_| "a file name that is not UTF-8")?;
        let name = crate::received::safe_name(name)
            .ok_or_else(|| format!("refused a file name from the network: {name:?}"))?
            .to_string();
        let size = u64::from_be_bytes(take(8)?.try_into().map_err(|_| short())?);
        // Sizes come from the network: their sum must not wrap around.
        total = total
            .checked_add(size)
            .ok_or("more file bytes than allowed")?;
        if total > MAX_FILES {
            return Err("more file bytes than allowed".into());
        }
        files.push((name, take(size as usize)?.to_vec()));
    }
    if !body.is_empty() {
        return Err("bytes after the last file".into());
    }
    Ok(files)
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
    // Checked before allocating: `Limits` only bounds what the decoder
    // keeps itself, not the frame buffer, and a few bytes of header can
    // claim 100000 x 100000 pixels (40 GB, and a failed allocation ends
    // the whole process).
    let size = reader
        .output_buffer_size()
        .filter(|&size| size <= MAX_PIXELS)
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
    #[test]
    fn too_many_files_at_once_are_refused() {
        let empty: Vec<(String, Vec<u8>)> = (0..=MAX_FILE_COUNT)
            .map(|n| (format!("{n}.txt"), Vec::new()))
            .collect();
        let bytes = encode(&Wire::Files(empty)).unwrap();
        assert!(decode(&bytes).is_err());
    }

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
    fn files_round_trip_and_bad_names_are_refused() {
        let files = vec![
            ("a.txt".to_string(), b"hello".to_vec()),
            ("vazio".to_string(), vec![]),
        ];
        round_trip(Wire::Files(files));
        let evil = encode(&Wire::Files(vec![("../evil".into(), vec![1])])).unwrap();
        assert!(decode(&evil).is_err());
        let mut short = encode(&Wire::Files(vec![("a".into(), vec![1, 2])])).unwrap();
        short.pop();
        assert!(decode(&short).is_err());
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode(&[]).is_err());
        assert!(decode(&[ASK, 1]).is_err());
        assert!(decode(&[TEXT, 0xff]).is_err());
        assert!(decode(&[IMAGE, 1, 2, 3]).is_err());
        assert!(decode(&[9]).is_err());
    }

    /// The PNG chunk checksum (CRC-32), to forge a header in a test.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn a_tiny_png_claiming_huge_pixels_is_refused_before_allocating() {
        let small = Content::Image {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        };
        let mut bytes = encode(&Wire::Content(small)).unwrap();
        // After the kind byte: the 8-byte signature, then IHDR's length and
        // type (8 bytes), its 13 data bytes and their checksum.
        let ihdr = 1 + 8 + 4;
        bytes[ihdr + 4..ihdr + 8].copy_from_slice(&100_000u32.to_be_bytes());
        bytes[ihdr + 8..ihdr + 12].copy_from_slice(&100_000u32.to_be_bytes());
        let crc = crc32(&bytes[ihdr..ihdr + 4 + 13]);
        bytes[ihdr + 17..ihdr + 21].copy_from_slice(&crc.to_be_bytes());
        // 100000 x 100000 RGBA is 40 GB: refused, not allocated.
        assert_eq!(decode(&bytes).unwrap_err(), "the image is too large");
    }

    #[test]
    fn file_sizes_that_would_wrap_around_are_refused() {
        let mut bytes = vec![FILES];
        bytes.extend(2u32.to_be_bytes());
        for (name, size) in [(b'a', 1), (b'b', u64::MAX)] {
            bytes.extend(1u16.to_be_bytes());
            bytes.push(name);
            bytes.extend(size.to_be_bytes());
        }
        bytes.push(0);
        assert!(decode(&bytes).is_err());
    }
}
