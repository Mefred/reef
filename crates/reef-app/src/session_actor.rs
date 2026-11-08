//! A thread-affine session actor for the Tauri application.
//!
//! SessionManager intentionally uses Rc<RefCell<_>> and must remain on one
//! thread. This actor owns it on a dedicated current-thread Tokio runtime and
//! exposes only a Send + Sync message handle to Tauri managed state.

use std::thread;

use reef_engine::model::{DeviceIdentity, InterfaceRef, StaticRoute, VlanConfig};
use reef_engine::parser::{parse_interfaces, parse_routes, parse_vlans};
use serde::Serialize;
use reef_session::{
    ConnectionConfig, SerialTransport, SessionManager, SshTransport, TelnetTransport, TransportType,
};
use tokio::runtime::Builder;
use tokio::sync::{mpsc, oneshot};
use tokio::task::LocalSet;


/// Parsed operational data for the device workspace. Individual command
/// failures are reported independently so unsupported commands do not hide
/// data returned by the other commands.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceWorkspace {
    pub interfaces: Vec<InterfaceRef>,
    pub interface_error: Option<String>,
    pub vlans: Vec<VlanConfig>,
    pub vlan_error: Option<String>,
    pub static_routes: Vec<StaticRoute>,
    pub route_error: Option<String>,
}

enum Command {
    Connect {
        device_id: String,
        config: ConnectionConfig,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Disconnect {
        device_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    SendCommand {
        device_id: String,
        command: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    IsConnected {
        device_id: String,
        reply: oneshot::Sender<Result<bool, String>>,
    },
    Identify {
        device_id: String,
        reply: oneshot::Sender<Result<DeviceIdentity, String>>,
    },
    Enable {
        device_id: String,
        secret: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Inspect {
        device_id: String,
        reply: oneshot::Sender<Result<DeviceWorkspace, String>>,
    },
}

/// Thread-safe handle to the thread-affine session manager.
#[derive(Clone)]
pub struct SessionActor {
    sender: mpsc::UnboundedSender<Command>,
}

impl SessionActor {
    /// Start the dedicated session thread and its local Tokio runtime.
    pub fn start() -> Result<Self, String> {
        let (sender, receiver) = mpsc::unbounded_channel();

        thread::Builder::new()
            .name("reef-session-actor".to_string())
            .spawn(move || {
                let runtime = Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to create Reef session runtime");
                let local = LocalSet::new();

                runtime.block_on(local.run_until(run_actor(receiver)));
            })
            .map_err(|error| format!("failed to start session actor: {error}"))?;

        Ok(Self { sender })
    }

    pub async fn connect(&self, device_id: String, config: ConnectionConfig) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::Connect {
                device_id,
                config,
                reply,
            })
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn disconnect(&self, device_id: String) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::Disconnect { device_id, reply })
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn send_command(&self, device_id: String, command: String) -> Result<String, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::SendCommand {
                device_id,
                command,
                reply,
            })
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn is_connected(&self, device_id: String) -> Result<bool, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::IsConnected { device_id, reply })
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn identify(&self, device_id: String) -> Result<DeviceIdentity, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::Identify { device_id, reply })
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }


    pub async fn inspect(&self, device_id: String) -> Result<DeviceWorkspace, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::Inspect { device_id, reply })
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn enable(&self, device_id: String, secret: String) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::Enable {
                device_id,
                secret,
                reply,
            })
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }
}

async fn run_actor(mut receiver: mpsc::UnboundedReceiver<Command>) {
    let mut manager = SessionManager::new();

    while let Some(command) = receiver.recv().await {
        match command {
            Command::Connect {
                device_id,
                config,
                reply,
            } => {
                // Replacing a device session is explicit: close the previous
                // transport before registering the new one.
                if manager.has_session(&device_id) {
                    let _ = manager.disconnect(&device_id).await;
                }

                match config.transport.clone() {
                    TransportType::Ssh => {
                        manager.register(&device_id, SshTransport::new(config.clone()));
                    }
                    TransportType::Telnet => {
                        manager.register(&device_id, TelnetTransport::new(config.clone()));
                    }
                    TransportType::Serial => {
                        manager.register(&device_id, SerialTransport::new(config.clone()));
                    }
                }

                let result = manager
                    .connect(&device_id, config)
                    .await
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            Command::Disconnect { device_id, reply } => {
                let result = manager
                    .disconnect(&device_id)
                    .await
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            Command::SendCommand {
                device_id,
                command,
                reply,
            } => {
                let result = manager
                    .send_command(&device_id, &command)
                    .await
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            Command::IsConnected { device_id, reply } => {
                let result = manager
                    .is_connected(&device_id)
                    .await
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            Command::Identify { device_id, reply } => {
                let result = manager
                    .identify(&device_id)
                    .await
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            Command::Inspect { device_id, reply } => {
                let interfaces = manager.send_command(&device_id, "show ip interface brief").await;
                let vlans = manager.send_command(&device_id, "show vlan brief").await;
                let routes = manager.send_command(&device_id, "show ip route").await;

                let (interfaces, interface_error) = match interfaces {
                    Ok(output) => (parse_interfaces(&output).model, None),
                    Err(error) => (Vec::new(), Some(error.to_string())),
                };
                let (vlans, vlan_error) = match vlans {
                    Ok(output) => (parse_vlans(&output).model, None),
                    Err(error) => (Vec::new(), Some(error.to_string())),
                };
                let (static_routes, route_error) = match routes {
                    Ok(output) => (parse_routes(&output).model, None),
                    Err(error) => (Vec::new(), Some(error.to_string())),
                };

                let _ = reply.send(Ok(DeviceWorkspace {
                    interfaces,
                    interface_error,
                    vlans,
                    vlan_error,
                    static_routes,
                    route_error,
                }));
            }
            Command::Enable {
                device_id,
                secret,
                reply,
            } => {
                let result = manager
                    .enable(&device_id, &secret)
                    .await
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionActor;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn managed_actor_handle_is_send_and_sync() {
        assert_send_sync::<SessionActor>();
    }
}
