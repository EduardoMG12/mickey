//! This PC's IPv4 networks on Windows and macOS, for discovery (#217):
//! 255.255.255.255 may leave through one adapter only, so Mickey also
//! announces on each network's own broadcast address.

use std::net::Ipv4Addr;

#[cfg(windows)]
mod imp {
    use super::Ipv4Addr;

    /// `MIB_IPADDRROW`: one IPv4 address of one adapter.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct IpAddrRow {
        /// The address and mask as their four bytes, in network order.
        address: u32,
        index: u32,
        mask: u32,
        broadcast: u32,
        reassembly: u32,
        unused: u16,
        kind: u16,
    }

    /// `wType` bits: an adapter that is unplugged, or an address being removed.
    const MIB_IPADDR_DISCONNECTED: u16 = 0x0008;
    const MIB_IPADDR_DELETED: u16 = 0x0040;
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;

    #[link(name = "iphlpapi")]
    unsafe extern "system" {
        fn GetIpAddrTable(table: *mut u8, size: *mut u32, order: i32) -> u32;
    }

    /// Each connected adapter's address and mask (loopback included; the
    /// caller decides which to use). Empty if Windows will not say.
    pub fn networks() -> Vec<(Ipv4Addr, Ipv4Addr)> {
        let mut size = 0u32;
        // SAFETY: a null table with size 0 only asks for the size needed.
        let first = unsafe { GetIpAddrTable(std::ptr::null_mut(), &mut size, 0) };
        if first != ERROR_INSUFFICIENT_BUFFER || size == 0 {
            return Vec::new();
        }
        // u32s, so the table (a u32 count, then rows of u32s) is aligned.
        let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
        // SAFETY: `buffer` holds at least `size` bytes and lives through the
        // call; Windows writes the count, then that many rows, within it.
        if unsafe { GetIpAddrTable(buffer.as_mut_ptr().cast(), &mut size, 0) } != 0 {
            return Vec::new();
        }
        let count = buffer[0] as usize;
        let row_words = size_of::<IpAddrRow>() / 4;
        let rows = &buffer[1..];
        if rows.len() < count * row_words {
            return Vec::new();
        }
        (0..count)
            .map(|i| {
                // SAFETY: row `i` lies inside `rows` (checked just above), which
                // is u32-aligned, as IpAddrRow needs.
                unsafe { *(rows[i * row_words..].as_ptr() as *const IpAddrRow) }
            })
            .filter(|row| row.kind & (MIB_IPADDR_DISCONNECTED | MIB_IPADDR_DELETED) == 0)
            .map(|row| {
                (
                    Ipv4Addr::from(row.address.to_ne_bytes()),
                    Ipv4Addr::from(row.mask.to_ne_bytes()),
                )
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::Ipv4Addr;

    /// Guard that releases the linked list returned by `getifaddrs`.
    struct IfAddrs(*mut libc::ifaddrs);

    impl Drop for IfAddrs {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: `self.0` came from a successful `getifaddrs` call
                // and has not been freed yet.
                unsafe { libc::freeifaddrs(self.0) };
            }
        }
    }

    /// Each active IPv4 interface's address and netmask (loopback included;
    /// the caller decides which to use).
    pub fn networks() -> Vec<(Ipv4Addr, Ipv4Addr)> {
        let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
        // SAFETY: `head` is a valid out-pointer; on success it is wrapped in
        // `IfAddrs` so `freeifaddrs` runs on every exit path.
        if unsafe { libc::getifaddrs(&mut head) } != 0 {
            return Vec::new();
        }
        let _guard = IfAddrs(head);
        let mut found = Vec::new();
        let mut current = head;
        while !current.is_null() {
            // SAFETY: `current` is a non-null node in the list returned by
            // `getifaddrs`, which stays alive until `_guard` drops.
            let iface = unsafe { &*current };
            current = iface.ifa_next;
            let up = iface.ifa_flags & (libc::IFF_UP as u32) != 0;
            if !up || iface.ifa_addr.is_null() || iface.ifa_netmask.is_null() {
                continue;
            }
            // SAFETY: `ifa_addr` was checked non-null; reading `sa_family`
            // tells whether it is a `sockaddr_in`.
            let family = unsafe { (*iface.ifa_addr).sa_family };
            if i32::from(family) != libc::AF_INET {
                continue;
            }
            // SAFETY: `sa_family == AF_INET`, so `ifa_addr` and `ifa_netmask`
            // point to `sockaddr_in` structs allocated by `getifaddrs`.
            let addr = unsafe { *(iface.ifa_addr.cast::<libc::sockaddr_in>()) };
            let mask = unsafe { *(iface.ifa_netmask.cast::<libc::sockaddr_in>()) };
            found.push((
                Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()),
                Ipv4Addr::from(mask.sin_addr.s_addr.to_ne_bytes()),
            ));
        }
        found
    }
}

pub fn networks() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    imp::networks()
}

#[cfg(test)]
mod tests {
    /// Run by hand on Windows: `<test exe> --ignored --nocapture networks`.
    #[test]
    #[ignore = "lists this PC's real adapters; run by hand on Windows"]
    fn networks_of_this_pc() {
        for (address, mask) in super::networks() {
            // Only the shape: addresses of the owner's networks stay private.
            let [a, ..] = address.octets();
            println!("{a}.x.x.x / {}", u32::from(mask).count_ones());
        }
    }
}
