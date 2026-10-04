//! rustls, verifying through the host's trust store.

use ureq::tls::{RootCerts, TlsConfig};

pub(super) fn config() -> TlsConfig {
    TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .build()
}
