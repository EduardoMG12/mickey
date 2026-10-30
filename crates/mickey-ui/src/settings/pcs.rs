//! What the settings window asks the running Mickey about other PCs: the
//! ones heard on the network, and the paired ones as it sees them.

use mickey_config::peers::Peers;

/// A PC running Mickey heard on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Nearby {
    pub(super) name: String,
    pub(super) address: String,
    pub(super) fingerprint: String,
}

/// `mickey discover --lines`: name, address, fingerprint per line. Empty
/// when Mickey cannot say.
pub(super) fn load_nearby() -> Vec<Nearby> {
    parse_nearby(&crate::mickey::ask(&["discover", "--lines"]).unwrap_or_default())
}

pub(super) fn parse_nearby(text: &str) -> Vec<Nearby> {
    text.lines()
        .filter_map(|line| {
            let [name, address, fingerprint] = line.split('\t').collect::<Vec<_>>()[..] else {
                return None;
            };
            Some(Nearby {
                name: name.to_string(),
                address: address.to_string(),
                fingerprint: fingerprint.to_string(),
            })
        })
        .collect()
}

/// The PC named in `mickey connect`'s answer (its `mickey: ` already
/// taken off), when a pairing started.
pub(super) fn pairing_with(answer: &str) -> Option<String> {
    let rest = answer.strip_prefix("pairing with ")?;
    let (name, _) = rest.split_once(": ")?;
    Some(name.to_string())
}

pub(super) fn load_peers() -> Result<Peers, String> {
    let text = crate::mickey::ask(&["peers"])?;
    Peers::from_text(&text).ok_or_else(|| format!("mickey peers answered `{text}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pairing_pc_is_read_from_mickeys_answer() {
        // What `mickey connect` prints, without its "mickey: ".
        assert_eq!(
            pairing_with("pairing with Desk PC: check that both PCs show the same code").as_deref(),
            Some("Desk PC")
        );
        assert_eq!(
            pairing_with("192.0.2.10:4242 is already one of this PC's PCs"),
            None
        );
    }

    #[test]
    fn nearby_lines_become_pcs() {
        let nearby = parse_nearby("Desk\t192.0.2.64:4242\tVK2A-L4MJ\nnonsense\n");
        assert_eq!(nearby.len(), 1);
        assert_eq!(nearby[0].address, "192.0.2.64:4242");
    }
}
