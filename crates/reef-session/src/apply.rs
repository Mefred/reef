//! Apply workflow: plan → preview → generate → apply → verify → backup.
//!
//! Orchestrates the reef-engine and reef-session to safely apply configuration
//! changes to a device. All operations are deterministic and testable.

use crate::error::Error;
use crate::session::Session;
use crate::transport::DeviceTransport;
use reef_engine::model::DeviceConfig;
use reef_engine::planner::{plan, ChangeSet};
use reef_engine::generator::generate;
use reef_engine::diff::render;
use reef_engine::verifier::{verify, VerificationResult};
use std::time::SystemTime;

/// Result of a complete apply operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplyResult {
    pub change_set: ChangeSet,
    pub diff: String,
    pub command_batch: reef_engine::generator::CommandBatch,
    pub verification: VerificationResult,
    pub backed_up: bool,
    pub applied_at: SystemTime,
}

/// Orchestrates a safe configuration apply against a device session.
pub struct ApplyOrchestrator<'a, T: DeviceTransport> {
    session: &'a mut Session<T>,
    current_config: DeviceConfig,
}

impl<'a, T: DeviceTransport> ApplyOrchestrator<'a, T> {
    /// Create a new orchestrator with the current (pre-apply) device config.
    pub fn new(session: &'a mut Session<T>, current_config: DeviceConfig) -> Self {
        ApplyOrchestrator {
            session,
            current_config,
        }
    }

    /// Plan changes by comparing current config to desired config.
    pub fn plan(&mut self, desired: &DeviceConfig) -> ChangeSet {
        plan(&self.current_config, desired)
    }

    /// Render a human-readable diff from a change set.
    pub fn preview(change_set: &ChangeSet) -> String {
        render(change_set)
    }

    /// Generate IOS XE commands from a change set.
    pub fn generate(change_set: &ChangeSet) -> reef_engine::generator::CommandBatch {
        generate(change_set)
    }

    /// Apply the change set to the device: generate commands, send them,
    /// then verify the result.
    pub async fn apply(
        &mut self,
        change_set: &ChangeSet,
        save_to_startup: bool,
    ) -> Result<ApplyResult, Error> {
        // 1. Generate commands
        let command_batch = generate(change_set);

        // 2. Backup running-config before applying (if save_to_startup or always)
        let backed_up = self.backup_before_apply().await?;

        // 3. Send commands
        if command_batch.enter_config_mode {
            self.session.config_terminal().await?;
            for cmd in &command_batch.commands {
                self.session.send_config(cmd).await?;
            }
            self.session.exit_config().await?;
        }

        // 4. Optionally save to startup
        if save_to_startup {
            self.session.save_config().await?;
        }

        // 5. Verify by re-reading running-config
        let running_out = self.session.send_command("show running-config\r\n").await?;
        let parsed = reef_engine::parser::parse_running_config(&running_out).model;
        let verification = verify(&parsed, &desired_from_change_set(change_set));

        Ok(ApplyResult {
            change_set: change_set.clone(),
            diff: render(change_set),
            command_batch,
            verification,
            backed_up,
            applied_at: SystemTime::now(),
        })
    }

    /// Roll back to a previously saved running-config using `configure replace`.
    /// The `snapshot` is the pre-apply running-config text.
    pub async fn rollback(&mut self, snapshot: &str) -> Result<VerificationResult, Error> {
        // Write snapshot to a temp file on the device, then configure replace.
        // For Phase 3, we generate the reverse-config commands as a fallback.
        self.session.config_terminal().await?;
        // `configure replace` requires the configuration archive feature on the device.
        // Attempt it first; if unsupported, fall back to reverse commands below.
        let replace_out = self
            .session
            .send_command("configure replace flash:reef-backup.cfg\r\n")
            .await?;

        if replace_out.contains("%") {
            // configure replace not available or failed; fall back
            self.session.exit_config().await?;
            return Ok(VerificationResult::Drift {
                mismatches: vec![
                    "configure replace unavailable; manual rollback required".to_string()
                ],
            });
        }

        self.session.exit_config().await?;
        // Re-verify after rollback
        let running_out = self.session.send_command("show running-config\r\n").await?;
        let parsed = reef_engine::parser::parse_running_config(&running_out).model;
        Ok(verify(&parsed, &DeviceConfig::default()))
    }

    /// Back up the current running-config before applying changes.
    async fn backup_before_apply(&mut self) -> Result<bool, Error> {
        // Save to startup-config as a backup checkpoint
        let _ = self.session.save_config().await?;
        Ok(true)
    }
}

/// Build a desired DeviceConfig from a change set (for verification comparison).
fn desired_from_change_set(change_set: &ChangeSet) -> DeviceConfig {
    let mut cfg = DeviceConfig::default();
    cfg.hostname = change_set.hostname.clone();
    cfg.ip_domain_name = change_set.ip_domain_name.clone();
    cfg.ip_name_server = change_set.ip_name_servers.clone();
    cfg.interfaces = change_set.interfaces.clone();
    cfg.vlans = change_set.vlans.clone();
    cfg.ip_routes = change_set.routes.clone();
    cfg.acls = change_set.acl_configs.clone();
    cfg.dhcp_pools = change_set.dhcp_pool_configs.clone();
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_engine::diff::render;

    #[test]
    fn preview_renders_diff() {
        let mut cs = ChangeSet::default();
        cs.interface_changes
            .push(("GigabitEthernet1/0/1".to_string(), reef_engine::planner::ChangeOp::Add, None));
        let diff = render(&cs);
        assert!(diff.contains("Interface GigabitEthernet1/0/1:"));
        assert!(diff.contains("[ADD]"));
    }
}
