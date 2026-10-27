//! Everything in Backslash that does not draw pixels: the extension protocol
//! and host, manifests, search ranking, the calculator and configuration.
//! Kept free of GTK so it is unit-testable and reusable by the CLI.

pub mod approvals;
pub mod calc;
pub mod catalog;
pub mod config;
pub mod host;
pub mod http;
pub mod manifest;
pub mod paths;
pub mod protocol;
pub mod registry;
pub mod runner;
pub mod runtime;
pub mod search;
pub mod secrets;
pub mod store;
pub mod system;
pub mod units;

/// Re-exported so the launcher writes config values with the same version.
pub use toml;
