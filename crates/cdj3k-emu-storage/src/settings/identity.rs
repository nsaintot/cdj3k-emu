//! The two values that identify a slot to the guest: its MAC and the SoC
//! serial the deck's `genkey_pr` keys `cabinet.img` with. Minting and
//! validation only - [`super::InstanceSettings`] owns the persistence.

/// 16 lowercase hex digits with a non-zero last byte, matching what an RK3399
/// prints for `Serial`.  `genkey_pr` repeats its hash input
/// `strtol(&serial[14..], 16)` times, so a zero there would hash nothing.
pub(super) fn is_valid_soc_serial(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && matches!(u8::from_str_radix(&s[14..], 16), Ok(n) if n != 0)
}

/// Mint a SoC serial from a v4 UUID's first 8 bytes, forcing the last byte
/// non-zero so `genkey_pr`'s repeat count is at least 1.
pub(super) fn generate_soc_serial() -> String {
    let b = uuid::Uuid::new_v4().into_bytes();
    format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7].max(1)
    )
}

pub(super) fn is_valid_mac(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && u8::from_str_radix(p, 16).is_ok())
}

/// Generate a random locally-administered unicast MAC.
/// Uses `uuid::Uuid::new_v4()` (already a dep) as the entropy source - its bytes
/// are cryptographically random on macOS/Linux. The first byte is forced to
/// `02` (LAA bit set, multicast bit clear) so it's a valid host address.
pub(super) fn generate_mac() -> String {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let m0 = (bytes[0] & 0xfe) | 0x02; // LAA, unicast
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        m0, bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `genkey_pr` can use, and what a mint therefore has to produce.
    #[test]
    fn a_mint_always_passes_its_own_validator() {
        assert!(is_valid_soc_serial("0123456789abcd05"));
        // Last byte zero: genkey_pr would repeat its hash input zero times.
        assert!(!is_valid_soc_serial("0123456789abcd00"));
        assert!(!is_valid_soc_serial("deadbeef"), "too short");
        assert!(!is_valid_soc_serial("zzzzzzzzzzzzzzzz"), "not hex");
        assert!(!is_valid_soc_serial("0123456789ABCD05"), "upper case");

        assert!(is_valid_mac("02:11:22:33:44:55"));
        assert!(!is_valid_mac("02:11:22:33:44"), "five octets");
        assert!(!is_valid_mac("02-11-22-33-44-55"), "not colon-separated");

        for _ in 0..64 {
            let serial = generate_soc_serial();
            assert!(is_valid_soc_serial(&serial), "{serial}");
            let mac = generate_mac();
            assert!(is_valid_mac(&mac), "{mac}");
            let first = u8::from_str_radix(&mac[..2], 16).unwrap();
            assert_eq!(first & 0x03, 0x02, "{mac}: LAA set, multicast clear");
        }
    }
}
