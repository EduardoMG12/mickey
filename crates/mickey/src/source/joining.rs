//! PCs joining through the group (PRD 0010): a peer shares its map of the
//! group, and the PCs this one does not know yet join, asked first when no
//! trusted PC paired with them here (#184).

use super::{Peer, Shared, Wake, save_key, save_position};
use crate::net::group;
use crate::threads::LockAnyway;
use mickey_config as config;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

/// The PCs in a peer's map (`members`, sent by `by`) this PC should know:
/// trusted ones join at once; for the others the person here is asked,
/// once per run (`asked`) and one question at a time. The indexes of the
/// peers added, to connect to.
pub(super) fn take_map(
    members: &[group::Member],
    by: [u8; 32],
    shared: &Shared,
    peers: &mut Vec<Peer>,
    positions: &mut Vec<Option<[i32; 2]>>,
    asked: &mut HashSet<[u8; 32]>,
    wake: &Sender<Wake>,
) -> Vec<usize> {
    let me = *shared.identity.public();
    let mut joined = Vec::new();
    for member in group::as_seen_by(members, &me) {
        let trusted = shared.trusted.lock_anyway().is_trusted(&member.key);
        if !trusted {
            // One question on screen at a time: a paired PC could send map
            // after map of made-up PCs. The others are asked when a later
            // map names them again.
            if group::is_local(member.address.ip())
                && !JOIN_ASKING.load(Ordering::SeqCst)
                && !refused_recently()
                && asked.insert(member.key)
            {
                let introducer = peers.iter().find(|peer| peer.key == Some(by)).map_or_else(
                    || crate::net::identity::fingerprint(&by),
                    |peer| peer.name.clone(),
                );
                ask_to_join(member, by, introducer, wake);
            }
            continue;
        }
        joined.extend(join(member, &by, shared, peers, positions));
    }
    joined
}

/// Whether a "let this PC in?" question is on screen (see `ask_to_join`).
static JOIN_ASKING: AtomicBool = AtomicBool::new(false);

/// After a refusal, no question for this long: maps naming new made-up
/// PCs each time cannot keep a window open.
const REFUSED_WAIT: Duration = Duration::from_secs(60);

static LAST_REFUSED: Mutex<Option<Instant>> = Mutex::new(None);

fn refused_recently() -> bool {
    LAST_REFUSED
        .lock_anyway()
        .is_some_and(|at| at.elapsed() < REFUSED_WAIT)
}

/// Asks, on its own thread (the main loop must keep moving keys), whether
/// to let `member` join, as `introducer` (who sent the map) says it is in
/// the group. A yes comes back as `Wake::Accepted`; anything else leaves
/// it out.
pub(super) fn ask_to_join(
    member: group::Member,
    by: [u8; 32],
    introducer: String,
    wake: &Sender<Wake>,
) {
    use std::sync::atomic::Ordering;
    if JOIN_ASKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let wake = wake.clone();
    crate::threads::spawn("ask to join", move || {
        let answer = crate::window::ui_command(&["join", &member.name, &introducer])
            .and_then(|mut ui| ui.status());
        match answer.map(|status| status.code()) {
            Ok(Some(0)) => {
                let _ = wake.send(Wake::Accepted(member, by));
            }
            Ok(Some(1)) => {
                *LAST_REFUSED.lock_anyway() = Some(Instant::now());
                println!("mickey: {} was not let in", member.name);
            }
            _ => eprintln!(
                "mickey: cannot ask whether to let {} in; it stays out",
                member.name
            ),
        }
        JOIN_ASKING.store(false, Ordering::SeqCst);
    });
}

/// One PC from a peer's map (already in this PC's coordinates): trusted,
/// because a trusted member vouches for it (the owner's choice, PRD 0010);
/// its place taken if it has one; added to config.toml and to `peers` if
/// this PC did not know it. Returns the new peer's index, to connect to.
pub(super) fn join(
    member: group::Member,
    by: &[u8; 32],
    shared: &Shared,
    peers: &mut Vec<Peer>,
    positions: &mut Vec<Option<[i32; 2]>>,
) -> Option<usize> {
    // Only PCs on a local network: a group map cannot make this PC call
    // out to the internet, or to itself (#184).
    if !group::is_local(member.address.ip()) {
        eprintln!(
            "mickey: {} from the group is not on a local network; ignored",
            member.name
        );
        return None;
    }
    {
        let mut trusted = shared.trusted.lock_anyway();
        // Noted with who introduced it, so forgetting that PC forgets it too.
        let note = format!(
            "{} via {}",
            member.address,
            crate::net::identity::fingerprint(by)
        );
        if !trusted.is_trusted(&member.key)
            && let Err(err) = trusted.add(member.key, &note)
        {
            eprintln!("mickey: {err}");
            return None;
        }
    }
    let known = peers
        .iter()
        .position(|peer| peer.key == Some(member.key) || peer.address == member.address);
    if let Some(index) = known {
        if let Some(position) = member.position
            && positions[index] != Some(position)
        {
            positions[index] = Some(position);
            save_position(&peers[index].name, position);
        }
        return None;
    }
    match add_peer(
        peers,
        positions,
        &member.name,
        member.address,
        member.position,
    ) {
        Ok(index) => {
            peers[index].key = Some(member.key);
            save_key(&peers[index].name, &member.key);
            println!("mickey: {} joined through the group", peers[index].name);
            Some(index)
        }
        Err(err) => {
            eprintln!("mickey: cannot add {} from the group: {err}", member.name);
            None
        }
    }
}

/// A PC new to this one: written into config.toml (a name already taken
/// gets a number) and added to `peers`, not yet connected. Its index.
pub(super) fn add_peer(
    peers: &mut Vec<Peer>,
    positions: &mut Vec<Option<[i32; 2]>>,
    name: &str,
    address: SocketAddr,
    position: Option<[i32; 2]>,
) -> Result<usize, String> {
    let name = config::path().and_then(|path| {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let (text, name) = config::add_peer(&text, name, address, position)?;
        config::write(&path, &text)?;
        Ok(name)
    })?;
    peers.push(Peer::new(name, address, None, Instant::now()));
    positions.push(position);
    Ok(peers.len() - 1)
}
