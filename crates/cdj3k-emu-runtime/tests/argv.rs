//! The QEMU command line [`QemuConfig::build_argv`] produces.

use std::path::PathBuf;

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::host::{self, Accelerator};
use cdj3k_emu_runtime::{QemuConfig, VmnetMode};

fn config() -> QemuConfig {
    let mut c = QemuConfig::new(PathBuf::from("/k/Image"), PathBuf::from("/k/initramfs"));
    c.audio = true;
    c.shm = true;
    c
}

/// Every host gets the same guest topology: six cores, the same RAM, GICv3.
#[test]
fn the_guest_topology_is_identical_on_every_host() {
    let a = config().build_argv().join(" ");
    assert!(a.contains("-smp 6"), "{a}");
    assert!(a.contains(&format!("{}B", QemuConfig::MEM_BYTES)), "{a}");
    assert!(a.contains("virt,gic-version=3"), "{a}");
}

/// The kernel command line carries each model's eMMC index: 0 for the
/// CDJ-1500X, 1 for the RK3399 decks.
#[test]
fn the_emmc_is_named_after_the_decks_own() {
    for (model, n) in [(Model::Cdj3k, 1), (Model::Cdj3kx, 1), (Model::Cdj1500x, 0)] {
        let mut c = config();
        c.model = model;
        let a = c.build_argv().join(" ");
        assert!(
            a.contains(&format!(" virtio_blk.emmc_index={n}")),
            "{model}: {a}"
        );
    }
}

const KVM: Accelerator = Accelerator {
    name: "kvm",
    in_kernel_gic: true,
};

#[test]
fn an_accelerated_guest_runs_the_hosts_cpu() {
    let mut c = config();
    c.accel = Some(KVM);
    let joined = c.build_argv().join(" ");
    assert!(joined.contains("-accel kvm"), "{joined}");
    assert!(joined.contains("-cpu host"), "{joined}");
    assert!(joined.contains("kernel-irqchip=on"), "{joined}");
}

#[test]
fn software_emulation_runs_the_decks_own_core() {
    let mut c = config();
    c.accel = None;
    let joined = c.build_argv().join(" ");
    assert!(!joined.contains("-accel"), "{joined}");
    assert!(joined.contains("-cpu cortex-a72"), "{joined}");
    assert!(joined.contains("kernel-irqchip=off"), "{joined}");
}

/// The larger ALSA period is for TCG alone; an accelerated guest keeps
/// the player's own.
#[test]
fn only_tcg_widens_the_audio_period() {
    let mut c = config();
    c.accel = None;
    assert!(c.build_argv().join(" ").contains(" cdj3k.snd_period=512"));
    c.accel = Some(KVM);
    assert!(!c.build_argv().join(" ").contains("cdj3k.snd_period"));
}

#[test]
fn the_audio_backend_is_the_hosts_own() {
    let joined = config().build_argv().join(" ");
    let Some(backend) = host::AUDIO else {
        // A host with no backend gets no audio device at all.
        assert!(!joined.contains("-audiodev"), "{joined}");
        assert!(!joined.contains("virtio-sound-device"), "{joined}");
        return;
    };
    assert!(
        joined.contains(&format!("-audiodev {}", backend.driver)),
        "{joined}"
    );
    assert!(
        joined.contains("virtio-sound-device,audiodev=audio0"),
        "{joined}"
    );
}

#[test]
fn a_pinned_output_device_uses_the_backends_own_selector() {
    let Some(backend) = host::AUDIO else {
        return;
    };
    let mut c = config();
    c.audio_device_uid = Some("Some Output".into());
    let joined = c.build_argv().join(" ");
    assert!(
        joined.contains(&format!("{}Some Output", backend.device_selector)),
        "{joined}"
    );
}

#[test]
fn the_ram_file_is_handed_to_qemu_as_the_machine_memory() {
    let c = config();
    let joined = c.build_argv().join(" ");
    assert!(joined.contains("memory-backend-file,id=ram0"), "{joined}");
    assert!(
        joined.contains(&c.shm_path().display().to_string()),
        "{joined}"
    );
    assert!(joined.contains("memory-backend=ram0"), "{joined}");
}

/// `argv` with `{sock}` replaced by the instance's socket directory and
/// `{sep}` by the host's path separator. `{sep}` marks the paths that the
/// config builds with `Path::join`, such as `ctrl.sock`, which use `\` on
/// Windows.
fn expand(c: &QemuConfig, argv: &[&str]) -> Vec<String> {
    let sock = c.sock_dir().display().to_string();
    argv.iter()
        .map(|a| {
            a.replace("{sock}", &sock)
                .replace("{sep}", std::path::MAIN_SEPARATOR_STR)
        })
        .collect()
}

/// The display, the monitors and the entropy source.
fn display_qmp_and_rng(c: &QemuConfig) -> Vec<String> {
    let mut v = expand(c, &["-display", "shm,path={sock}/main.shm", "-qmp"]);
    v.push(format!("tcp:localhost:{},server=on,wait=off", c.qmp_port));
    if cdj3k_emu_runtime::disk::PASSES_FDS {
        v.extend(expand(
            c,
            &["-qmp", "unix:{sock}{sep}qmp-fd.sock,server=on,wait=off"],
        ));
    }
    v.extend(expand(
        c,
        &[
            "-object",
            host::RNG_OBJECT,
            "-device",
            "virtio-rng-device,rng=rng0",
        ],
    ));
    v
}

/// The guest's channels, the jog buffer and the USB slot.
fn channels_jog_and_usb(c: &QemuConfig) -> Vec<String> {
    expand(
        c,
        &[
            "-device",
            "virtio-serial-device,max_ports=8",
            "-chardev",
            "socket,id=vserial_ctrl,path={sock}{sep}ctrl.sock,server=on,wait=off",
            "-device",
            "virtserialport,chardev=vserial_ctrl,name=cdj3k.ctrl",
            "-chardev",
            "socket,id=vserial_cfg,path={sock}{sep}cfg.sock,server=on,wait=off",
            "-device",
            "virtserialport,chardev=vserial_cfg,name=cdj3k.cfg",
            "-chardev",
            "socket,id=vserial_usb-link,path={sock}{sep}usb-link.sock,server=on,wait=off",
            "-device",
            "virtserialport,chardev=vserial_usb-link,name=cdj3k.usb-link",
            "-object",
            "memory-backend-file,id=jogshm,mem-path={sock}{sep}jog.shm,size=1M,share=on",
            "-device",
            "ivshmem-plain,memdev=jogshm,master=on",
            "-drive",
            "file={sock}{sep}usb.placeholder,if=none,id=usb0,format=raw,cache=writeback,\
             file.locking=off",
            "-device",
            "virtio-blk-device,drive=usb0,id=usb0",
        ],
    )
}

/// The `-audiodev` and sound device for this host's backend, if it has
/// one.
fn audio(selected: &str) -> Vec<String> {
    let Some(backend) = host::AUDIO else {
        return Vec::new();
    };
    let device = if selected.is_empty() {
        String::new()
    } else {
        format!("{}{selected}", backend.device_selector)
    };
    vec![
        "-audiodev".into(),
        format!(
            "{},id=audio0,in.voices=0{}{device}",
            backend.driver, backend.options
        ),
        "-device".into(),
        "virtio-sound-device,audiodev=audio0,streams=1".into(),
    ]
}

/// A default software-emulated deck: the whole argv, in order.
#[test]
fn a_minimal_config_builds_this_exact_argv() {
    let mut c = QemuConfig::new(PathBuf::from("/k/Image"), PathBuf::from("/k/initramfs"));
    c.accel = None;
    let mut want = expand(
        &c,
        &[
            "cdj3k-emu-qemu",
            "-machine",
            "virt,gic-version=3,kernel-irqchip=off",
            "-m",
            "3221225472B",
            "-cpu",
            "cortex-a72",
            "-smp",
            "6",
            "-kernel",
            "/k/Image",
            "-initrd",
            "/k/initramfs",
            "-append",
            "root=/dev/ram0 rdinit=/init loglevel=7 nowatchdog \
             rng_core.default_quality=1024 console=ttyAMA0,115200 \
             systemd.journald.forward_to_console=1 virtio_gpu.modeset=1 \
             virtio_blk.emmc_index=1 dummy.numdummies=0",
            "-serial",
            "null",
            "-monitor",
            "none",
            "-parallel",
            "none",
        ],
    );
    want.extend(display_qmp_and_rng(&c));
    want.extend(expand(
        &c,
        &[
            "-netdev",
            "user,id=net0,hostfwd=tcp::2222-:22",
            "-device",
            "virtio-net-device,netdev=net0,mrg_rxbuf=off",
        ],
    ));
    want.extend(channels_jog_and_usb(&c));
    want.extend(expand(
        &c,
        &[
            "-device",
            "virtio-gpu-device,id=virtio-gpu0,xres=1280,yres=720,max_outputs=1",
            "-no-reboot",
            "-gdb",
            "tcp::1235",
        ],
    ));
    assert_eq!(c.build_argv(), want);
}

/// Every option set on an accelerated deck: the whole argv, in order.
#[test]
fn a_full_config_builds_this_exact_argv() {
    let mut c = QemuConfig::new(PathBuf::from("/k/Image"), PathBuf::from("/k/initramfs"));
    c.instance_id = 3;
    c.model = Model::Cdj1500x;
    c.accel = Some(KVM);
    c.shm = true;
    c.audio = true;
    c.audio_device_uid = Some("Some Output".into());
    c.service_mode = true;
    c.emmc_img = Some(PathBuf::from("/e/emmc.qcow2"));
    c.net_vmnet = Some(VmnetMode::Bridged("en0".into()));
    c.qmp_port = 4448;
    c.gdb_port = 1238;
    c.ssh_port = 2225;
    c.mac = Some("02:11:22:33:44:55".into());
    c.soc_serial = Some("c0ffee".into());
    c.serial_log = true;
    let mut want = expand(
        &c,
        &[
            "cdj3k-emu-qemu",
            "-machine",
            "virt,gic-version=3,kernel-irqchip=on",
            "-m",
            "3221225472B",
            "-accel",
            "kvm",
            "-cpu",
            "host",
            "-smp",
            "6",
            "-kernel",
            "/k/Image",
            "-initrd",
            "/k/initramfs",
            "-append",
            "root=/dev/ram0 rdinit=/init loglevel=7 nowatchdog \
             rng_core.default_quality=1024 console=ttyAMA0,115200 \
             systemd.journald.forward_to_console=1 virtio_gpu.modeset=1 subucom_testmode \
             virtio_blk.emmc_index=0 dummy.numdummies=0 cdj3k.serial=c0ffee snd-dummy.enable=0",
            "-serial",
            "file:{sock}{sep}serial.log",
            "-object",
            "memory-backend-file,id=ram0,size=3221225472B,mem-path={sock}{sep}ram.shm,share=on",
            "-machine",
            "virt,gic-version=3,kernel-irqchip=on,memory-backend=ram0",
        ],
    );
    want.extend(display_qmp_and_rng(&c));
    want.extend(audio("Some Output"));
    want.extend(expand(
        &c,
        &[
            "-netdev",
            "vmnet-bridged,id=net0,ifname=en0",
            "-device",
            "virtio-net-device,netdev=net0,mac=02:11:22:33:44:55,mrg_rxbuf=off",
        ],
    ));
    want.extend(channels_jog_and_usb(&c));
    want.extend(expand(
        &c,
        &[
            "-drive",
            "file=/e/emmc.qcow2,if=none,id=emmc0,format=qcow2,cache=writeback,\
             file.locking=off",
            "-device",
            "virtio-blk-device,drive=emmc0,id=emmc0",
            "-device",
            "virtio-gpu-device,id=virtio-gpu0,xres=1280,yres=800,max_outputs=1",
            "-no-reboot",
            "-gdb",
            "tcp::1238",
        ],
    ));
    assert_eq!(c.build_argv(), want);
}

/// A TAP link wins over vmnet, and software emulation with audio takes
/// the wider period: the whole argv, in order.
#[test]
fn a_tap_config_builds_this_exact_argv() {
    let mut c = QemuConfig::new(PathBuf::from("/k/Image"), PathBuf::from("/k/initramfs"));
    c.instance_id = 2;
    c.model = Model::Cdj3kx;
    c.accel = None;
    c.audio = true;
    c.net_tap_iface = Some("tap0".into());
    c.net_tap_fd = Some(7);
    c.net_vmnet = Some(VmnetMode::Host);
    c.gdb_port = 1237;
    let mut want = expand(
        &c,
        &[
            "cdj3k-emu-qemu",
            "-machine",
            "virt,gic-version=3,kernel-irqchip=off",
            "-m",
            "3221225472B",
            "-cpu",
            "cortex-a72",
            "-smp",
            "6",
            "-kernel",
            "/k/Image",
            "-initrd",
            "/k/initramfs",
            "-append",
            "root=/dev/ram0 rdinit=/init loglevel=7 nowatchdog \
             rng_core.default_quality=1024 console=ttyAMA0,115200 \
             systemd.journald.forward_to_console=1 virtio_gpu.modeset=1 \
             virtio_blk.emmc_index=1 dummy.numdummies=0 snd-dummy.enable=0 cdj3k.snd_period=512",
            "-serial",
            "null",
            "-monitor",
            "none",
            "-parallel",
            "none",
        ],
    );
    want.extend(display_qmp_and_rng(&c));
    want.extend(audio(""));
    want.extend([
        "-netdev".into(),
        cdj3k_emu_runtime::net::tap_netdev("net0", Some("tap0"), Some(7)).unwrap(),
        "-device".into(),
        "virtio-net-device,netdev=net0,mac=0a:00:00:00:00:02,mrg_rxbuf=off".into(),
    ]);
    want.extend(channels_jog_and_usb(&c));
    want.extend(expand(
        &c,
        &[
            "-device",
            "virtio-gpu-device,id=virtio-gpu0,xres=1280,yres=800,max_outputs=1",
            "-no-reboot",
            "-gdb",
            "tcp::1237",
        ],
    ));
    assert_eq!(c.build_argv(), want);
}
