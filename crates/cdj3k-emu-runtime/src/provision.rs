//! Booting the short-lived guest that provisions the initramfs.
//!
//! `cdj3k-emu-firmware` describes the provisioning boot but cannot start one —
//! QEMU lives here. This supplies the [`GuestRunner`] it asks for.
//!
//! The boot is nothing like a player launch: no displays, no sockets, no
//! audio, no networking. It unpacks an initramfs that carries the patch
//! scripts, runs them, writes the result to a scratch disk and powers off.

use std::path::Path;

use cdj3k_emu_firmware::GuestRunner;
use cdj3k_emu_platform::host::{self, Accelerator};

/// How long to wait before assuming the guest will never power off.
///
/// Generous because the work is bounded but slow under TCG: unpacking a
/// ~150 MiB initramfs, running the patch set, and tarring the result back.
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// Boots the provisioning guest with the emulator's own QEMU.
pub struct QemuGuestRunner {
    /// The accelerators to try, fastest first. Software emulation follows
    /// them.
    ///
    /// Measured on this rootfs: the in-kernel vGIC boots it in 1.1 s against
    /// 1.8 s for the emulated one, and 29.6 s under TCG.
    pub accelerators: Vec<Accelerator>,
    /// Provision passwordless root SSH into the guest: `cdj3k.ssh=1` on the
    /// provisioning boot, which the patch dispatcher reads as `ENABLE_SSH`.
    pub ssh: bool,
}

impl Default for QemuGuestRunner {
    fn default() -> Self {
        Self {
            accelerators: host::accelerators().unwrap_or_default(),
            ssh: false,
        }
    }
}

impl GuestRunner for QemuGuestRunner {
    fn run(&self, kernel: &Path, initramfs: &Path, scratch: &Path) -> Result<(), String> {
        // QEMU exits rather than degrading when it cannot get the acceleration
        // it was asked for, so the ladder is walked here. A shipped build is
        // entitled and stops on the first rung.
        let mut output = String::new();
        for accel in self.ladder() {
            output = spawn_qemu(&provisioning_argv(accel, kernel, initramfs, scratch, self.ssh))?;
            match refused(&output) {
                Some(reason) => eprintln!("[provision] {accel:?} unavailable ({reason}), retrying"),
                None => break,
            }
        }

        cdj3k_emu_firmware::initramfs_guest::guest_outcome(&output)
            .map_err(|why| format!("{why}:\n{}", tail(&output, 20)))
    }
}

impl QemuGuestRunner {
    /// The rungs to try: each accelerator, then software emulation (`None`).
    fn ladder(&self) -> Vec<Option<Accelerator>> {
        self.accelerators.iter().copied().map(Some).chain([None]).collect()
    }
}

/// Why QEMU refused to start, when the reason is the accelerator.
///
/// An unentitled build reports the missing vGIC rather than the missing
/// entitlement, because upstream QEMU ignores `hv_vm_create`'s HV_DENIED — so
/// both spellings have to be recognised.
fn refused(output: &str) -> Option<&'static str> {
    if output.contains("error creating platform VGIC") || output.contains("HV_NO_DEVICE") {
        Some("no in-kernel vGIC; the build may not carry com.apple.security.hypervisor")
    } else if output.contains("Could not access HVF")
        || output.contains("failed to initialize KVM")
        || output.contains("Could not access KVM kernel module")
    {
        Some("no hypervisor")
    } else if output.contains("invalid accelerator") {
        // A rung this QEMU was not built with: degrade to the next rung
        // instead of reporting a missing rootfs.
        Some("this QEMU was not built with that accelerator")
    } else {
        None
    }
}

/// The QEMU command line for a provisioning boot.
///
/// `argv[0]` is the program name the in-process QEMU expects, matching
/// [`crate::config::QemuConfig::build_argv`].
fn provisioning_argv(
    accel: Option<Accelerator>,
    kernel: &Path,
    initramfs: &Path,
    scratch: &Path,
    ssh: bool,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "cdj3k-emu-qemu".into(),
        "-machine".into(),
        if accel.is_some_and(|a| a.in_kernel_gic) {
            "virt,gic-version=3,kernel-irqchip=on"
        } else {
            "virt,gic-version=3,kernel-irqchip=off"
        }
        .into(),
        "-m".into(),
        "2G".into(),
        // Six, to match the topology the player boots with: a patch step can
        // branch on the core count, and this rootfs is the one that will run.
        "-smp".into(),
        "6".into(),
        "-nographic".into(),
        "-no-reboot".into(),
        // No network at all. Left to itself the `virt` machine creates a
        // default virtio-net-pci, which wants a PXE romfile the bundled QEMU
        // does not carry — and this guest has nothing to talk to.
        "-nic".into(),
        "none".into(),
    ];
    match accel {
        Some(a) => args.extend(["-accel".into(), a.name.into(), "-cpu".into(), "host".into()]),
        None => args.extend(["-cpu".into(), "cortex-a72".into()]),
    }
    args.extend([
        "-kernel".into(),
        kernel.display().to_string(),
        "-initrd".into(),
        initramfs.display().to_string(),
        // MMIO, not PCI: `if=virtio` builds a virtio-blk-pci that wants a PXE
        // romfile the bundled QEMU does not carry. The player attaches its
        // disks the same way.
        //
        // The guest sees this as mmcblk1, not vda: the kernel renames
        // virtio-blk so the firmware finds its eMMC where it expects one.
        "-drive".into(),
        format!("file={},format=raw,if=none,id=handback", scratch.display()),
        "-device".into(),
        "virtio-blk-device,drive=handback".into(),
        "-append".into(),
        format!(
            "root=/dev/ram0 rdinit=/cdj3k-init console=ttyAMA0 loglevel=4 panic=5{}",
            if ssh { " cdj3k.ssh=1" } else { "" }
        ),
    ]);
    args
}

/// Run QEMU to completion and return everything it printed.
fn spawn_qemu(argv: &[String]) -> Result<String, String> {
    let mut cmd = crate::qemu_exec::qemu_command(None)
        .map_err(|e| format!("locating QEMU: {e}"))?
        .with_argv(argv);

    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start the provisioning guest: {e}"))?;

    let started = std::time::Instant::now();
    let mut child = child;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if started.elapsed() > TIMEOUT {
                    let _ = child.kill();
                    return Err(format!(
                        "the provisioning guest did not finish within {}s",
                        TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => return Err(format!("waiting for the provisioning guest: {e}")),
        }
    }

    let out = child
        .wait_with_output()
        .map_err(|e| format!("reading the provisioning guest's output: {e}"))?;
    Ok(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

fn tail(s: &str, lines: usize) -> String {
    let all: Vec<&str> = s.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HVF_GIC: Option<Accelerator> = Some(Accelerator {
        name: "hvf",
        in_kernel_gic: true,
    });
    const HVF: Option<Accelerator> = Some(Accelerator {
        name: "hvf",
        in_kernel_gic: false,
    });
    const TCG: Option<Accelerator> = None;

    fn argv(accel: Option<Accelerator>) -> Vec<String> {
        provisioning_argv(
            accel,
            Path::new("/k/Image"),
            Path::new("/k/provision.cpio.gz"),
            Path::new("/k/scratch.img"),
            false,
        )
    }

    #[test]
    fn the_guest_is_pointed_at_our_init_not_the_firmwares() {
        let a = argv(HVF).join(" ");
        assert!(a.contains("rdinit=/cdj3k-init"), "{a}");
        assert!(
            a.contains("-no-reboot"),
            "a guest that reboots never returns"
        );
    }

    #[test]
    fn the_scratch_disk_is_attached_over_mmio_not_pci() {
        let a = argv(HVF).join(" ");
        assert!(
            a.contains("file=/k/scratch.img,format=raw,if=none,id=handback"),
            "{a}"
        );
        // virtio-blk-pci wants a romfile the bundled QEMU has no copy of.
        assert!(a.contains("virtio-blk-device,drive=handback"), "{a}");
    }

    #[test]
    fn only_an_in_kernel_gic_rung_asks_for_one() {
        assert!(argv(HVF_GIC).join(" ").contains("kernel-irqchip=on"));
        for lower in [HVF, TCG] {
            assert!(argv(lower).join(" ").contains("kernel-irqchip=off"));
        }
    }

    #[test]
    fn only_the_bottom_rung_gives_up_the_hypervisor() {
        for accelerated in [HVF_GIC, HVF] {
            assert!(argv(accelerated).contains(&"hvf".to_string()));
        }
        assert!(!argv(TCG).contains(&"-accel".to_string()));
        assert!(argv(TCG).contains(&"cortex-a72".to_string()));
    }

    #[test]
    fn the_ladder_is_the_hosts_rungs_then_software_emulation() {
        let runner = QemuGuestRunner {
            accelerators: vec![HVF_GIC.unwrap(), HVF.unwrap()],
            ssh: false,
        };
        assert_eq!(runner.ladder(), vec![HVF_GIC, HVF, TCG]);
        let none = QemuGuestRunner {
            accelerators: Vec::new(),
            ssh: false,
        };
        assert_eq!(none.ladder(), vec![TCG]);
    }

    #[test]
    fn both_spellings_of_a_missing_entitlement_are_recognised() {
        // Upstream QEMU ignores hv_vm_create's HV_DENIED, so an unentitled
        // build reports the vGIC rather than the entitlement.
        assert!(refused("cdj3k-emu-qemu: -accel hvf: error creating platform VGIC").is_some());
        assert!(refused("Error: ret = HV_NO_DEVICE (0xfae94006)").is_some());
        assert!(refused("-accel hvf: Could not access HVF. Is the executable signed").is_some());
        // A rung this QEMU was not built with.
        assert!(
            refused("qemu-system-aarch64: -accel hvf: invalid accelerator hvf").is_some(),
            "an accelerator this build lacks must degrade, not fail the install"
        );
        assert!(refused("Could not access KVM kernel module: No such file").is_some());
        assert!(refused("=== cdj3k: handed back rc=0 ===").is_none());
    }

    #[test]
    fn argv0_is_the_program_name_the_in_process_qemu_expects() {
        assert_eq!(argv(HVF)[0], "cdj3k-emu-qemu");
    }

    /// SSH is asked for on the kernel command line, and only when chosen.
    #[test]
    fn ssh_rides_the_command_line_only_when_asked() {
        let (k, i, s) = (Path::new("/k/Image"), Path::new("/k/p.cpio.gz"), Path::new("/k/s.img"));
        assert!(!provisioning_argv(TCG, k, i, s, false).join(" ").contains("cdj3k.ssh"));
        let on = provisioning_argv(TCG, k, i, s, true);
        let append = on.iter().skip_while(|a| *a != "-append").nth(1).unwrap();
        assert!(append.ends_with(" cdj3k.ssh=1"), "{append}");
    }

    #[test]
    fn nothing_the_player_needs_is_attached() {
        let a = argv(HVF).join(" ");
        assert!(
            a.contains("-nic none"),
            "the default NIC must be refused: {a}"
        );
        for unwanted in ["-display", "-audiodev", "-netdev", "-qmp", "-chardev"] {
            assert!(!a.contains(unwanted), "{unwanted} has no place here: {a}");
        }
    }
}
