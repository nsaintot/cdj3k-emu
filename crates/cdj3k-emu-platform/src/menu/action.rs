//! What a click does.
//!
//! [`apply`] is the only place a menu action changes state, for every
//! provider. It changes `menu_state` and nothing else. Work that needs the
//! host toolkit, such as a file picker or a new process, comes back as an
//! [`ActionEffect`] that the caller carries out once the lock is released.

use super::id::MenuId;
use crate::menu_state;

/// Work that a click requires and that [`apply`] leaves to the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionEffect {
    None,
    /// Ask for a path to create a new virtual USB image at.
    PickImageToCreate,
    /// Ask for an existing virtual USB image to mount.
    PickImageToMount,
    /// Bring up another slot's window.
    LaunchInstance(u32),
}

/// Apply one action to the shared state.
///
/// Takes the state guard so a provider can batch several actions under one
/// acquisition; `menu_state::lock()` is not reentrant.
pub fn apply(s: &mut menu_state::AppState, id: &MenuId) -> ActionEffect {
    match id {
        MenuId::ManageEmulation
        | MenuId::ModsEnabled
        | MenuId::Restart
        | MenuId::ServiceMode
        | MenuId::Haptic
        | MenuId::PcLink
        | MenuId::CheckForUpdate => emulation(s, id),
        MenuId::ScreenExtended | MenuId::JogScreen | MenuId::MainScreen | MenuId::DebugScreen => {
            view(s, id)
        }
        MenuId::Audio | MenuId::Alc => audio(s, id),
        MenuId::AudioDeviceDefault => select_audio_device(s, None),
        MenuId::AudioDevice(uid) => select_audio_device(s, Some(uid)),
        MenuId::NetNat => select_interface(s, menu_state::NET_SEL_NONE),
        MenuId::NetVmnetHost => select_interface(s, menu_state::NET_SEL_VMNET_HOST),
        MenuId::NetInterface(n) => select_interface(s, *n),
        MenuId::EjectCurrentMedia => s.usb_eject_req = true,
        MenuId::PhysicalDisk(i) => select_physical_disk(s, *i),
        MenuId::CreateVirtualUsb => return ActionEffect::PickImageToCreate,
        MenuId::MountVirtualUsb => return ActionEffect::PickImageToMount,
        MenuId::Instance(n) => return ActionEffect::LaunchInstance(*n),
    }
    ActionEffect::None
}

/// The Emulation menu's actions.
fn emulation(s: &mut menu_state::AppState, id: &MenuId) {
    match id {
        MenuId::ManageEmulation => s.manage_emulation_requested = true,
        MenuId::ModsEnabled => {
            s.mods_enabled = !s.mods_enabled;
            s.mods_toggle_requested = true;
            if s.qemu_running {
                s.shade_forced = true;
                s.restart_requested = true;
            }
        }
        MenuId::CheckForUpdate => s.update_check_requested = true,
        MenuId::Restart => {
            s.shade_forced = true;
            s.restart_requested = true;
        }
        MenuId::ServiceMode => {
            s.service_mode = !s.service_mode;
            s.shade_forced = true;
            s.restart_requested = true;
        }
        MenuId::Haptic => {
            // This toggle is host-side only. The jog physics reads the flag at
            // each detent crossing, and `haptic_toggle_requested` tells the
            // runtime worker to persist it. Nothing restarts QEMU or reaches
            // the guest.
            s.haptic_enabled = !s.haptic_enabled;
            s.haptic_toggle_requested = true;
        }
        MenuId::PcLink => {
            // Flip the state at once so that the checkmark responds
            // immediately. The runtime worker reads `pc_link_toggle_requested`,
            // sends the cfgd command, and creates or drops the PcLink.
            s.pc_link_enabled = !s.pc_link_enabled;
            s.pc_link_toggle_requested = true;
        }
        _ => {}
    }
}

/// The View menu's toggles.
fn view(s: &mut menu_state::AppState, id: &MenuId) {
    match id {
        MenuId::ScreenExtended => s.screen_extended = !s.screen_extended,
        MenuId::JogScreen => s.jog_screen_popped = !s.jog_screen_popped,
        MenuId::MainScreen => s.main_screen_popped = !s.main_screen_popped,
        MenuId::DebugScreen => s.debug_screen_popped = !s.debug_screen_popped,
        _ => {}
    }
}

/// The Audio menu's toggles.
fn audio(s: &mut menu_state::AppState, id: &MenuId) {
    match id {
        MenuId::Audio => {
            s.audio_enabled = !s.audio_enabled;
            s.shade_forced = true;
            s.audio_toggle_requested = true;
        }
        MenuId::Alc => {
            // The runtime worker consumes this and sends
            // `set audio_sync_enabled <0|1>` to the guest cfg daemon.
            s.alc_enabled = !s.alc_enabled;
            s.alc_toggle_requested = true;
        }
        _ => {}
    }
}

/// Pin output to one device, or follow the system default with `None`.
fn select_audio_device(s: &mut menu_state::AppState, uid: Option<&String>) {
    if s.audio_device_uid.as_ref() != uid {
        s.audio_device_uid = uid.cloned();
        s.audio_device_toggle_requested = true;
        s.shade_forced = true;
    }
}

/// Select a network. Selecting a different network restarts QEMU, so the boot
/// shade comes down at once.
fn select_interface(s: &mut menu_state::AppState, selection: u32) {
    if s.selected_interface != selection {
        s.shade_forced = true;
    }
    s.selected_interface = selection;
}

/// Switch to the clicked disk. Selecting the mounted disk again does nothing,
/// and only Eject Current Media ejects.
fn select_physical_disk(s: &mut menu_state::AppState, index: usize) {
    let idx = index as i32;
    if s.usb_phys_mounted_idx != idx {
        s.usb_phys_toggle_idx = idx;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> menu_state::AppState {
        menu_state::AppState::default()
    }

    #[test]
    fn a_toggle_flips_and_arms_its_request() {
        let mut s = state();
        let before = s.audio_enabled;
        assert_eq!(apply(&mut s, &MenuId::Audio), ActionEffect::None);
        assert_eq!(s.audio_enabled, !before);
        assert!(s.audio_toggle_requested);
        assert!(s.shade_forced);
    }

    #[test]
    fn haptics_do_not_force_a_shade_because_nothing_restarts() {
        let mut s = state();
        apply(&mut s, &MenuId::Haptic);
        assert!(s.haptic_toggle_requested);
        assert!(!s.shade_forced);
    }

    #[test]
    fn reselecting_the_current_network_does_not_force_a_shade() {
        let mut s = state();
        s.selected_interface = menu_state::NET_SEL_NONE;
        apply(&mut s, &MenuId::NetNat);
        assert!(!s.shade_forced);

        apply(&mut s, &MenuId::NetVmnetHost);
        assert!(s.shade_forced);
        assert_eq!(s.selected_interface, menu_state::NET_SEL_VMNET_HOST);
    }

    #[test]
    fn reselecting_the_current_audio_device_is_inert() {
        let mut s = state();
        s.audio_device_uid = Some("uid-a".into());
        apply(&mut s, &MenuId::AudioDevice("uid-a".into()));
        assert!(!s.audio_device_toggle_requested);

        apply(&mut s, &MenuId::AudioDevice("uid-b".into()));
        assert!(s.audio_device_toggle_requested);
        assert_eq!(s.audio_device_uid.as_deref(), Some("uid-b"));
    }

    #[test]
    fn selecting_the_mounted_disk_does_not_arm_a_switch() {
        let mut s = state();
        s.usb_phys_mounted_idx = 1;
        apply(&mut s, &MenuId::PhysicalDisk(1));
        assert_eq!(s.usb_phys_toggle_idx, state().usb_phys_toggle_idx);

        apply(&mut s, &MenuId::PhysicalDisk(2));
        assert_eq!(s.usb_phys_toggle_idx, 2);
    }

    #[test]
    fn the_effects_are_reported_not_performed() {
        let mut s = state();
        assert_eq!(
            apply(&mut s, &MenuId::CreateVirtualUsb),
            ActionEffect::PickImageToCreate
        );
        assert_eq!(
            apply(&mut s, &MenuId::MountVirtualUsb),
            ActionEffect::PickImageToMount
        );
        assert_eq!(
            apply(&mut s, &MenuId::Instance(3)),
            ActionEffect::LaunchInstance(3)
        );
        // None of the three touched state.
        assert!(!s.usb_eject_req);
        assert!(s.usb_virtual_img.is_none());
    }
}
