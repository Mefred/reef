//! Command generator: structured model + change set -> IOS XE CLI commands.
//!
//! Pure function: takes a `ChangeSet` and produces an ordered `CommandBatch`.
//! No device or GUI dependency. Commands are idempotent where IOS XE allows
//! re-entering existing configuration.

use crate::model::{
    AclRule, Action, AddressSpec, AclType, DhcpPool, InterfaceConfig, InterfaceMode, NatConfig,
    NatPool, NatRule, PortSpec, Protocol, Secret, StaticRoute,
};
use crate::planner::{ChangeOp, ChangeSet};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};

/// An ordered batch of IOS XE commands to execute in one session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandBatch {
    pub enter_config_mode: bool,
    pub commands: Vec<String>,
    pub exit_config_mode: bool,
    pub save_to_startup: bool,
}

/// Generate commands from a change set.
pub fn generate(change_set: &ChangeSet) -> CommandBatch {
    let mut commands: Vec<String> = Vec::new();

    // --- System settings (global config) ---
    for op in &change_set.system {
        match op {
            ChangeOp::Add | ChangeOp::Modify => {
                if let Some(hostname) = &change_set.hostname {
                    commands.push(format!("hostname {}", hostname));
                }
                if let Some(domain) = &change_set.ip_domain_name {
                    commands.push(format!("ip domain-name {}", domain));
                }
                for ns in &change_set.ip_name_servers {
                    commands.push(format!("ip name-server {}", ns));
                }
            }
            ChangeOp::Remove => {}
            ChangeOp::NoChange => {}
        }
    }

    // --- Interfaces ---
    for (name, op) in &change_set.interface_changes {
        match op {
            ChangeOp::Add | ChangeOp::Modify => {
                if let Some(iface) = change_set.interfaces.get(name) {
                    commands.push(format!("interface {}", name));
                    emit_interface_commands(iface, &mut commands);
                    commands.push("exit".to_string());
                }
            }
            ChangeOp::Remove => {
                commands.push(format!("no interface {}", name));
            }
            ChangeOp::NoChange => {}
        }
    }

    // --- VLANs ---
    for (id, op) in &change_set.vlan_changes {
        match op {
            ChangeOp::Add | ChangeOp::Modify => {
                if let Some(vlan) = change_set.vlans.get(id) {
                    commands.push(format!("vlan {}", vlan.id));
                    if !vlan.name.is_empty() {
                        commands.push(format!("name {}", vlan.name));
                    }
                    if !vlan.enabled {
                        commands.push("shutdown".to_string());
                    }
                    commands.push("exit".to_string());
                }
            }
            ChangeOp::Remove => {
                commands.push(format!("no vlan {}", id));
            }
            ChangeOp::NoChange => {}
        }
    }

    // --- Static IPv4 routes ---
    for (i, op) in change_set.ip_routes.iter().enumerate() {
        match op {
            ChangeOp::Add | ChangeOp::Modify => {
                if i < change_set.routes.len() {
                    let route = &change_set.routes[i];
                    let mask = netmask(route.prefix.prefix_len());
                    let mut cmd = format!(
                        "ip route {} {} {}",
                        route.prefix.addr(), mask, route.next_hop
                    );
                    if let Some(dist) = route.admin_distance {
                        cmd.push_str(&format!(" {}", dist));
                    }
                    commands.push(cmd);
                }
            }
            ChangeOp::Remove => {
                if i < change_set.routes.len() {
                    let route = &change_set.routes[i];
                    let mask = netmask(route.prefix.prefix_len());
                    commands.push(format!(
                        "no ip route {} {} {}",
                        route.prefix.addr(), mask, route.next_hop
                    ));
                }
            }
            ChangeOp::NoChange => {}
        }
    }

    // --- ACLs ---
    for (name, op) in &change_set.acls {
        match op {
            ChangeOp::Add | ChangeOp::Modify => {
                if let Some(acl) = change_set.acl_configs.get(name) {
                    let type_str = match acl.acl_type {
                        AclType::Standard => "standard",
                        AclType::Extended => "extended",
                    };
                    commands.push(format!("ip access-list {} {}", type_str, acl.name));
                    for rule in &acl.rules {
                        commands.push(emit_acl_rule(rule));
                    }
                    commands.push("exit".to_string());
                }
            }
            ChangeOp::Remove => {
                commands.push(format!("no ip access-list {}", name));
            }
            ChangeOp::NoChange => {}
        }
    }

    // --- DHCP pools ---
    for (name, op) in &change_set.dhcp_pools {
        match op {
            ChangeOp::Add | ChangeOp::Modify => {
                if let Some(pool) = change_set.dhcp_pool_configs.get(name) {
                    commands.push(format!("ip dhcp pool {}", pool.name));
                    commands.push(format!(
                        "network {} {}",
                        pool.network.addr(),
                        netmask(pool.network.prefix_len())
                    ));
                    if let Some(router) = pool.default_router {
                        commands.push(format!("default-router {}", router));
                    }
                    for dns in &pool.dns_servers {
                        commands.push(format!("dns-server {}", dns));
                    }
                    if let Some(lease) = pool.lease_time {
                        let secs = lease.as_secs();
                        let days = secs / 86400;
                        let hours = (secs % 86400) / 3600;
                        let mins = (secs % 3600) / 60;
                        commands.push(format!("lease {} {} {}", days, hours, mins));
                    }
                    commands.push("exit".to_string());
                }
            }
            ChangeOp::Remove => {
                commands.push(format!("no ip dhcp pool {}", name));
            }
            ChangeOp::NoChange => {}
        }
    }

    CommandBatch {
        enter_config_mode: !commands.is_empty(),
        commands,
        exit_config_mode: true,
        save_to_startup: change_set.affects_connectivity,
    }
}

fn emit_interface_commands(iface: &InterfaceConfig, commands: &mut Vec<String>) {
    if let Some(desc) = &iface.description {
        commands.push(format!("description {}", desc));
    }
    if let Some(ip) = &iface.ipv4 {
        commands.push(format!(
            "ip address {} {}",
            ip.address,
            netmask(ip.prefix_len)
        ));
    }
    if let Some(ip) = &iface.ipv6 {
        commands.push(format!(
            "ipv6 address {} {}",
            ip.address,
            ip.prefix_len
        ));
    }
    if !iface.enabled {
        commands.push("shutdown".to_string());
    }
    match iface.mode {
        InterfaceMode::Access => {
            commands.push("switchport mode access".to_string());
            if let Some(vlan) = iface.access_vlan {
                commands.push(format!("switchport access vlan {}", vlan));
            }
        }
        InterfaceMode::Trunk => {
            commands.push("switchport mode trunk".to_string());
            if !iface.trunk_vlans.is_empty() {
                let list: Vec<String> = iface.trunk_vlans.iter().map(|v| v.to_string()).collect();
                commands.push(format!("switchport trunk allowed vlan {}", list.join(",")));
            }
            if let Some(native) = iface.native_vlan {
                commands.push(format!("switchport trunk native vlan {}", native));
            }
        }
        InterfaceMode::Routed => {
            commands.push("no switchport".to_string());
        }
        InterfaceMode::L2 => {}
    }
    if let Some(mtu) = iface.mtu {
        commands.push(format!("mtu {}", mtu));
    }
    if let Some(speed) = &iface.speed {
        match speed {
            crate::model::Speed::Auto => commands.push("speed auto".to_string()),
            crate::model::Speed::Mbps(m) => commands.push(format!("speed {}", m)),
        }
    }
    if let Some(duplex) = &iface.duplex {
        match duplex {
            crate::model::Duplex::Auto => commands.push("duplex auto".to_string()),
            crate::model::Duplex::Half => commands.push("duplex half".to_string()),
            crate::model::Duplex::Full => commands.push("duplex full".to_string()),
        }
    }
}

fn emit_acl_rule(rule: &AclRule) -> String {
    let action = match rule.action {
        Action::Permit => "permit",
        Action::Deny => "deny",
    };
    let proto = rule
        .protocol
        .as_ref()
        .map(|p| match p {
            Protocol::Ip => "ip",
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
            Protocol::Icmp => "icmp",
            Protocol::Other(s) => s.as_str(),
        })
        .unwrap_or("ip");
    let src = emit_address_spec(&rule.source);
    let dst = emit_address_spec(&rule.destination);
    format!("{} {} {} {}", action, proto, src, dst)
}

fn emit_address_spec(spec: &AddressSpec) -> String {
    match spec {
        AddressSpec::Any => "any".to_string(),
        AddressSpec::Host(ip) => format!("host {}", ip),
        AddressSpec::Network(net) => format!("{} {}", net.addr(), netmask(net.prefix_len())),
    }
}

fn netmask(prefix_len: u8) -> std::net::Ipv4Addr {
    let bits: u32 = if prefix_len == 0 {
        0
    } else {
        !((1u32 << (32 - prefix_len)) - 1)
    };
    std::net::Ipv4Addr::from(bits.to_be_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Duplex, Ipv4Address, Ipv6Address, Secret, Speed};
    use std::collections::HashMap;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn generate_interface_modify() {
        let mut interfaces = HashMap::new();
        interfaces.insert(
            "GigabitEthernet1/0/1".to_string(),
            InterfaceConfig {
                name: "GigabitEthernet1/0/1".to_string(),
                description: Some("Uplink".to_string()),
                ipv4: Some(Ipv4Address {
                    address: Ipv4Addr::new(10, 0, 0, 1),
                    prefix_len: 24,
                }),
                enabled: true,
                mode: InterfaceMode::Routed,
                ..Default::default()
            },
        );

        let cs = ChangeSet {
            hostname: None,
            ip_domain_name: None,
            ip_name_servers: Vec::new(),
            system: Vec::new(),
            interface_changes: vec![("GigabitEthernet1/0/1".to_string(), ChangeOp::Modify)],
            interfaces: interfaces.clone(),
            vlan_changes: Vec::new(),
            vlans: HashMap::new(),
            ip_routes: Vec::new(),
            routes: Vec::new(),
            acls: Vec::new(),
            acl_configs: HashMap::new(),
            dhcp_pools: Vec::new(),
            dhcp_pool_configs: HashMap::new(),
            affects_connectivity: false,
        };

        let batch = generate(&cs);
        assert!(batch.enter_config_mode);
        assert!(batch
            .commands
            .iter()
            .any(|c| c == "interface GigabitEthernet1/0/1"));
        assert!(batch
            .commands
            .iter()
            .any(|c| c.contains("ip address 10.0.0.1 255.255.255.0")));
    }

    #[test]
    fn generate_trunk_interface() {
        let mut interfaces = HashMap::new();
        interfaces.insert(
            "GigabitEthernet1/0/2".to_string(),
            InterfaceConfig {
                name: "GigabitEthernet1/0/2".to_string(),
                description: None,
                ipv4: None,
                ipv6: None,
                enabled: true,
                mode: InterfaceMode::Trunk,
                access_vlan: None,
                trunk_vlans: vec![10, 20, 30],
                native_vlan: Some(99),
                mtu: None,
                speed: None,
                duplex: None,
            },
        );

        let cs = ChangeSet {
            hostname: None,
            ip_domain_name: None,
            ip_name_servers: Vec::new(),
            system: Vec::new(),
            interface_changes: vec![("GigabitEthernet1/0/2".to_string(), ChangeOp::Add)],
            interfaces: interfaces.clone(),
            vlan_changes: Vec::new(),
            vlans: HashMap::new(),
            ip_routes: Vec::new(),
            routes: Vec::new(),
            acls: Vec::new(),
            acl_configs: HashMap::new(),
            dhcp_pools: Vec::new(),
            dhcp_pool_configs: HashMap::new(),
            affects_connectivity: false,
        };

        let batch = generate(&cs);
        assert!(batch.commands.iter().any(|c| c == "switchport mode trunk"));
        assert!(batch
            .commands
            .iter()
            .any(|c| c == "switchport trunk allowed vlan 10,20,30"));
        assert!(batch
            .commands
            .iter()
            .any(|c| c == "switchport trunk native vlan 99"));
    }

    #[test]
    fn generate_access_list() {
        let mut acls = HashMap::new();
        acls.insert(
            "ACL1".to_string(),
            crate::model::AclConfig {
                name: "ACL1".to_string(),
                acl_type: AclType::Standard,
                rules: vec![AclRule {
                    action: Action::Permit,
                    protocol: Some(Protocol::Ip),
                    source: AddressSpec::Any,
                    destination: AddressSpec::Any,
                    ports: None,
                }],
            },
        );

        let cs = ChangeSet {
            hostname: None,
            ip_domain_name: None,
            ip_name_servers: Vec::new(),
            system: Vec::new(),
            interface_changes: Vec::new(),
            interfaces: HashMap::new(),
            vlan_changes: Vec::new(),
            vlans: HashMap::new(),
            ip_routes: Vec::new(),
            routes: Vec::new(),
            acls: vec![("ACL1".to_string(), ChangeOp::Add)],
            dhcp_pools: Vec::new(),
            acl_configs: acls,
            dhcp_pool_configs: HashMap::new(),
            affects_connectivity: false,
        };

        let batch = generate(&cs);
        assert!(batch.commands.iter().any(|c| c == "ip access-list standard ACL1"));
        assert!(batch.commands.iter().any(|c| c == "permit ip any any"));
    }

    #[test]
    fn generate_system_hostname() {
        let cs = ChangeSet {
            hostname: Some("Switch1".to_string()),
            ip_domain_name: None,
            ip_name_servers: Vec::new(),
            system: vec![ChangeOp::Modify],
            interface_changes: Vec::new(),
            interfaces: HashMap::new(),
            vlan_changes: Vec::new(),
            vlans: HashMap::new(),
            ip_routes: Vec::new(),
            routes: Vec::new(),
            acls: Vec::new(),
            acl_configs: HashMap::new(),
            dhcp_pools: Vec::new(),
            dhcp_pool_configs: HashMap::new(),
            affects_connectivity: false,
        };

        let batch = generate(&cs);
        assert!(batch.commands.iter().any(|c| c == "hostname Switch1"));
    }
}
