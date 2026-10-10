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

/// `CDJ3K_KCMD_EXTRA`, trimmed: appended to the kernel cmdline. Lets a dev
/// boot hold a unit back (`systemd.mask=EP145.service`) so the console shell
/// is ready before the app starts, which is the only way to instrument its
/// startup without racing the getty.
fn kcmd_extra() -> Option<String> {
    let extra = std::env::var_os("CDJ3K_KCMD_EXTRA")?;
    let extra = extra.to_string_lossy();
    let extra = extra.trim();
    (!extra.is_empty()).then(|| extra.to_string())
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
        [
            vec!["cdj3k-emu-qemu".to_string()],
            self.machine_args(),
            self.kernel_args(),
            self.serial_args(),
            self.ram_file_args(),
            self.display_and_qmp_args(),
            rng_args(),
            self.audio_args(),
            self.net_args(),
            self.channel_args(),
            self.jog_shm_args(),
            self.disk_args(),
            self.gpu_args(),
            vec!["-no-reboot".into()],
            opt("-gdb", format!("tcp::{}", self.gdb_port)),
        ]
        .concat()
    }

    /// The `-machine` value before any memory backend is named.
    fn machine_base(&self) -> &'static str {
        // An in-kernel GIC saves a vCPU exit per interrupt, the dominant
        // cost for the player's IRQ load.
        if self.accel.is_some_and(|a| a.in_kernel_gic) {
            "virt,gic-version=3,kernel-irqchip=on"
        } else {
            "virt,gic-version=3,kernel-irqchip=off"
        }
    }

    /// Machine, RAM, accelerator, CPU model and core count.
    fn machine_args(&self) -> Vec<String> {
        let cpu = match self.accel {
            Some(accel) => [opt("-accel", accel.name), opt("-cpu", "host")].concat(),
            // The deck is an A72, so emulate one: the app reads /proc/cpuinfo.
            None => opt("-cpu", "cortex-a72"),
        };
        [
            opt("-machine", self.machine_base()),
            opt("-m", format!("{}B", Self::MEM_BYTES)),
            cpu,
            // The RK3399 has 6 cores (2x A72 + 4x A53) and the player app pins
            // work to specific ones: cryptsetup to core 4 (genkey_pr |
            // initoptenv, which opens cabinet.img), the X server to core 5.
            // Fewer cores make those `taskset -c 4/5` calls fail, so the
            // cabinet never unlocks and Device Library Plus cannot read its
            // keys.
            opt("-smp", "6"),
        ]
        .concat()
    }

    /// Kernel, initramfs and kernel command line.
    fn kernel_args(&self) -> Vec<String> {
        [
            opt("-kernel", self.kernel.display().to_string()),
            opt("-initrd", self.initramfs.display().to_string()),
            opt("-append", self.kernel_cmdline()),
        ]
        .concat()
    }

    fn kernel_cmdline(&self) -> String {
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
        // The built-in dummy driver (CONFIG_DUMMY=y) creates dummy0 at boot
        // unless numdummies is 0. The CDJ-3000's link-monitor.sh reads the
        // MAC of every interface and exits on a second one, before eth0 gets
        // an address. Guest patch 23 adds mlan0 as a dummy where the player
        // has one.
        kcmd.push_str(" dummy.numdummies=0");
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
        if let Some(extra) = kcmd_extra() {
            kcmd.push(' ');
            kcmd.push_str(&extra);
        }
        kcmd
    }

    /// Where the guest console goes.
    fn serial_args(&self) -> Vec<String> {
        if !self.serial_log {
            // No serial sink: also suppress QEMU's default monitor and
            // parallel chardevs. Otherwise the default-monitor allocator
            // creates a hidden QemuTextConsole that arms the VT100 cursor
            // blink timer (~250 ms self-rearm), and each fire walks the
            // glyph cache under BQL - measured at 0.4-0.7% of the audio
            // refill thread on a two-instance setup. Empty defaults =
            // no text consoles created = no cursor work.
            return [
                opt("-serial", "null"),
                opt("-monitor", "none"),
                opt("-parallel", "none"),
            ]
            .concat();
        }
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
        opt("-serial", sink)
    }

    /// Guest RAM backed by [`Self::shm_path`], when `shm` is set.
    fn ram_file_args(&self) -> Vec<String> {
        if !self.shm {
            return Vec::new();
        }
        [
            opt(
                "-object",
                format!(
                    "memory-backend-file,id=ram0,size={}B,mem-path={},share=on",
                    Self::MEM_BYTES,
                    self.shm_path().display()
                ),
            ),
            opt(
                "-machine",
                format!("{},memory-backend=ram0", self.machine_base()),
            ),
        ]
        .concat()
    }

    /// The main LCD's shared framebuffer and the QMP monitors.
    fn display_and_qmp_args(&self) -> Vec<String> {
        let mut args = [
            opt(
                "-display",
                format!("shm,path={}/main.shm", self.sock_dir().display()),
            ),
            opt(
                "-qmp",
                format!("tcp:localhost:{},server=on,wait=off", self.qmp_port),
            ),
        ]
        .concat();
        if crate::disk::PASSES_FDS {
            args.extend(opt(
                "-qmp",
                format!("unix:{},server=on,wait=off", self.qmp_fd_socket().display()),
            ));
        }
        args
    }

    /// The virtio-sound device on this host's audio backend.
    fn audio_args(&self) -> Vec<String> {
        // A host with no backend gets no `-audiodev` rather than another
        // host's driver, which QEMU would refuse.
        let (true, Some(backend)) = (self.audio, host::AUDIO) else {
            // When disabled: no virtio-sound device. The guest auto-loads
            // snd-dummy so JUCE still enumerates an ALSA card without
            // generating any guest↔host audio traffic.
            return Vec::new();
        };
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
        [
            opt("-audiodev", audiodev),
            opt("-device", "virtio-sound-device,audiodev=audio0,streams=1"),
        ]
        .concat()
    }

    /// The guest NIC: on the TAP link when there is one, else on vmnet, else
    /// on user-mode networking with SSH forwarded.
    fn net_args(&self) -> Vec<String> {
        let mac = self
            .mac
            .clone()
            .unwrap_or_else(|| format!("0a:00:00:00:00:{:02x}", self.instance_id & 0xff));
        let bridged =
            crate::net::tap_netdev("net0", self.net_tap_iface.as_deref(), self.net_tap_fd)
                .or_else(|| self.net_vmnet.as_ref().map(|mode| mode.netdev_arg("net0")));
        let (netdev, device) = match bridged {
            Some(netdev) => (
                netdev,
                format!("virtio-net-device,netdev=net0,mac={},mrg_rxbuf=off", mac),
            ),
            None => (
                format!("user,id=net0,hostfwd=tcp::{}-:22", self.ssh_port),
                "virtio-net-device,netdev=net0,mrg_rxbuf=off".into(),
            ),
        };
        [opt("-netdev", netdev), opt("-device", device)].concat()
    }

    /// The virtio-serial ports in [`CHANNELS`], each on `{sock_dir}/<name>.sock`.
    fn channel_args(&self) -> Vec<String> {
        let sock = self.sock_dir();
        let mut args = opt("-device", "virtio-serial-device,max_ports=8");
        for name in CHANNELS {
            let sock_path = sock.join(format!("{}.sock", name));
            args.extend(opt(
                "-chardev",
                format!(
                    "socket,id=vserial_{},path={},server=on,wait=off",
                    name,
                    sock_path.display()
                ),
            ));
            args.extend(opt(
                "-device",
                format!(
                    "virtserialport,chardev=vserial_{},name=cdj3k.{}",
                    name, name
                ),
            ));
        }
        args
    }

    /// ivshmem-plain (jog LCD zero-copy frame buffer). The guest's
    /// deck_shim.so writes extracted 320×240 XRGB pixels directly into
    /// BAR2; the host mmaps `jog.shm` and polls the seqlock counter.
    fn jog_shm_args(&self) -> Vec<String> {
        [
            opt(
                "-object",
                format!(
                    "memory-backend-file,id=jogshm,mem-path={},size=1M,share=on",
                    self.jog_shm_path().display()
                ),
            ),
            opt("-device", "ivshmem-plain,memdev=jogshm,master=on"),
        ]
        .concat()
    }

    /// The USB slot and the eMMC.
    ///
    /// Pioneer virtio_blk.c probes in reverse virtio-mmio slot order (last
    /// listed = first probed). USB is listed BEFORE eMMC so the indices work
    /// out:
    ///   usb0  (listed first  → probed second → index 1 → /dev/sdb)
    ///   emmc0 (listed second → probed first  → index 0 → /dev/mmcblk1)
    ///
    /// The USB slot is always present, initially backed by a 1-sector
    /// placeholder. UsbManager hot-swaps the medium via QMP
    /// blockdev-change-medium without restarting QEMU;
    /// virtio_blk_change_media fires virtio_notify_config which triggers
    /// Pioneer's virtblk_config_changed → revalidate_disk on the guest.
    ///
    /// file.locking=off - qcow2/raw open uses fcntl byte-range locks at offset
    /// 100; if a prior QEMU subprocess hasn't fully released its FD when we
    /// restart, the new process aborts with "Failed to lock byte 100". We
    /// gate exclusive access at the host (.app) level instead.
    fn disk_args(&self) -> Vec<String> {
        let mut args = [
            opt(
                "-drive",
                format!(
                    "file={},if=none,id=usb0,format=raw,cache=writeback,file.locking=off",
                    self.usb_placeholder_path().display()
                ),
            ),
            opt("-device", "virtio-blk-device,drive=usb0,id=usb0"),
        ]
        .concat();
        if let Some(img) = &self.emmc_img {
            args.extend(opt(
                "-drive",
                format!(
                    "file={},if=none,id=emmc0,format=qcow2,cache=writeback,file.locking=off",
                    img.display()
                ),
            ));
            args.extend(opt("-device", "virtio-blk-device,drive=emmc0,id=emmc0"));
        }
        args
    }

    /// The main LCD, at the model's scanout mode.
    fn gpu_args(&self) -> Vec<String> {
        let (lcd_w, lcd_h) = self.model.main_lcd();
        opt(
            "-device",
            format!("virtio-gpu-device,id=virtio-gpu0,xres={lcd_w},yres={lcd_h},max_outputs=1"),
        )
    }
}

/// The virtio-serial channels, each a `cdj3k.<name>` port in the guest.
const CHANNELS: [&str; 3] = [
    // Bidirectional: subucom_forwarder bridges subucom_ctrl ↔ host.
    "ctrl",
    // Bidirectional: cdj3k-cfgd ↔ host runtime.
    //   host→guest: usb attach|detach, set/get sysfs params
    //   guest→host: usb_state, param values, latency every 3s
    "cfg",
    // Bidirectional: pc-link-bridge in the guest ↔ cdj3k-emu-runtime.
    // Carries HID (/dev/hidraw0) and MIDI (/dev/snd/midiC1D0) frames
    // for the dummy_hcd-attached gadget; the host surfaces HID as an
    // IOHIDUserDevice and MIDI as a CoreMIDI endpoint. Idle until the
    // pc_link toggle goes on.
    "usb-link",
];

/// The host's entropy source behind a virtio-rng device.
fn rng_args() -> Vec<String> {
    [
        opt("-object", host::RNG_OBJECT),
        opt("-device", "virtio-rng-device,rng=rng0"),
    ]
    .concat()
}

/// One option and its value.
fn opt(flag: &str, value: impl Into<String>) -> Vec<String> {
    vec![flag.into(), value.into()]
}
