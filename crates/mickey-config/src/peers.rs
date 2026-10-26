//! What `mickey peers` prints (PRD 0005), shared with the settings window:
//! this PC's monitors and, per peer, whether it is online, its name and the
//! monitors it reported. One line each, with tabs (shown as `→`) between
//! the fields, since names may hold spaces:
//!
//! ```text
//! this→1536x864+0+0
//! peer→online→Windows→2560x1080+0+0;1920x1080+246+-1080
//! ```

/// A monitor in the space the cursor moves in (logical pixels).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Monitor {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    pub name: String,
    pub online: bool,
    /// It answered with another identity than the one this PC knows (it
    /// was reinstalled?), so it is refused until removed and connected
    /// again (#289). Shown as the state `reinstalled`.
    pub reinstalled: bool,
    pub monitors: Vec<Monitor>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Peers {
    pub this_pc: Vec<Monitor>,
    pub peers: Vec<PeerInfo>,
}

pub fn monitors_to_text(monitors: &[Monitor]) -> String {
    monitors
        .iter()
        .map(|m| format!("{}x{}+{}+{}", m.width, m.height, m.x, m.y))
        .collect::<Vec<_>>()
        .join(";")
}

fn monitors_from_text(text: &str) -> Option<Vec<Monitor>> {
    text.split(';')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (size, at) = part.split_once('+')?;
            let (width, height) = size.split_once('x')?;
            let (x, y) = split_position(at)?;
            Some(Monitor {
                x,
                y,
                width: width.parse().ok()?,
                height: height.parse().ok()?,
            })
        })
        .collect()
}

/// `"246+-1080"` → (246, -1080).
fn split_position(text: &str) -> Option<(i32, i32)> {
    let cut = text[1..].find('+')? + 1;
    Some((text[..cut].parse().ok()?, text[cut + 1..].parse().ok()?))
}

impl Peers {
    pub fn to_text(&self) -> String {
        let mut lines = vec![format!("this\t{}", monitors_to_text(&self.this_pc))];
        for peer in &self.peers {
            let state = match (peer.online, peer.reinstalled) {
                (true, _) => "online",
                (false, true) => "reinstalled",
                (false, false) => "offline",
            };
            lines.push(format!(
                "peer\t{state}\t{}\t{}",
                peer.name,
                monitors_to_text(&peer.monitors)
            ));
        }
        lines.join("\n")
    }

    pub fn from_text(text: &str) -> Option<Peers> {
        let mut peers = Peers::default();
        for line in text.lines().filter(|line| !line.is_empty()) {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["this", monitors] => peers.this_pc = monitors_from_text(monitors)?,
                ["peer", state, name, monitors] => peers.peers.push(PeerInfo {
                    name: name.to_string(),
                    online: *state == "online",
                    reinstalled: *state == "reinstalled",
                    monitors: monitors_from_text(monitors)?,
                }),
                // A peer with no monitors ends in a tab, which trimming the
                // answer takes away.
                ["peer", state, name] => peers.peers.push(PeerInfo {
                    name: name.to_string(),
                    online: *state == "online",
                    reinstalled: *state == "reinstalled",
                    monitors: Vec::new(),
                }),
                _ => return None,
            }
        }
        Some(peers)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_reinstalled_pc_is_its_own_state() {
        let peers = Peers {
            this_pc: vec![],
            peers: vec![PeerInfo {
                name: "Notebook".into(),
                online: false,
                reinstalled: true,
                monitors: vec![],
            }],
        };
        let text = peers.to_text();
        assert!(text.contains("peer\treinstalled\tNotebook"), "{text}");
        let back = Peers::from_text(&text).unwrap();
        assert!(back.peers[0].reinstalled && !back.peers[0].online);
    }

    use super::*;

    #[test]
    fn the_owners_setup_round_trips() {
        let peers = Peers {
            this_pc: vec![Monitor {
                x: 0,
                y: 0,
                width: 1536,
                height: 864,
            }],
            peers: vec![PeerInfo {
                name: "Windows".into(),
                online: true,
                reinstalled: false,
                monitors: vec![
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
                ],
            }],
        };
        let text = peers.to_text();
        assert!(
            text.contains("2560x1080+0+0;1920x1080+246+-1080;1080x1920+2560+-845"),
            "{text}"
        );
        assert_eq!(Peers::from_text(&text), Some(peers));
    }

    #[test]
    fn an_offline_peer_without_monitors_and_bad_lines() {
        let text = "this\t1536x864+0+0\npeer\toffline\tTeste\t\n";
        assert_eq!(Peers::from_text(text).unwrap().peers[0].monitors, []);
        // The same text with the trailing tab trimmed, as a caller may do.
        assert_eq!(Peers::from_text(text.trim()), Peers::from_text(text));
        assert_eq!(Peers::from_text("nonsense"), None);
        assert_eq!(Peers::from_text("this\t10x10"), None);
    }
}
