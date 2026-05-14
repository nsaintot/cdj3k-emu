fn main() {
    // Embed @executable_path as an rpath so libcdj3k-emu-qemu.dylib is found
    // next to the binary inside the app bundle - no install_name_tool needed.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");
    }
}
