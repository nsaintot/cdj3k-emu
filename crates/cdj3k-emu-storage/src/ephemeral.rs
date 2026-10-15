//! Ephemeral slots. `--instance N --ephemeral` runs slot N as slot
//! N + [`MAX_INSTANCES`], a copy that the app deletes when the window closes,
//! so the session never changes slot N.
//!
//! The copy holds slot N's settings without its MAC and USB device, and its
//! mod list and installed mods. Its kernel, initramfs and eMMC are hard links
//! to slot N's files: the guest writes the eMMC through an overlay, and the
//! link takes the same lock as slot N's eMMC, so slot N and its ephemeral copy
//! never run at once. [`remove_leftovers`] runs at every start of the app and
//! deletes the copies that crashed sessions left.

use std::fs;
use std::io;
use std::path::Path;

use cdj3k_emu_platform::menu_state::MAX_INSTANCES;

use crate::{instance_dir, SlotClaim};

/// The files linked from slot N.
const LINKED: [&str; 3] = ["Image", "initramfs-patched.cpio.gz", "emmc.qcow2"];
/// The settings that the copy does not take from slot N: the MAC, so that the
/// two decks never share one, and the USB device.
const DROPPED_SETTINGS: [&str; 3] = ["mac", "usb_virtual_path", "usb_physical_bsd"];
/// The slot's claim file, which [`prepare`] keeps.
const CLAIM_FILE: &str = ".open";

/// The slot that an ephemeral session of slot `base` runs as.
pub fn slot_for(base: u32) -> u32 {
    base + MAX_INSTANCES
}

/// Whether `instance_id` is an ephemeral slot.
pub fn is_ephemeral(instance_id: u32) -> bool {
    instance_id > MAX_INSTANCES
}

/// Fill ephemeral slot `slot`, which this process has claimed, from slot
/// `base`.
pub fn prepare(base: u32, slot: u32) -> io::Result<()> {
    assert!(is_ephemeral(slot) && !is_ephemeral(base));
    let from = instance_dir(base);
    let to = instance_dir(slot);
    clear_except_claim(&to)?;
    for name in LINKED {
        let src = from.join(name);
        if src.exists() {
            fs::hard_link(&src, to.join(name))?;
        }
    }
    let settings = fs::read_to_string(from.join("settings.txt")).unwrap_or_default();
    fs::write(to.join("settings.txt"), without_dropped(&settings))?;
    for rel in ["mods/list.txt", "mods/installed"] {
        let src = from.join(rel);
        if src.exists() {
            copy_tree(&src, &to.join(rel))?;
        }
    }
    Ok(())
}

/// Delete ephemeral slot `slot` and its runtime folder. The hard links go;
/// slot N's files stay.
pub fn remove(slot: u32) -> io::Result<()> {
    assert!(is_ephemeral(slot));
    let _ = fs::remove_dir_all(cdj3k_emu_platform::runtime_paths::instance_dir(slot));
    match fs::remove_dir_all(instance_dir(slot)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Delete the ephemeral slots that no process holds, which crashed sessions
/// left. Returns the slots it deleted.
pub fn remove_leftovers() -> Vec<u32> {
    let mut removed = Vec::new();
    for slot in slot_for(1)..=slot_for(MAX_INSTANCES) {
        if !instance_dir(slot).exists() {
            continue;
        }
        if let Ok(Some(_claim)) = SlotClaim::take(slot) {
            if remove(slot).is_ok() {
                removed.push(slot);
            }
        }
    }
    removed
}

fn clear_except_claim(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_name() == CLAIM_FILE {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn without_dropped(settings: &str) -> String {
    settings
        .lines()
        .filter(|line| {
            let key = line.split('=').next().unwrap_or("").trim();
            !DROPPED_SETTINGS.contains(&key)
        })
        .map(|line| format!("{line}\n"))
        .collect()
}

fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    if !src.is_dir() {
        return fs::copy(src, dst).map(drop);
    }
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        copy_tree(&entry.path(), &dst.join(entry.file_name()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestHome;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    fn base_slot() -> std::path::PathBuf {
        let base = instance_dir(1);
        write(&base.join("Image"), "kernel");
        write(&base.join("emmc.qcow2"), "emmc");
        write(
            &base.join("settings.txt"),
            "mac=02:00:00:00:00:01\nmodel=cdj3k\nsoc_serial=abc\nusb_virtual_path=/u.img\n",
        );
        write(&base.join("mods/list.txt"), "on mine\tmine.tgz\n");
        write(&base.join("mods/installed/mine/mod.toml"), "v1");
        base
    }

    #[test]
    fn an_ephemeral_slot_copies_its_base_without_touching_it() {
        let _home = TestHome::new("ephemeral-prepare");
        let base = base_slot();
        let slot = slot_for(1);
        let claim = SlotClaim::take(slot).unwrap().unwrap();
        prepare(1, slot).unwrap();
        let eph = instance_dir(slot);
        assert_eq!(
            read(&eph.join("settings.txt")),
            "model=cdj3k\nsoc_serial=abc\n"
        );
        assert_eq!(read(&eph.join("Image")), "kernel");
        assert_eq!(read(&eph.join("mods/installed/mine/mod.toml")), "v1");

        write(&eph.join("settings.txt"), "model=cdj3k\nmods_enabled=0\n");
        write(&eph.join("mods/installed/e2e-hello/mod.toml"), "x");
        remove(slot).unwrap();
        drop(claim);
        assert!(!eph.exists());
        assert_eq!(read(&base.join("emmc.qcow2")), "emmc");
        assert!(read(&base.join("settings.txt")).starts_with("mac=02:00:00:00:00:01\n"));
        assert!(!base.join("mods/installed/e2e-hello").exists());
    }

    #[test]
    fn leftovers_go_but_a_running_session_stays() {
        let _home = TestHome::new("ephemeral-leftovers");
        base_slot();
        let running = SlotClaim::take(slot_for(1)).unwrap().unwrap();
        prepare(1, slot_for(1)).unwrap();
        fs::create_dir_all(instance_dir(slot_for(2))).unwrap();

        assert_eq!(remove_leftovers(), [slot_for(2)]);
        assert!(instance_dir(slot_for(1)).exists());
        drop(running);
        assert_eq!(remove_leftovers(), [slot_for(1)]);
        assert!(instance_dir(1).join("emmc.qcow2").exists());
    }
}
