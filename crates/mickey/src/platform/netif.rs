//! This PC's IPv4 networks on Windows, for discovery (#217): Windows sends
//! 255.255.255.255 out of one adapter only, so Mickey also announces on each
//! network's own broadcast address.

use std::net::Ipv4Addr;

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
