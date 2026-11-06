//! Screen controller: bridges Slint UI with reef-engine (planning/generation)
//! and reef-session (device sessions).
//!
//! The controller owns the desired config state, runs plan → diff, and
//! coordinates live device operations through the session manager.

use reef_engine::model::{DeviceConfig, InterfaceConfig, InterfaceMode, Ipv4Address};
use reef_engine::planner::{plan, ChangeOp, ChangeSet};
use reef_engine::generator::generate;
use reef_engine::verifier::{verify, VerificationResult};
use reef_session::{ConnectionConfig, HostKeyPolicy, SessionManager, TelnetTransport, TransportType};
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::rc::Rc;

/// Controller for a single device's configuration screens.
pub struct ScreenController {
    session_manager: Option<Rc<RefCell<SessionManager>>>,
    device_id: Option<String>,
    current_config: DeviceConfig,
    desired_config: DeviceConfig,
    last_diff: String,
    last_verification: Option<VerificationResult>,
    has_session: bool,
    /// Currently selected interface name (for form field updates).
    selected_interface: Option<String>,
    /// Callback to populate Slint interface form fields from Rust.
    set_interface_form_cb: Option<Box<dyn Fn(String, String, bool, String, String)>>,
}

impl ScreenController {
    pub fn new(current_config: DeviceConfig) -> Self {
        ScreenController {
            session_manager: None,
            device_id: None,
            current_config: current_config.clone(),
            desired_config: current_config,
            last_diff: String::new(),
            last_verification: None,
            has_session: false,
            selected_interface: None,
            set_interface_form_cb: None,
        }
    }

    /// Attach a session manager and device ID for live operations.
    pub fn bind_session(
        &mut self,
        session_manager: Rc<RefCell<SessionManager>>,
        device_id: String,
    ) {
        self.session_manager = Some(session_manager);
        self.device_id = Some(device_id);
        self.has_session = true;
    }

    /// Clear the session binding.
    pub fn unbind_session(&mut self) {
        self.session_manager = None;
        self.device_id = None;
        self.has_session = false;
    }

    pub fn set_has_session(&mut self, has: bool) {
        self.has_session = has;
    }

    /// Get the device ID.
    pub fn device_id(&self) -> Option<String> {
        self.device_id.clone()
    }

    /// Register a callback to populate Slint interface form fields from Rust.
    pub fn set_interface_form_callback<F: Fn(String, String, bool, String, String) + 'static>(
        &mut self,
        cb: F,
    ) {
        self.set_interface_form_cb = Some(Box::new(cb));
    }

    /// Notify the UI to populate the interface form fields.
    fn notify_interface_form(&self, name: &str, desc: &str, enabled: bool, ip: &str, mask: &str) {
        if let Some(cb) = &self.set_interface_form_cb {
            cb(name.to_string(), desc.to_string(), enabled, ip.to_string(), mask.to_string());
        }
    }

    /// Connect to a device using the given configuration.
    pub async fn connect_device(&mut self, config: ConnectionConfig) -> Result<String, reef_session::Error> {
        let sm = self.session_manager.as_ref().ok_or_else(|| {
            reef_session::Error::Transport("no session manager".to_string())
        })?;
        let device_id = self.device_id.as_deref().unwrap_or("device").to_string();

        sm.borrow_mut().register(&device_id, TelnetTransport::new(config.clone()));

        sm.borrow_mut().connect(&device_id, config).await?;

        // Identify the device
        let identity = sm.borrow_mut().identify(&device_id).await?;
        let hostname = identity.hostname.clone();
        self.current_config.hostname = Some(hostname.clone());

        // Fetch running-config
        let running_out = sm.borrow_mut().send_command(&device_id, "show running-config\r\n").await?;
        let parsed = reef_engine::parser::parse_running_config(&running_out).model;

        // Merge parsed config into current_config (preserve identity fields)
        self.current_config = parsed;
        self.current_config.hostname = Some(hostname);

        // Reset desired config to current
        self.desired_config = self.current_config.clone();

        self.last_diff = String::new();
        self.has_session = true;

        Ok(format!("Connected to {} ({})", identity.hostname, identity.iosxe_version))
    }

    /// Disconnect from the device.
    pub async fn disconnect_device(&mut self) -> Result<(), reef_session::Error> {
        if let Some(sm) = &self.session_manager {
            if let Some(device_id) = &self.device_id {
                sm.borrow_mut().disconnect(device_id).await?;
            }
        }
        self.has_session = false;
        Ok(())
    }

    /// Refresh device config from the live device.
    pub async fn refresh_config(&mut self) -> Result<String, reef_session::Error> {
        let sm = self.session_manager.as_ref().ok_or_else(|| {
            reef_session::Error::Transport("no session manager".to_string())
        })?;
        let device_id = self.device_id.as_deref().ok_or_else(|| {
            reef_session::Error::Transport("no device id".to_string())
        })?;

        let running_out = sm.borrow_mut().send_command(device_id, "show running-config\r\n").await?;
        let parsed = reef_engine::parser::parse_running_config(&running_out).model;
        self.current_config = parsed;

        // Re-plan diff
        let change_set = plan(&self.current_config, &self.desired_config);
        self.last_diff = reef_engine::diff::render(&change_set);

        Ok(self.last_diff.clone())
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

    /// Whether the current desired config would be disruptive (e.g. changing
    /// an interface that has an IP address — potential management-interface
    /// impact).
    pub fn is_disruptive(&self) -> bool {
        self.desired_config.interfaces.values().any(|i| i.ipv4.is_some())
            || self.current_config.interfaces.values().any(|i| i.ipv4.is_some())
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

    /// Get interface subnet mask for display.
    pub fn interface_mask(&self, name: &str) -> Option<String> {
        self.desired_config
            .interfaces
            .get(name)
            .and_then(|i| i.ipv4.as_ref())
            .map(|ip| {
                let bits: u32 = if ip.prefix_len == 0 {
                    0
                } else {
                    !((1u32 << (32 - ip.prefix_len)) - 1)
                };
                std::net::Ipv4Addr::from(bits.to_be_bytes()).to_string()
            })
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

    /// Apply the planned changes to the live device.
    pub async fn apply_changes(&mut self) -> Result<String, reef_session::Error> {
        let sm = self.session_manager.as_ref().ok_or_else(|| {
            reef_session::Error::Transport("no session manager".to_string())
        })?;
        let device_id = self.device_id.as_deref().ok_or_else(|| {
            reef_session::Error::Transport("no device id".to_string())
        })?;

        let change_set = plan(&self.current_config, &self.desired_config);

        // Check for disruptive changes
        let is_disruptive = change_set.affects_connectivity;

        // Generate commands
        let batch = generate(&change_set);

        if batch.commands.is_empty() {
            return Ok("No changes to apply.".to_string());
        }

        // Backup running-config before applying
        {
            let sm = &*sm.borrow();
            let _ = sm.save_config(device_id).await;

            // Send commands
            sm.config_terminal(device_id).await?;
            for cmd in &batch.commands {
                sm.send_config(device_id, cmd).await?;
            }
            sm.exit_config(device_id).await?;

            // Save to startup-config if disruptive
            if is_disruptive || change_set.affects_connectivity {
                sm.save_config(device_id).await?;
            }

            // Verify by re-reading running-config
            let running_out = sm.send_command(device_id, "show running-config\r\n").await?;
            let parsed = reef_engine::parser::parse_running_config(&running_out).model;
            let result = verify(&parsed, &self.desired_config);
            self.last_verification = Some(result.clone());

            if result.is_success() {
                self.current_config = self.desired_config.clone();
                self.last_diff = String::new();
                Ok(format!("Apply successful. Verification: verified."))
            } else {
                Err(reef_session::Error::Transport(format!(
                    "Apply failed verification: {:?}", result
                )))
            }
        }
    }

    /// Roll back the last applied changes.
    pub async fn rollback_changes(&mut self) -> Result<String, reef_session::Error> {
        let sm = self.session_manager.as_ref().ok_or_else(|| {
            reef_session::Error::Transport("no session manager".to_string())
        })?;
        let device_id = self.device_id.as_deref().ok_or_else(|| {
            reef_session::Error::Transport("no device id".to_string())
        })?;

        // Rollback: restore the current_config (pre-apply state) by generating
        // reverse commands from the current desired back to current.
        let reverse_cs = plan(&self.desired_config, &self.current_config);
        let reverse_batch = generate(&reverse_cs);

        if reverse_batch.commands.is_empty() {
            return Ok("Nothing to roll back.".to_string());
        }

        {
            let sm = &*sm.borrow();
            sm.config_terminal(device_id).await?;
            for cmd in &reverse_batch.commands {
                sm.send_config(device_id, cmd).await?;
            }
            sm.exit_config(device_id).await?;
            sm.save_config(device_id).await?;

            // Verify rollback
            let running_out = sm.send_command(device_id, "show running-config\r\n").await?;
            let parsed = reef_engine::parser::parse_running_config(&running_out).model;
            let result = verify(&parsed, &self.current_config);

            if result.is_success() {
                self.desired_config = self.current_config.clone();
                self.last_diff = String::new();
                Ok("Rollback successful. Configuration restored.".to_string())
            } else {
                Err(reef_session::Error::Transport(format!(
                    "Rollback verification failed: {:?}", result
                )))
            }
        }
    }

    /// Save running-config to startup-config on the device.
    pub async fn save_config(&mut self) -> Result<(), reef_session::Error> {
        let sm = self.session_manager.as_ref().ok_or_else(|| {
            reef_session::Error::Transport("no session manager".to_string())
        })?;
        let device_id = self.device_id.as_deref().ok_or_else(|| {
            reef_session::Error::Transport("no device id".to_string())
        })?;
        {
            let sm = &*sm.borrow();
            sm.save_config(device_id).await
        }
    }

    /// Get a reference to the current (pre-change) config.
    pub fn current_config(&self) -> &DeviceConfig {
        &self.current_config
    }

    /// Get a reference to the desired config.
    pub fn desired_config(&self) -> &DeviceConfig {
        &self.desired_config
    }

    /// Get a mutable reference to the desired config.
    pub fn desired_config_mut(&mut self) -> &mut DeviceConfig {
        &mut self.desired_config
    }

    /// Set the currently selected interface name.
    pub fn set_selected_interface(&mut self, name: String) {
        self.selected_interface = Some(name);
    }

    /// Get the currently selected interface name.
    pub fn selected_interface_name(&self) -> Option<String> {
        self.selected_interface.clone()
    }

    /// Returns whether a live session is available for apply.
    pub fn has_session(&self) -> bool {
        self.has_session
    }

    /// Get the last verification result.
    pub fn last_verification(&self) -> Option<&VerificationResult> {
        self.last_verification.as_ref()
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
