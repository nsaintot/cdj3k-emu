//! CRC-16/X-25 (ITU-T, polynomial `0x8408`, init `0xFFFF`, output XOR'd with
//! `0xFFFF`). Trailer of every MISO frame; also used for ad-hoc integrity
//! checks of MOSI captures.

/// Compute the CRC-16/X-25 of `data`.
pub fn crc16_x25(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &b in data {
        crc ^= b as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0x8408
            } else {
                crc >> 1
            };
        }
    }
    crc ^ 0xFFFF
}
