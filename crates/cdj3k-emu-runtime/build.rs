use std::path::PathBuf;

fn main() {
    // Only link on macOS - the dylib is HVF+Cocoa specific.
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "macos" {
        return;
    }

    // Allow override via env for CI / bundle builds.
    let lib_dir: PathBuf = std::env::var("CDJ3K_EMU_QEMU_LIB_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // Default: relative to workspace root (qemu/install/lib).
            // CARGO_MANIFEST_DIR = crates/cdj3k-emu-runtime
            let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
            manifest.join("../../qemu/install/lib")
        });

    let lib_dir = lib_dir.canonicalize().unwrap_or(lib_dir);

    println!("cargo:rustc-link-search={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=cdj3k-emu-qemu");
    // Embed an rpath so the binary finds the dylib without DYLD_LIBRARY_PATH.
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());

    println!("cargo:rerun-if-env-changed=CDJ3K_EMU_QEMU_LIB_DIR");
    println!(
        "cargo:rerun-if-changed={}",
        lib_dir.join("libcdj3k-emu-qemu.dylib").display()
    );
}
