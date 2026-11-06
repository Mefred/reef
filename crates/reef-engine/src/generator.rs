//! Command generator: structured model + change set -> IOS XE CLI commands.
//!
//! Pure function: takes a `ChangeSet` and produces an ordered `CommandBatch`.
//! No device or GUI dependency.

use crate::planner::{ChangeOp, ChangeSet};
use crate::model::InterfaceConfig;

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
    let mut commands = Vec::new();

    for (name, op) in &change_set.interface_changes {
        match op {
            crate::planner::ChangeOp::Add | crate::planner::ChangeOp::Modify => {
                commands.push(format!("interface {}", name));
                if let Some(iface) = change_set.interfaces.get(name) {
                    emit_interface_commands(iface, &mut commands);
                }
                commands.push("exit".to_string());
            }
            crate::planner::ChangeOp::Remove => {
                commands.push(format!("no interface {}", name));
            }
            crate::planner::ChangeOp::NoChange => {}
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
    if !iface.enabled {
        commands.push("shutdown".to_string());
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
    use crate::model::{Ipv4Address, InterfaceMode};
    use std::net::Ipv4Addr;
    use std::collections::HashMap;

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
            interface_changes: vec![("GigabitEthernet1/0/1".to_string(), ChangeOp::Modify)],
            vlan_changes: Vec::new(),
            ip_routes: Vec::new(),
            ipv6_routes: Vec::new(),
            acls: Vec::new(),
            dhcp_pools: Vec::new(),
            nat: None,
            system: Vec::new(),
            interfaces,
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
}
