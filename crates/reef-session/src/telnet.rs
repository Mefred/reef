//! Telnet transport over raw TCP (NVT), implemented with tokio TcpStream.
//! No encryption — marked insecure in the UI.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::config::ConnectionConfig;
use crate::error::Error;
use crate::transport::{DeviceTransport, TransportType};

/// Telnet IAC (Interpret As Command) byte.
const IAC: u8 = 255;
const WILL: u8 = 251;
const WONT: u8 = 252;
const DO: u8 = 253;
const DONT: u8 = 254;

pub struct TelnetTransport {
    stream: Option<TcpStream>,
    connected: bool,
    read_buf: Vec<u8>,
    config: ConnectionConfig,
}

impl TelnetTransport {
    pub fn new(config: ConnectionConfig) -> Self {
        TelnetTransport {
            stream: None,
            connected: false,
            read_buf: Vec::new(),
            config,
        }
    }

    /// Negotiate telnet options: we will not echo, do not go ahead, etc.
    async fn negotiate(&mut self) -> Result<(), Error> {
        let stream = self.stream.as_mut().ok_or_else(|| Error::NotConnected)?;
        // Disable echo locally, suppress go-ahead, request terminal type.
        let init: &[u8] = &[
            IAC, WONT, 1,   // WONT echo
            IAC, WILL, 3,   // WILL suppress go-ahead
            IAC, WILL, 0,   // WILL binary
            IAC, DO, 0,     // DO binary
            IAC, WILL, 24,  // WILL terminal type
        ];
        stream.write_all(init).await.map_err(|e| Error::Transport(format!("telnet negotiate: {}", e)))?;
        stream.flush().await.map_err(|e| Error::Transport(format!("telnet flush: {}", e)))?;
        Ok(())
    }
}

impl DeviceTransport for TelnetTransport {
    async fn connect(&mut self) -> Result<(), Error> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| Error::Transport(format!("telnet connect: {}", e)))?;
        self.negotiate().await?;
        self.stream = Some(stream);
        self.connected = true;
        self.disable_paging().await?;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), Error> {
        if let Some(mut s) = self.stream.take() {
            let _ = s.shutdown().await;
        }
        self.connected = false;
        Ok(())
    }

    async fn send(&mut self, data: &str) -> Result<(), Error> {
        let stream = self.stream.as_mut().ok_or_else(|| Error::NotConnected)?;
        stream
            .write_all(data.as_bytes())
            .await
            .map_err(|e| Error::Transport(format!("telnet write: {}", e)))?;
        stream.flush().await.map_err(|e| Error::Transport(format!("telnet flush: {}", e)))?;
        Ok(())
    }

    async fn send_raw(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let stream = self.stream.as_mut().ok_or_else(|| Error::NotConnected)?;
        stream
            .write_all(bytes)
            .await
            .map_err(|e| Error::Transport(format!("telnet write: {}", e)))?;
        stream.flush().await.map_err(|e| Error::Transport(format!("telnet flush: {}", e)))?;
        Ok(())
    }

    async fn read_available(&mut self, _timeout: Duration) -> Result<String, Error> {
        let stream = self.stream.as_mut().ok_or_else(|| Error::NotConnected)?;
        let mut tmp = [0u8; 4096];
        match tokio::time::timeout(Duration::from_millis(100), stream.read(&mut tmp)).await {
            Ok(Ok(0)) => Ok(String::new()),
            Ok(Ok(n)) => {
                let s = String::from_utf8_lossy(&tmp[..n]).to_string();
                self.process_telnet_stream(&s);
                Ok(String::from_utf8_lossy(&self.read_buf).to_string())
            }
            Ok(Err(e)) => Err(Error::Transport(format!("telnet read: {}", e))),
            Err(_) => Ok(String::new()), // timeout -> nothing available
        }
    }

    async fn read_until(&mut self, pattern: &str, timeout: Duration) -> Result<String, Error> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut accumulated = String::new();

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(Error::Timeout);
            }
            let chunk = self.read_available(remaining).await?;
            if !chunk.is_empty() {
                accumulated.push_str(&chunk);
                if accumulated.contains(pattern) {
                    return Ok(accumulated);
                }
            }
            if remaining < Duration::from_millis(100) {
                tokio::time::sleep(remaining).await;
            } else {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Telnet
    }
}

impl TelnetTransport {
    /// Disable paging (`terminal length 0`).
    async fn disable_paging(&mut self) -> Result<(), Error> {
        self.send("terminal length 0\r\n").await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = self.read_available(Duration::from_millis(300)).await?;
        Ok(())
    }

    /// Strip Telnet IAC sequences and buffer the resulting text.
    fn process_telnet_stream(&mut self, raw: &str) {
        let bytes = raw.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == IAC && i + 2 < bytes.len() {
                // Skip 3-byte IAC command (IAC, cmd, option).
                i += 3;
                continue;
            }
            if bytes[i] == IAC && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        let text = String::from_utf8_lossy(&out);
        self.read_buf.extend_from_slice(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telnet_transport_constructs() {
        let config = ConnectionConfig::telnet("10.0.0.1", "admin");
        let _t = TelnetTransport::new(config);
        assert!(true);
    }
}
