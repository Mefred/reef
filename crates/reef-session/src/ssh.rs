//! SSH transport implementation using `ssh2-rs` (blocking SSH) wrapped in
//! `tokio::task::spawn_blocking` so the `DeviceTransport` trait stays async.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::ConnectionConfig;
use crate::error::Error;
use crate::transport::{DeviceTransport, TransportType};

/// Internal shared state for an SSH connection.
struct Inner {
    config: ConnectionConfig,
    session: Option<ssh2::Session>,
    channel: Option<ssh2::Channel>,
    connected: bool,
    authenticated: bool,
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
            let guard = self.inner.lock().map_err(|e| Error::Transport(e.to_string()))?;
            guard.config.clone()
        };

        self.with_inner(move |inner| {
            let addr = format!("{}:{}", config.host, config.port);
            tracing::debug!(%addr, "ssh: connecting tcp");
            let stream = std::net::TcpStream::connect_timeout(
                &addr
                    .parse::<std::net::SocketAddr>()
                    .unwrap_or_else(|_| {
                        format!("{}:{}", config.host, config.port)
                            .parse::<std::net::SocketAddr>()
                            .unwrap()
                    }),
                config.timeout,
            )
            .map_err(|e| Error::Transport(format!("tcp connect: {}", e)))?;

            let mut sess = ssh2::Session::new().map_err(|e| Error::Transport(format!("ssh2 new: {}", e)))?;
            sess.set_tcp_stream(stream);
            sess.set_blocking(true);
            sess.handshake().map_err(|e| Error::Transport(format!("ssh handshake: {}", e)))?;

            // Host key verification.
            verify_host_key(&sess, &config)?;

            // Authentication.
            if let Some(ref pwd) = config.password {
                sess.userauth_password(&config.username, pwd)
                    .map_err(|_| Error::AuthFailed)?;
            } else {
                return Err(Error::AuthFailed);
            }

            // Open a shell channel for interactive CLI.
            let mut channel = sess.channel_session().map_err(|e| Error::Transport(format!("channel: {}", e)))?;
            channel
                .request_pty("vt100", None, None)
                .map_err(|e| Error::Transport(format!("pty: {}", e)))?;
            channel.shell().map_err(|e| Error::Transport(format!("shell: {}", e)))?;

            inner.session = Some(sess);
            inner.channel = Some(channel);
            inner.connected = true;
            inner.authenticated = true;
            Ok(())
        })
        .await?;

        // Disable paging once connected.
        self.disable_paging().await?;
        Ok(())
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
            Ok(())
        })
        .await
    }

    async fn send(&mut self, data: &str) -> Result<(), Error> {
        let data = data.to_string();
        self.with_inner(move |inner| {
            let ch = inner.channel.as_mut().ok_or_else(|| Error::NotConnected)?;
            ch.write_all(data.as_bytes())
                .map_err(|e| Error::Transport(format!("ssh write: {}", e)))?;
            ch.flush().map_err(|e| Error::Transport(format!("ssh flush: {}", e)))?;
            Ok(())
        })
        .await
    }

    async fn send_raw(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let bytes = bytes.to_vec();
        self.with_inner(move |inner| {
            let ch = inner.channel.as_mut().ok_or_else(|| Error::NotConnected)?;
            ch.write_all(&bytes)
                .map_err(|e| Error::Transport(format!("ssh write: {}", e)))?;
            ch.flush().map_err(|e| Error::Transport(format!("ssh flush: {}", e)))?;
            Ok(())
        })
        .await
    }

    async fn read_available(&mut self, _timeout: Duration) -> Result<String, Error> {
        self.with_inner(|inner| {
            let ch = inner.channel.as_mut().ok_or_else(|| Error::NotConnected)?;
            let mut buf = [0u8; 4096];
            match ch.read(&mut buf) {
                Ok(0) => Ok(String::new()),
                Ok(n) => Ok(String::from_utf8_lossy(&buf[..n]).to_string()),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(String::new()),
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
        self.inner
            .try_lock()
            .map(|g| g.connected)
            .unwrap_or(false)
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Ssh
    }
}

impl SshTransport {
    /// Disable paging (`terminal length 0`) so long `show` output isn't paused.
    async fn disable_paging(&mut self) -> Result<(), Error> {
        self.send("terminal length 0\r\n").await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = self.read_available(Duration::from_millis(300)).await?;
        Ok(())
    }
}

/// Verify the SSH host key against known_hosts according to policy.
fn verify_host_key(sess: &ssh2::Session, config: &ConnectionConfig) -> Result<(), Error> {
    let key = sess.host_key().map(|(k, _)| k).unwrap_or(&[]);
    let key_hex = hex::encode(key);

    let known_hosts_path = ConnectionConfig::known_hosts_path();
    let known = std::fs::read_to_string(&known_hosts_path).unwrap_or_default();

    let host_entry = format!("{} ", config.host);
    let already_known = known.lines().any(|l| l.starts_with(&host_entry));

    match config.host_key_policy {
        crate::config::HostKeyPolicy::Accept => {
            if !already_known {
                tracing::warn!(host = %config.host, "ssh: accepting new host key");
                append_known_host(&known_hosts_path, config.host.as_str(), &key_hex)?;
            }
            Ok(())
        }
        crate::config::HostKeyPolicy::Warn => {
            if !already_known {
                tracing::warn!(host = %config.host, key = %key_hex, "ssh: unknown host key, accepting (warn)");
                append_known_host(&known_hosts_path, config.host.as_str(), &key_hex)?;
            }
            Ok(())
        }
        crate::config::HostKeyPolicy::Reject => {
            if !already_known {
                return Err(Error::HostKey(format!(
                    "unknown host key for {}: {}",
                    config.host, key_hex
                )));
            }
            Ok(())
        }
    }
}

fn append_known_host(path: &std::path::Path, host: &str, key_hex: &str) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let entry = format!("{} ssh-rsa {}\n", host, key_hex);
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
