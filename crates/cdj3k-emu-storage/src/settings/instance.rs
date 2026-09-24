//! A slot's identity, hardware selections and menu state.

use std::io;
use std::path::PathBuf;

use cdj3k_emu_panel::Model;

use super::identity::{generate_mac, generate_soc_serial, is_valid_mac, is_valid_soc_serial};
use super::kv::{instance_path, locked, read_kv, write_kv, SLOT_KEYS};

#[derive(Debug, Clone)]
pub struct InstanceSettings {
    /// Locally-administered MAC, e.g. "0a:11:22:33:44:55". Stable across launches.
    pub mac: String,
    /// SoC serial the guest publishes as the `Serial` line of `/proc/cpuinfo`,
    /// e.g. "0123456789abcd05": 16 lowercase hex digits, last byte non-zero.
    ///
    /// `genkey_pr` derives the `cabinet.img` LUKS passphrase as
    /// `sha512hex((model + serial) repeated N + 1 times)`, where `N` is
    /// `strtol(last two digits of the serial, 16)`.  `initoptenv` only opens
    /// the container with it - nothing re-keys - so the serial has to be the
    /// one the staged cabinet was keyed for, and changing it afterwards locks
    /// `cabinet.img` out.
    pub soc_serial: String,
    /// Whether QEMU is launched with the audio backend.  When true virtio-sound
    /// is added and routed to CoreAudio; when false no ALSA card is present.
    pub audio_enabled: bool,
    /// Selected CoreAudio device UID, or `None` for "system default output".
    /// Stable across reboots; the human-readable name is re-resolved at
    /// enumeration time and never persisted.
    pub audio_device_uid: Option<String>,
    /// "Enable ALC" (audio-latency compensation) toggle. Mirrors the guest's
    /// `audio_sync_enabled` sysfs param. Defaults to **true** for new
    /// instances - the sync compensation is the better experience for the
    /// vast majority of users; opting out is a power-user choice.
    /// The runtime worker pushes this value to the guest cfg daemon once
    /// per QEMU boot so the kernel module's default-off doesn't override us.
    pub alc_enabled: bool,
    /// "Trackpad Haptics" toggle.  Gates the Force Touch detent clicks emitted
    /// by `cdj3k_emu_platform::haptic::actuate` as the jog crosses detents.
    /// Defaults to **true**; users on hardware without an actuator see no
    /// change either way (the platform layer no-ops silently).
    pub haptic_enabled: bool,
    /// "PC Link (USB-B cable)" toggle.  When true, the runtime brings up
    /// the host-side virtual CoreMIDI + HID endpoints and starts the
    /// in-guest `cdj3k-pc-link-bridge.service`.  Off by default; the
    /// emulated cable starts unplugged.  Persisted across launches; the
    /// runtime worker re-applies the state when the guest comes back up.
    pub pc_link_enabled: bool,
    /// The model this slot emulates - the one its [`FirmwarePaths`] hold.
    /// `None` until something is installed. The app opens the slot on this
    /// model directly and shows the picker only for a fresh slot or in the
    /// "Manage Emulation" window.
    ///
    /// [`FirmwarePaths`]: crate::FirmwarePaths
    pub model: Option<Model>,
    /// The firmware release installed in this slot, e.g. "3.20", as the
    /// installer read it from the UPD's `IMAGES/RELEASE.TXT`. The eMMC's
    /// U-Boot env holds the same string, but behind the qcow2 mapping, so the
    /// menus read it from here. `None` for a slot installed before this key
    /// existed, or from an image that carries no release file.
    pub firmware_release: Option<String>,
    /// Last user-selected network interface name (e.g. "en0"), or `None` for
    /// "no network".  Restored on launch if the iface is still present;
    /// otherwise kept on disk so it can re-bind when the iface returns.
    pub net_iface: Option<String>,
    /// Last user-selected virtual USB image path, or `None`.  Restored on
    /// launch if the file still exists; kept on disk regardless.
    pub usb_virtual_path: Option<PathBuf>,
    /// Last user-selected physical USB disk BSD name (e.g. "disk2"), or
    /// `None`.  Restored on launch if the disk is still present; kept on disk
    /// regardless.
    pub usb_physical_bsd: Option<String>,
}

impl InstanceSettings {
    /// Load from disk, or generate + persist a fresh MAC if no file/key exists.
    pub fn load_or_init(instance_id: u32) -> Self {
        let _g = locked();
        Self::load_or_init_unlocked(instance_id)
    }

    /// Load, mutate and save under the process-wide settings lock, so two
    /// threads persisting different fields cannot lose each other's write.
    pub fn update(instance_id: u32, mutate: impl FnOnce(&mut Self)) -> io::Result<()> {
        let _g = locked();
        let mut s = Self::load_or_init_unlocked(instance_id);
        mutate(&mut s);
        s.save_unlocked(instance_id)
    }

    fn load_or_init_unlocked(instance_id: u32) -> Self {
        let path = instance_path(instance_id);
        let mut map = read_kv(&path);
        let mut minted = false;
        let mac = match map.get("mac") {
            Some(m) if is_valid_mac(m) => m.clone(),
            _ => {
                let m = generate_mac();
                map.insert("mac".into(), m.clone());
                minted = true;
                m
            }
        };
        let soc_serial = match map.get("soc_serial") {
            Some(s) if is_valid_soc_serial(s) => s.clone(),
            _ => {
                let s = generate_soc_serial();
                map.insert("soc_serial".into(), s.clone());
                minted = true;
                s
            }
        };
        if minted {
            if let Err(e) = write_kv(&path, &map, SLOT_KEYS) {
                // Disk write failed — the minted MAC and SoC serial won't
                // survive a restart, but we'd rather proceed with one-shot
                // values than refuse to launch the slot.  Log so the user sees
                // it in Console.app instead of a silent churn; a churning SoC
                // serial costs the slot its `cabinet.img` (see `soc_serial`).
                eprintln!(
                    "cdj3k-emu-storage: failed to persist identity for instance {}: {}",
                    instance_id, e
                );
            }
        }
        let audio_enabled = map
            .get("audio_enabled")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let audio_device_uid = map
            .get("audio_device_uid")
            .filter(|v| !v.is_empty())
            .cloned();
        // ALC default: ON. New users get the sync compensation by default;
        // opt-out is a power-user choice that the toggle persists.
        let alc_enabled = map
            .get("alc_enabled")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        // Trackpad haptics default: ON.  See `haptic_enabled` doc-comment.
        let haptic_enabled = map
            .get("haptic_enabled")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        // PC link default: OFF - emulated cable starts unplugged.
        let pc_link_enabled = map
            .get("pc_link_enabled")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let model = map.get("model").and_then(|v| Model::parse(v));
        let firmware_release = map
            .get("firmware_release")
            .filter(|v| !v.is_empty())
            .cloned();
        let net_iface = map.get("net_iface").filter(|v| !v.is_empty()).cloned();
        let usb_virtual_path = map
            .get("usb_virtual_path")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let usb_physical_bsd = map
            .get("usb_physical_bsd")
            .filter(|v| !v.is_empty())
            .cloned();
        Self {
            mac,
            soc_serial,
            audio_enabled,
            audio_device_uid,
            alc_enabled,
            haptic_enabled,
            pc_link_enabled,
            model,
            firmware_release,
            net_iface,
            usb_virtual_path,
            usb_physical_bsd,
        }
    }

    /// The slot's saved model, read without creating or writing anything
    /// (unlike [`Self::load_or_init`], which mints a MAC for a new slot).
    pub fn saved_model(instance_id: u32) -> Option<Model> {
        read_kv(&instance_path(instance_id))
            .get("model")
            .and_then(|v| Model::parse(v))
    }

    /// The firmware release recorded for `instance_id`, without loading the
    /// rest of the slot's settings. Mirrors [`Self::saved_model`].
    pub fn saved_firmware_release(instance_id: u32) -> Option<String> {
        read_kv(&instance_path(instance_id))
            .get("firmware_release")
            .filter(|v| !v.is_empty())
            .cloned()
    }

    /// A fresh SoC serial, not yet given to any slot. The firmware installer
    /// keys a new eMMC's cabinet for it and records it once the install is
    /// complete. A minted serial opens no real deck's cabinet: that needs the
    /// deck's own serial, which [`Self::set_soc_serial`] pins. Nothing else
    /// may change the serial - see [`Self::soc_serial`].
    pub fn mint_soc_serial() -> String {
        generate_soc_serial()
    }

    /// `serial` in the stored form, or an error if `genkey_pr` could not use
    /// it.
    pub fn parse_soc_serial(serial: &str) -> io::Result<String> {
        let serial = serial.trim().to_ascii_lowercase();
        if !is_valid_soc_serial(&serial) {
            return Err(io::Error::other(format!(
                "not a usable SoC serial: {serial:?} - want 16 hex digits with a non-zero last byte"
            )));
        }
        Ok(serial)
    }

    /// Pin `instance_id`'s SoC serial to `serial`, returning the stored form.
    ///
    /// Set this to a real deck's `/proc/cpuinfo` serial and the guest's
    /// `genkey_pr` derives that deck's `cabinet.img` passphrase, so its cabinet
    /// opens here. Rejects anything `genkey_pr` could not use.
    pub fn set_soc_serial(instance_id: u32, serial: &str) -> io::Result<String> {
        let serial = Self::parse_soc_serial(serial)?;
        Self::update(instance_id, {
            let serial = serial.clone();
            move |s| s.soc_serial = serial
        })?;
        Ok(serial)
    }

    pub fn save(&self, instance_id: u32) -> io::Result<()> {
        let _g = locked();
        self.save_unlocked(instance_id)
    }

    fn save_unlocked(&self, instance_id: u32) -> io::Result<()> {
        let path = instance_path(instance_id);
        let mut map = read_kv(&path);
        map.insert("mac".into(), self.mac.clone());
        map.insert("soc_serial".into(), self.soc_serial.clone());
        map.insert(
            "audio_enabled".into(),
            (if self.audio_enabled { "1" } else { "0" }).to_string(),
        );
        map.insert(
            "audio_device_uid".into(),
            self.audio_device_uid.clone().unwrap_or_default(),
        );
        map.insert(
            "alc_enabled".into(),
            (if self.alc_enabled { "1" } else { "0" }).to_string(),
        );
        map.insert(
            "haptic_enabled".into(),
            (if self.haptic_enabled { "1" } else { "0" }).to_string(),
        );
        map.insert(
            "pc_link_enabled".into(),
            (if self.pc_link_enabled { "1" } else { "0" }).to_string(),
        );
        map.insert(
            "model".into(),
            self.model.map(|m| m.slug().to_string()).unwrap_or_default(),
        );
        map.insert(
            "firmware_release".into(),
            self.firmware_release.clone().unwrap_or_default(),
        );
        map.insert(
            "net_iface".into(),
            self.net_iface.clone().unwrap_or_default(),
        );
        map.insert(
            "usb_virtual_path".into(),
            self.usb_virtual_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        map.insert(
            "usb_physical_bsd".into(),
            self.usb_physical_bsd.clone().unwrap_or_default(),
        );
        write_kv(&path, &map, SLOT_KEYS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `update` keeps the fields other writers own (the minted MAC) and the
    /// saved model comes back through the read-only `saved_model`.
    #[test]
    fn update_preserves_other_keys_and_persists_the_model() {
        let _home = crate::TestHome::new("settings");

        let slot = 7;
        assert_eq!(InstanceSettings::saved_model(slot), None);
        let first = InstanceSettings::load_or_init(slot);
        let (mac, serial) = (first.mac, first.soc_serial);
        InstanceSettings::update(slot, |s| s.model = Some(Model::Cdj3kx)).unwrap();
        InstanceSettings::update(slot, |s| s.audio_enabled = true).unwrap();

        let s = InstanceSettings::load_or_init(slot);
        assert_eq!(s.mac, mac, "the MAC survives updates");
        assert_eq!(s.soc_serial, serial, "the SoC serial survives updates");
        // Only a firmware install mints a new one, and it sticks.
        let fresh = InstanceSettings::mint_soc_serial();
        assert!(is_valid_soc_serial(&fresh), "{fresh}");
        InstanceSettings::update(slot, |s| s.soc_serial = fresh.clone()).unwrap();
        assert_eq!(InstanceSettings::load_or_init(slot).soc_serial, fresh);
        assert_eq!(s.model, Some(Model::Cdj3kx));
        assert!(s.audio_enabled);
        assert_eq!(InstanceSettings::saved_model(slot), Some(Model::Cdj3kx));
        // No temp file left behind by the atomic write.
        let leftovers: Vec<_> = std::fs::read_dir(crate::instance_dir(slot))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        // Pinning a real deck's serial normalises and sticks; a value
        // genkey_pr could not use is refused without disturbing it.
        let pinned = InstanceSettings::set_soc_serial(slot, " 0123456789ABCD05 ").unwrap();
        assert_eq!(pinned, "0123456789abcd05", "trimmed and lower-cased");
        assert!(InstanceSettings::set_soc_serial(slot, "0123456789abcd00").is_err());
        assert!(InstanceSettings::set_soc_serial(slot, "deadbeef").is_err());
        assert_eq!(InstanceSettings::load_or_init(slot).soc_serial, pinned);
    }
}
