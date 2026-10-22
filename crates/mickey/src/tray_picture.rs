//! The tray icon's picture (PRD 0009): the owner's line-art rat, white for
//! dark bars or black for light ones, with a dot in the bottom-right corner
//! saying where the keyboard and mouse are. `assets/icons/tray.sh` makes
//! the raw pictures, so Mickey needs no image crate.

/// Pictures are `SIZE` x `SIZE` pixels, 4 bytes each: red, green, blue,
/// alpha, row by row.
pub const SIZE: usize = 32;
const BYTES: usize = SIZE * SIZE * 4;

static WHITE: &[u8; BYTES] = include_bytes!("../assets/tray-white.rgba");
static BLACK: &[u8; BYTES] = include_bytes!("../assets/tray-black.rgba");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    /// The keyboard and mouse control this PC.
    Here,
    /// They control another PC.
    Elsewhere,
    /// No other PC is connected.
    Alone,
}

impl Dot {
    fn colour(self) -> [u8; 3] {
        match self {
            Dot::Here => [0x2e, 0xc4, 0x4a],
            Dot::Elsewhere => [0xe5, 0x39, 0x35],
            Dot::Alone => [0x9e, 0x9e, 0x9e],
        }
    }
}

/// Where the dot sits, and how big it is, in pixels.
const CENTRE: (i32, i32) = (25, 25);
const RADIUS: i32 = 6;

/// The rat with `dot`; `light_bar` picks the black rat.
pub fn picture(dot: Dot, light_bar: bool) -> Vec<u8> {
    let mut pixels = if light_bar { BLACK } else { WHITE }.to_vec();
    let [r, g, b] = dot.colour();
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let (dx, dy) = (x - CENTRE.0, y - CENTRE.1);
            let distance = dx * dx + dy * dy;
            // A dark ring around the dot keeps it apart from the lines.
            let colour = if distance <= RADIUS * RADIUS {
                [r, g, b, 255]
            } else if distance <= (RADIUS + 1) * (RADIUS + 1) {
                [0x20, 0x20, 0x20, 255]
            } else {
                continue;
            };
            let at = (y as usize * SIZE + x as usize) * 4;
            pixels[at..at + 4].copy_from_slice(&colour);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], x: usize, y: usize) -> &[u8] {
        let at = (y * SIZE + x) * 4;
        &pixels[at..at + 4]
    }

    #[test]
    fn the_dot_has_its_colour_and_a_ring() {
        let here = picture(Dot::Here, false);
        assert_eq!(pixel(&here, 25, 25), [0x2e, 0xc4, 0x4a, 255]);
        assert_eq!(
            pixel(&here, 25 + 7, 25).len(),
            4,
            "the ring may leave the picture"
        );
        assert_eq!(pixel(&here, 25, 25 - 7), [0x20, 0x20, 0x20, 255]);
        let away = picture(Dot::Elsewhere, false);
        assert_eq!(pixel(&away, 25, 25), [0xe5, 0x39, 0x35, 255]);
    }

    #[test]
    fn the_rest_of_the_rat_is_untouched() {
        let here = picture(Dot::Here, false);
        assert_eq!(here.len(), BYTES);
        assert_eq!(&here[..BYTES / 2], &WHITE[..BYTES / 2], "the top half");
        assert_ne!(picture(Dot::Alone, true), picture(Dot::Alone, false));
    }
}
