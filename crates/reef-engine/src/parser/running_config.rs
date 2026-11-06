//! Parser for `show running-config`.
//!
//! Converts the full running configuration into a structured `DeviceConfig`.
//! This is order-tolerant: unknown lines are preserved as raw snippets.

use super::{ParseResult, ParseWarning};
use crate::model::{DeviceConfig, InterfaceConfig, InterfaceMode, VlanConfig};

pub fn parse_running_config(output: &str) -> ParseResult<DeviceConfig> {
    let mut config = DeviceConfig::default();
    let mut warnings = Vec::new();

    let mut current_interface: Option<String> = None;

    for (i, line) in output.lines().enumerate() {
        let trimmed = line.trim();

        if trimmed.starts_with("hostname ") {
            config.hostname = Some(trimmed["hostname ".len()..].trim().to_string());
            continue;
        }
        if trimmed.starts_with("ip domain-name ") {
            config.ip_domain_name = Some(trimmed["ip domain-name ".len()..].trim().to_string());
            continue;
        }
        if trimmed.starts_with("interface ") {
            let name = trimmed["interface ".len()..].trim().to_string();
            current_interface = Some(name.clone());
            config.interfaces.insert(
                name,
                InterfaceConfig {
                    enabled: true,
                    ..Default::default()
                },
            );
            continue;
        }
        if let Some(ref iface_name) = current_interface {
            if trimmed.starts_with("description ") {
                if let Some(iface) = config.interfaces.get_mut(iface_name) {
                    iface.description = Some(trimmed["description ".len()..].trim().to_string());
                }
                continue;
            }
            if trimmed == "shutdown" {
                if let Some(iface) = config.interfaces.get_mut(iface_name) {
                    iface.enabled = false;
                }
                continue;
            }
            if trimmed == "no shutdown" {
                if let Some(iface) = config.interfaces.get_mut(iface_name) {
                    iface.enabled = true;
                }
                continue;
            }
            if trimmed.starts_with("ip address ") {
                if let Some(iface) = config.interfaces.get_mut(iface_name) {
                    let rest = trimmed["ip address ".len()..].trim();
                    if let Some(addr) = parse_ipv4_address(rest) {
                        iface.ipv4 = Some(addr);
                    } else {
                        warnings.push(ParseWarning::new(
                            i + 1,
                            trimmed.to_string(),
                            "unparsed ip address".to_string(),
                        ));
                    }
                }
                continue;
            }
            if trimmed.starts_with("switchport access vlan ") {
                if let Some(iface) = config.interfaces.get_mut(iface_name) {
                    if let Ok(vlan) = trimmed["switchport access vlan ".len()..].trim().parse::<u16>() {
                        iface.access_vlan = Some(vlan);
                        iface.mode = InterfaceMode::Access;
                    }
                }
                continue;
            }
        }

        if trimmed.starts_with("vlan ") {
            // Best-effort VLAN capture; detailed VLAN parsing handled by parse_vlans.
            if let Ok(id) = trimmed["vlan ".len()..].trim().parse::<u16>() {
                config.vlans.insert(id, VlanConfig { id, name: String::new(), enabled: true });
            }
            continue;
        }

        if !trimmed.is_empty() && !trimmed.starts_with("!") && !trimmed.starts_with("Current configuration") {
            warnings.push(ParseWarning::new(
                i + 1,
                trimmed.to_string(),
                "unhandled running-config line".to_string(),
            ));
        }
    }

    ParseResult::new(config).with_warnings(warnings)
}

fn parse_ipv4_address(rest: &str) -> Option<crate::model::Ipv4Address> {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() < 2 {
        return None;
    }
    let addr: std::net::Ipv4Addr = parts[0].parse().ok()?;
    let mask: std::net::Ipv4Addr = parts[1].parse().ok()?;
    let prefix_len = mask_to_prefix_len(mask)?;
    Some(crate::model::Ipv4Address { address: addr, prefix_len })
}

fn mask_to_prefix_len(mask: std::net::Ipv4Addr) -> Option<u8> {
    let bits = u32::from_be_bytes(mask.octets());
    if bits == 0 {
        return Some(0);
    }
    let trailing = bits.trailing_zeros();
    // Validate contiguous mask: all 1s must be on the left
    if bits == (!0_u32 << trailing) {
        Some((32 - trailing) as u8)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hostname_and_interface() {
        let out = "hostname Router1\ninterface GigabitEthernet0/0/0\n description Uplink\n ip address 10.0.0.1 255.255.255.0\n no shutdown\n";
        let r = parse_running_config(out);
        assert_eq!(r.model.hostname.as_deref(), Some("Router1"));
        let iface = r.model.interfaces.get("GigabitEthernet0/0/0").unwrap();
        assert!(iface.enabled);
        assert_eq!(iface.description.as_deref(), Some("Uplink"));
        assert!(iface.ipv4.is_some());
        assert_eq!(iface.ipv4.as_ref().unwrap().address.to_string(), "10.0.0.1");
    }
}
