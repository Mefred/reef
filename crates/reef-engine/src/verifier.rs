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
