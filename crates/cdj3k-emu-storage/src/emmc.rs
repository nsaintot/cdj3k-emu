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

use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crc::{Crc, CRC_32_ISO_HDLC};

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
    /// Version metadata from the firmware ISO.
    pub firmware: FirmwareInfo,
}

impl EmmcConfig {
    pub fn new(path: PathBuf, instance_id: u32) -> Self {
        Self {
            path,
            instance_id,
            firmware: FirmwareInfo::default(),
        }
    }
}

/// Default path: `~/Library/Application Support/<BUNDLE_ID>/instance-N/emmc.qcow2`.
pub fn default_path(instance_id: u32) -> PathBuf {
    crate::app_data_dir()
        .join(format!("instance-{}", instance_id))
        .join("emmc.qcow2")
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
    write_gpt_raw(&raw_path, config.instance_id, &config.firmware)?;

    // Step 2: convert to qcow2.  `qemu-img` is resolved by
    // [`cdj3k_emu_platform::bundled::tool`], which prefers the copy bundled
    // next to the running executable so Finder-launched .app instances don't
    // depend on the user's shell PATH.
    convert_to_qcow2(&raw_path, &config.path)?;
    std::fs::remove_file(&raw_path)?;

    Ok(&config.path)
}

/// Build and write a valid U-Boot environment block at UBOOT_ENV_OFFSET.
///
/// Format: [CRC32-LE 4 bytes][key=value\0 ... \0\0][zero padding to UBOOT_ENV_SIZE]
/// CRC32 covers bytes [4..UBOOT_ENV_SIZE] (the env data region).
fn write_uboot_env(
    file: &mut std::fs::File,
    instance_id: u32,
    fw: &FirmwareInfo,
) -> std::io::Result<()> {
    let serial = format!("DJMP{:06}EH", instance_id);

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
        ("model", "CDJ3K-RK3399"),
        ("part", "0:6"),
        ("pxefile_addr_r", "0x00600000"),
        ("ramdisk_addr_r", "0x0a200000"),
        ("release", fw.release.as_deref().unwrap_or("")),
        ("rev_apl", fw.rev_apl.as_deref().unwrap_or("")),
        ("rev_kernel", fw.rev_kernel.as_deref().unwrap_or("")),
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
    (bytes + SECTOR - 1) / SECTOR
}

fn write_gpt_raw(raw_path: &Path, instance_id: u32, fw: &FirmwareInfo) -> std::io::Result<()> {
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
        if cursor % align != 0 {
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
    write_uboot_env(&mut file, instance_id, fw)
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
