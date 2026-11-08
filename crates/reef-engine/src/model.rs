//! Structured configuration data models for Cisco IOS XE.
//!
//! These models are the source of truth for configuration management. They are
//! transport-independent and GUI-independent.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A wrapper around sensitive values (passwords, secrets, keys) that are
/// excluded from logs, diffs, and UI display by default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Secret(pub String);

impl std::fmt::Display for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "****")
    }
}

/// Top-level device configuration parsed from `show running-config` or built
/// by the GUI before applying changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub hostname: Option<String>,
    pub ip_domain_name: Option<String>,
    pub ip_name_server: Vec<String>,
    pub interfaces: HashMap<String, InterfaceConfig>,
    pub vlans: HashMap<u16, VlanConfig>,
    pub ip_routes: Vec<StaticRoute>,
    pub ipv6_routes: Vec<StaticRoute>,
    pub acls: HashMap<String, AclConfig>,
    pub dhcp_pools: HashMap<String, DhcpPool>,
    pub nat: Option<NatConfig>,
    pub ssh: Option<SshConfig>,
    pub line_configs: HashMap<String, LineConfig>,
    pub banner: Option<String>,
}

/// A single interface (physical or logical).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceConfig {
    pub name: String, // e.g., "GigabitEthernet1/0/1", "Vlan10", "Loopback0"
    pub description: Option<String>,
    pub enabled: bool,
    pub mode: InterfaceMode,
    pub access_vlan: Option<u16>,
    pub trunk_vlans: Vec<u16>,
    pub native_vlan: Option<u16>,
    pub ipv4: Option<Ipv4Address>,
    pub ipv6: Option<Ipv6Address>,
    pub mtu: Option<u16>,
    pub duplex: Option<Duplex>,
    pub speed: Option<Speed>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Copy)]
pub enum InterfaceMode {
    Access,
    Trunk,
    Routed,
    L2,
}

impl Default for InterfaceMode {
    fn default() -> Self {
        InterfaceMode::Routed
    }
}

impl Default for InterfaceConfig {
    fn default() -> Self {
        InterfaceConfig {
            name: String::new(),
            description: None,
            enabled: true,
            mode: InterfaceMode::default(),
            access_vlan: None,
            trunk_vlans: Vec::new(),
            native_vlan: None,
            ipv4: None,
            ipv6: None,
            mtu: None,
            duplex: None,
            speed: None,
        }
    }
}

/// IPv4 address with prefix length.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv4Address {
    pub address: std::net::Ipv4Addr,
    pub prefix_len: u8,
}

/// IPv6 address with prefix length.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ipv6Address {
    pub address: std::net::Ipv6Addr,
    pub prefix_len: u8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Duplex {
    Auto,
    Half,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Speed {
    Auto,
    Mbps(u32),
}

/// A VLAN entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VlanConfig {
    pub id: u16,
    pub name: String,
    pub enabled: bool,
}

/// A static route.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StaticRoute {
    pub prefix: ipnet::IpNet,
    pub next_hop: std::net::IpAddr,
    pub admin_distance: Option<u16>,
    pub permanent: bool,
}

/// An ACL (access control list).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclConfig {
    pub name: String,
    pub acl_type: AclType,
    pub rules: Vec<AclRule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Copy)]
pub enum AclType {
    Standard,
    Extended,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AclRule {
    pub action: Action,
    pub protocol: Option<Protocol>,
    pub source: AddressSpec,
    pub destination: AddressSpec,
    pub ports: Option<PortSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Permit,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    Ip,
    Tcp,
    Udp,
    Icmp,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressSpec {
    Any,
    Host(std::net::IpAddr),
    Network(ipnet::IpNet),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortSpec {
    Eq(u16),
    Range(u16, u16),
    Any,
}

/// A DHCP pool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DhcpPool {
    pub name: String,
    pub network: ipnet::IpNet,
    pub default_router: Option<std::net::IpAddr>,
    pub dns_servers: Vec<std::net::IpAddr>,
    pub lease_time: Option<std::time::Duration>,
}

/// NAT configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatConfig {
    pub inside_interfaces: Vec<String>,
    pub outside_interfaces: Vec<String>,
    pub pools: Vec<NatPool>,
    pub rules: Vec<NatRule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatPool {
    pub name: String,
    pub start: std::net::Ipv4Addr,
    pub end: std::net::Ipv4Addr,
    pub netmask: std::net::Ipv4Addr,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NatRule {
    pub name: String,
    pub source_acl: Option<String>,
    pub pool: Option<String>,
    pub overload: bool,
}

/// SSH device-access configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConfig {
    pub enabled: bool,
    pub version: SshVersion,
    pub timeout: Option<u16>,
    pub authentication_retries: Option<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshVersion {
    V1,
    V2,
    Both,
}

impl Default for SshVersion {
    fn default() -> Self {
        SshVersion::V2
    }
}

/// Line configuration (console 0, vty ranges).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineConfig {
    pub line: String, // e.g., "console 0", "vty 0 15"
    pub transport: Vec<Transport>,
    pub password: Option<Secret>,
    pub login: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transport {
    Ssh,
    Telnet,
    All,
    None,
}

/// Device identity gathered during initial connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub hostname: String,
    pub iosxe_version: String,
    pub platform: String,
    pub chassis_serial: String,
    pub processor: String,
    pub interfaces: Vec<InterfaceRef>,
    pub capabilities: DeviceCapabilities,
}

/// A lightweight reference to a discovered interface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InterfaceRef {
    pub name: String,
    pub oper_status: Option<String>,
    pub ipv4: Option<Ipv4Address>,
    pub ipv6: Option<Ipv6Address>,
}

/// Detected device capabilities used to gate available UI screens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    pub supports_l2_switchports: bool,
    pub supports_ipv6: bool,
    pub supports_static_routing: bool,
    pub supports_dynamic_routing: bool,
    pub supports_nat: bool,
    pub supports_dhcp: bool,
    pub supports_acl: bool,
    pub max_vlans: Option<u16>,
    pub interface_naming: InterfaceNaming,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterfaceNaming {
    SlotPort,       // GigabitEthernet0/0/0 (ISR)
    SwitchSlotPort, // GigabitEthernet1/0/1 (stacked Catalyst)
    Other(String),
}

impl Default for DeviceCapabilities {
    fn default() -> Self {
        DeviceCapabilities {
            supports_l2_switchports: false,
            supports_ipv6: true,
            supports_static_routing: true,
            supports_dynamic_routing: false,
            supports_nat: false,
            supports_dhcp: false,
            supports_acl: true,
            max_vlans: None,
            interface_naming: InterfaceNaming::Other(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_interface_config_is_enabled_routed() {
        let iface = InterfaceConfig::default();
        assert!(iface.enabled);
        assert!(matches!(iface.mode, InterfaceMode::Routed));
    }

    #[test]
    fn secret_displays_masked() {
        let s = Secret("hunter2".to_string());
        assert_eq!(format!("{}", s), "****");
    }

    #[test]
    fn device_config_default_is_empty() {
        let cfg = DeviceConfig::default();
        assert!(cfg.interfaces.is_empty());
        assert!(cfg.vlans.is_empty());
        assert!(cfg.ip_routes.is_empty());
        assert!(cfg.hostname.is_none());
    }
}
