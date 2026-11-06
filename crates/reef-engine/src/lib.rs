//! Reef configuration engine for Cisco IOS XE.
//!
//! This crate is intentionally pure: it depends on neither the GUI (Slint) nor
//! any transport (SSH/Telnet/Serial). It owns the structured configuration
//! data models, the parser (CLI output -> model), the generator (model ->
//! IOS XE commands), the validator, the planner/diff, and the verifier.
//!
//! All configuration logic lives here so it can be unit-tested headlessly.

pub mod diff;
pub mod generator;
pub mod model;
pub mod parser;
pub mod planner;
pub mod validator;
pub mod verifier;

// Re-export the top-level types for convenience.
pub use model::{DeviceCapabilities, DeviceConfig, DeviceIdentity, InterfaceConfig};
pub use planner::ChangeSet;
pub use verifier::VerificationResult;
