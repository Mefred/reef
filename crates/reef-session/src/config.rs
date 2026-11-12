//! Connection configuration for device sessions.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

use crate::transport::TransportType;

/// Policy for handling SSH host keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostKeyPolicy {
    /// Trust and persist a first-seen host key. Changed keys are still rejected.
    Accept,
    /// Accept and log a warning for a first-seen key. Changed keys are still rejected.
    Warn,
    /// Reject first-seen and changed host keys.
    Reject,
}

impl Default for HostKeyPolicy {
    fn default() -> Self {
        // Unknown host keys must be explicitly enrolled. A warning-only
        // default would silently trust a first-seen or changed endpoint.
        HostKeyPolicy::Reject
    }
}

/// Configuration for connecting to a single device.
///
/// Secret fields are never serialized and are masked by the manual `Debug`
/// implementation below.
#[derive(Clone, Serialize, Deserialize)]
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
    #[serde(skip_serializing)]
    pub password: Option<String>,
    /// Enable/privilege escalation secret (optional).
    #[serde(skip_serializing)]
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

impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectionConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("transport", &self.transport)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field(
                "enable_secret",
                &self.enable_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("host_key_policy", &self.host_key_policy)
            .field("timeout", &self.timeout)
            .field("serial_port", &self.serial_port)
            .field("serial_baud", &self.serial_baud)
            .finish()
    }
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
    ///
    /// Always USB-relative (`Config/known_hosts`). Verification itself is
    /// never disabled; this only chooses where enrolled keys are stored.
    pub fn known_hosts_path() -> PathBuf {
        reef_logging::paths::known_hosts_path().unwrap_or_else(|_| {
            std::env::temp_dir().join(format!("reef-known_hosts-{}-fallback", std::process::id()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::HostKeyPolicy;

    #[test]
    fn unknown_host_keys_are_rejected_by_default() {
        assert_eq!(HostKeyPolicy::default(), HostKeyPolicy::Reject);
    }
}
