//! Validator: checks configuration plans for conflicts and invalid references.
//!
//! Returns a list of validation problems. Warnings are non-fatal; errors
//! should block the apply.

use crate::model::{DeviceConfig, InterfaceConfig, InterfaceMode};
use crate::planner::{ChangeOp, ChangeSet};

/// A validation problem with severity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationProblem {
    pub severity: Severity,
    pub element: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// Validate a change set against the current device config.
pub fn validate(change_set: &ChangeSet, current: &DeviceConfig) -> Vec<ValidationProblem> {
    let mut problems = Vec::new();

    for (name, op, _prev_ip) in &change_set.interface_changes {
        if *op == ChangeOp::Remove {
            problems.push(ValidationProblem {
                severity: Severity::Warning,
                element: name.clone(),
                message: "removing an interface may disrupt traffic".to_string(),
            });
            continue;
        }
        if let Some(iface) = change_set.interfaces.get(name) {
            validate_interface(iface, &mut problems, name);
        }
    }

    problems
}

fn validate_interface(iface: &InterfaceConfig, problems: &mut Vec<ValidationProblem>, name: &str) {
    match iface.mode {
        InterfaceMode::Access => {
            if iface.ipv4.is_some() || iface.ipv6.is_some() {
                problems.push(ValidationProblem {
                    severity: Severity::Error,
                    element: name.to_string(),
                    message: "access (L2) interface should not have an IP address".to_string(),
                });
            }
        }
        InterfaceMode::Trunk => {
            if iface.native_vlan.is_none() {
                problems.push(ValidationProblem {
                    severity: Severity::Warning,
                    element: name.to_string(),
                    message: "trunk interface without native VLAN defaults to VLAN 1".to_string(),
                });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::DeviceConfig;

    #[test]
    fn access_interface_with_ip_is_error() {
        let mut iface = InterfaceConfig::default();
        iface.mode = InterfaceMode::Access;
        iface.ipv4 = Some(crate::model::Ipv4Address {
            address: std::net::Ipv4Addr::new(10, 0, 0, 1),
            prefix_len: 24,
        });
        let mut cs = ChangeSet::default();
        cs.interfaces.insert("GigabitEthernet1/0/1".to_string(), iface);
        cs.interface_changes
            .push(("GigabitEthernet1/0/1".to_string(), ChangeOp::Modify, None));

        let problems = validate(&cs, &DeviceConfig::default());
        assert_eq!(problems.len(), 1);
        assert!(matches!(problems[0].severity, Severity::Error));
    }
}
