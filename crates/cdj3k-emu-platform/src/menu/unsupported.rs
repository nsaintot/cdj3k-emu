//! Hosts with no native menu: the in-window strip.

use super::egui_provider::EguiProvider;
use super::provider::MenuProvider;

pub fn draws_in_window() -> bool {
    true
}

pub fn host_provider() -> Box<dyn MenuProvider> {
    Box::<EguiProvider>::default()
}
