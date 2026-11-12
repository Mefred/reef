//! Portable diagnostic logging for Reef.
//!
//! This crate owns the USB-relative directory layout ([`paths`]), the
//! structured diagnostic engine, and terminal session recording
//! ([`record`]).
//!
//! Diagnostics are written in two forms at the same time:
//!
//! * a machine-readable JSON Lines file (one object per event), and
//! * a human-readable text file.
//!
//! Every event carries the process run id, a millisecond timestamp, a level,
//! and a category. The `tracing` subscriber installed by [`init`] forwards
//! ordinary `tracing` events into the same files, so existing backend logging
//! is captured without duplicating call sites.
//!
//! Logs must never contain passwords, private keys, SNMP communities, or
//! authentication tokens. Use [`redact_text`] before recording any value that
//! could contain a secret. Session recording applies it automatically.

pub mod paths;
pub mod record;

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub use paths::{layout, layout_optional, PortableLayout};

/// Diagnostic severity. Ordered from most to least verbose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Trace => "TRACE",
            Level::Debug => "DEBUG",
            Level::Info => "INFO",
            Level::Warn => "WARN",
            Level::Error => "ERROR",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "trace" => Some(Level::Trace),
            "debug" => Some(Level::Debug),
            "info" => Some(Level::Info),
            "warn" | "warning" => Some(Level::Warn),
            "error" => Some(Level::Error),
            _ => None,
        }
    }

    fn from_tracing(level: &tracing::Level) -> Self {
        if *level == tracing::Level::TRACE {
            Level::Trace
        } else if *level == tracing::Level::DEBUG {
            Level::Debug
        } else if *level == tracing::Level::INFO {
            Level::Info
        } else if *level == tracing::Level::WARN {
            Level::Warn
        } else {
            Level::Error
        }
    }
}

/// One structured diagnostic event.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticEvent {
    pub timestamp: String,
    pub unix_ms: i64,
    pub run_id: String,
    pub level: String,
    pub category: String,
    pub message: String,
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

/// Maximum size of each diagnostic file part before rotation.
const DIAGNOSTIC_PART_BYTES: u64 = 8 * 1024 * 1024;
/// Number of diagnostic parts retained per stream.
const DIAGNOSTIC_MAX_PARTS: u32 = 8;

struct Inner {
    jsonl: Sink,
    human: Sink,
    degraded: Option<String>,
    closed: bool,
}

/// The process-wide diagnostic engine.
pub struct Diagnostics {
    run_id: String,
    started_at: DateTime<Utc>,
    layout: PortableLayout,
    minimum_level: Level,
    writable_failures: Vec<String>,
    inner: Mutex<Inner>,
}

impl Diagnostics {
    /// Open the engine under an explicit layout. Used by tests and by
    /// [`init`]; callers should normally use the global helpers instead.
    pub fn open(
        layout: PortableLayout,
        run_id: impl Into<String>,
        minimum_level: Level,
        writable_failures: Vec<String>,
    ) -> Result<Self, String> {
        layout.create_directories()?;
        let run_id = run_id.into();
        let started_at = Utc::now();
        let stem = format!(
            "reef-diagnostics-{}-{}",
            started_at.format("%Y%m%d-%H%M%S"),
            short_id(&run_id)
        );
        let jsonl = Sink::new(
            layout.diagnostics.join(format!("{stem}.jsonl")),
            DIAGNOSTIC_PART_BYTES,
            DIAGNOSTIC_MAX_PARTS,
        )?;
        let human = Sink::new(
            layout.diagnostics.join(format!("{stem}.log")),
            DIAGNOSTIC_PART_BYTES,
            DIAGNOSTIC_MAX_PARTS,
        )?;

        Ok(Diagnostics {
            run_id,
            started_at,
            layout,
            minimum_level,
            writable_failures,
            inner: Mutex::new(Inner {
                jsonl,
                human,
                degraded: None,
                closed: false,
            }),
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }

    pub fn layout(&self) -> &PortableLayout {
        &self.layout
    }

    pub fn minimum_level(&self) -> Level {
        self.minimum_level
    }

    pub fn writable_failures(&self) -> &[String] {
        &self.writable_failures
    }

    pub fn is_enabled(&self, level: Level) -> bool {
        matches!(current_level(), Some(current) if level >= current) || level >= self.minimum_level
    }

    /// Record an event. Fields must be a JSON object; any other value is
    /// wrapped as `{"value": ...}`. Secrets must already be redacted or be
    /// passed through [`redact_text`].
    pub fn event(&self, level: Level, category: &str, message: &str, fields: Value) {
        let fields = match fields {
            Value::Object(map) => map,
            Value::Null => Map::new(),
            other => {
                let mut map = Map::new();
                map.insert("value".to_string(), other);
                map
            }
        };
        let fields = fields
            .into_iter()
            .map(|(key, value)| (key, redact_json(&value)))
            .collect::<BTreeMap<_, _>>();

        let now = Utc::now();
        let event = DiagnosticEvent {
            timestamp: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            unix_ms: now.timestamp_millis(),
            run_id: self.run_id.clone(),
            level: level.as_str().to_string(),
            category: category.to_string(),
            message: redact_text(message),
            fields,
            target: None,
            thread: None,
        };

        self.write(&event);
    }

    fn write(&self, event: &DiagnosticEvent) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.closed {
            return;
        }

        let json_line = match serde_json::to_string(event) {
            Ok(line) => line,
            Err(error) => {
                inner.degraded = Some(format!("cannot serialize diagnostic event: {error}"));
                return;
            }
        };
        if let Err(error) = inner.jsonl.write_line(&json_line) {
            inner.degraded = Some(format!("cannot write JSONL diagnostics: {error}"));
        }

        let mut human = format!(
            "{} [{}] [{}] {}",
            event.timestamp, event.level, event.category, event.message
        );
        for (key, value) in &event.fields {
            human.push(' ');
            human.push_str(key);
            human.push('=');
            human.push_str(&human_value(value));
        }
        if let Err(error) = inner.human.write_line(&human) {
            inner.degraded = Some(format!("cannot write human diagnostics: {error}"));
        }
    }

    pub fn degraded(&self) -> Option<String> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.degraded.clone())
    }

    /// Flush both streams without closing them.
    pub fn flush(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.jsonl.flush();
            inner.human.flush();
        }
    }

    /// Write a final event and close both streams. Safe to call repeatedly.
    pub fn close(&self) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.closed {
            return;
        }
        inner.jsonl.flush();
        inner.human.flush();
        inner.closed = true;
    }

    /// Path of the current JSON Lines file.
    pub fn jsonl_path(&self) -> Option<PathBuf> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.jsonl.current_path())
    }

    /// Path of the current human-readable file.
    pub fn human_path(&self) -> Option<PathBuf> {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.human.current_path())
    }
}

static DIAGNOSTICS: OnceLock<Arc<Diagnostics>> = OnceLock::new();
static LEVEL: OnceLock<Level> = OnceLock::new();

/// Initialize the process diagnostic engine and tracing subscriber.
///
/// Reads `REEF_LOG_LEVEL` (or `REEF_LOG`) for the minimum level; defaults to
/// `INFO`. `REEF_PORTABLE_ROOT` overrides the portable root for development
/// and tests. The application never sets that variable.
pub fn init() -> Result<(), String> {
    if DIAGNOSTICS.get().is_some() {
        return Ok(());
    }

    let level = configured_level();
    let _ = LEVEL.set(level);

    let layout = PortableLayout::detect()?;
    layout.create_directories()?;
    let run_id = uuid::Uuid::new_v4().to_string();
    layout.ensure_default_config(&run_id)?;
    let writable_failures = layout.check_writable();
    let engine = Diagnostics::open(layout, run_id, level, writable_failures)?;

    if DIAGNOSTICS.set(Arc::new(engine)).is_err() {
        return Ok(());
    }

    install_tracing(level);
    info(
        "startup",
        "Reef diagnostics initialized",
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "level": level.as_str(),
            "portable": diagnostics().map(|d| d.layout().portable).unwrap_or(false),
            "writableFailures": diagnostics().map(|d| d.writable_failures().len()).unwrap_or(0),
        }),
    );
    Ok(())
}

fn configured_level() -> Level {
    std::env::var("REEF_LOG_LEVEL")
        .ok()
        .and_then(|value| Level::parse(&value))
        .or_else(|| {
            std::env::var("REEF_LOG").ok().and_then(|value| {
                value
                    .split(',')
                    .filter_map(|directive| directive.split('=').nth(1))
                    .find_map(Level::parse)
            })
        })
        .unwrap_or(Level::Info)
}

fn current_level() -> Option<Level> {
    LEVEL.get().copied()
}

/// Global diagnostic engine, if [`init`] succeeded.
pub fn diagnostics() -> Option<&'static Arc<Diagnostics>> {
    DIAGNOSTICS.get()
}

pub fn run_id() -> Option<String> {
    diagnostics().map(|diagnostics| diagnostics.run_id().to_string())
}

pub fn started_at() -> Option<DateTime<Utc>> {
    diagnostics().map(|diagnostics| diagnostics.started_at())
}

/// Record an event through the global engine. If initialization failed, fall
/// back to stderr so the message is not silently lost.
pub fn event(level: Level, category: &str, message: &str, fields: Value) {
    match diagnostics() {
        Some(diagnostics) => diagnostics.event(level, category, message, fields),
        None => {
            if level >= Level::Warn {
                eprintln!(
                    "{} [{}] [{}] {} {}",
                    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                    level.as_str(),
                    category,
                    redact_text(message),
                    fields
                );
            }
        }
    }
}

pub fn trace(category: &str, message: &str, fields: Value) {
    event(Level::Trace, category, message, fields);
}

pub fn debug(category: &str, message: &str, fields: Value) {
    event(Level::Debug, category, message, fields);
}

pub fn info(category: &str, message: &str, fields: Value) {
    event(Level::Info, category, message, fields);
}

pub fn warn(category: &str, message: &str, fields: Value) {
    event(Level::Warn, category, message, fields);
}

pub fn error(category: &str, message: &str, fields: Value) {
    event(Level::Error, category, message, fields);
}

/// Log a user-visible test note and persist it under `Logs/TestNotes`.
pub fn record_test_note(note: &str) -> Result<PathBuf, String> {
    let layout = paths::layout()?;
    layout.create_directories()?;
    let path = layout.test_notes.join("test-notes.md");
    let timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let run = run_id().unwrap_or_else(|| "unknown-run".to_string());
    let entry = format!(
        "- {timestamp} (run `{run}`): {}\n",
        redact_text(note.trim())
    );
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    file.write_all(entry.as_bytes())
        .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    info("test", "Test note recorded", json!({ "note": note }));
    Ok(path)
}

/// Flush and close the process diagnostics. A final shutdown event is written.
pub fn shutdown() {
    info("shutdown", "Reef diagnostics shutting down", json!({}));
    if let Some(diagnostics) = diagnostics() {
        diagnostics.flush();
        diagnostics.close();
    }
}

fn install_tracing(level: Level) {
    let default = format!(
        "reef={level},reef_app={level},reef_session={level},reef_logging={level},reef_storage={level},reef_engine={level},warn",
        level = level.as_str().to_ascii_lowercase()
    );
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    let layer = DiagnosticsLayer;
    let subscriber = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .with(fmt::layer().with_target(false).with_thread_ids(true));
    // Another subscriber may already be installed (for example in a test
    // harness). Diagnostics still work; tracing capture is best effort.
    let _ = subscriber.try_init();
}

struct DiagnosticsLayer;

impl<S: Subscriber> Layer<S> for DiagnosticsLayer {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let Some(diagnostics) = diagnostics() else {
            return;
        };
        let level = Level::from_tracing(event.metadata().level());
        if !diagnostics.is_enabled(level) {
            return;
        }

        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message.unwrap_or_default();
        let target = Some(event.metadata().target().to_string());
        let thread = std::thread::current().name().map(str::to_string);
        let fields = visitor
            .fields
            .into_iter()
            .map(|(key, value)| (key, redact_json(&value)))
            .collect::<BTreeMap<_, _>>();

        let now = Utc::now();
        diagnostics.write(&DiagnosticEvent {
            timestamp: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            unix_ms: now.timestamp_millis(),
            run_id: diagnostics.run_id().to_string(),
            level: level.as_str().to_string(),
            category: "tracing".to_string(),
            message: redact_text(&message),
            fields,
            target,
            thread,
        });
    }
}

#[derive(Default)]
struct FieldVisitor {
    message: Option<String>,
    fields: BTreeMap<String, Value>,
}

impl Visit for FieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let text = format!("{value:?}");
        if field.name() == "message" {
            self.message = Some(text);
        } else {
            self.fields
                .insert(field.name().to_string(), Value::String(text));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = Some(value.to_string());
        } else {
            self.fields
                .insert(field.name().to_string(), Value::String(value.to_string()));
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields.insert(field.name().to_string(), json!(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields.insert(field.name().to_string(), json!(value));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields.insert(field.name().to_string(), json!(value));
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.fields.insert(field.name().to_string(), json!(value));
    }
}

struct Sink {
    base: PathBuf,
    part: u32,
    bytes: u64,
    max_bytes: u64,
    max_parts: u32,
    writer: Option<BufWriter<File>>,
}

impl Sink {
    fn new(base: PathBuf, max_bytes: u64, max_parts: u32) -> Result<Self, String> {
        let mut sink = Sink {
            base,
            part: 0,
            bytes: 0,
            max_bytes,
            max_parts,
            writer: None,
        };
        sink.rotate()?;
        Ok(sink)
    }

    fn rotate(&mut self) -> Result<(), String> {
        self.flush();
        self.part = self.part.saturating_add(1);
        let path = if self.part == 1 {
            self.base.clone()
        } else {
            let stem = self
                .base
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("diagnostics");
            let extension = self
                .base
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("log");
            self.base
                .with_file_name(format!("{stem}-part{:03}.{extension}", self.part))
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
        self.bytes = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        self.writer = Some(BufWriter::new(file));
        self.remove_old_parts();
        Ok(())
    }

    fn remove_old_parts(&self) {
        if self.part <= self.max_parts {
            return;
        }
        let stale = self.part - self.max_parts;
        let stem = match self.base.file_stem().and_then(|stem| stem.to_str()) {
            Some(stem) => stem,
            None => return,
        };
        let extension = self
            .base
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("log");
        let path = if stale == 1 {
            self.base.clone()
        } else {
            self.base
                .with_file_name(format!("{stem}-part{stale:03}.{extension}"))
        };
        let _ = std::fs::remove_file(path);
    }

    fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        if self.bytes >= self.max_bytes {
            if let Err(error) = self.rotate() {
                return Err(std::io::Error::other(error));
            }
        }
        // Reserve room for the newline so parts do not exceed their target by
        // much; a single large line is still written intact.
        let written = match self.writer.as_mut() {
            Some(writer) => {
                writer.write_all(line.as_bytes())?;
                writer.write_all(b"\n")?;
                writer.flush()?;
                line.len() as u64 + 1
            }
            None => 0,
        };
        self.bytes = self.bytes.saturating_add(written);
        Ok(())
    }

    fn flush(&mut self) {
        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.flush();
        }
    }

    fn current_path(&self) -> Option<PathBuf> {
        if self.part == 0 {
            return None;
        }
        if self.part == 1 {
            Some(self.base.clone())
        } else {
            let stem = self.base.file_stem().and_then(|stem| stem.to_str())?;
            let extension = self.base.extension().and_then(|ext| ext.to_str())?;
            Some(
                self.base
                    .with_file_name(format!("{stem}-part{:03}.{extension}", self.part)),
            )
        }
    }
}

fn human_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn short_id(run_id: &str) -> String {
    run_id.chars().take(8).collect()
}

fn redact_json(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(redact_text(text)),
        Value::Array(items) => Value::Array(items.iter().map(redact_json).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), redact_json(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Remove ANSI/VT control sequences from terminal text.
pub fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(character) = chars.next() {
        if character != '\u{1b}' {
            output.push(character);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                // CSI: parameters/intermediates until a final byte 0x40..=0x7e.
                for next in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                // OSC: terminated by BEL or ST (ESC \).
                while let Some(next) = chars.next() {
                    if next == '\u{7}' {
                        break;
                    }
                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some('P') | Some('X') | Some('^') | Some('_') => {
                chars.next();
                // String controls: terminated by ST.
                while let Some(next) = chars.next() {
                    if next == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    output
}

/// Redact values that look like credentials from terminal text.
///
/// The redaction is deliberately conservative: it can hide harmless text, but
/// it must not let passwords, enable secrets, SNMP communities, private keys,
/// or bearer tokens reach a log file. The original text is never written.
pub fn redact_text(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_private_key = false;

    for (index, line) in input.split_inclusive('\n').enumerate() {
        if index > 0 && !output.ends_with('\n') {
            output.push('\n');
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);

        if in_private_key {
            if trimmed.contains("END") && trimmed.contains("PRIVATE KEY") {
                in_private_key = false;
                output.push_str("[REDACTED PRIVATE KEY BLOCK]\n");
            }
            continue;
        }
        if trimmed.contains("BEGIN") && trimmed.contains("PRIVATE KEY") {
            in_private_key = true;
            output.push_str("[REDACTED PRIVATE KEY BLOCK]\n");
            continue;
        }

        output.push_str(&redact_line(trimmed));
        if line.ends_with('\n') {
            output.push('\n');
        }
    }
    output
}

fn redact_line(line: &str) -> String {
    const KEYWORDS: &[&str] = &[
        "password",
        "passwd",
        "enable secret",
        "secret",
        "token",
        "api-key",
        "apikey",
        "community",
        "key-string",
        "authorization:",
        "authorization ",
        "private-key",
        "passphrase",
    ];

    let lowered = line.to_ascii_lowercase();
    let mut result = line.to_string();
    let mut search_from = 0usize;

    loop {
        if search_from >= lowered.len() {
            break;
        }
        let mut earliest: Option<(usize, &str)> = None;
        for keyword in KEYWORDS {
            if let Some(relative) = lowered[search_from..].find(keyword) {
                let absolute = search_from + relative;
                let end = absolute + keyword.len();
                // Skip an exact word boundary issue only for "secret" inside
                // "secretary"-style prose. A keyword match is otherwise kept.
                if *keyword == "secret" {
                    let before_ok =
                        absolute == 0 || !lowered.as_bytes()[absolute - 1].is_ascii_alphanumeric();
                    let after_ok =
                        end == lowered.len() || !lowered.as_bytes()[end].is_ascii_alphanumeric();
                    if !(before_ok && after_ok) {
                        continue;
                    }
                }
                if earliest.is_none_or(|(position, _)| absolute < position) {
                    earliest = Some((absolute, keyword));
                }
            }
        }
        let Some((position, keyword)) = earliest else {
            break;
        };
        let value_start = position + keyword.len();
        let bytes = result.as_bytes();
        let mut start = value_start;
        while start < bytes.len() && (bytes[start] as char).is_ascii_whitespace() {
            start += 1;
        }
        // Skip separators such as '=', ':' and a quoted opening.
        while start < bytes.len() && matches!(bytes[start], b'=' | b':' | b'"' | b'\'') {
            start += 1;
            while start < bytes.len() && (bytes[start] as char).is_ascii_whitespace() {
                start += 1;
            }
        }
        if start >= bytes.len() {
            break;
        }
        let redact_rest = [
            "password",
            "passwd",
            "enable secret",
            "secret",
            "key-string",
            "passphrase",
            "private-key",
            "api-key",
            "apikey",
            "authorization:",
            "authorization ",
        ]
        .contains(&keyword);
        let mut end = start;
        if redact_rest {
            end = result.len();
        } else {
            while end < bytes.len() {
                let byte = bytes[end];
                if byte.is_ascii_whitespace() || byte == b'"' || byte == b'\'' || byte == b',' {
                    break;
                }
                end += 1;
            }
        }
        if end == start {
            break;
        }
        let mut replaced = String::with_capacity(result.len() + 12);
        replaced.push_str(&result[..start]);
        replaced.push_str("[REDACTED]");
        replaced.push_str(&result[end..]);
        result = replaced;
        // Continue searching after the inserted marker. The keyword stays in
        // place, so start past it to avoid redacting the marker repeatedly.
        search_from = position + keyword.len() + "[REDACTED]".len();
        if search_from >= result.len() {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_layout(name: &str) -> PortableLayout {
        let root = std::env::temp_dir().join(format!(
            "reef-diagnostics-{name}-{}-{}",
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

    #[test]
    fn writes_jsonl_and_human_events_with_run_id() {
        let layout = test_layout("events");
        let diagnostics =
            Diagnostics::open(layout.clone(), "run-test-1234", Level::Trace, Vec::new())
                .expect("open diagnostics");
        diagnostics.event(
            Level::Info,
            "connection",
            "Connected",
            json!({ "host": "192.0.2.1", "duration_ms": 42 }),
        );
        diagnostics.close();

        let jsonl = diagnostics.jsonl_path().expect("jsonl path");
        let text = std::fs::read_to_string(&jsonl).expect("read jsonl");
        let event: DiagnosticEvent =
            serde_json::from_str(text.lines().next().expect("one event")).expect("parse event");
        assert_eq!(event.run_id, "run-test-1234");
        assert_eq!(event.level, "INFO");
        assert_eq!(event.category, "connection");
        assert_eq!(event.fields["duration_ms"], json!(42));
        assert!(event.timestamp.contains('T') && event.timestamp.ends_with('Z'));

        let human = std::fs::read_to_string(diagnostics.human_path().unwrap()).unwrap();
        assert!(human.contains("[INFO] [connection] Connected"));
        assert!(human.contains("host=192.0.2.1"));
        std::fs::remove_dir_all(&layout.root).expect("remove layout");
    }

    #[test]
    fn strips_ansi_sequences() {
        let input = "\u{1b}[31mred\u{1b}[0m text\u{1b}]0;title\u{7} done";
        assert_eq!(strip_ansi(input), "red text done");
    }

    #[test]
    fn redacts_common_secret_forms() {
        let samples = [
            "enable secret 5 $1$abcd$efgh",
            "username admin secret 5 $1$abcd$efgh",
            "snmp-server community public RO",
            "password hunter2",
            "set token=abc123",
            "Authorization: Bearer abcdef",
            "crypto key-string 0123456789",
        ];
        for sample in samples {
            let redacted = redact_text(sample);
            assert!(
                redacted.contains("[REDACTED]"),
                "expected redaction for {sample:?}, got {redacted:?}"
            );
            for secret in [
                "$1$abcd$efgh",
                "public",
                "hunter2",
                "abc123",
                "abcdef",
                "0123456789",
            ] {
                if sample.contains(secret) {
                    assert!(
                        !redacted.contains(secret),
                        "secret {secret:?} leaked from {sample:?}: {redacted:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn redacts_private_key_blocks() {
        let input = "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----\nafter\n";
        let redacted = redact_text(input);
        assert!(!redacted.contains("MIIabc"));
        assert!(redacted.contains("[REDACTED PRIVATE KEY BLOCK]"));
        assert!(redacted.contains("before"));
        assert!(redacted.contains("after"));
    }

    #[test]
    fn normal_show_output_is_not_redacted() {
        let input = "interface GigabitEthernet0/0/1\n description users\n";
        assert_eq!(redact_text(input), input);
    }

    #[test]
    fn rotates_diagnostic_files() {
        let layout = test_layout("rotation");
        let diagnostics =
            Diagnostics::open(layout.clone(), "run-rotate", Level::Trace, Vec::new()).unwrap();
        {
            let mut inner = diagnostics.inner.lock().unwrap();
            inner.jsonl.max_bytes = 512;
            inner.human.max_bytes = 512;
        }
        for index in 0..80 {
            diagnostics.event(
                Level::Info,
                "test",
                "rotation event",
                json!({ "index": index, "padding": "x".repeat(64) }),
            );
        }
        diagnostics.close();
        let parts = std::fs::read_dir(&layout.diagnostics)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("part"))
            .count();
        assert!(parts > 0, "expected at least one rotated part");
        std::fs::remove_dir_all(&layout.root).unwrap();
    }
}
