use std::path::PathBuf;
use std::time::Duration;

use reef_session::{ConnectionConfig, HostKeyPolicy, TransportType};
use serde::Deserialize;
use tauri::State;

use crate::session_actor::{ConfigurationDraft, ConfigurationPreview, SessionActor};

/// Frontend-safe connection input. Credentials are intentionally transient
/// and are never copied into the saved profile model.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub device_id: String,
    pub host: String,
    pub port: u16,
    pub transport: TransportType,
    pub username: String,
    pub password: Option<String>,
    pub enable_secret: Option<String>,
    pub serial_port: Option<String>,
    pub serial_baud: Option<u32>,
    pub accept_unknown_host_key: Option<bool>,
}

impl ConnectRequest {
    fn into_config(self) -> Result<ConnectionConfig, String> {
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
            host_key_policy: if self.accept_unknown_host_key.unwrap_or(false) {
                HostKeyPolicy::Accept
            } else {
                HostKeyPolicy::Reject
            },
            timeout: Duration::from_secs(10),
            serial_port: self.serial_port.map(PathBuf::from),
            serial_baud: self.serial_baud.unwrap_or(9600),
        })
    }
}

#[tauri::command]
pub async fn connect_device(
    actor: State<'_, SessionActor>,
    request: ConnectRequest,
) -> Result<(), String> {
    let device_id = request.device_id.clone();
    let config = request.into_config()?;
    actor.connect(device_id, config).await
}

#[tauri::command]
pub async fn disconnect_device(
    actor: State<'_, SessionActor>,
    device_id: String,
) -> Result<(), String> {
    actor.disconnect(device_id).await
}

#[tauri::command]
pub async fn send_device_command(
    actor: State<'_, SessionActor>,
    device_id: String,
    command: String,
) -> Result<String, String> {
    validate_device_command(&command)?;
    actor.send_command(device_id, command).await
}

/// Keep one frontend invocation to one bounded CLI command. The terminal
/// enforces the same limit for normal input, but the Tauri boundary must also
/// reject oversized or multi-line payloads from any caller.
fn validate_device_command(command: &str) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err("command cannot be empty".to_string());
    }
    if command.len() > 4096 {
        return Err("command exceeds the 4096-byte limit".to_string());
    }
    if command.chars().any(char::is_control) {
        return Err("command cannot contain control characters or line breaks".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn device_is_connected(
    actor: State<'_, SessionActor>,
    device_id: String,
) -> Result<bool, String> {
    actor.is_connected(device_id).await
}

#[tauri::command]
pub async fn identify_device(
    actor: State<'_, SessionActor>,
    device_id: String,
) -> Result<reef_engine::model::DeviceIdentity, String> {
    actor.identify(device_id).await
}

#[tauri::command]
pub async fn inspect_device_workspace(
    actor: State<'_, SessionActor>,
    device_id: String,
) -> Result<crate::session_actor::DeviceWorkspace, String> {
    actor.inspect(device_id).await
}

#[tauri::command]
pub async fn get_device_configuration(
    actor: State<'_, SessionActor>,
    device_id: String,
) -> Result<crate::session_actor::ConfigurationSnapshot, String> {
    actor.get_configuration(device_id).await
}

#[tauri::command]
pub async fn enable_device(
    actor: State<'_, SessionActor>,
    device_id: String,
    secret: String,
) -> Result<(), String> {
    actor.enable(device_id, secret).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(transport: TransportType, host: &str) -> ConnectRequest {
        ConnectRequest {
            device_id: "test-device".to_string(),
            host: host.to_string(),
            port: 22,
            transport,
            username: "admin".to_string(),
            password: None,
            enable_secret: None,
            serial_port: None,
            serial_baud: None,
            accept_unknown_host_key: None,
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
        assert!(validate_device_command("show version\\nreload").is_err());
        assert!(validate_device_command("show\\u{0000}version").is_err());
        assert!(validate_device_command(&"x".repeat(4097)).is_err());
        assert!(validate_device_command(&"x".repeat(4096)).is_ok());
    }
}

#[tauri::command]
pub async fn preview_device_configuration(
    actor: State<'_, SessionActor>,
    device_id: String,
    draft: ConfigurationDraft,
) -> Result<ConfigurationPreview, String> {
    actor.preview_configuration(device_id, draft).await
}
