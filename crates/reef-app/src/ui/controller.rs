//! Screen controller: bridges Slint UI with reef-engine (planning/generation)
//! and reef-session (device sessions).
//!
//! The controller owns the desired config state, runs plan → diff, and
//! coordinates live device operations through the session manager.

use ipnet::IpNet;
use reef_engine::generator::generate;
use reef_engine::model::{
    AclRule, Action, AddressSpec, DeviceConfig, InterfaceConfig, InterfaceMode, Ipv4Address,
    Protocol,
};
use reef_engine::planner::{plan, ChangeOp, ChangeSet};
use reef_engine::verifier::{verify, VerificationResult};
use reef_session::{
    ConnectionConfig, HostKeyPolicy, SessionManager, TelnetTransport, TransportType,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::rc::Rc;

/// Controller for a single device's configuration screens.
pub struct ScreenController {
    session_manager: Option<Rc<RefCell<SessionManager>>>,
    device_id: Option<String>,
    current_config: DeviceConfig,
    desired_config: DeviceConfig,
    last_diff: String,
    last_verification: Option<VerificationResult>,
    has_session: bool,
    /// Currently selected interface name (for form field updates).
    selected_interface: Option<String>,
    /// Currently selected VLAN ID (for form field updates).
    selected_vlan: Option<String>,
    /// Callback to populate Slint interface form fields from Rust.
    set_interface_form_cb: Option<Box<dyn Fn(String, String, bool, String, String)>>,
}

impl ScreenController {
    pub fn new(current_config: DeviceConfig) -> Self {
        ScreenController {
            session_manager: None,
            device_id: None,
            current_config: current_config.clone(),
            desired_config: current_config,
            last_diff: String::new(),
            last_verification: None,
            has_session: false,
            selected_interface: None,
            selected_vlan: None,
            set_interface_form_cb: None,
        }
    }

    /// Attach a session manager and device ID for live operations.
    pub fn bind_session(
        &mut self,
        session_manager: Rc<RefCell<SessionManager>>,
        device_id: String,
    ) {
        self.session_manager = Some(session_manager);
        self.device_id = Some(device_id);
        self.has_session = true;
    }

    /// Clear the session binding.
    pub fn unbind_session(&mut self) {
        self.session_manager = None;
        self.device_id = None;
        self.has_session = false;
    }

    pub fn set_has_session(&mut self, has: bool) {
        self.has_session = has;
    }

    /// Get the device ID.
    pub fn device_id(&self) -> Option<String> {
        self.device_id.clone()
    }

    /// Register a callback to populate Slint interface form fields from Rust.
    pub fn set_interface_form_callback<F: Fn(String, String, bool, String, String) + 'static>(
        &mut self,
        cb: F,
    ) {
        self.set_interface_form_cb = Some(Box::new(cb));
    }

    /// Notify the UI to populate the interface form fields.
    fn notify_interface_form(&self, name: &str, desc: &str, enabled: bool, ip: &str, mask: &str) {
        if let Some(cb) = &self.set_interface_form_cb {
            cb(
                name.to_string(),
                desc.to_string(),
                enabled,
                ip.to_string(),
                mask.to_string(),
            );
        }
    }

    /// Connect to a device using the given configuration.
    pub async fn connect_device(
        &mut self,
        config: ConnectionConfig,
    ) -> Result<String, reef_session::Error> {
        let sm = self
            .session_manager
            .as_ref()
            .ok_or_else(|| reef_session::Error::Transport("no session manager".to_string()))?;
        let device_id = self.device_id.as_deref().unwrap_or("device").to_string();

        sm.borrow_mut()
            .register(&device_id, TelnetTransport::new(config.clone()));

        sm.borrow_mut().connect(&device_id, config).await?;

        // Identify the device
        let identity = sm.borrow_mut().identify(&device_id).await?;
        let hostname = identity.hostname.clone();
        self.current_config.hostname = Some(hostname.clone());

        // Fetch running-config
        let running_out = sm
            .borrow_mut()
            .send_command(&device_id, "show running-config\r\n")
            .await?;
        let parsed = reef_engine::parser::parse_running_config(&running_out).model;

        // Merge parsed config into current_config (preserve identity fields)
        self.current_config = parsed;
        self.current_config.hostname = Some(hostname);

        // Reset desired config to current
        self.desired_config = self.current_config.clone();

        self.last_diff = String::new();
        self.has_session = true;

        Ok(format!(
            "Connected to {} ({})",
            identity.hostname, identity.iosxe_version
        ))
    }

    /// Disconnect from the device.
    pub async fn disconnect_device(&mut self) -> Result<(), reef_session::Error> {
        if let Some(sm) = &self.session_manager {
            if let Some(device_id) = &self.device_id {
                sm.borrow_mut().disconnect(device_id).await?;
            }
        }
        self.has_session = false;
        Ok(())
    }

    /// Refresh device config from the live device.
    pub async fn refresh_config(&mut self) -> Result<String, reef_session::Error> {
        let sm = self
            .session_manager
            .as_ref()
            .ok_or_else(|| reef_session::Error::Transport("no session manager".to_string()))?;
        let device_id = self
            .device_id
            .as_deref()
            .ok_or_else(|| reef_session::Error::Transport("no device id".to_string()))?;

        let running_out = sm
            .borrow_mut()
            .send_command(device_id, "show running-config\r\n")
            .await?;
        let parsed = reef_engine::parser::parse_running_config(&running_out).model;
        self.current_config = parsed;

        // Re-plan diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);

        Ok(self.last_diff.clone())
    }

    /// Handle an interface edit from the UI: update desired config, re-plan,
    /// re-diff. Returns the new diff text for display.
    pub fn update_interface(
        &mut self,
        name: &str,
        description: Option<String>,
        enabled: bool,
        ip: Option<&str>,
        mask: Option<&str>,
    ) -> String {
        let iface = self
            .desired_config
            .interfaces
            .entry(name.to_string())
            .or_insert(InterfaceConfig {
                name: name.to_string(),
                description: None,
                enabled: true,
                mode: InterfaceMode::Routed,
                access_vlan: None,
                trunk_vlans: Vec::new(),
                native_vlan: None,
                ipv4: None,
                ipv6: None,
                mtu: None,
                duplex: None,
                speed: None,
            });

        if let Some(d) = description {
            iface.description = Some(d);
        }
        iface.enabled = enabled;

        if let Some(addr) = ip {
            if let (Ok(ip_addr), Ok(mask_addr)) = (
                addr.parse::<Ipv4Addr>(),
                mask.unwrap_or("255.255.255.0").parse::<Ipv4Addr>(),
            ) {
                let prefix_len = mask_to_prefix_len(mask_addr).unwrap_or(24);
                iface.ipv4 = Some(Ipv4Address {
                    address: ip_addr,
                    prefix_len,
                });
            }
        }

        // Re-plan and re-diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);
        self.last_diff.clone()
    }

    /// Load VLANs from the desired config (returns list of VLAN IDs).
    pub fn load_vlans(&self) -> Vec<String> {
        self.desired_config
            .vlans
            .keys()
            .map(|k| k.to_string())
            .collect()
    }

    /// Select a VLAN for editing: populate form fields.
    pub fn select_vlan(&mut self, vlan_id: &str) -> (String, String, bool) {
        self.selected_vlan = Some(vlan_id.to_string());
        if let Some(vlan) = self
            .desired_config
            .vlans
            .get(&vlan_id.parse::<u16>().unwrap_or(0))
        {
            (vlan_id.to_string(), vlan.name.clone(), vlan.enabled)
        } else {
            (vlan_id.to_string(), String::new(), true)
        }
    }

    /// Save VLAN changes from the UI: update desired config, re-plan,
    /// re-diff. Returns the new diff text for display.
    pub fn save_vlan(&mut self, vlan_id: &str, name: &str, enabled: bool) -> String {
        let vlan_id_num: u16 = vlan_id.parse().unwrap_or(0);
        let vlan = self.desired_config.vlans.entry(vlan_id_num).or_insert(
            reef_engine::model::VlanConfig {
                id: vlan_id_num,
                name: String::new(),
                enabled: true,
            },
        );

        vlan.name = name.to_string();
        vlan.enabled = enabled;

        // Re-plan and re-diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);
        self.last_diff.clone()
    }

    /// Load routes from the desired config (returns list of prefix strings).
    pub fn load_routes(&self) -> Vec<String> {
        self.desired_config
            .ip_routes
            .iter()
            .map(|r| r.prefix.to_string())
            .collect()
    }

    /// Save a static route from the UI: update desired config, re-plan,
    /// re-diff. Returns the new diff text for display.
    pub fn save_route(
        &mut self,
        prefix: &str,
        next_hop: &str,
        distance: &str,
        _permanent: &str,
    ) -> String {
        let prefix_net: ipnet::IpNet = prefix
            .parse()
            .unwrap_or_else(|_| "0.0.0.0/0".parse().unwrap());
        let next_hop: std::net::IpAddr = next_hop
            .parse()
            .unwrap_or_else(|_| "0.0.0.0".parse().unwrap());
        let admin_distance: Option<u16> = distance.parse().ok();

        // Check if route already exists
        if let Some(existing) = self
            .desired_config
            .ip_routes
            .iter_mut()
            .find(|r| r.prefix == prefix_net)
        {
            existing.next_hop = next_hop;
            existing.admin_distance = admin_distance;
        } else {
            self.desired_config
                .ip_routes
                .push(reef_engine::model::StaticRoute {
                    prefix: prefix_net,
                    next_hop,
                    admin_distance,
                    permanent: false,
                });
        }

        // Re-plan and re-diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);
        self.last_diff.clone()
    }

    /// Load ACLs from the desired config (returns list of ACL names).
    pub fn load_acls(&self) -> Vec<String> {
        self.desired_config.acls.keys().cloned().collect()
    }

    /// Select an ACL for editing: populate form fields.
    pub fn select_acl(&self, acl_name: &str) -> (String, String, String, String, String) {
        if let Some(acl) = self.desired_config.acls.get(acl_name) {
            let first_rule =
                acl.rules
                    .first()
                    .cloned()
                    .unwrap_or_else(|| reef_engine::model::AclRule {
                        action: reef_engine::model::Action::Permit,
                        protocol: Some(reef_engine::model::Protocol::Ip),
                        source: reef_engine::model::AddressSpec::Any,
                        destination: reef_engine::model::AddressSpec::Any,
                        ports: None,
                    });
            let action = match first_rule.action {
                reef_engine::model::Action::Permit => "permit",
                reef_engine::model::Action::Deny => "deny",
            };
            let protocol = match first_rule.protocol {
                Some(reef_engine::model::Protocol::Ip) => "ip".to_string(),
                Some(reef_engine::model::Protocol::Tcp) => "tcp".to_string(),
                Some(reef_engine::model::Protocol::Udp) => "udp".to_string(),
                Some(reef_engine::model::Protocol::Icmp) => "icmp".to_string(),
                Some(reef_engine::model::Protocol::Other(s)) => s.clone(),
                None => "ip".to_string(),
            };
            let source = match &first_rule.source {
                reef_engine::model::AddressSpec::Any => "any".to_string(),
                reef_engine::model::AddressSpec::Host(ip) => format!("host {}", ip),
                reef_engine::model::AddressSpec::Network(net) => format!("{} {}", net.addr(), net),
            };
            let dest = match &first_rule.destination {
                reef_engine::model::AddressSpec::Any => "any".to_string(),
                reef_engine::model::AddressSpec::Host(ip) => format!("host {}", ip),
                reef_engine::model::AddressSpec::Network(net) => format!("{} {}", net.addr(), net),
            };
            (
                acl_name.to_string(),
                action.to_string(),
                protocol.to_string(),
                source,
                dest,
            )
        } else {
            (
                "".to_string(),
                "permit".to_string(),
                "ip".to_string(),
                "any".to_string(),
                "any".to_string(),
            )
        }
    }

    /// Save an ACL rule from the UI: update desired config, re-plan,
    /// re-diff. Returns the new diff text for display.
    pub fn save_acl_rule(
        &mut self,
        acl_name: &str,
        action: &str,
        protocol: &str,
        source: &str,
        destination: &str,
        _ports: &str,
        _wildcard: &str,
    ) -> String {
        let action_enum = match action {
            "permit" => reef_engine::model::Action::Permit,
            _ => reef_engine::model::Action::Deny,
        };
        let protocol_enum = match protocol {
            "tcp" => reef_engine::model::Protocol::Tcp,
            "udp" => reef_engine::model::Protocol::Udp,
            "icmp" => reef_engine::model::Protocol::Icmp,
            _ => reef_engine::model::Protocol::Ip,
        };
        let source_spec = Self::parse_address_spec(source);
        let dest_spec = Self::parse_address_spec(destination);

        // Find or create ACL
        let acl = self
            .desired_config
            .acls
            .entry(acl_name.to_string())
            .or_insert(reef_engine::model::AclConfig {
                name: acl_name.to_string(),
                acl_type: reef_engine::model::AclType::Standard,
                rules: Vec::new(),
            });

        // Remove existing rules and add new one (simplified: replace all rules)
        acl.rules = vec![reef_engine::model::AclRule {
            action: action_enum,
            protocol: Some(protocol_enum),
            source: source_spec,
            destination: dest_spec,
            ports: None,
        }];

        // Re-plan and re-diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);
        self.last_diff.clone()
    }

    fn parse_address_spec(s: &str) -> reef_engine::model::AddressSpec {
        if s == "any" {
            return reef_engine::model::AddressSpec::Any;
        }
        if s.starts_with("host ") {
            let ip: std::net::IpAddr = s[5..]
                .parse()
                .unwrap_or_else(|_| "0.0.0.0".parse().unwrap());
            return reef_engine::model::AddressSpec::Host(ip);
        }
        if s.contains('/') {
            let parts: Vec<&str> = s.split('/').collect();
            if let (Ok(ip), Ok(prefix)) = (parts[0].parse(), parts[1].parse::<u8>()) {
                return reef_engine::model::AddressSpec::Network(
                    ipnet::IpNet::new(ip, prefix).unwrap_or_else(|_| "0.0.0.0/0".parse().unwrap()),
                );
            }
        }
        reef_engine::model::AddressSpec::Any
    }

    /// Load DHCP pools from the desired config (returns list of pool names).
    pub fn load_dhcp_pools(&self) -> Vec<String> {
        self.desired_config.dhcp_pools.keys().cloned().collect()
    }

    /// Select a DHCP pool for editing: populate form fields.
    pub fn select_dhcp_pool(
        &self,
        pool_name: &str,
    ) -> (String, String, String, String, String, String) {
        if let Some(pool) = self.desired_config.dhcp_pools.get(pool_name) {
            (
                pool.name.clone(),
                pool.network.addr().to_string(),
                pool.network.prefix_len().to_string(),
                pool.default_router
                    .map(|ip| ip.to_string())
                    .unwrap_or_default(),
                pool.dns_servers
                    .iter()
                    .map(|ip| ip.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
                pool.lease_time
                    .map(|d| d.as_secs().to_string())
                    .unwrap_or_default(),
            )
        } else {
            (
                pool_name.to_string(),
                "0.0.0.0".to_string(),
                "24".to_string(),
                String::new(),
                String::new(),
                String::new(),
            )
        }
    }

    /// Save DHCP pool changes from the UI: update desired config, re-plan,
    /// re-diff. Returns the new diff text for display.
    pub fn save_dhcp_pool(
        &mut self,
        name: &str,
        network: &str,
        mask: &str,
        default_router: &str,
        dns_servers: &str,
        lease_time: &str,
    ) -> String {
        let network_str = format!("{}/{}", network, mask);
        let net: ipnet::IpNet = network_str
            .parse()
            .unwrap_or_else(|_| "0.0.0.0/0".parse().unwrap());
        let default_router: Option<std::net::IpAddr> = if default_router.is_empty() {
            None
        } else {
            default_router.parse().ok()
        };
        let dns_servers: Vec<std::net::IpAddr> = dns_servers
            .split_whitespace()
            .filter_map(|s| s.parse().ok())
            .collect();
        let lease_time: Option<std::time::Duration> = if lease_time.is_empty() {
            None
        } else {
            lease_time
                .parse::<u64>()
                .ok()
                .map(std::time::Duration::from_secs)
        };

        let pool = self
            .desired_config
            .dhcp_pools
            .entry(name.to_string())
            .or_insert(reef_engine::model::DhcpPool {
                name: name.to_string(),
                network: net,
                default_router: None,
                dns_servers: Vec::new(),
                lease_time: None,
            });

        pool.network = net;
        pool.default_router = default_router;
        pool.dns_servers = dns_servers;
        pool.lease_time = lease_time;

        // Re-plan and re-diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);
        self.last_diff.clone()
    }

    /// Load NAT config from the desired config (returns NAT type, inside, outside).
    pub fn load_nat_config(&self) -> (String, String, String) {
        if let Some(nat) = &self.desired_config.nat {
            (
                "static".to_string(),
                nat.inside_interfaces.join(", "),
                nat.outside_interfaces.join(", "),
            )
        } else {
            ("static".to_string(), String::new(), String::new())
        }
    }

    /// Save NAT config from the UI: update desired config, re-plan,
    /// re-diff. Returns the new diff text for display.
    pub fn save_nat_config(
        &mut self,
        nat_type: &str,
        inside: &str,
        outside: &str,
        _extra: &str,
    ) -> String {
        let inside_interfaces: Vec<String> =
            inside.split_whitespace().map(|s| s.to_string()).collect();
        let outside_interfaces: Vec<String> =
            outside.split_whitespace().map(|s| s.to_string()).collect();

        self.desired_config.nat = Some(reef_engine::model::NatConfig {
            inside_interfaces,
            outside_interfaces,
            pools: Vec::new(),
            rules: Vec::new(),
        });

        // Re-plan and re-diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);
        self.last_diff.clone()
    }

    /// Load SSH config from the desired config.
    pub fn load_ssh_config(&self) -> (bool, String, String, String) {
        if let Some(ssh) = &self.desired_config.ssh {
            (
                ssh.enabled,
                match ssh.version {
                    reef_engine::model::SshVersion::V1 => "1".to_string(),
                    reef_engine::model::SshVersion::V2 => "2".to_string(),
                    reef_engine::model::SshVersion::Both => "both".to_string(),
                },
                ssh.timeout.map(|t| t.to_string()).unwrap_or_default(),
                ssh.authentication_retries
                    .map(|r| r.to_string())
                    .unwrap_or_default(),
            )
        } else {
            (false, "2".to_string(), String::new(), String::new())
        }
    }

    /// Save SSH config from the UI: update desired config, re-plan,
    /// re-diff. Returns the new diff text for display.
    pub fn save_ssh_config(
        &mut self,
        enabled: bool,
        version: &str,
        timeout: &str,
        retries: &str,
    ) -> String {
        let ssh_version = match version {
            "1" => reef_engine::model::SshVersion::V1,
            "both" => reef_engine::model::SshVersion::Both,
            _ => reef_engine::model::SshVersion::V2,
        };
        let timeout: Option<u16> = if timeout.is_empty() {
            None
        } else {
            timeout.parse().ok()
        };
        let retries: Option<u16> = if retries.is_empty() {
            None
        } else {
            retries.parse().ok()
        };

        self.desired_config.ssh = Some(reef_engine::model::SshConfig {
            enabled,
            version: ssh_version,
            timeout,
            authentication_retries: retries,
        });

        // Re-plan and re-diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);
        self.last_diff.clone()
    }

    /// Load system config from the desired config.
    pub fn load_system_config(&self) -> (String, String, String) {
        (
            self.desired_config.hostname.clone().unwrap_or_default(),
            self.desired_config
                .ip_domain_name
                .clone()
                .unwrap_or_default(),
            self.desired_config.ip_name_server.join(" "),
        )
    }

    /// Save system config from the UI: update desired config, re-plan,
    /// re-diff. Returns the new diff text for display.
    pub fn save_system_config(&mut self, hostname: &str, domain: &str, dns: &str) -> String {
        self.desired_config.hostname = if hostname.is_empty() {
            None
        } else {
            Some(hostname.to_string())
        };
        self.desired_config.ip_domain_name = if domain.is_empty() {
            None
        } else {
            Some(domain.to_string())
        };
        self.desired_config.ip_name_server = if dns.is_empty() {
            Vec::new()
        } else {
            dns.split_whitespace().map(|s| s.to_string()).collect()
        };

        // Re-plan and re-diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);
        self.last_diff.clone()
    }

    /// Get the currently selected VLAN name.
    pub fn selected_vlan_name(&self) -> Option<String> {
        self.selected_vlan.clone()
    }

    /// Get the current diff text.
    pub fn diff(&self) -> &str {
        &self.last_diff
    }

    /// Whether the current desired config would be disruptive (e.g. changing
    /// an interface that has an IP address — potential management-interface
    /// impact).
    pub fn is_disruptive(&self) -> bool {
        self.desired_config
            .interfaces
            .values()
            .any(|i| i.ipv4.is_some())
            || self
                .current_config
                .interfaces
                .values()
                .any(|i| i.ipv4.is_some())
    }

    /// Get the list of interface names.
    pub fn interface_names(&self) -> Vec<String> {
        self.desired_config.interfaces.keys().cloned().collect()
    }

    /// Get interface IP for display.
    pub fn interface_ip(&self, name: &str) -> Option<String> {
        self.desired_config
            .interfaces
            .get(name)
            .and_then(|i| i.ipv4.as_ref())
            .map(|ip| format!("{}/{}", ip.address, ip.prefix_len))
    }

    /// Get interface subnet mask for display.
    pub fn interface_mask(&self, name: &str) -> Option<String> {
        self.desired_config
            .interfaces
            .get(name)
            .and_then(|i| i.ipv4.as_ref())
            .map(|ip| {
                let bits: u32 = if ip.prefix_len == 0 {
                    0
                } else {
                    !((1u32 << (32 - ip.prefix_len)) - 1)
                };
                std::net::Ipv4Addr::from(bits.to_be_bytes()).to_string()
            })
    }

    /// Get interface enabled state.
    pub fn interface_enabled(&self, name: &str) -> bool {
        self.desired_config
            .interfaces
            .get(name)
            .map(|i| i.enabled)
            .unwrap_or(false)
    }

    /// Get interface description.
    pub fn interface_description(&self, name: &str) -> Option<String> {
        self.desired_config
            .interfaces
            .get(name)
            .and_then(|i| i.description.clone())
    }

    /// Generate the command batch for the current desired config.
    pub fn generate_batch(&self) -> reef_engine::generator::CommandBatch {
        let change_set = plan(&self.current_config, &self.desired_config);
        generate(&change_set)
    }

    /// Apply the planned changes to the live device.
    pub async fn apply_changes(&mut self) -> Result<String, reef_session::Error> {
        let sm = self
            .session_manager
            .as_ref()
            .ok_or_else(|| reef_session::Error::Transport("no session manager".to_string()))?;
        let device_id = self
            .device_id
            .as_deref()
            .ok_or_else(|| reef_session::Error::Transport("no device id".to_string()))?;

        let change_set = plan(&self.current_config, &self.desired_config);

        // Check for disruptive changes
        let is_disruptive = change_set.affects_connectivity;

        // Generate commands
        let batch = generate(&change_set);

        if batch.commands.is_empty() {
            return Ok("No changes to apply.".to_string());
        }

        // Backup running-config before applying
        {
            let sm = &*sm.borrow();
            let _ = sm.save_config(device_id).await;

            // Send commands
            sm.config_terminal(device_id).await?;
            for cmd in &batch.commands {
                sm.send_config(device_id, cmd).await?;
            }
            sm.exit_config(device_id).await?;

            // Save to startup-config if disruptive
            if is_disruptive || change_set.affects_connectivity {
                sm.save_config(device_id).await?;
            }

            // Verify by re-reading running-config
            let running_out = sm
                .send_command(device_id, "show running-config\r\n")
                .await?;
            let parsed = reef_engine::parser::parse_running_config(&running_out).model;
            let result = verify(&parsed, &self.desired_config);
            self.last_verification = Some(result.clone());

            if result.is_success() {
                self.current_config = self.desired_config.clone();
                self.last_diff = String::new();
                Ok(format!("Apply successful. Verification: verified."))
            } else {
                Err(reef_session::Error::Transport(format!(
                    "Apply failed verification: {:?}",
                    result
                )))
            }
        }
    }

    /// Roll back the last applied changes.
    pub async fn rollback_changes(&mut self) -> Result<String, reef_session::Error> {
        let sm = self
            .session_manager
            .as_ref()
            .ok_or_else(|| reef_session::Error::Transport("no session manager".to_string()))?;
        let device_id = self
            .device_id
            .as_deref()
            .ok_or_else(|| reef_session::Error::Transport("no device id".to_string()))?;

        // Rollback: restore the current_config (pre-apply state) by generating
        // reverse commands from the current desired back to current.
        let reverse_cs = plan(&self.desired_config, &self.current_config);
        let reverse_batch = generate(&reverse_cs);

        if reverse_batch.commands.is_empty() {
            return Ok("Nothing to roll back.".to_string());
        }

        {
            let sm = &*sm.borrow();
            sm.config_terminal(device_id).await?;
            for cmd in &reverse_batch.commands {
                sm.send_config(device_id, cmd).await?;
            }
            sm.exit_config(device_id).await?;
            sm.save_config(device_id).await?;

            // Verify rollback
            let running_out = sm
                .send_command(device_id, "show running-config\r\n")
                .await?;
            let parsed = reef_engine::parser::parse_running_config(&running_out).model;
            let result = verify(&parsed, &self.current_config);

            if result.is_success() {
                self.desired_config = self.current_config.clone();
                self.last_diff = String::new();
                Ok("Rollback successful. Configuration restored.".to_string())
            } else {
                Err(reef_session::Error::Transport(format!(
                    "Rollback verification failed: {:?}",
                    result
                )))
            }
        }
    }

    /// Save running-config to startup-config on the device.
    pub async fn save_config(&mut self) -> Result<(), reef_session::Error> {
        let sm = self
            .session_manager
            .as_ref()
            .ok_or_else(|| reef_session::Error::Transport("no session manager".to_string()))?;
        let device_id = self
            .device_id
            .as_deref()
            .ok_or_else(|| reef_session::Error::Transport("no device id".to_string()))?;
        {
            let sm = &*sm.borrow();
            sm.save_config(device_id).await
        }
    }

    /// Get a reference to the current (pre-change) config.
    pub fn current_config(&self) -> &DeviceConfig {
        &self.current_config
    }

    /// Get a reference to the desired config.
    pub fn desired_config(&self) -> &DeviceConfig {
        &self.desired_config
    }

    /// Get a mutable reference to the desired config.
    pub fn desired_config_mut(&mut self) -> &mut DeviceConfig {
        &mut self.desired_config
    }

    /// Set the currently selected interface name.
    pub fn set_selected_interface(&mut self, name: String) {
        self.selected_interface = Some(name);
    }

    /// Get the currently selected interface name.
    pub fn selected_interface_name(&self) -> Option<String> {
        self.selected_interface.clone()
    }

    /// Returns whether a live session is available for apply.
    pub fn has_session(&self) -> bool {
        self.has_session
    }

    /// Get the last verification result.
    pub fn last_verification(&self) -> Option<&VerificationResult> {
        self.last_verification.as_ref()
    }
}

fn mask_to_prefix_len(mask: Ipv4Addr) -> Option<u8> {
    let bits = u32::from_be_bytes(mask.octets());
    if bits == 0 {
        return Some(0);
    }
    let trailing = bits.trailing_zeros();
    if bits == (!0_u32 << trailing) {
        Some((32 - trailing) as u8)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_engine::model::DeviceConfig;

    #[test]
    fn controller_update_interface() {
        let cfg = DeviceConfig::default();
        let mut ctrl = ScreenController::new(cfg);
        let diff = ctrl.update_interface(
            "GigabitEthernet1/0/1",
            Some("Uplink".into()),
            true,
            Some("10.0.0.1"),
            Some("255.255.255.0"),
        );
        assert!(diff.contains("GigabitEthernet1/0/1"));
        assert!(diff.contains("[ADD]"));
    }

    #[test]
    fn controller_interface_ip() {
        let cfg = DeviceConfig::default();
        let mut ctrl = ScreenController::new(cfg);
        ctrl.update_interface(
            "GigabitEthernet1/0/1",
            None,
            true,
            Some("10.0.0.1"),
            Some("255.255.255.0"),
        );
        assert_eq!(
            ctrl.interface_ip("GigabitEthernet1/0/1"),
            Some("10.0.0.1/24".to_string())
        );
        assert!(ctrl.interface_enabled("GigabitEthernet1/0/1"));
        assert_eq!(
            ctrl.interface_names(),
            vec!["GigabitEthernet1/0/1".to_string()]
        );
    }
}
