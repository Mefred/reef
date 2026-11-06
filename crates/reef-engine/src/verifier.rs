//! Verifier: post-apply check that running-config matches intended state.
//!
//! Re-parses `show running-config` after apply and compares against the
//! desired model.

use crate::model::DeviceConfig;

/// Result of verifying the device state after applying changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerificationResult {
    /// The device model matches the desired state.
    Verified,
    /// The device model differs from desired; mismatches described.
    Drift { mismatches: Vec<String> },
    /// Verification could not be completed (parse failure, etc.).
    Unverifiable { reason: String },
}

impl VerificationResult {
    pub fn is_success(&self) -> bool {
        matches!(self, VerificationResult::Verified)
    }
}

/// Compare current (post-apply) config against desired config.
pub fn verify(current: &DeviceConfig, desired: &DeviceConfig) -> VerificationResult {
    let mut mismatches = Vec::new();

    if current.hostname != desired.hostname {
        mismatches.push(format!(
            "hostname: current={:?} desired={:?}",
            current.hostname, desired.hostname
        ));
    }
    if current.ip_domain_name != desired.ip_domain_name {
        mismatches.push("ip domain-name mismatch".to_string());
    }
    if current.ip_name_server != desired.ip_name_server {
        mismatches.push("ip name-server mismatch".to_string());
    }

    // Compare interfaces by name
    for (name, desired_iface) in &desired.interfaces {
        match current.interfaces.get(name) {
            None => mismatches.push(format!("interface {}: missing", name)),
            Some(current_iface) if current_iface != desired_iface => {
                mismatches.push(format!("interface {}: config differs", name));
            }
            _ => {}
        }
    }
    for name in current.interfaces.keys() {
        if !desired.interfaces.contains_key(name) {
            mismatches.push(format!("interface {}: unexpected on device", name));
        }
    }

    // Compare VLANs by ID
    for (id, desired_vlan) in &desired.vlans {
        match current.vlans.get(id) {
            None => mismatches.push(format!("vlan {}: missing", id)),
            Some(current_vlan) if current_vlan != desired_vlan => {
                mismatches.push(format!("vlan {}: config differs", id));
            }
            _ => {}
        }
    }
    for id in current.vlans.keys() {
        if !desired.vlans.contains_key(id) {
            mismatches.push(format!("vlan {}: unexpected on device", id));
        }
    }

    // Compare static routes
    for (i, desired_route) in desired.ip_routes.iter().enumerate() {
        let found = current.ip_routes.iter().any(|cr| cr == desired_route);
        if !found {
            mismatches.push(format!(
                "ip route {} via {}: missing",
                desired_route.prefix, desired_route.next_hop
            ));
        }
    }
    for current_route in &current.ip_routes {
        if !desired.ip_routes.iter().any(|dr| dr == current_route) {
            mismatches.push(format!(
                "ip route {} via {}: unexpected on device",
                current_route.prefix, current_route.next_hop
            ));
        }
    }

    // Compare ACLs by name
    for (name, desired_acl) in &desired.acls {
        match current.acls.get(name) {
            None => mismatches.push(format!("acl {}: missing", name)),
            Some(current_acl) if current_acl != desired_acl => {
                mismatches.push(format!("acl {}: config differs", name));
            }
            _ => {}
        }
    }
    for name in current.acls.keys() {
        if !desired.acls.contains_key(name) {
            mismatches.push(format!("acl {}: unexpected on device", name));
        }
    }

    // Compare DHCP pools by name
    for (name, desired_pool) in &desired.dhcp_pools {
        match current.dhcp_pools.get(name) {
            None => mismatches.push(format!("dhcp pool {}: missing", name)),
            Some(current_pool) if current_pool != desired_pool => {
                mismatches.push(format!("dhcp pool {}: config differs", name));
            }
            _ => {}
        }
    }
    for name in current.dhcp_pools.keys() {
        if !desired.dhcp_pools.contains_key(name) {
            mismatches.push(format!("dhcp pool {}: unexpected on device", name));
        }
    }

    // Compare NAT
    if current.nat != desired.nat {
        mismatches.push("nat: config differs".to_string());
    }

    if mismatches.is_empty() {
        VerificationResult::Verified
    } else {
        VerificationResult::Drift { mismatches }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeviceConfig, InterfaceConfig};

    #[test]
    fn verify_matches() {
        let current = DeviceConfig::default();
        let desired = DeviceConfig::default();
        assert!(verify(&current, &desired).is_success());
    }

    #[test]
    fn verify_detects_missing_interface() {
        let current = DeviceConfig::default();
        let mut desired = DeviceConfig::default();
        desired.interfaces.insert("Loopback0".to_string(), InterfaceConfig::default());
        let result = verify(&current, &desired);
        assert!(!result.is_success());
    }
}
