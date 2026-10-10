//! Persistent settings: one file per slot, and one for the app.
//!
//! A slot's file is `instance-N/settings.txt` in the per-user data directory
//! ([`crate::app_data_dir`]).
//!
//! [`InstanceSettings`] and [`PanelSettings`] own disjoint sets of keys in
//! that one file. A slot's identity, hardware and menu state live in the
//! first, the deck's own knobs in the second. [`kv`] holds the file layer both
//! go through - the paths, the per-file key registries and the locked atomic
//! write - and [`identity`] mints and validates the two values a slot is known
//! to the guest by. [`AppSettings`] owns the app-wide file, which holds the
//! updater's choices.
//!
//! The format is plain `key=value\n` lines. The crate does not use serde: the
//! value space is small, and the file stays editable by hand for debugging.
//!
//! The UI, menu and runtime-worker threads all persist: every write lands
//! atomically (a temporary file renamed into place, so a reader never sees a
//! truncated file)
//! and every load-modify-save runs under one process-wide lock
//! ([`InstanceSettings::update`]), so two threads cannot lose each other's
//! fields.

mod app;
mod identity;
mod instance;
mod kv;

pub(crate) use kv::{kv_text, read_kv};
mod panel;

pub use app::AppSettings;
pub use instance::InstanceSettings;
pub use kv::prune_app_file;
pub use panel::PanelSettings;
