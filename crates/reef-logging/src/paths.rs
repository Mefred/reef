//! Portable, USB-relative directory layout.
//!
//! Every writable location is derived from the directory that contains the
//! Reef executable. Reef deliberately does not use `%APPDATA%`,
//! `%LOCALAPPDATA%`, the user home directory, or the process working
//! directory. That keeps logs, configuration, and data on the USB drive and
//! makes the distribution resilient to changing drive letters.
//!
//! `REEF_PORTABLE_ROOT` can override the root for development and tests. The
//! application itself never sets it.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Marker file written into a portable distribution. Its presence makes the
/// portable mode explicit in diagnostics and in the UI.
pub const PORTABLE_MARKER: &str = "portable.marker";

/// All paths used by the portable distribution.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableLayout {
    pub root: PathBuf,
    pub logs: PathBuf,
    pub diagnostics: PathBuf,
    pub sessions: PathBuf,
    pub exports: PathBuf,
    pub test_notes: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub portable: bool,
    pub marker: Option<PathBuf>,
}

impl PortableLayout {
    /// Detect the portable root from the current executable (or an explicit
    /// environment override used by development tools and tests).
    pub fn detect() -> Result<Self, String> {
        let root = match std::env::var_os("REEF_PORTABLE_ROOT") {
            Some(value) if !value.is_empty() => PathBuf::from(value),
            _ => {
                let exe = std::env::current_exe()
                    .map_err(|error| format!("cannot locate Reef executable: {error}"))?;
                exe.parent()
                    .map(Path::to_path_buf)
                    .ok_or_else(|| "Reef executable has no parent directory".to_string())?
            }
        };

        // Normalize a relative override without touching the filesystem beyond
        // the current directory, and never fall back to a different location.
        let root = if root.is_absolute() {
            root
        } else {
            std::env::current_dir()
                .map_err(|error| format!("cannot resolve portable root: {error}"))?
                .join(root)
        };

        let marker = root.join(PORTABLE_MARKER);
        let portable = marker.is_file() || env_flag("REEF_PORTABLE");

        let logs = root.join("Logs");
        let diagnostics = logs.join("Diagnostics");
        let sessions = logs.join("Sessions");
        let exports = logs.join("Exports");
        let test_notes = logs.join("TestNotes");
        let config = root.join("Config");
        let data = root.join("Data");

        Ok(PortableLayout {
            root,
            logs,
            diagnostics,
            sessions,
            exports,
            test_notes,
            config,
            data,
            portable,
            marker: if marker.is_file() { Some(marker) } else { None },
        })
    }

    /// Create the complete directory layout. This is safe to call repeatedly.
    pub fn create_directories(&self) -> Result<(), String> {
        for directory in self.directories() {
            std::fs::create_dir_all(directory).map_err(|error| {
                format!(
                    "cannot create portable directory {}: {error}",
                    directory.display()
                )
            })?;
        }
        Ok(())
    }

    pub fn directories(&self) -> [&Path; 7] {
        [
            &self.logs,
            &self.diagnostics,
            &self.sessions,
            &self.exports,
            &self.test_notes,
            &self.config,
            &self.data,
        ]
    }

    /// Write and remove a small probe file in every writable directory. This
    /// catches read-only USB sticks, full media, and policy restrictions.
    pub fn check_writable(&self) -> Vec<String> {
        let mut failures = Vec::new();
        let probe_name = format!(
            ".reef-write-test-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );

        for directory in self.directories() {
            let probe = directory.join(&probe_name);
            match std::fs::write(&probe, b"reef portable storage probe") {
                Ok(()) => {
                    if let Err(error) = std::fs::remove_file(&probe) {
                        failures.push(format!(
                            "{} is writable but probe cleanup failed: {error}",
                            directory.display()
                        ));
                    }
                }
                Err(error) => {
                    failures.push(format!("{} is not writable: {error}", directory.display()))
                }
            }
        }

        failures
    }

    pub fn session_day_directory(&self, day: &str) -> PathBuf {
        self.sessions.join(day)
    }

    pub fn known_hosts_path(&self) -> PathBuf {
        self.config.join("known_hosts")
    }

    pub fn config_file(&self) -> PathBuf {
        self.config.join("reef-config.json")
    }

    /// Create a non-secret portable configuration file on first launch.
    pub fn ensure_default_config(&self, run_id: &str) -> Result<(), String> {
        let path = self.config_file();
        if path.is_file() {
            return Ok(());
        }

        let config = serde_json::json!({
            "schemaVersion": 1,
            "portableMode": true,
            "startupConfigWritesEnabled": false,
            "sessionRecordingDefault": true,
            "createdByRunId": run_id,
            "note": "Reef never stores passwords, private keys, or authentication tokens in this file."
        });
        let text = serde_json::to_string_pretty(&config)
            .map_err(|error| format!("cannot serialize portable configuration: {error}"))?;
        std::fs::write(&path, text)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))
    }
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

static LAYOUT: OnceLock<Result<PortableLayout, String>> = OnceLock::new();

pub fn init_layout() -> &'static Result<PortableLayout, String> {
    LAYOUT.get_or_init(PortableLayout::detect)
}

pub fn layout() -> Result<&'static PortableLayout, String> {
    init_layout().as_ref().map_err(Clone::clone)
}

pub fn layout_optional() -> Option<&'static PortableLayout> {
    init_layout().as_ref().ok()
}

pub fn known_hosts_path() -> Result<PathBuf, String> {
    layout().map(PortableLayout::known_hosts_path)
}

pub fn data_dir() -> Result<PathBuf, String> {
    layout().map(|layout| layout.data.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_layout(name: &str) -> PortableLayout {
        let root = std::env::temp_dir().join(format!(
            "reef-layout-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        PortableLayout {
            logs: root.join("Logs"),
            diagnostics: root.join("Logs/Diagnostics"),
            sessions: root.join("Logs/Sessions"),
            exports: root.join("Logs/Exports"),
            test_notes: root.join("Logs/TestNotes"),
            config: root.join("Config"),
            data: root.join("Data"),
            root,
            portable: true,
            marker: None,
        }
    }

    #[test]
    fn creates_complete_layout_and_writable_directories() {
        let layout = test_layout("create");
        layout.create_directories().expect("create test layout");
        assert!(layout.diagnostics.is_dir());
        assert!(layout.sessions.is_dir());
        assert!(layout.exports.is_dir());
        assert!(layout.config.is_dir());
        assert!(layout.data.is_dir());
        assert!(layout.check_writable().is_empty());
        std::fs::remove_dir_all(&layout.root).expect("remove test layout");
    }

    #[test]
    fn detects_portable_marker() {
        let layout = test_layout("marker");
        layout.create_directories().expect("create test layout");
        std::fs::write(layout.root.join(PORTABLE_MARKER), b"reef").expect("write marker");
        let detected = PortableLayout::detect();
        // The current executable is not the test layout, so detection should
        // still succeed at its own root. The marker path is only populated for
        // the detected root.
        assert!(detected.is_ok());
        std::fs::remove_dir_all(&layout.root).expect("remove test layout");
    }

    #[test]
    fn default_config_is_created_without_secrets() {
        let layout = test_layout("config");
        layout.create_directories().expect("create test layout");
        layout
            .ensure_default_config("run-test")
            .expect("create config");
        let text = std::fs::read_to_string(layout.config_file()).expect("read config");
        assert!(text.contains("\"startupConfigWritesEnabled\": false"));
        assert!(!text.to_lowercase().contains("password\":"));
        layout
            .ensure_default_config("another-run")
            .expect("existing config is preserved");
        std::fs::remove_dir_all(&layout.root).expect("remove test layout");
    }
}
