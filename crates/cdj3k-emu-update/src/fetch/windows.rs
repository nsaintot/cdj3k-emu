//! SChannel through native-tls, with the system's own roots.

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};

pub(super) fn config() -> TlsConfig {
    TlsConfig::builder()
        .provider(TlsProvider::NativeTls)
        .root_certs(RootCerts::PlatformVerifier)
        .build()
}
