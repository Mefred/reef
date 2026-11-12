//! A thread-affine session actor for the native Reef UI.
//!
//! SessionManager intentionally uses Rc<RefCell<_>> and must remain on one
//! thread. This actor owns it on a dedicated current-thread Tokio runtime and
//! exposes only a Send + Sync message handle to the wgpui application.

use std::sync::{Arc, Mutex};
use std::thread;

use reef_engine::model::{
    DeviceIdentity, InterfaceConfig, InterfaceMode, InterfaceRef, Ipv4Address, StaticRoute,
    VlanConfig,
};
use reef_engine::parser::parse_running_config;
use reef_engine::parser::{parse_interfaces, parse_routes, parse_vlans};
use reef_logging::record::RecordingStatus;
use reef_session::{
    ConnectionConfig, SerialTransport, SessionManager, SshTransport, TelnetTransport,
    TransportMetadata, TransportType,
};
use serde::{Deserialize, Serialize};
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

/// Safe configuration summary for the frontend. Secret-bearing fields from
/// DeviceConfig (line passwords, credentials and banners) are intentionally
/// excluded from this transport DTO.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationSnapshot {
    pub hostname: Option<String>,
    pub domain_name: Option<String>,
    pub name_servers: Vec<String>,
    pub interfaces: Vec<ConfiguredInterface>,
    pub vlans: Vec<VlanConfig>,
    pub static_routes: Vec<String>,
    pub acls: Vec<ConfiguredAcl>,
    pub dhcp_pools: Vec<ConfiguredDhcpPool>,
    pub nat: Option<ConfiguredNat>,
    pub ssh: Option<ConfiguredSsh>,
    pub line_access: Vec<ConfiguredLineAccess>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredInterface {
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub mode: String,
    pub access_vlan: Option<u16>,
    pub ipv4: Option<String>,
    pub mtu: Option<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredAcl {
    pub name: String,
    pub acl_type: String,
    pub rules: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredDhcpPool {
    pub name: String,
    pub network: String,
    pub default_router: Option<String>,
    pub dns_servers: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredNat {
    pub inside_interfaces: Vec<String>,
    pub outside_interfaces: Vec<String>,
    pub pools: Vec<ConfiguredNatPool>,
    pub rules: Vec<ConfiguredNatRule>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredNatPool {
    pub name: String,
    pub start: String,
    pub end: String,
    pub netmask: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredNatRule {
    pub name: String,
    pub source_acl: Option<String>,
    pub pool: Option<String>,
    pub overload: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredSsh {
    pub enabled: bool,
    pub version: String,
    pub timeout: Option<u16>,
    pub authentication_retries: Option<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredLineAccess {
    pub line: String,
    pub login_required: bool,
    pub transport: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationDraft {
    /// Set-only fields: omitted values preserve the running configuration.
    pub hostname: Option<String>,
    pub domain_name: Option<String>,
    /// Additional name servers; existing servers are preserved.
    #[serde(default)]
    pub add_name_servers: Vec<String>,
    /// Name servers to remove.
    #[serde(default)]
    pub remove_name_servers: Vec<String>,
    /// New VLANs only. Existing VLANs cannot be modified or removed by this draft.
    #[serde(default)]
    pub add_vlans: Vec<AddVlanDraft>,
    /// VLAN IDs to remove.
    #[serde(default)]
    pub remove_vlan_ids: Vec<u16>,
    /// Interface configurations to add or modify.
    #[serde(default)]
    pub add_interfaces: Vec<InterfaceDraft>,
    /// Interface names to remove.
    #[serde(default)]
    pub remove_interface_names: Vec<String>,
    /// Static routes to add.
    #[serde(default)]
    pub add_routes: Vec<RouteDraft>,
    /// Static route prefixes to remove.
    #[serde(default)]
    pub remove_route_prefixes: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddVlanDraft {
    pub id: u16,
    pub name: String,
    /// Desired administrative state. `None` preserves the existing state when
    /// the VLAN already exists, and defaults to enabled for new VLANs.
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceDraft {
    pub name: String,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub mode: Option<String>,
    pub access_vlan: Option<u16>,
    pub ipv4_address: Option<String>,
    pub ipv4_prefix_len: Option<u8>,
    pub shutdown: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteDraft {
    pub prefix: String,
    pub next_hop: String,
    pub admin_distance: Option<u16>,
}

/// Non-secret diagnostics for an active device session.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDiagnostics {
    pub connected: bool,
    pub command_count: u64,
    pub transport: TransportMetadata,
    pub recording: Option<RecordingStatus>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationPreview {
    pub diff: String,
    pub commands: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
    pub affects_connectivity: bool,
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
    GetConfiguration {
        device_id: String,
        reply: oneshot::Sender<Result<ConfigurationSnapshot, String>>,
    },
    PreviewConfiguration {
        device_id: String,
        draft: ConfigurationDraft,
        reply: oneshot::Sender<Result<ConfigurationPreview, String>>,
    },
    ApplyConfiguration {
        device_id: String,
        commands: Vec<String>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    ListKnownHosts {
        reply: oneshot::Sender<Result<Vec<reef_session::KnownHostEntry>, String>>,
    },
    EnrollHostKey {
        host: String,
        key_type: String,
        key_hex: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    RemoveKnownHost {
        host: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    CheckHostKey {
        host: String,
        reply: oneshot::Sender<Result<reef_session::HostKeyStatus, String>>,
    },
    SendInterrupt {
        device_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    StartRecording {
        device_id: String,
        reply: oneshot::Sender<Result<RecordingStatus, String>>,
    },
    StopRecording {
        device_id: String,
        reply: oneshot::Sender<Result<Option<RecordingStatus>, String>>,
    },
    GetRecordingStatus {
        device_id: String,
        reply: oneshot::Sender<Result<Option<RecordingStatus>, String>>,
    },
    GetSessionDiagnostics {
        device_id: String,
        reply: oneshot::Sender<Result<SessionDiagnostics, String>>,
    },
    Shutdown {
        reply: oneshot::Sender<Result<(), String>>,
    },
}

/// Thread-safe handle to the thread-affine session manager.
#[derive(Clone)]
pub struct SessionActor {
    sender: mpsc::Sender<Command>,
    join: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

impl SessionActor {
    /// Start the dedicated session thread and its local Tokio runtime.
    pub fn start() -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel(64);
        let (startup_sender, startup_receiver) = std::sync::mpsc::sync_channel(1);

        let join = thread::Builder::new()
            .name("reef-session-actor".to_string())
            .spawn(move || {
                let runtime = match Builder::new_current_thread().enable_all().build() {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = startup_sender.send(Err(format!(
                            "failed to create Reef session runtime: {error}"
                        )));
                        return;
                    }
                };
                let local = LocalSet::new();

                if startup_sender.send(Ok(())).is_err() {
                    return;
                }

                runtime.block_on(local.run_until(run_actor(receiver)));
            })
            .map_err(|error| format!("failed to start session actor thread: {error}"))?;

        startup_receiver
            .recv()
            .map_err(|_| "session actor stopped during startup".to_string())??;

        Ok(Self {
            sender,
            join: Arc::new(Mutex::new(Some(join))),
        })
    }

    pub async fn connect(&self, device_id: String, config: ConnectionConfig) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::Connect {
                device_id,
                config,
                reply,
            })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn disconnect(&self, device_id: String) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::Disconnect { device_id, reply })
            .await
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
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn is_connected(&self, device_id: String) -> Result<bool, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::IsConnected { device_id, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn identify(&self, device_id: String) -> Result<DeviceIdentity, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::Identify { device_id, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }
    pub async fn inspect(&self, device_id: String) -> Result<DeviceWorkspace, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::Inspect { device_id, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn get_configuration(
        &self,
        device_id: String,
    ) -> Result<ConfigurationSnapshot, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::GetConfiguration { device_id, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn preview_configuration(
        &self,
        device_id: String,
        draft: ConfigurationDraft,
    ) -> Result<ConfigurationPreview, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::PreviewConfiguration {
                device_id,
                draft,
                reply,
            })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    /// Send a batch of configuration commands to the device in one config
    /// session. Only running-configuration changes are sent; startup-config
    /// writes remain permanently disabled.
    pub async fn apply_configuration(
        &self,
        device_id: String,
        commands: Vec<String>,
    ) -> Result<String, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::ApplyConfiguration {
                device_id,
                commands,
                reply,
            })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn send_interrupt(&self, device_id: String) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::SendInterrupt { device_id, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn list_known_hosts(&self) -> Result<Vec<reef_session::KnownHostEntry>, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::ListKnownHosts { reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn enroll_host_key(
        &self,
        host: String,
        key_type: String,
        key_hex: String,
    ) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::EnrollHostKey {
                host,
                key_type,
                key_hex,
                reply,
            })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn remove_known_host(&self, host: String) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::RemoveKnownHost { host, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn check_host_key(
        &self,
        host: String,
    ) -> Result<reef_session::HostKeyStatus, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::CheckHostKey { host, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn start_recording(&self, device_id: String) -> Result<RecordingStatus, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::StartRecording { device_id, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn stop_recording(
        &self,
        device_id: String,
    ) -> Result<Option<RecordingStatus>, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::StopRecording { device_id, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn recording_status(
        &self,
        device_id: String,
    ) -> Result<Option<RecordingStatus>, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::GetRecordingStatus { device_id, reply })
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    pub async fn session_diagnostics(
        &self,
        device_id: String,
    ) -> Result<SessionDiagnostics, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::GetSessionDiagnostics { device_id, reply })
            .await
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
            .await
            .map_err(|_| "session actor is unavailable".to_string())?;
        response
            .await
            .map_err(|_| "session actor stopped before replying".to_string())?
    }

    /// Stop the actor after disconnecting all active device transports.
    /// The reply is sent only after cleanup has completed.
    pub async fn shutdown(&self) -> Result<(), String> {
        let (reply, response) = oneshot::channel();
        let response_result = match self.sender.send(Command::Shutdown { reply }).await {
            Ok(()) => match response.await {
                Ok(result) => result,
                Err(_) => Err("session actor stopped before shutdown completed".to_string()),
            },
            Err(_) => Err("session actor is unavailable".to_string()),
        };

        // The shutdown response follows transport cleanup. Join as well so the
        // actor runtime and thread have fully exited before the app terminates.
        let join = self
            .join
            .lock()
            .map_err(|_| "session actor join handle was poisoned".to_string())?
            .take();
        if let Some(join) = join {
            join.join()
                .map_err(|_| "session actor thread panicked".to_string())?;
        }

        response_result
    }
}

/// Keep map-backed configuration sections stable across refreshes. HashMap
/// iteration order is intentionally unspecified, but the UI and previews should
/// not reorder unchanged device configuration between requests.
fn sort_configuration_snapshot(snapshot: &mut ConfigurationSnapshot) {
    snapshot
        .interfaces
        .sort_by(|left, right| left.name.cmp(&right.name));
    snapshot.vlans.sort_by_key(|vlan| vlan.id);
    snapshot.static_routes.sort();
    snapshot
        .acls
        .sort_by(|left, right| left.name.cmp(&right.name));
    snapshot
        .dhcp_pools
        .sort_by(|left, right| left.name.cmp(&right.name));
    snapshot
        .line_access
        .sort_by(|left, right| left.line.cmp(&right.line));
    if let Some(nat) = snapshot.nat.as_mut() {
        nat.inside_interfaces.sort();
        nat.outside_interfaces.sort();
        nat.pools.sort_by(|left, right| left.name.cmp(&right.name));
        nat.rules.sort_by(|left, right| left.name.cmp(&right.name));
    }
}

fn validate_hostname(value: &str) -> Result<String, String> {
    let hostname = value.trim();
    if hostname.is_empty()
        || hostname.len() > 63
        || !hostname
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "hostname must be 1-63 ASCII letters, digits, hyphens, or underscores".to_string(),
        );
    }
    Ok(hostname.to_string())
}

fn validate_domain_name(value: &str) -> Result<String, String> {
    let domain = value.trim();
    let valid = !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if !valid {
        return Err("domain name must contain valid DNS labels".to_string());
    }
    Ok(domain.to_string())
}

fn normalize_name_server(value: &str) -> Result<String, String> {
    value
        .trim()
        .parse::<std::net::IpAddr>()
        .map(|address| address.to_string())
        .map_err(|_| format!("invalid name-server IP address: {value}"))
}

fn validate_vlan_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty()
        || name.len() > 32
        || !name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
    {
        return Err(
            "VLAN name must be 1-32 ASCII letters, digits, underscores, or hyphens".to_string(),
        );
    }

    Ok(name.to_string())
}

fn validate_vlan_id(id: u16) -> Result<(), String> {
    let valid_id = (2..=1001).contains(&id) || (1006..=4094).contains(&id);
    if !valid_id {
        return Err(format!(
            "VLAN ID {id} is reserved or outside the supported range (2-1001, 1006-4094)"
        ));
    }
    Ok(())
}

fn validate_vlan_draft(id: u16, name: &str) -> Result<String, String> {
    validate_vlan_id(id)?;
    validate_vlan_name(name)
}

async fn run_actor(mut receiver: mpsc::Receiver<Command>) {
    let mut manager = SessionManager::new();

    let shutdown_reply = loop {
        let Some(command) = receiver.recv().await else {
            break None;
        };
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
                let interfaces = manager
                    .send_command(&device_id, "show ip interface brief")
                    .await;
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

            Command::GetConfiguration { device_id, reply } => {
                let result = manager
                    .send_command(&device_id, "show running-config")
                    .await
                    .map_err(|error| error.to_string())
                    .map(|output| {
                        let parsed = parse_running_config(&output);
                        let config = parsed.model;
                        let mut snapshot = ConfigurationSnapshot {
                            hostname: config.hostname,
                            domain_name: config.ip_domain_name,
                            name_servers: config.ip_name_server,
                            interfaces: config
                                .interfaces
                                .into_values()
                                .map(|interface| ConfiguredInterface {
                                    name: interface.name,
                                    description: interface.description,
                                    enabled: interface.enabled,
                                    mode: format!("{:?}", interface.mode),
                                    access_vlan: interface.access_vlan,
                                    ipv4: interface.ipv4.map(|address| {
                                        format!("{}/{}", address.address, address.prefix_len)
                                    }),
                                    mtu: interface.mtu,
                                })
                                .collect(),
                            vlans: config.vlans.into_values().collect(),
                            static_routes: config
                                .ip_routes
                                .iter()
                                .map(|route| format!("{} via {}", route.prefix, route.next_hop))
                                .collect(),
                            acls: config
                                .acls
                                .into_values()
                                .map(|acl| ConfiguredAcl {
                                    name: acl.name,
                                    acl_type: format!("{:?}", acl.acl_type),
                                    rules: acl
                                        .rules
                                        .into_iter()
                                        .map(|rule| {
                                            format!(
                                                "{:?} {:?} from {:?} to {:?}",
                                                rule.action,
                                                rule.protocol,
                                                rule.source,
                                                rule.destination
                                            )
                                        })
                                        .collect(),
                                })
                                .collect(),
                            dhcp_pools: config
                                .dhcp_pools
                                .into_values()
                                .map(|pool| ConfiguredDhcpPool {
                                    name: pool.name,
                                    network: pool.network.to_string(),
                                    default_router: pool
                                        .default_router
                                        .map(|router| router.to_string()),
                                    dns_servers: pool
                                        .dns_servers
                                        .into_iter()
                                        .map(|server| server.to_string())
                                        .collect(),
                                })
                                .collect(),
                            nat: config.nat.map(|nat| ConfiguredNat {
                                inside_interfaces: nat.inside_interfaces,
                                outside_interfaces: nat.outside_interfaces,
                                pools: nat
                                    .pools
                                    .into_iter()
                                    .map(|pool| ConfiguredNatPool {
                                        name: pool.name,
                                        start: pool.start.to_string(),
                                        end: pool.end.to_string(),
                                        netmask: pool.netmask.to_string(),
                                    })
                                    .collect(),
                                rules: nat
                                    .rules
                                    .into_iter()
                                    .map(|rule| ConfiguredNatRule {
                                        name: rule.name,
                                        source_acl: rule.source_acl,
                                        pool: rule.pool,
                                        overload: rule.overload,
                                    })
                                    .collect(),
                            }),
                            ssh: config.ssh.map(|ssh| ConfiguredSsh {
                                enabled: ssh.enabled,
                                version: format!("{:?}", ssh.version),
                                timeout: ssh.timeout,
                                authentication_retries: ssh.authentication_retries,
                            }),
                            line_access: config
                                .line_configs
                                .into_values()
                                .map(|line| ConfiguredLineAccess {
                                    line: line.line,
                                    login_required: line.login,
                                    transport: line
                                        .transport
                                        .into_iter()
                                        .map(|transport| format!("{:?}", transport))
                                        .collect(),
                                })
                                .collect(),
                            warnings: parsed
                                .warnings
                                .into_iter()
                                .map(|warning| {
                                    format!("line {}: {}", warning.line_no, warning.reason)
                                })
                                .collect(),
                        };
                        sort_configuration_snapshot(&mut snapshot);
                        snapshot
                    });
                let _ = reply.send(result);
            }
            Command::PreviewConfiguration {
                device_id,
                draft,
                reply,
            } => {
                let result = async {
                    let output = manager
                        .send_command(&device_id, "show running-config")
                        .await
                        .map_err(|error| error.to_string())?;
                    let parsed = parse_running_config(&output);
                    let current = parsed.model;
                    let mut desired = current.clone();

                    if let Some(hostname) = draft.hostname {
                        desired.hostname = Some(validate_hostname(&hostname)?);
                    }
                    if let Some(domain) = draft.domain_name {
                        desired.ip_domain_name = Some(validate_domain_name(&domain)?);
                    }
                    for server in draft.add_name_servers {
                        let normalized = normalize_name_server(&server)?;
                        if !desired.ip_name_server.contains(&normalized) {
                            desired.ip_name_server.push(normalized);
                        }
                    }

                    let mut requested_vlan_ids = std::collections::HashSet::new();
                    for vlan in draft.add_vlans {
                        if !requested_vlan_ids.insert(vlan.id) {
                            return Err(format!(
                                "VLAN ID {} appears more than once in the draft",
                                vlan.id
                            ));
                        }
                        match desired.vlans.get_mut(&vlan.id) {
                            Some(existing) => {
                                // Existing VLANs can be renamed or suspended.
                                // The ID itself is the identity of the VLAN and
                                // cannot be changed by this draft.
                                existing.name = validate_vlan_name(&vlan.name)?;
                                if let Some(enabled) = vlan.enabled {
                                    existing.enabled = enabled;
                                }
                            }
                            None => {
                                let name = validate_vlan_draft(vlan.id, &vlan.name)?;
                                desired.vlans.insert(
                                    vlan.id,
                                    VlanConfig {
                                        id: vlan.id,
                                        name,
                                        enabled: vlan.enabled.unwrap_or(true),
                                    },
                                );
                            }
                        }
                    }
                    for vlan_id in &draft.remove_vlan_ids {
                        desired.vlans.remove(vlan_id);
                    }

                    // Remove name servers
                    for ns in &draft.remove_name_servers {
                        if let Ok(normalized) = normalize_name_server(ns) {
                            desired.ip_name_server.retain(|s| s != &normalized);
                        }
                    }

                    // Interface add/modify
                    for iface_draft in &draft.add_interfaces {
                        let name = iface_draft.name.trim();
                        if name.is_empty() {
                            return Err("interface name cannot be empty".to_string());
                        }
                        let mut iface =
                            desired.interfaces.get(name).cloned().unwrap_or_else(|| {
                                InterfaceConfig {
                                    name: name.to_string(),
                                    ..Default::default()
                                }
                            });
                        if let Some(desc) = &iface_draft.description {
                            iface.description = Some(desc.clone());
                        }
                        if let Some(enabled) = iface_draft.enabled {
                            iface.enabled = enabled;
                        }
                        if let Some(mode_str) = &iface_draft.mode {
                            iface.mode = match mode_str.as_str() {
                                "access" => InterfaceMode::Access,
                                "trunk" => InterfaceMode::Trunk,
                                "routed" | "layer3" => InterfaceMode::Routed,
                                "l2" => InterfaceMode::L2,
                                _ => return Err(format!("unknown interface mode: {mode_str}")),
                            };
                        }
                        if let Some(vlan) = iface_draft.access_vlan {
                            iface.access_vlan = Some(vlan);
                        }
                        if let Some(addr_str) = &iface_draft.ipv4_address {
                            if let Some(prefix_len) = iface_draft.ipv4_prefix_len {
                                let addr: std::net::Ipv4Addr = addr_str
                                    .parse()
                                    .map_err(|_| format!("invalid IPv4 address: {addr_str}"))?;
                                iface.ipv4 = Some(Ipv4Address {
                                    address: addr,
                                    prefix_len,
                                });
                            } else {
                                return Err(format!(
                                    "IPv4 prefix length required for interface {name}"
                                ));
                            }
                        }
                        if iface_draft.shutdown.unwrap_or(false) {
                            iface.enabled = false;
                        }
                        desired.interfaces.insert(name.to_string(), iface);
                    }
                    for iface_name in &draft.remove_interface_names {
                        desired.interfaces.remove(iface_name);
                    }

                    // Route add
                    for route_draft in &draft.add_routes {
                        let prefix: ipnet::IpNet = route_draft
                            .prefix
                            .parse()
                            .map_err(|_| format!("invalid route prefix: {}", route_draft.prefix))?;
                        let next_hop: std::net::IpAddr = route_draft
                            .next_hop
                            .parse()
                            .map_err(|_| format!("invalid next-hop: {}", route_draft.next_hop))?;
                        desired.ip_routes.push(StaticRoute {
                            prefix,
                            next_hop,
                            admin_distance: route_draft.admin_distance,
                            permanent: false,
                        });
                    }
                    // Route remove
                    for prefix_str in &draft.remove_route_prefixes {
                        if let Ok(prefix) = prefix_str.parse::<ipnet::IpNet>() {
                            desired.ip_routes.retain(|r| r.prefix != prefix);
                        }
                    }

                    let changes = reef_engine::planner::plan(&current, &desired);
                    let problems = reef_engine::validator::validate(&changes, &current);
                    let errors = problems
                        .iter()
                        .filter(|problem| {
                            problem.severity == reef_engine::validator::Severity::Error
                        })
                        .map(|problem| format!("{}: {}", problem.element, problem.message))
                        .collect::<Vec<_>>();
                    let warnings = problems
                        .iter()
                        .filter(|problem| {
                            problem.severity == reef_engine::validator::Severity::Warning
                        })
                        .map(|problem| format!("{}: {}", problem.element, problem.message))
                        .collect::<Vec<_>>();
                    let diff = reef_engine::diff::render(&changes);
                    let batch = reef_engine::generator::generate(&changes);
                    Ok(ConfigurationPreview {
                        diff,
                        commands: if errors.is_empty() {
                            batch.commands
                        } else {
                            Vec::new()
                        },
                        warnings,
                        errors,
                        affects_connectivity: changes.affects_connectivity,
                    })
                }
                .await;
                let _ = reply.send(result);
            }
            Command::ApplyConfiguration {
                device_id,
                commands,
                reply,
            } => {
                let result = manager
                    .send_config_batch(&device_id, &commands)
                    .await
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
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
            Command::SendInterrupt { device_id, reply } => {
                let result = manager
                    .send_interrupt(&device_id)
                    .await
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            Command::ListKnownHosts { reply } => {
                let entries = reef_session::SshTransport::list_known_hosts_static();
                let _ = reply.send(Ok(entries));
            }
            Command::EnrollHostKey {
                host,
                key_type,
                key_hex,
                reply,
            } => {
                let result =
                    reef_session::SshTransport::enroll_host_key_static(&host, &key_type, &key_hex);
                let _ = reply.send(result.map_err(|e| e.to_string()));
            }
            Command::RemoveKnownHost { host, reply } => {
                let result = reef_session::SshTransport::remove_known_host_static(&host);
                let _ = reply.send(result.map_err(|e| e.to_string()));
            }
            Command::CheckHostKey { host, reply } => {
                let status = reef_session::SshTransport::check_host_key_static(&host);
                let _ = reply.send(Ok(status));
            }
            Command::StartRecording { device_id, reply } => {
                let result = manager
                    .start_recording(&device_id)
                    .map_err(|error| error.to_string());
                let _ = reply.send(result);
            }
            Command::StopRecording { device_id, reply } => {
                let _ = reply.send(Ok(manager.stop_recording(&device_id)));
            }
            Command::GetRecordingStatus { device_id, reply } => {
                let _ = reply.send(Ok(manager.recording_status(&device_id)));
            }
            Command::GetSessionDiagnostics { device_id, reply } => {
                let result = async {
                    if !manager.has_session(&device_id) {
                        return Err(format!("no session for device {device_id}"));
                    }
                    let connected = manager
                        .is_connected(&device_id)
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok(SessionDiagnostics {
                        connected,
                        command_count: manager.command_count(&device_id).unwrap_or(0),
                        transport: manager
                            .transport_metadata(&device_id)
                            .unwrap_or_else(|| TransportMetadata::new(TransportType::Ssh)),
                        recording: manager.recording_status(&device_id),
                    })
                }
                .await;
                let _ = reply.send(result);
            }
            Command::Shutdown { reply } => break Some(reply),
        }
    };

    // The actor owns every transport. Close them before its runtime and thread
    // are dropped; report individual failures without skipping other sessions.
    let failures = manager.disconnect_all().await;
    for (device_id, error) in &failures {
        tracing::warn!(%device_id, %error, "failed to disconnect device during session actor shutdown");
    }

    if let Some(reply) = shutdown_reply {
        let result = if failures.is_empty() {
            Ok(())
        } else {
            Err(failures
                .into_iter()
                .map(|(device_id, error)| format!("{device_id}: {error}"))
                .collect::<Vec<_>>()
                .join("; "))
        };
        let _ = reply.send(result);
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

    #[tokio::test]
    async fn shutdown_completes_and_rejects_subsequent_operations() {
        let actor = SessionActor::start().expect("actor should start");

        actor
            .shutdown()
            .await
            .expect("empty actor should shut down");

        let error = actor
            .is_connected("missing-device".to_string())
            .await
            .expect_err("stopped actor should reject new work");
        assert!(error.contains("unavailable"));
    }

    #[tokio::test]
    async fn actor_remains_available_after_a_device_operation_error() {
        let actor = SessionActor::start().expect("actor should start");

        let command_error = actor
            .send_command("missing-device".to_string(), "show version".to_string())
            .await
            .expect_err("unknown device command should fail");
        assert!(command_error.contains("missing-device"));

        let connection_error = actor
            .is_connected("missing-device".to_string())
            .await
            .expect_err("unknown device status should fail");
        assert!(connection_error.contains("missing-device"));
    }

    #[test]
    fn configuration_snapshot_sections_are_sorted_deterministically() {
        let mut snapshot = super::ConfigurationSnapshot {
            hostname: None,
            domain_name: None,
            name_servers: Vec::new(),
            interfaces: vec![
                super::ConfiguredInterface {
                    name: "GigabitEthernet0/0/2".to_string(),
                    description: None,
                    enabled: true,
                    mode: "Routed".to_string(),
                    access_vlan: None,
                    ipv4: None,
                    mtu: None,
                },
                super::ConfiguredInterface {
                    name: "GigabitEthernet0/0/1".to_string(),
                    description: None,
                    enabled: true,
                    mode: "Routed".to_string(),
                    access_vlan: None,
                    ipv4: None,
                    mtu: None,
                },
            ],
            vlans: vec![
                super::VlanConfig {
                    id: 20,
                    name: "USERS".to_string(),
                    enabled: true,
                },
                super::VlanConfig {
                    id: 10,
                    name: "SERVERS".to_string(),
                    enabled: true,
                },
            ],
            static_routes: vec![
                "192.0.2.0/24 via 192.0.2.1".to_string(),
                "10.0.0.0/8 via 10.0.0.1".to_string(),
            ],
            acls: vec![
                super::ConfiguredAcl {
                    name: "Z-ACL".to_string(),
                    acl_type: "Extended".to_string(),
                    rules: Vec::new(),
                },
                super::ConfiguredAcl {
                    name: "A-ACL".to_string(),
                    acl_type: "Standard".to_string(),
                    rules: Vec::new(),
                },
            ],
            dhcp_pools: vec![
                super::ConfiguredDhcpPool {
                    name: "USERS".to_string(),
                    network: "10.0.0.0/24".to_string(),
                    default_router: None,
                    dns_servers: Vec::new(),
                },
                super::ConfiguredDhcpPool {
                    name: "SERVERS".to_string(),
                    network: "192.0.2.0/24".to_string(),
                    default_router: None,
                    dns_servers: Vec::new(),
                },
            ],
            nat: None,
            ssh: None,
            line_access: Vec::new(),
            warnings: Vec::new(),
        };

        super::sort_configuration_snapshot(&mut snapshot);

        assert_eq!(snapshot.interfaces[0].name, "GigabitEthernet0/0/1");
        assert_eq!(snapshot.vlans[0].id, 10);
        assert_eq!(snapshot.static_routes[0], "10.0.0.0/8 via 10.0.0.1");
        assert_eq!(snapshot.acls[0].name, "A-ACL");
        assert_eq!(snapshot.dhcp_pools[0].name, "SERVERS");
    }

    #[test]
    fn validates_hostname_boundaries_and_characters() {
        assert_eq!(
            super::validate_hostname(" Router-01 ").unwrap(),
            "Router-01"
        );
        assert!(super::validate_hostname("").is_err());
        assert!(super::validate_hostname(&"a".repeat(64)).is_err());
        assert!(super::validate_hostname("router.example").is_err());
    }

    #[test]
    fn validates_dns_domain_labels() {
        assert_eq!(
            super::validate_domain_name(" lab.example.net ").unwrap(),
            "lab.example.net"
        );
        assert!(super::validate_domain_name("").is_err());
        assert!(super::validate_domain_name("-invalid.example").is_err());
        assert!(super::validate_domain_name("invalid..example").is_err());
        assert!(super::validate_domain_name("invalid_label.example").is_err());
    }

    #[test]
    fn validates_vlan_id_ranges_and_names() {
        assert_eq!(
            super::validate_vlan_draft(10, " USERS-01 ").unwrap(),
            "USERS-01"
        );
        assert!(super::validate_vlan_draft(1, "DEFAULT").is_err());
        assert!(super::validate_vlan_draft(1002, "FDDI").is_err());
        assert!(super::validate_vlan_draft(4095, "INVALID").is_err());
        assert!(super::validate_vlan_draft(20, "").is_err());
        assert!(super::validate_vlan_draft(20, "VLAN NAME").is_err());
        assert!(super::validate_vlan_draft(20, &"A".repeat(33)).is_err());
    }

    #[test]
    fn normalizes_ipv4_and_ipv6_name_servers() {
        assert_eq!(
            super::normalize_name_server(" 1.1.1.1 ").unwrap(),
            "1.1.1.1"
        );
        assert_eq!(
            super::normalize_name_server("2001:4860:4860::8888").unwrap(),
            "2001:4860:4860::8888"
        );
        assert!(super::normalize_name_server("dns.example.net").is_err());
    }
}
