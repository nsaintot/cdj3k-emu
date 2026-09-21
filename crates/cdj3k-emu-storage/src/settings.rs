//! Persistent settings: one file per slot.
//!
//! Layout (macOS):
//!   ~/Library/Application Support/<BUNDLE_ID>/instance-N/settings.txt
//!
//! [`InstanceSettings`] and [`PanelSettings`] own disjoint sets of keys in
//! that one file. A slot's identity, hardware and menu state live in the
//! first, the deck's own knobs in the second. [`kv`] holds the file layer both
//! go through - the paths, the per-file key registries and the locked atomic
//! write - and [`identity`] mints and validates the two values a slot is known
//! to the guest by.
//!
//! Format is plain `key=value\n` lines. No serde dep; the value space is tiny
//! and the file is human-editable for debugging.
//!
//! The UI, menu and runtime-worker threads all persist: every write lands
//! atomically (temp file + rename, so a reader never sees a truncated file)
//! and every load-modify-save runs under one process-wide lock
//! ([`InstanceSettings::update`]), so two threads cannot lose each other's
//! fields.

mod identity;
mod instance;
mod kv;
mod panel;

pub use instance::InstanceSettings;
pub use kv::prune_app_file;
pub use panel::PanelSettings;
