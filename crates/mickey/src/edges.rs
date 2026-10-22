//! Screen edges (PRD 0005): when the cursor is at the edge of this PC's
//! desktop on a peer's side, and where it comes in on the peer.
//!
//! Positions along an edge travel as a fraction (0.0 at the top or left,
//! 1.0 at the bottom or right) of the whole desktop, so PCs with different
//! resolutions keep the same relative height.

use crate::monitors::Monitor;
use mickey_config::Side;

/// The desktop's bounding box: (left, top, right, bottom), right and
/// bottom exclusive.
fn bounds(monitors: &[Monitor]) -> Option<(i32, i32, i32, i32)> {
    let left = monitors.iter().map(|m| m.x).min()?;
    let top = monitors.iter().map(|m| m.y).min()?;
    let right = monitors.iter().map(|m| m.x + m.width).max()?;
    let bottom = monitors.iter().map(|m| m.y + m.height).max()?;
    Some((left, top, right, bottom))
}

fn contains(monitor: &Monitor, x: i32, y: i32) -> bool {
    (monitor.x..monitor.x + monitor.width).contains(&x)
        && (monitor.y..monitor.y + monitor.height).contains(&y)
}

fn on_screen(monitors: &[Monitor], x: i32, y: i32) -> bool {
    monitors.iter().any(|m| contains(m, x, y))
}

/// One pixel further in the direction of `side`.
fn step(side: Side, x: i32, y: i32) -> (i32, i32) {
    match side {
        Side::Left => (x - 1, y),
        Side::Right => (x + 1, y),
        Side::Above => (x, y - 1),
        Side::Below => (x, y + 1),
    }
}

/// If the cursor at `x, y` is pushing against the desktop's edge on `side`
/// (one pixel further is off every monitor), the fraction along that edge.
///
/// Uses the monitor the cursor is on, not the bounding box: on a desktop
/// whose monitors do not line up, the edge is wherever the cursor cannot go
/// further.
pub fn leaving(monitors: &[Monitor], side: Side, x: i32, y: i32) -> Option<f32> {
    let (left, top, right, bottom) = bounds(monitors)?;
    let (next_x, next_y) = step(side, x, y);
    if !on_screen(monitors, x, y) || on_screen(monitors, next_x, next_y) {
        return None;
    }
    let fraction = |at: i32, start: i32, end: i32| {
        // `end - 1` is the last pixel, so both extremes are reachable.
        (at - start) as f32 / (end - 1 - start).max(1) as f32
    };
    Some(match side {
        Side::Left | Side::Right => fraction(y, top, bottom),
        Side::Above | Side::Below => fraction(x, left, right),
    })
}

/// Where the cursor comes in on a desktop entered through its `side` edge
/// (a peer on our right is entered through its left), at `fraction` along
/// it. If no monitor reaches that point of the edge, the nearest one does.
pub fn entering(monitors: &[Monitor], side: Side, fraction: f32) -> Option<(i32, i32)> {
    let (left, top, right, bottom) = bounds(monitors)?;
    let along = |start: i32, end: i32| {
        start + (fraction.clamp(0.0, 1.0) * (end - 1 - start) as f32).round() as i32
    };
    let horizontal = matches!(side, Side::Left | Side::Right);
    let target = if horizontal {
        along(top, bottom)
    } else {
        along(left, right)
    };
    // The monitors' spans on the edge's axis, and how far each is from
    // `target` (0 when it covers it).
    let distance = |m: &Monitor| {
        let (start, end) = if horizontal {
            (m.y, m.y + m.height)
        } else {
            (m.x, m.x + m.width)
        };
        (start - target).max(target - (end - 1)).max(0)
    };
    let nearest = monitors.iter().map(distance).min()?;
    // Among the monitors that reach `target` best, the outermost on `side`.
    let monitor = monitors
        .iter()
        .filter(|m| distance(m) == nearest)
        .min_by_key(|m| match side {
            Side::Left => m.x,
            Side::Right => -(m.x + m.width),
            Side::Above => m.y,
            Side::Below => -(m.y + m.height),
        })?;
    let clamp_y = target.clamp(monitor.y, monitor.y + monitor.height - 1);
    let clamp_x = target.clamp(monitor.x, monitor.x + monitor.width - 1);
    Some(match side {
        Side::Left => (monitor.x, clamp_y),
        Side::Right => (monitor.x + monitor.width - 1, clamp_y),
        Side::Above => (clamp_x, monitor.y),
        Side::Below => (clamp_x, monitor.y + monitor.height - 1),
    })
}

/// The edge a peer on `side` of us is entered through.
pub fn facing(side: Side) -> Side {
    match side {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
        Side::Above => Side::Below,
        Side::Below => Side::Above,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: [Monitor; 1] = [Monitor {
        x: 0,
        y: 0,
        width: 1536,
        height: 864,
    }];
    /// The owner's Windows PC: a wide main monitor, one above it, one
    /// sideways on the right that sticks out above.
    const WINDOWS: [Monitor; 3] = [
        Monitor {
            x: 0,
            y: 0,
            width: 2560,
            height: 1080,
        },
        Monitor {
            x: 246,
            y: -1080,
            width: 1920,
            height: 1080,
        },
        Monitor {
            x: 2560,
            y: -845,
            width: 1080,
            height: 1920,
        },
    ];

    #[test]
    fn only_the_last_pixel_on_that_side_leaves() {
        assert_eq!(leaving(&LAPTOP, Side::Right, 1535, 0), Some(0.0));
        assert_eq!(leaving(&LAPTOP, Side::Right, 1535, 863), Some(1.0));
        assert_eq!(leaving(&LAPTOP, Side::Right, 1534, 400), None);
        assert_eq!(leaving(&LAPTOP, Side::Left, 1535, 400), None);
        assert_eq!(leaving(&LAPTOP, Side::Above, 768, 0), Some(768.0 / 1535.0));
    }

    #[test]
    fn a_monitor_beside_is_not_an_edge() {
        // The main monitor's right edge touches the sideways one.
        assert_eq!(leaving(&WINDOWS, Side::Right, 2559, 500), None);
        // Above the main monitor, left of the one above: the cursor cannot
        // go up there, so it is an edge even though it is not the top.
        assert!(leaving(&WINDOWS, Side::Above, 100, 0).is_some());
        assert!(leaving(&WINDOWS, Side::Right, 3639, 0).is_some());
    }

    #[test]
    fn the_height_is_kept_from_one_desktop_to_the_other() {
        let fraction = leaving(&LAPTOP, Side::Right, 1535, 432).unwrap();
        // Windows sits on the laptop's right, so it is entered on its left.
        let (x, y) = entering(&WINDOWS, facing(Side::Right), fraction).unwrap();
        // Half way down Windows' desktop (-1080..1080) is y = 0, on the
        // main monitor's left edge.
        assert_eq!(x, 0);
        assert!((-2..=2).contains(&y), "{y}");
    }

    #[test]
    fn the_outermost_monitor_takes_the_cursor() {
        // y = 0 on the right edge: both the main and the sideways monitor
        // reach it; the sideways one sticks out further right.
        let fraction = 1080.0 / 2159.0;
        assert_eq!(entering(&WINDOWS, Side::Right, fraction), Some((3639, 0)));
        // The top of the left edge: only the upper monitor reaches it.
        assert_eq!(entering(&WINDOWS, Side::Left, 0.0), Some((246, -1080)));
    }

    #[test]
    fn where_no_monitor_reaches_the_nearest_one_does() {
        let apart = [
            Monitor {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            Monitor {
                x: 200,
                y: 0,
                width: 100,
                height: 100,
            },
        ];
        // x = 150 is in the gap; the right monitor is 50 away, the left 51.
        assert_eq!(entering(&apart, Side::Above, 0.5), Some((200, 0)));
        assert_eq!(entering(&[], Side::Left, 0.5), None);
    }
}
