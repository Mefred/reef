//! Device session and transport management for Reef.
//!
//! This crate owns the `DeviceTransport` trait and the SSH/Telnet/Serial
//! implementations. It is transport-agnostic at the session layer.

mod config;
mod error;
mod prompt;
mod ssh;
mod telnet;
mod serial;
mod transport;
mod session;
mod manager;
mod apply;

pub use apply::ApplyOrchestrator;
pub use apply::ApplyResult;

// Re-export public API.
pub use config::{ConnectionConfig, HostKeyPolicy};
pub use error::Error;
pub use prompt::{detect, PromptMode};
pub use ssh::SshTransport;
pub use telnet::TelnetTransport;
pub use serial::SerialTransport;
pub use session::Session;
pub use manager::SessionManager;
pub use transport::TransportType;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert!(true);
    }
}
