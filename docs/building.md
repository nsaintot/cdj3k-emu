# Building

This page covers building the emulator and its packages from source on each
host. CI builds the releases; see [Releases and updates](releases.md).

## Every host

```bash
git clone https://github.com/nsaintot/cdj3k-emu
cd cdj3k-emu

# 1. Fetch the winit release from crates.io and apply winit/patches/.
./winit/fetch.sh

# 2. Build QEMU for this host. The script clones upstream at the pinned
#    commit, applies qemu/patches/, and builds with HVF and Cocoa on macOS or
#    with KVM and PipeWire on Linux (about 10 minutes). For Windows, see
#    --windows below.
./qemu/build.sh

# 3. Build the guest payload in Docker: the aarch64 kernel, the out-of-tree
#    modules, the shim and the guest tools. `make abi-check` checks the shim
#    against the deck's glibc.
./build.sh
```

`./winit/fetch.sh` has to run once before **any** cargo command, including a
plain `cargo build` and rust-analyzer: `Cargo.toml` patches winit from
`winit/src`, which the script makes, and Cargo resolves that path before it
builds anything. Re-running it is a no-op until the version or a patch
changes.

## macOS

```bash
# Stage dist/macos/CDJ3K Emulator.app. The Homebrew dylibs that QEMU links
# against are copied into the bundle and rewritten to @loader_path, so the app
# runs on a Mac without Homebrew.
packaging/macos/stage.sh                                  # development build for this Mac's architecture, from qemu/install/

# Build the universal .app, which holds arm64 and x86_64. Each architecture
# gets its own QEMU, built against static libraries from qemu/macos-deps.sh at
# the macOS 15.0 deployment target, and lipo merges the two slices. It builds
# on either kind of Mac and needs
# `rustup target add aarch64-apple-darwin x86_64-apple-darwin`.
./qemu/build.sh --macos arm64
./qemu/build.sh --macos x86_64
packaging/macos/stage.sh --arch universal

# Build only the arm64 slice, for Apple silicon.
./qemu/build.sh --macos arm64
packaging/macos/stage.sh --arch arm64

# Sign the staged .app into dist/, optionally with a .dmg.
packaging/macos/build.sh                                  # ad-hoc signed (HVF works, FDA does not)
packaging/macos/build.sh --sign "Apple Development"       # real cert (enables Full Disk Access)
packaging/macos/build.sh --sign "Developer ID Application" --dmg

# A release build: Developer ID, notarized and stapled .app and .dmg. Store the
# notary credentials (an app-specific password from appleid.apple.com) once:
xcrun notarytool store-credentials cdj3k-emu-notarization --apple-id <APPLE_ID> --team-id <TEAM_ID>
packaging/macos/build.sh --sign "Developer ID Application" --dmg --notarize
```

## Linux

```bash
# Stage one tree for this architecture, laid out as /opt/cdj3k-emu: the app,
# QEMU, qemu-img, the vendored libraries ($ORIGIN) and the guest payload.
packaging/linux/stage.sh

# Build the .deb, the .rpm (with nfpm) and the AppImage from that tree.
packaging/linux/build.sh
```

`docker/Dockerfile.linux-pkg` carries the packaging tools and sets a bookworm
glibc floor. `docker/Dockerfile.linux-host` carries the QEMU build
dependencies. `stage.sh --qemu DIR` uses a QEMU built elsewhere (`DIR/bin/`).

## Windows

```bash
# Build QEMU for Windows into qemu/install-windows-<arch>/, with every DLL it
# loads (scripts/bundle-dlls.sh). From Linux or macOS it cross-builds in the
# docker/Dockerfile.windows-qemu image (llvm-mingw); in an MSYS2 UCRT64
# (x86_64) or CLANGARM64 (aarch64) shell it builds natively.
./qemu/build.sh --windows x86_64|aarch64

# Build the app for <arch>-pc-windows-gnullvm in the same image, with its
# DLLs, into dist/windows-bin-<arch>/.
packaging/windows/cross-build.sh x64|arm64
```

```powershell
# On Windows, stage the tree (stage.ps1) and compile the Inno Setup installer
# into dist\CDJ3K-Emulator-<version>-windows-<arch>.exe.
packaging\windows\build.ps1 -Arch x64 -QemuDir qemu\install-windows-x86_64
```

`build.ps1` needs Inno Setup 6.6 or newer. `stage.ps1` needs Git for Windows,
whose bash builds the patch dispatcher. The installer is Authenticode-signed when
`CDJ3K_SIGN_CERT_SHA1` holds a certificate thumbprint.
