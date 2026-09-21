//! The deck's own panel state, one set per slot.

use std::io;

use super::kv::{instance_path, locked, read_kv, write_kv, SLOT_KEYS};

/// The two panel rotaries and the screen-extend view flag, one set per slot.
///
/// Stored in the slot's own `settings.txt`, but as a struct of its own rather
/// than fields on [`InstanceSettings`]: the UI owns these three and persists
/// them on its own schedule, and reading them must not mint a slot identity
/// the way [`InstanceSettings::load_or_init`] does.
#[derive(Debug, Clone, Copy)]
pub struct PanelSettings {
    /// Draw the LCD larger than the deck's, over the decoration around it.
    pub screen_extended: bool,
    /// JOG ADJUST rotary, `0.0` (LIGHT) to `1.0` (HEAVY). Host-side only: it
    /// picks the jog brake and the drawn gear angle and reaches no MISO frame,
    /// matching the deck, where JOG ADJUST is mechanical friction.
    pub jog_adjust: f32,
    /// VINYL SPEED ADJUST rotary. Rides byte `miso_frame::fields::VINYL` of
    /// every frame; the firmware reads it for the vinyl-mode start/stop time.
    pub vinyl_speed: u8,
}

impl Default for PanelSettings {
    fn default() -> Self {
        Self {
            screen_extended: false,
            jog_adjust: 0.5,
            vinyl_speed: 0,
        }
    }
}

impl PanelSettings {
    /// Read `instance_id`'s knobs without creating or writing anything.
    ///
    /// A key the slot does not carry takes its default, and the next save
    /// writes it. The app-wide `settings.txt` is not consulted.
    pub fn load(instance_id: u32) -> Self {
        let _g = locked();
        let map = read_kv(&instance_path(instance_id));
        let d = Self::default();
        Self {
            screen_extended: map
                .get("screen_extended")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(d.screen_extended),
            // A non-finite value would index the brake table as NaN and wedge
            // the jog, so it falls back to the default instead of clamping.
            jog_adjust: map
                .get("jog_adjust")
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .map(|v| v.clamp(0.0, 1.0))
                .unwrap_or(d.jog_adjust),
            vinyl_speed: map
                .get("vinyl_speed")
                .and_then(|v| v.parse().ok())
                .unwrap_or(d.vinyl_speed),
        }
    }

    /// Write the three keys into the slot's file, leaving the rest alone.
    pub fn save(&self, instance_id: u32) -> io::Result<()> {
        let _g = locked();
        let path = instance_path(instance_id);
        let mut map = read_kv(&path);
        map.insert(
            "screen_extended".into(),
            if self.screen_extended { "1" } else { "0" }.into(),
        );
        map.insert("jog_adjust".into(), format!("{}", self.jog_adjust));
        map.insert("vinyl_speed".into(), format!("{}", self.vinyl_speed));
        write_kv(&path, &map, SLOT_KEYS)
    }
}

#[cfg(test)]
mod tests {
    use super::super::InstanceSettings;
    use super::*;

    /// Each slot keeps its own knobs, a slot that carries none takes the
    /// defaults, and neither read mints an identity.
    #[test]
    fn panel_settings_are_per_slot() {
        let _home = crate::TestHome::new("panel");

        let fresh = PanelSettings::load(1);
        let d = PanelSettings::default();
        assert_eq!(fresh.jog_adjust, d.jog_adjust);
        assert_eq!(fresh.vinyl_speed, d.vinyl_speed);
        assert!(!fresh.screen_extended);
        assert!(
            !instance_path(1).exists(),
            "reading must not create the slot"
        );

        // Slot 1 takes its own positions; slot 2 keeps the defaults.
        PanelSettings {
            screen_extended: true,
            jog_adjust: 1.0,
            vinyl_speed: 7,
        }
        .save(1)
        .unwrap();
        let one = PanelSettings::load(1);
        assert_eq!((one.jog_adjust, one.vinyl_speed), (1.0, 7));
        assert!(one.screen_extended);
        assert_eq!(PanelSettings::load(2).vinyl_speed, d.vinyl_speed);

        // The knobs share the file with the slot identity without clobbering it.
        let mac = InstanceSettings::load_or_init(1).mac;
        PanelSettings {
            screen_extended: false,
            jog_adjust: 0.0,
            vinyl_speed: 1,
        }
        .save(1)
        .unwrap();
        assert_eq!(InstanceSettings::load_or_init(1).mac, mac);
        assert_eq!(PanelSettings::load(1).jog_adjust, 0.0);

        // Garbage falls back to the default rather than wedging the brake.
        let mut map = read_kv(&instance_path(1));
        map.insert("jog_adjust".into(), "NaN".into());
        write_kv(&instance_path(1), &map, SLOT_KEYS).unwrap();
        assert_eq!(PanelSettings::load(1).jog_adjust, d.jog_adjust);
    }
}
