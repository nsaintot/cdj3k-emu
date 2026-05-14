//! Resolve helper-tool paths relative to the running executable.
//!
//! `bundle.sh` drops `qemu-img`, `socket_vmnet`, and friends into
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
            let candidate = dir.join(name);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    PathBuf::from(name)
}
