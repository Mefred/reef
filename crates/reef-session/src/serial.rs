//! Serial console transport using `tokio-serial`. Used for out-of-band
//! management when the network is down.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serial::{SerialPortBuilderExt, SerialStream};

use crate::config::ConnectionConfig;
use crate::error::Error;
use crate::transport::{DeviceTransport, TransportType};

pub struct SerialTransport {
    stream: Option<SerialStream>,
    connected: bool,
    read_buf: Vec<u8>,
    config: ConnectionConfig,
}

impl SerialTransport {
    pub fn new(config: ConnectionConfig) -> Self {
        SerialTransport {
            stream: None,
            connected: false,
            read_buf: Vec::new(),
            config,
        }
    }

    fn open_port(&self) -> Result<SerialStream, Error> {
        let port = self
            .config
            .serial_port
            .as_ref()
            .ok_or_else(|| Error::Transport("serial port not configured".to_string()))?;
        // Defaults are 8N1; baud rate is the only required setting.
        let stream = tokio_serial::new(port.to_string_lossy(), self.config.serial_baud)
            .open_native_async()
            .map_err(|e| Error::Transport(format!("serial open {}: {}", port.display(), e)))?;
        Ok(stream)
    }
}

impl DeviceTransport for SerialTransport {
    async fn connect(&mut self) -> Result<(), Error> {
        let stream = self.open_port()?;
        self.stream = Some(stream);
        self.connected = true;
        self.disable_paging().await?;
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), Error> {
        self.stream = None;
        self.connected = false;
        Ok(())
    }

    async fn send(&mut self, data: &str) -> Result<(), Error> {
        let stream = self.stream.as_mut().ok_or_else(|| Error::NotConnected)?;
        stream
            .write_all(data.as_bytes())
            .await
            .map_err(|e| Error::Transport(format!("serial write: {}", e)))?;
        stream.flush().await.map_err(|e| Error::Transport(format!("serial flush: {}", e)))?;
        Ok(())
    }

    async fn send_raw(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let stream = self.stream.as_mut().ok_or_else(|| Error::NotConnected)?;
        stream
            .write_all(bytes)
            .await
            .map_err(|e| Error::Transport(format!("serial write: {}", e)))?;
        stream.flush().await.map_err(|e| Error::Transport(format!("serial flush: {}", e)))?;
        Ok(())
    }

    async fn read_available(&mut self, _timeout: Duration) -> Result<String, Error> {
        let stream = self.stream.as_mut().ok_or_else(|| Error::NotConnected)?;
        let mut tmp = [0u8; 4096];
        match tokio::time::timeout(Duration::from_millis(100), stream.read(&mut tmp)).await {
            Ok(Ok(0)) => Ok(String::new()),
            Ok(Ok(n)) => {
                let s = String::from_utf8_lossy(&tmp[..n]).to_string();
                self.read_buf.extend_from_slice(s.as_bytes());
                Ok(String::from_utf8_lossy(&self.read_buf).to_string())
            }
            Ok(Err(e)) => Err(Error::Transport(format!("serial read: {}", e))),
            Err(_) => Ok(String::new()),
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
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn is_connected(&self) -> bool {
        self.connected
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Serial
    }
}

impl SerialTransport {
    async fn disable_paging(&mut self) -> Result<(), Error> {
        self.send("terminal length 0\r\n").await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = self.read_available(Duration::from_millis(300)).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_transport_constructs() {
        let config = ConnectionConfig::serial("/dev/ttyUSB0");
        let _t = SerialTransport::new(config);
        assert!(true);
    }
}
