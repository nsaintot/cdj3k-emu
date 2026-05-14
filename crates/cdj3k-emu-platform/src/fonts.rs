//! Font-family identifiers shared by the font registration site
//! (`app/cdj3k-emu/src/main.rs`) and every drawing module that picks a
//! family via `egui::FontFamily::Name(...)`.
//!
//! Keeping these as `&'static str` rather than an enum so they slot
//! directly into `egui::FontFamily::Name(<str>.into())` without conversion.

/// Default proportional family — the "rest of the chrome" font.
pub const NIMBUS_SANS: &str = "nimbus-sans";

/// Bold weight — used for emphatic labels (BEAT SYNC, hot-cue glyphs, etc.).
pub const NIMBUS_SANS_BOLD: &str = "nimbus-sans-bold";

/// Condensed proportional — used where a label needs to fit a narrow column
/// (QUANTIZE, TAG TRACK, etc.).
pub const NIMBUS_SANS_CONDENSED: &str = "nimbus-sans-condensed";
