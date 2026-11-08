//! Device session: wraps a transport with IOS XE-specific interaction
//! primitives (enable, config_terminal, paging, prompt detection).

use crate::config::ConnectionConfig;
use crate::error::Error;
use crate::prompt::{detect, PromptMode};
use crate::transport::DeviceTransport;
use reef_engine::model::DeviceIdentity;
use reef_engine::parser::{parse_inventory, parse_version};
use std::time::Duration;

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
        }
    }

    /// Connect using the given configuration. After connecting,
    /// disables paging, performs login authentication, and detects the initial prompt.
    pub async fn connect(&mut self, config: &ConnectionConfig) -> Result<(), Error> {
        self.config = config.clone();
        self.transport.connect().await?;
        self.paging_disabled = true;
        self.perform_login().await?;
        let _ = self.refresh_prompt().await?;
        Ok(())
    }

    /// Perform login authentication (username/password) for transports that require it.
    async fn perform_login(&mut self) -> Result<(), Error> {
        // Read the initial banner and look for a username prompt.
        let out = self
            .transport
            .read_available(Duration::from_secs(5))
            .await?;

        // Check if we're already at a prompt (some devices don't require login).
        // The initial prompt may be consumed by this read, so record its state
        // here rather than relying on a later refresh to see it again.
        if let Some(prompt) = detect(&out) {
            if matches!(
                &prompt.mode,
                PromptMode::UserExec | PromptMode::PrivilegedExec
            ) {
                self.current_mode = prompt.mode.clone();
                self.authenticated = true;
                self.privileged = matches!(&prompt.mode, PromptMode::PrivilegedExec);
                if let Some(hostname) = extract_hostname(&prompt.raw) {
                    self.hostname = Some(hostname);
                }
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
            self.transport
                .send(format!("{}\r\n", self.config.username).as_str())
                .await?;
            let out2 = self.read_login_response().await?;
            prompt_output = out2.clone();
            if out2.to_lowercase().contains("password:") {
                if self.config.password.is_none() {
                    return Err(Error::AuthFailed);
                }
                self.transport
                    .send(format!("{}\r\n", self.config.password.as_ref().unwrap()).as_str())
                    .await?;
                prompt_output = self.read_login_response().await?;
            }
        }

        if let Some(prompt) = prompt_output.lines().rev().find_map(detect) {
            self.current_mode = prompt.mode.clone();
            self.authenticated = matches!(
                &prompt.mode,
                PromptMode::UserExec | PromptMode::PrivilegedExec
            );
            self.privileged = matches!(&prompt.mode, PromptMode::PrivilegedExec);
            if let Some(hostname) = extract_hostname(&prompt.raw) {
                self.hostname = Some(hostname);
            }
        }

        Ok(())
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
                    tracing::warn!(attempt, "reconnect attempt failed, retrying");
                    tokio::time::sleep(delay).await;
                    delay = delay.saturating_mul(2);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::Transport("max retries exceeded".into())))
    }

    /// Disconnect from the device.
    pub async fn disconnect(&mut self) -> Result<(), Error> {
        self.transport.disconnect().await?;
        self.authenticated = false;
        self.privileged = false;
        self.paging_disabled = false;
        self.current_mode = PromptMode::Unknown(String::new());
        self.hostname = None;
        Ok(())
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

            let chunk = self.transport.read_available(remaining).await?;
            if !chunk.is_empty() {
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

    /// Read from the transport until the current prompt is seen again.
    async fn read_until_prompt(&mut self) -> Result<String, Error> {
        let expect = match self.current_mode {
            PromptMode::PrivilegedExec => "#",
            PromptMode::UserExec => ">",
            PromptMode::GlobalConfig => "(config)#",
            PromptMode::InterfaceConfig => "(config-if)#",
            PromptMode::LineConfig => "(config-line)#",
            PromptMode::PasswordPrompt => "Password:",
            PromptMode::OtherConfig(_) => "#",
            PromptMode::Unknown(_) => "#",
        };

        let output = self
            .transport
            .read_until(expect, Duration::from_secs(10))
            .await?;
        Ok(output)
    }

    /// Send a line and read until the prompt returns.
    async fn send_and_read(&mut self, data: &str) -> Result<String, Error> {
        self.transport.send(data).await?;
        self.read_until_prompt().await
    }

    /// Refresh the detected prompt mode by reading available output.
    async fn refresh_prompt(&mut self) -> Result<(), Error> {
        let out = self
            .transport
            .read_available(Duration::from_millis(300))
            .await?;
        for line in out.lines() {
            if let Some(p) = detect(line) {
                self.current_mode = p.mode.clone();
                match &self.current_mode {
                    PromptMode::PrivilegedExec => {
                        self.privileged = true;
                        self.authenticated = true;
                    }
                    PromptMode::UserExec => {
                        self.privileged = false;
                        self.authenticated = true;
                    }
                    _ => {}
                }
                if matches!(
                    self.current_mode,
                    PromptMode::PrivilegedExec | PromptMode::UserExec
                ) {
                    if let Some(host) = extract_hostname(&p.raw) {
                        self.hostname = Some(host);
                    }
                }
            }
        }
        Ok(())
    }

    /// Escalate to privileged EXEC with the enable secret.
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

            let chunk = self.transport.read_available(remaining).await?;
            if !chunk.is_empty() {
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

            self.transport
                .send(format!("{secret}\r\n").as_str())
                .await?;
            let response = self
                .transport
                .read_until("#", Duration::from_secs(10))
                .await?;
            if response.to_lowercase().contains("% invalid input") {
                return Err(Error::PrivilegeEscalationFailed);
            }

            let Some(prompt) = response.lines().rev().find_map(detect) else {
                return Err(Error::PrivilegeEscalationFailed);
            };
            if !matches!(&prompt.mode, PromptMode::PrivilegedExec) {
                return Err(Error::PrivilegeEscalationFailed);
            }
            self.current_mode = prompt.mode.clone();
            if let Some(hostname) = extract_hostname(&prompt.raw) {
                self.hostname = Some(hostname);
            }
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
        Ok(())
    }

    /// Enter global configuration mode.
    pub async fn config_terminal(&mut self) -> Result<(), Error> {
        if !self.privileged {
            return Err(Error::PrivilegeEscalationFailed);
        }
        let _ = self.send_and_read("configure terminal\r\n").await?;
        self.refresh_prompt().await?;
        Ok(())
    }

    /// Exit configuration mode back to privileged EXEC.
    pub async fn exit_config(&mut self) -> Result<(), Error> {
        let _ = self.send_and_read("end\r\n").await?;
        self.refresh_prompt().await?;
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
        for cmd in cmds {
            let out = self.send_config(cmd).await?;
            combined.push_str(&out);
        }
        self.exit_config().await?;
        Ok(combined)
    }

    /// Identify the device: gather `show version`, `show inventory`, and
    /// parse into a `DeviceIdentity`.
    pub async fn identify(&mut self) -> Result<DeviceIdentity, Error> {
        if !self.privileged {
            return Err(Error::PrivilegeEscalationFailed);
        }
        let version_out = self.send_command("show version").await?;
        let mut identity = parse_version(&version_out).model;

        if let Some(ref h) = self.hostname {
            identity.hostname = h.clone();
        }

        let inv_out = self.send_command("show inventory").await?;
        parse_inventory(&mut identity, &inv_out);

        Ok(identity)
    }

    /// Save running-config to startup-config.
    pub async fn save_config(&mut self) -> Result<(), Error> {
        if !self.privileged {
            return Err(Error::PrivilegeEscalationFailed);
        }
        let _ = self
            .send_command("copy running-config startup-config")
            .await?;
        Ok(())
    }
}

fn extract_hostname(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let host = trimmed.trim_end_matches(|c| c == '#' || c == '>');
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_hostname_works() {
        assert_eq!(extract_hostname("Router#"), Some("Router".to_string()));
        assert_eq!(extract_hostname("Switch>"), Some("Switch".to_string()));
        assert_eq!(extract_hostname(""), None);
    }
}
