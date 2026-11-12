//! Pure validation and command-guarding helpers shared by the native UI.
//!
//! Keeping these rules outside the UI makes them testable without a window or
//! a device session and guarantees the native UI and the session service
//! enforce the same limits.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use reef_session::{ConnectionConfig, HostKeyPolicy, TransportType};
use reef_storage::{DeviceProfile, TransportType as ProfileTransport};

/// A single interactive command may not exceed this many bytes.
pub const MAX_COMMAND_BYTES: usize = 4096;
/// Visible command output is bounded to this many bytes before display.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const TRUNCATION_NOTICE: &str = "\r\n[output truncated at 1 MiB]\r\n";

/// Frontend-safe connection input. Credentials are transient and are never
/// copied into the saved profile model.
#[derive(Debug, Clone)]
pub struct ConnectionRequest {
    pub device_id: String,
    pub host: String,
    pub port: u16,
    pub transport: TransportType,
    pub username: String,
    pub password: Option<String>,
    pub enable_secret: Option<String>,
    pub serial_port: Option<String>,
    pub serial_baud: Option<u32>,
    pub accept_unknown_host_key: bool,
}

impl ConnectionRequest {
    pub fn into_config(self) -> Result<ConnectionConfig, String> {
        if self.device_id.trim().is_empty() {
            return Err("device id cannot be empty".to_string());
        }
        if self.transport != TransportType::Serial && self.host.trim().is_empty() {
            return Err("host cannot be empty".to_string());
        }
        if self.transport == TransportType::Serial
            && self
                .serial_port
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err("serial port cannot be empty".to_string());
        }
        if self.transport != TransportType::Serial && self.port == 0 {
            return Err("port must be between 1 and 65535".to_string());
        }

        Ok(ConnectionConfig {
            host: self.host.trim().to_string(),
            port: self.port,
            transport: self.transport,
            username: self.username.trim().to_string(),
            password: self.password,
            enable_secret: self.enable_secret,
            host_key_policy: if self.accept_unknown_host_key {
                HostKeyPolicy::Accept
            } else {
                HostKeyPolicy::Reject
            },
            timeout: std::time::Duration::from_secs(10),
            serial_port: self.serial_port.map(PathBuf::from),
            serial_baud: self.serial_baud.unwrap_or(9600),
        })
    }
}

/// Validate profile form fields and build a profile. Passwords and enable
/// secrets are deliberately not part of this model.
pub fn build_profile(
    name: &str,
    host: &str,
    port: &str,
    transport: ProfileTransport,
    username: &str,
) -> Result<DeviceProfile, String> {
    let name = name.trim();
    let host = host.trim();
    let username = username.trim();

    if name.is_empty() {
        return Err("profile name cannot be empty".to_string());
    }
    if name.len() > 80 {
        return Err("profile name exceeds the 80-character limit".to_string());
    }
    if host.is_empty() {
        return Err("host or serial port cannot be empty".to_string());
    }
    if username.is_empty() && transport != ProfileTransport::Serial {
        return Err("username cannot be empty".to_string());
    }

    let port = if transport == ProfileTransport::Serial {
        0
    } else {
        let port = port
            .trim()
            .parse::<u16>()
            .map_err(|_| "port must be a number between 1 and 65535".to_string())?;
        if port == 0 {
            return Err("port must be between 1 and 65535".to_string());
        }
        port
    };

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();

    Ok(DeviceProfile {
        id: format!("profile-{timestamp}-{}", std::process::id()),
        name: name.to_string(),
        host: host.to_string(),
        port,
        transport,
        username: username.to_string(),
    })
}

/// Reject empty, oversized, or multi-line interactive commands.
pub fn validate_device_command(command: &str) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err("command cannot be empty".to_string());
    }
    if command.len() > MAX_COMMAND_BYTES {
        return Err(format!(
            "command exceeds the {MAX_COMMAND_BYTES}-byte limit"
        ));
    }
    if command.chars().any(char::is_control) {
        return Err("command cannot contain control characters or line breaks".to_string());
    }
    Ok(())
}

/// Bound data returned to the UI so a single command cannot flood terminal
/// scrollback or the renderer with an arbitrarily large response.
pub fn bound_command_output(mut output: String) -> String {
    if output.len() > MAX_OUTPUT_BYTES {
        let mut end = MAX_OUTPUT_BYTES;
        while !output.is_char_boundary(end) {
            end -= 1;
        }
        output.truncate(end);
        output.push_str(TRUNCATION_NOTICE);
    }
    output
}

/// Split the free-form name-server field on commas and whitespace.
pub fn split_name_servers(value: &str) -> Vec<String> {
    value
        .split(|character: char| character.is_whitespace() || character == ',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(transport: TransportType, host: &str) -> ConnectionRequest {
        ConnectionRequest {
            device_id: "test-device".to_string(),
            host: host.to_string(),
            port: 22,
            transport,
            username: "admin".to_string(),
            password: None,
            enable_secret: None,
            serial_port: None,
            serial_baud: None,
            accept_unknown_host_key: false,
        }
    }

    #[test]
    fn ssh_rejects_unknown_host_keys_by_default() {
        let config = request(TransportType::Ssh, "192.0.2.10")
            .into_config()
            .expect("valid SSH request");
        assert_eq!(config.host_key_policy, HostKeyPolicy::Reject);
    }

    #[test]
    fn ssh_can_explicitly_trust_unknown_host_keys() {
        let mut request = request(TransportType::Ssh, "192.0.2.10");
        request.accept_unknown_host_key = true;
        let config = request.into_config().expect("valid SSH request");
        assert_eq!(config.host_key_policy, HostKeyPolicy::Accept);
    }

    #[test]
    fn network_connection_requires_a_host() {
        assert!(request(TransportType::Ssh, " ").into_config().is_err());
    }

    #[test]
    fn serial_connection_requires_a_port_path() {
        assert!(request(TransportType::Serial, "").into_config().is_err());
    }

    #[test]
    fn device_command_must_be_a_single_bounded_line() {
        assert!(validate_device_command("show version").is_ok());
        assert!(validate_device_command("   ").is_err());
        assert!(validate_device_command("show version\nreload").is_err());
        assert!(validate_device_command("show\u{0000}version").is_err());
        assert!(validate_device_command(&"x".repeat(MAX_COMMAND_BYTES + 1)).is_err());
        assert!(validate_device_command(&"x".repeat(MAX_COMMAND_BYTES)).is_ok());
    }

    #[test]
    fn terminal_output_is_bounded_without_splitting_utf8() {
        let prefix = "x".repeat(MAX_OUTPUT_BYTES - 1);
        let output = format!("{prefix}é");
        let bounded = bound_command_output(output);
        assert!(bounded.starts_with(&prefix));
        assert!(bounded.ends_with("\r\n[output truncated at 1 MiB]\r\n"));
        assert!(bounded.is_char_boundary(bounded.len()));
        assert!(!bound_command_output("short output".to_string()).contains("truncated"));
    }

    #[test]
    fn profile_validation_rejects_missing_fields_and_bad_ports() {
        assert!(
            build_profile("Core", "", "22", ProfileTransport::Ssh, "admin").is_err(),
            "host is required"
        );
        assert!(
            build_profile("", "192.0.2.1", "22", ProfileTransport::Ssh, "admin").is_err(),
            "name is required"
        );
        assert!(
            build_profile("Core", "192.0.2.1", "0", ProfileTransport::Ssh, "admin").is_err(),
            "network port zero is invalid"
        );
        assert!(
            build_profile(
                "Core",
                "192.0.2.1",
                "not-a-port",
                ProfileTransport::Ssh,
                "admin"
            )
            .is_err(),
            "non-numeric port is invalid"
        );
        assert!(
            build_profile("Core", "192.0.2.1", "22", ProfileTransport::Ssh, " ").is_err(),
            "network username is required"
        );
    }

    #[test]
    fn serial_profiles_allow_empty_username_and_ignore_port() {
        let profile = build_profile("Console", "COM3", "", ProfileTransport::Serial, "")
            .expect("serial profile should be valid");
        assert_eq!(profile.port, 0);
        assert_eq!(profile.transport, ProfileTransport::Serial);
        assert_eq!(profile.username, "");
    }

    #[test]
    fn name_server_input_is_split_and_trimmed() {
        assert_eq!(
            split_name_servers(" 1.1.1.1, 8.8.8.8\n2001:4860:4860::8888 "),
            vec![
                "1.1.1.1".to_string(),
                "8.8.8.8".to_string(),
                "2001:4860:4860::8888".to_string()
            ]
        );
        assert!(split_name_servers(" , ").is_empty());
    }
}
