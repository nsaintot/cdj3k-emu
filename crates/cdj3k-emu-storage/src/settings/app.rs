//! The settings that belong to the app rather than to a slot: the updater's.

use std::io;

use super::kv::{
    app_path, flag, flag_value, locked, read_kv, text, try_read_kv, write_kv, APP_KEYS,
};

/// What the updater remembers between launches. Stored in the app-wide
/// `settings.txt`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    /// Look for a newer release when the first window opens. On by default;
    /// "Check for Update…" works either way.
    pub update_auto_check: bool,
    /// A release the user chose to skip. The automatic check offers only a
    /// newer one; a check from the menu offers it still.
    pub update_skip_version: Option<String>,
    /// The Unix time before which the automatic check stays quiet, set by
    /// "Remind Me Later". It is 0 when no reminder is set.
    pub update_remind_after: u64,
    /// Download and prepare what the automatic check finds without asking,
    /// then offer the restart. Off by default. On macOS, Sparkle's own setting
    /// applies instead.
    pub update_auto_install: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            update_auto_check: true,
            update_skip_version: None,
            update_remind_after: 0,
            update_auto_install: false,
        }
    }
}

impl AppSettings {
    pub fn load() -> Self {
        let _g = locked();
        Self::load_unlocked()
    }

    fn load_unlocked() -> Self {
        let map = read_kv(&app_path());
        let d = Self::default();
        Self {
            update_auto_check: flag(&map, "update_auto_check", d.update_auto_check),
            update_skip_version: text(&map, "update_skip_version"),
            update_remind_after: map
                .get("update_remind_after")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.update_remind_after),
            update_auto_install: flag(&map, "update_auto_install", d.update_auto_install),
        }
    }

    /// Load, mutate and save under the settings lock.
    pub fn update(mutate: impl FnOnce(&mut Self)) -> io::Result<()> {
        let _g = locked();
        let path = app_path();
        let mut map = try_read_kv(&path)?;
        let mut s = Self::load_unlocked();
        mutate(&mut s);
        map.insert("update_auto_check".into(), flag_value(s.update_auto_check));
        map.insert(
            "update_skip_version".into(),
            s.update_skip_version.unwrap_or_default(),
        );
        map.insert(
            "update_remind_after".into(),
            s.update_remind_after.to_string(),
        );
        map.insert(
            "update_auto_install".into(),
            flag_value(s.update_auto_install),
        );
        write_kv(&path, &map, APP_KEYS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key survives its own save and the startup prune.
    #[test]
    fn app_settings_round_trip_through_the_prune() {
        let _home = crate::TestHome::new("appsettings");
        assert_eq!(AppSettings::load(), AppSettings::default());
        AppSettings::update(|s| {
            s.update_auto_check = false;
            s.update_skip_version = Some("0.4.0".into());
            s.update_remind_after = 1_790_000_000;
            s.update_auto_install = true;
        })
        .unwrap();
        super::super::prune_app_file();
        let s = AppSettings::load();
        assert!(!s.update_auto_check);
        assert_eq!(s.update_skip_version.as_deref(), Some("0.4.0"));
        assert_eq!(s.update_remind_after, 1_790_000_000);
        assert!(s.update_auto_install);
    }
}
