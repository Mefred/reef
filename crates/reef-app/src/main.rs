//! Reef desktop application entry point.
//!
//! Tauri owns the desktop lifecycle. The session actor keeps the existing
//! thread-affine SessionManager isolated from Tauri's Send + Sync state.

mod commands;
mod session_actor;

use std::sync::Arc;

use reef_logging::init;
use reef_storage::Storage;
use session_actor::SessionActor;
use tauri::Manager;

fn main() {
    let _ = init();
    tracing::info!("Starting Reef application");

    tauri::Builder::default()
        .setup(|app| {
            let storage =
                Storage::open().map_err(|error| std::io::Error::other(error.to_string()))?;
            app.manage(Arc::new(storage));

            let session_actor = SessionActor::start().map_err(std::io::Error::other)?;
            app.manage(session_actor);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::profiles::list_profiles,
            commands::profiles::save_profile,
            commands::profiles::delete_profile,
            commands::devices::connect_device,
            commands::devices::disconnect_device,
            commands::devices::send_device_command,
            commands::devices::device_is_connected,
            commands::devices::identify_device,
            commands::devices::enable_device
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Reef desktop application");
}
