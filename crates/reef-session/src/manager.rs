//! Session manager: owns active device sessions keyed by device id.
//!
//! Uses type erasure (`DynSession`) so sessions over different transports
//! (SSH, Telnet, Serial) can be stored in a single map.
//!
//! Runs on a single thread (the GUI thread), so `Rc<RefCell<>>` is used
//! instead of `Arc<Mutex<>>`.

use crate::config::ConnectionConfig;
use crate::error::Error;
use crate::session::Session;
use crate::transport::{DeviceTransport, TransportMetadata};
use reef_engine::model::DeviceIdentity;
use reef_logging::record::RecordingStatus;
use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

/// Trait object for a device session, transport-agnostic.
pub trait DynSession {
    fn connect<'a>(
        &'a mut self,
        config: ConnectionConfig,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>>;
    fn disconnect<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>>;
    fn enable<'a>(
        &'a mut self,
        secret: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>>;
    fn send_command<'a>(
        &'a mut self,
        cmd: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, Error>> + 'a>>;
    fn send_config<'a>(
        &'a mut self,
        cmd: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, Error>> + 'a>>;
    fn send_config_batch<'a>(
        &'a mut self,
        cmds: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<String, Error>> + 'a>>;
    fn identify<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<DeviceIdentity, Error>> + 'a>>;
    fn send_interrupt<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>>;
    fn start_recording(&mut self) -> Result<RecordingStatus, Error>;
    fn stop_recording(&mut self) -> Option<RecordingStatus>;
    fn recording_status(&self) -> Option<RecordingStatus>;
    fn recording_path(&self) -> Option<String>;
    fn transport_metadata(&self) -> TransportMetadata;
    fn command_count(&self) -> u64;
    fn is_connected(&self) -> bool;
    fn transport_type(&self) -> crate::transport::TransportType;
}

/// Wraps a concrete `Session<T>` as a `DynSession`.
struct ConcreteSession<T: DeviceTransport> {
    session: Session<T>,
    config: Option<ConnectionConfig>,
}

impl<T: DeviceTransport> DynSession for ConcreteSession<T> {
    fn connect<'a>(
        &'a mut self,
        config: ConnectionConfig,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>> {
        Box::pin(async move {
            self.config = Some(config.clone());
            self.session.connect(&config).await
        })
    }

    fn disconnect<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>> {
        Box::pin(async move { self.session.disconnect().await })
    }

    fn enable<'a>(
        &'a mut self,
        secret: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>> {
        Box::pin(async move { self.session.enable(secret).await })
    }

    fn send_command<'a>(
        &'a mut self,
        cmd: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, Error>> + 'a>> {
        Box::pin(async move { self.session.send_command(cmd).await })
    }

    fn send_config<'a>(
        &'a mut self,
        cmd: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, Error>> + 'a>> {
        Box::pin(async move { self.session.send_config(cmd).await })
    }

    fn send_config_batch<'a>(
        &'a mut self,
        cmds: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<String, Error>> + 'a>> {
        Box::pin(async move { self.session.send_config_batch(cmds).await })
    }

    fn identify<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<DeviceIdentity, Error>> + 'a>> {
        Box::pin(async move { self.session.identify().await })
    }

    fn send_interrupt<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = Result<(), Error>> + 'a>> {
        Box::pin(async move { self.session.send_interrupt().await })
    }

    fn start_recording(&mut self) -> Result<RecordingStatus, Error> {
        self.session.start_recording()
    }

    fn stop_recording(&mut self) -> Option<RecordingStatus> {
        self.session.stop_recording()
    }

    fn recording_status(&self) -> Option<RecordingStatus> {
        self.session.recording_status()
    }

    fn recording_path(&self) -> Option<String> {
        self.session.recording_path()
    }

    fn transport_metadata(&self) -> TransportMetadata {
        self.session.transport_metadata()
    }

    fn command_count(&self) -> u64 {
        self.session.command_count()
    }

    fn is_connected(&self) -> bool {
        self.session.is_connected()
    }

    fn transport_type(&self) -> crate::transport::TransportType {
        self.session.transport_type()
    }
}

/// Manages active device sessions keyed by device id.
pub struct SessionManager {
    sessions: HashMap<String, Rc<RefCell<Box<dyn DynSession>>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        SessionManager {
            sessions: HashMap::new(),
        }
    }

    /// Register a new session for a device with a concrete transport.
    pub fn register<T: DeviceTransport + 'static>(&mut self, device_id: &str, transport: T) {
        let session: Box<dyn DynSession> = Box::new(ConcreteSession {
            session: Session::new(transport, ConnectionConfig::default()).with_device_id(device_id),
            config: None,
        });
        self.sessions
            .insert(device_id.to_string(), Rc::new(RefCell::new(session)));
    }

    /// Check whether a device has an active session.
    pub fn has_session(&self, device_id: &str) -> bool {
        self.sessions.contains_key(device_id)
    }

    /// Get the transport type for a device's session.
    pub fn transport_type(&self, device_id: &str) -> Option<crate::transport::TransportType> {
        self.sessions
            .get(device_id)
            .and_then(|s| s.try_borrow().ok())
            .map(|g| g.transport_type())
    }

    /// Returns the number of active sessions.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Connect to a device's session using the given configuration.
    pub async fn connect(&self, device_id: &str, config: ConnectionConfig) -> Result<(), Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.connect(config).await
    }

    /// Disconnect a device's session.
    pub async fn disconnect(&self, device_id: &str) -> Result<(), Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.disconnect().await
    }

    /// Escalate to privileged EXEC on a device.
    pub async fn enable(&self, device_id: &str, secret: &str) -> Result<(), Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.enable(secret).await
    }

    /// Send a privileged EXEC command and return output.
    pub async fn send_command(&self, device_id: &str, cmd: &str) -> Result<String, Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.send_command(cmd).await
    }

    /// Send a configuration command.
    pub async fn send_config(&self, device_id: &str, cmd: &str) -> Result<String, Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.send_config(cmd).await
    }

    /// Send a batch of configuration commands.
    pub async fn send_config_batch(
        &self,
        device_id: &str,
        cmds: &[String],
    ) -> Result<String, Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.send_config_batch(cmds).await
    }

    /// Enter global configuration mode.
    pub async fn config_terminal(&self, device_id: &str) -> Result<(), Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.send_config("configure terminal\r\n").await?;
        Ok(())
    }

    /// Exit configuration mode back to privileged EXEC.
    pub async fn exit_config(&self, device_id: &str) -> Result<(), Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.send_config("end\r\n").await?;
        Ok(())
    }

    /// Identify the device.
    pub async fn identify(&self, device_id: &str) -> Result<DeviceIdentity, Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.identify().await
    }

    /// Interrupt the running command on the device (Ctrl+C).
    pub async fn send_interrupt(&self, device_id: &str) -> Result<(), Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.send_interrupt().await
    }

    /// Start recording terminal I/O for a device session.
    pub fn start_recording(&self, device_id: &str) -> Result<RecordingStatus, Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let mut guard = session.borrow_mut();
        guard.start_recording()
    }

    /// Stop recording terminal I/O for a device session.
    pub fn stop_recording(&self, device_id: &str) -> Option<RecordingStatus> {
        self.sessions
            .get(device_id)
            .and_then(|s| s.try_borrow_mut().ok())
            .and_then(|mut g| g.stop_recording())
    }

    /// Current recording status for a device session, if one exists.
    pub fn recording_status(&self, device_id: &str) -> Option<RecordingStatus> {
        self.sessions
            .get(device_id)
            .and_then(|s| s.try_borrow().ok())
            .and_then(|g| g.recording_status())
    }

    /// Path of the active or most recent recording, if any.
    pub fn recording_path(&self, device_id: &str) -> Option<String> {
        self.sessions
            .get(device_id)
            .and_then(|s| s.try_borrow().ok())
            .and_then(|g| g.recording_path())
    }

    /// Negotiated transport details for a device session.
    pub fn transport_metadata(&self, device_id: &str) -> Option<TransportMetadata> {
        self.sessions
            .get(device_id)
            .and_then(|s| s.try_borrow().ok())
            .map(|g| g.transport_metadata())
    }

    /// Number of commands sent during the current connection.
    pub fn command_count(&self, device_id: &str) -> Option<u64> {
        self.sessions
            .get(device_id)
            .and_then(|s| s.try_borrow().ok())
            .map(|g| g.command_count())
    }

    /// Disconnect every registered session during application shutdown.
    ///
    /// Returns failures individually so one broken transport does not prevent
    /// the remaining sessions from receiving a disconnect attempt.
    pub async fn disconnect_all(&self) -> Vec<(String, Error)> {
        let device_ids = self.sessions.keys().cloned().collect::<Vec<_>>();
        let mut failures = Vec::new();

        for device_id in device_ids {
            if let Err(error) = self.disconnect(&device_id).await {
                failures.push((device_id, error));
            }
        }

        failures
    }

    /// Check if a device session is connected.
    pub async fn is_connected(&self, device_id: &str) -> Result<bool, Error> {
        let session = self
            .sessions
            .get(device_id)
            .ok_or_else(|| Error::NotFound(format!("no session for device {}", device_id)))?;
        let guard = session.borrow();
        Ok(guard.is_connected())
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::DeviceTransport;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    struct TestTransport {
        connected: bool,
        fail_disconnect: bool,
        disconnect_calls: Rc<Cell<usize>>,
    }

    impl DeviceTransport for TestTransport {
        async fn connect(&mut self) -> Result<(), Error> {
            self.connected = true;
            Ok(())
        }

        async fn disconnect(&mut self) -> Result<(), Error> {
            self.disconnect_calls.set(self.disconnect_calls.get() + 1);
            if self.fail_disconnect {
                return Err(Error::Transport("simulated disconnect failure".to_string()));
            }
            self.connected = false;
            Ok(())
        }

        async fn send(&mut self, _data: &str) -> Result<(), Error> {
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
            Ok(String::new())
        }

        async fn read_available(&mut self, _timeout: Duration) -> Result<String, Error> {
            Ok(String::new())
        }

        fn is_connected(&self) -> bool {
            self.connected
        }

        fn transport_type(&self) -> crate::transport::TransportType {
            crate::transport::TransportType::Telnet
        }
    }

    #[test]
    fn manager_starts_empty() {
        let m = SessionManager::new();
        assert_eq!(m.len(), 0);
    }

    #[tokio::test]
    async fn disconnect_all_attempts_every_transport_after_a_failure() {
        let first_disconnects = Rc::new(Cell::new(0));
        let second_disconnects = Rc::new(Cell::new(0));
        let mut manager = SessionManager::new();

        manager.register(
            "first",
            TestTransport {
                connected: false,
                fail_disconnect: true,
                disconnect_calls: Rc::clone(&first_disconnects),
            },
        );
        manager.register(
            "second",
            TestTransport {
                connected: false,
                fail_disconnect: false,
                disconnect_calls: Rc::clone(&second_disconnects),
            },
        );

        manager
            .connect("first", ConnectionConfig::default())
            .await
            .expect("first transport should connect");
        manager
            .connect("second", ConnectionConfig::default())
            .await
            .expect("second transport should connect");

        let failures = manager.disconnect_all().await;

        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "first");
        assert_eq!(first_disconnects.get(), 1);
        assert_eq!(second_disconnects.get(), 1);
        assert!(manager.is_connected("first").await.unwrap());
        assert!(!manager.is_connected("second").await.unwrap());
    }
}
