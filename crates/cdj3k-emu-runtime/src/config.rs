use std::path::PathBuf;

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::host::{self, Accelerator};

/// Configuration for a single QEMU player instance.
#[derive(Clone, Debug)]
pub struct QemuConfig {
    /// Instance index - selects /tmp/cdj3k-emu/instance-{id}/ socket directory.
    pub instance_id: u32,

    /// The player model this instance boots: sets the main LCD scanout mode.
    pub model: Model,

    /// aarch64 kernel image.
    pub kernel: PathBuf,

    /// Initramfs image (initramfs-patched.cpio.gz).
    pub initramfs: PathBuf,

    /// The hypervisor to run under, or `None` for software emulation.
    pub accel: Option<Accelerator>,

    /// Back guest RAM with a MAP_SHARED file for host-side mmap injection.
    pub shm: bool,

    /// Expose virtio-sound-device.
    pub audio: bool,

    /// Which host output to bind the stream to, named the way this host's
    /// audio backend names one: a CoreAudio device UID
    /// (kAudioDevicePropertyDeviceUID) on macOS, a PipeWire sink's node name
    /// on Linux. `None` means "follow the system default output". Only
    /// meaningful when `audio == true`.
    pub audio_device_uid: Option<String>,

    /// Boot into EP122 service/test mode.
    pub service_mode: bool,

    /// eMMC qcow2 image.
    /// virtio_blk.c maps device index 0 → /dev/mmcblk1 (major 179, base minor 8).
    /// Partitions p1..p8 appear as mmcblk1p1..mmcblk1p8.
    /// p7 = settings (/home/root/settings), p8 = user data (/mnt).
    pub emmc_img: Option<PathBuf>,

    /// vmnet backend for Pro DJ Link.  QEMU opens the interface itself via
    /// `-netdev vmnet-host` / `vmnet-bridged`, unprivileged under the
    /// `com.apple.developer.networking.vmnet` entitlement.
    pub net_vmnet: Option<crate::net::vmnet::VmnetMode>,

    /// TAP interface name. Informational where QEMU is handed a descriptor;
    /// the adapter QEMU opens where it is not (Windows).
    pub net_tap_iface: Option<String>,

    /// Open file descriptor for the QEMU-side TAP device.
    /// When set, QEMU receives `-netdev tap,fd=<N>`.
    /// The fd must have FD_CLOEXEC cleared before exec.
    pub net_tap_fd: Option<i32>,

    /// QMP TCP port.  Default: 4445 + instance_id (so multiple instances can coexist).
    pub qmp_port: u16,

    /// GDB TCP port.  Default: 1235 + instance_id.
    pub gdb_port: u16,

    /// SSH port forwarded from guest :22.  Default: 2222 + instance_id.
    pub ssh_port: u16,

    /// Persistent MAC for the guest virtio-net device. When `None`, falls back
    /// to the deterministic `0a:00:00:00:00:{instance_id}`.
    pub mac: Option<String>,

    /// SoC serial the guest publishes as the `Serial` line of `/proc/cpuinfo`,
    /// passed as `cdj3k.serial=` on the kernel cmdline and picked up by guest
    /// patch 12.  `None` leaves the line absent, and `genkey_pr` cannot derive
    /// the `cabinet.img` key.
    pub soc_serial: Option<String>,

    /// When `true`, QEMU writes the guest serial console to
    /// `{sock_dir}/serial.log`. Disabled by default in release builds - the
    /// file grows unbounded over a long session and is only useful when
    /// debugging boot / kernel panics. Enable from the CLI with `--serial-log`
    /// or by setting `CDJ3K_SERIAL_LOG=1`.
    pub serial_log: bool,
}

/// `CDJ3K_SERIAL_SOCKET=1`: put the guest console on a unix socket rather than
/// a log file. A guest built with `ENABLE_SSH=1` autologins root on it, so it
/// is a scriptable shell.
fn serial_is_socket() -> bool {
    std::env::var_os("CDJ3K_SERIAL_SOCKET").is_some_and(|v| v != "0" && !v.is_empty())
}

impl QemuConfig {
    /// Guest RAM in bytes. Set to 3 GiB.
    pub const MEM_BYTES: u64 = 0xC000_0000;

    pub fn new(kernel: PathBuf, initramfs: PathBuf) -> Self {
        Self {
            instance_id: 0,
            model: Model::Cdj3k,
            kernel,
            initramfs,
            accel: host::accelerator(),
            shm: false,
            audio: false,
            audio_device_uid: None,
            service_mode: false,
            emmc_img: None,
            net_vmnet: None,
            net_tap_iface: None,
            net_tap_fd: None,
            qmp_port: 4445,
            gdb_port: 1235,
            ssh_port: 2222,
            mac: None,
            soc_serial: None,
            serial_log: false,
        }
    }

    /// Socket directory: see [`runtime_paths::instance_dir`].
    pub fn sock_dir(&self) -> PathBuf {
        cdj3k_emu_platform::runtime_paths::instance_dir(self.instance_id)
    }

    /// The second QMP monitor, on a Unix socket: the only transport that can
    /// carry a file descriptor into QEMU (`add-fd`).
    pub fn qmp_fd_socket(&self) -> PathBuf {
        self.sock_dir().join("qmp-fd.sock")
    }

    /// Shared RAM file path (only when shm=true).
    pub fn shm_path(&self) -> PathBuf {
        self.sock_dir().join("ram.shm")
    }

    /// ivshmem-backed file for the jog-LCD zero-copy frame buffer.
    /// 1 MiB; layout matches `JOG_SHM_*` in `guest/shim/shim.h` and
    /// `crates/cdj3k-emu-streams/src/jog_stream.rs`.
    pub fn jog_shm_path(&self) -> PathBuf {
        self.sock_dir().join("jog.shm")
    }

    /// Placeholder backing file for the always-present USB virtio-blk slot.
    /// Created by QemuInstance::spawn; swapped live via QMP blockdev-change-medium.
    pub fn usb_placeholder_path(&self) -> PathBuf {
        self.sock_dir().join("usb.placeholder")
    }

    /// Build the argv list to pass to cdj3k_emu_qemu_run.
    pub fn build_argv(&self) -> Vec<String> {
        let sock = self.sock_dir();

        // An in-kernel GIC saves a vCPU exit per interrupt, the dominant
        // cost for the player's IRQ load.
        let in_kernel_gic = self.accel.is_some_and(|a| a.in_kernel_gic);
        let machine_base = if in_kernel_gic {
            "virt,gic-version=3,kernel-irqchip=on"
        } else {
            "virt,gic-version=3,kernel-irqchip=off"
        };

        let mut args: Vec<String> = vec![
            "cdj3k-emu-qemu".into(),
            "-machine".into(),
            machine_base.into(),
            "-m".into(),
            format!("{}B", Self::MEM_BYTES),
        ];

        match self.accel {
            Some(accel) => {
                args.extend(["-accel".into(), accel.name.into()]);
                args.extend(["-cpu".into(), "host".into()]);
            }
            // The deck is an A72, so emulate one: the app reads /proc/cpuinfo.
            None => args.extend(["-cpu".into(), "cortex-a72".into()]),
        }

        // The RK3399 has 6 cores (2x A72 + 4x A53) and the player app pins
        // work to specific ones: cryptsetup to core 4 (genkey_pr | initoptenv,
        // which opens cabinet.img), the X server to core 5. Fewer cores make
        // those `taskset -c 4/5` calls fail, so the cabinet never unlocks and
        // Device Library Plus cannot read its keys.
        args.extend(["-smp".into(), "6".into()]);

        args.extend(["-kernel".into(), self.kernel.display().to_string()]);
        args.extend(["-initrd".into(), self.initramfs.display().to_string()]);

        let mut kcmd =
            "root=/dev/ram0 rdinit=/init loglevel=7 nowatchdog rng_core.default_quality=1024"
                .to_string();
        // PL011 UART console works on vanilla 6.6 - always wire it up.
        kcmd.push_str(" console=ttyAMA0,115200");
        // Forward journal to console so app crash details appear in the serial
        // log - except when the console is the interactive shell, where the
        // forwarded stream drowns the prompt and anything typed at it.
        if !serial_is_socket() {
            kcmd.push_str(" systemd.journald.forward_to_console=1");
        }
        // virtio_gpu_init() requires VIRTIO_F_VERSION_1; force-legacy=off on the
        // MMIO transport ensures that bit is negotiated.
        kcmd.push_str(" virtio_gpu.modeset=1");
        if self.service_mode {
            kcmd.push_str(" subucom_testmode");
        }
        // The guest kernel names the eMMC `mmcblk<emmc_index>` (patch
        // 01-virtio-blk-naming).
        kcmd.push_str(&format!(
            " virtio_blk.emmc_index={}",
            self.model.spec().emmc_index
        ));
        // Guest patch 12 turns this into the `Serial` line of /proc/cpuinfo,
        // which genkey_pr hashes with the model to key cabinet.img.
        if let Some(serial) = &self.soc_serial {
            kcmd.push_str(&format!(" cdj3k.serial={}", serial));
        }
        // snd-dummy is built-in (CONFIG_SND_DUMMY=y) so its card always
        // auto-registers first - and JUCE picks card 0. When the real
        // virtio-snd path is wired up, disable Dummy so JUCE opens vsnd
        // (which becomes card 0 when Dummy is silent).
        if self.audio {
            kcmd.push_str(" snd-dummy.enable=0");
            // Under TCG the guest cannot take the player's 64-frame period
            // (1500 wakeups a second into a 2.7 ms ring at 96 kHz); a larger
            // one gives the audio thread room to be late. Read by the guest's
            // insmod-virtio-snd.
            if self.accel.is_none() {
                kcmd.push_str(" cdj3k.snd_period=512");
            }
        }
        // `CDJ3K_KCMD_EXTRA`: append to the kernel cmdline. Lets a dev boot
        // hold a unit back (`systemd.mask=EP145.service`) so the console shell
        // is ready before the app starts, which is the only way to instrument
        // its startup without racing the getty.
        if let Some(extra) = std::env::var_os("CDJ3K_KCMD_EXTRA") {
            let extra = extra.to_string_lossy();
            if !extra.trim().is_empty() {
                kcmd.push(' ');
                kcmd.push_str(extra.trim());
            }
        }
        args.extend(["-append".into(), kcmd]);

        if self.serial_log {
            // `CDJ3K_SERIAL_SOCKET=1` puts the console on a unix socket instead
            // of a file. A guest built with `ENABLE_SSH=1` autologins root on
            // ttyAMA0, so connecting to it is a shell - which is how the guest
            // gets traced without a debugger or an sshd.
            let sink = if serial_is_socket() {
                let sock = self.sock_dir().join("serial.sock").display().to_string();
                format!("unix:{},server=on,wait=off", sock)
            } else {
                format!("file:{}", self.sock_dir().join("serial.log").display())
            };
            args.extend(["-serial".into(), sink]);
        } else {
            // No serial sink: also suppress QEMU's default monitor and
            // parallel chardevs. Otherwise the default-monitor allocator
            // creates a hidden QemuTextConsole that arms the VT100 cursor
            // blink timer (~250 ms self-rearm), and each fire walks the
            // glyph cache under BQL - measured at 0.4-0.7% of the audio
            // refill thread on a two-instance setup. Empty defaults =
            // no text consoles created = no cursor work.
            args.extend([
                "-serial".into(),
                "null".into(),
                "-monitor".into(),
                "none".into(),
                "-parallel".into(),
                "none".into(),
            ]);
        }

        if self.shm {
            let path = self.shm_path();
            args.extend([
                "-object".into(),
                format!(
                    "memory-backend-file,id=ram0,size={}B,mem-path={},share=on",
                    Self::MEM_BYTES,
                    path.display()
                ),
                "-machine".into(),
                format!("{},memory-backend=ram0", machine_base),
            ]);
        }

        args.extend([
            "-display".into(),
            format!("shm,path={}/main.shm", sock.display()),
        ]);
        args.extend([
            "-qmp".into(),
            format!("tcp:localhost:{},server=on,wait=off", self.qmp_port),
        ]);
        if crate::disk::PASSES_FDS {
            args.extend([
                "-qmp".into(),
                format!("unix:{},server=on,wait=off", self.qmp_fd_socket().display()),
            ]);
        }

        args.extend([
            "-object".into(),
            host::RNG_OBJECT.into(),
            "-device".into(),
            "virtio-rng-device,rng=rng0".into(),
        ]);

        // A host with no backend gets no `-audiodev` rather than another
        // host's driver, which QEMU would refuse.
        if let (true, Some(backend)) = (self.audio, host::AUDIO) {
            let mut audiodev = String::from(backend.driver);
            audiodev.push_str(",id=audio0,in.voices=0");
            audiodev.push_str(backend.options);
            if let Some(uid) = self.audio_device_uid.as_deref().filter(|s| !s.is_empty()) {
                // A pinned device from the per-instance Audio Output picker is
                // named here so the backend binds to it rather than to the
                // system default. Both selectors' values contain ':' and
                // spaces, which QEMU's `-audiodev` parser passes through — it
                // splits on ',' and '=' only.
                audiodev.push_str(backend.device_selector);
                audiodev.push_str(uid);
            }
            args.extend([
                "-audiodev".into(),
                audiodev,
                "-device".into(),
                "virtio-sound-device,audiodev=audio0,streams=1".into(),
            ]);
        }
        // When disabled: no virtio-sound device. The guest auto-loads
        // snd-dummy so JUCE still enumerates an ALSA card without
        // generating any guest↔host audio traffic.

        let mac = self
            .mac
            .clone()
            .unwrap_or_else(|| format!("0a:00:00:00:00:{:02x}", self.instance_id & 0xff));
        if let Some(netdev) =
            crate::net::tap_netdev("net0", self.net_tap_iface.as_deref(), self.net_tap_fd)
        {
            args.extend([
                "-netdev".into(),
                netdev,
                "-device".into(),
                format!("virtio-net-device,netdev=net0,mac={},mrg_rxbuf=off", mac),
            ]);
        } else if let Some(mode) = &self.net_vmnet {
            args.extend([
                "-netdev".into(),
                mode.netdev_arg("net0"),
                "-device".into(),
                format!("virtio-net-device,netdev=net0,mac={},mrg_rxbuf=off", mac),
            ]);
        } else {
            args.extend([
                "-netdev".into(),
                format!("user,id=net0,hostfwd=tcp::{}-:22", self.ssh_port),
                "-device".into(),
                "virtio-net-device,netdev=net0,mrg_rxbuf=off".into(),
            ]);
        }

        args.extend(["-device".into(), "virtio-serial-device,max_ports=8".into()]);
        for (name, nr) in &[
            // Bidirectional: subucom_forwarder bridges subucom_ctrl ↔ host.
            ("ctrl", None::<u32>),
            // Bidirectional: cdj3k-cfgd ↔ host runtime.
            //   host→guest: usb attach|detach, set/get sysfs params
            //   guest→host: usb_state, param values, latency every 3s
            ("cfg", None),
            // Bidirectional: pc-link-bridge in the guest ↔ cdj3k-emu-runtime.
            // Carries HID (/dev/hidraw0) and MIDI (/dev/snd/midiC1D0) frames
            // for the dummy_hcd-attached gadget; the host surfaces HID as an
            // IOHIDUserDevice and MIDI as a CoreMIDI endpoint. Idle until the
            // pc_link toggle goes on.
            ("usb-link", None),
        ] {
            let sock_path = sock.join(format!("{}.sock", name));
            args.extend([
                "-chardev".into(),
                format!(
                    "socket,id=vserial_{},path={},server=on,wait=off",
                    name,
                    sock_path.display()
                ),
            ]);
            let mut dev = format!(
                "virtserialport,chardev=vserial_{},name=cdj3k.{}",
                name, name
            );
            if let Some(n) = nr {
                dev.push_str(&format!(",nr={}", n));
            }
            args.extend(["-device".into(), dev]);
        }

        // ivshmem-plain (jog LCD zero-copy frame buffer). The guest's
        // deck_shim.so writes extracted 320×240 XRGB pixels directly into
        // BAR2; the host mmaps `jog.shm` and polls the seqlock counter.
        args.extend([
            "-object".into(),
            format!(
                "memory-backend-file,id=jogshm,mem-path={},size=1M,share=on",
                self.jog_shm_path().display()
            ),
            "-device".into(),
            "ivshmem-plain,memdev=jogshm,master=on".into(),
        ]);

        // Pioneer virtio_blk.c probes in reverse virtio-mmio slot order (last listed = first
        // probed).  USB is listed BEFORE eMMC so the indices work out:
        //   usb0  (listed first  → probed second → index 1 → /dev/sdb)
        //   emmc0 (listed second → probed first  → index 0 → /dev/mmcblk1)
        //
        // The USB slot is always present, initially backed by a 1-sector placeholder.
        // UsbManager hot-swaps the medium via QMP blockdev-change-medium without
        // restarting QEMU; virtio_blk_change_media fires virtio_notify_config which
        // triggers Pioneer's virtblk_config_changed → revalidate_disk on the guest.
        // file.locking=off - qcow2/raw open uses fcntl byte-range locks at offset
        // 100; if a prior QEMU subprocess hasn't fully released its FD when we
        // restart, the new process aborts with "Failed to lock byte 100". We
        // gate exclusive access at the host (.app) level instead.
        args.extend([
            "-drive".into(),
            format!(
                "file={},if=none,id=usb0,format=raw,cache=writeback,file.locking=off",
                self.usb_placeholder_path().display()
            ),
            "-device".into(),
            "virtio-blk-device,drive=usb0,id=usb0".into(),
        ]);

        if let Some(img) = &self.emmc_img {
            args.extend([
                "-drive".into(),
                format!(
                    "file={},if=none,id=emmc0,format=qcow2,cache=writeback,file.locking=off",
                    img.display()
                ),
                "-device".into(),
                "virtio-blk-device,drive=emmc0,id=emmc0".into(),
            ]);
        }

        let (lcd_w, lcd_h) = self.model.main_lcd();
        args.extend([
            "-device".into(),
            format!("virtio-gpu-device,id=virtio-gpu0,xres={lcd_w},yres={lcd_h},max_outputs=1"),
        ]);

        args.extend(["-no-reboot".into()]);
        args.extend(["-gdb".into(), format!("tcp::{}", self.gdb_port)]);

        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
