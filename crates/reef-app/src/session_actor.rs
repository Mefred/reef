//! A thread-affine session actor for the Tauri application.
//!
//! SessionManager intentionally uses Rc<RefCell<_>> and must remain on one
//! thread. This actor owns it on a dedicated current-thread Tokio runtime and
//! exposes only a Send + Sync message handle to Tauri managed state.

use std::thread;

use reef_engine::model::{DeviceIdentity, InterfaceRef, StaticRoute, VlanConfig};
use reef_engine::parser::parse_running_config;
use reef_engine::parser::{parse_interfaces, parse_routes, parse_vlans};
use reef_session::{
    ConnectionConfig, SerialTransport, SessionManager, SshTransport, TelnetTransport, TransportType,
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
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfiguredInterface {
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub mode: String,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationDraft {
    /// Set-only fields: omitted values preserve the running configuration.
    pub hostname: Option<String>,
    pub domain_name: Option<String>,
    /// Additional name servers; existing servers are preserved.
    #[serde(default)]
    pub add_name_servers: Vec<String>,
}

#[derive(Debug, Serialize)]
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

    pub async fn get_configuration(
        &self,
        device_id: String,
    ) -> Result<ConfigurationSnapshot, String> {
        let (reply, response) = oneshot::channel();
        self.sender
            .send(Command::GetConfiguration { device_id, reply })
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
            .send(Command::PreviewConfiguration { device_id, draft, reply })
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
                        ConfigurationSnapshot {
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
                            warnings: parsed
                                .warnings
                                .into_iter()
                                .map(|warning| {
                                    format!("line {}: {}", warning.line_no, warning.reason)
                                })
                                .collect(),
                        }
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
                        let hostname = hostname.trim();
                        if hostname.is_empty() || hostname.len() > 63
                            || !hostname.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                        {
                            return Err("hostname must be 1-63 ASCII letters, digits, hyphens, or underscores".to_string());
                        }
                        desired.hostname = Some(hostname.to_string());
                    }
                    if let Some(domain) = draft.domain_name {
                        let domain = domain.trim();
                        let valid = !domain.is_empty() && domain.len() <= 253
                            && domain.split('.').all(|label| {
                                !label.is_empty() && label.len() <= 63
                                    && label.as_bytes()[0].is_ascii_alphanumeric()
                                    && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                                    && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                            });
                        if !valid {
                            return Err("domain name must contain valid DNS labels".to_string());
                        }
                        desired.ip_domain_name = Some(domain.to_string());
                    }
                    for server in draft.add_name_servers {
                        let parsed = server.trim().parse::<std::net::IpAddr>()
                            .map_err(|_| format!("invalid name-server IP address: {server}"))?;
                        let normalized = parsed.to_string();
                        if !desired.ip_name_server.contains(&normalized) {
                            desired.ip_name_server.push(normalized);
                        }
                    }

                    let changes = reef_engine::planner::plan(&current, &desired);
                    let problems = reef_engine::validator::validate(&changes, &current);
                    let errors = problems.iter()
                        .filter(|problem| matches!(&problem.severity, &reef_engine::validator::Severity::Error))
                        .map(|problem| format!("{}: {}", problem.element, problem.message))
                        .collect::<Vec<_>>();
                    let warnings = problems.iter()
                        .filter(|problem| matches!(&problem.severity, &reef_engine::validator::Severity::Warning))
                        .map(|problem| format!("{}: {}", problem.element, problem.message))
                        .collect::<Vec<_>>();
                    let diff = reef_engine::diff::render(&changes);
                    let batch = reef_engine::generator::generate(&changes);
                    Ok(ConfigurationPreview {
                        diff,
                        commands: if errors.is_empty() { batch.commands } else { Vec::new() },
                        warnings,
                        errors,
                        affects_connectivity: changes.affects_connectivity,
                    })
                }.await;
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
