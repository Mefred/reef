//! Reef application entry point.
//!
//! Slint desktop GUI that drives the reef-engine configuration engine through
//! reef-session transport sessions.

mod app;
mod controller;

use reef_logging::init;

fn main() {
    // Initialize logging (REEF_LOG env var controls filter).
    let _ = init();

    tracing::info!("Starting Reef application");

    // TODO: Launch Slint main window (Phase 4).
    println!("Reef v0.1.0 - implementation in progress (see PLAN.md)");
}
