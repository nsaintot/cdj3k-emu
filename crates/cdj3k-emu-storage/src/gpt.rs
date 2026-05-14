//! Pure-Rust GPT partition table writer.
//!
//! Writes a GUID Partition Table into any writable object that implements
//! `std::io::Write + std::io::Seek`.  The caller is responsible for creating
//! the underlying file/image at the right size beforehand.
//!
//! Reference: UEFI Specification 2.10, §5 "GUID Partition Table (GPT)".

use std::io::{self, Seek, SeekFrom, Write};

use crc::Crc;
use uuid::Uuid;

const SECTOR: u64 = 512;
const GPT_HEADER_LBA: u64 = 1;
const GPT_ENTRIES_LBA: u64 = 2;
const GPT_ENTRY_SIZE: u32 = 128;
const GPT_MAX_ENTRIES: u32 = 128;
/// Sectors consumed by the entry table: 128 entries * 128 bytes / 512 bytes-per-sector.
const GPT_ENTRY_TABLE_SECTORS: u64 = (GPT_MAX_ENTRIES as u64 * GPT_ENTRY_SIZE as u64) / SECTOR;
/// Backup-GPT footprint at the end of the disk: 1 header sector + the entry table.
pub const GPT_BACKUP_RESERVED_SECTORS: u64 = 1 + GPT_ENTRY_TABLE_SECTORS;
/// First LBA usable for partition data: protective MBR (LBA 0) + primary header
/// (LBA 1) + primary entry table = LBA 2 + entry-table sectors.
const GPT_FIRST_USABLE_LBA: u64 = GPT_ENTRIES_LBA + GPT_ENTRY_TABLE_SECTORS;

// Well-known partition type GUIDs (mixed-endian as per UEFI spec).
const TYPE_LINUX_DATA: [u8; 16] = guid_to_mixed(
    0x0FC63DAF,
    0x8483,
    0x4772,
    [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4],
);
const TYPE_LINUX_RESERVED: [u8; 16] = guid_to_mixed(
    0x8DA63339,
    0x0007,
    0x60C0,
    [0xC4, 0x36, 0x08, 0x3A, 0xC8, 0x23, 0x09, 0x08],
);

const CRC32: Crc<u32> = Crc::<u32>::new(&crc::CRC_32_ISO_HDLC);

/// A partition entry passed to [`write_gpt`].
#[derive(Clone, Debug)]
pub struct PartEntry {
    /// Partition type GUID bytes (mixed-endian).  Use [`linux_data_type()`] or
    /// [`linux_reserved_type()`] for common types.
    pub type_guid: [u8; 16],
    /// Unique partition GUID (random).
    pub part_guid: [u8; 16],
    /// First LBA (512-byte sectors).
    pub first_lba: u64,
    /// Last LBA (inclusive).
    pub last_lba: u64,
    /// Attribute flags (0 = none).
    pub attrs: u64,
    /// Partition name (UTF-16LE, max 36 chars).
    pub name: &'static str,
}

impl PartEntry {
    pub fn new(type_guid: [u8; 16], first_lba: u64, last_lba: u64, name: &'static str) -> Self {
        Self {
            type_guid,
            part_guid: *Uuid::new_v4().as_bytes(),
            first_lba,
            last_lba,
            attrs: 0,
            name,
        }
    }
}

pub fn linux_data_type() -> [u8; 16] {
    TYPE_LINUX_DATA
}
pub fn linux_reserved_type() -> [u8; 16] {
    TYPE_LINUX_RESERVED
}

/// Write a complete GPT (protective MBR + primary header + entries + backup) into `w`.
/// `disk_sectors` is the total number of 512-byte sectors on the disk.
pub fn write_gpt<W: Write + Seek>(
    w: &mut W,
    disk_sectors: u64,
    partitions: &[PartEntry],
) -> io::Result<()> {
    assert!(partitions.len() <= GPT_MAX_ENTRIES as usize);

    // ── Protective MBR ───────────────────────────────────────────────────────
    let mut pmbr = [0u8; 512];
    // MBR signature.
    pmbr[510] = 0x55;
    pmbr[511] = 0xAA;
    // Single protective partition (type 0xEE, covers whole disk).
    let entry = &mut pmbr[446..462];
    entry[4] = 0xEE; // type: GPT protective
    entry[8..12].copy_from_slice(&1u32.to_le_bytes()); // first LBA
    let size = (disk_sectors.saturating_sub(1).min(0xFFFF_FFFF)) as u32;
    entry[12..16].copy_from_slice(&size.to_le_bytes()); // sector count

    w.seek(SeekFrom::Start(0))?;
    w.write_all(&pmbr)?;

    // ── Partition entries ─────────────────────────────────────────────────────
    let entries_bytes = (GPT_MAX_ENTRIES * GPT_ENTRY_SIZE) as usize;
    let mut entry_buf = vec![0u8; entries_bytes];
    for (i, p) in partitions.iter().enumerate() {
        let off = i * GPT_ENTRY_SIZE as usize;
        let e = &mut entry_buf[off..off + GPT_ENTRY_SIZE as usize];
        e[0..16].copy_from_slice(&p.type_guid);
        e[16..32].copy_from_slice(&p.part_guid);
        e[32..40].copy_from_slice(&p.first_lba.to_le_bytes());
        e[40..48].copy_from_slice(&p.last_lba.to_le_bytes());
        e[48..56].copy_from_slice(&p.attrs.to_le_bytes());
        // Name: UTF-16LE, null-padded to 72 bytes (36 chars).
        let name_bytes = &mut e[56..128];
        for (j, ch) in p.name.encode_utf16().take(36).enumerate() {
            let b = ch.to_le_bytes();
            name_bytes[j * 2] = b[0];
            name_bytes[j * 2 + 1] = b[1];
        }
    }

    let entries_crc = CRC32.checksum(&entry_buf);

    // Primary entries at LBA 2.
    w.seek(SeekFrom::Start(GPT_ENTRIES_LBA * SECTOR))?;
    w.write_all(&entry_buf)?;

    // Backup entry table sits just before the backup header (last sector).
    let backup_entries_lba = disk_sectors - GPT_BACKUP_RESERVED_SECTORS;
    w.seek(SeekFrom::Start(backup_entries_lba * SECTOR))?;
    w.write_all(&entry_buf)?;

    // ── Primary GPT header ────────────────────────────────────────────────────
    let primary = build_header(
        GPT_HEADER_LBA,
        disk_sectors - 1,
        GPT_ENTRIES_LBA,
        disk_sectors,
        entries_crc,
    );
    w.seek(SeekFrom::Start(GPT_HEADER_LBA * SECTOR))?;
    w.write_all(&primary)?;

    // ── Backup GPT header ─────────────────────────────────────────────────────
    let backup = build_header(
        disk_sectors - 1,
        GPT_HEADER_LBA,
        backup_entries_lba,
        disk_sectors,
        entries_crc,
    );
    w.seek(SeekFrom::Start((disk_sectors - 1) * SECTOR))?;
    w.write_all(&backup)?;

    w.flush()
}

fn build_header(
    my_lba: u64,
    alternate_lba: u64,
    entries_start_lba: u64,
    disk_sectors: u64,
    entries_crc: u32,
) -> Vec<u8> {
    let mut h = vec![0u8; 92];

    h[0..8].copy_from_slice(b"EFI PART"); // signature
    h[8..12].copy_from_slice(&[0x00, 0x00, 0x01, 0x00]); // revision 1.0
    h[12..16].copy_from_slice(&92u32.to_le_bytes()); // header size
                                                     // h[16..20] = header CRC (filled below)
                                                     // h[20..24] = reserved (0)
    h[24..32].copy_from_slice(&my_lba.to_le_bytes());
    h[32..40].copy_from_slice(&alternate_lba.to_le_bytes());
    h[40..48].copy_from_slice(&GPT_FIRST_USABLE_LBA.to_le_bytes());
    let last_usable_lba = disk_sectors - GPT_BACKUP_RESERVED_SECTORS - 1;
    h[48..56].copy_from_slice(&last_usable_lba.to_le_bytes());
    // Disk GUID at h[56..72]: generate once per call (not ideal but fine for our use).
    let disk_guid = *Uuid::new_v4().as_bytes();
    h[56..72].copy_from_slice(&disk_guid);
    h[72..80].copy_from_slice(&entries_start_lba.to_le_bytes());
    h[80..84].copy_from_slice(&GPT_MAX_ENTRIES.to_le_bytes());
    h[84..88].copy_from_slice(&GPT_ENTRY_SIZE.to_le_bytes());
    h[88..92].copy_from_slice(&entries_crc.to_le_bytes());

    let header_crc = CRC32.checksum(&h);
    h[16..20].copy_from_slice(&header_crc.to_le_bytes());

    // Pad to one sector.
    h.resize(SECTOR as usize, 0);
    h
}

/// Convert a UEFI GUID (data1/data2/data3/data4) into the 16-byte mixed-endian
/// representation used in GPT partition type/unique GUIDs.
const fn guid_to_mixed(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] {
    [
        (d1 & 0xFF) as u8,
        ((d1 >> 8) & 0xFF) as u8,
        ((d1 >> 16) & 0xFF) as u8,
        ((d1 >> 24) & 0xFF) as u8,
        (d2 & 0xFF) as u8,
        ((d2 >> 8) & 0xFF) as u8,
        (d3 & 0xFF) as u8,
        ((d3 >> 8) & 0xFF) as u8,
        d4[0],
        d4[1],
        d4[2],
        d4[3],
        d4[4],
        d4[5],
        d4[6],
        d4[7],
    ]
}
