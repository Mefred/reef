//! Reef desktop application entry point.
//!
//! The UI is rendered natively with wgpui. The session actor keeps the existing
//! thread-affine `SessionManager` isolated from the UI event loop.
//!
//! On Windows, suppress the console window so the application launches as a
//! GUI program with no flashing terminal.
#![windows_subsystem = "windows"]

mod app;
mod diagnostics;
mod operations;
mod palette;
mod session_actor;
mod text_field;
mod ui;

use std::sync::Arc;

use app::{DismissModal, ReefApp};
use diagnostics::AppDiagnostics;
use reef_logging::init;
use reef_storage::Storage;
use session_actor::SessionActor;
use wgpui::{
    actions, prelude::*, px, size, App, Application, Bounds, KeyBinding, TitlebarOptions,
    WindowBounds, WindowOptions,
};

actions!(reef, [Quit]);

fn main() {
    match init() {
        Ok(()) => {}
        Err(error) => eprintln!("Reef diagnostics could not start: {error}"),
    }
    install_panic_logging();
    tracing::info!("Starting Reef application");

    let app_diagnostics = match AppDiagnostics::start() {
        Ok(diagnostics) => Arc::new(diagnostics),
        Err(error) => fatal("cannot initialize portable diagnostics", &error),
    };

    let storage = match Storage::open() {
        Ok(storage) => Arc::new(storage),
        Err(error) => fatal("cannot open Reef profile storage", &error.to_string()),
    };

    let session_actor = match SessionActor::start() {
        Ok(actor) => actor,
        Err(error) => fatal("cannot start the Reef session actor", &error),
    };
    let shutdown_actor = session_actor.clone();

    reef_logging::info(
        "startup",
        "Reef desktop window starting",
        serde_json::json!({
            "startupConfigWritesEnabled": diagnostics::STARTUP_CONFIG_WRITES_ENABLED,
            "sessionRecordingDefault": diagnostics::SESSION_RECORDING_DEFAULT,
            "version": env!("CARGO_PKG_VERSION"),
        }),
    );

    Application::new().run(move |cx: &mut App| {
        cx.bind_keys(text_field::key_bindings());
        cx.bind_keys([
            KeyBinding::new("ctrl-q", Quit, None),
            KeyBinding::new("escape", app::DismissModal, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(1280.), px(840.)), cx);
        let session = session_actor.clone();
        let storage = Arc::clone(&storage);
        let diagnostics = Arc::clone(&app_diagnostics);

        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(1024.), px(680.))),
                app_id: Some("reef".to_string()),
                titlebar: Some(TitlebarOptions {
                    title: Some("Reef — Network Management".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_window, cx| {
                let view = cx.new(|cx| ReefApp::new(session, storage, diagnostics, cx));
                let weak = view.downgrade();
                let weak_for_action = weak.clone();
                view.update(cx, |app, cx| app.bootstrap(cx));
                cx.on_action(move |_: &DismissModal, _cx| {
                    let weak = weak_for_action.clone();
                    _cx.defer(move |cx| {
                        weak.update(cx, |this, cx| {
                            this.dismiss_modal();
                            cx.notify();
                        })
                        .ok();
                    });
                });
                view
            },
        );

        match opened {
            Ok(_) => cx.activate(true),
            Err(error) => {
                reef_logging::error(
                    "startup",
                    "Reef desktop window could not be opened",
                    serde_json::json!({ "error": error.to_string() }),
                );
                eprintln!("Reef desktop window could not be opened: {error}");
                cx.quit();
            }
        }
    });

    let shutdown_result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to create runtime for session actor shutdown: {error}"))
        .and_then(|runtime| {
            runtime
                .block_on(shutdown_actor.shutdown())
                .map_err(|error| error.to_string())
        });

    if let Err(error) = shutdown_result {
        tracing::warn!(%error, "session actor shutdown completed with errors");
        reef_logging::error(
            "shutdown",
            "Session actor shutdown completed with errors",
            serde_json::json!({ "error": error }),
        );
    }

    reef_logging::info(
        "shutdown",
        "Reef application stopped normally",
        serde_json::json!({}),
    );
    reef_logging::shutdown();
}

fn fatal(context: &str, error: &str) -> ! {
    reef_logging::error("startup", context, serde_json::json!({ "error": error }));
    eprintln!("{context}: {error}");
    reef_logging::shutdown();
    std::process::exit(1);
}

/// Preserve diagnostics when the process panics unexpectedly: record the panic
/// location/message, flush the log streams, then run the default hook.
fn install_panic_logging() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown".to_string());
        let message = if let Some(message) = info.payload().downcast_ref::<&str>() {
            (*message).to_string()
        } else if let Some(message) = info.payload().downcast_ref::<String>() {
            message.clone()
        } else {
            "non-string panic payload".to_string()
        };
        reef_logging::error(
            "panic",
            "Unhandled panic in Reef",
            serde_json::json!({
                "message": message,
                "location": location,
            }),
        );
        reef_logging::shutdown();
        previous(info);
    }));
}
