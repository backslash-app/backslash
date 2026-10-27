//! HTTP client settings shared by everything that downloads.

use std::time::Duration;

/// An agent that trusts the system's certificate store, so it also works
/// behind proxies whose authority the user installed.
pub fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
        .build()
        .new_agent()
}
