//! Device session and transport management for Reef.
//!
//! This crate owns the `DeviceTransport` trait and the SSH/Telnet/Serial
//! implementations. It is transport-agnostic at the session layer.

mod apply;
mod config;
mod error;
mod manager;
mod prompt;
mod serial;
mod session;
mod ssh;
mod telnet;
mod transport;

pub use apply::ApplyOrchestrator;
pub use apply::ApplyResult;

// Re-export public API.
pub use config::{ConnectionConfig, HostKeyPolicy};
pub use error::Error;
pub use manager::SessionManager;
pub use prompt::{detect, PromptMode};
pub use serial::SerialTransport;
pub use session::Session;
pub use ssh::{HostKeyStatus, SshTransport};
pub use telnet::TelnetTransport;
pub use transport::TransportType;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert!(true);
    }
}
