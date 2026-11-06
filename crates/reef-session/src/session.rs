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
    authenticated: bool,
    privileged: bool,
    paging_disabled: bool,
    current_mode: PromptMode,
    hostname: Option<String>,
}

impl<T: DeviceTransport> Session<T> {
    pub fn new(transport: T) -> Self {
        Session {
            transport,
            authenticated: false,
            privileged: false,
            paging_disabled: false,
            current_mode: PromptMode::Unknown(String::new()),
            hostname: None,
        }
    }

    /// Connect using the transport's configured connection. After connecting,
    /// disables paging and detects the initial prompt.
    pub async fn connect(&mut self) -> Result<(), Error> {
        self.transport.connect().await?;
        self.paging_disabled = true;
        let _ = self.refresh_prompt().await?;
        Ok(())
    }

    /// Reconnect with exponential backoff. Re-uses the transport's stored
    /// configuration. Returns the last error if all retries are exhausted.
    pub async fn reconnect(&mut self, max_retries: u32) -> Result<(), Error> {
        let mut delay = Duration::from_secs(1);
        let mut last_err = None;
        for attempt in 1..=max_retries {
            match self.connect().await {
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

        let output = self.transport.read_until(expect, Duration::from_secs(10)).await?;
        Ok(output)
    }

    /// Send a line and read until the prompt returns.
    async fn send_and_read(&mut self, data: &str) -> Result<String, Error> {
        self.transport.send(data).await?;
        self.read_until_prompt().await
    }

    /// Refresh the detected prompt mode by reading available output.
    async fn refresh_prompt(&mut self) -> Result<(), Error> {
        let out = self.transport.read_available(Duration::from_millis(300)).await?;
        for line in out.lines() {
            if let Some(p) = detect(line) {
                self.current_mode = p.mode.clone();
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
        let out = self.send_and_read("enable\r\n").await?;
        let is_password_prompt =
            out.contains("Password:") || detect(&out).map_or(false, |p| matches!(p.mode, PromptMode::PasswordPrompt));
        if is_password_prompt {
            self.transport.send(format!("{}\r\n", secret).as_str()).await?;
            let out2 = self.read_until_prompt().await?;
            if out2.contains("% Invalid input") || !out2.contains("#") {
                return Err(Error::PrivilegeEscalationFailed);
            }
        }
        self.refresh_prompt().await?;
        if matches!(self.current_mode, PromptMode::PrivilegedExec) {
            self.privileged = true;
            self.authenticated = true;
            Ok(())
        } else {
            Err(Error::PrivilegeEscalationFailed)
        }
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
        let version_out = self.send_command("show version\r\n").await?;
        let mut identity = parse_version(&version_out).model;

        if let Some(ref h) = self.hostname {
            identity.hostname = h.clone();
        }

        let inv_out = self.send_command("show inventory\r\n").await?;
        parse_inventory(&mut identity, &inv_out);

        Ok(identity)
    }

    /// Save running-config to startup-config.
    pub async fn save_config(&mut self) -> Result<(), Error> {
        if !self.privileged {
            return Err(Error::PrivilegeEscalationFailed);
        }
        let _ = self.send_command("copy running-config startup-config\r\n").await?;
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
