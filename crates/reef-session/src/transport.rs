//! Common transport abstraction.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Type of transport in use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportType {
    Ssh,
    Telnet,
    Serial,
}

/// Every device transport implements this trait so the session layer is
/// transport-agnostic.
///
/// Phase 0 stubs do not require Send + Sync; real tokio-based transports will
/// add those bounds when implemented.
pub trait DeviceTransport {
    async fn connect(&mut self) -> Result<(), super::Error>;
    async fn disconnect(&mut self) -> Result<(), super::Error>;
    async fn send(&mut self, data: &str) -> Result<(), super::Error>;
    async fn send_raw(&mut self, bytes: &[u8]) -> Result<(), super::Error>;
    async fn read_until(&mut self, pattern: &str, timeout: Duration) -> Result<String, super::Error>;
    async fn read_available(&mut self, timeout: Duration) -> Result<String, super::Error>;
    fn is_connected(&self) -> bool;
    fn transport_type(&self) -> TransportType;
}
