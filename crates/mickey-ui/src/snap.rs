//! Where a peer's desktop goes when dropped in the Layout tab: against the
//! nearest monitor of this PC, touching it without overlapping, like the
//! monitors in Windows' display settings.

use mickey_config::peers::Monitor;

/// Closer than this (desktop pixels), a dropped peer lines up with the
/// monitor's corner instead of stopping a few pixels off.
const ALIGN: i32 = 120;

pub fn shifted(monitor: &Monitor, [x, y]: [i32; 2]) -> Monitor {
    Monitor {
        x: monitor.x + x,
        y: monitor.y + y,
        ..*monitor
    }
}

fn overlap(a: &Monitor, b: &Monitor) -> bool {
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
}

/// `value` within `low..=high`, pulled to either end when it is close.
fn slide(value: i32, low: i32, high: i32, ends: [i32; 2]) -> i32 {
    let value = value.clamp(low, high);
    ends.into_iter()
        .filter(|end| (end - value).abs() < ALIGN)
        .min_by_key(|end| (end - value).abs())
        .unwrap_or(value)
}

/// The position nearest to `wanted` at which the peer's monitors touch one
/// of this PC's along an edge (sharing at least one pixel of it) without
/// overlapping any.
pub fn snap(home: &[Monitor], peer: &[Monitor], wanted: [i32; 2]) -> Option<[i32; 2]> {
    let [want_x, want_y] = wanted;
    let mut candidates = Vec::new();
    for h in home {
        for p in peer {
            // Beside: x is fixed by the edge; y slides while the two still
            // share some height, and lines up with the top or bottom.
            let low = h.y - p.y - p.height + 1;
            let high = h.y + h.height - 1 - p.y;
            let y = slide(
                want_y,
                low,
                high,
                [h.y - p.y, h.y + h.height - p.y - p.height],
            );
            candidates.push([h.x + h.width - p.x, y]); // right of h
            candidates.push([h.x - p.width - p.x, y]); // left of h
            // Above or below: the same with the axes swapped.
            let low = h.x - p.x - p.width + 1;
            let high = h.x + h.width - 1 - p.x;
            let x = slide(
                want_x,
                low,
                high,
                [h.x - p.x, h.x + h.width - p.x - p.width],
            );
            candidates.push([x, h.y + h.height - p.y]); // below h
            candidates.push([x, h.y - p.height - p.y]); // above h
        }
    }
    candidates
        .into_iter()
        .filter(|&at| {
            peer.iter()
                .all(|p| home.iter().all(|h| !overlap(&shifted(p, at), h)))
        })
        .min_by_key(|[x, y]| {
            let (dx, dy) = (i64::from(x - want_x), i64::from(y - want_y));
            dx * dx + dy * dy
        })
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
    /// The owner's Windows PC.
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
    fn a_drop_on_the_left_touches_the_main_monitor() {
        // Dropped a bit away from the main monitor's left edge.
        assert_eq!(snap(&WINDOWS, &LAPTOP, [-1700, 500]), Some([-1536, 500]));
    }

    #[test]
    fn near_a_corner_it_lines_up_with_it() {
        // Left of the main monitor, 60 px below its top: lines up with it.
        assert_eq!(snap(&WINDOWS, &LAPTOP, [-1536, 60]), Some([-1536, 0]));
        // Near its bottom: the laptop's bottom lines up with the monitor's.
        assert_eq!(
            snap(&WINDOWS, &LAPTOP, [-1536, 1080 - 864 + 50]),
            Some([-1536, 216])
        );
    }

    #[test]
    fn it_never_overlaps_and_always_shares_an_edge() {
        // Dropped right on top of the main monitor.
        let [x, y] = snap(&WINDOWS, &LAPTOP, [800, 100]).unwrap();
        let placed = shifted(&LAPTOP[0], [x, y]);
        assert!(WINDOWS.iter().all(|m| !overlap(&placed, m)), "{placed:?}");
        // Far below everything: comes back to touch the main monitor.
        assert_eq!(snap(&WINDOWS, &LAPTOP, [500, 5000]), Some([500, 1080]));
    }

    #[test]
    fn no_monitors_no_place() {
        assert_eq!(snap(&[], &LAPTOP, [0, 0]), None);
    }
}
