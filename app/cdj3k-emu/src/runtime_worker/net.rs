//! Network interface selection: attaching the guest to the chosen interface
//! and restarting QEMU onto it.

use std::time::Instant;

use cdj3k_emu_platform::menu_state;
use cdj3k_emu_runtime::mods_report::ModsReport;
use cdj3k_emu_runtime::{NetAttachment, NetKeepAlive, QemuConfig};
use cdj3k_emu_storage::mods::BootMods;

use super::qemu::apply_menu_to_config;
use super::{persist_inst, Worker, NET_REFRESH_INTERVAL};

/// Point `config` at a successful attachment on `name` and return what has to
/// stay alive while QEMU uses it.
pub fn adopt_attachment(config: &mut QemuConfig, name: &str, a: NetAttachment) -> NetKeepAlive {
    eprintln!("cdj3k-emu: the guest is bridged on {name}.");
    config.net_vmnet = a.vmnet;
    config.net_tap_iface = a.tap_iface;
    config.net_tap_fd = a.tap_fd;
    a.keep
}

impl Worker {
    pub(super) fn switch_network(&mut self, selected: u32) {
        self.prev_net_idx = selected;
        self._active_net = NetKeepAlive::Nothing;
        self.config.net_vmnet = None;
        self.config.net_tap_iface = None;
        self.config.net_tap_fd = None;

        if let Some((name, result)) = self.attach_selection(selected) {
            match result {
                Ok(a) => self._active_net = adopt_attachment(&mut self.config, &name, a),
                Err(e) => self.reject_selection(&name, &e),
            }
        }

        let boot = apply_menu_to_config(&mut self.config, &self.mods);
        self.restart_for_network(boot);

        let final_name = persisted_iface_name();
        persist_inst(self.config.instance_id, |s| s.net_iface = final_name);
    }

    /// Attach to the selected interface. Returns `None` when no interface is
    /// selected.
    fn attach_selection(&self, selected: u32) -> Option<(String, std::io::Result<NetAttachment>)> {
        match selected {
            menu_state::NET_SEL_NONE => None,
            menu_state::NET_SEL_VMNET_HOST => Some((
                "host-only".to_string(),
                cdj3k_emu_runtime::net_attach_host_only(),
            )),
            idx => {
                // Release the menu lock before net_attach, because net_attach
                // can wait on an elevation prompt and the UI locks the menu
                // state every frame.
                let name = menu_state::lock()
                    .net_ifaces
                    .get(idx as usize)
                    .map(|i| i.name.clone());
                // `launch.rs` makes the same call, so a selection made while
                // QEMU runs behaves like one made at boot.
                name.map(|name| {
                    let r = cdj3k_emu_runtime::net_attach(
                        &name,
                        &self.config.mac.clone().unwrap_or_default(),
                        self.config.instance_id,
                    );
                    (name, r)
                })
            }
        }
    }

    /// Revert the menu to no interface and show why.
    fn reject_selection(&mut self, name: &str, e: &std::io::Error) {
        let msg = format!("cannot bridge on {name}: {e}");
        eprintln!("cdj3k-emu: {msg}");
        let mut s = menu_state::lock();
        s.selected_interface = menu_state::NET_SEL_NONE;
        s.net_error_message = Some(msg);
        self.prev_net_idx = menu_state::NET_SEL_NONE;
    }

    /// Restart a running QEMU on the new network backend. `boot` holds the
    /// mods for the restart.
    fn restart_for_network(&mut self, boot: BootMods) {
        let Some(inst) = self.instance.as_mut() else {
            menu_state::lock().shade_forced = false;
            return;
        };
        let (prev_phys, prev_virt) = {
            let s = menu_state::lock();
            (s.usb_phys_mounted_idx, s.usb_virtual_mounted)
        };
        self.cfg_client.new_boot();
        self.last_mods_report = ModsReport::Unknown;
        self.mod_logs_due.clear();
        match inst.restart(self.config.clone()) {
            Ok(()) => {
                cdj3k_emu_storage::mods::set_current_boot(boot);
                self.reset_usb_and_queue_remount(prev_phys, prev_virt);
                self.alc_pushed_for_boot = false;
                menu_state::lock().shade_forced = false;
            }
            Err(e) => eprintln!("cdj3k-emu: restarting after the network change failed: {e:?}"),
        }
    }

    pub(super) fn refresh_net_list(&mut self) {
        if self.last_net_refresh.elapsed() >= NET_REFRESH_INTERVAL {
            menu_state::refresh_net_interfaces();
            self.last_net_refresh = Instant::now();
        }
    }
}

/// The selection as it is saved: the interface name, the vmnet host-only
/// token, or `None` when no interface is selected, by the user or after a
/// failed attach.
fn persisted_iface_name() -> Option<String> {
    let s = menu_state::lock();
    match s.selected_interface {
        menu_state::NET_SEL_NONE => None,
        menu_state::NET_SEL_VMNET_HOST => Some(menu_state::NET_IFACE_VMNET_HOST_TOKEN.to_string()),
        idx => s.net_ifaces.get(idx as usize).map(|i| i.name.clone()),
    }
}
