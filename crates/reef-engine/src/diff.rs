//! Diff rendering: human-readable text representation of a change set.

use crate::model::InterfaceConfig;
use crate::planner::{ChangeOp, ChangeSet};

/// Render a change set as a human-readable diff string.
pub fn render(change_set: &ChangeSet) -> String {
    let mut out = String::new();

    for (name, op) in &change_set.interface_changes {
        match op {
            ChangeOp::Add => {
                out.push_str(&format!("Interface {}:\n  [ADD]    create interface\n", name));
                if let Some(iface) = change_set.interfaces.get(name) {
                    render_interface_fields(iface, &mut out);
                }
            }
            ChangeOp::Modify => {
                out.push_str(&format!("Interface {}:\n  [MODIFY]\n", name));
                if let Some(iface) = change_set.interfaces.get(name) {
                    render_interface_fields(iface, &mut out);
                }
            }
            ChangeOp::Remove => {
                out.push_str(&format!("Interface {}:\n  [REMOVE] delete interface\n", name));
            }
            ChangeOp::NoChange => {}
        }
    }

    if out.is_empty() {
        out.push_str("No changes.\n");
    }

    out
}

fn render_interface_fields(iface: &InterfaceConfig, out: &mut String) {
    if let Some(desc) = &iface.description {
        out.push_str(&format!("  [SET]    description {}\n", desc));
    }
    if let Some(ip) = &iface.ipv4 {
        out.push_str(&format!(
            "  [SET]    ip address {}/{}\n",
            ip.address, ip.prefix_len
        ));
    }
    if !iface.enabled {
        out.push_str("  [SET]    shutdown\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Ipv4Address;
    use std::collections::HashMap;
    use std::net::Ipv4Addr;

    #[test]
    fn render_add_interface() {
        let mut iface = InterfaceConfig::default();
        iface.description = Some("Uplink".to_string());
        iface.ipv4 = Some(Ipv4Address {
            address: Ipv4Addr::new(10, 0, 0, 1),
            prefix_len: 24,
        });
        let mut interfaces = HashMap::new();
        interfaces.insert("GigabitEthernet1/0/1".to_string(), iface);
        let cs = ChangeSet {
            interface_changes: vec![("GigabitEthernet1/0/1".to_string(), ChangeOp::Add)],
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
        let out = render(&cs);
        assert!(out.contains("Interface GigabitEthernet1/0/1:"));
        assert!(out.contains("[ADD]"));
        assert!(out.contains("description Uplink"));
    }

    #[test]
    fn render_no_changes() {
        let cs = ChangeSet::default();
        assert_eq!(render(&cs), "No changes.\n");
    }
}
