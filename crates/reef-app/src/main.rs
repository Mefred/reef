//! Reef application entry point.
//!
//! Slint desktop GUI that drives the reef-engine configuration engine through
//! reef-session transport sessions.

mod app;

use reef_logging::init;

fn main() {
    // Initialize logging (REEF_LOG env var controls filter).
    let _ = init();

    tracing::info!("Starting Reef application");

    // Launch Slint main window (blocks until window is closed).
    let app_window = app::AppWindow::new().expect("failed to create Slint window");
    app_window.show().expect("failed to show Slint window");
}
