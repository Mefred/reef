//! Serial transport implementation (stub).

use crate::transport::{DeviceTransport, TransportType};
use crate::Error;
use std::time::Duration;

pub struct SerialTransport {
    connected: bool,
}

impl SerialTransport {
    pub fn new() -> Self {
        SerialTransport { connected: false }
    }
}

impl DeviceTransport for SerialTransport {
    async fn connect(&mut self) -> Result<(), Error> {
        self.connected = true;
        Ok(())
    }
    async fn disconnect(&mut self) -> Result<(), Error> {
        self.connected = false;
        Ok(())
    }
    async fn send(&mut self, _data: &str) -> Result<(), Error> {
        Ok(())
    }
    async fn send_raw(&mut self, _bytes: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    async fn read_until(&mut self, _pattern: &str, _timeout: Duration) -> Result<String, Error> {
        Ok(String::new())
    }
    async fn read_available(&mut self, _timeout: Duration) -> Result<String, Error> {
        Ok(String::new())
    }
    fn is_connected(&self) -> bool {
        self.connected
    }
    fn transport_type(&self) -> TransportType {
        TransportType::Serial
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_starts_disconnected() {
        let t = SerialTransport::new();
        assert!(!t.is_connected());
    }
}
