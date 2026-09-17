// egui draw helpers take their geometry and colours as plain arguments.
#![allow(clippy::too_many_arguments)]

pub mod app;

pub use app::firmware_wizard::provision_blocking;
pub use app::shell::{CdjShell, LaunchOutcome, RuntimeHost, ShellConfig};
