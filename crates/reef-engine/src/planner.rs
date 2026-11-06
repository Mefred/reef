//! Planner: compares current and desired config models to produce a change set.
//!
//! The change set is transport-independent and serializable for the GUI diff
//! preview.

use crate::model::{InterfaceConfig, VlanConfig, StaticRoute, AclConfig, DeviceConfig};
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
    pub interface_changes: Vec<(String, ChangeOp)>,
    pub vlan_changes: Vec<(u16, ChangeOp)>,
    pub ip_routes: Vec<ChangeOp>,
    pub ipv6_routes: Vec<ChangeOp>,
    pub acls: Vec<(String, ChangeOp)>,
    pub dhcp_pools: Vec<(String, ChangeOp)>,
    pub nat: Option<ChangeOp>,
    pub system: Vec<ChangeOp>,
    /// The full desired interface map (needed by the generator to emit commands).
    pub interfaces: HashMap<String, InterfaceConfig>,
    pub affects_connectivity: bool,
}

impl Default for ChangeSet {
    fn default() -> Self {
        ChangeSet {
            interface_changes: Vec::new(),
            vlan_changes: Vec::new(),
            ip_routes: Vec::new(),
            ipv6_routes: Vec::new(),
            acls: Vec::new(),
            dhcp_pools: Vec::new(),
            nat: None,
            system: Vec::new(),
            interfaces: HashMap::new(),
            affects_connectivity: false,
        }
    }
}

/// Compare current and desired configs, returning a change set.
pub fn plan(current: &DeviceConfig, desired: &DeviceConfig) -> ChangeSet {
    let mut cs = ChangeSet::default();

    // Interfaces
    for (name, desired_iface) in &desired.interfaces {
        match current.interfaces.get(name) {
            None => cs
                .interface_changes
                .push((name.clone(), ChangeOp::Add)),
            Some(current_iface) if current_iface != desired_iface => cs
                .interface_changes
                .push((name.clone(), ChangeOp::Modify)),
            Some(_) => cs
                .interface_changes
                .push((name.clone(), ChangeOp::NoChange)),
        }
    }
    for name in current.interfaces.keys() {
        if !desired.interfaces.contains_key(name) {
            cs.interface_changes
                .push((name.clone(), ChangeOp::Remove));
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

    cs.interfaces = desired.interfaces.clone();
    cs.affects_connectivity = current.hostname != desired.hostname; // placeholder heuristic

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
}
