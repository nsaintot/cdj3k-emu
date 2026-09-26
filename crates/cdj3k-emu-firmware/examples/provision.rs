//! Drive the real provisioning entry point.
//!
//! Usage: <initramfs.cpio.gz> <resources-dir> <out.cpio.gz>

use std::path::Path;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let [src, resources, out] = a.as_slice() else {
        eprintln!("usage: <initramfs.cpio.gz> <resources-dir> <out.cpio.gz>");
        std::process::exit(2);
    };
    match cdj3k_emu_firmware::patch_initramfs(Path::new(src), Path::new(resources), Path::new(out))
    {
        Ok(()) => println!("provisioned {out}"),
        Err(e) => {
            eprintln!("FAILED: {e}");
            std::process::exit(1);
        }
    }
}
