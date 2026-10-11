//! Starts the runtime for one model. It resolves the model's firmware, builds
//! the QEMU config from the slot's settings, prepares the network backend,
//! starts QEMU and hands everything to the runtime worker. It implements the
//! shell's [`RuntimeHost`], so the picker can boot the chosen model.

use std::path::PathBuf;

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::menu_state;
use cdj3k_emu_runtime::{QemuConfig, QemuInstance};
use cdj3k_emu_storage::mods::BootMods;
use cdj3k_emu_storage::FirmwarePaths;
use cdj3k_emu_ui::{LaunchOutcome, RuntimeHost};

use crate::runtime_worker::{self, PrebuiltNet};

/// Boot options that the command line fixes for the process. Every launch uses them.
pub struct Host {
    pub instance: u32,
    /// The kernel given with `--kernel`. When it is unset, the model's
    /// provisioned kernel is used.
    pub kernel: Option<PathBuf>,
    /// The initramfs given with `--initramfs`. When it is unset, the
    /// provisioned initramfs is used.
    pub initramfs: Option<PathBuf>,
    pub no_emmc: bool,
    pub serial_log: bool,
    /// Folders given with `--mod`, booted in addition to the slot's mods.
    pub extra_mods: Vec<PathBuf>,
    /// `--no-mods`: boot without the slot's mods.
    pub no_mods: bool,
    /// `--no-spawn`: never boot anything.
    pub ui_only: bool,
    /// This process's claim on the slot. It is `None` when another process
    /// owns the slot, and then this process may not swap a staged install in.
    pub claim: Option<&'static cdj3k_emu_storage::SlotClaim>,
}

/// Record `model` as the slot's model so the next launch of this slot opens
/// on it directly. A failed write is only logged.
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

    fn apply_staged(&mut self) -> std::io::Result<Option<cdj3k_emu_storage::StagedRecord>> {
        match self.claim {
            Some(claim) => cdj3k_emu_storage::apply_staged(claim),
            None => Ok(None),
        }
    }

    fn launch(&mut self, model: Model) -> LaunchOutcome {
        if self.ui_only {
            return LaunchOutcome::UiOnly;
        }
        if self.installed_model() != Some(model) {
            return LaunchOutcome::NotProvisioned;
        }
        // Record the model before the worker starts, because the worker reads
        // the slot settings as soon as it starts.
        persist_model(self.instance, model);
        self.spawn(model);
        LaunchOutcome::Started
    }
}

impl Host {
    /// Build the QEMU config, prepare the network backend, and start QEMU and
    /// the runtime worker. No worker may be running.
    fn spawn(&self, model: Model) {
        let r = self.resolve();
        let inst_settings = cdj3k_emu_storage::InstanceSettings::load_or_init(self.instance);

        let mods = ModsLaunch {
            base: r.initramfs.clone(),
            extra: self.extra_mods.clone(),
            no_mods: self.no_mods,
        };
        let boot = mods.boot(self.instance, model);
        let mut config = self.qemu_config(model, &r, &inst_settings, &boot);
        let prebuilt_net = prebuild_net(&mut config, &inst_settings.mac, self.instance);

        match QemuInstance::spawn(config.clone()) {
            Ok(inst) => {
                eprintln!("cdj3k-emu: the QEMU subprocess started for the {model}.");
                cdj3k_emu_storage::mods::set_current_boot(boot);
                runtime_worker::spawn(Some(inst), config, prebuilt_net, mods);
            }
            Err(e) => {
                eprintln!("cdj3k-emu: starting QEMU failed: {e:?}");
                runtime_worker::spawn(None, config, prebuilt_net, mods);
            }
        }
    }

    fn qemu_config(
        &self,
        model: Model,
        r: &Resolved,
        inst_settings: &cdj3k_emu_storage::InstanceSettings,
        boot: &BootMods,
    ) -> QemuConfig {
        let instance = self.instance;
        let mut config = QemuConfig::new(r.kernel.clone(), boot.initramfs.clone());
        config.instance_id = instance;
        config.model = model;
        config.ssh_port = 2222 + instance as u16;
        config.qmp_port = 4445 + instance as u16;
        config.gdb_port = 1235 + instance as u16;
        config.emmc_img = (!self.no_emmc).then(|| r.emmc.clone());
        config.audio = inst_settings.audio_enabled;
        config.audio_device_uid = inst_settings.audio_device_uid.clone();
        config.service_mode = menu_state::lock().service_mode;
        config.mac = Some(inst_settings.mac.clone());
        config.soc_serial = Some(inst_settings.soc_serial.clone());
        config.serial_log = self.serial_log;
        config
    }
}

/// If a network interface is selected, select the vmnet mode or set up the
/// TAP bridge before the very first QEMU spawn, so the initial process already
/// has the right `-netdev` and needs no immediate restart. The worker seeds
/// `prev_net_idx` from the result, so its first poll iteration leaves the
/// network alone.
fn prebuild_net(config: &mut QemuConfig, mac: &str, instance: u32) -> PrebuiltNet {
    let mut prebuilt_net = PrebuiltNet {
        keep: cdj3k_emu_runtime::NetKeepAlive::Nothing,
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
    let attempt = if initial_net_idx == menu_state::NET_SEL_VMNET_HOST {
        Some((
            "host-only".to_string(),
            cdj3k_emu_runtime::net_attach_host_only(),
        ))
    } else {
        iface_name.map(|name| {
            let r = cdj3k_emu_runtime::net_attach(&name, mac, instance);
            (name, r)
        })
    };
    if let Some((name, result)) = attempt {
        match result {
            Ok(a) => {
                prebuilt_net.keep = runtime_worker::adopt_attachment(config, &name, a);
                prebuilt_net.initial_net_idx = initial_net_idx;
            }
            Err(e) => {
                eprintln!("cdj3k-emu: cannot bridge on {name}: {e}");
                menu_state::lock().selected_interface = menu_state::NET_SEL_NONE;
            }
        }
    }
    prebuilt_net
}

/// The inputs for adding mods to a boot. The runtime worker rebuilds the
/// initramfs before every QEMU start, so a change to the slot's mods takes
/// effect on the next restart.
#[derive(Clone, Debug)]
pub struct ModsLaunch {
    /// The slot's initramfs, or the one given with `--initramfs`.
    pub base: PathBuf,
    /// Folders given with `--mod`, booted in addition to the slot's mods.
    pub extra: Vec<PathBuf>,
    /// `--no-mods`: boot without the slot's mods.
    pub no_mods: bool,
}

impl ModsLaunch {
    /// The next boot's mods and initramfs: the base with the mods appended,
    /// or the base alone when no mod runs or the mods cannot be prepared.
    /// The caller records it with `set_current_boot` once QEMU has started.
    pub fn boot(&self, instance: u32, model: Model) -> BootMods {
        // Record the request count before reading the mods, so that a mods
        // restart requested during the read causes another restart.
        {
            let mut s = menu_state::lock();
            s.mods_restart_seq_booted = s.mods_restart_seq;
        }
        let settings = cdj3k_emu_storage::InstanceSettings::load_or_init(instance);
        // An outdated slot has no mods runner in its guest, so it boots
        // without mods.
        let outdated = cdj3k_emu_storage::slot_outdated(instance);
        let boot = cdj3k_emu_storage::mods::prepare_boot(
            instance,
            &self.base,
            model,
            settings.firmware_release.as_deref(),
            cdj3k_emu_storage::mods::slot_mods_on(instance) && !self.no_mods,
            if outdated { &[] } else { &self.extra },
        );
        match boot {
            Ok(mut boot) => {
                boot.no_mods = self.no_mods;
                if !boot.run.is_empty() {
                    eprintln!("cdj3k-emu: booting with mods: {}", boot.run.join(", "));
                }
                for (name, compat) in &boot.incompatible {
                    if !matches!(compat, cdj3k_emu_storage::mods::Compat::Invalid(_)) {
                        eprintln!(
                            "cdj3k-emu: mod {name} is incompatible with this slot and was skipped"
                        );
                    }
                }
                boot
            }
            Err(e) => {
                eprintln!(
                    "cdj3k-emu: preparing the mods failed, so the deck boots without them: {e}"
                );
                BootMods {
                    initramfs: self.base.clone(),
                    run: Vec::new(),
                    slot_run: Vec::new(),
                    incompatible: Vec::new(),
                    no_mods: self.no_mods,
                }
            }
        }
    }
}
