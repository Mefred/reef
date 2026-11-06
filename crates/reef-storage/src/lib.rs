//! Persistent storage for Reef: device profiles, saved configs, change history.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A saved device connection profile.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub transport: TransportType,
    pub username: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TransportType {
    Ssh,
    Telnet,
    Serial,
}

/// Errors for the storage layer.
#[derive(Debug, Error)]
pub enum Error {
    #[error("storage error: {0}")]
    Io(String),
    #[error("not found: {0}")]
    NotFound(String),
}

pub struct Storage;

impl Storage {
    pub fn new() -> Self {
        Storage
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_creates() {
        let _ = Storage::new();
        assert!(true);
    }
}
