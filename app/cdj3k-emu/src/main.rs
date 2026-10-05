// No console window on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod launch;
mod runtime_worker;

use std::path::PathBuf;

use egui::{FontData, FontDefinitions, FontFamily, FontTweak};

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::fonts::{NIMBUS_SANS, NIMBUS_SANS_BOLD, NIMBUS_SANS_CONDENSED};

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
        FontData::from_static(NIMBUS_SANS_DATA).tweak(tweak),
    );
    fonts.font_data.insert(
        NIMBUS_SANS_BOLD.to_owned(),
        FontData::from_static(NIMBUS_BOLD_DATA).tweak(tweak),
    );
    fonts.font_data.insert(
        NIMBUS_SANS_CONDENSED.to_owned(),
        FontData::from_static(NIMBUS_CONDENSED_DATA).tweak(tweak_condensed),
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

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Started as the QEMU worker: runs QEMU and exits.
    cdj3k_emu_runtime::qemu_exec::run_worker_if_asked(&args);

    // Started as the elevated network helper: does its work and exits.
    cdj3k_emu_runtime::net::run_helper_if_asked(&args);

    cdj3k_emu_platform::desktop::announce_running();

    // Slot 1 is the default for a fresh launch. Slots 2..=4 are reachable
    // via `--instance N` (the "Instances" menu launches us with `open -n`).
    let mut instance: u32 = 1;

    // `--model <slug>` / `CDJ3K_MODEL`: skip the picker and boot this
    // model right away (headless captures, dev loops, launch scripts).
    let mut initial_model: Option<Model> = std::env::var("CDJ3K_MODEL")
        .ok()
        .and_then(|v| Model::parse(&v));

    let mut kernel: Option<PathBuf> = None;
    let mut initramfs: Option<PathBuf> = None;
    let mut no_emmc = false;
    // Default off: serial log grows unbounded and is only useful for debugging
    // boot/kernel panics. Enabled via `--serial-log` or `CDJ3K_SERIAL_LOG=1`.
    let mut serial_log = std::env::var_os("CDJ3K_SERIAL_LOG")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);
    // Default off: enables puffin scopes + binds a localhost TCP listener
    // for puffin_viewer.  Off in shipping builds; enable with `--profile`
    // or `CDJ3K_PROFILE=1` for performance investigations.
    let mut profile = std::env::var_os("CDJ3K_PROFILE")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);

    // `--service-mode` boots straight into the sub-CPU test mode (the
    // "Service Mode" menu item, which otherwise needs a restart).
    let mut service_mode = false;
    // `--provision <.UPD|.iso> [--key <file>]`: run the Install Firmware
    // pipeline for `--instance` / `--model` without a window, then exit.
    let mut provision_path: Option<PathBuf> = None;
    let mut key_path: Option<PathBuf> = None;
    // `--no-spawn` / `CDJ3K_NO_SPAWN=1`: chassis only - no runtime worker, no
    // QEMU, no boot shade. For layout work and `CDJ3K_SCREENSHOT` captures.
    let mut no_spawn = std::env::var_os("CDJ3K_NO_SPAWN")
        .map(|v| v != "0" && !v.is_empty())
        .unwrap_or(false);
    // `--after-update`: started by the updater while the build it replaced is
    // still exiting, so the slot is waited for rather than found taken.
    let mut after_update = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--instance" => {
                i += 1;
                if i < args.len() {
                    instance = args[i].parse().unwrap_or(0);
                }
            }
            "--model" => {
                i += 1;
                if i < args.len() {
                    match Model::parse(&args[i]) {
                        Some(m) => initial_model = Some(m),
                        None => eprintln!(
                            "cdj3k-emu: unknown --model {:?} ({}) - showing the picker",
                            args[i],
                            Model::ALL.map(|m| m.spec().slug).join(" | ")
                        ),
                    }
                }
            }
            "--kernel" => {
                i += 1;
                if i < args.len() {
                    kernel = Some(PathBuf::from(&args[i]));
                }
            }
            "--initramfs" => {
                i += 1;
                if i < args.len() {
                    initramfs = Some(PathBuf::from(&args[i]));
                }
            }
            "--no-emmc" => {
                no_emmc = true;
            }
            "--serial-log" => {
                serial_log = true;
            }
            "--profile" => {
                profile = true;
            }
            "--service-mode" => {
                service_mode = true;
            }
            "--provision" => {
                i += 1;
                if i < args.len() {
                    provision_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--key" => {
                i += 1;
                if i < args.len() {
                    key_path = Some(PathBuf::from(&args[i]));
                }
            }
            "--no-spawn" => {
                no_spawn = true;
            }
            "--after-update" => {
                after_update = true;
            }
            _ => {}
        }
        i += 1;
    }

    {
        let mut s = cdj3k_emu_platform::menu_state::lock();
        s.current_instance_id = instance;
        s.ui_only = no_spawn;
        s.pc_link_supported = cdj3k_emu_runtime::pc_link::is_supported();
    }

    // Held until the process exits, so other windows see the slot as open. A
    // process without it never swaps an install into the slot.
    static SLOT_CLAIM: std::sync::OnceLock<cdj3k_emu_storage::SlotClaim> =
        std::sync::OnceLock::new();
    let claim = if after_update {
        cdj3k_emu_storage::SlotClaim::take_within(instance, std::time::Duration::from_secs(60))
    } else {
        cdj3k_emu_storage::SlotClaim::take(instance)
    };
    match claim {
        Ok(Some(claim)) => {
            let _ = SLOT_CLAIM.set(claim);
        }
        // Another window owns the slot: bring it forward rather than run a
        // second emulation over it. `--provision` stages and needs no claim.
        Ok(None) if provision_path.is_none() => {
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

    // Keys older releases kept in the app-wide settings file.
    cdj3k_emu_storage::prune_app_file();

    // What the Instances menu writes beside a slot number. The menu lives a
    // crate below the settings, so it asks through this.
    cdj3k_emu_platform::menu_state::set_slot_busy_fn(cdj3k_emu_storage::slot_in_use);
    cdj3k_emu_platform::menu_state::set_slot_note_fn(|n| {
        cdj3k_emu_storage::slot_summary(n).map(|(model, release)| match release {
            Some(rel) => format!("{} {rel}", model.title()),
            None => model.title().to_string(),
        })
    });

    let socket_dir = cdj3k_emu_platform::runtime_paths::instance_dir(instance)
        .to_string_lossy()
        .into_owned();

    // ── Slot settings → menu mirrors ──────────────────────────────────────
    // Seeded once here so the Audio/ALC/... checkboxes show the correct
    // initial state on the picker and the panel alike. The runtime worker
    // re-reads `audio_enabled` on every restart via apply_menu_to_config and
    // pushes `alc_enabled` to the guest cfg daemon once per QEMU boot (see
    // runtime_worker::run). Booting itself happens when a model is chosen
    // (`launch::Host`).
    if !no_spawn {
        let inst_settings = cdj3k_emu_storage::InstanceSettings::load_or_init(instance);
        {
            let mut s = cdj3k_emu_platform::menu_state::lock();
            s.audio_enabled = inst_settings.audio_enabled;
            s.audio_device_uid = inst_settings.audio_device_uid.clone();
            s.alc_enabled = inst_settings.alc_enabled;
            s.haptic_enabled = inst_settings.haptic_enabled;
            s.pc_link_enabled =
                inst_settings.pc_link_enabled && cdj3k_emu_runtime::pc_link::is_supported();
            s.service_mode = service_mode;
        }

        // Restore network interface selection (best effort).  If the saved
        // ifname is no longer present, leave selected_interface at "none" -
        // the saved value stays on disk and binds again when the iface
        // returns. The vmnet-host mode round-trips via a reserved token
        // (not a valid BSD ifname) so it can never collide with a real iface.
        cdj3k_emu_platform::menu_state::refresh_net_interfaces();
        if let Some(saved_name) = &inst_settings.net_iface {
            let mut s = cdj3k_emu_platform::menu_state::lock();
            if saved_name == cdj3k_emu_platform::menu_state::NET_IFACE_VMNET_HOST_TOKEN {
                s.selected_interface = cdj3k_emu_platform::menu_state::NET_SEL_VMNET_HOST;
            } else if let Some(idx) = s.net_ifaces.iter().position(|i| &i.name == saved_name) {
                s.selected_interface = idx as u32;
            }
        }

        // Restore virtual USB image path (best effort).  Only seed if the file
        // still exists; the runtime worker picks it up via the deferred
        // remount path once QEMU is ready.  Saved path stays on disk
        // regardless, so reattaching a missing image (e.g. external drive
        // remounted) works on a future launch.
        if let Some(path) = &inst_settings.usb_virtual_path {
            if path.exists() {
                cdj3k_emu_platform::menu_state::lock().usb_virtual_img = Some(path.clone());
            }
        }
    }

    // A slot remembers the model it last booted: open on it directly and
    // show the picker only for a fresh slot or from "Manage Emulation".
    // `--model` / `CDJ3K_MODEL` override it for this launch; `--no-spawn`
    // ignores it so chassis work can always reach the picker.
    if let Some(upd) = provision_path {
        let Some(model) = initial_model else {
            eprintln!("cdj3k-emu: --provision needs --model (cdj3k | cdj3kx)");
            std::process::exit(2);
        };
        match cdj3k_emu_ui::provision_blocking(upd, key_path.as_deref(), instance, model) {
            // The slot's window, if one is open, swaps it in itself.
            Ok(()) => match SLOT_CLAIM.get().map(cdj3k_emu_storage::apply_staged) {
                Some(Ok(_)) => {
                    eprintln!("cdj3k-emu: {model} firmware installed in slot {instance}");
                    std::process::exit(0);
                }
                Some(Err(e)) => {
                    eprintln!("cdj3k-emu: swapping the installed firmware in failed: {e}");
                    std::process::exit(1);
                }
                None => {
                    eprintln!(
                        "cdj3k-emu: {model} firmware installed for slot {instance}; its open \
                         window swaps it in on the emulation's next start"
                    );
                    std::process::exit(0);
                }
            },
            Err(e) => {
                eprintln!("cdj3k-emu: provisioning failed: {e}");
                std::process::exit(1);
            }
        }
    }

    cdj3k_emu_storage::adopt_unrecorded_slot(instance);

    if initial_model.is_none() && !no_spawn {
        initial_model = cdj3k_emu_storage::InstanceSettings::saved_model(instance);
    }

    let host = launch::Host {
        instance,
        kernel,
        initramfs,
        no_emmc,
        serial_log,
        ui_only: no_spawn,
        claim: SLOT_CLAIM.get(),
    };

    // ── UI ────────────────────────────────────────────────────────────────
    let mut options = cdj3k_emu_platform::desktop::native_options(instance);

    // Windows (title bar, taskbar) and X11 take the window's icon. macOS reads
    // `CFBundleIconFile`; there an empty `IconData` keeps eframe from setting
    // its built-in icon (`AppTitleIconSetter::new` treats it as None).
    let icon = if cdj3k_emu_platform::desktop::OWN_WINDOW_ICON {
        eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon_256.png"))
            .unwrap_or_default()
    } else {
        egui::IconData::default()
    };
    options.viewport = std::mem::take(&mut options.viewport).with_icon(icon);

    // Window close and Quit run eframe's `on_exit`; a signal or a plain
    // return run these.
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
                },
            )))
        }),
    )
    .unwrap();
}
