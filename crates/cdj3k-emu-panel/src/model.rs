//! The players the emulator supports; see [`Model`] for the current set.
//!
//! All per-model data lives in [`ModelSpec`], which [`Model::spec`] returns.
//! [`Model`] only identifies a player; `Model::spec` is the only place that
//! matches on it.

use std::fmt;

use crate::button::Btn;
use crate::frame::FrameBit;
use crate::miso_frame::{self, PanelState, MISO_SIZE};
use crate::mosi_frame::{MosiFrame, MOSI_SIZE};
use crate::spec::ModelSpec;
use crate::{cdj1500x, cdj3k, cdj3kx};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Model {
    /// CDJ-3000.
    #[default]
    Cdj3k,
    /// CDJ-3000X.
    Cdj3kx,
    /// CDJ-1500X.
    Cdj1500x,
}

impl Model {
    pub const ALL: [Model; 3] = [Model::Cdj3k, Model::Cdj3kx, Model::Cdj1500x];

    /// The spec of this model.
    pub const fn spec(self) -> &'static ModelSpec {
        match self {
            Model::Cdj3k => &cdj3k::SPEC,
            Model::Cdj3kx => &cdj3kx::SPEC,
            Model::Cdj1500x => &cdj1500x::SPEC,
        }
    }

    /// Product name as printed on the chassis.
    pub fn title(self) -> &'static str {
        self.spec().title
    }

    /// Stable identifier for directory names and settings.
    pub fn slug(self) -> &'static str {
        self.spec().slug
    }

    /// What Pioneer calls this deck's `.UPD`. See
    /// [`ModelSpec::firmware_file_names`](crate::ModelSpec::firmware_file_names).
    pub fn firmware_file_names(self) -> &'static [&'static str] {
        self.spec().firmware_file_names
    }

    /// Returns the [`Model`] for an update file given its original name as published by Pioneer,
    /// such as `CDJ3Kv322.UPD` for a CDJ-3000. Returns `None` for an unknown or renamed file.
    pub fn from_firmware_file_name(name: &str) -> Option<Model> {
        // Split on either separator, because the name can be a path from any host.
        let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
        let stem = stem.rsplit_once('.').map_or(stem, |(s, _)| s);
        // Trim the version Pioneer appends: `v322`, `_119`.
        let stem = stem.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
        let stem = stem.strip_suffix(['v', 'V', '_']).unwrap_or(stem);
        Model::ALL.into_iter().find(|m| {
            m.firmware_file_names()
                .iter()
                .any(|n| n.eq_ignore_ascii_case(stem))
        })
    }

    /// Main LCD framebuffer `(width, height)` in pixels.
    pub fn main_lcd(self) -> (u32, u32) {
        self.spec().main_lcd
    }

    /// The frame's touch words for a contact at a point given as fractions
    /// of the display (0..1 from its top-left corner). See
    /// [`ModelSpec::touch_units`](crate::ModelSpec::touch_units).
    pub fn touch_units(self, fx: f32, fy: f32) -> (u16, u16) {
        self.spec().touch_units(fx, fy)
    }

    /// Where `btn` sits in this player's frame, or `None` if it has no such
    /// button.
    pub fn button(self, btn: Btn) -> Option<FrameBit> {
        self.spec().miso.button(btn)
    }

    /// [`Model::button`] from a written name: a [`Btn::name`]
    /// (case-insensitive) or a raw frame bit (see [`miso_frame::raw_bit`]). A
    /// raw bit is an address in the player's own frame and is taken as
    /// written.
    pub fn button_by_name(self, name: &str) -> Option<FrameBit> {
        miso_frame::raw_bit(name).or_else(|| self.button(Btn::from_name(name)?))
    }

    /// This player's MISO frame for `state`.
    pub fn encode(self, state: &PanelState) -> [u8; MISO_SIZE] {
        self.spec().miso.encode(state)
    }

    /// A MOSI frame read through this player's codec.
    pub fn mosi_frame(self, bytes: [u8; MOSI_SIZE]) -> MosiFrame {
        let spec = self.spec();
        MosiFrame::new(bytes, spec.mosi, &spec.leds)
    }

    /// Parse a user spelling, case-insensitive: the slug (`cdj3k`, `cdj3kx`),
    /// the product name (`3000`, `cdj3000`, `cdj-3000`, `3000x`, ...) or the
    /// short form (`3k`, `3kx`).
    pub fn parse(s: &str) -> Option<Model> {
        let s = s.trim().to_ascii_lowercase();
        // Match the slug first, because it starts with `cdj` and the prefix
        // strip below removes that.
        if let Some(m) = Model::ALL.into_iter().find(|m| m.spec().slug == s) {
            return Some(m);
        }
        let s = s.strip_prefix("cdj").unwrap_or(&s);
        let s = s.strip_prefix('-').unwrap_or(s);
        Model::ALL
            .into_iter()
            .find(|m| m.spec().aliases.contains(&s))
    }
}

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_spellings() {
        for s in ["cdj3k", "3000", "cdj3000", "CDJ-3000", " Cdj3000 ", "3k"] {
            assert_eq!(Model::parse(s), Some(Model::Cdj3k), "{s}");
        }
        for s in ["cdj3kx", "3000x", "cdj3000x", "CDJ-3000X", "3000X", "3kx"] {
            assert_eq!(Model::parse(s), Some(Model::Cdj3kx), "{s}");
        }
        for s in ["cdj1500x", "1500x", "cdj1500x", "CDJ-1500X", "1k5x"] {
            assert_eq!(Model::parse(s), Some(Model::Cdj1500x), "{s}");
        }
        assert_eq!(Model::parse("2000"), None);
        assert_eq!(Model::parse(""), None);
        // The Pioneer application number is not a spelling we take.
        for s in [
            "122", "145", "166", "ep122", "ep145", "ep166", "EP122", "EP-145",
        ] {
            assert_eq!(Model::parse(s), None, "{s}");
        }
    }

    #[test]
    fn slug_round_trips() {
        for m in Model::ALL {
            assert_eq!(Model::parse(m.slug()), Some(m));
        }
    }

    /// Each spelling reaches one model.
    #[test]
    fn spellings_identify_one_model_each() {
        for m in Model::ALL {
            for other in Model::ALL {
                if m == other {
                    continue;
                }
                assert_ne!(m.slug(), other.slug());
                for alias in m.spec().aliases {
                    assert!(
                        !other.spec().aliases.contains(alias),
                        "{alias:?} names both {m} and {other}"
                    );
                    assert_ne!(alias, &other.slug(), "{alias:?} is {other}'s slug");
                }
            }
        }
    }

    #[test]
    fn touch_units_per_model() {
        use crate::spec::TOUCH_DOWN;

        // CDJ-3000: sub-CPU frame units, origin at the right edge.
        assert_eq!(Model::Cdj3k.touch_units(0.0, 0.0), (1000, 0));
        assert_eq!(Model::Cdj3k.touch_units(1.0, 1.0), (0, 1000));
        assert_eq!(Model::Cdj3k.touch_units(0.5, 0.5), (500, 500));

        // CDJ-3000X: screen pixels on its 1280x800 scanout, with the contact
        // flagged, so its top-left pixel is a touch.
        assert_eq!(Model::Cdj3kx.touch_units(0.0, 0.0), (TOUCH_DOWN, 0));
        assert_eq!(
            Model::Cdj3kx.touch_units(1.0, 1.0),
            (1279 | TOUCH_DOWN, 799)
        );

        // A point off the display clamps to its edge.
        assert_eq!(Model::Cdj3kx.touch_units(-0.5, 2.0), (TOUCH_DOWN, 799));
        assert_eq!(Model::Cdj3k.touch_units(2.0, -1.0), (0, 0));
    }
}

#[cfg(test)]
mod firmware_file_name_tests {
    use super::Model;

    /// The names Pioneer publishes, and the ones that are not ours.
    #[test]
    fn places_an_update_file_by_its_name() {
        for (name, want) in [
            ("CDJ3Kv322.UPD", Some(Model::Cdj3k)),
            ("CDJ3Kv000.UPD", Some(Model::Cdj3k)),
            ("/a/b/CDJ3000Xv140.UPD", Some(Model::Cdj3kx)),
            (r"C:\Users\u\Downloads\CDJ3Kv322.UPD", Some(Model::Cdj3k)),
            ("cdj3000xv140.upd", Some(Model::Cdj3kx)),
            ("CDJ1500Xv110.UPD", Some(Model::Cdj1500x)),
            ("EP166v110.UPD", Some(Model::Cdj1500x)),
            // Other Pioneer products, and a name we cannot place.
            ("XDJAZv130.UPD", None),
            ("DJM-A9_119.upd", None),
            ("firmware.UPD", None),
            ("CDJ3000v140.UPD", None),
        ] {
            assert_eq!(Model::from_firmware_file_name(name), want, "{name}");
        }
        assert_eq!(
            Model::from_firmware_file_name("CDJ3000Xv140.UPD"),
            Some(Model::Cdj3kx)
        );
    }
}
