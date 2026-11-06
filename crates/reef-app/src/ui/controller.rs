//! Screen controller: bridges Slint UI with reef-engine (planning/generation).
//!
//! The controller owns the desired config state, runs plan → diff, and
//! prepares command batches for apply. Device sessions are optional for
//! UI-only operation (Phase 5); live apply requires a connected session.

use reef_engine::model::{DeviceConfig, InterfaceConfig, InterfaceMode, Ipv4Address};
use reef_engine::planner::{plan, ChangeSet};
use reef_engine::generator::generate;
use reef_engine::verifier::{verify, VerificationResult};
use std::collections::HashMap;
use std::net::Ipv4Addr;

/// Controller for a single device's configuration screens.
pub struct ScreenController {
    current_config: DeviceConfig,
    desired_config: DeviceConfig,
    last_diff: String,
    last_verification: Option<VerificationResult>,
    has_session: bool,
}

impl ScreenController {
    pub fn new(current_config: DeviceConfig) -> Self {
        ScreenController {
            current_config: current_config.clone(),
            desired_config: current_config,
            last_diff: String::new(),
            last_verification: None,
            has_session: false,
        }
    }

    pub fn set_has_session(&mut self, has: bool) {
        self.has_session = has;
    }

    /// Handle an interface edit from the UI: update desired config, re-plan,
    /// re-diff. Returns the new diff text for display.
    pub fn update_interface(
        &mut self,
        name: &str,
        description: Option<String>,
        enabled: bool,
        ip: Option<&str>,
        mask: Option<&str>,
    ) -> String {
        let iface = self
            .desired_config
            .interfaces
            .entry(name.to_string())
            .or_insert(InterfaceConfig {
                name: name.to_string(),
                enabled: true,
                ..Default::default()
            });

        if let Some(d) = description {
            iface.description = Some(d);
        }
        iface.enabled = enabled;

        if let Some(addr) = ip {
            if let (Ok(ip_addr), Ok(mask_addr)) = (
                addr.parse::<Ipv4Addr>(),
                mask.unwrap_or("255.255.255.0").parse::<Ipv4Addr>(),
            ) {
                let prefix_len = mask_to_prefix_len(mask_addr).unwrap_or(24);
                iface.ipv4 = Some(Ipv4Address {
                    address: ip_addr,
                    prefix_len,
                });
            }
        }

        // Re-plan and re-diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);
        self.last_diff.clone()
    }

    /// Get the current diff text.
    pub fn diff(&self) -> &str {
        &self.last_diff
    }

    /// Get the list of interface names.
    pub fn interface_names(&self) -> Vec<String> {
        self.desired_config.interfaces.keys().cloned().collect()
    }

    /// Get interface IP for display.
    pub fn interface_ip(&self, name: &str) -> Option<String> {
        self.desired_config
            .interfaces
            .get(name)
            .and_then(|i| i.ipv4.as_ref())
            .map(|ip| format!("{}/{}", ip.address, ip.prefix_len))
    }

    /// Get interface enabled state.
    pub fn interface_enabled(&self, name: &str) -> bool {
        self.desired_config
            .interfaces
            .get(name)
            .map(|i| i.enabled)
            .unwrap_or(false)
    }

    /// Get interface description.
    pub fn interface_description(&self, name: &str) -> Option<String> {
        self.desired_config.interfaces.get(name).and_then(|i| i.description.clone())
    }

    /// Generate the command batch for the current desired config.
    pub fn generate_batch(&self) -> reef_engine::generator::CommandBatch {
        let change_set = plan(&self.current_config, &self.desired_config);
        generate(&change_set)
    }

    /// Returns whether a live session is available for apply.
    pub fn has_session(&self) -> bool {
        self.has_session
    }
}

fn mask_to_prefix_len(mask: Ipv4Addr) -> Option<u8> {
    let bits = u32::from_be_bytes(mask.octets());
    if bits == 0 {
        return Some(0);
    }
    let trailing = bits.trailing_zeros();
    if bits == (!0_u32 << trailing) {
        Some((32 - trailing) as u8)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_engine::model::DeviceConfig;

    #[test]
    fn controller_update_interface() {
        let cfg = DeviceConfig::default();
        let mut ctrl = ScreenController::new(cfg);
        let diff = ctrl.update_interface("GigabitEthernet1/0/1", Some("Uplink".into()), true, Some("10.0.0.1"), Some("255.255.255.0"));
        assert!(diff.contains("GigabitEthernet1/0/1"));
        assert!(diff.contains("[ADD]"));
    }

    #[test]
    fn controller_interface_ip() {
        let cfg = DeviceConfig::default();
        let mut ctrl = ScreenController::new(cfg);
        ctrl.update_interface("GigabitEthernet1/0/1", None, true, Some("10.0.0.1"), Some("255.255.255.0"));
        assert_eq!(ctrl.interface_ip("GigabitEthernet1/0/1"), Some("10.0.0.1/24".to_string()));
        assert!(ctrl.interface_enabled("GigabitEthernet1/0/1"));
        assert_eq!(ctrl.interface_names(), vec!["GigabitEthernet1/0/1".to_string()]);
    }
}
