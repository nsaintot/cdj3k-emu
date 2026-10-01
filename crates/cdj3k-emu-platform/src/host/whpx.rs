//! Which Windows builds run an arm64 guest under WHPX.
//!
//! Compiled everywhere so the gate is tested everywhere; only the Windows
//! adapter reads a build number.

/// Windows 11 24H2.
pub const MIN_BUILD: u32 = 26100;
/// The April 2025 optional update (KB5055627) of 24H2, the first with the
/// release arm64 WHPX API.
pub const MIN_UBR: u32 = 3915;

/// Whether `build`.`ubr` is at or past the first arm64 WHPX release.
pub fn build_supports_arm64_whpx(build: u32, ubr: u32) -> bool {
    (build, ubr) >= (MIN_BUILD, MIN_UBR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_supported_build_is_the_boundary() {
        assert!(build_supports_arm64_whpx(26100, 3915));
        assert!(!build_supports_arm64_whpx(26100, 3914));
        assert!(!build_supports_arm64_whpx(26100, 0));
    }

    #[test]
    fn later_feature_releases_pass_whatever_their_ubr() {
        assert!(build_supports_arm64_whpx(26200, 0));
    }

    #[test]
    fn windows_10_and_11_23h2_fail() {
        assert!(!build_supports_arm64_whpx(19045, 5000));
        assert!(!build_supports_arm64_whpx(22631, 9999));
    }
}
