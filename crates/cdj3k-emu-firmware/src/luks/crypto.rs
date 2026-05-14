//! Block-cipher and KDF helpers for LUKS1 decryption.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use aes::cipher::KeyInit;
use aes::Aes256;
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use hmac::Hmac;
use pbkdf2::pbkdf2;
use sha1::Sha1;
use sha2::Sha256;
use xts_mode::Xts128;

use super::{Luks1Header, LUKS_SECTOR};

type Aes256Cbc = cbc::Decryptor<Aes256>;

pub(super) fn xts_tweak(sector: u64, mode: &str) -> [u8; 16] {
    let mut t = [0u8; 16];
    if mode.contains("plain64") {
        t[..8].copy_from_slice(&sector.to_le_bytes());
    } else {
        t[..4].copy_from_slice(&(sector as u32).to_le_bytes());
    }
    t
}

// ── PBKDF2 helper ────────────────────────────────────────────────────────────

pub(super) fn pbkdf2_derive(
    hash_spec: &str,
    pwd: &[u8],
    salt: &[u8],
    iters: u32,
    out: &mut [u8],
) -> Result<(), ()> {
    match hash_spec.to_lowercase().as_str() {
        "sha256" => pbkdf2::<Hmac<Sha256>>(pwd, salt, iters, out).map_err(|_| ()),
        _ => pbkdf2::<Hmac<Sha1>>(pwd, salt, iters, out).map_err(|_| ()),
    }
}

// ── AFC merge ────────────────────────────────────────────────────────────────

/// LUKS1 Anti-Forensic Coding merge (cryptsetup af.c AF_merge).
///
/// For each stripe i in 0..stripes-2:
///   d ^= stripe_i
///   diffuse(d): for each hash_len-byte block j: d[j] = H(BE32(j) || d[j])
/// master_key = d XOR stripe_{stripes-1}
pub(super) fn af_merge(src: &[u8], key_bytes: usize, stripes: usize, hash_spec: &str) -> Vec<u8> {
    let mut d = vec![0u8; key_bytes];
    for i in 0..stripes - 1 {
        let stripe = &src[i * key_bytes..(i + 1) * key_bytes];
        for (a, b) in d.iter_mut().zip(stripe) {
            *a ^= b;
        }
        d = diffuse(&d, hash_spec);
    }
    let last = &src[(stripes - 1) * key_bytes..stripes * key_bytes];
    for (a, b) in d.iter_mut().zip(last) {
        *a ^= b;
    }
    d
}

/// cryptsetup diffuse(): hash each hash_len-byte block independently.
/// Block counter resets to 0 for every call (per stripe).
/// IV encoding: be32_to_cpu(i) on LE machine = bytes of BE32(i) = i.to_be_bytes().
pub(super) fn diffuse(buf: &[u8], hash_spec: &str) -> Vec<u8> {
    if hash_spec.eq_ignore_ascii_case("sha256") {
        diffuse_sha256(buf)
    } else {
        diffuse_sha1(buf)
    }
}

pub(super) fn diffuse_sha256(buf: &[u8]) -> Vec<u8> {
    use sha2::Digest;
    const HLEN: usize = 32;
    let blocks = buf.len() / HLEN;
    let padding = buf.len() % HLEN;
    let mut out = Vec::with_capacity(buf.len());
    for j in 0..blocks {
        let mut h = sha2::Sha256::new();
        h.update((j as u32).to_be_bytes());
        h.update(&buf[j * HLEN..(j + 1) * HLEN]);
        out.extend_from_slice(&h.finalize());
    }
    if padding > 0 {
        let mut h = sha2::Sha256::new();
        h.update((blocks as u32).to_be_bytes());
        h.update(&buf[blocks * HLEN..]);
        let digest = h.finalize();
        out.extend_from_slice(&digest[..padding]);
    }
    out
}

pub(super) fn diffuse_sha1(buf: &[u8]) -> Vec<u8> {
    use sha1::Digest;
    const HLEN: usize = 20;
    let blocks = buf.len() / HLEN;
    let padding = buf.len() % HLEN;
    let mut out = Vec::with_capacity(buf.len());
    for j in 0..blocks {
        let mut h = sha1::Sha1::new();
        h.update((j as u32).to_be_bytes());
        h.update(&buf[j * HLEN..(j + 1) * HLEN]);
        out.extend_from_slice(&h.finalize());
    }
    if padding > 0 {
        let mut h = sha1::Sha1::new();
        h.update((blocks as u32).to_be_bytes());
        h.update(&buf[blocks * HLEN..]);
        let digest = h.finalize();
        out.extend_from_slice(&digest[..padding]);
    }
    out
}

pub(super) fn decrypt_payload<R: Read + Seek>(
    r: &mut R,
    out_path: &Path,
    h: &Luks1Header,
    master_key: &[u8],
) -> io::Result<()> {
    let mut out = std::fs::File::create(out_path)?;
    r.seek(SeekFrom::Start(h.payload_offset as u64 * LUKS_SECTOR))?;

    let mode = h.cipher_mode.to_lowercase();

    if mode.starts_with("xts") {
        decrypt_payload_xts(r, &mut out, master_key)
    } else {
        decrypt_payload_cbc(r, &mut out, master_key, &mode)
    }
}

pub(super) fn decrypt_payload_xts<R: Read>(
    r: &mut R,
    out: &mut std::fs::File,
    master_key: &[u8],
) -> io::Result<()> {
    if master_key.len() != 64 {
        return Err(io::Error::other("XTS payload requires 64-byte master key"));
    }
    let c1 =
        Aes256::new_from_slice(&master_key[..32]).map_err(|e| io::Error::other(e.to_string()))?;
    let c2 =
        Aes256::new_from_slice(&master_key[32..]).map_err(|e| io::Error::other(e.to_string()))?;
    let xts = Xts128::new(c1, c2);

    let mut sector_buf = [0u8; 512];
    let mut sector_num: u64 = 0;

    loop {
        let n = read_sector(r, &mut sector_buf)?;
        if n == 0 {
            break;
        }
        let mut block = sector_buf[..n].to_vec();
        while block.len() % 16 != 0 {
            block.push(0);
        }

        let mut tweak = [0u8; 16];
        tweak[..8].copy_from_slice(&sector_num.to_le_bytes());
        xts.decrypt_sector(&mut block, tweak);

        out.write_all(&block[..n])?;
        sector_num += 1;
    }
    out.flush()
}

pub(super) fn decrypt_payload_cbc<R: Read>(
    r: &mut R,
    out: &mut std::fs::File,
    master_key: &[u8],
    mode: &str,
) -> io::Result<()> {
    let essiv_key: Option<[u8; 32]> = if mode.contains("essiv") {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(master_key);
        Some(h.finalize().into())
    } else {
        None
    };

    let mut sector_buf = [0u8; 512];
    let mut sector_num: u64 = 0;

    loop {
        let n = read_sector(r, &mut sector_buf)?;
        if n == 0 {
            break;
        }
        let mut block = sector_buf[..n].to_vec();
        while block.len() % 16 != 0 {
            block.push(0);
        }

        let iv = match &essiv_key {
            Some(ek) => essiv_iv(ek, sector_num),
            None => {
                let mut iv = [0u8; 16];
                iv[..8].copy_from_slice(&sector_num.to_le_bytes());
                iv
            }
        };

        if let Ok(cipher) = Aes256Cbc::new_from_slices(master_key, &iv) {
            let _ = cipher.decrypt_padded_mut::<cbc::cipher::block_padding::NoPadding>(&mut block);
        }

        out.write_all(&block[..n])?;
        sector_num += 1;
    }
    out.flush()
}

pub(super) fn read_sector<R: Read>(r: &mut R, buf: &mut [u8; 512]) -> io::Result<usize> {
    let mut total = 0;
    while total < 512 {
        match r.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

pub(super) fn essiv_iv(essiv_key: &[u8; 32], sector: u64) -> [u8; 16] {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};

    let mut iv = [0u8; 16];
    iv[..8].copy_from_slice(&sector.to_le_bytes());

    let cipher = aes::Aes256::new(GenericArray::from_slice(essiv_key));
    let mut block = aes::Block::clone_from_slice(&iv);
    cipher.encrypt_block(&mut block);
    block.into()
}
