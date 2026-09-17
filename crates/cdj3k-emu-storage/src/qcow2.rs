//! Reading a byte range out of a qcow2 image.
//!
//! A slot's eMMC is written raw and converted to qcow2, so anything the
//! installer put at a fixed disk offset - the U-Boot environment - sits behind
//! the image's own mapping afterwards. This follows one guest offset to its
//! host cluster and reads it: two tables, no compression, no backing file.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAGIC: [u8; 4] = [0x51, 0x46, 0x49, 0xfb];
/// Bits 9..55 of an L1 or L2 entry: the host offset of the table or cluster
/// it points at. The rest are flags.
const OFFSET_MASK: u64 = 0x00ff_ffff_ffff_fe00;
/// An L2 entry with this bit set describes a compressed cluster, which this
/// reader does not follow. `qemu-img convert` writes none unless asked.
const COMPRESSED: u64 = 1 << 62;

/// `len` bytes from guest offset `offset`.
///
/// `Ok(None)` when the file is not a qcow2 image, or when the range is not
/// mapped - an unallocated cluster reads as zeros, which is never the data a
/// caller came for.
pub fn read_at(path: &Path, offset: u64, len: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut f = File::open(path)?;
    let mut head = [0u8; 48];
    f.read_exact(&mut head)?;
    if head[..4] != MAGIC {
        return Ok(None);
    }
    let cluster_bits = u32::from_be_bytes(head[20..24].try_into().expect("4 bytes"));
    if !(9..=21).contains(&cluster_bits) {
        return Ok(None);
    }
    let cluster = 1u64 << cluster_bits;
    let l1_len = u32::from_be_bytes(head[36..40].try_into().expect("4 bytes")) as u64;
    let l1_offset = u64::from_be_bytes(head[40..48].try_into().expect("8 bytes"));
    let l2_entries = cluster / 8;

    let mut out = Vec::with_capacity(len);
    let mut pos = offset;
    while out.len() < len {
        let within = pos % cluster;
        let take = ((cluster - within) as usize).min(len - out.len());
        let guest_cluster = pos / cluster;
        let l1_index = guest_cluster / l2_entries;
        if l1_index >= l1_len {
            return Ok(None);
        }
        let l2_table = read_u64(&mut f, l1_offset + l1_index * 8)? & OFFSET_MASK;
        if l2_table == 0 {
            return Ok(None);
        }
        let entry = read_u64(&mut f, l2_table + (guest_cluster % l2_entries) * 8)?;
        let host = entry & OFFSET_MASK;
        if host == 0 || entry & COMPRESSED != 0 {
            return Ok(None);
        }
        let mut buf = vec![0u8; take];
        f.seek(SeekFrom::Start(host + within))?;
        f.read_exact(&mut buf)?;
        out.extend_from_slice(&buf);
        pos += take as u64;
    }
    Ok(Some(out))
}

fn read_u64(f: &mut File, at: u64) -> std::io::Result<u64> {
    let mut b = [0u8; 8];
    f.seek(SeekFrom::Start(at))?;
    f.read_exact(&mut b)?;
    Ok(u64::from_be_bytes(b))
}
