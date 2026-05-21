mod runtime_worker;

use std::path::PathBuf;

use egui::{FontData, FontDefinitions, FontFamily, FontTweak};

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
        FontData::from_static(NIMBUS_SANS_DATA).tweak(tweak).into(),
    );
    fonts.font_data.insert(
        NIMBUS_SANS_BOLD.to_owned(),
        FontData::from_static(NIMBUS_BOLD_DATA).tweak(tweak).into(),
    );
    fonts.font_data.insert(
        NIMBUS_SANS_CONDENSED.to_owned(),
        FontData::from_static(NIMBUS_CONDENSED_DATA)
            .tweak(tweak_condensed)
            .into(),
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

    // ── Worker mode ───────────────────────────────────────────────────────
    // When spawned as a QEMU subprocess, run QEMU directly and exit.
    // Each Start spawns a fresh process so QEMU global state is always clean.
    #[cfg(target_os = "macos")]
    if args.get(1).map(|s| s == "--qemu-worker").unwrap_or(false) {
        use std::ffi::CString;
        extern "C" {
            fn cdj3k_emu_qemu_run(
                argc: libc::c_int,
                argv: *const *const libc::c_char,
            ) -> libc::c_int;
            fn cdj3k_emu_qemu_abort();
        }

        // Capture parent PID before it can be reparented (launchd takes over
        // after the parent dies, changing getppid() to 1).  The watchdog thread
        // polls the original PID and force-quits if the parent is gone.
        let parent_pid = unsafe { libc::getppid() };
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if unsafe { libc::kill(parent_pid, 0) } != 0 {
                eprintln!("cdj3k-emu-worker: parent gone, aborting");
                unsafe {
                    cdj3k_emu_qemu_abort();
                    libc::_exit(0)
                };
            }
        });

        let qemu_args: Vec<String> = args.into_iter().skip(2).collect();
        eprintln!("cdj3k-emu-worker: {}", qemu_args.join(" "));
        let c_strings: Vec<CString> = qemu_args
            .iter()
            .map(|s| CString::new(s.as_str()).expect("argv NUL"))
            .collect();
        let c_ptrs: Vec<*const libc::c_char> = c_strings.iter().map(|cs| cs.as_ptr()).collect();
        let code = unsafe { cdj3k_emu_qemu_run(c_ptrs.len() as libc::c_int, c_ptrs.as_ptr()) };
        // cdj3k_emu_qemu_run returned via longjmp from exit() shim; orphaned QEMU
        // threads remain in this subprocess - _exit kills them all atomically.
        unsafe { libc::_exit(code) };
    }

    // Slot 1 is the default for a fresh launch. Slots 2..=4 are reachable
    // via `--instance N` (the "Instances" menu launches us with `open -n`).
    let mut instance: u32 = 1;

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

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--instance" => {
                i += 1;
                if i < args.len() {
                    instance = args[i].parse().unwrap_or(0);
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
            _ => {}
        }
        i += 1;
    }

    cdj3k_emu_platform::menu_state::lock().current_instance_id = instance;

    let socket_dir = cdj3k_emu_platform::runtime_paths::instance_dir(instance)
        .to_string_lossy()
        .into_owned();

    // ── QEMU lifecycle ────────────────────────────────────────────────────
    // Two modes:
    //   --kernel/--initramfs  dev: explicit path override (boot immediately)
    //   .app default          resolve from App Support; boot only when all
    //                         three files exist (wizard provisions them first)
    #[cfg(target_os = "macos")]
    {
        use cdj3k_emu_runtime::{QemuConfig, QemuInstance, SocketVmnet, TapBridge};

        let instance_dir = cdj3k_emu_storage::emmc::default_path(instance)
            .parent()
            .unwrap()
            .to_path_buf();
        let resolved_kernel = kernel.unwrap_or_else(|| instance_dir.join("Image"));
        let resolved_initramfs =
            initramfs.unwrap_or_else(|| instance_dir.join("initramfs-patched.cpio.gz"));
        let emmc_path = cdj3k_emu_storage::emmc::default_path(instance);

        let emmc_img = if no_emmc {
            None
        } else {
            Some(emmc_path.clone())
        };

        let inst_settings = cdj3k_emu_storage::InstanceSettings::load_or_init(instance);
        // Seed the menu mirrors so the Audio/ALC checkboxes show the correct
        // initial state.  The runtime worker re-reads `audio_enabled` on every
        // restart via apply_menu_to_config and pushes `alc_enabled` to the
        // guest cfg daemon once per QEMU boot (see runtime_worker::run).
        {
            let mut s = cdj3k_emu_platform::menu_state::lock();
            s.audio_enabled = inst_settings.audio_enabled;
            s.audio_device_uid = inst_settings.audio_device_uid.clone();
            s.alc_enabled = inst_settings.alc_enabled;
            s.haptic_enabled = inst_settings.haptic_enabled;
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

        let mut config = QemuConfig::new(resolved_kernel.clone(), resolved_initramfs.clone());
        config.instance_id = instance;
        config.ssh_port = 2222 + instance as u16;
        config.qmp_port = 4445 + instance as u16;
        config.gdb_port = 1235 + instance as u16;
        config.emmc_img = emmc_img;
        config.audio = inst_settings.audio_enabled;
        config.audio_device_uid = inst_settings.audio_device_uid.clone();
        config.mac = Some(inst_settings.mac);
        config.serial_log = serial_log;

        // ── Pre-build network backend ────────────────────────────────────────
        // If a saved network interface is available, set up the TAP bridge or
        // socket_vmnet daemon before the very first QEMU spawn so the initial
        // process already has the right -netdev and we don't have to restart
        // immediately.  prev_net_idx in the worker is seeded from this value
        // so the first poll iteration is a no-op for network setup.
        let mut prebuilt_net = runtime_worker::PrebuiltNet {
            tap_bridge: None,
            vmnet: None,
            initial_net_idx: cdj3k_emu_platform::menu_state::NET_SEL_NONE,
        };
        let (initial_net_idx, iface_name) = {
            let s = cdj3k_emu_platform::menu_state::lock();
            let idx = s.selected_interface;
            let name = match idx {
                cdj3k_emu_platform::menu_state::NET_SEL_NONE
                | cdj3k_emu_platform::menu_state::NET_SEL_VMNET_HOST => None,
                n => s.net_ifaces.get(n as usize).map(|i| i.name.clone()),
            };
            (idx, name)
        };
        if initial_net_idx == cdj3k_emu_platform::menu_state::NET_SEL_VMNET_HOST {
            match SocketVmnet::start_host() {
                Ok(sv) => {
                    eprintln!(
                        "cdj3k-emu: vmnet host-only up  socket={}",
                        sv.socket_path().display()
                    );
                    config.net_socket_vmnet = Some(sv.socket_path().to_path_buf());
                    prebuilt_net.vmnet = Some(sv);
                    prebuilt_net.initial_net_idx = initial_net_idx;
                }
                Err(e) => {
                    eprintln!("cdj3k-emu: initial vmnet host-only start failed: {e}");
                    cdj3k_emu_platform::menu_state::lock().selected_interface =
                        cdj3k_emu_platform::menu_state::NET_SEL_NONE;
                }
            }
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
                        cdj3k_emu_platform::menu_state::lock().selected_interface =
                            cdj3k_emu_platform::menu_state::NET_SEL_NONE;
                    }
                }
            } else {
                match SocketVmnet::start_bridged(&name) {
                    Ok(sv) => {
                        eprintln!(
                            "cdj3k-emu: vmnet up on {}  socket={}",
                            name,
                            sv.socket_path().display()
                        );
                        config.net_socket_vmnet = Some(sv.socket_path().to_path_buf());
                        prebuilt_net.vmnet = Some(sv);
                        prebuilt_net.initial_net_idx = initial_net_idx;
                    }
                    Err(e) => {
                        eprintln!("cdj3k-emu: initial vmnet start failed: {e}");
                        cdj3k_emu_platform::menu_state::lock().selected_interface =
                            cdj3k_emu_platform::menu_state::NET_SEL_NONE;
                    }
                }
            }
        }

        let can_boot = resolved_kernel.exists()
            && resolved_initramfs.exists()
            && (no_emmc || emmc_path.exists());

        if can_boot {
            match QemuInstance::spawn(config.clone()) {
                Ok(inst) => {
                    eprintln!("cdj3k-emu: QEMU subprocess started");
                    runtime_worker::spawn(Some(inst), config, prebuilt_net);
                }
                Err(e) => {
                    eprintln!("cdj3k-emu: QEMU start failed: {e:?}");
                    runtime_worker::spawn(None, config, prebuilt_net);
                }
            }
        } else {
            eprintln!("cdj3k-emu: firmware not provisioned - opening Install Firmware wizard");
            cdj3k_emu_platform::menu_state::lock().firmware_wizard_requested = true;
            runtime_worker::spawn(None, config, prebuilt_net);
        }
    }

    // ── UI ────────────────────────────────────────────────────────────────
    let mut options = cdj3k_emu_platform::desktop::native_options(instance);

    // Opt out of eframe's built-in default app icon (a white "e" on black,
    // baked into eframe via `data/icon.png` and substituted whenever the
    // viewport's `icon` field is None).  Passing an empty `IconData::default()`
    // is the documented escape hatch: `AppTitleIconSetter::new` recognises
    // it as equivalent to None and skips `NSApplication.setApplicationIconImage:`
    // on macOS entirely, which lets the Dock read `CFBundleIconFile`
    // (`cdj3k-emu.icns` in `Contents/Resources/`) from Info.plist.
    options.viewport = std::mem::take(&mut options.viewport).with_icon(egui::IconData::default());

    // Register the sock dir for shutdown cleanup. Three exit paths:
    //   - window-X / Cmd-Q  → eframe::App::on_exit (handled in CdjApp)
    //   - Ctrl-C, SIGTERM   → signal handler below (calls cleanup inline,
    //                         then _exit; doesn't rely on atexit, which on
    //                         macOS is unreliable across NSApplication quit)
    //   - normal return     → atexit (belt-and-suspenders)
    #[cfg(target_os = "macos")]
    {
        let _ = cdj3k_emu_runtime::SHUTDOWN_SOCK_DIR
            .set(cdj3k_emu_platform::runtime_paths::instance_dir(instance));
        extern "C" fn cleanup_at_exit() {
            cdj3k_emu_runtime::cleanup_runtime_files();
        }
        extern "C" fn on_signal(_sig: libc::c_int) {
            cdj3k_emu_platform::menu_state::APP_SHUTDOWN
                .store(true, std::sync::atomic::Ordering::Relaxed);
            cdj3k_emu_runtime::kill_qemu_child_now();
            cdj3k_emu_runtime::cleanup_runtime_files();
            unsafe { libc::_exit(0) };
        }
        unsafe {
            libc::atexit(cleanup_at_exit);
            for sig in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
                libc::signal(sig, on_signal as *const () as libc::sighandler_t);
            }
        }
    }

    let app_name = format!(
        "{} - {}",
        cdj3k_emu_platform::app_meta::APP_DISPLAY_NAME,
        instance
    );
    cdj3k_emu_platform::desktop::set_app_name(&app_name);

    eframe::run_native(
        &app_name,
        options,
        Box::new(move |cc| {
            configure_helvetica_medium(&cc.egui_ctx);
            cdj3k_emu_platform::desktop::on_creation_context(cc, instance);
            Ok(Box::new(cdj3k_emu_ui::app::CdjApp::new(
                socket_dir.clone(),
                cc.egui_ctx.clone(),
                profile,
            )))
        }),
    )
    .unwrap();
}
