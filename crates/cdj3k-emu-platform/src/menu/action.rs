//! What a click does.
//!
//! [`apply`] is the only place a menu action changes state, for every
//! provider. It mutates `menu_state` and nothing else: anything needing the
//! host toolkit — a file picker, a new process — comes back as an
//! [`ActionEffect`] for the caller to carry out once the lock is released.

use super::id::MenuId;
use crate::menu_state;

/// Work a click implies that [`apply`] leaves to the caller.
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
        MenuId::ManageEmulation => {
            s.manage_emulation_requested = true;
        }
        MenuId::ModsEnabled => {
            s.mods_enabled = !s.mods_enabled;
            s.mods_toggle_requested = true;
            if s.qemu_running {
                s.shade_forced = true;
                s.restart_requested = true;
            }
        }
        MenuId::CheckForUpdate => {
            s.update_check_requested = true;
        }
        MenuId::Restart => {
            s.shade_forced = true;
            s.restart_requested = true;
        }
        MenuId::ScreenExtended => {
            s.screen_extended = !s.screen_extended;
        }
        MenuId::ServiceMode => {
            s.service_mode = !s.service_mode;
            s.shade_forced = true;
            s.restart_requested = true;
        }
        MenuId::Audio => {
            s.audio_enabled = !s.audio_enabled;
            s.shade_forced = true;
            s.audio_toggle_requested = true;
        }
        MenuId::Alc => {
            // Runtime worker consumes this and pushes
            // `set audio_sync_enabled <0|1>` to the guest cfg daemon.
            s.alc_enabled = !s.alc_enabled;
            s.alc_toggle_requested = true;
        }
        MenuId::Haptic => {
            // Pure host-side toggle. The jog physics layer reads the flag at
            // each detent-crossing; `haptic_toggle_requested` tells the runtime
            // worker to persist it. No QEMU restart, no guest push.
            s.haptic_enabled = !s.haptic_enabled;
            s.haptic_toggle_requested = true;
        }
        MenuId::PcLink => {
            // Flip the user-facing state immediately so the checkmark feels
            // responsive; the runtime worker reads `pc_link_toggle_requested`
            // and does the cfgd command plus the PcLink construct/drop.
            s.pc_link_enabled = !s.pc_link_enabled;
            s.pc_link_toggle_requested = true;
        }
        MenuId::JogScreen => s.jog_screen_popped = !s.jog_screen_popped,
        MenuId::MainScreen => s.main_screen_popped = !s.main_screen_popped,
        MenuId::DebugScreen => s.debug_screen_popped = !s.debug_screen_popped,
        MenuId::NetNat => {
            if s.selected_interface != menu_state::NET_SEL_NONE {
                s.shade_forced = true;
            }
            s.selected_interface = menu_state::NET_SEL_NONE;
        }
        MenuId::NetVmnetHost => {
            if s.selected_interface != menu_state::NET_SEL_VMNET_HOST {
                s.shade_forced = true;
            }
            s.selected_interface = menu_state::NET_SEL_VMNET_HOST;
        }
        MenuId::NetInterface(n) => {
            if s.selected_interface != *n {
                s.shade_forced = true;
            }
            s.selected_interface = *n;
        }
        MenuId::CreateVirtualUsb => return ActionEffect::PickImageToCreate,
        MenuId::MountVirtualUsb => return ActionEffect::PickImageToMount,
        MenuId::EjectCurrentMedia => {
            s.usb_eject_req = true;
        }
        MenuId::AudioDeviceDefault => {
            if s.audio_device_uid.is_some() {
                s.audio_device_uid = None;
                s.audio_device_toggle_requested = true;
                s.shade_forced = true;
            }
        }
        MenuId::AudioDevice(uid) => {
            if s.audio_device_uid.as_deref() != Some(uid.as_str()) {
                s.audio_device_uid = Some(uid.clone());
                s.audio_device_toggle_requested = true;
                s.shade_forced = true;
            }
        }
        MenuId::PhysicalDisk(i) => {
            // Click-to-switch only. Re-selecting the mounted disk is a no-op
            // (the runtime worker filters it), and ejecting goes exclusively
            // through Eject Current Media.
            let idx = *i as i32;
            if s.usb_phys_mounted_idx != idx {
                s.usb_phys_toggle_idx = idx;
            }
        }
        MenuId::Instance(n) => return ActionEffect::LaunchInstance(*n),
    }
    ActionEffect::None
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
