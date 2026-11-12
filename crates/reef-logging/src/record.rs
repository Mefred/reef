//! Terminal session recording.
//!
//! A [`SessionRecorder`] is attached to a device session and appends both
//! directions of the terminal to a plain-text file under `Logs/Sessions`.
//! Output is captured at the backend, before the frontend applies any display
//! limit, so a long `show` command is not lost when the UI truncates its
//! scrollback or return value.
//!
//! Recorded text is ANSI-stripped and passed through
//! [`crate::redact_text`]. Passwords sent during login and enable escalation
//! are never handed to the recorder in the first place.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::paths::PortableLayout;
use crate::{redact_text, strip_ansi};

const PART_BYTES: u64 = 16 * 1024 * 1024;
const TOTAL_BYTES: u64 = 512 * 1024 * 1024;

/// Identifying metadata written at the top of a recording. It intentionally
/// contains no passwords, enable secrets, or key material.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingMetadata {
    pub run_id: String,
    pub device_id: String,
    pub host: String,
    pub port: u16,
    pub transport: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
}

/// Live recording counters exposed to the UI.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatus {
    pub active: bool,
    pub path: Option<String>,
    pub started_at: String,
    pub duration_ms: i64,
    pub bytes: u64,
    pub parts: u32,
    pub input_chunks: u64,
    pub output_chunks: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub redactions: u64,
    pub ansi_sequences_removed: u64,
    pub size_limit_reached: bool,
    pub error: Option<String>,
}

impl RecordingStatus {
    pub fn idle() -> Self {
        RecordingStatus {
            active: false,
            path: None,
            started_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            duration_ms: 0,
            bytes: 0,
            parts: 0,
            input_chunks: 0,
            output_chunks: 0,
            bytes_in: 0,
            bytes_out: 0,
            redactions: 0,
            ansi_sequences_removed: 0,
            size_limit_reached: false,
            error: None,
        }
    }
}

struct Inner {
    base: PathBuf,
    writer: Option<BufWriter<File>>,
    part: u32,
    bytes: u64,
    part_bytes: u64,
    total_bytes: u64,
    part_limit: u64,
    total_limit: u64,
    active: bool,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    input_chunks: u64,
    output_chunks: u64,
    bytes_in: u64,
    bytes_out: u64,
    redactions: u64,
    ansi_removed: u64,
    size_limit_reached: bool,
    error: Option<String>,
    current_path: PathBuf,
}

/// Append-only terminal recording for one device session.
pub struct SessionRecorder {
    metadata: RecordingMetadata,
    inner: Mutex<Inner>,
}

impl SessionRecorder {
    pub fn start(layout: &PortableLayout, metadata: RecordingMetadata) -> Result<Self, String> {
        Self::start_with_limits(layout, metadata, PART_BYTES, TOTAL_BYTES)
    }

    fn start_with_limits(
        layout: &PortableLayout,
        metadata: RecordingMetadata,
        part_limit: u64,
        total_limit: u64,
    ) -> Result<Self, String> {
        let started_at = Utc::now();
        let day = started_at.format("%Y-%m-%d").to_string();
        let directory = layout.session_day_directory(&day);
        std::fs::create_dir_all(&directory).map_err(|error| {
            format!(
                "cannot create session directory {}: {error}",
                directory.display()
            )
        })?;

        let device = sanitize(&metadata.device_id);
        let stamp = started_at.format("%H%M%S%.3f").to_string();
        let base = directory.join(format!("{device}-{stamp}.log"));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&base)
            .map_err(|error| format!("cannot create recording {}: {error}", base.display()))?;

        let mut inner = Inner {
            base: base.clone(),
            writer: Some(BufWriter::new(file)),
            part: 1,
            bytes: 0,
            part_bytes: 0,
            total_bytes: 0,
            part_limit,
            total_limit,
            active: true,
            started_at,
            ended_at: None,
            input_chunks: 0,
            output_chunks: 0,
            bytes_in: 0,
            bytes_out: 0,
            redactions: 0,
            ansi_removed: 0,
            size_limit_reached: false,
            error: None,
            current_path: base,
        };

        inner.write_raw(&format!(
            "# Reef terminal session recording\n\
             # run_id: {}\n\
             # device_id: {}\n\
             # endpoint: {}:{} ({})\n\
             # username: {}\n\
             # hostname: {}\n\
             # started_at: {}\n\
             # secrets: passwords, enable secrets, SNMP communities, tokens, and private keys are never recorded\n\
             # note: output is captured at the backend; ANSI escape sequences are stripped for readability\n\
             \n",
            metadata.run_id,
            metadata.device_id,
            metadata.host,
            metadata.port,
            metadata.transport,
            metadata.username,
            metadata.hostname.as_deref().unwrap_or("-"),
            started_at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ))?;

        Ok(SessionRecorder {
            metadata,
            inner: Mutex::new(inner),
        })
    }

    pub fn metadata(&self) -> &RecordingMetadata {
        &self.metadata
    }

    /// Record a line/command sent to the device. The command is redacted.
    pub fn record_input(&self, data: &str) {
        let cleaned = redact_text(&strip_ansi(data));
        self.record("TX", &cleaned, true);
    }

    /// Record output received from the device. All bytes are appended; the
    /// caller's display limits do not apply here.
    pub fn record_output(&self, data: &str) {
        let stripped = strip_ansi(data);
        let removed = count_ansi(data);
        let cleaned = redact_text(&stripped);
        let redactions = if cleaned != stripped { 1 } else { 0 };
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return,
        };
        inner.ansi_removed = inner.ansi_removed.saturating_add(removed);
        inner.redactions = inner.redactions.saturating_add(redactions);
        inner.output_chunks = inner.output_chunks.saturating_add(1);
        inner.bytes_out = inner.bytes_out.saturating_add(data.len() as u64);
        inner.append_entry("RX", &cleaned, data.len() as u64);
    }

    /// Record a non-terminal diagnostic note in the same timeline.
    pub fn note(&self, message: &str) {
        let cleaned = redact_text(message);
        self.record("NOTE", &cleaned, false);
    }

    fn record(&self, direction: &str, data: &str, input: bool) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return,
        };
        if input {
            inner.input_chunks = inner.input_chunks.saturating_add(1);
            inner.bytes_in = inner.bytes_in.saturating_add(data.len() as u64);
        }
        inner.append_entry(direction, data, data.len() as u64);
    }

    pub fn status(&self) -> RecordingStatus {
        let inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return RecordingStatus::idle(),
        };
        let ended = inner.ended_at.unwrap_or_else(Utc::now);
        RecordingStatus {
            active: inner.active,
            path: Some(inner.current_path.to_string_lossy().to_string()),
            started_at: inner
                .started_at
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            duration_ms: ended
                .signed_duration_since(inner.started_at)
                .num_milliseconds(),
            bytes: inner.total_bytes,
            parts: inner.part,
            input_chunks: inner.input_chunks,
            output_chunks: inner.output_chunks,
            bytes_in: inner.bytes_in,
            bytes_out: inner.bytes_out,
            redactions: inner.redactions,
            ansi_sequences_removed: inner.ansi_removed,
            size_limit_reached: inner.size_limit_reached,
            error: inner.error.clone(),
        }
    }

    /// Current file path, including the newest rotated part.
    pub fn path(&self) -> Option<PathBuf> {
        self.inner
            .lock()
            .ok()
            .map(|inner| inner.current_path.clone())
    }

    /// Close the recording and write a summary footer. Safe to call twice.
    pub fn stop(&self) {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(_) => return,
        };
        if !inner.active {
            return;
        }
        let ended = Utc::now();
        inner.ended_at = Some(ended);
        let summary = format!(
            "\n# recording stopped_at: {}\n\
             # duration_ms: {}\n\
             # bytes: {}\n\
             # parts: {}\n\
             # input_chunks: {}\n\
             # output_chunks: {}\n\
             # redactions: {}\n\
             # ansi_sequences_removed: {}\n\
             # size_limit_reached: {}\n",
            ended.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            ended
                .signed_duration_since(inner.started_at)
                .num_milliseconds(),
            inner.total_bytes,
            inner.part,
            inner.input_chunks,
            inner.output_chunks,
            inner.redactions,
            inner.ansi_removed,
            inner.size_limit_reached,
        );
        inner.write_raw(&summary).ok();
        inner.active = false;
    }
}

impl Inner {
    fn append_entry(&mut self, direction: &str, data: &str, original_bytes: u64) {
        if !self.active {
            return;
        }
        let timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let normalised = data.replace("\r\n", "\n").replace('\r', "\n");
        let entry = format!("[{timestamp}] {direction} ({original_bytes} bytes)\n{normalised}\n");
        self.write_entry(entry.as_bytes());
    }

    fn write_entry(&mut self, bytes: &[u8]) {
        if !self.active {
            return;
        }
        if self.total_bytes.saturating_add(bytes.len() as u64) > self.total_limit {
            self.size_limit_reached = true;
            self.active = false;
            self.error = Some(format!(
                "recording stopped at the {} byte session limit; earlier output is complete",
                self.total_limit
            ));
            let notice = format!(
                "\n# RECORDING STOPPED: {} byte limit reached. No further terminal output was recorded.\n",
                self.total_limit
            );
            self.write_raw(&notice).ok();
            return;
        }
        if self.part_bytes.saturating_add(bytes.len() as u64) > self.part_limit {
            if let Err(error) = self.rotate() {
                self.error = Some(error);
                self.active = false;
                return;
            }
        }
        if let Err(error) = self.write_raw_bytes(bytes) {
            self.error = Some(error);
            self.active = false;
        }
    }

    fn rotate(&mut self) -> Result<(), String> {
        self.writer = None;
        self.part = self.part.saturating_add(1);
        let stem = self
            .base
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("session");
        let path = self
            .base
            .with_file_name(format!("{stem}-part{:03}.log", self.part));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("cannot rotate recording to {}: {error}", path.display()))?;
        self.current_path = path;
        self.part_bytes = 0;
        self.writer = Some(BufWriter::new(file));
        self.write_raw(&format!("# continued: part {}\n", self.part))
    }

    fn write_raw(&mut self, text: &str) -> Result<(), String> {
        self.write_raw_bytes(text.as_bytes())
    }

    fn write_raw_bytes(&mut self, bytes: &[u8]) -> Result<(), String> {
        let writer = match self.writer.as_mut() {
            Some(writer) => writer,
            None => return Ok(()),
        };
        writer
            .write_all(bytes)
            .and_then(|_| writer.flush())
            .map_err(|error| format!("cannot write session recording: {error}"))?;
        self.bytes = self.bytes.saturating_add(bytes.len() as u64);
        self.part_bytes = self.part_bytes.saturating_add(bytes.len() as u64);
        self.total_bytes = self.total_bytes.saturating_add(bytes.len() as u64);
        Ok(())
    }
}

impl Drop for SessionRecorder {
    fn drop(&mut self) {
        self.stop();
    }
}

fn sanitize(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .take(48)
        .collect::<String>();
    if cleaned.is_empty() {
        "device".to_string()
    } else {
        cleaned
    }
}

fn count_ansi(input: &str) -> u64 {
    input.matches('\u{1b}').count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::PortableLayout;

    fn layout(name: &str) -> PortableLayout {
        let root = std::env::temp_dir().join(format!(
            "reef-recording-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
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

    fn metadata() -> RecordingMetadata {
        RecordingMetadata {
            run_id: "run-recording".to_string(),
            device_id: "device/one".to_string(),
            host: "192.0.2.1".to_string(),
            port: 22,
            transport: "Ssh".to_string(),
            username: "admin".to_string(),
            hostname: Some("router".to_string()),
        }
    }

    #[test]
    fn records_both_directions_and_redacts_secrets() {
        let layout = layout("basic");
        let recorder = SessionRecorder::start(&layout, metadata()).expect("start");
        recorder.record_input("show version\r\n");
        recorder.record_input("enable secret hunter2\r\n");
        recorder.record_output("\u{1b}[32mCisco IOS XE\u{1b}[0m\r\nRouter#");
        let path = recorder.path().expect("path");
        recorder.stop();

        let text = std::fs::read_to_string(&path).expect("read recording");
        assert!(text.contains("TX"));
        assert!(text.contains("show version"));
        assert!(text.contains("Cisco IOS XE"));
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("\u{1b}[32m"));
        assert!(text.contains("recording stopped_at"));
        assert!(text.contains("note: output is captured at the backend"));

        let status = recorder.status();
        assert_eq!(status.input_chunks, 2);
        assert_eq!(status.output_chunks, 1);
        assert!(!status.active);
        let _ = std::fs::remove_dir_all(&layout.root);
    }

    #[test]
    fn rotates_at_part_boundary_without_losing_output() {
        let layout = layout("rotate");
        let recorder =
            SessionRecorder::start_with_limits(&layout, metadata(), 256, 1024 * 1024).unwrap();
        for index in 0..40 {
            recorder.record_output(&format!("line {index} {}\n", "x".repeat(30)));
        }
        let status = recorder.status();
        assert!(status.parts > 1, "expected rotation, got {status:?}");
        let parts = std::fs::read_dir(
            layout.session_day_directory(&Utc::now().format("%Y-%m-%d").to_string()),
        )
        .unwrap()
        .filter_map(Result::ok)
        .count();
        assert!(parts >= status.parts as usize);
        let _ = std::fs::remove_dir_all(&layout.root);
    }

    #[test]
    fn size_limit_stops_with_an_explicit_notice() {
        let layout = layout("limit");
        let recorder = SessionRecorder::start_with_limits(&layout, metadata(), 4096, 256).unwrap();
        recorder.record_output(&"y".repeat(1000));
        let status = recorder.status();
        assert!(status.size_limit_reached);
        assert!(!status.active);
        assert!(status.error.is_some());
        let path = recorder.path().unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("RECORDING STOPPED"));
        let _ = std::fs::remove_dir_all(&layout.root);
    }
}
