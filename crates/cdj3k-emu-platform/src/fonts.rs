//! Font-family identifiers shared by the font registration site
//! (`app/cdj3k-emu/src/main.rs`) and every drawing module that picks a
//! family via `egui::FontFamily::Name(...)`.
//!
//! These are `&'static str` constants, not an enum, so they go straight into
//! `egui::FontFamily::Name(<str>.into())` without conversion.

/// The default proportional family, used for the rest of the chrome.
pub const NIMBUS_SANS: &str = "nimbus-sans";

/// The bold weight, used for emphasized labels such as BEAT SYNC and the
/// hot-cue glyphs.
pub const NIMBUS_SANS_BOLD: &str = "nimbus-sans-bold";

/// The condensed proportional family, used where a label must fit a narrow
/// column, such as QUANTIZE and TAG TRACK.
pub const NIMBUS_SANS_CONDENSED: &str = "nimbus-sans-condensed";
