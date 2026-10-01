//! The BSD/macOS/Linux address walk, shared by hosts that have `getifaddrs`.
//!
//! The interface's name is all this layer knows, so the decision of whether
//! it can carry a guest belongs to the caller: see [`super::macos`] and
//! [`super::linux`].

use super::NetIf;

/// Whether the kernel knows an interface named `name`.
pub fn exists(name: &str) -> bool {
    let Ok(name) = std::ffi::CString::new(name) else {
        return false;
    };
    // SAFETY: `name` is a NUL-terminated string that outlives the call.
    unsafe { libc::if_nametoindex(name.as_ptr()) != 0 }
}

/// Non-loopback, up, IPv4 interfaces whose name `keep` accepts, plus its
/// prefix length from the netmask.
pub fn walk(keep: fn(&str) -> bool) -> Vec<NetIf> {
    let mut out = Vec::new();

    // SAFETY: `getifaddrs` fills a NULL-terminated list that `freeifaddrs`
    // releases; every `ifa_addr` and `ifa_netmask` is checked against its
    // family before being cast to `sockaddr_in`.
    unsafe {
        let mut addrs: *mut libc::ifaddrs = std::ptr::null_mut();
        if libc::getifaddrs(&mut addrs) != 0 {
            return out;
        }

        let mut cur = addrs;
        while !cur.is_null() {
            let ifa = &*cur;
            cur = ifa.ifa_next;

            let sa = ifa.ifa_addr;
            if sa.is_null() {
                continue;
            }
            if (*sa).sa_family as i32 != libc::AF_INET {
                continue;
            }
            if (ifa.ifa_flags & libc::IFF_LOOPBACK as u32) != 0 {
                continue;
            }
            if (ifa.ifa_flags & libc::IFF_UP as u32) == 0 {
                continue;
            }

            let sin = &*(sa as *const libc::sockaddr_in);
            let ip = u32::from_be(sin.sin_addr.s_addr);
            let a = (ip >> 24) as u8;
            let b = (ip >> 16) as u8;
            let c = (ip >> 8) as u8;
            let d = ip as u8;

            let mask_sa = ifa.ifa_netmask;
            let prefix_len = if !mask_sa.is_null() && (*mask_sa).sa_family as i32 == libc::AF_INET {
                let mask_sin = &*(mask_sa as *const libc::sockaddr_in);
                u32::from_be(mask_sin.sin_addr.s_addr).count_ones() as u8
            } else {
                0
            };

            let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                .to_string_lossy()
                .into_owned();

            if !keep(&name) {
                continue;
            }

            out.push(NetIf {
                name,
                addr: format!("{a}.{b}.{c}.{d}"),
                prefix_len,
            });
        }

        libc::freeifaddrs(addrs);
    }

    out
}
