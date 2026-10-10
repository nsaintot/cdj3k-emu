/// The macOS SDK version recorded in the binary. AppKit chooses its behaviour
/// by this version. With the macOS 27 SDK, the panel's frames reach the screen
/// unevenly and the jog rotation stutters.
const MACOS_SDK_VERSION: &str = "15.5";

fn main() {
    // Embed @executable_path as an rpath, so that libcdj3k-emu-qemu.dylib is
    // found next to the binary inside the app bundle.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path");

        // The minimum macOS version is the deployment target when one is set,
        // and otherwise rustc's default for the architecture.
        println!("cargo:rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");
        let min =
            std::env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| {
                match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
                    Ok("x86_64") => "10.12".into(),
                    _ => "11.0".into(),
                }
            });
        println!("cargo:rustc-link-arg=-Wl,-platform_version,macos,{min},{MACOS_SDK_VERSION}");
    }
}
