//! Parsers for `show version` and `show inventory`.

use super::{ParseError, ParseResult, ParseWarning};
use crate::model::{DeviceCapabilities, DeviceIdentity, InterfaceNaming, InterfaceRef};

/// Parse `show version` output into a `DeviceIdentity`.
///
/// This is intentionally tolerant: fields that cannot be found are left blank
/// rather than failing, because output varies across IOS XE releases and
/// platforms. Callers should inspect the returned `warnings` for anything
/// unexpected.
pub fn parse_version(output: &str) -> ParseResult<DeviceIdentity> {
    let mut identity = DeviceIdentity {
        hostname: String::new(),
        iosxe_version: String::new(),
        platform: String::new(),
        chassis_serial: String::new(),
        processor: String::new(),
        interfaces: Vec::new(),
        capabilities: DeviceCapabilities::default(),
    };
    let mut warnings = Vec::new();

    for (i, line) in output.lines().enumerate() {
        let lineno = i + 1;
        let trimmed = line.trim();

        if trimmed.starts_with("Cisco IOS XE Software") || trimmed.contains("IOS XE Software") {
            if identity.iosxe_version.is_empty() {
                identity.iosxe_version = extract_version(trimmed);
            }
        }
        if trimmed.contains("Cisco ") && trimmed.contains("(") && platform_is_cpu(trimmed) {
            if identity.processor.is_empty() {
                identity.processor = trimmed.to_string();
            }
        }
        if trimmed.contains("chassis") || trimmed.contains("Chassis") {
            // Serial often appears on a separate `show inventory` line; best-effort here.
        }
    }

    // Hostname is best taken from the CLI prompt rather than show version; leave
    // blank if unknown and let the caller fill it from the detected prompt.
    if identity.hostname.is_empty() {
        warnings.push(ParseWarning::new(
            0,
            String::new(),
            "hostname not found in show version; use prompt or show running-config hostname".to_string(),
        ));
    }

    ParseResult::new(identity).with_warnings(warnings)
}

fn extract_version(line: &str) -> String {
    // Look for patterns like "Version 17.12.0a" or "17.12.0a".
    // Keep trailing release suffixes (e.g. "a", "b", "r").
    let mut out = String::new();
    for token in line.split_whitespace() {
        let t = token.trim_matches(|c: char| !(c.is_ascii_digit() || c == '.' || c.is_ascii_alphabetic()));
        if !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == '.' || c.is_ascii_alphabetic()) {
            // Require at least one digit to avoid matching pure words
            if t.chars().any(|c| c.is_ascii_digit()) {
                out = t.to_string();
                break;
            }
        }
    }
    out
}

/// Heuristic: a line like "Cisco IOS XE Software, Version ..." often also
/// names the CPU/platform in parentheses.
fn platform_is_cpu(line: &str) -> bool {
    line.contains("processor") || line.contains("CPU") || line.contains("Cisco ")
}

/// Merge `show inventory` fields into an existing `DeviceIdentity`.
pub fn merge_inventory(identity: &mut DeviceIdentity, output: &str) {
    for (_i, line) in output.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.contains("PID:") {
            // Platform/PID often on the same line as NAME or DESCR.
            let pid = extract_after(trimmed, "PID:");
            if !pid.is_empty() && identity.platform.is_empty() {
                identity.platform = pid;
            }
        }
        if trimmed.contains("SN:") {
            let sn = extract_after(trimmed, "SN:");
            if !sn.is_empty() && identity.chassis_serial.is_empty() {
                identity.chassis_serial = sn;
            }
        }
        if trimmed.contains("NAME:") {
            // Best-effort model hint
            if identity.platform.is_empty() {
                identity.platform = extract_after(trimmed, "NAME:").trim().to_string();
            }
        }
    }
}

fn extract_after(line: &str, key: &str) -> String {
    line.split(key)
        .nth(1)
        .map(|s| s.trim().split_whitespace().next().unwrap_or("").to_string())
        .unwrap_or_default()
}

/// Parse operational interface state from `show ip interface brief`.
pub fn parse_interfaces(output: &str) -> ParseResult<Vec<InterfaceRef>> {
    let mut interfaces = Vec::new();
    let mut warnings = Vec::new();

    for (i, line) in output.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("Interface")
            || trimmed.starts_with("---")
        {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].to_string();
        // Heuristic: skip non-interface lines (e.g. routing table headers)
        if !is_interface_name(&name) {
            warnings.push(ParseWarning::new(
                i + 1,
                trimmed.to_string(),
                "non-interface line skipped".to_string(),
            ));
            continue;
        }
        interfaces.push(InterfaceRef {
            name,
            oper_status: parts.get(4).map(|s| s.to_string()),
            ipv4: None,
            ipv6: None,
        });
    }

    ParseResult::new(interfaces).with_warnings(warnings)
}

fn is_interface_name(name: &str) -> bool {
    name.starts_with("GigabitEthernet")
        || name.starts_with("TenGigabitEthernet")
        || name.starts_with("FastEthernet")
        || name.starts_with("Vlan")
        || name.starts_with("Loopback")
        || name.starts_with("Port-channel")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_extracts_version() {
        let out = "Cisco IOS XE Software, Version 17.12.0a\n";
        let r = parse_version(out);
        assert_eq!(r.model.iosxe_version, "17.12.0a");
    }

    #[test]
    fn parse_interfaces_skips_header() {
        let out = "Interface              IP-Address      OK? Method Status                Protocol\nGigabitEthernet0/0/0   10.0.0.1        YES NVRAM  up                    up\n";
        let r = parse_interfaces(out);
        assert_eq!(r.model.len(), 1);
        assert_eq!(r.model[0].name, "GigabitEthernet0/0/0");
    }

    #[test]
    fn extract_after_handles_missing_key() {
        assert_eq!(extract_after("no key here", "PID:"), "");
    }
}
