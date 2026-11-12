//! SSH transport implementation using `ssh2-rs` (blocking SSH) wrapped in
//! `tokio::task::spawn_blocking` so the `DeviceTransport` trait stays async.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::ConnectionConfig;
use crate::error::Error;
use crate::transport::{DeviceTransport, TransportMetadata, TransportType};
use serde::{Deserialize, Serialize};

/// Internal shared state for an SSH connection.
struct Inner {
    config: ConnectionConfig,
    session: Option<ssh2::Session>,
    channel: Option<ssh2::Channel>,
    connected: bool,
    authenticated: bool,
    metadata: BTreeMap<String, String>,
}

pub struct SshTransport {
    inner: Arc<Mutex<Inner>>,
}

impl SshTransport {
    pub fn new(config: ConnectionConfig) -> Self {
        SshTransport {
            inner: Arc::new(Mutex::new(Inner {
                config,
                session: None,
                channel: None,
                connected: false,
                authenticated: false,
                metadata: BTreeMap::new(),
            })),
        }
    }

    /// Run a blocking closure against the SSH inner state.
    async fn with_inner<F, R>(&self, f: F) -> Result<R, Error>
    where
        F: FnOnce(&mut Inner) -> Result<R, Error> + Send + 'static,
        R: Send + 'static,
    {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = inner.lock().map_err(|e| Error::Transport(e.to_string()))?;
            f(&mut guard)
        })
        .await
        .map_err(|e| Error::Transport(e.to_string()))?
    }
}

impl DeviceTransport for SshTransport {
    async fn connect(&mut self) -> Result<(), Error> {
        let config = {
            let guard = self
                .inner
                .lock()
                .map_err(|e| Error::Transport(e.to_string()))?;
            guard.config.clone()
        };

        self.with_inner(move |inner| {
            let endpoint = format!("{}:{}", config.host, config.port);
            tracing::debug!(%endpoint, "ssh: resolving endpoint");
            let addresses = (config.host.as_str(), config.port)
                .to_socket_addrs()
                .map_err(|error| Error::Transport(format!("cannot resolve {endpoint}: {error}")))?
                .collect::<Vec<_>>();
            if addresses.is_empty() {
                return Err(Error::Transport(format!(
                    "cannot resolve {endpoint}: no addresses"
                )));
            }
            let mut last_error = None;
            let mut stream = None;
            for address in addresses {
                match TcpStream::connect_timeout(&address, config.timeout) {
                    Ok(candidate) => {
                        stream = Some(candidate);
                        break;
                    }
                    Err(error) => last_error = Some(error),
                }
            }
            let stream = stream.ok_or_else(|| {
                Error::Transport(format!(
                    "tcp connect {endpoint}: {}",
                    last_error
                        .map(|error| error.to_string())
                        .unwrap_or_else(|| "no route".to_string())
                ))
            })?;
            let _ = stream.set_nodelay(true);

            let mut sess =
                ssh2::Session::new().map_err(|e| Error::Transport(format!("ssh2 new: {}", e)))?;
            sess.set_tcp_stream(stream);
            sess.set_blocking(true);

            // Offer legacy key-exchange algorithms commonly used by Cisco IOS
            // devices alongside modern ones. These SHA1-based KEX algorithms
            // are compiled out of libssh2 by default; the LIBSSH2_KEX_SHA1_ENABLE
            // define must be passed to the C compiler when building libssh2-sys
            // (see the Windows CI job and build instructions). PuTTY offers
            // these by default, which is why a device may connect with PuTTY
            // but not with Reef.
            if let Err(error) = sess.method_pref(
                ssh2::MethodType::Kex,
                "curve25519-sha256@libssh.org,curve25519-sha256,ecdh-sha2-nistp256,ecdh-sha2-nistp384,ecdh-sha2-nistp521,diffie-hellman-group14-sha256,diffie-hellman-group14-sha1,diffie-hellman-group1-sha1,diffie-hellman-group-exchange-sha256,diffie-hellman-group-exchange-sha1",
            ) {
                tracing::warn!(
                    host = %config.host,
                    "ssh: could not set key-exchange preference: {error}"
                );
            }

            sess.handshake()
                .map_err(|e| Error::Transport(format!("ssh handshake: {}", e)))?;

            // Host key verification must happen before authentication. The
            // policy is never bypassed and a changed key is always rejected.
            let host_key = verify_host_key(&sess, &config)?;

            let mut metadata = BTreeMap::new();
            metadata.insert("hostKeyType".to_string(), host_key.key_type.clone());
            metadata.insert(
                "hostKeyFingerprintSha256".to_string(),
                host_key.fingerprint_sha256.clone(),
            );
            if let Some(kex) = sess.methods(ssh2::MethodType::Kex) {
                metadata.insert("kex".to_string(), kex.to_string());
            }
            if let Some(cipher) = sess.methods(ssh2::MethodType::CryptCs) {
                metadata.insert("cipherClientToServer".to_string(), cipher.to_string());
            }
            if let Some(cipher) = sess.methods(ssh2::MethodType::CryptSc) {
                metadata.insert("cipherServerToClient".to_string(), cipher.to_string());
            }
            if let Some(mac) = sess.methods(ssh2::MethodType::MacCs) {
                metadata.insert("macClientToServer".to_string(), mac.to_string());
            }
            if let Some(mac) = sess.methods(ssh2::MethodType::MacSc) {
                metadata.insert("macServerToClient".to_string(), mac.to_string());
            }

            // Authentication: password first, then keyboard-interactive as a
            // fallback for devices that expose only that method. Secrets are
            // never written to logs or recording files.
            let mut authenticated = false;
            if let Some(ref password) = config.password {
                if sess.userauth_password(&config.username, password).is_ok() {
                    authenticated = true;
                    metadata.insert("authMethod".to_string(), "password".to_string());
                }
            }
            if !authenticated {
                let methods = sess
                    .auth_methods(&config.username)
                    .unwrap_or("")
                    .to_string();
                if methods.contains("keyboard-interactive") {
                    if let Some(ref password) = config.password {
                        let mut prompter = KeyboardInteractivePassword {
                            password: password.clone(),
                        };
                        if sess
                            .userauth_keyboard_interactive(&config.username, &mut prompter)
                            .is_ok()
                        {
                            authenticated = true;
                            metadata.insert(
                                "authMethod".to_string(),
                                "keyboard-interactive".to_string(),
                            );
                        }
                    }
                }
            }
            if !authenticated {
                return Err(Error::AuthFailed);
            }

            // Open a shell channel for interactive CLI.
            let mut channel = sess
                .channel_session()
                .map_err(|e| Error::Transport(format!("channel: {}", e)))?;
            channel
                .request_pty("vt100", None, None)
                .map_err(|e| Error::Transport(format!("pty: {}", e)))?;
            channel
                .shell()
                .map_err(|e| Error::Transport(format!("shell: {}", e)))?;

            inner.session = Some(sess);
            inner.channel = Some(channel);
            inner.connected = true;
            inner.authenticated = true;
            inner.metadata = metadata;
            Ok(())
        })
        .await
    }

    async fn disconnect(&mut self) -> Result<(), Error> {
        self.with_inner(|inner| {
            if let Some(mut ch) = inner.channel.take() {
                let _ = ch.close();
                let _ = ch.wait_close();
            }
            if let Some(sess) = inner.session.take() {
                let _ = sess.disconnect(None, "bye", None);
            }
            inner.connected = false;
            inner.authenticated = false;
            inner.metadata.clear();
            Ok(())
        })
        .await
    }

    async fn send(&mut self, data: &str) -> Result<(), Error> {
        let data = data.to_string();
        self.with_inner(move |inner| {
            let timeout_ms = inner.config.timeout.as_millis().min(u128::from(u32::MAX)) as u32;
            if let Some(session) = inner.session.as_ref() {
                session.set_timeout(timeout_ms);
            }
            let ch = inner.channel.as_mut().ok_or_else(|| Error::NotConnected)?;
            ch.write_all(data.as_bytes())
                .map_err(|e| Error::Transport(format!("ssh write: {}", e)))?;
            ch.flush()
                .map_err(|e| Error::Transport(format!("ssh flush: {}", e)))?;
            Ok(())
        })
        .await
    }

    async fn send_raw(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let bytes = bytes.to_vec();
        self.with_inner(move |inner| {
            let timeout_ms = inner.config.timeout.as_millis().min(u128::from(u32::MAX)) as u32;
            if let Some(session) = inner.session.as_ref() {
                session.set_timeout(timeout_ms);
            }
            let ch = inner.channel.as_mut().ok_or_else(|| Error::NotConnected)?;
            ch.write_all(&bytes)
                .map_err(|e| Error::Transport(format!("ssh write: {}", e)))?;
            ch.flush()
                .map_err(|e| Error::Transport(format!("ssh flush: {}", e)))?;
            Ok(())
        })
        .await
    }

    async fn read_available(&mut self, _timeout: Duration) -> Result<String, Error> {
        self.with_inner(|inner| {
            if let Some(session) = inner.session.as_ref() {
                // Keep a blocking SSH session but bound each read so the
                // session actor can observe timeouts and shutdown requests.
                session.set_timeout(250);
            }
            let ch = inner.channel.as_mut().ok_or_else(|| Error::NotConnected)?;
            let mut buf = [0u8; 4096];
            match ch.read(&mut buf) {
                Ok(0) => Ok(String::new()),
                Ok(n) => Ok(String::from_utf8_lossy(&buf[..n]).to_string()),
                Err(ref e) if is_io_timeout(e) => Ok(String::new()),
                Err(e) => Err(Error::Transport(format!("ssh read: {}", e))),
            }
        })
        .await
    }

    async fn read_until(&mut self, pattern: &str, timeout: Duration) -> Result<String, Error> {
        let pattern = pattern.to_string();
        let deadline = std::time::Instant::now() + timeout;
        let mut accumulated = String::new();

        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            let chunk = self.read_available(Duration::from_millis(200)).await?;
            if !chunk.is_empty() {
                accumulated.push_str(&chunk);
                if accumulated.contains(&pattern) {
                    return Ok(accumulated);
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn is_connected(&self) -> bool {
        self.inner.try_lock().map(|g| g.connected).unwrap_or(false)
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Ssh
    }

    fn metadata(&self) -> TransportMetadata {
        let mut metadata = TransportMetadata::new(TransportType::Ssh);
        if let Ok(inner) = self.inner.lock() {
            metadata.details.insert(
                "endpoint".to_string(),
                format!("{}:{}", inner.config.host, inner.config.port),
            );
            metadata
                .details
                .insert("username".to_string(), inner.config.username.clone());
            metadata.details.extend(inner.metadata.clone());
        }
        metadata
    }
}

/// Static helpers for host key management (called from the session actor).
impl SshTransport {
    pub fn list_known_hosts_static() -> Vec<KnownHostEntry> {
        crate::ssh::list_known_hosts()
    }

    pub fn enroll_host_key_static(host: &str, key_type: &str, key_hex: &str) -> Result<(), Error> {
        crate::ssh::enroll_host_key(host, key_type, key_hex)
    }

    pub fn remove_known_host_static(host: &str) -> Result<(), Error> {
        crate::ssh::remove_known_host(host)
    }

    pub fn check_host_key_static(host: &str) -> HostKeyStatus {
        crate::ssh::check_host_key(host)
    }
}

fn is_timeout(error: &ssh2::Error) -> bool {
    if matches!(
        error.code(),
        ssh2::ErrorCode::Session(-9) | ssh2::ErrorCode::Session(-37)
    ) {
        return true;
    }
    let text = error.to_string().to_ascii_lowercase();
    text.contains("timeout") || text.contains("would block") || text.contains("eagain")
}

fn is_io_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    ) || is_timeout_text(&error.to_string())
}

fn is_timeout_text(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("timeout") || text.contains("would block") || text.contains("eagain")
}

struct KeyboardInteractivePassword {
    password: String,
}

impl ssh2::KeyboardInteractivePrompt for KeyboardInteractivePassword {
    fn prompt(
        &mut self,
        _username: &str,
        _instructions: &str,
        prompts: &[ssh2::Prompt<'_>],
    ) -> Vec<String> {
        prompts.iter().map(|_| self.password.clone()).collect()
    }
}

/// Check the host key for a host against known_hosts without connecting.
/// Returns information about whether the key is known, changed, or unknown.
pub fn check_host_key(host: &str) -> HostKeyStatus {
    let known_hosts_path = ConnectionConfig::known_hosts_path();
    let known = std::fs::read_to_string(&known_hosts_path).unwrap_or_default();

    let host_entry = format!("{} ", host);
    for line in known.lines() {
        if line.starts_with(&host_entry) {
            // Parse the existing entry: host key_type key_hex
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                return HostKeyStatus::Known {
                    key_type: parts[1].to_string(),
                    key_hex: parts[2].to_string(),
                };
            }
            return HostKeyStatus::Known {
                key_type: "unknown".to_string(),
                key_hex: String::new(),
            };
        }
    }
    HostKeyStatus::Unknown
}

/// Host key facts captured for diagnostics and shown to the user.
struct VerifiedHostKey {
    key_type: String,
    fingerprint_sha256: String,
}

/// Verify the SSH host key against known_hosts according to policy.
fn verify_host_key(
    sess: &ssh2::Session,
    config: &ConnectionConfig,
) -> Result<VerifiedHostKey, Error> {
    let (key, key_type) = sess.host_key().unwrap_or((&[], ssh2::HostKeyType::Unknown));
    let key_hex = hex::encode(key);
    let key_type_str = match key_type {
        ssh2::HostKeyType::Rsa => "ssh-rsa",
        ssh2::HostKeyType::Dss => "ssh-dss",
        ssh2::HostKeyType::Ecdsa256 => "ecdsa-sha2-nistp256",
        ssh2::HostKeyType::Ecdsa384 => "ecdsa-sha2-nistp384",
        ssh2::HostKeyType::Ecdsa521 => "ecdsa-sha2-nistp521",
        ssh2::HostKeyType::Ed25519 => "ssh-ed25519",
        ssh2::HostKeyType::Unknown => "unknown",
    };
    let fingerprint = sess
        .host_key_hash(ssh2::HashType::Sha256)
        .map(hex::encode)
        .unwrap_or_else(|| key_hex.clone());
    let facts = VerifiedHostKey {
        key_type: key_type_str.to_string(),
        fingerprint_sha256: fingerprint,
    };

    let known_hosts_path = ConnectionConfig::known_hosts_path();
    let known = std::fs::read_to_string(&known_hosts_path).unwrap_or_default();

    let host_entry = format!("{} ", config.host);
    let already_known = known.lines().any(|l| l.starts_with(&host_entry));

    // If already known, verify the key matches. A changed key is rejected
    // under every policy, including Accept/Warn.
    if already_known {
        for line in known.lines() {
            if line.starts_with(&host_entry) {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let stored_hex = parts[2];
                    if stored_hex != key_hex {
                        return Err(Error::HostKeyChanged(format!(
                            "host key for {} has changed (stored: {}, current: {})",
                            config.host, stored_hex, key_hex
                        )));
                    }
                }
                break;
            }
        }
    }

    match config.host_key_policy {
        crate::config::HostKeyPolicy::Accept => {
            if !already_known {
                tracing::warn!(
                    host = %config.host,
                    key_type = %key_type_str,
                    "ssh: accepting new host key"
                );
                append_known_host(
                    &known_hosts_path,
                    config.host.as_str(),
                    key_type_str,
                    &key_hex,
                )?;
            }
            Ok(facts)
        }
        crate::config::HostKeyPolicy::Warn => {
            if !already_known {
                tracing::warn!(
                    host = %config.host,
                    key_type = %key_type_str,
                    key = %key_hex,
                    "ssh: unknown host key, accepting (warn)"
                );
                append_known_host(
                    &known_hosts_path,
                    config.host.as_str(),
                    key_type_str,
                    &key_hex,
                )?;
            }
            Ok(facts)
        }
        crate::config::HostKeyPolicy::Reject => {
            if !already_known {
                return Err(Error::HostKeyEnrollmentRequired(format!(
                    "unknown host key for {} (type {}): {}. Enroll the host key before connecting.",
                    config.host, key_type_str, key_hex
                )));
            }
            Ok(facts)
        }
    }
}

/// A single known host key entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownHostEntry {
    pub host: String,
    pub key_type: String,
    pub key_hex: String,
}

/// Status of a host key in known_hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostKeyStatus {
    /// The host key is known and matches.
    Known { key_type: String, key_hex: String },
    /// The host is not in known_hosts.
    Unknown,
}

/// List all known host key entries from the known_hosts file.
pub fn list_known_hosts() -> Vec<KnownHostEntry> {
    let known_hosts_path = ConnectionConfig::known_hosts_path();
    let known = std::fs::read_to_string(&known_hosts_path).unwrap_or_default();
    let mut entries = Vec::new();
    for line in known.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            entries.push(KnownHostEntry {
                host: parts[0].to_string(),
                key_type: parts[1].to_string(),
                key_hex: parts[2].to_string(),
            });
        }
    }
    entries
}

/// Enroll (trust) a host key by appending it to known_hosts.
pub fn enroll_host_key(host: &str, key_type: &str, key_hex: &str) -> Result<(), Error> {
    append_known_host(
        &ConnectionConfig::known_hosts_path(),
        host,
        key_type,
        key_hex,
    )
}

/// Remove a host key entry from known_hosts.
pub fn remove_known_host(host: &str) -> Result<(), Error> {
    let known_hosts_path = ConnectionConfig::known_hosts_path();
    let known = std::fs::read_to_string(&known_hosts_path).unwrap_or_default();
    let kept: Vec<String> = known
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !trimmed.starts_with(&format!("{} ", host))
        })
        .map(|l| l.to_string())
        .collect();
    if let Some(parent) = known_hosts_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&known_hosts_path, kept.join("\n") + "\n")
        .map_err(|e| Error::Transport(format!("known_hosts write: {}", e)))
}

fn append_known_host(
    path: &std::path::Path,
    host: &str,
    key_type: &str,
    key_hex: &str,
) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let entry = format!("{} {} {}\n", host, key_type, key_hex);
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .write(true)
        .open(path)
        .map_err(|e| Error::Transport(format!("known_hosts write: {}", e)))?
        .write_all(entry.as_bytes())
        .map_err(|e| Error::Transport(format!("known_hosts write: {}", e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_transport_constructs() {
        let config = ConnectionConfig::ssh("10.0.0.1", "admin");
        let _t = SshTransport::new(config);
        assert!(true);
    }
}
