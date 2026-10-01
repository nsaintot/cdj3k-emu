//! Linux: the in-window strip under the window manager's title bar.

use super::egui_provider::EguiProvider;
use super::provider::MenuProvider;

pub fn draws_in_window() -> bool {
    true
}

pub fn host_provider() -> Box<dyn MenuProvider> {
    Box::<EguiProvider>::default()
}
