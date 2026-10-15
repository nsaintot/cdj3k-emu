// Build a GUI app on Windows, so that no console window opens.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod cli;
mod launch;
mod runtime_worker;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use egui::{FontData, FontDefinitions, FontFamily, FontTweak};

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::fonts::{NIMBUS_SANS, NIMBUS_SANS_BOLD, NIMBUS_SANS_CONDENSED};
use cdj3k_emu_platform::{menu_state, window_socket};

use cli::Cli;

fn configure_helvetica_medium(ctx: &egui::Context) {
    const NIMBUS_SANS_DATA: &[u8] = include_bytes!("../assets/nimbus-sans-l.regular.otf");
    const NIMBUS_BOLD_DATA: &[u8] = include_bytes!("../assets/nimbus-sans-l.bold.otf");
    const NIMBUS_CONDENSED_DATA: &[u8] =
        include_bytes!("../assets/nimbus-sans-t.regular.condensed.otf");

    let mut fonts = FontDefinitions::default();

    let tweak = FontTweak {
        y_offset_factor: 0.25,
        ..Default::default()
    };
    let tweak_condensed = FontTweak {
        y_offset_factor: 0.15,
        ..Default::default()
    };

    fonts.font_data.insert(
        NIMBUS_SANS.to_owned(),
        std::sync::Arc::new(FontData::from_static(NIMBUS_SANS_DATA).tweak(tweak.clone())),
    );
    fonts.font_data.insert(
        NIMBUS_SANS_BOLD.to_owned(),
        std::sync::Arc::new(FontData::from_static(NIMBUS_BOLD_DATA).tweak(tweak)),
    );
    fonts.font_data.insert(
        NIMBUS_SANS_CONDENSED.to_owned(),
        std::sync::Arc::new(FontData::from_static(NIMBUS_CONDENSED_DATA).tweak(tweak_condensed)),
    );

    if let Some(family) = fonts.families.get_mut(&FontFamily::Proportional) {
        family.insert(0, NIMBUS_SANS.to_owned());
    }
    for name in [NIMBUS_SANS, NIMBUS_SANS_BOLD, NIMBUS_SANS_CONDENSED] {
        fonts
            .families
            .insert(FontFamily::Name(name.into()), vec![name.to_owned()]);
    }
    ctx.set_fonts(fonts);
}

/// Held until the process exits, so other windows see the slot as open. A
/// process without it never swaps an install into the slot.
static SLOT_CLAIM: OnceLock<cdj3k_emu_storage::SlotClaim> = OnceLock::new();

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // When the process was started as the QEMU worker, this runs QEMU and exits.
    cdj3k_emu_runtime::qemu_exec::run_worker_if_asked(&args);

    // When the process was started as the elevated network helper, this does
    // the helper's work and exits.
    cdj3k_emu_runtime::net::run_helper_if_asked(&args);

    // `cdj3k-emu ctl ...` drives a running slot's control API and exits.
    cdj3k_emu_control::cli::run_if_asked(&args);

    let mut cli = Cli::parse(&args);
    if cli.help {
        print!("{}", cli::USAGE);
        return;
    }

    cdj3k_emu_platform::desktop::announce_running();

    let instance = slot_to_run(&cli);
    if cli.headless {
        start_headless(cli.instance, cli.initial_model.is_some());
    }

    seed_menu_from_cli(instance, &cli);

    claim_slot(instance, cli.after_update, cli.provision_path.is_some());
    if cli.ephemeral {
        prepare_ephemeral(cli.instance, instance);
    }

    // Remove the keys that older releases kept in the app-wide settings file.
    cdj3k_emu_storage::prune_app_file();

    install_slot_menu_queries();

    let socket_dir = cdj3k_emu_platform::runtime_paths::instance_dir(instance)
        .to_string_lossy()
        .into_owned();

    if !cli.no_spawn {
        seed_menu_from_settings(instance, cli.service_mode);
    }

    if let Some(upd) = cli.provision_path.take() {
        provision_and_exit(upd, cli.key_path.as_deref(), instance, cli.initial_model);
    }

    cdj3k_emu_storage::adopt_unrecorded_slot(instance);

    // A slot remembers the model it last booted, and the app opens on that
    // model directly. The picker shows only for a fresh slot or from "Manage
    // Emulation". `--model` or `CDJ3K_MODEL` overrides the saved model for this
    // launch, and `--no-spawn` ignores it so that chassis work can always reach
    // the picker.
    if cli.initial_model.is_none() && !cli.no_spawn {
        cli.initial_model = cdj3k_emu_storage::InstanceSettings::saved_model(instance);
    }

    let host = launch::Host {
        instance,
        kernel: cli.kernel,
        initramfs: cli.initramfs,
        no_emmc: cli.no_emmc,
        serial_log: cli.serial_log,
        extra_mods: cli.extra_mods,
        no_mods: cli.no_mods,
        ui_only: cli.no_spawn,
        claim: SLOT_CLAIM.get(),
    };

    run_ui(instance, socket_dir, cli.profile, cli.initial_model, host);
    end_session(instance, cli.ephemeral);
}

/// Delete the ephemeral slot when its session ends.
fn end_session(instance: u32, ephemeral: bool) {
    if !ephemeral {
        return;
    }
    if let Err(e) = cdj3k_emu_storage::ephemeral::remove(instance) {
        eprintln!("cdj3k-emu: ephemeral slot {instance} was not deleted: {e}");
    }
}

// The E2E runner finds an ephemeral session at slot N plus this offset.
const _: () = assert!(cdj3k_emu_control::EPHEMERAL_SLOT_OFFSET == menu_state::MAX_INSTANCES);

/// The slot this launch runs: `--instance`, or its ephemeral slot with
/// `--ephemeral`. It first deletes the ephemeral slots that crashed sessions
/// left, and exits on a slot number outside 1 to MAX_INSTANCES.
fn slot_to_run(cli: &Cli) -> u32 {
    use cdj3k_emu_storage::ephemeral;
    for slot in ephemeral::remove_leftovers() {
        eprintln!("cdj3k-emu: deleted ephemeral slot {slot}, which a crashed session left.");
    }
    if !(1..=menu_state::MAX_INSTANCES).contains(&cli.instance) {
        eprintln!(
            "cdj3k-emu: --instance takes a slot from 1 to {}.",
            menu_state::MAX_INSTANCES
        );
        std::process::exit(2);
    }
    if cli.ephemeral {
        ephemeral::slot_for(cli.instance)
    } else {
        cli.instance
    }
}

/// Fill the ephemeral slot that this process has claimed from slot `base`, or
/// exit when that fails.
fn prepare_ephemeral(base: u32, instance: u32) {
    if let Err(e) = cdj3k_emu_storage::ephemeral::prepare(base, instance) {
        eprintln!("cdj3k-emu: slot {base} could not be copied to ephemeral slot {instance}: {e}");
        std::process::exit(1);
    }
}

/// Give the menu state this launch's slot and options.
fn seed_menu_from_cli(instance: u32, cli: &Cli) {
    let mut s = menu_state::lock();
    s.current_instance_id = instance;
    s.ui_only = cli.no_spawn;
    s.dev_mode = cli.dev;
    s.ephemeral = cli.ephemeral;
    s.ephemeral_reset_requested = cli.ephemeral;
    s.pc_link_supported = cdj3k_emu_runtime::pc_link::is_supported();
}

/// Run without a window, or exit when the slot has no firmware and no
/// `--model` was given. This runs before the slot is claimed, so a wrong slot
/// number leaves no claim behind.
fn start_headless(instance: u32, model_given: bool) {
    if !model_given && cdj3k_emu_storage::InstanceSettings::saved_model(instance).is_none() {
        eprintln!(
            "cdj3k-emu: --headless needs a slot with firmware, or --model; slot {instance} has none."
        );
        std::process::exit(2);
    }
    cdj3k_emu_platform::desktop::set_headless();
}

/// Take this process's claim on `instance`, waiting for it after an update.
/// Exits when another window owns the slot, unless `provisioning`.
fn claim_slot(instance: u32, after_update: bool, provisioning: bool) {
    let claim = if after_update {
        cdj3k_emu_storage::SlotClaim::take_within(instance, std::time::Duration::from_secs(60))
    } else {
        cdj3k_emu_storage::SlotClaim::take(instance)
    };
    match claim {
        Ok(Some(claim)) => {
            let _ = SLOT_CLAIM.set(claim);
        }
        // Another window owns the slot, so bring that window forward and exit.
        // `--provision` only stages an install and needs no claim.
        Ok(None) if !provisioning => {
            eprintln!("cdj3k-emu: slot {instance} is already open in another process");
            if let Some(pid) = cdj3k_emu_storage::slot_holder(instance) {
                if let Err(e) = cdj3k_emu_platform::desktop::activate_process(pid) {
                    eprintln!("cdj3k-emu: bringing slot {instance}'s window forward failed: {e}");
                }
            }
            std::process::exit(0);
        }
        Ok(None) => {}
        Err(e) => eprintln!("cdj3k-emu: claiming slot {instance} failed: {e}"),
    }
}

/// Give the Instances menu the busy state and the note it shows beside each
/// slot number. The menu's crate cannot depend on the settings crate, so it
/// reads them through these callbacks.
fn install_slot_menu_queries() {
    menu_state::set_slot_busy_fn(cdj3k_emu_storage::slot_in_use);
    menu_state::set_slot_note_fn(|n| {
        cdj3k_emu_storage::slot_summary(n).map(|(model, release)| match release {
            Some(rel) => format!("{} {rel}", model.title()),
            None => model.title().to_string(),
        })
    });
}

/// Copy the slot's settings into the menu once, so that the Audio, ALC and
/// other checkboxes show the correct initial state in the picker and the panel.
/// The runtime worker re-reads `audio_enabled` on every restart in
/// `apply_menu_to_config`, and pushes `alc_enabled` to the guest's cfg daemon
/// once per QEMU boot. Booting starts when a model is chosen, in `launch::Host`.
fn seed_menu_from_settings(instance: u32, service_mode: bool) {
    let inst_settings = cdj3k_emu_storage::InstanceSettings::load_or_init(instance);
    {
        let mut s = menu_state::lock();
        s.audio_enabled = inst_settings.audio_enabled;
        s.audio_device_uid = inst_settings.audio_device_uid.clone();
        s.alc_enabled = inst_settings.alc_enabled;
        s.haptic_enabled = inst_settings.haptic_enabled;
        s.mods_enabled = inst_settings.mods_enabled;
        s.dev_mode |= inst_settings.dev_mode;
        s.pc_link_enabled =
            inst_settings.pc_link_enabled && cdj3k_emu_runtime::pc_link::is_supported();
        s.service_mode = service_mode;
    }

    // Restore the network interface selection on a best-effort basis. If the
    // saved interface is no longer present, leave selected_interface at
    // "none". The saved value stays on disk and applies again when the
    // interface returns. The vmnet host-only mode is saved as a reserved token
    // that is not a valid BSD interface name, so it never matches a real
    // interface.
    menu_state::refresh_net_interfaces();
    if let Some(saved_name) = &inst_settings.net_iface {
        let mut s = menu_state::lock();
        if saved_name == menu_state::NET_IFACE_VMNET_HOST_TOKEN {
            s.selected_interface = menu_state::NET_SEL_VMNET_HOST;
        } else if let Some(idx) = s.net_ifaces.iter().position(|i| &i.name == saved_name) {
            s.selected_interface = idx as u32;
        }
    }

    // Restore the virtual USB image path if the file still exists. The
    // runtime worker mounts it through the deferred remount once QEMU is
    // ready. The saved path stays on disk either way, so an image on an
    // external drive that is not mounted now attaches again on a later launch.
    if let Some(path) = &inst_settings.usb_virtual_path {
        if path.exists() {
            menu_state::lock().usb_virtual_img = Some(path.clone());
        }
    }
}

/// `--provision`: install `upd` into the slot for `model`, then exit.
fn provision_and_exit(upd: PathBuf, key: Option<&Path>, instance: u32, model: Option<Model>) -> ! {
    let Some(model) = model else {
        eprintln!("cdj3k-emu: --provision needs --model, such as --model cdj3k.");
        std::process::exit(2);
    };
    if let Err(e) = cdj3k_emu_ui::provision_blocking(upd, key, instance, model) {
        eprintln!("cdj3k-emu: provisioning failed: {e}");
        std::process::exit(1);
    }
    // When the slot's window is open, that window swaps the install in.
    match SLOT_CLAIM.get().map(cdj3k_emu_storage::apply_staged) {
        Some(Ok(_)) => {
            eprintln!("cdj3k-emu: the {model} firmware is installed in slot {instance}.");
            std::process::exit(0);
        }
        Some(Err(e)) => {
            eprintln!("cdj3k-emu: swapping the installed firmware in failed: {e}");
            std::process::exit(1);
        }
        None => {
            let when = notify_install_ready(instance);
            eprintln!(
                "cdj3k-emu: the {model} firmware is installed for slot {instance}. \
                 The slot's open window swaps it in {when}."
            );
            std::process::exit(0);
        }
    }
}

/// Tell the window that owns the slot that an install is staged, and return
/// when that window swaps it in.
fn notify_install_ready(instance: u32) -> &'static str {
    match window_socket::send(instance, window_socket::Command::InstallReady) {
        Ok(Some(window_socket::Reply::Ok)) => {
            "now if its emulation is stopped, otherwise when the emulation stops"
        }
        _ => "when the emulation stops or next starts",
    }
}

fn run_ui(
    instance: u32,
    socket_dir: String,
    profile: bool,
    initial_model: Option<Model>,
    host: launch::Host,
) {
    let mut options = cdj3k_emu_platform::desktop::native_options(instance);
    options.viewport = std::mem::take(&mut options.viewport).with_icon(window_icon());

    // Closing the window or choosing Quit runs eframe's `on_exit`. These
    // hooks clean up when the process gets a signal or `main` returns.
    cdj3k_emu_runtime::process::install_exit_cleanup(
        cdj3k_emu_platform::runtime_paths::instance_dir(instance),
    );

    let app_name = format!(
        "{} - {}",
        cdj3k_emu_platform::app_meta::APP_DISPLAY_NAME,
        instance
    );
    cdj3k_emu_platform::desktop::set_app_name(&app_name);

    let mut host = Some(host);
    eframe::run_native(
        &app_name,
        options,
        Box::new(move |cc| {
            configure_helvetica_medium(&cc.egui_ctx);
            cdj3k_emu_platform::desktop::on_creation_context(cc);
            Ok(Box::new(cdj3k_emu_ui::CdjShell::new(
                cc,
                cdj3k_emu_ui::ShellConfig {
                    instance,
                    socket_dir: socket_dir.clone(),
                    profile,
                    initial_model,
                    host: Box::new(host.take().expect("app created once")),
                    window_socket: open_window_socket(instance, &cc.egui_ctx),
                },
            )))
        }),
    )
    .unwrap();
}

/// Windows (title bar, taskbar) and X11 take the window's icon. macOS reads
/// `CFBundleIconFile`; there an empty `IconData` keeps eframe from setting
/// its built-in icon (`AppTitleIconSetter::new` treats it as None).
fn window_icon() -> egui::IconData {
    if cdj3k_emu_platform::desktop::OWN_WINDOW_ICON {
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_256.png"))
            .unwrap_or_default()
    } else {
        egui::IconData::default()
    }
}

/// Only the process that holds the slot's claim listens on the slot's
/// window socket.
fn open_window_socket(instance: u32, ctx: &egui::Context) -> Option<window_socket::WindowSocket> {
    SLOT_CLAIM.get()?;
    let ctx = ctx.clone();
    window_socket::WindowSocket::bind(instance, move || ctx.request_repaint())
        .inspect_err(|e| {
            eprintln!("cdj3k-emu: slot {instance}: opening the window socket failed: {e}")
        })
        .ok()
}
