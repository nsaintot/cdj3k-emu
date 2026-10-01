//! The Windows disk adapter's logic that needs no Windows API: descriptor and
//! extent parsing, which disks may be offered, the PowerShell the elevated
//! steps run, and the messages.
//!
//! Compiled on every host so it is tested everywhere; only Windows calls into
//! it.
#![cfg_attr(not(windows), allow(dead_code))]

/// `STORAGE_BUS_TYPE` values a stick or card reader reports.
pub const BUS_USB: u32 = 7;
pub const BUS_SD: u32 = 12;
pub const BUS_MMC: u32 = 13;

/// The parts of a `STORAGE_DEVICE_DESCRIPTOR` the disk list uses.
#[derive(Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub bus_type: u32,
    pub removable: bool,
    pub vendor: String,
    pub product: String,
}

fn u32_at(buf: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(off..off + 4)?.try_into().ok()?))
}

fn i64_at(buf: &[u8], off: usize) -> Option<i64> {
    Some(i64::from_le_bytes(buf.get(off..off + 8)?.try_into().ok()?))
}

/// The NUL-terminated ASCII string at `off`; offset 0 means "absent".
fn cstr_at(buf: &[u8], off: u32) -> String {
    let Some(tail) = (off != 0).then(|| buf.get(off as usize..)).flatten() else {
        return String::new();
    };
    let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
    String::from_utf8_lossy(&tail[..end]).trim().to_string()
}

/// Read a `STORAGE_DEVICE_DESCRIPTOR` as `IOCTL_STORAGE_QUERY_PROPERTY`
/// returns it: a 36-byte header whose offsets point into the same buffer.
pub fn parse_descriptor(buf: &[u8]) -> Option<Descriptor> {
    let size = u32_at(buf, 4)? as usize;
    if size < 36 || size > buf.len() {
        return None;
    }
    let buf = &buf[..size];
    Some(Descriptor {
        removable: *buf.get(10)? != 0,
        vendor: cstr_at(buf, u32_at(buf, 12)?),
        product: cstr_at(buf, u32_at(buf, 16)?),
        bus_type: u32_at(buf, 28)?,
    })
}

/// `DiskSize` of a `DISK_GEOMETRY_EX`: a 24-byte `DISK_GEOMETRY`, then the
/// size in bytes.
pub fn geometry_disk_size(buf: &[u8]) -> Option<u64> {
    u64::try_from(i64_at(buf, 24)?).ok()
}

/// Disk numbers of a `VOLUME_DISK_EXTENTS`: a count, 4 bytes of padding, then
/// 24-byte extents that start with the disk number.
pub fn parse_disk_extents(buf: &[u8]) -> Vec<u32> {
    let count = u32_at(buf, 0).unwrap_or(0) as usize;
    let mut disks = Vec::new();
    for i in 0..count {
        let Some(n) = u32_at(buf, 8 + i * 24) else {
            break;
        };
        if !disks.contains(&n) {
            disks.push(n);
        }
    }
    disks
}

/// Whether a disk may be offered to the deck: a USB, SD or MMC bus, a medium
/// in it, and not a disk Windows runs from.
pub fn is_eligible(bus_type: u32, size_bytes: u64, is_system: bool) -> bool {
    matches!(bus_type, BUS_USB | BUS_SD | BUS_MMC) && size_bytes > 0 && !is_system
}

/// `PhysicalDrive<N>`.
pub fn drive_name(n: u32) -> String {
    format!("PhysicalDrive{n}")
}

/// `\\.\PhysicalDrive<N>`.
pub fn drive_path(n: u32) -> String {
    format!(r"\\.\PhysicalDrive{n}")
}

/// The number in `PhysicalDrive<N>`, and nothing else.
pub fn drive_number(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("PhysicalDrive")?;
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// `\\?\Volume{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}` without a trailing
/// separator: the shape `FindFirstVolumeW` yields, and the only shape the
/// elevated script is given.
pub fn is_volume_path(p: &str) -> bool {
    let Some(guid) = p
        .strip_prefix(r"\\?\Volume{")
        .and_then(|g| g.strip_suffix('}'))
    else {
        return false;
    };
    let groups: Vec<&str> = guid.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(g, n)| g.len() == n && g.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// `S-1-<digits>(-<digits>)*`.
pub fn is_valid_sid(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("S-1-") else {
        return false;
    };
    !rest.is_empty()
        && rest
            .split('-')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The most recognisable name for a disk: the volume label, else the drive's
/// own name, else its number.
pub fn display_name(volume_label: &str, d: &Descriptor, n: u32) -> String {
    let drive = format!("{} {}", d.vendor, d.product);
    let name = [volume_label.trim(), drive.trim()]
        .into_iter()
        .find(|s| !s.is_empty())
        .map(str::to_string);
    name.unwrap_or_else(|| drive_name(n))
}

/// Sizes as a disk vendor writes them: powers of ten.
pub fn human_size(bytes: u64) -> String {
    const GB: f64 = 1_000_000_000.0;
    const MB: f64 = 1_000_000.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", b / MB)
    }
}

/// What a busy volume reads as.
pub fn busy_message(mount: &str, holders: &[String]) -> String {
    let by = if holders.is_empty() {
        String::from("another program")
    } else {
        holders.join(", ")
    };
    format!("{mount} is in use by {by}. Close it there, then choose the disk again.")
}

/// A VHDX file starts with the signature `vhdxfile`.
pub fn is_vhdx(header: &[u8]) -> bool {
    header.starts_with(b"vhdxfile")
}

// ── PowerShell ───────────────────────────────────────────────────────────────

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &b)| acc | ((b as u32) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A command line that runs `script` in Windows PowerShell.
///
/// `-EncodedCommand` takes the script as base64 of UTF-16LE, which needs no
/// quoting through the elevation prompt. The first word is the executable.
pub fn powershell_command(script: &str) -> String {
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    format!(
        "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {}",
        base64(&utf16)
    )
}

/// A PowerShell single-quoted string literal.
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Give `sid` read and write on a disk and its volumes.
///
/// The security descriptor of a device object lasts until the device is
/// removed, so one prompt covers every open until the disk is unplugged. The
/// disk must take the grant; a volume that refuses is left to the ordinary
/// open, which a removable volume allows a standard user. A failure writes its
/// message to `err_file` and exits 1.
pub fn grant_script(sid: &str, disk: &str, volumes: &[String], err_file: &str) -> String {
    let volumes = volumes
        .iter()
        .map(|v| ps_quote(v))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "$ErrorActionPreference='Stop';\
         $sid=New-Object Security.Principal.SecurityIdentifier({sid});\
         $rights=[Security.AccessControl.FileSystemRights]'Read,Write,Synchronize';\
         $open=[Security.AccessControl.FileSystemRights]'ReadPermissions,ChangePermissions';\
         function Grant($p){{\
         $f=New-Object IO.FileStream($p,'Open',$open,'ReadWrite',8,'None');\
         try{{$a=$f.GetAccessControl();\
         $a.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid,$rights,'Allow')));\
         $f.SetAccessControl($a)}}finally{{$f.Dispose()}}}};\
         try{{Grant {disk};\
         foreach($v in @({volumes})){{try{{Grant $v}}catch{{}}}}}}\
         catch{{[IO.File]::WriteAllText({err},$_.Exception.Message);exit 1}}",
        sid = ps_quote(sid),
        disk = ps_quote(disk),
        err = ps_quote(err_file),
    )
}

/// Partition and format an empty VHDX the way the Linux adapter lays out an
/// image: MBR, one partition from 1 MiB to the end, exFAT labelled REKORDBOX.
///
/// The image is attached without a drive letter so Explorer never offers it,
/// and always detached. A failure writes its message to `err_file`
/// and exits 1.
pub fn format_script(image: &str, err_file: &str) -> String {
    format!(
        "$ErrorActionPreference='Stop';$p={image};$m=$null;\
         try{{\
         $m=Mount-DiskImage -ImagePath $p -StorageType VHDX -NoDriveLetter -PassThru;\
         $d=$m|Get-Disk;\
         $d|Initialize-Disk -PartitionStyle MBR;\
         $d|New-Partition -UseMaximumSize -Offset 1048576 -MbrType IFS|\
         Format-Volume -FileSystem exFAT -NewFileSystemLabel REKORDBOX -Confirm:$false|Out-Null}}\
         catch{{[IO.File]::WriteAllText({err},$_.Exception.Message);exit 1}}\
         finally{{if($m){{Dismount-DiskImage -ImagePath $p|Out-Null}}}}",
        image = ps_quote(image),
        err = ps_quote(err_file),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(bus: u32, removable: u8, vendor: &str, product: &str) -> Vec<u8> {
        let mut buf = vec![0u8; 36];
        let v_off = buf.len() as u32;
        buf.extend(vendor.bytes());
        buf.push(0);
        let p_off = buf.len() as u32;
        buf.extend(product.bytes());
        buf.push(0);
        let size = buf.len() as u32;
        buf[0..4].copy_from_slice(&1u32.to_le_bytes());
        buf[4..8].copy_from_slice(&size.to_le_bytes());
        buf[10] = removable;
        buf[12..16].copy_from_slice(&v_off.to_le_bytes());
        buf[16..20].copy_from_slice(&p_off.to_le_bytes());
        buf[28..32].copy_from_slice(&bus.to_le_bytes());
        buf
    }

    #[test]
    fn a_descriptor_yields_bus_and_names() {
        let d = parse_descriptor(&descriptor(BUS_USB, 1, "SanDisk ", "Cruzer Blade ")).unwrap();
        assert_eq!(
            d,
            Descriptor {
                bus_type: BUS_USB,
                removable: true,
                vendor: "SanDisk".into(),
                product: "Cruzer Blade".into(),
            }
        );
    }

    #[test]
    fn absent_strings_are_empty_and_truncation_is_refused() {
        let mut buf = descriptor(BUS_SD, 0, "", "");
        buf[12..20].fill(0);
        let d = parse_descriptor(&buf).unwrap();
        assert!(d.vendor.is_empty() && d.product.is_empty() && !d.removable);
        assert!(parse_descriptor(&buf[..20]).is_none());
        assert!(parse_descriptor(&[]).is_none());
        // A size field larger than what was returned.
        let mut short = buf.clone();
        short[4..8].copy_from_slice(&9999u32.to_le_bytes());
        assert!(parse_descriptor(&short).is_none());
    }

    #[test]
    fn a_string_offset_past_the_buffer_is_empty() {
        let mut buf = descriptor(BUS_USB, 1, "A", "B");
        buf[12..16].copy_from_slice(&500u32.to_le_bytes());
        assert_eq!(parse_descriptor(&buf).unwrap().vendor, "");
    }

    #[test]
    fn geometry_size_follows_the_geometry() {
        let mut buf = vec![0u8; 32];
        buf[24..32].copy_from_slice(&32_015_679_488i64.to_le_bytes());
        assert_eq!(geometry_disk_size(&buf), Some(32_015_679_488));
        buf[24..32].copy_from_slice(&(-1i64).to_le_bytes());
        assert_eq!(geometry_disk_size(&buf), None);
        assert_eq!(geometry_disk_size(&buf[..30]), None);
    }

    #[test]
    fn extents_list_each_disk_once() {
        let mut buf = vec![0u8; 8 + 3 * 24];
        buf[0..4].copy_from_slice(&3u32.to_le_bytes());
        for (i, n) in [2u32, 2, 5].into_iter().enumerate() {
            buf[8 + i * 24..12 + i * 24].copy_from_slice(&n.to_le_bytes());
        }
        assert_eq!(parse_disk_extents(&buf), vec![2, 5]);
        // A count the buffer cannot back stops at what is there.
        buf[0..4].copy_from_slice(&50u32.to_le_bytes());
        assert_eq!(parse_disk_extents(&buf), vec![2, 5]);
        assert!(parse_disk_extents(&[]).is_empty());
    }

    #[test]
    fn only_sticks_and_cards_that_are_not_the_system_are_offered() {
        assert!(is_eligible(BUS_USB, 32_000_000_000, false));
        assert!(is_eligible(BUS_SD, 1, false));
        assert!(is_eligible(BUS_MMC, 1, false));
        assert!(!is_eligible(BUS_USB, 32_000_000_000, true));
        assert!(!is_eligible(BUS_USB, 0, false));
        // SATA (11), NVMe (17), file-backed virtual (15).
        for bus in [11, 17, 15, 0] {
            assert!(!is_eligible(bus, 32_000_000_000, false), "{bus}");
        }
    }

    #[test]
    fn drive_names_round_trip_and_nothing_else_parses() {
        assert_eq!(drive_number(&drive_name(3)), Some(3));
        assert_eq!(drive_path(12), r"\\.\PhysicalDrive12");
        for bad in [
            "PhysicalDrive",
            "PhysicalDrive1; calc",
            "PhysicalDrive-1",
            r"\\.\PhysicalDrive1",
            "disk2",
            "PhysicalDrive12345",
            "",
        ] {
            assert_eq!(drive_number(bad), None, "{bad}");
        }
    }

    #[test]
    fn volume_paths_are_guids_and_nothing_else() {
        assert!(is_volume_path(
            r"\\?\Volume{0a1b2c3d-4e5f-6789-abcd-ef0123456789}"
        ));
        for bad in [
            r"\\?\Volume{0a1b2c3d-4e5f-6789-abcd-ef0123456789}\",
            r"\\?\Volume{0a1b2c3d-4e5f-6789-abcd-ef012345678}",
            r"\\?\Volume{0a1b2c3d-4e5f-6789-abcd-ef012345678g}",
            r"\\?\Volume{0a1b2c3d-4e5f-6789-abcd-ef0123456789}'; calc; '",
            r"\\.\C:",
            "",
        ] {
            assert!(!is_volume_path(bad), "{bad}");
        }
    }

    #[test]
    fn sids_are_dash_separated_numbers() {
        assert!(is_valid_sid(
            "S-1-5-21-1004336348-1177238915-682003330-1000"
        ));
        for bad in ["S-1-", "S-1-5-'x", "S-2-5", "S-1-5--1", "", "S-1-5 "] {
            assert!(!is_valid_sid(bad), "{bad}");
        }
    }

    #[test]
    fn the_name_prefers_the_label_then_the_drive() {
        let d = Descriptor {
            bus_type: BUS_USB,
            removable: true,
            vendor: "SanDisk".into(),
            product: "Cruzer".into(),
        };
        assert_eq!(display_name("PIONEER", &d, 2), "PIONEER");
        assert_eq!(display_name(" ", &d, 2), "SanDisk Cruzer");
        let bare = Descriptor {
            vendor: String::new(),
            product: String::new(),
            ..d
        };
        assert_eq!(display_name("", &bare, 2), "PhysicalDrive2");
    }

    #[test]
    fn sizes_read_the_way_the_vendor_writes_them() {
        assert_eq!(human_size(32_000_000_000), "32.0 GB");
        assert_eq!(human_size(512_000_000), "512 MB");
    }

    #[test]
    fn busy_names_its_holders() {
        assert_eq!(
            busy_message(r"E:\", &["explorer.exe".into(), "Notepad".into()]),
            r"E:\ is in use by explorer.exe, Notepad. Close it there, then choose the disk again."
        );
        assert!(busy_message(r"E:\", &[]).contains("another program"));
    }

    #[test]
    fn vhdx_is_recognised_by_signature() {
        assert!(is_vhdx(b"vhdxfile\0\0"));
        assert!(!is_vhdx(b"vhdxfil"));
        assert!(!is_vhdx(&[0u8; 512]));
    }

    #[test]
    fn base64_matches_the_rfc_vectors() {
        for (input, want) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), want, "{input}");
        }
    }

    #[test]
    fn the_command_encodes_the_script_as_utf16() {
        assert_eq!(
            powershell_command("a"),
            "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand YQA="
        );
    }

    #[test]
    fn quotes_in_a_path_cannot_leave_the_literal() {
        assert_eq!(ps_quote(r"C:\it's\x.vhdx"), r"'C:\it''s\x.vhdx'");
    }

    #[test]
    fn the_grant_names_the_disk_and_every_volume() {
        let v = r"\\?\Volume{0a1b2c3d-4e5f-6789-abcd-ef0123456789}".to_string();
        let s = grant_script(
            "S-1-5-21-1",
            r"\\.\PhysicalDrive2",
            std::slice::from_ref(&v),
            r"C:\t\e.err",
        );
        assert!(s.contains(r"SecurityIdentifier('S-1-5-21-1')"));
        assert!(s.contains(r"Grant '\\.\PhysicalDrive2'"));
        assert!(s.contains(&format!("@('{v}')")));
        assert!(s.contains(r"WriteAllText('C:\t\e.err'"));
        assert!(grant_script("S-1-5-21-1", "d", &[], "e").contains("@()"));
    }

    #[test]
    fn the_format_script_lays_out_the_same_partition_as_linux() {
        let s = format_script(r"C:\usb's.vhdx", r"C:\usb's.vhdx.err");
        assert!(s.contains(r"$p='C:\usb''s.vhdx'"));
        assert!(s.contains("-PartitionStyle MBR"));
        assert!(s.contains("-Offset 1048576 -MbrType IFS"));
        assert!(s.contains("-FileSystem exFAT -NewFileSystemLabel REKORDBOX"));
        assert!(s.contains("Dismount-DiskImage"));
    }
}
