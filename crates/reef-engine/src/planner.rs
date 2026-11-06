//! Planner: compares current and desired config models to produce a change set.
//!
//! The change set is transport-independent and serializable for the GUI diff
//! preview.

use crate::model::{
    AclConfig, DhcpPool, InterfaceConfig, StaticRoute, VlanConfig, DeviceConfig,
};
use std::collections::HashMap;

/// Operation for a single config element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeOp {
    Add,
    Modify,
    Remove,
    NoChange,
}

/// The full change set between current and desired device configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeSet {
    // System settings
    pub hostname: Option<String>,
    pub ip_domain_name: Option<String>,
    pub ip_name_servers: Vec<String>,
    pub system: Vec<ChangeOp>,

    // Interfaces
    pub interface_changes: Vec<(String, ChangeOp, Option<crate::model::Ipv4Address>)>,
    /// The full desired interface map (needed by the generator to emit commands).
    pub interfaces: HashMap<String, InterfaceConfig>,

    // VLANs
    pub vlan_changes: Vec<(u16, ChangeOp)>,
    /// The full desired VLAN map.
    pub vlans: HashMap<u16, VlanConfig>,

    // Static IPv4 routes
    pub ip_routes: Vec<ChangeOp>,
    /// The full desired route list (parallel to ip_routes).
    pub routes: Vec<StaticRoute>,

    // ACLs
    pub acls: Vec<(String, ChangeOp)>,
    /// The full desired ACL map.
    pub acl_configs: HashMap<String, AclConfig>,

    // DHCP pools
    pub dhcp_pools: Vec<(String, ChangeOp)>,
    /// The full desired DHCP pool map.
    pub dhcp_pool_configs: HashMap<String, DhcpPool>,

    pub affects_connectivity: bool,
}

impl Default for ChangeSet {
    fn default() -> Self {
        ChangeSet {
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
            acls: Vec::new(),
            acl_configs: HashMap::new(),
            dhcp_pools: Vec::new(),
            dhcp_pool_configs: HashMap::new(),
            affects_connectivity: false,
        }
    }
}

/// Compare current and desired configs, returning a change set.
pub fn plan(current: &DeviceConfig, desired: &DeviceConfig) -> ChangeSet {
    let mut cs = ChangeSet::default();

    // System settings
    cs.hostname = desired.hostname.clone();
    cs.ip_domain_name = desired.ip_domain_name.clone();
    cs.ip_name_servers = desired.ip_name_server.clone();

    if current.hostname != desired.hostname {
        cs.system.push(ChangeOp::Modify);
    }
    if current.ip_domain_name != desired.ip_domain_name {
        cs.system.push(ChangeOp::Modify);
    }
    if current.ip_name_server != desired.ip_name_server {
        cs.system.push(ChangeOp::Modify);
    }

    // Interfaces
    for (name, desired_iface) in &desired.interfaces {
        match current.interfaces.get(name) {
            None => cs
                .interface_changes
                .push((name.clone(), ChangeOp::Add, None)),
            Some(current_iface) if current_iface != desired_iface => cs
                .interface_changes
                .push((name.clone(), ChangeOp::Modify, current_iface.ipv4.clone())),
            Some(_) => cs
                .interface_changes
                .push((name.clone(), ChangeOp::NoChange, None)),
        }
    }
    for name in current.interfaces.keys() {
        if !desired.interfaces.contains_key(name) {
            cs.interface_changes
                .push((name.clone(), ChangeOp::Remove, current.interfaces.get(name).and_then(|i| i.ipv4.clone())));
        }
    }

    // VLANs
    for (id, desired_vlan) in &desired.vlans {
        match current.vlans.get(id) {
            None => cs.vlan_changes.push((*id, ChangeOp::Add)),
            Some(current_vlan) if current_vlan != desired_vlan => {
                cs.vlan_changes.push((*id, ChangeOp::Modify))
            }
            Some(_) => cs.vlan_changes.push((*id, ChangeOp::NoChange)),
        }
    }
    for id in current.vlans.keys() {
        if !desired.vlans.contains_key(id) {
            cs.vlan_changes.push((*id, ChangeOp::Remove));
        }
    }

    // Routes (parallel vectors)
    for (i, desired_route) in desired.ip_routes.iter().enumerate() {
        let found = current.ip_routes.iter().any(|cr| cr == desired_route);
        cs.ip_routes.push(if found {
            ChangeOp::NoChange
        } else {
            ChangeOp::Add
        });
        cs.routes.push(desired_route.clone());
    }

    // ACLs
    for (name, desired_acl) in &desired.acls {
        match current.acls.get(name) {
            None => cs.acls.push((name.clone(), ChangeOp::Add)),
            Some(current_acl) if current_acl != desired_acl => {
                cs.acls.push((name.clone(), ChangeOp::Modify))
            }
            Some(_) => cs.acls.push((name.clone(), ChangeOp::NoChange)),
        }
    }
    for name in current.acls.keys() {
        if !desired.acls.contains_key(name) {
            cs.acls.push((name.clone(), ChangeOp::Remove));
        }
    }

    // DHCP pools
    for (name, desired_pool) in &desired.dhcp_pools {
        match current.dhcp_pools.get(name) {
            None => cs.dhcp_pools.push((name.clone(), ChangeOp::Add)),
            Some(current_pool) if current_pool != desired_pool => {
                cs.dhcp_pools.push((name.clone(), ChangeOp::Modify))
            }
            Some(_) => cs.dhcp_pools.push((name.clone(), ChangeOp::NoChange)),
        }
    }
    for name in current.dhcp_pools.keys() {
        if !desired.dhcp_pools.contains_key(name) {
            cs.dhcp_pools.push((name.clone(), ChangeOp::Remove));
        }
    }

    cs.interfaces = desired.interfaces.clone();
    cs.vlans = desired.vlans.clone();
    cs.acl_configs = desired.acls.clone();
    cs.dhcp_pool_configs = desired.dhcp_pools.clone();

    // affects_connectivity is true if any change could disrupt management
    // access: changing IP/subnet on an interface that currently has an IP,
    // or removing an interface that currently has an IP.
    let mut affects = false;
    for (name, op, _prev_ip) in &cs.interface_changes {
        match op {
            ChangeOp::Modify | ChangeOp::Remove => {
                if let Some(cur_iface) = current.interfaces.get(name) {
                    if cur_iface.ipv4.is_some() {
                        affects = true;
                    }
                }
            }
            ChangeOp::Add => {
                if let Some(des_iface) = desired.interfaces.get(name) {
                    if des_iface.ipv4.is_some() {
                        affects = true;
                    }
                }
            }
            ChangeOp::NoChange => {}
        }
    }
    cs.affects_connectivity = affects;

    cs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeviceConfig;

    #[test]
    fn plan_detects_added_interface() {
        let current = DeviceConfig::default();
        let mut desired = DeviceConfig::default();
        desired.interfaces.insert(
            "GigabitEthernet1/0/1".to_string(),
            InterfaceConfig::default(),
        );
        let cs = plan(&current, &desired);
        assert_eq!(cs.interface_changes.len(), 1);
        assert!(matches!(cs.interface_changes[0].1, ChangeOp::Add));
    }

    #[test]
    fn plan_detects_removed_interface() {
        let mut current = DeviceConfig::default();
        current.interfaces.insert("Loopback0".to_string(), InterfaceConfig::default());
        let desired = DeviceConfig::default();
        let cs = plan(&current, &desired);
        assert!(matches!(cs.interface_changes[0].1, ChangeOp::Remove));
    }

    #[test]
    fn plan_detects_hostname_change() {
        let mut current = DeviceConfig::default();
        current.hostname = Some("Old".to_string());
        let mut desired = DeviceConfig::default();
        desired.hostname = Some("New".to_string());
        let cs = plan(&current, &desired);
        assert_eq!(cs.hostname, Some("New".to_string()));
        assert!(!cs.system.is_empty());
    }
}
