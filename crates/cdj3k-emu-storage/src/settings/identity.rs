//! The two values that identify a slot to the guest: its MAC and the SoC
//! serial the deck's `genkey_pr` keys `cabinet.img` with. This module only
//! mints and validates them; [`super::InstanceSettings`] stores them.

/// Whether `s` has 16 lowercase hex digits. Any last byte works:
/// `genkey_pr` hashes its input N + 1 times, where N is the last byte, so a
/// serial such as `0123456789abcd00` hashes its input once.
pub(super) fn is_valid_soc_serial(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Mint a SoC serial from a v4 UUID's first 8 bytes.
pub(super) fn generate_soc_serial() -> String {
    let b = uuid::Uuid::new_v4().into_bytes();
    b[..8].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Generate a random locally administered unicast MAC.
///
/// The bytes come from a v4 UUID, whose bytes are cryptographically random.
/// The first byte gets the LAA bit set and the multicast bit cleared, so the
/// address is a valid host address.
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

    /// A minted serial and MAC always pass their validators.
    #[test]
    fn a_mint_always_passes_its_own_validator() {
        assert!(is_valid_soc_serial("0123456789abcd05"));
        assert!(is_valid_soc_serial("0123456789abcd00"), "a zero last byte");
        assert!(!is_valid_soc_serial("deadbeef"), "too short");
        assert!(!is_valid_soc_serial("zzzzzzzzzzzzzzzz"), "not hex");
        assert!(!is_valid_soc_serial("0123456789ABCD05"), "upper case");

        for _ in 0..64 {
            let serial = generate_soc_serial();
            assert!(is_valid_soc_serial(&serial), "{serial}");
            let mac = generate_mac();
            assert!(cdj3k_emu_platform::net::is_valid_mac(&mac), "{mac}");
            let first = u8::from_str_radix(&mac[..2], 16).unwrap();
            assert_eq!(first & 0x03, 0x02, "{mac}: LAA set, multicast clear");
        }
    }
}
