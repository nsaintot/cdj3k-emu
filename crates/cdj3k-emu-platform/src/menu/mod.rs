//! The application menu.
//!
//! Split in two so the same menu can be presented by toolkits that have
//! nothing in common:
//!
//! * [`service::MenuService`] owns the content — which rows exist, what they
//!   say, which are ticked — and [`action::apply`] owns what a click does.
//!   Both are portable and know nothing about how the menu is drawn.
//! * A [`provider::MenuProvider`] draws a [`model::MenuModel`] and reports the
//!   [`id::MenuId`]s that were clicked: the native bar through `muda` on
//!   macOS, the in-window strip ([`egui_provider`]) elsewhere.
//!
//! Adding a menu entry therefore means touching the service and nothing else.

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

/// Height the in-window menu takes out of the window, or 0 where the menu is
/// the host's own.
///
/// The chassis has a fixed aspect, so whatever the strip occupies has to come
/// off the height the aspect lock works with — otherwise the window is right
/// and the deck inside it is letterboxed.
pub fn in_window_bar_height() -> f32 {
    if draws_in_window() {
        egui_provider::BAR_HEIGHT
    } else {
        0.0
    }
}

/// Draw the in-window menu. Call once per frame, before the chassis; a no-op
/// where the host draws the menu itself.
pub fn draw_in_window(ui: &mut egui::Ui) {
    with_egui_provider(|p| p.draw(ui));
}

/// Run the menu's keyboard shortcuts without drawing it.
///
/// For a screen that carries its own chrome — the startup picker has a slot
/// switcher of its own, so a strip above it would say the same thing twice —
/// while keeping `Ctrl Q` and the rest live.
pub fn shortcuts_only(ctx: &egui::Context) {
    with_egui_provider(|p| p.shortcuts_only(ctx));
}

/// Where the in-window menu drew this frame: the strip, its open panels and
/// any tooltip, in points. Empty where the host draws the menu itself.
///
/// For a pass painted over the whole window — the deck's bloom runs at
/// `Order::Debug`, above every menu — to leave the chrome alone.
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

/// Bring up another slot's window — what the Instances menu does, exposed for
/// the slot switcher in the setup window's identity strip.
pub fn open_instance(target: u32) {
    launch_instance(target);
}

/// Build the menu and install it. Call once on the first frame, main thread.
pub fn setup_menu() {
    let mut menu = Menu {
        service: service::MenuService::new(),
        provider: host_provider(),
    };
    let model = menu.service.model();
    menu.provider.apply(&model);

    MENU.with(|cell| *cell.borrow_mut() = Some(menu));
}

/// Deliver clicks and refresh the menu. Call every frame, main thread.
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

    // Outside the borrow: a file dialog spins its own event loop, and
    // launching a slot re-reads the state lock.
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

/// True while a file dialog of ours is up, so a second click cannot open a
/// second one behind the first.
static PICKER_OPEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask for an image, off the UI thread.
///
/// A file dialog blocks until answered; run on the frame loop it would stop
/// the app answering the compositor, which then reports it as not
/// responding. The answer lands in `menu_state` for the worker's next poll.
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

/// Modal conditions other threads raise by setting a flag. Each is taken
/// before the dialog goes up, so a blocked frame cannot re-raise it.
fn drain_alerts() {
    let perm_denied = menu_state::lock().usb_phys_perm_denied.take();
    if let Some(prompt) = perm_denied {
        show_raw_disk_alert(prompt);
    }
    if std::mem::take(&mut menu_state::lock().midi_driver_replaced) {
        show_midi_driver_alert();
    }
    // Drop the guard before the modal so other threads aren't blocked while
    // it is up.
    let net_err = menu_state::lock().net_error_message.take();
    if let Some(msg) = net_err {
        show_net_error_alert(&msg);
    }
    let usb_err = menu_state::lock().usb_error_message.take();
    if let Some(msg) = usb_err {
        show_usb_error_alert(&msg);
    }
}
