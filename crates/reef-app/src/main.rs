//! Reef desktop application entry point.
//!
//! Tauri owns the desktop lifecycle. The session actor keeps the existing
//! thread-affine SessionManager isolated from Tauri's Send + Sync state.

mod commands;
mod session_actor;

use std::sync::{Arc, Mutex};

use reef_logging::init;
use reef_storage::Storage;
use session_actor::SessionActor;
use tauri::Manager;

fn main() {
    let _ = init();
    tracing::info!("Starting Reef application");

    // Keep a shutdown handle outside Tauri-managed state so the actor can be
    // explicitly stopped and awaited after the desktop event loop exits.
    let shutdown_actor = Arc::new(Mutex::new(None));
    let setup_shutdown_actor = Arc::clone(&shutdown_actor);

    let run_result = tauri::Builder::default()
        .setup(move |app| {
            let storage =
                Storage::open().map_err(|error| std::io::Error::other(error.to_string()))?;
            app.manage(Arc::new(storage));

            let session_actor = SessionActor::start().map_err(std::io::Error::other)?;
            app.manage(session_actor.clone());
            *setup_shutdown_actor
                .lock()
                .map_err(|_| std::io::Error::other("session shutdown handle was poisoned"))? =
                Some(session_actor);
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
            commands::devices::enable_device,
            commands::devices::inspect_device_workspace,
            commands::devices::get_device_configuration,
            commands::devices::preview_device_configuration
        ])
        .run(tauri::generate_context!());

    let actor = match shutdown_actor.lock() {
        Ok(mut actor) => actor.take(),
        Err(_) => {
            tracing::error!("session shutdown handle was poisoned");
            None
        }
    };

    if let Some(actor) = actor {
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => {
                if let Err(error) = runtime.block_on(actor.shutdown()) {
                    tracing::warn!(%error, "session actor shutdown completed with errors");
                }
            }
            Err(error) => {
                tracing::error!(%error, "failed to create runtime for session actor shutdown");
            }
        }
    }

    if let Err(error) = run_result {
        panic!("failed to run Reef desktop application: {error}");
    }
}
