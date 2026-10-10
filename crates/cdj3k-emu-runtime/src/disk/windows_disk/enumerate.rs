//! Find the whole disks that may be offered, and the volumes on each.

use std::collections::HashMap;

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{
    FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetVolumeInformationW,
    GetVolumePathNamesForVolumeNameW,
};
use windows_sys::Win32::System::Ioctl::{
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_QUERY_PROPERTY,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

use super::{ioctl, open, wide};
use crate::disk::windows_parse as parse;
use crate::disk::{human_size, PhysicalDisk};

/// `IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`, which windows-sys does not carry.
const IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS: u32 = 0x0056_0000;

/// How many disk numbers are probed, starting at 0.
const MAX_DRIVES: u32 = 32;

/// A volume and the place where it is mounted.
#[derive(Clone, Debug)]
pub(super) struct Volume {
    /// The `\\?\Volume{guid}` path, which is the path that a handle opens.
    pub(super) path: String,
    /// The first drive letter or folder it is mounted at.
    pub(super) mount: Option<String>,
}

/// List the whole disks that the user could plausibly give to the deck.
pub fn list_removable() -> Vec<PhysicalDisk> {
    // A disk that Windows runs from is never offered. When the system disk
    // cannot be identified, no disk is offered.
    let Some(system) = system_disks() else {
        return Vec::new();
    };
    let volumes = volumes_by_disk();
    (0..MAX_DRIVES)
        .filter_map(|n| describe(n, system.contains(&n), volumes.get(&n)))
        .collect()
}

fn describe(n: u32, is_system: bool, volumes: Option<&Vec<Volume>>) -> Option<PhysicalDisk> {
    let path = parse::drive_path(n);
    let h = open(&path, 0).ok()?;

    let mut buf = [0u8; 1024];
    // A zeroed query selects PropertyId StorageDeviceProperty and QueryType
    // PropertyStandardQuery.
    ioctl(&h, IOCTL_STORAGE_QUERY_PROPERTY, &[0u8; 12], &mut buf).ok()?;
    let descriptor = parse::parse_descriptor(&buf)?;

    // An empty card reader slot answers with no medium.
    let mut geometry = [0u8; 256];
    ioctl(&h, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut geometry).ok()?;
    let size_bytes = parse::geometry_disk_size(&geometry)?;

    if !parse::is_eligible(descriptor.bus_type, size_bytes, is_system) {
        return None;
    }
    let volume_label = volumes
        .into_iter()
        .flatten()
        .find_map(|v| volume_label(&v.path))
        .unwrap_or_default();
    Some(PhysicalDisk {
        label: format!(
            "{} ({})",
            parse::display_name(&volume_label, &descriptor, n),
            human_size(size_bytes)
        ),
        bsd_name: parse::drive_name(n),
        bsd_path: path,
        size_bytes,
    })
}

/// Return the disks that the Windows directory lives on.
fn system_disks() -> Option<Vec<u32>> {
    let mut buf = [0u16; 260];
    // SAFETY: the buffer's length is passed.
    let len = unsafe { GetSystemDirectoryW(buf.as_mut_ptr(), buf.len() as u32) } as usize;
    let dir = String::from_utf16(buf.get(..len)?).ok()?;
    let drive: String = dir.chars().take(2).collect();
    if drive.len() != 2 || !drive.ends_with(':') {
        return None;
    }
    let disks = disks_of_volume(&format!(r"\\.\{drive}"))?;
    (!disks.is_empty()).then_some(disks)
}

/// Return the numbers of the disks that a volume spans.
fn disks_of_volume(path: &str) -> Option<Vec<u32>> {
    let h = open(path, 0).ok()?;
    let mut buf = [0u8; 1024];
    ioctl(&h, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], &mut buf).ok()?;
    Some(parse::parse_disk_extents(&buf))
}

/// List every mounted volume, grouped under each disk that it lies on.
fn volumes_by_disk() -> HashMap<u32, Vec<Volume>> {
    let mut by_disk: HashMap<u32, Vec<Volume>> = HashMap::new();
    let mut name = [0u16; 260];
    // SAFETY: the buffer's length is passed.
    let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), name.len() as u32) };
    if find == INVALID_HANDLE_VALUE {
        return by_disk;
    }
    loop {
        let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        let with_slash = String::from_utf16_lossy(&name[..len]);
        let path = with_slash.trim_end_matches('\\').to_string();
        if parse::is_volume_path(&path) {
            if let Some(disks) = disks_of_volume(&path) {
                let volume = Volume {
                    mount: mount_point(&with_slash),
                    path,
                };
                for d in disks {
                    by_disk.entry(d).or_default().push(volume.clone());
                }
            }
        }
        // SAFETY: `find` is the live search handle; the buffer's length is passed.
        if unsafe { FindNextVolumeW(find, name.as_mut_ptr(), name.len() as u32) } == 0 {
            break;
        }
    }
    // SAFETY: `find` came from FindFirstVolumeW.
    unsafe { FindVolumeClose(find) };
    by_disk
}

pub(super) fn volumes_of(disk: u32) -> Vec<Volume> {
    volumes_by_disk().remove(&disk).unwrap_or_default()
}

/// Return the first place where a volume is mounted, such as `E:\`.
fn mount_point(volume_with_slash: &str) -> Option<String> {
    let name = wide(volume_with_slash);
    let mut buf = [0u16; 512];
    let mut len = 0u32;
    // SAFETY: `name` is NUL-terminated; the buffer's length is passed.
    let ok = unsafe {
        GetVolumePathNamesForVolumeNameW(
            name.as_ptr(),
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut len,
        )
    };
    if ok == 0 {
        return None;
    }
    let first = buf.split(|&c| c == 0).next()?;
    (!first.is_empty()).then(|| String::from_utf16_lossy(first))
}

/// Return a volume's label, when its filesystem is readable.
pub(super) fn volume_label(volume_path: &str) -> Option<String> {
    let root = wide(&format!("{volume_path}\\"));
    let mut label = [0u16; 262];
    // SAFETY: `root` is NUL-terminated; the label buffer's length is passed and
    // the other outputs are not wanted.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
        )
    };
    if ok == 0 {
        return None;
    }
    let len = label.iter().position(|&c| c == 0)?;
    let label = String::from_utf16_lossy(&label[..len]);
    (!label.trim().is_empty()).then_some(label)
}
