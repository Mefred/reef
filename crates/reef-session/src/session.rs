//! Device session: wraps a transport with IOS XE-specific interaction
//! primitives (enable, config_terminal, paging, prompt detection).
//!
//! Every managed command passes through [`crate::safety::check_command`] before
//! it reaches the transport. Startup-configuration writes are refused and
//! logged, while normal running-config edits and `show` commands continue to
//! work.

use crate::config::ConnectionConfig;
use crate::error::Error;
use crate::prompt::{detect, DetectedPrompt, PromptMode};
use crate::safety;
use crate::transport::{DeviceTransport, TransportMetadata};
use reef_engine::model::DeviceIdentity;
use reef_engine::parser::{parse_inventory, parse_version};
use reef_logging::record::{RecordingMetadata, RecordingStatus, SessionRecorder};
use serde_json::json;
use std::time::{Duration, Instant};

/// A session to a network device. Generic over the transport so the same
/// session logic works over SSH, Telnet, or Serial.
pub struct Session<T: DeviceTransport> {
    transport: T,
    config: ConnectionConfig,
    authenticated: bool,
    privileged: bool,
    paging_disabled: bool,
    current_mode: PromptMode,
    hostname: Option<String>,
    device_id: String,
    recorder: Option<SessionRecorder>,
    auto_record: bool,
    recording_error: Option<String>,
    command_count: u64,
    connected_at: Option<Instant>,
}

impl<T: DeviceTransport> Session<T> {
    pub fn new(transport: T, config: ConnectionConfig) -> Self {
        Session {
            transport,
            config,
            authenticated: false,
            privileged: false,
            paging_disabled: false,
            current_mode: PromptMode::Unknown(String::new()),
            hostname: None,
            device_id: String::new(),
            recorder: None,
            // Unit tests opt in explicitly so they do not write logs into the
            // build directory. The application and SessionManager enable it.
            auto_record: !cfg!(test),
            recording_error: None,
            command_count: 0,
            connected_at: None,
        }
    }

    pub fn with_device_id(mut self, device_id: impl Into<String>) -> Self {
        self.device_id = device_id.into();
        self
    }

    pub fn set_device_id(&mut self, device_id: impl Into<String>) {
        self.device_id = device_id.into();
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    pub fn set_auto_record(&mut self, enabled: bool) {
        self.auto_record = enabled;
    }

    pub fn command_count(&self) -> u64 {
        self.command_count
    }

    /// Connect using the given configuration. After connecting, performs login
    /// authentication, detects the initial prompt, and disables paging.
    pub async fn connect(&mut self, config: &ConnectionConfig) -> Result<(), Error> {
        let started = Instant::now();
        self.stop_recording();
        self.reset_session_state();
        self.config = config.clone();

        if self.auto_record {
            if let Err(error) = self.start_recording() {
                let error = error.to_string();
                self.recording_error = Some(error.clone());
                reef_logging::warn(
                    "recording",
                    "Session recording could not start",
                    json!({
                        "deviceId": self.device_id,
                        "error": error,
                    }),
                );
            }
        }

        reef_logging::info(
            "connection",
            "Connecting to device",
            json!({
                "deviceId": self.device_id,
                "transport": config.transport.as_str(),
                "host": config.host,
                "port": config.port,
                "username": config.username,
                "hostKeyPolicy": format!("{:?}", config.host_key_policy),
                "timeoutMs": config.timeout.as_millis() as u64,
            }),
        );

        if let Err(error) = self.transport.connect().await {
            // A transport may allocate resources before reporting a handshake
            // or initialization error. Give it a chance to release them.
            let _ = self.transport.disconnect().await;
            reef_logging::error(
                "connection",
                "Transport connection failed",
                json!({
                    "deviceId": self.device_id,
                    "durationMs": started.elapsed().as_millis() as u64,
                    "error": error.to_string(),
                }),
            );
            self.stop_recording();
            self.reset_session_state();
            return Err(error);
        }

        let metadata = self.transport.metadata();
        reef_logging::info(
            "connection",
            "Transport established",
            json!({
                "deviceId": self.device_id,
                "durationMs": started.elapsed().as_millis() as u64,
                "transport": metadata.transport,
                "details": metadata.details,
            }),
        );
        if let Some(recorder) = &self.recorder {
            recorder.note(&format!(
                "transport established: {} {:?}",
                metadata.transport, metadata.details
            ));
        }

        if let Err(error) = self.perform_login().await {
            // Authentication or prompt negotiation failed after the transport
            // opened. Do not leave a half-initialized connection behind.
            let _ = self.transport.disconnect().await;
            reef_logging::error(
                "authentication",
                "Device login failed",
                json!({
                    "deviceId": self.device_id,
                    "host": config.host,
                    "error": error.to_string(),
                }),
            );
            self.stop_recording();
            self.reset_session_state();
            return Err(error);
        }

        if let Err(error) = self.refresh_prompt().await {
            let _ = self.transport.disconnect().await;
            reef_logging::error(
                "prompt",
                "Initial prompt detection failed",
                json!({ "deviceId": self.device_id, "error": error.to_string() }),
            );
            self.stop_recording();
            self.reset_session_state();
            return Err(error);
        }

        // Paging can only be disabled after login. Sending it before login
        // corrupts the username/password exchange on many devices.
        self.disable_paging_after_login().await;

        self.connected_at = Some(Instant::now());
        reef_logging::info(
            "session",
            "Device session ready",
            json!({
                "deviceId": self.device_id,
                "durationMs": started.elapsed().as_millis() as u64,
                "hostname": self.hostname,
                "mode": format!("{:?}", self.current_mode),
                "privileged": self.privileged,
                "pagingDisabled": self.paging_disabled,
                "recording": self.recording_status().map(|status| status.active).unwrap_or(false),
            }),
        );
        if let Some(recorder) = &self.recorder {
            recorder.note(&format!(
                "session ready: hostname={} mode={:?} privileged={}",
                self.hostname.as_deref().unwrap_or("-"),
                self.current_mode,
                self.privileged
            ));
        }
        Ok(())
    }

    fn reset_session_state(&mut self) {
        self.authenticated = false;
        self.privileged = false;
        self.paging_disabled = false;
        self.current_mode = PromptMode::Unknown(String::new());
        self.hostname = None;
        self.connected_at = None;
    }

    /// Perform login authentication (username/password) for transports that require it.
    async fn perform_login(&mut self) -> Result<(), Error> {
        // Some serial console devices do not send a banner until they receive
        // input. Send a wake-up carriage return so the device starts talking.
        if self.transport.transport_type() == crate::transport::TransportType::Serial {
            let _ = self.transport.send("\r\n").await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        // Read the initial banner and look for a username prompt.
        let out = self
            .transport
            .read_available(Duration::from_secs(5))
            .await?;
        self.record_rx(&out);

        // Check if we're already at a prompt (some devices don't require login).
        // The initial prompt may be consumed by this read, so record its state
        // here rather than relying on a later refresh to see it again.
        if let Some(prompt) = detect(&out) {
            if matches!(
                &prompt.mode,
                PromptMode::UserExec | PromptMode::PrivilegedExec
            ) {
                self.apply_prompt(&prompt);
                return Ok(());
            }
        }

        // Send username if prompted. Keep the final response because reading
        // the prompt consumes it from the transport buffer.
        let mut prompt_output = out.clone();
        if out.to_lowercase().contains("username:") || out.to_lowercase().contains("login:") {
            if self.config.username.is_empty() {
                return Err(Error::AuthFailed);
            }
            let username_line = format!("{}\r\n", self.config.username);
            if let Some(recorder) = &self.recorder {
                recorder.record_input(&format!("<username> {}\r\n", self.config.username));
            }
            self.transport.send(&username_line).await?;
            let out2 = self.read_login_response().await?;
            prompt_output = out2.clone();
            if out2.to_lowercase().contains("password:") {
                if self.config.password.is_none() {
                    return Err(Error::AuthFailed);
                }
                if let Some(recorder) = &self.recorder {
                    recorder.note("login password submitted (value withheld)");
                }
                self.transport
                    .send(&format!(
                        "{}\r\n",
                        self.config.password.as_ref().expect("checked password")
                    ))
                    .await?;
                prompt_output = self.read_login_response().await?;
            }
        } else if out.to_lowercase().contains("password:") {
            if let Some(ref password) = self.config.password {
                if let Some(recorder) = &self.recorder {
                    recorder.note("login password submitted (value withheld)");
                }
                self.transport.send(&format!("{password}\r\n")).await?;
                prompt_output = self.read_login_response().await?;
            } else {
                return Err(Error::AuthFailed);
            }
        }

        if let Some(prompt) = prompt_output.lines().rev().find_map(detect) {
            self.apply_prompt(&prompt);
        }

        Ok(())
    }

    fn apply_prompt(&mut self, prompt: &DetectedPrompt) {
        self.current_mode = prompt.mode.clone();
        self.authenticated = matches!(
            &prompt.mode,
            PromptMode::UserExec
                | PromptMode::PrivilegedExec
                | PromptMode::GlobalConfig
                | PromptMode::InterfaceConfig
                | PromptMode::LineConfig
                | PromptMode::OtherConfig(_)
        );
        self.privileged = !matches!(&prompt.mode, PromptMode::UserExec);
        if let Some(hostname) = extract_hostname(&prompt.raw) {
            if matches!(
                prompt.mode,
                PromptMode::PrivilegedExec | PromptMode::UserExec
            ) {
                self.hostname = Some(hostname);
            }
        }
    }

    /// Reconnect with exponential backoff. Re-uses the transport's stored
    /// configuration. Returns the last error if all retries are exhausted.
    pub async fn reconnect(&mut self, max_retries: u32) -> Result<(), Error> {
        let mut delay = Duration::from_secs(1);
        let mut last_err = None;
        for attempt in 1..=max_retries {
            let config = self.config.clone();
            match self.connect(&config).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_err = Some(e);
                    if attempt == max_retries {
                        break;
                    }
                    reef_logging::warn(
                        "connection",
                        "Reconnect attempt failed",
                        json!({
                            "deviceId": self.device_id,
                            "attempt": attempt,
                            "retryDelayMs": delay.as_millis() as u64,
                        }),
                    );
                    tokio::time::sleep(delay).await;
                    delay = delay.saturating_mul(2);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Transport("max retries exceeded".into())))
    }

    /// Disconnect from the device. Any partial recording is flushed and
    /// closed, preserving output captured before an unexpected end.
    pub async fn disconnect(&mut self) -> Result<(), Error> {
        let session_duration = self
            .connected_at
            .map(|started| started.elapsed().as_millis() as u64);
        let result = self.transport.disconnect().await;
        self.stop_recording();
        reef_logging::info(
            "session",
            "Device session closed",
            json!({
                "deviceId": self.device_id,
                "hostname": self.hostname,
                "durationMs": session_duration,
                "commands": self.command_count,
                "transportError": result.as_ref().err().map(|error| error.to_string()),
            }),
        );
        // Clear local authentication and prompt state even if the underlying
        // transport reports a close error. The session must not appear usable.
        self.reset_session_state();
        result
    }

    pub fn is_connected(&self) -> bool {
        self.transport.is_connected()
    }

    pub fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    pub fn is_privileged(&self) -> bool {
        self.privileged
    }

    pub fn transport_type(&self) -> crate::transport::TransportType {
        self.transport.transport_type()
    }

    pub fn transport_metadata(&self) -> TransportMetadata {
        self.transport.metadata()
    }

    /// Read login output until a password request or an EXEC prompt appears.
    /// Login prompts cannot use read_until_prompt because the current CLI mode
    /// is not known yet and the next response may be "Password:" rather than
    /// the normal EXEC prompt character.
    async fn read_login_response(&mut self) -> Result<String, Error> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut output = String::new();

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }

            let chunk = self
                .transport
                .read_available(remaining.min(Duration::from_millis(250)))
                .await?;
            if !chunk.is_empty() {
                self.record_rx(&chunk);
                output.push_str(&chunk);
            }

            if output.to_lowercase().contains("password:") {
                return Ok(output);
            }

            if output
                .lines()
                .rev()
                .find_map(detect)
                .map_or(false, |prompt| {
                    matches!(
                        &prompt.mode,
                        PromptMode::UserExec | PromptMode::PrivilegedExec
                    )
                })
            {
                return Ok(output);
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// Read from the transport until the current prompt is seen again. Every
    /// chunk is written to the session recording before the next read, so a
    /// timeout or disconnect does not discard output that already arrived.
    async fn read_until_prompt(&mut self, timeout: Duration) -> Result<String, Error> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut accumulated = String::new();

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            let chunk = self
                .transport
                .read_available(remaining.min(Duration::from_millis(250)))
                .await?;
            if !chunk.is_empty() {
                self.record_rx(&chunk);
                accumulated.push_str(&chunk);
                let tail = accumulated.rsplit('\n').next().unwrap_or(&accumulated);
                if let Some(prompt) = detect(tail) {
                    self.apply_prompt(&prompt);
                    return Ok(accumulated);
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn send_and_read(&mut self, data: &str) -> Result<String, Error> {
        self.send_and_read_with_timeout(data, Duration::from_secs(10))
            .await
    }

    async fn send_and_read_with_timeout(
        &mut self,
        data: &str,
        timeout: Duration,
    ) -> Result<String, Error> {
        self.guard_command(data)?;
        let started = Instant::now();
        let trimmed = data.trim();
        reef_logging::debug(
            "terminal",
            "Sending command",
            json!({
                "deviceId": self.device_id,
                "command": reef_logging::redact_text(trimmed),
                "bytes": data.len(),
            }),
        );
        if let Some(recorder) = &self.recorder {
            recorder.record_input(data);
        }

        if let Err(error) = self.transport.send(data).await {
            reef_logging::error(
                "terminal",
                "Command send failed",
                json!({
                    "deviceId": self.device_id,
                    "command": reef_logging::redact_text(trimmed),
                    "error": error.to_string(),
                }),
            );
            if let Some(recorder) = &self.recorder {
                recorder.note(&format!("command send failed: {error}"));
            }
            return Err(error);
        }

        let output = match self.read_until_prompt(timeout).await {
            Ok(output) => output,
            Err(error) => {
                reef_logging::warn(
                    "terminal",
                    "Command did not complete",
                    json!({
                        "deviceId": self.device_id,
                        "command": reef_logging::redact_text(trimmed),
                        "durationMs": started.elapsed().as_millis() as u64,
                        "error": error.to_string(),
                    }),
                );
                if let Some(recorder) = &self.recorder {
                    recorder.note(&format!("command did not complete: {error}"));
                }
                return Err(error);
            }
        };

        self.command_count = self.command_count.saturating_add(1);
        reef_logging::debug(
            "performance",
            "Command completed",
            json!({
                "deviceId": self.device_id,
                "durationMs": started.elapsed().as_millis() as u64,
                "outputBytes": output.len(),
                "commands": self.command_count,
            }),
        );
        Ok(output)
    }

    /// Enforce the mandatory startup-config write policy and log a blocked
    /// attempt without recording the command value verbatim.
    fn guard_command(&self, data: &str) -> Result<(), Error> {
        match safety::check_command(data) {
            Ok(()) => Ok(()),
            Err(violation) => {
                reef_logging::warn(
                    "safety",
                    "Blocked startup-config write attempt",
                    json!({
                        "deviceId": self.device_id,
                        "host": self.config.host,
                        "rule": violation.rule,
                        "reason": violation.reason,
                        "command": reef_logging::redact_text(data.trim()),
                        "blocked": true,
                    }),
                );
                if let Some(recorder) = &self.recorder {
                    recorder.note(&format!(
                        "BLOCKED startup-config write attempt ({}): {}",
                        violation.rule,
                        reef_logging::redact_text(data.trim())
                    ));
                }
                Err(violation.error())
            }
        }
    }

    fn record_rx(&self, data: &str) {
        if data.is_empty() {
            return;
        }
        if let Some(recorder) = &self.recorder {
            recorder.record_output(data);
        }
    }

    async fn refresh_prompt(&mut self) -> Result<(), Error> {
        let out = self
            .transport
            .read_available(Duration::from_millis(300))
            .await?;
        self.record_rx(&out);
        for line in out.lines() {
            if let Some(prompt) = detect(line) {
                self.apply_prompt(&prompt);
            }
        }
        Ok(())
    }

    async fn disable_paging_after_login(&mut self) {
        match self
            .send_and_read_with_timeout("terminal length 0\r\n", Duration::from_secs(3))
            .await
        {
            Ok(output) => {
                self.paging_disabled = true;
                if output.contains('%') {
                    reef_logging::warn(
                        "session",
                        "Device rejected terminal length 0",
                        json!({
                            "deviceId": self.device_id,
                            "output": reef_logging::redact_text(output.trim()),
                        }),
                    );
                }
            }
            Err(error) => {
                reef_logging::warn(
                    "session",
                    "Could not disable paging",
                    json!({ "deviceId": self.device_id, "error": error.to_string() }),
                );
            }
        }
    }

    /// Escalate to privileged EXEC with the enable secret. The secret is sent
    /// to the device but never written to logs or the recording.
    pub async fn enable(&mut self, secret: &str) -> Result<(), Error> {
        if self.privileged {
            return Ok(());
        }

        self.transport.send("enable\r\n").await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut output = String::new();
        let mut prompt_mode = None;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }

            let chunk = self
                .transport
                .read_available(remaining.min(Duration::from_millis(250)))
                .await?;
            if !chunk.is_empty() {
                self.record_rx(&chunk);
                output.push_str(&chunk);
            }

            if output.to_lowercase().contains("password:") {
                break;
            }

            if let Some(prompt) = output.lines().rev().find_map(detect) {
                if matches!(
                    &prompt.mode,
                    PromptMode::UserExec | PromptMode::PrivilegedExec
                ) {
                    prompt_mode = Some(prompt.mode);
                    break;
                }
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        if output.to_lowercase().contains("password:") {
            if secret.is_empty() {
                return Err(Error::PrivilegeEscalationFailed);
            }
            if let Some(recorder) = &self.recorder {
                recorder.note("enable secret submitted (value withheld)");
            }

            self.transport.send(&format!("{secret}\r\n")).await?;
            let response = match self.read_until_prompt(Duration::from_secs(10)).await {
                Ok(response) => response,
                Err(error) => {
                    reef_logging::warn(
                        "authentication",
                        "Enable escalation did not complete",
                        json!({ "deviceId": self.device_id, "error": error.to_string() }),
                    );
                    return Err(error);
                }
            };
            if response.to_lowercase().contains("% invalid input")
                || response.to_lowercase().contains("% bad secrets")
            {
                return Err(Error::PrivilegeEscalationFailed);
            }

            let Some(prompt) = response.lines().rev().find_map(detect) else {
                return Err(Error::PrivilegeEscalationFailed);
            };
            if !matches!(&prompt.mode, PromptMode::PrivilegedExec) {
                return Err(Error::PrivilegeEscalationFailed);
            }
            self.apply_prompt(&prompt);
        } else if let Some(mode) = prompt_mode {
            if !matches!(&mode, PromptMode::PrivilegedExec) {
                return Err(Error::PrivilegeEscalationFailed);
            }
            self.current_mode = mode;
        } else {
            return Err(Error::PrivilegeEscalationFailed);
        }

        self.privileged = true;
        self.authenticated = true;
        reef_logging::info(
            "authentication",
            "Privileged EXEC enabled",
            json!({ "deviceId": self.device_id, "hostname": self.hostname }),
        );
        Ok(())
    }

    /// Enter global configuration mode.
    pub async fn config_terminal(&mut self) -> Result<(), Error> {
        if !self.privileged {
            return Err(Error::PrivilegeEscalationFailed);
        }
        self.send_and_read("configure terminal\r\n").await?;
        Ok(())
    }

    /// Exit configuration mode back to privileged EXEC.
    pub async fn exit_config(&mut self) -> Result<(), Error> {
        self.send_and_read("end\r\n").await?;
        Ok(())
    }

    /// Execute a single command in privileged EXEC mode and return output.
    pub async fn send_command(&mut self, cmd: &str) -> Result<String, Error> {
        if !self.privileged {
            return Err(Error::PrivilegeEscalationFailed);
        }
        let out = self.send_and_read(&format!("{}\r\n", cmd)).await?;
        Ok(out)
    }

    /// Execute a single command in global configuration mode.
    pub async fn send_config(&mut self, cmd: &str) -> Result<String, Error> {
        if !self.privileged {
            return Err(Error::PrivilegeEscalationFailed);
        }
        let out = self.send_and_read(&format!("{}\r\n", cmd)).await?;
        Ok(out)
    }

    /// Execute a batch of configuration commands in one config session.
    pub async fn send_config_batch(&mut self, cmds: &[String]) -> Result<String, Error> {
        if !self.privileged {
            return Err(Error::PrivilegeEscalationFailed);
        }
        self.config_terminal().await?;
        let mut combined = String::new();
        let mut first_error = None;
        for cmd in cmds {
            match self.send_config(cmd).await {
                Ok(out) => combined.push_str(&out),
                Err(error) => {
                    first_error = Some(error);
                    break;
                }
            }
        }
        // Always attempt to leave configuration mode, even when a command
        // failed, so the session is not left in a sub-mode.
        if let Err(error) = self.exit_config().await {
            if first_error.is_none() {
                first_error = Some(error);
            } else {
                reef_logging::warn(
                    "session",
                    "Could not leave configuration mode after a failed batch",
                    json!({ "deviceId": self.device_id, "error": error.to_string() }),
                );
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(combined),
        }
    }

    /// Identify the device: gather `show version`, `show inventory`, and
    /// parse into a `DeviceIdentity`.
    pub async fn identify(&mut self) -> Result<DeviceIdentity, Error> {
        if !self.privileged {
            return Err(Error::PrivilegeEscalationFailed);
        }
        let started = Instant::now();
        let version_out = self.send_command("show version").await?;
        let mut identity = parse_version(&version_out).model;

        if let Some(ref h) = self.hostname {
            identity.hostname = h.clone();
        }

        let inv_out = self.send_command("show inventory").await?;
        parse_inventory(&mut identity, &inv_out);
        reef_logging::debug(
            "device",
            "Device identified",
            json!({
                "deviceId": self.device_id,
                "hostname": identity.hostname,
                "platform": identity.platform,
                "iosxeVersion": identity.iosxe_version,
                "durationMs": started.elapsed().as_millis() as u64,
            }),
        );

        Ok(identity)
    }

    /// Send a Ctrl+C (0x03) interrupt to the device. Used by the terminal UI
    /// when the user cancels a command; it is not a configuration command.
    pub async fn send_interrupt(&mut self) -> Result<(), Error> {
        self.transport.send_raw(b"\x03").await?;
        if let Some(recorder) = &self.recorder {
            recorder.note("TX interrupt (Ctrl+C)");
        }
        reef_logging::info(
            "terminal",
            "Interrupt sent to device",
            json!({ "deviceId": self.device_id }),
        );
        Ok(())
    }

    /// Start recording this session's terminal I/O.
    pub fn start_recording(&mut self) -> Result<RecordingStatus, Error> {
        if let Some(recorder) = &self.recorder {
            return Ok(recorder.status());
        }
        let layout = reef_logging::paths::layout().map_err(|error| {
            Error::Io(std::io::Error::other(format!(
                "portable layout is unavailable: {error}"
            )))
        })?;
        let metadata = RecordingMetadata {
            run_id: reef_logging::run_id().unwrap_or_else(|| "unknown-run".to_string()),
            device_id: if self.device_id.is_empty() {
                self.config.host.clone()
            } else {
                self.device_id.clone()
            },
            host: self.config.host.clone(),
            port: self.config.port,
            transport: self.config.transport.as_str().to_string(),
            username: self.config.username.clone(),
            hostname: self.hostname.clone(),
        };
        let recorder = SessionRecorder::start(layout, metadata)
            .map_err(|error| Error::Io(std::io::Error::other(error)))?;
        let status = recorder.status();
        self.recorder = Some(recorder);
        self.recording_error = None;
        reef_logging::info(
            "recording",
            "Session recording started",
            json!({
                "deviceId": self.device_id,
                "path": status.path,
            }),
        );
        Ok(status)
    }

    /// Stop recording and return the final counters.
    pub fn stop_recording(&mut self) -> Option<RecordingStatus> {
        let recorder = self.recorder.as_ref()?;
        recorder.stop();
        let status = recorder.status();
        reef_logging::info(
            "recording",
            "Session recording stopped",
            json!({
                "deviceId": self.device_id,
                "path": status.path,
                "bytes": status.bytes,
                "parts": status.parts,
                "redactions": status.redactions,
                "sizeLimitReached": status.size_limit_reached,
            }),
        );
        Some(status)
    }

    pub fn recording_status(&self) -> Option<RecordingStatus> {
        self.recorder.as_ref().map(SessionRecorder::status)
    }

    pub fn recording_path(&self) -> Option<String> {
        self.recorder
            .as_ref()
            .and_then(SessionRecorder::path)
            .map(|path| path.to_string_lossy().to_string())
    }

    pub fn recording_error(&self) -> Option<&str> {
        self.recording_error.as_deref()
    }
}

fn extract_hostname(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let host = trimmed.trim_end_matches(|c| c == '#' || c == '>');
    let host = host.split('(').next().unwrap_or(host).trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{DeviceTransport, TransportType};
    use std::collections::VecDeque;
    use std::time::Duration;

    struct TestTransport {
        connected: bool,
        initial_output: String,
        connect_fails: bool,
        disconnect_fails: bool,
        disconnect_calls: usize,
        sent: Vec<String>,
        responses: VecDeque<String>,
        privileged_prompt: bool,
    }

    impl TestTransport {
        fn new(initial_output: &str) -> Self {
            Self {
                connected: false,
                initial_output: initial_output.to_string(),
                connect_fails: false,
                disconnect_fails: false,
                disconnect_calls: 0,
                sent: Vec::new(),
                responses: VecDeque::new(),
                privileged_prompt: true,
            }
        }
    }

    impl DeviceTransport for TestTransport {
        async fn connect(&mut self) -> Result<(), Error> {
            self.connected = true;
            if self.connect_fails {
                return Err(Error::Transport("simulated handshake failure".to_string()));
            }
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), Error> {
            self.disconnect_calls += 1;
            if self.disconnect_fails {
                return Err(Error::Transport("simulated close failure".to_string()));
            }
            self.connected = false;
            Ok(())
        }

        async fn send(&mut self, data: &str) -> Result<(), Error> {
            self.sent.push(data.to_string());
            Ok(())
        }

        async fn send_raw(&mut self, _bytes: &[u8]) -> Result<(), Error> {
            Ok(())
        }

        async fn read_until(
            &mut self,
            _pattern: &str,
            _timeout: Duration,
        ) -> Result<String, Error> {
            Ok(self.responses.pop_front().unwrap_or_default())
        }

        async fn read_available(&mut self, _timeout: Duration) -> Result<String, Error> {
            if let Some(response) = self.responses.pop_front() {
                return Ok(response);
            }
            Ok(std::mem::take(&mut self.initial_output))
        }

        fn is_connected(&self) -> bool {
            self.connected
        }

        fn transport_type(&self) -> TransportType {
            TransportType::Telnet
        }
    }

    #[tokio::test]
    async fn failed_transport_setup_attempts_to_release_partial_resources() {
        let mut transport = TestTransport::new("");
        transport.connect_fails = true;
        let mut session = Session::new(transport, ConnectionConfig::default());

        let result = session.connect(&ConnectionConfig::default()).await;

        assert!(matches!(result, Err(Error::Transport(_))));
        assert!(!session.is_authenticated());
        assert!(!session.is_privileged());
        assert_eq!(session.transport.disconnect_calls, 1);
    }

    #[tokio::test]
    async fn failed_login_closes_transport_and_resets_session_state() {
        let transport = TestTransport::new("Username:");
        let mut session = Session::new(transport, ConnectionConfig::default());

        let result = session.connect(&ConnectionConfig::default()).await;

        assert!(matches!(result, Err(Error::AuthFailed)));
        assert!(!session.is_connected());
        assert!(!session.is_authenticated());
        assert!(!session.is_privileged());
        assert_eq!(session.transport.disconnect_calls, 1);
        assert!(matches!(session.current_mode, PromptMode::Unknown(_)));
        assert!(session.hostname.is_none());
    }

    #[tokio::test]
    async fn disconnect_resets_authentication_even_when_transport_close_fails() {
        let mut transport = TestTransport::new("");
        transport.connected = true;
        transport.disconnect_fails = true;
        let mut session = Session::new(transport, ConnectionConfig::default());
        session.authenticated = true;
        session.privileged = true;
        session.paging_disabled = true;
        session.current_mode = PromptMode::PrivilegedExec;
        session.hostname = Some("router".to_string());

        let result = session.disconnect().await;

        assert!(matches!(result, Err(Error::Transport(_))));
        assert!(!session.is_authenticated());
        assert!(!session.is_privileged());
        assert!(!session.paging_disabled);
        assert!(matches!(session.current_mode, PromptMode::Unknown(_)));
        assert!(session.hostname.is_none());
        assert_eq!(session.transport.disconnect_calls, 1);
    }

    #[tokio::test]
    async fn startup_config_writes_never_reach_the_transport() {
        let mut transport = TestTransport::new("Router#");
        transport.connected = true;
        let mut session = Session::new(transport, ConnectionConfig::default());
        session.privileged = true;
        session.authenticated = true;
        session.current_mode = PromptMode::PrivilegedExec;

        for command in [
            "write memory",
            "copy running-config startup-config",
            "copy run start",
            "erase startup-config",
        ] {
            let error = session
                .send_command(command)
                .await
                .expect_err("startup write must be blocked");
            assert!(matches!(error, Error::StartupConfigWriteBlocked { .. }));
        }

        assert!(
            session.transport.sent.is_empty(),
            "blocked commands reached the transport: {:?}",
            session.transport.sent
        );
    }

    #[tokio::test]
    async fn normal_commands_are_sent_and_recorded() {
        let mut transport = TestTransport::new("Router#");
        transport.connected = true;
        transport
            .responses
            .push_back("Cisco IOS XE\r\nRouter#".to_string());
        let mut session = Session::new(transport, ConnectionConfig::default());
        session.privileged = true;
        session.authenticated = true;
        session.current_mode = PromptMode::PrivilegedExec;

        let output = session.send_command("show version").await.unwrap();
        assert!(output.contains("Cisco IOS XE"));
        assert_eq!(session.transport.sent, vec!["show version\r\n"]);
        assert_eq!(session.command_count, 1);
    }

    #[test]
    fn extract_hostname_works() {
        assert_eq!(extract_hostname("Router#"), Some("Router".to_string()));
        assert_eq!(extract_hostname("Switch>"), Some("Switch".to_string()));
        assert_eq!(
            extract_hostname("Router(config)#"),
            Some("Router".to_string())
        );
        assert_eq!(extract_hostname(""), None);
    }
}
