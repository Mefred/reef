//! Parser for `show ip interface brief` / `show ipv6 interface brief`.

use super::{ParseResult, ParseWarning};
use crate::model::InterfaceRef;

pub fn parse_interfaces(output: &str) -> ParseResult<Vec<InterfaceRef>> {
    let mut interfaces = Vec::new();
    let mut warnings = Vec::new();

    for (i, line) in output.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("Interface") || trimmed.starts_with("---") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let name = parts[0].to_string();
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
    fn skips_header_and_returns_interface() {
        let out = "Interface              IP-Address      OK? Method Status                Protocol\nGigabitEthernet0/0/0   10.0.0.1        YES NVRAM  up                    up\n";
        let r = parse_interfaces(out);
        assert_eq!(r.model.len(), 1);
        assert_eq!(r.model[0].name, "GigabitEthernet0/0/0");
    }
}
