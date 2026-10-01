//! Resolve helper-tool paths relative to the running executable.
//!
//! `bundle.sh` drops `qemu-img` and friends into
//! `<app>.app/Contents/MacOS/` alongside the main binary.  At runtime we
//! prefer those bundled copies so end users don't need anything on `$PATH`.

use std::path::PathBuf;

/// Path to a helper tool bundled next to the current executable, falling
/// back to the bare tool name (so `Command::new` consults `$PATH`) when no
/// bundled copy is found.  This keeps dev runs (`cargo run`) working — the
/// fallback hits a Homebrew install — while shipped `.app` bundles always
/// use the embedded binary.
pub fn tool(name: &str) -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(found) = beside(dir, name) {
                return found;
            }
        }
    }
    PathBuf::from(name)
}

/// `name` in `dir`, as this host spells an executable.
///
/// On Windows the bare stem `qemu-img` names the file `qemu-img.exe`.
fn with_exe_suffix(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let suffixed = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    (suffixed != name).then(|| dir.join(suffixed))
}

/// The bundled copy of `name` beside the executable, whatever the host calls
/// it.
fn beside(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let candidate = dir.join(name);
    if candidate.exists() {
        return Some(candidate);
    }
    with_exe_suffix(dir, name).filter(|p| p.exists())
}

/// Where the payload the emulator ships with lives: the guest kernel, the
/// initramfs patch scripts and the guest tools.
///
/// The layout differs per package, so the search runs in order:
///
/// 1. `CDJ3K_RESOURCES`, which a dev run or an AppImage's launcher sets.
/// 2. `<exe>/../Resources`, the macOS `.app`.
/// 3. `<exe>/../share/cdj3k-emu`, the Linux prefix — `/opt/cdj3k-emu/bin`
///    beside `/opt/cdj3k-emu/share/cdj3k-emu`, and the same inside an
///    AppImage's mount.
/// 4. `<exe>/resources`, a developer's hand-copied mirror.
///
/// The last entry is returned unchecked so a caller reports a missing file
/// rather than a missing directory.
pub fn resources() -> PathBuf {
    resources_for(std::env::var_os(RESOURCES_ENV), std::env::current_exe().ok())
}

/// [`resources`] for a given override and executable path.
fn resources_for(over: Option<std::ffi::OsString>, exe: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = over {
        return PathBuf::from(dir);
    }
    let Some(exe) = exe else {
        return PathBuf::from("resources");
    };
    let Some(bin) = exe.parent() else {
        return PathBuf::from("resources");
    };
    let prefix = bin.parent().unwrap_or(bin);
    for candidate in [
        prefix.join("Resources"),
        prefix.join("share").join("cdj3k-emu"),
        bin.join("resources"),
    ] {
        if candidate.is_dir() {
            return candidate;
        }
    }
    bin.join("resources")
}

/// Override for [`resources`]. The AppImage's `AppRun` sets it, because the
/// mount point moves every launch.
pub const RESOURCES_ENV: &str = "CDJ3K_RESOURCES";

/// Find a system tool, including where a desktop session's `PATH` does not
/// look ([`crate::child::EXTRA_TOOL_DIRS`]). Storage tools the app shells out to
/// go through here.
///
/// Returns the bare name when nothing is found, so the caller's error says
/// what is missing rather than where it looked.
pub fn system_tool(name: &str) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::env::split_paths(&path)
        .chain(crate::child::EXTRA_TOOL_DIRS.iter().map(PathBuf::from))
        .collect::<Vec<_>>();
    find_tool(&dirs, name)
}

/// The first runnable `name` in `dirs`, or the bare name.
fn find_tool(dirs: &[PathBuf], name: &str) -> PathBuf {
    if name.contains('/') || std::path::Path::new(name).is_absolute() {
        return PathBuf::from(name);
    }
    dirs.iter()
        .find_map(|dir| executable_in(dir, name))
        .unwrap_or_else(|| PathBuf::from(name))
}

/// `name` as this host spells it in `dir`, when this host would run it.
fn executable_in(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    let candidate = dir.join(name);
    if crate::child::is_runnable(&candidate) {
        return Some(candidate);
    }
    with_exe_suffix(dir, name).filter(|p| crate::child::is_runnable(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cdj3k-bundled-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_override_wins_over_every_layout() {
        let got = resources_for(Some("/somewhere/else".into()), Some("/opt/x/bin/app".into()));
        assert_eq!(got, PathBuf::from("/somewhere/else"));
    }

    #[test]
    fn each_package_layout_is_found_beside_the_executable() {
        let root = fixture("layout");
        let app = root.join("App.app/Contents");
        std::fs::create_dir_all(app.join("MacOS")).unwrap();
        std::fs::create_dir_all(app.join("Resources")).unwrap();
        let opt = root.join("opt");
        std::fs::create_dir_all(opt.join("bin")).unwrap();
        std::fs::create_dir_all(opt.join("share/cdj3k-emu")).unwrap();

        assert_eq!(resources_for(None, Some(app.join("MacOS/app"))), app.join("Resources"));
        assert_eq!(
            resources_for(None, Some(opt.join("bin/app"))),
            opt.join("share/cdj3k-emu")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A tool found only in a later directory is still found.
    #[test]
    fn a_tool_is_found_in_a_later_directory() {
        let root = fixture("tool");
        let (first, later) = (root.join("bin"), root.join("sbin"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&later).unwrap();
        let tool = later.join(format!("mkfs.x{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&tool, b"").unwrap();
        crate::child::set_runnable(&tool).unwrap();

        assert_eq!(find_tool(&[first.clone(), later.clone()], "mkfs.x"), tool);
        assert_eq!(find_tool(&[first], "mkfs.x"), PathBuf::from("mkfs.x"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
