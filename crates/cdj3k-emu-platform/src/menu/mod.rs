//! The application menu.
//!
//! The menu is split in two so that toolkits with nothing in common can
//! present it:
//!
//! * [`service::MenuService`] owns the content: which rows exist, what they
//!   say and which are ticked. [`action::apply`] owns what a click does.
//!   Both are portable and know nothing about how the menu is drawn.
//! * A [`provider::MenuProvider`] draws a [`model::MenuModel`] and reports the
//!   [`id::MenuId`]s that were clicked: the native bar through `muda` on
//!   macOS, the in-window strip ([`egui_provider`]) elsewhere.
//!
//! Adding a menu entry therefore changes only the service.

use std::cell::RefCell;

pub mod action;
pub mod egui_provider;
pub mod id;
mod launch;
pub mod model;
pub mod provider;
mod service;

#[cfg(target_os = "macos")]
mod muda_provider;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
#[path = "unsupported.rs"]
mod imp;

use imp::{draws_in_window, host_provider};

pub use action::ActionEffect;
pub use id::MenuId;
pub use model::{MenuModel, MenuNode, Predefined};
pub use provider::MenuProvider;

use crate::menu_state;
use launch::{
    launch_instance, show_midi_driver_alert, show_net_error_alert, show_raw_disk_alert,
    show_usb_error_alert,
};

struct Menu {
    service: service::MenuService,
    provider: Box<dyn MenuProvider>,
}

thread_local! {
    static MENU: RefCell<Option<Menu>> = const { RefCell::new(None) };
}

/// The height the in-window menu takes from the window, or 0 where the host
/// draws its own menu.
///
/// The chassis has a fixed aspect, so the aspect lock works with the window
/// height minus the strip. Otherwise the deck inside the window is
/// letterboxed.
pub fn in_window_bar_height() -> f32 {
    if draws_in_window() {
        egui_provider::BAR_HEIGHT
    } else {
        0.0
    }
}

/// Draw the in-window menu. Call it once per frame, before the chassis. It
/// does nothing where the host draws the menu itself.
pub fn draw_in_window(ui: &mut egui::Ui) {
    with_egui_provider(|p| p.draw(ui));
}

/// Run the menu's keyboard shortcuts without drawing it.
///
/// A screen with its own chrome uses this. The startup picker has its own slot
/// switcher, so a strip above it would repeat it, and `Ctrl Q` and the other
/// shortcuts still work.
pub fn shortcuts_only(ctx: &egui::Context) {
    with_egui_provider(|p| p.shortcuts_only(ctx));
}

/// Where the in-window menu drew this frame: the strip, its open panels and
/// any tooltip, in points. Empty where the host draws the menu itself.
///
/// A pass painted over the whole window, such as the deck's bloom at
/// `Order::Debug` above every menu, uses this to leave the chrome alone.
pub fn chrome_rects(ctx: &egui::Context) -> Vec<egui::Rect> {
    let mut rects = Vec::new();
    with_egui_provider(|p| rects.extend_from_slice(p.chrome()));
    if rects.is_empty() {
        return rects;
    }
    ctx.memory(|m| {
        for layer in m.areas().visible_layer_ids() {
            if layer.order == egui::Order::Tooltip {
                rects.extend(m.area_rect(layer.id));
            }
        }
    });
    rects
}

fn with_egui_provider(f: impl FnOnce(&mut egui_provider::EguiProvider)) {
    MENU.with(|cell| {
        if let Some(menu) = cell.borrow_mut().as_mut() {
            // The provider is behind `dyn MenuProvider`, and only the
            // in-window one draws.
            if let Some(egui) = menu.provider.as_egui() {
                f(egui);
            }
        }
    });
}

/// Open another slot's window, as the Instances menu does. The slot switcher
/// in the setup window's identity strip uses it.
pub fn open_instance(target: u32) {
    launch_instance(target);
}

/// Build the menu and install it. Call it once, on the first frame, on the
/// main thread.
pub fn setup_menu() {
    let mut menu = Menu {
        service: service::MenuService::new(),
        provider: host_provider(),
    };
    let model = menu.service.model();
    menu.provider.apply(&model);

    MENU.with(|cell| *cell.borrow_mut() = Some(menu));
}

/// Deliver clicks and refresh the menu. Call it every frame, on the main
/// thread.
pub fn sync_menu() {
    drain_alerts();

    let effects = MENU.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(menu) = borrow.as_mut() else {
            return Vec::new();
        };

        // One lock for the whole batch: `menu_state::lock()` is not reentrant,
        // and two clicks can arrive in the same frame.
        let clicked = menu.provider.poll();
        let effects = if clicked.is_empty() {
            Vec::new()
        } else {
            let mut s = menu_state::lock();
            clicked
                .iter()
                .map(|id| action::apply(&mut s, id))
                .filter(|e| *e != ActionEffect::None)
                .collect()
        };

        // Rebuild after the clicks land so a toggle shows its new state on the
        // same frame it was clicked.
        let model = menu.service.model();
        menu.provider.apply(&model);

        effects
    });

    // Perform the effects outside the borrow: a file dialog runs its own event
    // loop, and launching a slot takes the state lock again.
    for effect in effects {
        perform(effect);
    }
}

/// Carry out the host-toolkit half of a click.
fn perform(effect: ActionEffect) {
    match effect {
        ActionEffect::None => {}
        ActionEffect::PickImageToCreate => pick_image(true),
        ActionEffect::PickImageToMount => pick_image(false),
        ActionEffect::LaunchInstance(n) => launch_instance(n),
    }
}

/// True while one of the app's file dialogs is open, so that a second click
/// cannot open another one behind it.
static PICKER_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask for an image, off the UI thread.
///
/// A file dialog blocks until it is answered. On the frame loop it would stop
/// the app from answering the compositor, which then reports the app as not
/// responding. The answer goes into `menu_state` for the worker's next poll.
fn pick_image(create: bool) {
    use std::sync::atomic::Ordering;
    if PICKER_OPEN.swap(true, Ordering::AcqRel) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("cdj3k-emu-file-dialog".into())
        .spawn(move || {
            let picked = if create {
                rfd::FileDialog::new()
                    .set_file_name(format!("usb.{}", crate::host::VIRTUAL_IMAGE_EXT))
                    .save_file()
            } else {
                rfd::FileDialog::new().pick_file()
            };
            if let Some(path) = picked {
                let mut s = menu_state::lock();
                s.usb_virtual_img = Some(path);
                if create {
                    s.usb_create_req = true;
                } else {
                    s.usb_virtual_mount_req = true;
                }
            }
            PICKER_OPEN.store(false, Ordering::Release);
        })
        .inspect_err(|_| PICKER_OPEN.store(false, Ordering::Release));
}

/// Show the alerts that other threads raise by setting a flag. Each flag is
/// taken before its dialog opens, so a blocked frame cannot raise it again.
fn drain_alerts() {
    let perm_denied = menu_state::lock().usb_phys_perm_denied.take();
    if let Some(prompt) = perm_denied {
        show_raw_disk_alert(prompt);
    }
    if std::mem::take(&mut menu_state::lock().midi_driver_replaced) {
        show_midi_driver_alert();
    }
    // Drop the guard before the dialog opens, so other threads are not blocked
    // while it is up.
    let net_err = menu_state::lock().net_error_message.take();
    if let Some(msg) = net_err {
        show_net_error_alert(&msg);
    }
    let usb_err = menu_state::lock().usb_error_message.take();
    if let Some(msg) = usb_err {
        show_usb_error_alert(&msg);
    }
}
