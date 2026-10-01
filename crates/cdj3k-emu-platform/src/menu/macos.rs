//! macOS: the native menu bar, or the in-window strip when
//! `CDJ3K_MENU_IN_WINDOW` is set (to exercise the Linux and Windows menu).

use super::provider::MenuProvider;
use super::{egui_provider, muda_provider};

const IN_WINDOW_ENV: &str = "CDJ3K_MENU_IN_WINDOW";

pub fn draws_in_window() -> bool {
    std::env::var_os(IN_WINDOW_ENV).is_some_and(|v| v != "0")
}

pub fn host_provider() -> Box<dyn MenuProvider> {
    if draws_in_window() {
        return Box::<egui_provider::EguiProvider>::default();
    }
    Box::<muda_provider::MudaProvider>::default()
}
