//! The `key=value` file layer: where the files live, which keys each may
//! hold, and the atomic locked read/write both settings structs go through.

use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn locked() -> MutexGuard<'static, ()> {
    SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn instance_path(instance_id: u32) -> PathBuf {
    crate::instance_dir(instance_id).join("settings.txt")
}

/// The app-wide `<BUNDLE_ID>/settings.txt`, which [`AppSettings`] owns.
///
/// [`AppSettings`]: super::AppSettings
pub(super) fn app_path() -> PathBuf {
    crate::app_data_dir().join("settings.txt")
}

/// Every key a slot's `settings.txt` may hold: the union of what
/// [`super::InstanceSettings`] and [`super::PanelSettings`] write, since both
/// share the file.
///
/// [`write_kv`] drops whatever is missing here, so a field added to either
/// struct has to be listed or it will not survive its own save. The
/// `the_slot_registry_covers_every_key_written` test enforces that.
pub(super) const SLOT_KEYS: &[&str] = &[
    "alc_enabled",
    "audio_device_uid",
    "audio_enabled",
    "dev_mode",
    "firmware_release",
    "fs",
    "haptic_enabled",
    "jog_adjust",
    "mac",
    "model",
    "mods_enabled",
    "net_iface",
    "pc_link_enabled",
    "screen_extended",
    "soc_serial",
    "usb_physical_bsd",
    "usb_virtual_path",
    "vinyl_speed",
];

/// Every key the app-wide `settings.txt` may hold: what [`AppSettings`]
/// writes. Everything else is per slot and lives in [`SLOT_KEYS`].
///
/// [`AppSettings`]: super::AppSettings
pub(super) const APP_KEYS: &[&str] = &[
    "update_auto_check",
    "update_auto_install",
    "update_remind_after",
    "update_skip_version",
];

/// Empty the app-wide `settings.txt` of every key [`APP_KEYS`] does not list.
///
/// The app calls this at startup, for the keys older releases kept there. A
/// file that is already clean is left alone.
pub fn prune_app_file() {
    let _g = locked();
    let path = app_path();
    let map = match try_read_kv(&path) {
        Ok(map) => map,
        Err(e) => {
            eprintln!(
                "cdj3k-emu-storage: could not read {}, so it is not pruned: {e}",
                path.display()
            );
            return;
        }
    };
    if map.keys().all(|k| APP_KEYS.contains(&k.as_str())) {
        return; // already clean, or no file at all
    }
    if let Err(e) = write_kv(&path, &map, APP_KEYS) {
        eprintln!(
            "cdj3k-emu-storage: failed to prune {}: {}",
            path.display(),
            e
        );
    }
}

/// The `key=value` pairs in `path`, or an empty map when the file cannot be
/// read. Only read-only callers use it; a writer goes through [`try_read_kv`].
pub(crate) fn read_kv(path: &Path) -> BTreeMap<String, String> {
    try_read_kv(path).unwrap_or_default()
}

/// The `key=value` pairs in `path`. A missing file is an empty map; any other
/// failure, such as a permission error or a byte that is not UTF-8, is an
/// error, so that a writer does not replace a file it could not read.
pub(super) fn try_read_kv(path: &Path) -> io::Result<BTreeMap<String, String>> {
    let s = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e),
    };
    Ok(s.lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect())
}

/// `key` as a flag: `1` or `true` (any case) is on, anything else off, and
/// `default` when the key is absent.
pub(super) fn flag(map: &BTreeMap<String, String>, key: &str, default: bool) -> bool {
    map.get(key)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(default)
}

/// How [`flag`] values are stored.
pub(super) fn flag_value(on: bool) -> String {
    if on { "1" } else { "0" }.to_string()
}

/// `key` as a number, 0 when it is absent or not a `u32`.
pub(super) fn number(map: &BTreeMap<String, String>, key: &str) -> u32 {
    map.get(key).and_then(|v| v.parse().ok()).unwrap_or(0)
}

/// `key`'s value, `None` when absent or empty.
pub(super) fn text(map: &BTreeMap<String, String>, key: &str) -> Option<String> {
    map.get(key).filter(|v| !v.is_empty()).cloned()
}

/// Write the keys of `map` that `known` lists to `path`, atomically: the text
/// goes to a sibling temp file, which is synced to disk and then renamed over
/// `path`. A concurrent reader, or a host that loses power, sees the old or
/// the new file, never an empty one (an empty read would mint a new MAC).
///
/// Anything outside `known` is dropped - a key a release retired, or one a
/// hand-edit left behind. Pass the registry of the file being written
/// ([`SLOT_KEYS`] or [`APP_KEYS`]), never one struct's own keys.
pub(super) fn write_kv(
    path: &Path,
    map: &BTreeMap<String, String>,
    known: &[&str],
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let buf = kv_text(map.iter().filter(|(k, _)| known.contains(&k.as_str())));
    let tmp = path.with_extension(format!("txt.{}.tmp", std::process::id()));
    let written = write_synced(&tmp, buf.as_bytes()).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// `pairs` as `key=value` lines, the format of every settings file.
pub(crate) fn kv_text<'a>(pairs: impl IntoIterator<Item = (&'a String, &'a String)>) -> String {
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}\n"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{InstanceSettings, PanelSettings};
    use super::*;
    use cdj3k_emu_panel::Model;

    /// `SLOT_KEYS` has to cover every key the two structs write, or the save
    /// that writes a new field would prune it straight back out.
    #[test]
    fn the_slot_registry_covers_every_key_written() {
        let _home = crate::TestHome::new("registry");

        let slot = 3;
        // Every field non-empty: an empty value still writes its key, but a
        // populated one also proves the value survives the filter.
        InstanceSettings {
            mac: "02:11:22:33:44:55".into(),
            soc_serial: Some("0123456789abcd05".into()),
            audio_enabled: true,
            audio_device_uid: Some("uid".into()),
            alc_enabled: true,
            haptic_enabled: true,
            pc_link_enabled: true,
            model: Some(Model::Cdj3kx),
            firmware_release: Some("3.20".into()),
            fs: 1,
            mods_enabled: false,
            dev_mode: true,
            net_iface: Some("en0".into()),
            usb_virtual_path: Some(PathBuf::from("/tmp/usb.img")),
            usb_physical_bsd: Some("disk2".into()),
        }
        .save(slot)
        .unwrap();
        PanelSettings {
            screen_extended: true,
            jog_adjust: 0.75,
            vinyl_speed: 9,
        }
        .save(slot)
        .unwrap();

        let written = read_kv(&instance_path(slot));
        let mut missing: Vec<&String> = written
            .keys()
            .filter(|k| !SLOT_KEYS.contains(&k.as_str()))
            .collect();
        missing.sort();
        assert!(missing.is_empty(), "not in SLOT_KEYS: {missing:?}");
        assert_eq!(
            written.len(),
            SLOT_KEYS.len(),
            "a registry key nothing writes"
        );

        // Nothing was lost to the other struct's save.
        let s = InstanceSettings::load_or_init(slot);
        assert_eq!(s.mac, "02:11:22:33:44:55");
        assert_eq!(s.soc_serial.as_deref(), Some("0123456789abcd05"));
        assert_eq!(s.firmware_release.as_deref(), Some("3.20"));
        assert_eq!(PanelSettings::load(slot).jog_adjust, 0.75);
    }

    /// A key no registry lists goes at the next write, and the app-wide file
    /// the panel knobs came from is emptied on its own.
    #[test]
    fn retired_keys_are_pruned() {
        let _home = crate::TestHome::new("prune");

        // A 0.1.x app-wide file.
        let app = app_path();
        std::fs::create_dir_all(app.parent().unwrap()).unwrap();
        std::fs::write(
            &app,
            "jog_adjust=0.25\nscreen_extended=1\nvinyl_speed=200\n",
        )
        .unwrap();
        prune_app_file();
        assert_eq!(std::fs::read_to_string(&app).unwrap(), "");
        // Idempotent: a clean file is not rewritten.
        let before = std::fs::metadata(&app).unwrap().modified().unwrap();
        prune_app_file();
        assert_eq!(std::fs::metadata(&app).unwrap().modified().unwrap(), before);

        // A retired key in a slot file survives until something saves.
        let slot = 4;
        let mac = InstanceSettings::load_or_init(slot).mac;
        let mut map = read_kv(&instance_path(slot));
        map.insert("some_retired_key".into(), "1".into());
        std::fs::write(
            instance_path(slot),
            map.iter()
                .map(|(k, v)| format!("{k}={v}\n"))
                .collect::<String>(),
        )
        .unwrap();
        assert!(read_kv(&instance_path(slot)).contains_key("some_retired_key"));
        InstanceSettings::update(slot, |s| s.audio_enabled = true).unwrap();
        let after = read_kv(&instance_path(slot));
        assert!(!after.contains_key("some_retired_key"), "{after:?}");
        assert_eq!(after.get("mac"), Some(&mac), "the live keys stay");
    }

    #[test]
    fn flags_and_text_read_as_written() {
        let map: BTreeMap<String, String> =
            [("on", "1"), ("yes", "TRUE"), ("no", "yes"), ("empty", "")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
        assert!(flag(&map, "on", false));
        assert!(flag(&map, "yes", false));
        assert!(!flag(&map, "no", true));
        assert!(flag(&map, "absent", true));
        assert_eq!(flag_value(true), "1");
        assert_eq!(flag_value(false), "0");
        assert_eq!(text(&map, "no").as_deref(), Some("yes"));
        assert_eq!(text(&map, "empty"), None);
        assert_eq!(text(&map, "absent"), None);
    }
}
