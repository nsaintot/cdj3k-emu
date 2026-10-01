//! Windows: the adapter list from `GetIfTable2`, with IPv4 addresses from
//! `GetAdaptersAddresses`, filtered by [`super::windows_kind`].
//!
//! `GetIfTable2` because a bridge member has no IP binding of its own, and
//! `GetAdaptersAddresses` lists IP interfaces only.

use std::collections::HashMap;

use windows::Win32::Foundation::ERROR_BUFFER_OVERFLOW;
use windows::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GetAdaptersAddresses, GetIfTable2, GAA_FLAG_SKIP_ANYCAST,
    GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH, MIB_IF_TABLE2,
};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC, SOCKADDR_IN};

use super::NetIf;
use super::windows_kind::Adapter;

/// A guest is bridged onto a physical NIC; there is no host-only network.
pub const HOST_ONLY: bool = false;

pub fn enumerate() -> Vec<NetIf> {
    let all = adapters();
    // A bridge member's address is the bridge's: Windows has at most one.
    let bridge_ipv4 = all.iter().find(|a| a.is_bridge()).and_then(|a| a.ipv4.clone());
    all.into_iter()
        .filter(Adapter::bridgeable)
        .map(|a| {
            let (addr, prefix_len) = a.ipv4.or_else(|| bridge_ipv4.clone()).unwrap_or_default();
            NetIf {
                name: a.friendly_name,
                addr,
                prefix_len,
            }
        })
        .collect()
}

/// `FilterInterface` in `MIB_IF_ROW2::InterfaceAndOperStatusFlags`: an NDIS
/// filter layered over another interface, listed under its own row.
const FILTER_INTERFACE: u8 = 1 << 1;

/// Every adapter the host has, with no filtering beyond NDIS filter layers.
pub fn adapters() -> Vec<Adapter> {
    let addrs = ipv4_by_luid();
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    // SAFETY: on success `table` points at a table the call allocated, freed
    // below; rows are read only within `NumEntries`.
    unsafe {
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return Vec::new();
        }
        let rows = std::slice::from_raw_parts(
            (*table).Table.as_ptr(),
            (*table).NumEntries as usize,
        );
        let out = rows
            .iter()
            .filter(|r| r.InterfaceAndOperStatusFlags._bitfield & FILTER_INTERFACE == 0)
            .map(|r| Adapter {
                friendly_name: wide(&r.Alias),
                description: wide(&r.Description),
                if_type: r.Type,
                up: r.OperStatus == IfOperStatusUp,
                ipv4: addrs.get(&r.InterfaceLuid.Value).cloned(),
            })
            .filter(|a| !a.friendly_name.is_empty())
            .collect();
        FreeMibTable(table.cast());
        out
    }
}

fn wide(chars: &[u16]) -> String {
    let len = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..len])
}

/// The first IPv4 address of every IP interface, by interface LUID.
fn ipv4_by_luid() -> HashMap<u64, (String, u8)> {
    let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
    // Regrown until the list fits; it can grow between calls. u64 elements
    // keep the structs aligned.
    let mut len = 16 * 1024u32;
    let mut buf: Vec<u64> = Vec::new();
    for _ in 0..4 {
        buf.resize((len as usize).div_ceil(8), 0);
        let head = buf.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        // SAFETY: `head` points at at least `len` writable bytes.
        let rc = unsafe { GetAdaptersAddresses(AF_UNSPEC.0 as u32, flags, None, Some(head), &mut len) };
        if rc == ERROR_BUFFER_OVERFLOW.0 {
            continue;
        }
        if rc != 0 {
            return HashMap::new();
        }
        return walk(head);
    }
    HashMap::new()
}

fn walk(head: *const IP_ADAPTER_ADDRESSES_LH) -> HashMap<u64, (String, u8)> {
    let mut out = HashMap::new();
    let mut cur = head;
    // SAFETY: the list was filled by `GetAdaptersAddresses` into a buffer that
    // outlives this walk; every pointer followed is null or inside it.
    unsafe {
        while !cur.is_null() {
            let a = &*cur;
            cur = a.Next;
            let mut u = a.FirstUnicastAddress;
            while !u.is_null() {
                let ua = &*u;
                u = ua.Next;
                let sa = ua.Address.lpSockaddr;
                if sa.is_null() || (*sa).sa_family != AF_INET {
                    continue;
                }
                let sin = &*(sa as *const SOCKADDR_IN);
                let ip = std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.S_un.S_addr));
                out.insert(a.Luid.Value, (ip.to_string(), ua.OnLinkPrefixLength));
                break;
            }
        }
    }
    out
}
