use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use reef_storage::{DeviceProfile, Storage, TransportType as ProfileTransport};
use serde::Deserialize;
use tauri::State;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProfileRequest {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub transport: ProfileTransport,
    pub username: String,
}

/// Load saved device profiles without blocking the Tauri command executor.
#[tauri::command]
pub async fn list_profiles(storage: State<'_, Arc<Storage>>) -> Result<Vec<DeviceProfile>, String> {
    let storage = Arc::clone(&storage);

    tauri::async_runtime::spawn_blocking(move || {
        storage.list_profiles().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("profile storage task failed: {error}"))?
}

/// Create a profile. Passwords and enable secrets are deliberately not part
/// of this DTO and must be entered for a connection, not saved in profile JSON.
#[tauri::command]
pub async fn save_profile(
    storage: State<'_, Arc<Storage>>,
    request: SaveProfileRequest,
) -> Result<DeviceProfile, String> {
    let name = request.name.trim().to_string();
    let host = request.host.trim().to_string();
    let username = request.username.trim().to_string();

    if name.is_empty() {
        return Err("profile name cannot be empty".to_string());
    }
    if host.is_empty() {
        return Err("host or serial port cannot be empty".to_string());
    }
    if username.is_empty() && request.transport != ProfileTransport::Serial {
        return Err("username cannot be empty".to_string());
    }
    if request.transport != ProfileTransport::Serial && request.port == 0 {
        return Err("port must be between 1 and 65535".to_string());
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let profile = DeviceProfile {
        id: format!("profile-{timestamp}-{}", std::process::id()),
        name,
        host,
        port: request.port,
        transport: request.transport,
        username,
    };

    let storage = Arc::clone(&storage);
    let saved = profile.clone();
    tauri::async_runtime::spawn_blocking(move || {
        storage
            .save_profile(&saved)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("profile storage task failed: {error}"))??;

    Ok(profile)
}

#[tauri::command]
pub async fn delete_profile(
    storage: State<'_, Arc<Storage>>,
    profile_id: String,
) -> Result<(), String> {
    let storage = Arc::clone(&storage);
    tauri::async_runtime::spawn_blocking(move || {
        storage
            .delete_profile(&profile_id)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("profile storage task failed: {error}"))?
}
