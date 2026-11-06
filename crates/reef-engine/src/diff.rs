//! Diff rendering: human-readable text representation of a change set.

use crate::model::InterfaceConfig;
use crate::planner::{ChangeOp, ChangeSet};

/// Render a change set as a human-readable diff string.
pub fn render(change_set: &ChangeSet) -> String {
    let mut out = String::new();

    // System settings
    let has_system_changes = change_set.system.iter().any(|op| *op != ChangeOp::NoChange);
    if has_system_changes {
        out.push_str("System:\n");
        if let Some(h) = &change_set.hostname {
            out.push_str(&format!("  [SET]    hostname {}\n", h));
        }
        if let Some(d) = &change_set.ip_domain_name {
            out.push_str(&format!("  [SET]    ip domain-name {}\n", d));
        }
        for ns in &change_set.ip_name_servers {
            out.push_str(&format!("  [SET]    ip name-server {}\n", ns));
        }
    }

    // Interfaces
    for (name, op, _prev_ip) in &change_set.interface_changes {
        match op {
            ChangeOp::Add => {
                out.push_str(&format!(
                    "Interface {}:\n  [ADD]    create interface\n",
                    name
                ));
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
                out.push_str(&format!(
                    "Interface {}:\n  [REMOVE] delete interface\n",
                    name
                ));
            }
            ChangeOp::NoChange => {}
        }
    }

    // VLANs
    for (id, op) in &change_set.vlan_changes {
        match op {
            ChangeOp::Add => {
                out.push_str(&format!("VLAN {}:\n  [ADD]    create vlan\n", id));
                if let Some(vlan) = change_set.vlans.get(id) {
                    render_vlan_fields(vlan, &mut out);
                }
            }
            ChangeOp::Modify => {
                out.push_str(&format!("VLAN {}:\n  [MODIFY]\n", id));
                if let Some(vlan) = change_set.vlans.get(id) {
                    render_vlan_fields(vlan, &mut out);
                }
            }
            ChangeOp::Remove => {
                out.push_str(&format!("VLAN {}:\n  [REMOVE] delete vlan\n", id));
            }
            ChangeOp::NoChange => {}
        }
    }

    // Static routes
    for (i, op) in change_set.ip_routes.iter().enumerate() {
        match op {
            ChangeOp::Add => {
                if let Some(route) = change_set.routes.get(i) {
                    out.push_str(&format!(
                        "Route {} via {}:\n  [ADD]    add static route\n",
                        route.prefix, route.next_hop
                    ));
                }
            }
            ChangeOp::Modify => {
                if let Some(route) = change_set.routes.get(i) {
                    out.push_str(&format!(
                        "Route {} via {}:\n  [MODIFY]\n",
                        route.prefix, route.next_hop
                    ));
                }
            }
            ChangeOp::Remove => {
                if let Some(route) = change_set.routes.get(i) {
                    out.push_str(&format!(
                        "Route {} via {}:\n  [REMOVE] delete static route\n",
                        route.prefix, route.next_hop
                    ));
                }
            }
            ChangeOp::NoChange => {}
        }
    }

    // ACLs
    for (name, op) in &change_set.acls {
        match op {
            ChangeOp::Add => {
                out.push_str(&format!("ACL {}:\n  [ADD]    create access-list\n", name));
            }
            ChangeOp::Modify => {
                out.push_str(&format!("ACL {}:\n  [MODIFY]\n", name));
            }
            ChangeOp::Remove => {
                out.push_str(&format!("ACL {}:\n  [REMOVE] delete access-list\n", name));
            }
            ChangeOp::NoChange => {}
        }
    }

    // DHCP pools
    for (name, op) in &change_set.dhcp_pools {
        match op {
            ChangeOp::Add => {
                out.push_str(&format!("DHCP pool {}:\n  [ADD]    create pool\n", name));
            }
            ChangeOp::Modify => {
                out.push_str(&format!("DHCP pool {}:\n  [MODIFY]\n", name));
            }
            ChangeOp::Remove => {
                out.push_str(&format!("DHCP pool {}:\n  [REMOVE] delete pool\n", name));
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
    out.push_str(&format!("  [SET]    mode {:?}\n", iface.mode));
}

fn render_vlan_fields(vlan: &crate::model::VlanConfig, out: &mut String) {
    if !vlan.name.is_empty() {
        out.push_str(&format!("  [SET]    name {}\n", vlan.name));
    }
    if !vlan.enabled {
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
            hostname: None,
            ip_domain_name: None,
            ip_name_servers: Vec::new(),
            system: Vec::new(),
            interface_changes: vec![("GigabitEthernet1/0/1".to_string(), ChangeOp::Add, None)],
            interfaces,
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

    #[test]
    fn render_system_settings() {
        let cs = ChangeSet {
            hostname: Some("Switch1".to_string()),
            ip_domain_name: Some("example.com".to_string()),
            ip_name_servers: vec!["8.8.8.8".to_string()],
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
        let out = render(&cs);
        assert!(out.contains("System:"));
        assert!(out.contains("hostname Switch1"));
        assert!(out.contains("ip domain-name example.com"));
        assert!(out.contains("ip name-server 8.8.8.8"));
    }
}
