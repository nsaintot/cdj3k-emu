//! Runtime bring-up for one model: resolve its firmware, build the QEMU
//! config from the slot's settings, pre-build the network backend, spawn QEMU
//! and hand everything to the runtime worker. Implements the shell's
//! [`RuntimeHost`] so the picker can boot whichever model was chosen.

use std::path::PathBuf;

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::menu_state;
use cdj3k_emu_storage::FirmwarePaths;
use cdj3k_emu_ui::{LaunchOutcome, RuntimeHost};

/// Boot options fixed for the process (CLI flags), applied to every launch.
pub struct Host {
    pub instance: u32,
    /// `--kernel` override (dev), else the provisioned kernel of the model.
    pub kernel: Option<PathBuf>,
    /// `--initramfs` override (dev), else the provisioned initramfs.
    pub initramfs: Option<PathBuf>,
    pub no_emmc: bool,
    pub serial_log: bool,
    /// `--no-spawn`: never boot anything.
    pub ui_only: bool,
}

/// Record `model` as the slot's model so the next launch of this slot opens
/// on it directly. Best-effort: a failed write only costs the shortcut.
fn persist_model(instance: u32, model: Model) {
    let r = cdj3k_emu_storage::InstanceSettings::update(instance, |s| s.model = Some(model));
    if let Err(e) = r {
        eprintln!("cdj3k-emu: persisting the slot model failed: {e}");
    }
}

struct Resolved {
    kernel: PathBuf,
    initramfs: PathBuf,
    emmc: PathBuf,
}

impl Host {
    fn resolve(&self) -> Resolved {
        let paths = FirmwarePaths::new(self.instance);
        Resolved {
            kernel: self.kernel.clone().unwrap_or(paths.kernel),
            initramfs: self.initramfs.clone().unwrap_or(paths.initramfs),
            emmc: paths.emmc,
        }
    }
}

impl RuntimeHost for Host {
    fn installed_model(&self) -> Option<Model> {
        let r = self.resolve();
        let provisioned =
            r.kernel.exists() && r.initramfs.exists() && (self.no_emmc || r.emmc.exists());
        provisioned
            .then(|| cdj3k_emu_storage::InstanceSettings::saved_model(self.instance))
            .flatten()
    }

    fn any_model_launchable(&self) -> bool {
        self.ui_only
    }

    fn clear_firmware(&mut self) -> std::io::Result<()> {
        FirmwarePaths::new(self.instance).remove()
    }

    fn launch(&mut self, model: Model) -> LaunchOutcome {
        if self.ui_only {
            return LaunchOutcome::UiOnly;
        }
        if self.installed_model() != Some(model) {
            return LaunchOutcome::NotProvisioned;
        }
        #[cfg(target_os = "macos")]
        {
            // Recorded before the worker exists; it reads the slot settings
            // as soon as it starts.
            persist_model(self.instance, model);
            self.spawn(model);
            LaunchOutcome::Started
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = model;
            LaunchOutcome::UiOnly
        }
    }
}

#[cfg(target_os = "macos")]
impl Host {
    /// Build the QEMU config, pre-build the network backend, spawn QEMU and
    /// the runtime worker. Requires no live worker.
    fn spawn(&self, model: Model) {
        use cdj3k_emu_runtime::{QemuConfig, QemuInstance, TapBridge, VmnetMode};

        use crate::runtime_worker;

        let instance = self.instance;
        let r = self.resolve();
        let inst_settings = cdj3k_emu_storage::InstanceSettings::load_or_init(instance);

        let mut config = QemuConfig::new(r.kernel.clone(), r.initramfs.clone());
        config.instance_id = instance;
        config.model = model;
        config.ssh_port = 2222 + instance as u16;
        config.qmp_port = 4445 + instance as u16;
        config.gdb_port = 1235 + instance as u16;
        config.emmc_img = (!self.no_emmc).then(|| r.emmc.clone());
        config.audio = inst_settings.audio_enabled;
        config.audio_device_uid = inst_settings.audio_device_uid.clone();
        config.service_mode = menu_state::lock().service_mode;
        config.mac = Some(inst_settings.mac);
        config.soc_serial = Some(inst_settings.soc_serial);
        config.serial_log = self.serial_log;

        // ── Pre-build network backend ────────────────────────────────────────
        // If a network interface is selected, select the vmnet mode or set up
        // the TAP bridge before the very first QEMU spawn so the initial
        // process already has the right -netdev and we don't have to restart
        // immediately.  prev_net_idx in the worker is seeded from this value
        // so the first poll iteration is a no-op for network setup.
        let mut prebuilt_net = runtime_worker::PrebuiltNet {
            tap_bridge: None,
            initial_net_idx: menu_state::NET_SEL_NONE,
        };
        let (initial_net_idx, iface_name) = {
            let s = menu_state::lock();
            let idx = s.selected_interface;
            let name = match idx {
                menu_state::NET_SEL_NONE | menu_state::NET_SEL_VMNET_HOST => None,
                n => s.net_ifaces.get(n as usize).map(|i| i.name.clone()),
            };
            (idx, name)
        };
        if initial_net_idx == menu_state::NET_SEL_VMNET_HOST {
            eprintln!("cdj3k-emu: vmnet host-only selected");
            config.net_vmnet = Some(VmnetMode::Host);
            prebuilt_net.initial_net_idx = initial_net_idx;
        }
        if let Some(name) = iface_name {
            if name.starts_with("tap") {
                match TapBridge::setup(&name, config.instance_id) {
                    Ok(tb) => {
                        config.net_tap_iface = Some(tb.qemu_tap.clone());
                        config.net_tap_fd = Some(tb.qemu_tap_fd);
                        prebuilt_net.tap_bridge = Some(tb);
                        prebuilt_net.initial_net_idx = initial_net_idx;
                    }
                    Err(e) => {
                        eprintln!("cdj3k-emu: initial tapbridge setup failed: {e}");
                        menu_state::lock().selected_interface = menu_state::NET_SEL_NONE;
                    }
                }
            } else if let Some(mode) = VmnetMode::bridged(&name) {
                eprintln!("cdj3k-emu: vmnet bridged on {name}");
                config.net_vmnet = Some(mode);
                prebuilt_net.initial_net_idx = initial_net_idx;
            } else {
                eprintln!("cdj3k-emu: invalid saved interface name: {name:?}");
                menu_state::lock().selected_interface = menu_state::NET_SEL_NONE;
            }
        }

        match QemuInstance::spawn(config.clone()) {
            Ok(inst) => {
                eprintln!("cdj3k-emu: QEMU subprocess started ({model})");
                runtime_worker::spawn(Some(inst), config, prebuilt_net);
            }
            Err(e) => {
                eprintln!("cdj3k-emu: QEMU start failed: {e:?}");
                runtime_worker::spawn(None, config, prebuilt_net);
            }
        }
    }
}
