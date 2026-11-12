//! Common transport abstraction.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// Type of transport in use.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportType {
    Ssh,
    Telnet,
    Serial,
}

impl TransportType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransportType::Ssh => "Ssh",
            TransportType::Telnet => "Telnet",
            TransportType::Serial => "Serial",
        }
    }
}

/// Non-secret connection details for diagnostics.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportMetadata {
    pub transport: String,
    /// Negotiated algorithms, host-key fingerprint, serial settings, and other
    /// non-secret facts. Never contains passwords or private keys.
    pub details: BTreeMap<String, String>,
}

impl TransportMetadata {
    pub fn new(transport: TransportType) -> Self {
        TransportMetadata {
            transport: transport.as_str().to_string(),
            details: BTreeMap::new(),
        }
    }

    pub fn with(mut self, key: &str, value: impl Into<String>) -> Self {
        self.details.insert(key.to_string(), value.into());
        self
    }
}

/// Every device transport implements this trait so the session layer is
/// transport-agnostic.
///
/// Note: `Send + Sync` is NOT required here because `ssh2::Session` is not
/// `Send`. Reef runs device I/O on a single thread (the GUI thread), so
/// `Send` is not needed.
pub trait DeviceTransport {
    async fn connect(&mut self) -> Result<(), super::Error>;
    async fn disconnect(&mut self) -> Result<(), super::Error>;
    async fn send(&mut self, data: &str) -> Result<(), super::Error>;
    async fn send_raw(&mut self, bytes: &[u8]) -> Result<(), super::Error>;
    async fn read_until(
        &mut self,
        pattern: &str,
        timeout: Duration,
    ) -> Result<String, super::Error>;
    async fn read_available(&mut self, timeout: Duration) -> Result<String, super::Error>;
    fn is_connected(&self) -> bool;
    fn transport_type(&self) -> TransportType;

    /// Non-secret connection metadata. Implementations may enrich this after
    /// a successful connect.
    fn metadata(&self) -> TransportMetadata {
        TransportMetadata::new(self.transport_type())
    }
}
