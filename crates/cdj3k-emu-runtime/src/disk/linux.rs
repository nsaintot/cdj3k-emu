//! Linux adapter: lsblk/udisks, polkit, and image formatting with sfdisk and
//! mkfs.exfat.

use super::{DiskProvider, HostDiskProvider, PhysicalDisk, RawMedium, UsbError};
use std::os::fd::OwnedFd;
use std::path::Path;
use std::process::Command;

/// What a refused raw open means here.
pub const PERMISSION_HINT: &str = "udisks did not authorize opening the device.";

pub const PASSES_FDS: bool = true;

pub const RETRY_PROMPT: &str = "cdj3k-emu needs write access to the raw disk device.\n\n\
     Click Retry to ask for an administrator password. \
     Access lasts while the disk is attached to the deck.";

impl DiskProvider for HostDiskProvider {
    fn list_removable(&self) -> Vec<PhysicalDisk> {
        super::linux_disk::list_removable()
    }

    fn unmount_host(&self, name: &str) -> std::io::Result<()> {
        super::linux_disk::unmount_disk(name)
    }

    fn remount_host(&self, name: &str) -> std::io::Result<()> {
        super::linux_disk::mount_disk(name)
    }
}

/// Hand QEMU the device as a descriptor it could not open itself, over the
/// monitor that can carry one.
pub(super) fn open_raw(disk: &PhysicalDisk, qmp_fd_socket: &Path) -> Result<RawMedium, UsbError> {
    let fd = open_device(&disk.bsd_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            UsbError::PermissionDenied(disk.bsd_path.clone())
        } else {
            UsbError::Io(e)
        }
    })?;
    let id = crate::qmp::add_fd(qmp_fd_socket, std::os::fd::AsFd::as_fd(&fd))?;
    Ok(RawMedium {
        filename: fdset_medium(id),
        format: None,
        fdset: Some(id),
    })
}

/// The medium for a block device QEMU holds as fdset `id`.
///
/// QEMU picks `host_device` over `file` by stat-ing the path, and
/// `/dev/fdset/N` is no block node, so the driver chain is spelled out.
fn fdset_medium(id: u64) -> String {
    let chain = serde_json::json!({
        "driver": "raw",
        "file": { "driver": "host_device", "filename": format!("/dev/fdset/{id}") },
    });
    format!("json:{chain}")
}

/// Open a whole disk read-write and exclusively, for QEMU to receive as a
/// descriptor.
///
/// `O_EXCL` on a whole disk fails with `EBUSY` while any of its partitions is
/// mounted, and while it is held nothing on the host can mount it, so the
/// guest never shares the filesystem with the host. A node this user cannot
/// open (`root:disk 0660`) is opened by udisks instead, under polkit's
/// `org.freedesktop.udisks2.open-device`.
fn open_device(dev_path: &str) -> std::io::Result<OwnedFd> {
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_EXCL)
        .open(dev_path)
    {
        Ok(f) => Ok(f.into()),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => udisks_open(dev_path),
        Err(e) => Err(e),
    }
}

/// `Block.OpenDevice` on the disk's udisks object, allowing polkit to ask.
fn udisks_open(dev_path: &str) -> std::io::Result<OwnedFd> {
    use std::collections::HashMap;
    use zbus::zvariant::Value;

    if !is_valid_block_device_path(dev_path) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("not a block device path: {dev_path}"),
        ));
    }
    let object = udisks_object_path(&dev_path["/dev/".len()..]);
    let options: HashMap<&str, Value> = HashMap::from([
        ("flags", Value::from(libc::O_EXCL)),
        ("auth.no_user_interaction", Value::from(false)),
    ]);
    let reply = zbus::blocking::Connection::system()
        .and_then(|conn| {
            conn.call_method(
                Some("org.freedesktop.UDisks2"),
                object.as_str(),
                Some("org.freedesktop.UDisks2.Block"),
                "OpenDevice",
                &("rw", options),
            )
        })
        .map_err(udisks_error)?;
    let fd: zbus::zvariant::OwnedFd = reply.body().deserialize().map_err(udisks_error)?;
    Ok(fd.into())
}

/// A refusal from polkit reads as a permission error; the rest keep udisks'
/// own message, which names the cause (`EBUSY` included).
fn udisks_error(e: zbus::Error) -> std::io::Error {
    let refused = matches!(&e, zbus::Error::MethodError(name, _, _)
        if name.as_str().starts_with("org.freedesktop.UDisks2.Error.NotAuthorized"));
    let kind = if refused {
        std::io::ErrorKind::PermissionDenied
    } else {
        std::io::ErrorKind::Other
    };
    std::io::Error::new(kind, format!("udisks OpenDevice: {e}"))
}

/// udisks' object path for a block device: letters and digits kept, every
/// other byte written as `_xx`.
fn udisks_object_path(name: &str) -> String {
    let mut path = String::from("/org/freedesktop/UDisks2/block_devices/");
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() {
            path.push(b as char);
        } else {
            path.push_str(&format!("_{b:02x}"));
        }
    }
    path
}

/// `/dev/<name>` where the name is a plain block-device name.
///
/// Rejects anything with a separator, a space or a quote, so the name maps to
/// exactly one udisks object however the path was obtained.
fn is_valid_block_device_path(path: &str) -> bool {
    let Some(name) = path.strip_prefix("/dev/") else {
        return false;
    };
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The format QEMU opens a virtual image with.
pub fn virtual_format(_img_path: &Path) -> &'static str {
    "raw"
}

/// Create a raw image of `size_bytes`, partitioned and formatted.
pub fn create_image(img_path: &Path, size_bytes: u64) -> std::io::Result<()> {
    super::create_raw_image(img_path, size_bytes)?;
    format_exfat(img_path)
}

/// Format a raw image file as MBR + exFAT, the same layout macOS produces.
///
/// All of it happens on the file: a `udisksctl loop-setup` node stays
/// `root:disk`, so a desktop user cannot partition or format it directly.
/// `sfdisk` and `mkfs.exfat` both take an ordinary file.
///
/// A rekordbox stick is MBR with one exFAT partition, and the guest's mount
/// script looks for `sdb1` before falling back to the whole device.
pub fn format_exfat(img_path: &Path) -> std::io::Result<()> {
    /// Where partitioners start a first partition.
    const ALIGN: u64 = 1 << 20;
    const SECTOR: u64 = 512;

    let total = std::fs::metadata(img_path)?.len();
    let part_sectors = total.saturating_sub(ALIGN) / SECTOR;
    if part_sectors == 0 {
        return Err(std::io::Error::other(format!(
            "{} is too small to hold a partition",
            img_path.display()
        )));
    }

    // `mkfs.exfat` has no offset option, so the filesystem is built in a file
    // of exactly the partition's size and moved into place.
    let fs_img = img_path.with_extension("exfat-part");
    let build = (|| -> std::io::Result<()> {
        std::fs::File::create(&fs_img)?.set_len(part_sectors * SECTOR)?;
        let status = Command::new(cdj3k_emu_platform::bundled::system_tool("mkfs.exfat"))
            .args(["-L", "REKORDBOX"])
            .arg(&fs_img)
            .stdout(std::process::Stdio::null())
            .status()
            .map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("mkfs.exfat not available ({e}); install exfatprogs"),
                )
            })?;
        if !status.success() {
            return Err(std::io::Error::other("mkfs.exfat failed"));
        }
        splice_allocated(&fs_img, img_path, ALIGN)
    })();
    let _ = std::fs::remove_file(&fs_img);
    build?;

    write_mbr(img_path, part_sectors)
}

/// One MBR partition of `sectors` starting at 1 MiB, typed exFAT.
fn write_mbr(img_path: &Path, sectors: u64) -> std::io::Result<()> {
    use std::io::Write;

    // `label: dos` is an MBR; type 7 is what every exFAT stick uses. The size
    // is given rather than left implicit so the partition matches the
    // filesystem already written into it.
    let mut sfdisk = Command::new(cdj3k_emu_platform::bundled::system_tool("sfdisk"))
        .arg(img_path)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("sfdisk not available ({e}); install util-linux"),
            )
        })?;
    sfdisk
        .stdin
        .take()
        .expect("piped")
        .write_all(format!("label: dos\nstart=2048, size={sectors}, type=7\n").as_bytes())?;
    let out = sfdisk.wait_with_output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(format!(
            "sfdisk: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// Copy the allocated regions of `src` into `dst` at `dst_offset`.
///
/// A fresh exFAT is a few megabytes of metadata in a file that is otherwise a
/// hole, so walking the extents keeps this independent of the stick's size: a
/// 1 TB image costs what a 32 GB one does. A filesystem that cannot answer
/// `SEEK_DATA` reports the whole file as allocated, which is slower and still
/// correct.
fn splice_allocated(src: &Path, dst: &Path, dst_offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    use std::os::unix::io::AsRawFd;

    let src_f = std::fs::File::open(src)?;
    let dst_f = std::fs::OpenOptions::new().write(true).open(dst)?;
    let end = src_f.metadata()?.len();
    let fd = src_f.as_raw_fd();
    let mut buf = vec![0u8; 1 << 20];
    let mut pos = 0u64;

    while pos < end {
        let data = unsafe { libc::lseek(fd, pos as libc::off_t, libc::SEEK_DATA) };
        let (from, to) = if data >= 0 {
            let hole = unsafe { libc::lseek(fd, data, libc::SEEK_HOLE) };
            (
                data as u64,
                if hole < 0 {
                    end
                } else {
                    (hole as u64).min(end)
                },
            )
        } else {
            match std::io::Error::last_os_error().raw_os_error() {
                // Nothing allocated past `pos`: the rest is a hole.
                Some(libc::ENXIO) => break,
                // The filesystem has no extent map. Take the lot.
                Some(libc::EINVAL) => (pos, end),
                _ => return Err(std::io::Error::last_os_error()),
            }
        };

        let mut at = from;
        while at < to {
            let n = buf.len().min((to - at) as usize);
            src_f.read_exact_at(&mut buf[..n], at)?;
            dst_f.write_all_at(&buf[..n], dst_offset + at)?;
            at += n as u64;
        }
        pos = to;
    }
    dst_f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::{is_valid_block_device_path, splice_allocated, udisks_object_path};
    use std::io::Write;
    use std::os::unix::fs::FileExt;

    /// An open file under a directory makes this process one of its holders.
    #[test]
    fn an_open_file_names_its_holder() {
        use super::super::linux_disk::holders_of;
        let dir = std::env::temp_dir().join(format!("cdj3k-holders-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = std::fs::File::create(dir.join("held")).unwrap();
        let me = std::fs::read_to_string("/proc/self/comm")
            .unwrap()
            .trim()
            .to_string();
        assert!(holders_of(&dir).contains(&me));
        drop(file);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(!holders_of(&dir).contains(&me));
    }

    #[test]
    fn accepts_the_shapes_lsblk_emits() {
        for ok in ["/dev/sdb", "/dev/sdb1", "/dev/nvme0n1p3", "/dev/mmcblk0"] {
            assert!(is_valid_block_device_path(ok), "{ok}");
        }
    }

    #[test]
    fn object_paths_escape_what_udisks_escapes() {
        assert_eq!(
            udisks_object_path("sdd"),
            "/org/freedesktop/UDisks2/block_devices/sdd"
        );
        assert_eq!(
            udisks_object_path("dm-0"),
            "/org/freedesktop/UDisks2/block_devices/dm_2d0"
        );
    }

    #[test]
    fn rejects_anything_that_is_not_a_plain_name() {
        for bad in [
            "/dev/sdb; rm -rf /",
            "/dev/../etc/passwd",
            "/dev/sd b",
            "/dev/sdb'",
            "/dev/",
            "sdb",
            "/tmp/evil",
            "/dev/disk/by-id/usb-x",
        ] {
            assert!(!is_valid_block_device_path(bad), "{bad}");
        }
    }

    /// A hole in the source stays a hole in the destination, and the written
    /// regions land at the right offset.
    #[test]
    fn allocated_regions_move_and_holes_do_not() {
        let dir = std::env::temp_dir().join(format!("cdj3k-splice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src");
        let dst = dir.join("dst");
        const LEN: u64 = 8 << 20;
        const OFFSET: u64 = 1 << 20;

        // Data at the front and near the back, a hole between them.
        let f = std::fs::File::create(&src).unwrap();
        f.set_len(LEN).unwrap();
        f.write_all_at(b"head", 0).unwrap();
        f.write_all_at(b"tail", LEN - 4).unwrap();
        drop(f);
        std::fs::File::create(&dst)
            .unwrap()
            .set_len(LEN + OFFSET)
            .unwrap();

        splice_allocated(&src, &dst, OFFSET).unwrap();

        let d = std::fs::File::open(&dst).unwrap();
        let mut got = [0u8; 4];
        d.read_exact_at(&mut got, OFFSET).unwrap();
        assert_eq!(&got, b"head");
        d.read_exact_at(&mut got, OFFSET + LEN - 4).unwrap();
        assert_eq!(&got, b"tail");
        // Nothing before the offset, and the hole is still zeroes.
        d.read_exact_at(&mut got, 0).unwrap();
        assert_eq!(&got, &[0; 4]);
        d.read_exact_at(&mut got, OFFSET + (LEN / 2)).unwrap();
        assert_eq!(&got, &[0; 4]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A source with nothing in it writes nothing: the destination keeps the
    /// contents it already had past the offset.
    #[test]
    fn an_empty_source_leaves_the_destination_alone() {
        let dir = std::env::temp_dir().join(format!("cdj3k-splice-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::File::create(&src)
            .unwrap()
            .set_len(4 << 20)
            .unwrap();
        let mut d = std::fs::File::create(&dst).unwrap();
        d.write_all(b"keep me").unwrap();
        d.set_len(8 << 20).unwrap();
        drop(d);

        splice_allocated(&src, &dst, 0).unwrap();

        let d = std::fs::File::open(&dst).unwrap();
        let mut got = [0u8; 7];
        d.read_exact_at(&mut got, 0).unwrap();
        assert_eq!(&got, b"keep me");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
