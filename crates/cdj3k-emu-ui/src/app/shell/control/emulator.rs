//! The control API's `media` and `setting` requests. Both act through the
//! menu's state, so a request does what the matching menu item does: the
//! runtime worker carries it out and saves it for the slot.

use std::path::PathBuf;
use std::time::Instant;

use cdj3k_emu_control::command::MEDIA_WAIT;
use cdj3k_emu_control::{self as control, MediaCommand, Responder};
use cdj3k_emu_platform::menu::action::apply;
use cdj3k_emu_platform::menu::id::MenuId;
use cdj3k_emu_platform::menu_state::{
    self, AppState, NET_SEL_NONE, NET_SEL_VMNET_HOST, NO_USB_MOUNTED,
};
use serde_json::{json, Value};

use super::{done, Answer, Refused};
use crate::app::shell::CdjShell;

/// A `media attach` or `media eject` waiting for the runtime worker.
pub(super) struct PendingMedia {
    /// The image an attach waits for; `None` for an eject.
    attach: Option<PathBuf>,
    deadline: Instant,
    reply: Responder,
}

/// The settings `setting` reads and sets, as they appear in the menu.
const SETTINGS: &[&str] = &[
    "audio",
    "alc",
    "haptics",
    "pc_link",
    "service_mode",
    "mods",
    "network",
];

impl CdjShell {
    /// Start an attach or an eject, or answer `media` with the port's state.
    pub(super) fn start_media(&mut self, command: MediaCommand, reply: Responder) -> Refused {
        if matches!(command, MediaCommand::Status) {
            reply.send(media_state(&menu_state::lock()));
            return None;
        }
        if cdj3k_emu_runtime::worker_is_finished() {
            return Some((reply, "the emulation is not running".into()));
        }
        if self.control.media.is_some() {
            return Some((reply, "a media change is under way".into()));
        }
        let mut s = menu_state::lock();
        let attach = match command {
            MediaCommand::Attach(path) if !path.is_file() => {
                return Some((reply, format!("{} is not a file", path.display())));
            }
            MediaCommand::Attach(path) => {
                if s.usb_virtual_mounted && s.usb_virtual_img.as_ref() == Some(&path) {
                    reply.send(media_state(&s));
                    return None;
                }
                // The worker sets it again once this image is attached.
                s.usb_virtual_mounted = false;
                s.usb_virtual_img = Some(path.clone());
                s.usb_virtual_mount_req = true;
                Some(path)
            }
            _ if !s.usb_virtual_mounted && s.usb_phys_mounted_idx == NO_USB_MOUNTED => {
                reply.send(media_state(&s));
                return None;
            }
            _ => {
                s.usb_eject_req = true;
                None
            }
        };
        drop(s);
        self.control.media = Some(PendingMedia {
            attach,
            deadline: Instant::now() + MEDIA_WAIT,
            reply,
        });
        None
    }

    /// Answer the media request once the port has reached the state it asked
    /// for, or once its time is up.
    pub(super) fn settle_media(&mut self) {
        let Some(pending) = &self.control.media else {
            return;
        };
        let s = menu_state::lock();
        let reached = match &pending.attach {
            Some(path) => s.usb_virtual_mounted && s.usb_virtual_img.as_ref() == Some(path),
            None => !s.usb_virtual_mounted && s.usb_phys_mounted_idx == NO_USB_MOUNTED,
        };
        let answer = if reached {
            media_state(&s)
        } else if Instant::now() >= pending.deadline {
            control::error("the media change did not finish in time; see the app's log")
        } else {
            return;
        };
        drop(s);
        if let Some(pending) = self.control.media.take() {
            pending.reply.send(answer);
        }
    }

    /// Read every setting, or set one as its menu item does.
    pub(super) fn on_setting(&mut self, setting: Option<(String, String)>) -> Answer {
        let Some((name, value)) = setting else {
            menu_state::refresh_net_interfaces();
            let s = menu_state::lock();
            let mut m = control::ok();
            for name in SETTINGS {
                m.insert((*name).into(), setting_value(&s, name));
            }
            return Ok(m.into());
        };
        menu_state::refresh_net_interfaces();
        let mut s = menu_state::lock();
        let Some(id) = setting_change(&s, &name, &value)? else {
            return done();
        };
        apply(&mut s, &id);
        let restarts = matches!(
            id,
            MenuId::Audio
                | MenuId::ServiceMode
                | MenuId::NetNat
                | MenuId::NetVmnetHost
                | MenuId::NetInterface(_)
        ) || (id == MenuId::ModsEnabled && s.qemu_running);
        drop(s);
        if restarts {
            self.control.restarting = true;
        }
        Ok(json!({ "ok": true, "restarts": restarts }))
    }
}

/// The USB port's state for a `media` reply.
fn media_state(s: &AppState) -> Value {
    let attached = if s.usb_virtual_mounted {
        "virtual"
    } else if s.usb_phys_mounted_idx != NO_USB_MOUNTED {
        "physical"
    } else {
        "none"
    };
    json!({
        "ok": true,
        "attached": attached,
        "image": s.usb_virtual_mounted.then(|| s.usb_virtual_img.clone()).flatten(),
    })
}

fn setting_value(s: &AppState, name: &str) -> Value {
    match name {
        "audio" => s.audio_enabled.into(),
        "alc" => s.alc_enabled.into(),
        "haptics" => s.haptic_enabled.into(),
        "pc_link" => (s.pc_link_supported && s.pc_link_enabled).into(),
        "service_mode" => s.service_mode.into(),
        "mods" => s.mods_enabled.into(),
        _ => match s.selected_interface {
            NET_SEL_NONE => "nat".into(),
            NET_SEL_VMNET_HOST => "host".into(),
            i => s
                .net_ifaces
                .get(i as usize)
                .map_or(Value::Null, |n| n.name.clone().into()),
        },
    }
}

/// The menu action that sets `name` to `value`; `None` when it already has
/// that value.
fn setting_change(s: &AppState, name: &str, value: &str) -> Result<Option<MenuId>, String> {
    let on = || match value.to_ascii_lowercase().as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => Err(format!("{name} takes on or off")),
    };
    let toggle = |current: bool, id: MenuId| Ok((current != on()?).then_some(id));
    match name {
        "audio" => toggle(s.audio_enabled, MenuId::Audio),
        "alc" => toggle(s.alc_enabled, MenuId::Alc),
        "haptics" => toggle(s.haptic_enabled, MenuId::Haptic),
        "pc_link" if !s.pc_link_supported => Err("this host has no PC Link".into()),
        "pc_link" => toggle(s.pc_link_enabled, MenuId::PcLink),
        "service_mode" => toggle(s.service_mode, MenuId::ServiceMode),
        "mods" => toggle(s.mods_enabled, MenuId::ModsEnabled),
        "network" => network_change(s, value),
        _ => Err(format!(
            "{name:?} is not a setting: {}",
            SETTINGS.join(", ")
        )),
    }
}

/// The menu action that selects the network `value`: `nat`, `host` or a host
/// interface's name; `None` when it is already selected.
fn network_change(s: &AppState, value: &str) -> Result<Option<MenuId>, String> {
    let (selection, id) = match value {
        "nat" => (NET_SEL_NONE, MenuId::NetNat),
        "host" if !cdj3k_emu_platform::net::HOST_ONLY => {
            return Err("this host has no host-only network".into())
        }
        "host" => (NET_SEL_VMNET_HOST, MenuId::NetVmnetHost),
        iface => {
            let i = s
                .net_ifaces
                .iter()
                .position(|n| n.name == iface)
                .ok_or_else(|| format!("{iface:?} is not nat, host or a network interface"))?
                as u32;
            (i, MenuId::NetInterface(i))
        }
    };
    Ok((s.selected_interface != selection).then_some(id))
}
