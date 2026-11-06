//! SSH transport implementation (stub).

use crate::transport::{DeviceTransport, TransportType};
use crate::Error;
use std::time::Duration;

pub struct SshTransport {
    connected: bool,
}

impl SshTransport {
    pub fn new() -> Self {
        SshTransport { connected: false }
    }
}

impl DeviceTransport for SshTransport {
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
        TransportType::Ssh
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_starts_disconnected() {
        let t = SshTransport::new();
        assert!(!t.is_connected());
    }
}
