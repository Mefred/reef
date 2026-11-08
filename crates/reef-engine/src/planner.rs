//! Planner: compares current and desired config models to produce a change set.
//!
//! The change set is transport-independent and serializable for the GUI diff
//! preview.

use crate::model::{AclConfig, DeviceConfig, DhcpPool, InterfaceConfig, StaticRoute, VlanConfig};
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

    if current.hostname != desired.hostname
        || current.ip_domain_name != desired.ip_domain_name
        || current.ip_name_server != desired.ip_name_server
    {
        // The generator emits the complete desired system block for one
        // operation, so represent any number of field changes as one op.
        cs.system.push(ChangeOp::Modify);
    }

    // Interfaces
    let mut desired_interface_names: Vec<_> = desired.interfaces.keys().collect();
    desired_interface_names.sort();
    for name in desired_interface_names {
        let desired_iface = &desired.interfaces[name];
        match current.interfaces.get(name) {
            None => cs
                .interface_changes
                .push((name.clone(), ChangeOp::Add, None)),
            Some(current_iface) if current_iface != desired_iface => cs.interface_changes.push((
                name.clone(),
                ChangeOp::Modify,
                current_iface.ipv4.clone(),
            )),
            Some(_) => cs
                .interface_changes
                .push((name.clone(), ChangeOp::NoChange, None)),
        }
    }
    let mut current_interface_names: Vec<_> = current.interfaces.keys().collect();
    current_interface_names.sort();
    for name in current_interface_names {
        if !desired.interfaces.contains_key(name) {
            cs.interface_changes.push((
                name.clone(),
                ChangeOp::Remove,
                current.interfaces.get(name).and_then(|i| i.ipv4.clone()),
            ));
        }
    }

    // VLANs
    let mut desired_vlan_ids: Vec<_> = desired.vlans.keys().collect();
    desired_vlan_ids.sort();
    for id in desired_vlan_ids {
        let desired_vlan = &desired.vlans[id];
        match current.vlans.get(id) {
            None => cs.vlan_changes.push((*id, ChangeOp::Add)),
            Some(current_vlan) if current_vlan != desired_vlan => {
                cs.vlan_changes.push((*id, ChangeOp::Modify))
            }
            Some(_) => cs.vlan_changes.push((*id, ChangeOp::NoChange)),
        }
    }
    let mut current_vlan_ids: Vec<_> = current.vlans.keys().collect();
    current_vlan_ids.sort();
    for id in current_vlan_ids {
        if !desired.vlans.contains_key(id) {
            cs.vlan_changes.push((*id, ChangeOp::Remove));
        }
    }

    // Routes (compare by prefix)
    let desired_route_map: HashMap<ipnet::IpNet, StaticRoute> = desired
        .ip_routes
        .iter()
        .map(|r| (r.prefix, r.clone()))
        .collect();
    let current_route_map: HashMap<ipnet::IpNet, StaticRoute> = current
        .ip_routes
        .iter()
        .map(|r| (r.prefix, r.clone()))
        .collect();

    // Process desired routes (adds and modifies)
    for desired_route in &desired.ip_routes {
        match current_route_map.get(&desired_route.prefix) {
            Some(current_route) => {
                if current_route == desired_route {
                    cs.ip_routes.push(ChangeOp::NoChange);
                } else {
                    // Same prefix, different next-hop or admin_distance -> Modify
                    cs.ip_routes.push(ChangeOp::Modify);
                }
            }
            None => cs.ip_routes.push(ChangeOp::Add),
        }
        cs.routes.push(desired_route.clone());
    }

    // Process current routes not in desired (removals)
    for current_route in &current.ip_routes {
        if !desired_route_map.contains_key(&current_route.prefix) {
            cs.ip_routes.push(ChangeOp::Remove);
            cs.routes.push(current_route.clone());
        }
    }

    // ACLs
    let mut desired_acl_names: Vec<_> = desired.acls.keys().collect();
    desired_acl_names.sort();
    for name in desired_acl_names {
        let desired_acl = &desired.acls[name];
        match current.acls.get(name) {
            None => cs.acls.push((name.clone(), ChangeOp::Add)),
            Some(current_acl) if current_acl != desired_acl => {
                cs.acls.push((name.clone(), ChangeOp::Modify))
            }
            Some(_) => cs.acls.push((name.clone(), ChangeOp::NoChange)),
        }
    }
    let mut current_acl_names: Vec<_> = current.acls.keys().collect();
    current_acl_names.sort();
    for name in current_acl_names {
        if !desired.acls.contains_key(name) {
            cs.acls.push((name.clone(), ChangeOp::Remove));
        }
    }

    // DHCP pools
    let mut desired_pool_names: Vec<_> = desired.dhcp_pools.keys().collect();
    desired_pool_names.sort();
    for name in desired_pool_names {
        let desired_pool = &desired.dhcp_pools[name];
        match current.dhcp_pools.get(name) {
            None => cs.dhcp_pools.push((name.clone(), ChangeOp::Add)),
            Some(current_pool) if current_pool != desired_pool => {
                cs.dhcp_pools.push((name.clone(), ChangeOp::Modify))
            }
            Some(_) => cs.dhcp_pools.push((name.clone(), ChangeOp::NoChange)),
        }
    }
    let mut current_pool_names: Vec<_> = current.dhcp_pools.keys().collect();
    current_pool_names.sort();
    for name in current_pool_names {
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
        current
            .interfaces
            .insert("Loopback0".to_string(), InterfaceConfig::default());
        let desired = DeviceConfig::default();
        let cs = plan(&current, &desired);
        assert!(matches!(cs.interface_changes[0].1, ChangeOp::Remove));
    }

    #[test]
    fn plan_orders_hash_map_changes_deterministically() {
        let current = DeviceConfig::default();
        let mut desired = DeviceConfig::default();

        desired.interfaces.insert(
            "GigabitEthernet1/0/2".to_string(),
            InterfaceConfig::default(),
        );
        desired.interfaces.insert(
            "GigabitEthernet1/0/1".to_string(),
            InterfaceConfig::default(),
        );
        desired.vlans.insert(
            20,
            crate::model::VlanConfig {
                id: 20,
                name: "USERS".to_string(),
                enabled: true,
            },
        );
        desired.vlans.insert(
            10,
            crate::model::VlanConfig {
                id: 10,
                name: "SERVERS".to_string(),
                enabled: true,
            },
        );
        desired.acls.insert(
            "Z-ACL".to_string(),
            AclConfig {
                name: "Z-ACL".to_string(),
                acl_type: crate::model::AclType::Standard,
                rules: Vec::new(),
            },
        );
        desired.acls.insert(
            "A-ACL".to_string(),
            AclConfig {
                name: "A-ACL".to_string(),
                acl_type: crate::model::AclType::Standard,
                rules: Vec::new(),
            },
        );
        desired.dhcp_pools.insert(
            "Z-POOL".to_string(),
            DhcpPool {
                name: "Z-POOL".to_string(),
                network: "192.0.2.0/24".parse().unwrap(),
                default_router: None,
                dns_servers: Vec::new(),
                lease_time: None,
            },
        );
        desired.dhcp_pools.insert(
            "A-POOL".to_string(),
            DhcpPool {
                name: "A-POOL".to_string(),
                network: "198.51.100.0/24".parse().unwrap(),
                default_router: None,
                dns_servers: Vec::new(),
                lease_time: None,
            },
        );

        let changes = plan(&current, &desired);
        let interface_names: Vec<_> = changes
            .interface_changes
            .iter()
            .map(|(name, _, _)| name.as_str())
            .collect();
        let vlan_ids: Vec<_> = changes.vlan_changes.iter().map(|(id, _)| *id).collect();

        assert_eq!(
            interface_names,
            vec!["GigabitEthernet1/0/1", "GigabitEthernet1/0/2"]
        );
        assert_eq!(vlan_ids, vec![10, 20]);
        let acl_names: Vec<_> = changes.acls.iter().map(|(name, _)| name.as_str()).collect();
        let pool_names: Vec<_> = changes
            .dhcp_pools
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert_eq!(acl_names, vec!["A-ACL", "Z-ACL"]);
        assert_eq!(pool_names, vec!["A-POOL", "Z-POOL"]);
    }

    #[test]
    fn plan_groups_system_setting_changes_into_one_operation() {
        let mut current = DeviceConfig::default();
        current.hostname = Some("old-router".to_string());
        current.ip_domain_name = Some("old.example".to_string());

        let mut desired = DeviceConfig::default();
        desired.hostname = Some("new-router".to_string());
        desired.ip_domain_name = Some("new.example".to_string());
        desired.ip_name_server = vec!["192.0.2.53".to_string()];

        let changes = plan(&current, &desired);

        assert_eq!(changes.system, vec![ChangeOp::Modify]);
        assert_eq!(changes.hostname.as_deref(), Some("new-router"));
        assert_eq!(changes.ip_domain_name.as_deref(), Some("new.example"));
        assert_eq!(changes.ip_name_servers, vec!["192.0.2.53"]);
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
