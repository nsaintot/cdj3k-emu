//! eMMC qcow2 provisioning.
//!
//! Creates a 29.1 GB qcow2 image with a GPT partition layout matching the
//! Pioneer CDJ-3000 eMMC (mmcblk1).  In QEMU, virtio_blk.c maps device
//! index 0 → /dev/mmcblk1 so the Pioneer init scripts find the expected paths.
//!
//! Partition layout:
//!   p1  4 MB  raw     Bootloader (LOADER) - zeroed placeholder; U-Boot env
//!                     block written at offset 0x3f8000 (matches fw_env.config)
//!   p2  4 MB  raw     TrustFirmware (BL3X) - zeroed placeholder
//!   p3  4 MB  raw     Rockchip resource (RSCE) - zeroed placeholder
//!   p4  128 MB  raw   Recovery firmware slot (FAT32 on real hw) - zeroed
//!   p5  256 MB  raw   App firmware slot A - zeroed
//!   p6  256 MB  raw   App firmware slot B - zeroed
//!   p7  64 MB  ext4   Settings (/home/root/settings)
//!   p8  ~28.4 GB ext4 User data / rekordbox cache (/mnt)
//!
//! p7 and p8 are left unformatted; the guest formats them on first boot
//! (settings-mount.sh detects blank signature and runs mkfs.ext4).
//! The image is qcow2 sparse - host disk usage is near-zero until written.

use std::collections::BTreeMap;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crc::{Crc, CRC_32_ISO_HDLC};

use cdj3k_emu_panel::Model;

use crate::gpt::{linux_data_type, write_gpt, PartEntry};

pub use cdj3k_emu_firmware::FirmwareInfo;

const MB: u64 = 1024 * 1024;
const GB: u64 = 1024 * MB;
const SECTOR: u64 = 512;

/// Total virtual disk size: 29.1 GB.
pub const EMMC_SIZE: u64 = 29 * GB + 100 * MB; // 29,360,128,000 bytes ≈ 29.1 GB

/// fw_env.config: /dev/mmcblk1  0x003f8000  0x8000
const UBOOT_ENV_OFFSET: u64 = 0x003f8000;
const UBOOT_ENV_SIZE: usize = 0x8000;

/// Configuration for eMMC image creation.
pub struct EmmcConfig {
    /// Output path for the qcow2 image.
    pub path: PathBuf,
    /// Instance ID - used to derive the serial number.
    pub instance_id: u32,
    /// The deck this image is for.  Selects the `model` U-Boot variable, which
    /// `genkey_pr` hashes with the SoC serial into the cabinet.img passphrase.
    pub model: Model,
    /// Version metadata from the firmware ISO.
    pub firmware: FirmwareInfo,
    /// The firmware's `images/cabinet.img`, staged raw in the recovery
    /// partition for the guest to install on its first boot.  `None` stages
    /// nothing, and the deck runs without Widevine or Device Library Plus.
    pub cabinet: Option<Vec<u8>>,
}

impl EmmcConfig {
    pub fn new(path: PathBuf, instance_id: u32) -> Self {
        Self {
            path,
            instance_id,
            model: Model::Cdj3k,
            firmware: FirmwareInfo::default(),
            cabinet: None,
        }
    }
}

/// The eMMC image of slot `instance_id`:
/// `~/Library/Application Support/<BUNDLE_ID>/instance-N/emmc.qcow2`.
pub fn default_path(instance_id: u32) -> PathBuf {
    crate::FirmwarePaths::new(instance_id).emmc
}

/// Ensure the eMMC qcow2 exists, creating and partitioning it if not.
/// Returns the path to the image.
pub fn provision_emmc(config: &EmmcConfig) -> std::io::Result<&Path> {
    if config.path.exists() {
        return Ok(&config.path);
    }
    if let Some(parent) = config.path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Step 1: create a sparse raw file, write the GPT, and inject the U-Boot env.
    let raw_path = config.path.with_extension("raw.tmp");
    write_gpt_raw(&raw_path, config)?;

    // Step 2: convert to qcow2.  `qemu-img` is resolved by
    // [`cdj3k_emu_platform::bundled::tool`], which prefers the copy bundled
    // next to the running executable so Finder-launched .app instances don't
    // depend on the user's shell PATH.
    convert_to_qcow2(&raw_path, &config.path)?;
    std::fs::remove_file(&raw_path)?;

    Ok(&config.path)
}

/// The U-Boot environment a slot's eMMC carries, as key/value pairs.
///
/// `None` when the block is not mapped in the image or its CRC does not cover
/// what follows it - a half-written or foreign image says nothing.
pub fn read_uboot_env(emmc: &Path) -> Option<BTreeMap<String, String>> {
    let block = crate::qcow2::read_at(emmc, UBOOT_ENV_OFFSET, UBOOT_ENV_SIZE).ok()??;
    let (stored, data) = block.split_at(4);
    let stored = u32::from_le_bytes(stored.try_into().expect("4 bytes"));
    if Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(data) != stored {
        return None;
    }
    let mut env = BTreeMap::new();
    for entry in data.split(|b| *b == 0) {
        // The list ends at the first empty entry: its double-null terminator.
        if entry.is_empty() {
            break;
        }
        if let Some((k, v)) = String::from_utf8_lossy(entry).split_once('=') {
            env.insert(k.to_string(), v.to_string());
        }
    }
    Some(env)
}

/// Build and write a valid U-Boot environment block at UBOOT_ENV_OFFSET.
///
/// Format: [CRC32-LE 4 bytes][key=value\0 ... \0\0][zero padding to UBOOT_ENV_SIZE]
/// CRC32 covers bytes [4..UBOOT_ENV_SIZE] (the env data region).
fn write_uboot_env(
    file: &mut std::fs::File,
    instance_id: u32,
    model: Model,
    fw: &FirmwareInfo,
) -> std::io::Result<()> {
    let serial = cdj3k_emu_platform::identity::device_serial(instance_id);

    // U-boot environment variables.
    let vars: &[(&str, &str)] = &[
        ("arch", "arm"),
        ("baudrate", "115200"),
        ("board", "evb_rk3399"),
        ("board_name", "evb_rk3399"),
        ("bootargs_emmc", "earlycon=uart8250,mmio32,0xff1a0000 swiotlb=1 console=ttyFIQ0 rw rootfstype=ramfs rootwait quiet loglevel=3 coherent_pool=1m"),
        ("bootcmd", "run bootcmd_emmc"),
        ("bootcmd_emmc", "setenv bootargs ${bootargs_emmc};load mmc ${part} ${kernel_addr_r} /${image};run booti_cmd"),
        ("bootdelay", "0"),
        ("booti_cmd", "booti ${kernel_addr_r} - ${fdt_addr_r}"),
        ("boot_type", "normal"),
        ("cpu", "armv8"),
        ("fdt_addr_r", "0x00280000"),
        ("image", "Image"),
        ("kernel_addr_r", "0x10480000"),
        ("kernel_bank", "B"),
        ("miniloader", fw.miniloader.as_deref().unwrap_or("")),
        ("model", model.spec().model_env),
        ("part", "0:6"),
        ("pxefile_addr_r", "0x00600000"),
        ("ramdisk_addr_r", "0x0a200000"),
        ("release", fw.release.as_deref().unwrap_or("")),
        ("rev_apl", fw.rev_apl.as_deref().unwrap_or("")),
        (model.spec().system_rev_env, fw.rev_system.as_deref().unwrap_or("")),
        ("serial_number", &serial),
        ("soc", "rockchip"),
        ("stderr", "serial,vidconsole"),
        ("stdout", "serial,vidconsole"),
        ("update_status", "success"),
        ("vendor", "rockchip"),
    ];

    // Build env data: null-terminated "key=value" strings + final null terminator.
    let mut env_data = Vec::with_capacity(UBOOT_ENV_SIZE - 4);
    for (k, v) in vars {
        env_data.extend_from_slice(k.as_bytes());
        env_data.push(b'=');
        env_data.extend_from_slice(v.as_bytes());
        env_data.push(0);
    }
    env_data.push(0); // double-null terminator

    // Pad to (UBOOT_ENV_SIZE - 4) with zeros.
    let data_len = UBOOT_ENV_SIZE - 4;
    assert!(
        env_data.len() <= data_len,
        "U-Boot env vars exceed block size"
    );
    env_data.resize(data_len, 0);

    let crc = Crc::<u32>::new(&CRC_32_ISO_HDLC).checksum(&env_data);

    file.seek(SeekFrom::Start(UBOOT_ENV_OFFSET))?;
    file.write_all(&crc.to_le_bytes())?;
    file.write_all(&env_data)?;

    Ok(())
}

fn sectors(bytes: u64) -> u64 {
    bytes.div_ceil(SECTOR)
}

/// Marker for the staged cabinet image, first line of a 512-byte ASCII header
/// at the start of the recovery partition.  The header carries the image
/// length; the image itself follows it. Guest patch 14 reads both.
const CABINET_STAGE_MAGIC: &str = "CDJ3KCAB1";
const CABINET_STAGE_HEADER: u64 = 512;

/// Write `cabinet` into the partition beginning at `first_lba`, behind the
/// ASCII header the guest parses.
fn stage_cabinet(
    file: &mut std::fs::File,
    part: &PartEntry,
    cabinet: &[u8],
) -> std::io::Result<()> {
    let capacity = (part.last_lba - part.first_lba + 1) * SECTOR;
    let needed = CABINET_STAGE_HEADER + cabinet.len() as u64;
    if needed > capacity {
        return Err(std::io::Error::other(format!(
            "cabinet.img ({} B) exceeds the {} partition ({} B)",
            cabinet.len(),
            part.name,
            capacity
        )));
    }

    let mut header = format!("{}\nsize={}\n", CABINET_STAGE_MAGIC, cabinet.len()).into_bytes();
    header.resize(CABINET_STAGE_HEADER as usize, 0);

    file.seek(SeekFrom::Start(part.first_lba * SECTOR))?;
    file.write_all(&header)?;
    file.write_all(cabinet)
}

fn write_gpt_raw(raw_path: &Path, config: &EmmcConfig) -> std::io::Result<()> {
    // Create sparse file at full virtual size.
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(raw_path)?;
    file.set_len(EMMC_SIZE)?;

    let disk_sectors = EMMC_SIZE / SECTOR;
    let data = linux_data_type();

    // Partition layout (all sizes in sectors, 1 MiB-aligned start).
    // First usable LBA = 34 (GPT overhead); align first partition to LBA 2048 (1 MiB).
    let align = 2048u64; // 1 MiB in 512-byte sectors
    let mut cursor = align;

    let mut p = |size_bytes: u64, name: &'static str| -> PartEntry {
        let n_sectors = sectors(size_bytes);
        let first = cursor;
        let last = first + n_sectors - 1;
        cursor = last + 1;
        // Align next start to 1 MiB boundary.
        if !cursor.is_multiple_of(align) {
            cursor = (cursor / align + 1) * align;
        }
        PartEntry::new(data, first, last, name)
    };

    let partitions = vec![
        p(4 * MB, "bootloader"),    // p1
        p(4 * MB, "trustfirmware"), // p2
        p(4 * MB, "resource"),      // p3
        p(128 * MB, "recovery"),    // p4
        p(256 * MB, "firmware-a"),  // p5
        p(256 * MB, "firmware-b"),  // p6
        p(64 * MB, "settings"),     // p7
        // p8: remainder of disk (leave room for backup GPT header + entry table).
        {
            let last = disk_sectors - crate::gpt::GPT_BACKUP_RESERVED_SECTORS - 1;
            PartEntry::new(data, cursor, last, "userdata")
        },
    ];

    let mut w = std::io::BufWriter::new(file);
    write_gpt(&mut w, disk_sectors, &partitions)?;
    w.flush()?;

    let mut file = w.into_inner().map_err(|e| e.into_error())?;
    // p4 (recovery) holds the staged cabinet image: the emulator boots the
    // kernel from `-kernel`, so nothing else reads that partition.
    if let Some(cabinet) = &config.cabinet {
        stage_cabinet(&mut file, &partitions[3], cabinet)?;
    }
    write_uboot_env(
        &mut file,
        config.instance_id,
        config.model,
        &config.firmware,
    )
}

fn convert_to_qcow2(raw: &Path, out: &Path) -> std::io::Result<()> {
    let status = Command::new(cdj3k_emu_platform::bundled::tool("qemu-img"))
        .args([
            "convert",
            "-f",
            "raw",
            "-O",
            "qcow2",
            "-o",
            "preallocation=off",
            &raw.to_string_lossy(),
            &out.to_string_lossy(),
        ])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("qemu-img convert failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `stage_cabinet` writes the ASCII header guest patch 14 parses, then the
    /// image, and refuses one the partition cannot hold rather than truncating.
    #[test]
    fn stages_the_cabinet_behind_its_header() {
        let dir = std::env::temp_dir().join(format!("cdj3k-cabinet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("emmc.raw");

        let first_lba = 26_624; // p4 in the layout above
        let part = PartEntry::new(linux_data_type(), first_lba, first_lba + 2048, "recovery");
        let cabinet: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();

        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        stage_cabinet(&mut file, &part, &cabinet).unwrap();

        let raw = std::fs::read(&path).unwrap();
        let base = (first_lba * SECTOR) as usize;
        let header = &raw[base..base + CABINET_STAGE_HEADER as usize];
        let end = header.iter().position(|b| *b == 0).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&header[..end]),
            format!("{CABINET_STAGE_MAGIC}\nsize=4096\n")
        );
        assert_eq!(&raw[base + CABINET_STAGE_HEADER as usize..], &cabinet[..]);

        // One sector past the partition is an error, and writes nothing.
        let huge = vec![0u8; (2049 * SECTOR) as usize];
        assert!(stage_cabinet(&mut file, &part, &huge).is_err());
        assert_eq!(std::fs::read(&path).unwrap().len(), raw.len());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
