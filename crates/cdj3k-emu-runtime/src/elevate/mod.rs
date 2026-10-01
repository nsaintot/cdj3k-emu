//! Running one shell command as root.
//!
//! Unix hosts run `/bin/sh -c <cmd>` as root, only the prompt differing;
//! Windows runs `cmd.exe /C <cmd>` as administrator.
//!
//! Networking is the only caller: creating a bridge, tap or macvtap needs
//! root, and no entitlement covers it.

/// Wrap a string in single quotes for `/bin/sh`, escaping any embedded `'`.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod imp;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod imp;
#[cfg(windows)]
#[path = "windows.rs"]
mod imp;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
#[path = "unsupported.rs"]
mod imp;

pub use imp::run_elevated;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_an_embedded_quote() {
        assert_eq!(sh_quote("plain"), "'plain'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote("a b"), "'a b'");
    }
}
