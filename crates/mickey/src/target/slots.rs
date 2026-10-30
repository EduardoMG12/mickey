//! A limit on the connections a target accepts at once, so a stranger on
//! the network cannot open enough of them to keep paired PCs out.

use crate::threads::LockAnyway;

/// At most this many connections on a port at once, paired or not: a few
/// PCs need a few. Without a cap, anyone on the network could open
/// connections until no thread could be made.
pub(super) const MAX_CONNECTIONS: usize = 32;

/// At most this many of a port's connections from one address: a PC uses
/// one or two, so one host on the network cannot take every slot and keep
/// the paired PCs from connecting.
pub(super) const MAX_PER_ADDRESS: usize = 4;

/// A port's connections: how many, and how many from each address.
#[derive(Default)]
pub(super) struct Slots {
    total: usize,
    by_address: std::collections::HashMap<std::net::IpAddr, usize>,
}

pub(super) type SharedSlots = std::sync::Arc<std::sync::Mutex<Slots>>;

/// One of a port's connections, given back when dropped.
pub(super) struct Slot(SharedSlots, std::net::IpAddr);

impl Slot {
    /// `None` when the port already has `MAX_CONNECTIONS`, or this address
    /// `MAX_PER_ADDRESS`: the caller drops the stream, which closes it.
    pub(super) fn take(slots: &SharedSlots, from: std::net::IpAddr) -> Option<Slot> {
        let mut counts = slots.lock_anyway();
        let mine = counts.by_address.get(&from).copied().unwrap_or(0);
        if counts.total >= MAX_CONNECTIONS || mine >= MAX_PER_ADDRESS {
            return None;
        }
        counts.total += 1;
        counts.by_address.insert(from, mine + 1);
        Some(Slot(slots.clone(), from))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut counts = self.0.lock_anyway();
        counts.total -= 1;
        if let Some(mine) = counts.by_address.get_mut(&self.1) {
            *mine -= 1;
            if *mine == 0 {
                counts.by_address.remove(&self.1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_address_cannot_take_every_slot() {
        let slots = SharedSlots::default();
        let stranger: std::net::IpAddr = "192.0.2.66".parse().unwrap();
        let held: Vec<Slot> = (0..10)
            .filter_map(|_| Slot::take(&slots, stranger))
            .collect();
        assert_eq!(held.len(), MAX_PER_ADDRESS);
        let friend: std::net::IpAddr = "192.0.2.7".parse().unwrap();
        assert!(
            Slot::take(&slots, friend).is_some(),
            "a paired PC still gets in"
        );
        drop(held);
        assert!(
            Slot::take(&slots, stranger).is_some(),
            "slots come back when closed"
        );
    }
}
