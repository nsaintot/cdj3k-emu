//! End-to-end guest provisioning, with QEMU driven by a plain subprocess.
//!
//! Usage: <initramfs.cpio.gz> <resources-dir> <kernel> <out.cpio.gz>

use std::path::Path;

use cdj3k_emu_firmware::{patch_initramfs_in_guest, GuestProvision, GuestRunner};

struct Qemu;

impl GuestRunner for Qemu {
    fn run(&self, kernel: &Path, initramfs: &Path, scratch: &Path) -> Result<(), String> {
        let out = std::process::Command::new("qemu-system-aarch64")
            .args(["-M", "virt", "-cpu", "cortex-a72", "-smp", "4", "-m", "2G"])
            .args(["-nographic", "-no-reboot"])
            .arg("-kernel")
            .arg(kernel)
            .arg("-initrd")
            .arg(initramfs)
            .arg("-drive")
            .arg(format!("file={},format=raw,if=virtio", scratch.display()))
            .args([
                "-append",
                "root=/dev/ram0 rdinit=/cdj3k-init console=ttyAMA0 loglevel=4 panic=5",
            ])
            .output()
            .map_err(|e| format!("qemu: {e}"))?;
        let log = String::from_utf8_lossy(&out.stdout);
        cdj3k_emu_firmware::initramfs_guest::guest_outcome(&log).map_err(String::from)
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [src, resources, kernel, out] = a.as_slice() else {
        eprintln!("usage: <initramfs.cpio.gz> <resources-dir> <kernel> <out.cpio.gz>");
        std::process::exit(2);
    };

    let scratch = std::path::PathBuf::from(out).with_extension("scratch.img");
    let size = cdj3k_emu_firmware::initramfs_guest::scratch_size_for(Path::new(src))
        .expect("size the scratch file");
    let f = std::fs::File::create(&scratch).expect("create scratch");
    f.set_len(size).expect("size scratch");
    drop(f);

    match patch_initramfs_in_guest(
        Path::new(src),
        Path::new(resources),
        Path::new(out),
        &GuestProvision {
            kernel: Path::new(kernel),
            scratch: &scratch,
            runner: &Qemu,
        },
    ) {
        Ok(()) => {
            if std::env::var_os("CDJ3K_KEEP_SCRATCH").is_none() {
                let _ = std::fs::remove_file(&scratch);
            }
            println!("provisioned {out}");
        }
        Err(e) => {
            eprintln!("FAILED: {e}");
            std::process::exit(1);
        }
    }
}
