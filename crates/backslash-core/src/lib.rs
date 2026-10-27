//! Everything in Backslash that does not draw pixels: the extension protocol
//! and host, manifests, search ranking, the calculator and configuration.
//! Kept free of GTK so it is unit-testable and reusable by the CLI.

pub mod calc;
pub mod config;
pub mod host;
pub mod manifest;
pub mod paths;
pub mod protocol;
pub mod registry;
pub mod runner;
pub mod search;
pub mod store;
