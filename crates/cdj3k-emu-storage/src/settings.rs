//! Persistent settings: a shared app file plus a per-instance file.
//!
//! Layout (macOS):
//!   ~/Library/Application Support/<BUNDLE_ID>/settings.txt            - app-wide
//!   ~/Library/Application Support/<BUNDLE_ID>/instance-N/settings.txt - per instance
//!
//! Format is plain `key=value\n` lines. Unknown keys are preserved on save so
//! older builds don't drop newer fields. No serde dep; the value space is tiny
//! and the file is human-editable for debugging.

use std::collections::BTreeMap;
use std::path::PathBuf;

fn app_path() -> PathBuf {
    crate::app_data_dir().join("settings.txt")
}

fn instance_path(instance_id: u32) -> PathBuf {
    crate::app_data_dir()
        .join(format!("instance-{}", instance_id))
        .join("settings.txt")
}

fn read_kv(path: &std::path::Path) -> BTreeMap<String, String> {
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

fn write_kv(path: &std::path::Path, map: &BTreeMap<String, String>) -> std::io::Result<()> {
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
    std::fs::write(path, buf)
}

// ── App-wide settings ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AppSettings {
    pub jog_adjust: f32, // [0, 1]
    pub vinyl_speed: u8,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            jog_adjust: 0.5,
            vinyl_speed: 0,
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let map = read_kv(&app_path());
        let mut s = Self::default();
        if let Some(v) = map.get("jog_adjust").and_then(|v| v.parse().ok()) {
            s.jog_adjust = f32::clamp(v, 0.0, 1.0);
        }
        if let Some(v) = map.get("vinyl_speed").and_then(|v| v.parse().ok()) {
            s.vinyl_speed = v;
        }
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = app_path();
        let mut map = read_kv(&path); // preserve unknown keys
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
        let path = instance_path(instance_id);
        let mut map = read_kv(&path);
        let mac = match map.get("mac") {
            Some(m) if is_valid_mac(m) => m.clone(),
            _ => {
                let m = generate_mac();
                map.insert("mac".into(), m.clone());
                if let Err(e) = write_kv(&path, &map) {
                    // Disk write failed — the generated MAC won't survive a
                    // restart, but we'd rather proceed with a one-shot MAC
                    // than refuse to launch the slot.  Log so the user sees
                    // it in Console.app instead of getting a silent MAC churn.
                    eprintln!(
                        "cdj3k-emu-storage: failed to persist MAC for instance {}: {}",
                        instance_id, e
                    );
                }
                m
            }
        };
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
            audio_enabled,
            audio_device_uid,
            alc_enabled,
            haptic_enabled,
            net_iface,
            usb_virtual_path,
            usb_physical_bsd,
        }
    }

    pub fn save(&self, instance_id: u32) -> std::io::Result<()> {
        let path = instance_path(instance_id);
        let mut map = read_kv(&path);
        map.insert("mac".into(), self.mac.clone());
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
