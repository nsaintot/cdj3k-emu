//! A slot's identity, hardware selections and menu state.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;

use cdj3k_emu_panel::Model;

use super::identity::{generate_mac, generate_soc_serial, is_valid_soc_serial};
use super::kv::{
    flag, flag_value, instance_path, locked, number, read_kv, text, write_kv, SLOT_KEYS,
};
use cdj3k_emu_platform::net::is_valid_mac;

#[derive(Debug, Clone)]
pub struct InstanceSettings {
    /// The slot's locally administered MAC, such as "0a:11:22:33:44:55". It
    /// stays the same across launches.
    pub mac: String,
    /// The SoC serial that the guest publishes as the `Serial` line of
    /// `/proc/cpuinfo`, such as "0123456789abcd05". It has 16 lowercase hex
    /// digits and a non-zero last byte.
    ///
    /// `genkey_pr` derives the `cabinet.img` LUKS passphrase as
    /// `sha512hex((model + serial) repeated N + 1 times)`, where `N` is
    /// `strtol(last two digits of the serial, 16)`. `initoptenv` only opens
    /// the container with it and nothing re-keys the container, so the serial
    /// has to be the one the staged cabinet was keyed for. Changing it
    /// afterwards locks `cabinet.img` out.
    pub soc_serial: String,
    /// Whether QEMU is launched with the audio backend. When true, virtio-sound
    /// is added and routed to the host's audio output; when false, the guest
    /// has no ALSA card.
    pub audio_enabled: bool,
    /// The selected output device's uid, as the host's audio backend names it,
    /// or `None` for "Default System Output". The uid is stable across reboots;
    /// the device name is resolved again at enumeration time and never saved.
    pub audio_device_uid: Option<String>,
    /// The "Enable ALC (Experimental)" toggle for audio-latency compensation,
    /// which mirrors the guest's `audio_sync_enabled` sysfs parameter. It
    /// defaults to true. The runtime worker pushes the value to the guest cfg
    /// daemon on every QEMU boot, because the kernel module defaults to off.
    pub alc_enabled: bool,
    /// The "Jog Haptics" toggle, which gates the Force Touch detent clicks that
    /// `cdj3k_emu_platform::haptic::actuate` emits as the jog crosses detents.
    /// It defaults to true and has no effect on hardware without an actuator.
    pub haptic_enabled: bool,
    /// The "PC Link (USB-B)" toggle. When true, the runtime brings up the
    /// host's virtual MIDI and HID endpoints and starts the in-guest
    /// `cdj3k-pc-link-bridge.service`. It is off by default, so the emulated
    /// cable starts unplugged. The runtime worker applies the state again
    /// when the guest comes back up.
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
    /// existed, or from an image without a release file.
    pub firmware_release: Option<String>,
    /// The guest feature set the slot was installed with
    /// ([`crate::FEATURE_SET`]); 0 for a slot installed before this key
    /// existed.
    pub fs: u32,
    /// "Enable Mods": whether the slot's enabled mods go into its boots
    /// ([`crate::mods`]). On by default; the list itself stays editable
    /// either way.
    pub mods_enabled: bool,
    /// The network interface the user last selected (for example "en0"), the
    /// host-only token, or `None` for "Default (NAT)". It is restored on
    /// launch if the interface is still present; otherwise it stays on disk so
    /// it can rebind when the interface returns.
    pub net_iface: Option<String>,
    /// The virtual USB image path the user last selected, or `None`. It is
    /// restored on launch if the file still exists, and stays on disk either
    /// way.
    pub usb_virtual_path: Option<PathBuf>,
    /// The BSD name of the physical USB disk the user last selected (for
    /// example "disk2"), or `None`. It is restored on launch if the disk is
    /// still present, and stays on disk either way.
    pub usb_physical_bsd: Option<String>,
}

impl InstanceSettings {
    /// Load the slot's settings, generating and saving a MAC and a SoC serial
    /// when either is missing or invalid.
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
        let (mac, minted_mac) = kept_or_minted(&mut map, "mac", is_valid_mac, generate_mac);
        let (soc_serial, minted_serial) = kept_or_minted(
            &mut map,
            "soc_serial",
            is_valid_soc_serial,
            generate_soc_serial,
        );
        if minted_mac || minted_serial {
            if let Err(e) = write_kv(&path, &map, SLOT_KEYS) {
                // The minted MAC and SoC serial will not survive a restart,
                // but the slot still launches with them. The failure is
                // logged, because a SoC serial that changes on every launch
                // costs the slot its `cabinet.img` (see `soc_serial`).
                eprintln!(
                    "cdj3k-emu-storage: failed to persist identity for instance {}: {}",
                    instance_id, e
                );
            }
        }
        Self {
            mac,
            soc_serial,
            audio_enabled: flag(&map, "audio_enabled", false),
            audio_device_uid: text(&map, "audio_device_uid"),
            alc_enabled: flag(&map, "alc_enabled", true),
            haptic_enabled: flag(&map, "haptic_enabled", true),
            pc_link_enabled: flag(&map, "pc_link_enabled", false),
            model: map.get("model").and_then(|v| Model::parse(v)),
            firmware_release: text(&map, "firmware_release"),
            fs: number(&map, "fs"),
            mods_enabled: flag(&map, "mods_enabled", true),
            net_iface: text(&map, "net_iface"),
            usb_virtual_path: text(&map, "usb_virtual_path").map(PathBuf::from),
            usb_physical_bsd: text(&map, "usb_physical_bsd"),
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
        text(&read_kv(&instance_path(instance_id)), "firmware_release")
    }

    /// The slot's [`Self::fs`], without loading the rest of its settings.
    pub fn saved_fs(instance_id: u32) -> u32 {
        number(&read_kv(&instance_path(instance_id)), "fs")
    }

    /// The slot's "Enable Mods", without loading the rest of its settings.
    pub fn saved_mods_enabled(instance_id: u32) -> bool {
        flag(&read_kv(&instance_path(instance_id)), "mods_enabled", true)
    }

    /// A fresh SoC serial, not yet given to any slot. The firmware installer
    /// keys a new eMMC's cabinet for it and records it once the install is
    /// complete. A minted serial opens no real deck's cabinet: that needs the
    /// deck's own serial, which [`Self::set_soc_serial`] pins. Nothing else
    /// may change the serial; see [`Self::soc_serial`].
    pub fn mint_soc_serial() -> String {
        generate_soc_serial()
    }

    /// Return `serial` trimmed and lower-cased, or an error if it is not 16 hex
    /// digits with a non-zero last byte.
    pub fn parse_soc_serial(serial: &str) -> io::Result<String> {
        let serial = serial.trim().to_ascii_lowercase();
        if !is_valid_soc_serial(&serial) {
            return Err(io::Error::other(format!(
                "{serial:?} is not a usable SoC serial; it needs 16 hex digits with a non-zero \
                 last byte"
            )));
        }
        Ok(serial)
    }

    /// Pin `instance_id`'s SoC serial to `serial`, returning the stored form.
    ///
    /// Set this to a real deck's `/proc/cpuinfo` serial and the guest's
    /// `genkey_pr` derives that deck's `cabinet.img` passphrase, so its cabinet
    /// opens here. Rejects a serial that [`Self::parse_soc_serial`] refuses.
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
        map.insert("audio_enabled".into(), flag_value(self.audio_enabled));
        map.insert(
            "audio_device_uid".into(),
            self.audio_device_uid.clone().unwrap_or_default(),
        );
        map.insert("alc_enabled".into(), flag_value(self.alc_enabled));
        map.insert("haptic_enabled".into(), flag_value(self.haptic_enabled));
        map.insert("pc_link_enabled".into(), flag_value(self.pc_link_enabled));
        map.insert(
            "model".into(),
            self.model.map(|m| m.slug().to_string()).unwrap_or_default(),
        );
        map.insert(
            "firmware_release".into(),
            self.firmware_release.clone().unwrap_or_default(),
        );
        map.insert("fs".into(), self.fs.to_string());
        map.insert("mods_enabled".into(), flag_value(self.mods_enabled));
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

/// Return `map[key]` when `valid` accepts it, or else a fresh value from
/// `generate` that is stored back into `map`, together with whether the value
/// was minted.
fn kept_or_minted(
    map: &mut BTreeMap<String, String>,
    key: &str,
    valid: fn(&str) -> bool,
    generate: fn() -> String,
) -> (String, bool) {
    match map.get(key) {
        Some(v) if valid(v) => (v.clone(), false),
        _ => {
            let v = generate();
            map.insert(key.into(), v.clone());
            (v, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh slot mints and persists its identity; the toggles take their
    /// defaults.
    #[test]
    fn a_fresh_slot_takes_the_defaults() {
        let _home = crate::TestHome::new("fresh");

        let s = InstanceSettings::load_or_init(2);
        assert!(is_valid_mac(&s.mac), "{}", s.mac);
        assert!(is_valid_soc_serial(&s.soc_serial), "{}", s.soc_serial);
        assert!(!s.audio_enabled);
        assert!(s.alc_enabled);
        assert!(s.haptic_enabled);
        assert!(!s.pc_link_enabled);
        assert_eq!(s.model, None);
        assert_eq!(s.audio_device_uid, None);
        assert_eq!(s.usb_virtual_path, None);

        let again = InstanceSettings::load_or_init(2);
        assert_eq!((again.mac, again.soc_serial), (s.mac, s.soc_serial));
    }

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

        // Pinning a real deck's serial normalises and stores it. An invalid
        // value is refused and leaves the stored serial unchanged.
        let pinned = InstanceSettings::set_soc_serial(slot, " 0123456789ABCD05 ").unwrap();
        assert_eq!(pinned, "0123456789abcd05", "trimmed and lower-cased");
        assert!(InstanceSettings::set_soc_serial(slot, "0123456789abcd00").is_err());
        assert!(InstanceSettings::set_soc_serial(slot, "deadbeef").is_err());
        assert_eq!(InstanceSettings::load_or_init(slot).soc_serial, pinned);
    }
}
