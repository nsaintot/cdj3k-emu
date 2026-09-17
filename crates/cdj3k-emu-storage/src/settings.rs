//! Persistent settings: a shared app file plus a per-instance file.
//!
//! Layout (macOS):
//!   ~/Library/Application Support/<BUNDLE_ID>/settings.txt            - app-wide
//!   ~/Library/Application Support/<BUNDLE_ID>/instance-N/settings.txt - per instance
//!
//! Format is plain `key=value\n` lines. Unknown keys are preserved on save so
//! older builds don't drop newer fields. No serde dep; the value space is tiny
//! and the file is human-editable for debugging.
//!
//! The UI, menu and runtime-worker threads all persist: every write lands
//! atomically (temp file + rename, so a reader never sees a truncated file)
//! and every load-modify-save runs under one process-wide lock
//! ([`InstanceSettings::update`]), so two threads cannot lose each other's
//! fields.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use cdj3k_emu_panel::Model;

static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

fn locked() -> MutexGuard<'static, ()> {
    SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn app_path() -> PathBuf {
    crate::app_data_dir().join("settings.txt")
}

fn instance_path(instance_id: u32) -> PathBuf {
    crate::instance_dir(instance_id).join("settings.txt")
}

fn read_kv(path: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    if let Ok(s) = std::fs::read_to_string(path) {
        for line in s.lines() {
            if let Some((k, v)) = line.split_once('=') {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    map
}

/// Write `map` to `path` atomically: the text goes to a sibling temp file
/// that is renamed over `path`, so a concurrent reader sees the old or the
/// new file, never an empty one (an empty read would mint a new MAC).
fn write_kv(path: &Path, map: &BTreeMap<String, String>) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut buf = String::new();
    for (k, v) in map {
        buf.push_str(k);
        buf.push('=');
        buf.push_str(v);
        buf.push('\n');
    }
    let tmp = path.with_extension(format!("txt.{}.tmp", std::process::id()));
    std::fs::write(&tmp, buf)?;
    std::fs::rename(&tmp, path)
}

// ── App-wide settings ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AppSettings {
    /// Draw the LCD larger than the deck's, over the decoration around it.
    /// A viewing preference, so it is app-wide rather than per slot.
    pub screen_extended: bool,
    pub jog_adjust: f32, // [0, 1]
    pub vinyl_speed: u8,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            screen_extended: false,
            jog_adjust: 0.5,
            vinyl_speed: 0,
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let map = read_kv(&app_path());
        let mut s = Self::default();
        if let Some(v) = map.get("screen_extended") {
            s.screen_extended = v == "1" || v.eq_ignore_ascii_case("true");
        }
        if let Some(v) = map.get("jog_adjust").and_then(|v| v.parse().ok()) {
            s.jog_adjust = f32::clamp(v, 0.0, 1.0);
        }
        if let Some(v) = map.get("vinyl_speed").and_then(|v| v.parse().ok()) {
            s.vinyl_speed = v;
        }
        s
    }

    pub fn save(&self) -> io::Result<()> {
        let _g = locked();
        let path = app_path();
        let mut map = read_kv(&path); // preserve unknown keys
        map.insert(
            "screen_extended".into(),
            if self.screen_extended { "1" } else { "0" }.into(),
        );
        map.insert("jog_adjust".into(), format!("{}", self.jog_adjust));
        map.insert("vinyl_speed".into(), format!("{}", self.vinyl_speed));
        write_kv(&path, &map)
    }
}

// ── Per-instance settings ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct InstanceSettings {
    /// Locally-administered MAC, e.g. "0a:11:22:33:44:55". Stable across launches.
    pub mac: String,
    /// SoC serial the guest publishes as the `Serial` line of `/proc/cpuinfo`,
    /// e.g. "0123456789abcdb6": 16 lowercase hex digits, last byte non-zero.
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
    /// model directly and shows the picker only for a fresh slot or when
    /// "Switch Emulation" asks for one.
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
            if let Err(e) = write_kv(&path, &map) {
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

    /// Mint a fresh SoC serial for `instance_id`, persist it and return it.
    ///
    /// Called by the firmware installer when it recreates the eMMC.  A minted
    /// serial opens no cabinet: staging a real deck's `cabinet.img` also needs
    /// that deck's serial, which [`Self::set_soc_serial`] pins.  Nothing else
    /// may change the serial - see [`Self::soc_serial`].
    pub fn regenerate_soc_serial(instance_id: u32) -> io::Result<String> {
        let serial = generate_soc_serial();
        Self::update(instance_id, |s| s.soc_serial = serial.clone())?;
        Ok(serial)
    }

    /// Pin `instance_id`'s SoC serial to `serial`, returning the stored form.
    ///
    /// Set this to a real deck's `/proc/cpuinfo` serial and the guest's
    /// `genkey_pr` derives that deck's `cabinet.img` passphrase, so its cabinet
    /// opens here. Rejects anything `genkey_pr` could not use.
    pub fn set_soc_serial(instance_id: u32, serial: &str) -> io::Result<String> {
        let serial = serial.trim().to_ascii_lowercase();
        if !is_valid_soc_serial(&serial) {
            return Err(io::Error::other(format!(
                "not a usable SoC serial: {serial:?} - want 16 hex digits with a non-zero last byte"
            )));
        }
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
        write_kv(&path, &map)
    }
}

/// 16 lowercase hex digits with a non-zero last byte, matching what an RK3399
/// prints for `Serial`.  `genkey_pr` repeats its hash input
/// `strtol(&serial[14..], 16)` times, so a zero there would hash nothing.
fn is_valid_soc_serial(s: &str) -> bool {
    s.len() == 16
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && matches!(u8::from_str_radix(&s[14..], 16), Ok(n) if n != 0)
}

/// Mint a SoC serial from a v4 UUID's first 8 bytes, forcing the last byte
/// non-zero so `genkey_pr`'s repeat count is at least 1.
fn generate_soc_serial() -> String {
    let b = uuid::Uuid::new_v4().into_bytes();
    format!(
        "{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0],
        b[1],
        b[2],
        b[3],
        b[4],
        b[5],
        b[6],
        b[7].max(1)
    )
}

fn is_valid_mac(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && u8::from_str_radix(p, 16).is_ok())
}

/// Generate a random locally-administered unicast MAC.
/// Uses `uuid::Uuid::new_v4()` (already a dep) as the entropy source - its bytes
/// are cryptographically random on macOS/Linux. The first byte is forced to
/// `02` (LAA bit set, multicast bit clear) so it's a valid host address.
fn generate_mac() -> String {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    let m0 = (bytes[0] & 0xfe) | 0x02; // LAA, unicast
    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        m0, bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `update` keeps the fields other writers own (the minted MAC) and the
    /// saved model comes back through the read-only `saved_model`.
    #[test]
    fn update_preserves_other_keys_and_persists_the_model() {
        let _env = crate::test_env_lock();
        let home = std::env::temp_dir().join(format!("cdj3k-settings-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::env::set_var("HOME", &home);

        let slot = 7;
        assert_eq!(InstanceSettings::saved_model(slot), None);
        let first = InstanceSettings::load_or_init(slot);
        let (mac, serial) = (first.mac, first.soc_serial);
        InstanceSettings::update(slot, |s| s.model = Some(Model::Cdj3kx)).unwrap();
        InstanceSettings::update(slot, |s| s.audio_enabled = true).unwrap();

        let s = InstanceSettings::load_or_init(slot);
        assert_eq!(s.mac, mac, "the MAC survives updates");
        assert_eq!(s.soc_serial, serial, "the SoC serial survives updates");
        assert!(is_valid_soc_serial(&s.soc_serial), "{}", s.soc_serial);
        // Only a firmware install mints a new one, and it sticks.
        let fresh = InstanceSettings::regenerate_soc_serial(slot).unwrap();
        assert!(is_valid_soc_serial(&fresh), "{fresh}");
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
        let pinned = InstanceSettings::set_soc_serial(slot, " 0123456789ABCDB6 ").unwrap();
        assert_eq!(pinned, "0123456789abcdb6", "trimmed and lower-cased");
        // Last byte zero: genkey_pr would repeat its input zero times.
        assert!(InstanceSettings::set_soc_serial(slot, "0123456789abcd00").is_err());
        assert!(InstanceSettings::set_soc_serial(slot, "deadbeef").is_err());
        assert!(InstanceSettings::set_soc_serial(slot, "zzzzzzzzzzzzzzzz").is_err());
        assert_eq!(InstanceSettings::load_or_init(slot).soc_serial, pinned);

        let _ = std::fs::remove_dir_all(&home);
    }
}
