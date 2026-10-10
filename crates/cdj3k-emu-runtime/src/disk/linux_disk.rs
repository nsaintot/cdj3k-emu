//! Removable-disk enumeration and mounting on Linux.
//!
//! This module runs the `lsblk` and `udisksctl` CLIs. `udisks2` asks polkit
//! whether the user may unmount a device. `lsblk --pairs` needs no JSON parser.
//!
//! The module is compiled on every host so that its parsing is tested
//! everywhere, but only Linux calls into it. On a host with no `lsblk` the enumeration finds nothing.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::BTreeMap;
use std::io;
use std::process::Command;

use super::{human_size, PhysicalDisk};

/// List the whole disks that the user could plausibly give to the deck.
pub fn list_removable() -> Vec<PhysicalDisk> {
    let Some(out) = lsblk(&["-d", "-o", "NAME,PATH,SIZE,RM,HOTPLUG,TYPE,MODEL,VENDOR"]) else {
        return Vec::new();
    };

    let mut disks = Vec::new();
    for row in out {
        // `RM` is set for media that can leave the drive, `HOTPLUG` for a
        // drive that can leave the machine. A USB stick usually reports the
        // second and not the first, so either will do.
        let removable =
            row.get("RM").is_some_and(|v| v == "1") || row.get("HOTPLUG").is_some_and(|v| v == "1");
        if !removable || row.get("TYPE").map(String::as_str) != Some("disk") {
            continue;
        }
        let Some(path) = row.get("PATH") else {
            continue;
        };
        let name = row.get("NAME").cloned().unwrap_or_default();
        let size_bytes = row.get("SIZE").and_then(|s| s.parse().ok()).unwrap_or(0);
        if size_bytes == 0 {
            // An empty card reader slot enumerates with no medium.
            continue;
        }

        disks.push(PhysicalDisk {
            label: format!("{} ({})", human_name(&row, &name), human_size(size_bytes)),
            bsd_name: name,
            bsd_path: path.clone(),
            size_bytes,
        });
    }
    disks
}

/// Unmount every mounted partition of a disk, so the guest can own it.
///
/// `udisksctl` asks polkit, which grants this for removable media without a
/// password on a normal desktop.
///
/// If a program still has a file open on a partition, the unmount fails with
/// `ResourceBusy` and names the programs that hold it, so the user knows what
/// to close.
pub fn unmount_disk(name: &str) -> io::Result<()> {
    for (part, mountpoint) in mounted_partitions(name) {
        if let Err(e) = udisksctl("unmount", &part) {
            if !e.to_string().contains("DeviceBusy") {
                return Err(e);
            }
            let holders = holders_of(std::path::Path::new(&mountpoint));
            let by = if holders.is_empty() {
                String::from("another program")
            } else {
                holders.join(", ")
            };
            return Err(io::Error::new(
                io::ErrorKind::ResourceBusy,
                format!(
                    "{mountpoint} is in use by {by}. Close it there, then choose the disk again."
                ),
            ));
        }
    }
    Ok(())
}

/// Return the names of this user's processes that have a file or working
/// directory under `mountpoint`, as `/proc/<pid>/comm` gives them, without
/// duplicates.
pub(super) fn holders_of(mountpoint: &std::path::Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return names;
    };
    for proc in procs.flatten() {
        let dir = proc.path();
        if !proc
            .file_name()
            .to_string_lossy()
            .bytes()
            .all(|b| b.is_ascii_digit())
        {
            continue;
        }
        let fds = std::fs::read_dir(dir.join("fd"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|fd| fd.path());
        let holds = std::iter::once(dir.join("cwd"))
            .chain(fds)
            .filter_map(|link| std::fs::read_link(link).ok())
            .any(|target| target.starts_with(mountpoint));
        if !holds {
            continue;
        }
        if let Ok(comm) = std::fs::read_to_string(dir.join("comm")) {
            let comm = comm.trim().to_string();
            if !names.contains(&comm) {
                names.push(comm);
            }
        }
    }
    names
}

/// Hand the disk back to the desktop.
///
/// This is best effort: the user may have unplugged the disk, and the guest
/// has already ejected it, so a partition that does not mount is not an error.
pub fn mount_disk(name: &str) -> io::Result<()> {
    for part in partitions(name) {
        let _ = udisksctl("mount", &part);
    }
    Ok(())
}

// ── Plumbing ──────────────────────────────────────────────────────────────────

fn udisksctl(verb: &str, dev: &str) -> io::Result<()> {
    let out = Command::new(cdj3k_emu_platform::bundled::system_tool("udisksctl"))
        .args([verb, "-b", dev, "--no-user-interaction"])
        .output()
        .map_err(|e| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("udisksctl is not available ({e}); install udisks2"),
            )
        })?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    // A partition that is already unmounted counts as success.
    if err.contains("NotMounted") || err.contains("not mounted") {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "udisksctl {verb} {dev}: {}",
        err.trim()
    )))
}

/// Return the partition device paths of one disk, in order.
fn partitions(disk: &str) -> Vec<String> {
    lsblk(&["-o", "PATH,TYPE", &format!("/dev/{disk}")])
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.get("TYPE").map(String::as_str) == Some("part"))
        .filter_map(|r| r.get("PATH").cloned())
        .collect()
}

/// Return the mounted partitions of one disk as (device path, mount point)
/// pairs.
fn mounted_partitions(disk: &str) -> Vec<(String, String)> {
    lsblk(&["-o", "PATH,TYPE,MOUNTPOINT", &format!("/dev/{disk}")])
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.get("TYPE").map(String::as_str) == Some("part"))
        .filter_map(|r| {
            let mountpoint = r.get("MOUNTPOINT").filter(|m| !m.is_empty())?;
            Some((r.get("PATH")?.clone(), mountpoint.clone()))
        })
        .collect()
}

/// Run `lsblk --pairs` and parse its `KEY="value"` rows.
fn lsblk(args: &[&str]) -> Option<Vec<BTreeMap<String, String>>> {
    let out = Command::new("lsblk")
        .arg("-P")
        .arg("-b")
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(parse_pairs)
            .collect(),
    )
}

/// Parse a row such as `NAME="sda" SIZE="512" MODEL="Some Disk"` into a map.
///
/// Values are double-quoted and may contain spaces, so the parser follows the
/// quotes.
fn parse_pairs(line: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut rest = line.trim();
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq].trim().to_string();
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else { break };
        out.insert(key, after[..end].trim().to_string());
        rest = &after[end + 1..];
    }
    out
}

/// Return the most recognisable name for a disk: its filesystem label, or else
/// the name that the drive reports for itself.
fn human_name(row: &BTreeMap<String, String>, fallback: &str) -> String {
    for key in ["LABEL", "MODEL", "VENDOR"] {
        if let Some(v) = row.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            return v.to_string();
        }
    }
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_with_spaces_in_the_value_survive() {
        let row = parse_pairs(r#"NAME="sdb" PATH="/dev/sdb" MODEL="Cruzer Blade" RM="1""#);
        assert_eq!(row.get("NAME").unwrap(), "sdb");
        assert_eq!(row.get("PATH").unwrap(), "/dev/sdb");
        assert_eq!(row.get("MODEL").unwrap(), "Cruzer Blade");
        assert_eq!(row.get("RM").unwrap(), "1");
    }

    #[test]
    fn an_empty_value_is_still_a_key() {
        let row = parse_pairs(r#"PATH="/dev/sdb1" TYPE="part" MOUNTPOINT="""#);
        assert_eq!(row.get("MOUNTPOINT").map(String::as_str), Some(""));
        assert_eq!(row.get("TYPE").unwrap(), "part");
    }

    #[test]
    fn a_truncated_row_does_not_hang_or_panic() {
        assert!(parse_pairs(r#"NAME="sdb" MODEL="unterminated"#).contains_key("NAME"));
        assert!(parse_pairs("").is_empty());
    }

    #[test]
    fn the_name_prefers_what_is_written_on_the_disk() {
        let mut row = BTreeMap::new();
        row.insert("VENDOR".into(), "Generic".into());
        assert_eq!(human_name(&row, "sdb"), "Generic");
        row.insert("MODEL".into(), "Cruzer".into());
        assert_eq!(human_name(&row, "sdb"), "Cruzer");
        row.insert("LABEL".into(), "PIONEER".into());
        assert_eq!(human_name(&row, "sdb"), "PIONEER");
        assert_eq!(human_name(&BTreeMap::new(), "sdb"), "sdb");
    }
}
