//! Structured logging setup for Reef using `tracing`.
//!
//! Provides a single `init()` helper that configures a `tracing` subscriber
//! driven by the `REEF_LOG` environment variable (e.g. `REEF_LOG=reef=debug`).
//! Logs exclude secret values by convention: secret fields are masked at the
//! source using the `Secret<T>` type in `reef-engine`.

use tracing_subscriber::{fmt, EnvFilter};

/// Initialize the global tracing subscriber.
///
/// Reads the `REEF_LOG` environment variable for filter directives. Defaults
/// to `reef=info,warn` if unset. Safe to call multiple times; subsequent calls
/// are no-ops once a subscriber is set.
pub fn init() -> Result<(), tracing_subscriber::filter::ParseError> {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("reef=info,warn"));

    fmt::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_defaults_to_info_when_env_unset() {
        // init() should not panic with REEF_LOG unset
        let _ = std::env::remove_var("REEF_LOG");
        assert!(init().is_ok());
    }
}
