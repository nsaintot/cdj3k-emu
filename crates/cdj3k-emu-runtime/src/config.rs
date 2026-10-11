use std::path::PathBuf;

use cdj3k_emu_panel::Model;
use cdj3k_emu_platform::host::{self, Accelerator};

/// Configuration for a single QEMU player instance.
#[derive(Clone, Debug)]
pub struct QemuConfig {
    /// The instance index, which selects the socket directory
    /// ([`cdj3k_emu_platform::runtime_paths::instance_dir`]).
    pub instance_id: u32,

    /// The player model this instance boots. It sets the main LCD scanout mode.
    pub model: Model,

    /// The aarch64 kernel image.
    pub kernel: PathBuf,

    /// The initramfs image (initramfs-patched.cpio.gz).
    pub initramfs: PathBuf,

    /// The hypervisor to run under, or `None` for software emulation.
    pub accel: Option<Accelerator>,

    /// Back the guest RAM with a MAP_SHARED file, so that the host can map it.
    pub shm: bool,

    /// Add a virtio-sound-device.
    pub audio: bool,

    /// Which host output to bind the stream to, named the way this host's
    /// audio backend names one: a CoreAudio device UID
    /// (kAudioDevicePropertyDeviceUID) on macOS, a PipeWire sink's node name
    /// on Linux. `None` follows the system default output. It applies only
    /// when `audio` is true.
    pub audio_device_uid: Option<String>,

    /// Boot EP122 into its service and test mode.
    pub service_mode: bool,

    /// The eMMC qcow2 image. The guest names it `/dev/mmcblk<emmc_index>`:
    /// mmcblk1 on the RK3399 decks and mmcblk0 on the CDJ-1500X. p7 holds the
    /// settings (/home/root/settings) and p8 the user data (/mnt).
    pub emmc_img: Option<PathBuf>,

    /// The vmnet back end for Pro DJ Link. QEMU opens the interface itself
    /// through `-netdev vmnet-host` or `vmnet-bridged`, unprivileged under the
    /// `com.apple.developer.networking.vmnet` entitlement.
    pub net_vmnet: Option<crate::net::vmnet::VmnetMode>,

    /// The TAP interface name. Where QEMU receives a descriptor, the name is
    /// informational; on Windows, QEMU opens the adapter by this name.
    pub net_tap_iface: Option<String>,

    /// An open file descriptor for QEMU's TAP device. When it is set, QEMU
    /// receives `-netdev tap,fd=<N>`. FD_CLOEXEC must be cleared on the fd
    /// before exec.
    pub net_tap_fd: Option<i32>,

    /// The QMP TCP port. The launcher uses 4445 plus the instance index, so
    /// that several instances can run at once.
    pub qmp_port: u16,

    /// The GDB TCP port. The launcher uses 1235 plus the instance index.
    pub gdb_port: u16,

    /// The host port forwarded to the guest's port 22. The launcher uses 2222
    /// plus the instance index.
    pub ssh_port: u16,

    /// The persistent MAC of the guest virtio-net device. When it is `None`,
    /// the device uses `0a:00:00:00:00:{instance_id}`.
    pub mac: Option<String>,

    /// The SoC serial that the guest publishes as the `Serial` line of
    /// `/proc/cpuinfo`. It is passed as `cdj3k.serial=` on the kernel command
    /// line, where guest patch 12 picks it up. With `None`, the line is
    /// absent and `genkey_pr` cannot derive the `cabinet.img` key.
    pub soc_serial: Option<String>,

    /// When `true`, QEMU writes the guest serial console to
    /// `{sock_dir}/serial.log`. It is off by default, because the file grows
    /// without limit over a long session and is useful only to debug the boot
    /// or a kernel panic. Turn it on with `--serial-log` or
    /// `CDJ3K_SERIAL_LOG=1`.
    pub serial_log: bool,
}

/// `CDJ3K_SERIAL_SOCKET=1` puts the guest console on a Unix socket instead of
/// a log file. A guest built with `ENABLE_SSH=1` logs root in on it
/// automatically, so the socket is a scriptable shell.
fn serial_is_socket() -> bool {
    std::env::var_os("CDJ3K_SERIAL_SOCKET").is_some_and(|v| v != "0" && !v.is_empty())
}

/// Return `CDJ3K_KCMD_EXTRA`, trimmed, for the end of the kernel command line.
/// A development boot can use it to hold a unit back
/// (`systemd.mask=EP145.service`), so that the console shell is ready before
/// the app starts. This is the only way to instrument the app's startup
/// without racing the getty.
fn kcmd_extra() -> Option<String> {
    let extra = std::env::var_os("CDJ3K_KCMD_EXTRA")?;
    let extra = extra.to_string_lossy();
    let extra = extra.trim();
    (!extra.is_empty()).then(|| extra.to_string())
}

impl QemuConfig {
    /// The guest RAM size in bytes: 3 GiB.
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

    /// The socket directory, from [`cdj3k_emu_platform::runtime_paths::instance_dir`].
    pub fn sock_dir(&self) -> PathBuf {
        cdj3k_emu_platform::runtime_paths::instance_dir(self.instance_id)
    }

    /// The second QMP monitor, on a Unix socket: the only transport that can
    /// carry a file descriptor into QEMU (`add-fd`).
    pub fn qmp_fd_socket(&self) -> PathBuf {
        self.sock_dir().join("qmp-fd.sock")
    }

    /// The path of the shared RAM file, used only when `shm` is true.
    pub fn shm_path(&self) -> PathBuf {
        self.sock_dir().join("ram.shm")
    }

    /// The ivshmem backing file for the jog LCD frame buffer, which the host
    /// reads without copying. It is 1 MiB, and its layout matches `JOG_SHM_*`
    /// in `guest/shim/shim.h` and `crates/cdj3k-emu-streams/src/jog_stream.rs`.
    pub fn jog_shm_path(&self) -> PathBuf {
        self.sock_dir().join("jog.shm")
    }

    /// The placeholder backing file for the USB virtio-blk slot, which is
    /// always present. QemuInstance::spawn creates it, and QMP
    /// blockdev-change-medium swaps it while the guest runs.
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

    /// The machine, RAM, accelerator, CPU model and core count.
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
            // The RK3399 has six cores, two A72 and four A53, and the player
            // app pins work to specific ones: cryptsetup to core 4 (the
            // `genkey_pr | initoptenv` pipeline that opens cabinet.img) and the
            // X server to core 5.
            // Fewer cores make those `taskset -c 4/5` calls fail, so the
            // cabinet never unlocks and Device Library Plus cannot read its
            // keys.
            opt("-smp", "6"),
        ]
        .concat()
    }

    /// The kernel, the initramfs and the kernel command line.
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
        // The PL011 UART console works on the vanilla 6.6 kernel, so it is
        // always enabled.
        kcmd.push_str(" console=ttyAMA0,115200");
        // Forward the journal to the console, so that app crash details appear
        // in the serial log. When the console is the interactive shell, the
        // forwarded stream would drown the prompt, so it stays off.
        if !serial_is_socket() {
            kcmd.push_str(" systemd.journald.forward_to_console=1");
        }
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
        // snd-dummy is built in (CONFIG_SND_DUMMY=y), so its card registers
        // first and JUCE picks card 0. With virtio-snd present, disable
        // snd-dummy so that vsnd becomes card 0 and JUCE opens it.
        if self.audio {
            kcmd.push_str(" snd-dummy.enable=0");
            // Under TCG the guest cannot take the player's 64-frame period
            // (1500 wakeups a second into a 2.7 ms ring at 96 kHz); a larger
            // one gives the audio thread room to be late. The guest's
            // insmod-virtio-snd reads this parameter.
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
            // Without a serial sink, also turn off QEMU's default monitor and
            // parallel chardevs. Otherwise QEMU creates a hidden
            // QemuTextConsole whose VT100 cursor blink timer fires every
            // 250 ms or so and walks the glyph cache under the BQL, which
            // costs the audio refill thread 0.4-0.7% with two instances
            // running.
            return [
                opt("-serial", "null"),
                opt("-monitor", "none"),
                opt("-parallel", "none"),
            ]
            .concat();
        }
        // `CDJ3K_SERIAL_SOCKET=1` puts the console on a Unix socket instead
        // of a file. A guest built with `ENABLE_SSH=1` logs root in on
        // ttyAMA0 automatically, so the socket gives a shell without a
        // debugger or an sshd.
        let sink = if serial_is_socket() {
            let sock = self.sock_dir().join("serial.sock").display().to_string();
            format!("unix:{},server=on,wait=off", sock)
        } else {
            format!("file:{}", self.sock_dir().join("serial.log").display())
        };
        opt("-serial", sink)
    }

    /// Back the guest RAM with [`Self::shm_path`] when `shm` is set.
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
            // With audio off there is no virtio-sound device. The guest loads
            // snd-dummy, so JUCE still finds an ALSA card and no audio passes
            // between the guest and the host.
            return Vec::new();
        };
        let mut audiodev = String::from(backend.driver);
        audiodev.push_str(",id=audio0,in.voices=0");
        audiodev.push_str(backend.options);
        if let Some(uid) = self.audio_device_uid.as_deref().filter(|s| !s.is_empty()) {
            // A pinned device from the per-instance Audio Output picker is
            // named here so the backend binds to it rather than to the
            // system default. Both selectors' values contain ':' and
            // spaces, which QEMU's `-audiodev` parser accepts, because it
            // splits only on ',' and '='.
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

    /// The ivshmem-plain device for the jog LCD frame buffer. The guest's
    /// deck_shim.so writes the 320×240 XRGB pixels directly into BAR2, and
    /// the host maps `jog.shm` and polls the seqlock counter.
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
    /// The guest probes the virtio-mmio slots in reverse order, so the last
    /// device listed is probed first and takes virtio-blk index 0. The
    /// guest's virtio_blk names index 0 `mmcblkN` and index 1 `sdb`
    /// (guest/kernel-patches/01-virtio-blk-naming.patch), so the USB slot is
    /// listed before the eMMC:
    ///
    /// - usb0 is listed first, probed second, and becomes /dev/sdb.
    /// - emmc0 is listed second, probed first, and becomes /dev/mmcblk1, or
    ///   /dev/mmcblk0 on the CDJ-1500X.
    ///
    /// The USB slot is always present and starts with a one-sector
    /// placeholder. UsbManager swaps the medium through QMP
    /// blockdev-change-medium without restarting QEMU. QEMU then notifies a
    /// config change, and the guest resizes the disk with
    /// set_capacity_and_notify.
    ///
    /// The drives use file.locking=off. Opening a qcow2 or raw image takes
    /// fcntl byte-range locks at offset 100, and if the previous QEMU has not
    /// released its fd when the slot restarts, the new process aborts with
    /// "Failed to lock byte 100". The app enforces exclusive access itself.
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
    // subucom_forwarder carries traffic both ways between subucom_ctrl and
    // the host.
    "ctrl",
    // cdj3k-cfgd and the host runtime talk both ways. The protocol is
    // described in `crate::cfg`.
    "cfg",
    // pc-link-bridge in the guest and cdj3k-emu-runtime talk both ways. The
    // port carries HID (/dev/hidraw0) and MIDI (/dev/snd/midiC1D0) frames for
    // the gadget attached to dummy_hcd; the host shows HID as an
    // IOHIDUserDevice and MIDI as a CoreMIDI endpoint. The port is idle until
    // the pc_link toggle turns on.
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
