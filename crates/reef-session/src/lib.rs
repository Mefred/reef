//! Device session and transport management for Reef.
//!
//! This crate owns the `DeviceTransport` trait and the SSH/Telnet/Serial
//! implementations. It is transport-agnostic at the session layer.

mod transport;
mod session;
mod manager;
mod ssh;
mod telnet;
mod serial;
mod error;

pub use error::Error;

pub use transport::DeviceTransport;
pub use session::Session;
pub use manager::SessionManager;

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder() {
        assert!(true);
    }
}
