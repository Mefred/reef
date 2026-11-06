//! Connection configuration for device sessions.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

use crate::transport::TransportType;
use directories::ProjectDirs;

/// Policy for handling SSH host keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostKeyPolicy {
    /// Automatically accept any host key (first use or changed).
    Accept,
    /// Connect but log a warning on unknown/changed host keys.
    Warn,
    /// Reject unknown or changed host keys.
    Reject,
}

impl Default for HostKeyPolicy {
    fn default() -> Self {
        HostKeyPolicy::Warn
    }
}

/// Configuration for connecting to a single device.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// Device hostname or IP address.
    pub host: String,
    /// TCP port (SSH/Telnet). Ignored for serial.
    pub port: u16,
    /// Transport type.
    pub transport: TransportType,
    /// Login username.
    pub username: String,
    /// Login password (optional; may be prompted).
    pub password: Option<String>,
    /// Enable/privilege escalation secret (optional).
    pub enable_secret: Option<String>,
    /// SSH host key verification policy.
    pub host_key_policy: HostKeyPolicy,
    /// Overall command timeout.
    pub timeout: Duration,
    /// Serial port path (e.g. "/dev/ttyUSB0", "COM1"). Used for serial transport.
    pub serial_port: Option<PathBuf>,
    /// Serial baud rate. Default 9600.
    pub serial_baud: u32,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        ConnectionConfig {
            host: String::new(),
            port: 22,
            transport: TransportType::Ssh,
            username: String::new(),
            password: None,
            enable_secret: None,
            host_key_policy: HostKeyPolicy::default(),
            timeout: Duration::from_secs(10),
            serial_port: None,
            serial_baud: 9600,
        }
    }
}

impl ConnectionConfig {
    /// Build an SSH connection config.
    pub fn ssh(host: impl Into<String>, username: impl Into<String>) -> Self {
        ConnectionConfig {
            host: host.into(),
            username: username.into(),
            transport: TransportType::Ssh,
            ..Default::default()
        }
    }

    /// Build a Telnet connection config.
    pub fn telnet(host: impl Into<String>, username: impl Into<String>) -> Self {
        ConnectionConfig {
            host: host.into(),
            username: username.into(),
            transport: TransportType::Telnet,
            port: 23,
            ..Default::default()
        }
    }

    /// Build a serial connection config.
    pub fn serial(port: impl Into<PathBuf>) -> Self {
        ConnectionConfig {
            transport: TransportType::Serial,
            serial_port: Some(port.into()),
            ..Default::default()
        }
    }

    /// Path to the known_hosts file used for SSH host key verification.
    pub fn known_hosts_path() -> PathBuf {
        ProjectDirs::from("", "", "reef")
            .map(|p| p.config_dir().to_path_buf())
            .unwrap_or_else(|| std::env::temp_dir())
            .join("known_hosts")
    }
}
