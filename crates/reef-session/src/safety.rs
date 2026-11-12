//! Mandatory startup-configuration write protection.
//!
//! Reef must never save, copy, or otherwise write a running configuration to
//! a device's startup configuration. This module classifies every command
//! that Reef sends through its managed command paths before the command
//! reaches a transport.
//!
//! Normal running-config edits (hostname, VLANs, interfaces, ACLs, routing)
//! and `show` commands are unaffected. The user can still use an out-of-band
//! terminal that is not controlled by Reef; that limitation is disclosed in
//! the packaged documentation.

use serde::Serialize;

/// A blocked startup-config write attempt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SafetyViolation {
    /// Short stable identifier for the matched rule.
    pub rule: &'static str,
    /// Human-readable explanation shown in the UI and written to diagnostics.
    pub reason: &'static str,
}

impl SafetyViolation {
    pub fn error(self) -> crate::error::Error {
        crate::error::Error::StartupConfigWriteBlocked {
            rule: self.rule.to_string(),
        }
    }
}

/// Check a command against the startup-write policy.
///
/// `Ok(())` means the command is allowed. `Err` means Reef must refuse to send
/// it and must log the attempt.
pub fn check_command(command: &str) -> Result<(), SafetyViolation> {
    // A managed command must be one line, but defend in depth: classify every
    // segment so an injected newline or semicolon cannot hide a write.
    if command
        .chars()
        .any(|character| matches!(character, '\r' | '\n' | ';'))
    {
        for segment in command.split(['\r', '\n', ';']) {
            let segment = segment.trim();
            if !segment.is_empty() {
                check_command(segment)?;
            }
        }
        return Ok(());
    }

    let normalized = normalize(command);
    if normalized.is_empty() {
        return Ok(());
    }
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    if tokens.is_empty() {
        return Ok(());
    }

    // `write`/`wr` family.
    match tokens[0] {
        "write" | "wr" => {
            if tokens.len() == 1 {
                return Err(SafetyViolation {
                    rule: "bare-write",
                    reason:
                        "a bare 'write' command saves the running configuration to startup-config",
                });
            }
            let target = tokens[1];
            if is_startup(target)
                || target.starts_with("mem")
                || target == "erase"
                || target.starts_with("start")
            {
                return Err(SafetyViolation {
                    rule: "write-memory",
                    reason: "'write memory' and its equivalents persist the running configuration",
                });
            }
            // `write terminal`/`write network` display configuration and are
            // not startup writes.
        }
        "copy" => {
            if tokens.len() == 1 {
                return Ok(());
            }
            let source = tokens[1];
            let destination = tokens.get(2).copied();
            if let Some(destination) = destination {
                if is_startup(destination) {
                    return Err(SafetyViolation {
                        rule: "copy-to-startup",
                        reason:
                            "copying any file to startup-config writes persistent startup state",
                    });
                }
            } else if is_running(source) {
                return Err(SafetyViolation {
                    rule: "ambiguous-copy",
                    reason: "copying running-config without an explicit destination can prompt for startup-config",
                });
            }
        }
        "erase" => {
            if tokens.len() == 1
                || tokens[1..]
                    .iter()
                    .any(|token| is_startup(token) || token.starts_with("nvram"))
            {
                return Err(SafetyViolation {
                    rule: "erase-startup",
                    reason: "erasing startup-config or NVRAM modifies persistent startup state",
                });
            }
        }
        "delete" => {
            if tokens[1..]
                .iter()
                .any(|token| is_startup(token) || token.starts_with("nvram"))
            {
                return Err(SafetyViolation {
                    rule: "delete-startup",
                    reason: "deleting startup-config or NVRAM modifies persistent startup state",
                });
            }
        }
        _ => {}
    }

    Ok(())
}

/// Canonical lowercase command with collapsed whitespace and `do` prefixes
/// removed. Newlines cannot appear in a managed command, but are collapsed
/// defensively so an injected multi-line value cannot hide a second command.
fn normalize(command: &str) -> String {
    let mut normalized = command
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character == '\r' || character == '\n' || character == '\t' {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    normalized = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let trimmed = normalized.trim_start();
        if let Some(rest) = trimmed.strip_prefix("do ") {
            normalized = rest.trim_start().to_string();
        } else {
            normalized = trimmed.to_string();
            break;
        }
    }
    normalized
}

fn is_running(token: &str) -> bool {
    matches!(
        token,
        "r" | "run"
            | "runn"
            | "runni"
            | "runnin"
            | "running"
            | "running-config"
            | "sys"
            | "system"
            | "system:"
            | "system:running-config"
    )
}

fn is_startup(token: &str) -> bool {
    matches!(
        token,
        "s" | "st"
            | "sta"
            | "star"
            | "start"
            | "startup"
            | "startup-config"
            | "startup:"
            | "nvram"
            | "nvram:"
            | "nvram:startup-config"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(command: &str) -> &'static str {
        check_command(command)
            .expect_err(&format!("{command:?} should be blocked"))
            .rule
    }

    #[test]
    fn blocks_all_running_to_startup_forms() {
        for command in [
            "write memory",
            "write mem",
            "wr mem",
            "write",
            "wr",
            "write startup-config",
            "copy running-config startup-config",
            "copy run start",
            "copy r s",
            "copy running-config nvram:",
            "copy run startup-config",
            "do write memory",
            "do copy running-config startup-config",
            "  COPY   RUNNING-CONFIG   STARTUP-CONFIG  ",
        ] {
            assert!(
                check_command(command).is_err(),
                "{command:?} must be blocked"
            );
        }
    }

    #[test]
    fn blocks_any_copy_to_startup_or_nvram() {
        for command in [
            "copy flash:backup.cfg startup-config",
            "copy tftp://192.0.2.1/config startup-config",
            "copy startup-config nvram:",
        ] {
            assert!(check_command(command).is_err(), "{command:?}");
        }
    }

    #[test]
    fn blocks_startup_erase_and_delete() {
        for command in [
            "erase startup-config",
            "erase nvram:",
            "write erase",
            "delete nvram:startup-config",
            "delete startup-config",
        ] {
            assert!(check_command(command).is_err(), "{command:?}");
        }
    }

    #[test]
    fn blocks_ambiguous_running_copy() {
        assert_eq!(blocked("copy running-config"), "ambiguous-copy");
    }

    #[test]
    fn allows_normal_running_config_edits_and_show_commands() {
        for command in [
            "show running-config",
            "show startup-config",
            "show version",
            "hostname Router-01",
            "vlan 10",
            "interface GigabitEthernet0/0/1",
            "ip route 0.0.0.0 0.0.0.0 192.0.2.1",
            "username admin secret 5 $1$abc$def",
            "copy running-config tftp://192.0.2.10/backup.cfg",
            "copy startup-config running-config",
            "write terminal",
            "configure terminal",
            "end",
            "reload",
            "terminal length 0",
        ] {
            assert!(
                check_command(command).is_ok(),
                "{command:?} should be allowed: {:?}",
                check_command(command)
            );
        }
    }

    #[test]
    fn blocks_injected_multi_line_commands() {
        assert!(check_command("show version\nwrite memory").is_err());
        assert!(check_command("show version;copy run start").is_err());
    }

    #[test]
    fn rule_identifiers_are_stable() {
        assert_eq!(blocked("write memory"), "write-memory");
        assert_eq!(
            blocked("copy running-config startup-config"),
            "copy-to-startup"
        );
        assert_eq!(blocked("erase startup-config"), "erase-startup");
        assert_eq!(blocked("write"), "bare-write");
    }
}
