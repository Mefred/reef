//! Application-level diagnostics: environment capture, status reporting, log
//! viewing, and ZIP export.
//!
//! The portable directory layout and the structured event engine live in
//! `reef-logging`. This module only adds the parts that need the application
//! binary: the environment snapshot, the list of log files for the UI, and a
//! self-contained diagnostic ZIP.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use reef_logging::paths::PortableLayout;
use serde::Serialize;
use serde_json::{json, Value};
use sysinfo::System;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// Startup-configuration writes are permanently disabled. This constant is
/// exposed to the UI so the restriction is visible, not hidden.
pub const STARTUP_CONFIG_WRITES_ENABLED: bool = false;
/// Session recording starts automatically for every new connection.
pub const SESSION_RECORDING_DEFAULT: bool = true;

/// Environment information captured once at startup.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInfo {
    pub app_version: String,
    pub os_name: String,
    pub os_version: String,
    pub kernel_version: String,
    pub architecture: String,
    pub host_name: String,
    pub logical_cpus: usize,
    pub total_memory_bytes: u64,
    pub executable: String,
    pub working_directory: String,
    pub portable_root: String,
    pub portable: bool,
    pub process_id: u32,
    pub debug_build: bool,
}

impl EnvironmentInfo {
    pub fn collect(layout: &PortableLayout) -> Self {
        let system = System::new_all();
        let executable = std::env::current_exe()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unavailable".to_string());
        let working_directory = std::env::current_dir()
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unavailable".to_string());

        EnvironmentInfo {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os_name: System::name().unwrap_or_else(|| "unknown".to_string()),
            os_version: System::long_os_version()
                .or_else(System::os_version)
                .unwrap_or_else(|| "unknown".to_string()),
            kernel_version: System::kernel_version().unwrap_or_else(|| "unknown".to_string()),
            architecture: std::env::consts::ARCH.to_string(),
            host_name: System::host_name().unwrap_or_else(|| "unknown".to_string()),
            logical_cpus: system.cpus().len().max(
                std::thread::available_parallelism()
                    .map(|value| value.get())
                    .unwrap_or(1),
            ),
            total_memory_bytes: system.total_memory(),
            executable,
            working_directory,
            portable_root: layout.root.to_string_lossy().to_string(),
            portable: layout.portable,
            process_id: std::process::id(),
            debug_build: cfg!(debug_assertions),
        }
    }

    fn as_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or_else(|_| json!({}))
    }
}

/// One log file the diagnostics UI can list or open.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFile {
    pub path: String,
    pub relative_path: String,
    pub kind: String,
    pub size_bytes: u64,
    pub modified_ms: u64,
}

/// Result of reading a log file for display.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogFileContent {
    pub path: String,
    pub content: String,
    pub size_bytes: u64,
    pub truncated: bool,
}

/// Live diagnostics status for the frontend.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsStatus {
    pub run_id: String,
    pub level: String,
    pub started_at: String,
    pub uptime_seconds: u64,
    pub portable: bool,
    pub root: String,
    pub logs_directory: String,
    pub diagnostics_directory: String,
    pub sessions_directory: String,
    pub exports_directory: String,
    pub test_notes_directory: String,
    pub jsonl_path: Option<String>,
    pub human_path: Option<String>,
    pub degraded: Option<String>,
    pub writable_failures: Vec<String>,
    pub environment: EnvironmentInfo,
    pub startup_config_writes_enabled: bool,
    pub session_recording_default: bool,
}

/// Application diagnostics state shared by the native Reef UI.
pub struct AppDiagnostics {
    started: Instant,
    environment: EnvironmentInfo,
}

impl AppDiagnostics {
    /// Capture the environment and write the startup diagnostic event.
    pub fn start() -> Result<Self, String> {
        let layout = reef_logging::paths::layout()?;
        let environment = EnvironmentInfo::collect(layout);
        let diagnostics = AppDiagnostics {
            started: Instant::now(),
            environment,
        };

        reef_logging::info(
            "startup",
            "Reef environment captured",
            json!({
                "environment": diagnostics.environment.as_json(),
                "startupConfigWritesEnabled": STARTUP_CONFIG_WRITES_ENABLED,
                "sessionRecordingDefault": SESSION_RECORDING_DEFAULT,
            }),
        );
        Ok(diagnostics)
    }

    pub fn status(&self) -> Result<DiagnosticsStatus, String> {
        let layout = reef_logging::paths::layout()?;
        layout.create_directories()?;
        let engine = reef_logging::diagnostics();

        Ok(DiagnosticsStatus {
            run_id: engine
                .map(|diagnostics| diagnostics.run_id().to_string())
                .unwrap_or_else(|| "unavailable".to_string()),
            level: engine
                .map(|diagnostics| diagnostics.minimum_level().as_str().to_string())
                .unwrap_or_else(|| "INFO".to_string()),
            started_at: engine
                .map(|diagnostics| diagnostics.started_at().to_rfc3339())
                .unwrap_or_else(|| "unavailable".to_string()),
            uptime_seconds: self.started.elapsed().as_secs(),
            portable: layout.portable,
            root: layout.root.to_string_lossy().to_string(),
            logs_directory: layout.logs.to_string_lossy().to_string(),
            diagnostics_directory: layout.diagnostics.to_string_lossy().to_string(),
            sessions_directory: layout.sessions.to_string_lossy().to_string(),
            exports_directory: layout.exports.to_string_lossy().to_string(),
            test_notes_directory: layout.test_notes.to_string_lossy().to_string(),
            jsonl_path: engine
                .and_then(|diagnostics| diagnostics.jsonl_path())
                .map(|path| path.to_string_lossy().to_string()),
            human_path: engine
                .and_then(|diagnostics| diagnostics.human_path())
                .map(|path| path.to_string_lossy().to_string()),
            degraded: engine.and_then(|diagnostics| diagnostics.degraded()),
            writable_failures: engine
                .map(|diagnostics| diagnostics.writable_failures().to_vec())
                .unwrap_or_default(),
            environment: self.environment.clone(),
            startup_config_writes_enabled: STARTUP_CONFIG_WRITES_ENABLED,
            session_recording_default: SESSION_RECORDING_DEFAULT,
        })
    }

    /// Flush diagnostics and produce a ZIP containing diagnostics, session
    /// recordings, test notes, the portable configuration, and an environment
    /// report.
    pub fn export(&self) -> Result<PathBuf, String> {
        if let Some(engine) = reef_logging::diagnostics() {
            engine.flush();
        }

        let layout = reef_logging::paths::layout()?;
        layout.create_directories()?;
        let status = self.status()?;
        let run_short = status.run_id.chars().take(8).collect::<String>();
        let stamp = unix_ms();
        let destination = layout
            .exports
            .join(format!("Reef-Diagnostics-{run_short}-{stamp}.zip"));

        let file = File::create(&destination)
            .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
        let mut archive = ZipWriter::new(file);
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);

        let environment = serde_json::to_string_pretty(&self.environment)
            .map_err(|error| format!("cannot serialize environment: {error}"))?;
        write_entry(
            &mut archive,
            options,
            "environment.json",
            environment.as_bytes(),
        )?;
        write_entry(
            &mut archive,
            options,
            "diagnostics-status.json",
            serde_json::to_string_pretty(&status)
                .map_err(|error| format!("cannot serialize status: {error}"))?
                .as_bytes(),
        )?;
        write_entry(
            &mut archive,
            options,
            "README.txt",
            export_readme().as_bytes(),
        )?;

        // Collect log files before creating the archive entries. The exports
        // directory is skipped so a previous export is never nested inside a
        // new one.
        let mut files = Vec::new();
        collect_files(&layout.logs, &layout.exports, &mut files).map_err(|error| {
            format!(
                "cannot enumerate log files under {}: {error}",
                layout.logs.display()
            )
        })?;
        files.sort();
        for path in files {
            let relative = path
                .strip_prefix(&layout.logs)
                .map_err(|_| format!("{} is not inside the log directory", path.display()))?;
            let archive_name = format!("Logs/{}", relative.to_string_lossy().replace('\\', "/"));
            let mut input = File::open(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let mut buffer = Vec::new();
            input
                .read_to_end(&mut buffer)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            write_entry(&mut archive, options, &archive_name, &buffer)?;
        }

        let config = layout.config_file();
        if config.is_file() {
            let mut buffer = Vec::new();
            File::open(&config)
                .and_then(|mut input| input.read_to_end(&mut buffer))
                .map_err(|error| format!("cannot read {}: {error}", config.display()))?;
            write_entry(&mut archive, options, "Config/reef-config.json", &buffer)?;
        }

        archive
            .finish()
            .map_err(|error| format!("cannot finish {}: {error}", destination.display()))?;

        reef_logging::info(
            "diagnostics",
            "Diagnostic export created",
            json!({
                "path": destination.to_string_lossy(),
                "runId": status.run_id,
            }),
        );
        Ok(destination)
    }

    /// List files under the portable `Logs` directory for the UI.
    pub fn list_files(&self) -> Result<Vec<LogFile>, String> {
        let layout = reef_logging::paths::layout()?;
        layout.create_directories()?;
        let mut paths = Vec::new();
        collect_files(&layout.logs, &layout.exports, &mut paths)
            .map_err(|error| format!("cannot enumerate logs: {error}"))?;
        paths.sort();

        let mut files = Vec::new();
        for path in paths {
            let metadata = match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => metadata,
                _ => continue,
            };
            let relative = path
                .strip_prefix(&layout.logs)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| path.to_string_lossy().to_string());
            files.push(LogFile {
                path: path.to_string_lossy().to_string(),
                relative_path: relative.clone(),
                kind: log_kind(&relative),
                size_bytes: metadata.len(),
                modified_ms: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64)
                    .unwrap_or(0),
            });
        }

        files.sort_by(|left, right| {
            right
                .modified_ms
                .cmp(&left.modified_ms)
                .then_with(|| left.relative_path.cmp(&right.relative_path))
        });
        Ok(files)
    }

    /// Read a log file for display. The path must resolve inside `Logs`.
    pub fn read_file(&self, path: &str) -> Result<LogFileContent, String> {
        const MAX_VIEW_BYTES: u64 = 2 * 1024 * 1024;
        let layout = reef_logging::paths::layout()?;
        let resolved = resolve_under(&layout.logs, path)?;
        let metadata = std::fs::metadata(&resolved)
            .map_err(|error| format!("cannot open {}: {error}", resolved.display()))?;
        if !metadata.is_file() {
            return Err(format!("{} is not a file", resolved.display()));
        }

        let file = File::open(&resolved)
            .map_err(|error| format!("cannot open {}: {error}", resolved.display()))?;
        let mut buffer = Vec::new();
        file.take(MAX_VIEW_BYTES)
            .read_to_end(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", resolved.display()))?;
        let truncated = metadata.len() > MAX_VIEW_BYTES;

        Ok(LogFileContent {
            path: resolved.to_string_lossy().to_string(),
            content: String::from_utf8_lossy(&buffer).to_string(),
            size_bytes: metadata.len(),
            truncated,
        })
    }

    /// Open the log directory in the platform file manager.
    pub fn open_logs_directory(&self) -> Result<PathBuf, String> {
        let layout = reef_logging::paths::layout()?;
        layout.create_directories()?;
        open_in_file_manager(&layout.logs)?;
        Ok(layout.logs.clone())
    }

    /// Record a free-form note from the classroom tester.
    pub fn test_note(&self, note: &str) -> Result<PathBuf, String> {
        if note.trim().is_empty() {
            return Err("test note cannot be empty".to_string());
        }
        if note.len() > 8192 {
            return Err("test note exceeds the 8192-byte limit".to_string());
        }
        reef_logging::record_test_note(note)
    }
}

fn write_entry<W: Write + std::io::Seek>(
    archive: &mut ZipWriter<W>,
    options: SimpleFileOptions,
    name: &str,
    bytes: &[u8],
) -> Result<(), String> {
    archive
        .start_file(name, options)
        .map_err(|error| format!("cannot add {name} to export: {error}"))?;
    archive
        .write_all(bytes)
        .map_err(|error| format!("cannot write {name} to export: {error}"))
}

fn collect_files(root: &Path, skip: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    let mut entries = std::fs::read_dir(root)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path == skip {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(&path, skip, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn resolve_under(root: &Path, candidate: &str) -> Result<PathBuf, String> {
    let requested = PathBuf::from(candidate);
    let requested = if requested.is_absolute() {
        requested
    } else {
        root.join(requested)
    };

    // Reject traversal components before touching the filesystem.
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("log path cannot contain '..'".to_string());
    }

    let canonical_root = root
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", root.display()))?;
    let canonical = requested
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", requested.display()))?;
    if !canonical.starts_with(&canonical_root) {
        return Err("log path is outside the Reef log directory".to_string());
    }
    Ok(canonical)
}

fn log_kind(relative: &str) -> String {
    if relative.starts_with("Diagnostics/") {
        "diagnostics".to_string()
    } else if relative.starts_with("Sessions/") {
        "session".to_string()
    } else if relative.starts_with("TestNotes/") {
        "test-note".to_string()
    } else if relative.starts_with("Exports/") {
        "export".to_string()
    } else {
        "other".to_string()
    }
}

fn open_in_file_manager(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer.exe");
        command.arg(path);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("cannot open {}: {error}", path.display()))
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn export_readme() -> String {
    concat!(
        "Reef diagnostic export\r\n",
        "=======================\r\n\r\n",
        "This archive contains structured JSONL diagnostics, a human-readable log,\r\n",
        "terminal session recordings, classroom test notes, the non-secret portable\r\n",
        "configuration, and an environment snapshot.\r\n\r\n",
        "Reef never records passwords, enable secrets, SNMP communities, private keys,\r\n",
        "or authentication tokens. Terminal recordings and diagnostic fields are\r\n",
        "redacted before they are written.\r\n\r\n",
        "Startup-configuration writes are permanently disabled in this build.\r\n",
        "See DIAGNOSTICS-README.txt in the distribution for the full log layout.\r\n"
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_paths_outside_the_log_directory() {
        let root = std::env::temp_dir().join(format!(
            "reef-app-diagnostics-{}-{}",
            std::process::id(),
            unix_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("inside.log"), b"inside").unwrap();

        assert!(resolve_under(&root, "inside.log").is_ok());
        assert!(resolve_under(&root, "../outside.log").is_err());
        assert!(resolve_under(&root, "/etc/passwd").is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn classifies_log_files() {
        assert_eq!(log_kind("Diagnostics/run.jsonl"), "diagnostics");
        assert_eq!(log_kind("Sessions/2026-01-01/a.log"), "session");
        assert_eq!(log_kind("TestNotes/test-notes.md"), "test-note");
        assert_eq!(log_kind("Exports/export.zip"), "export");
        assert_eq!(log_kind("something.txt"), "other");
    }

    #[test]
    fn startup_writes_stay_disabled() {
        assert!(!STARTUP_CONFIG_WRITES_ENABLED);
        assert!(SESSION_RECORDING_DEFAULT);
    }
}
