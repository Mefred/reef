//! Parser for `show vlan brief`.

use super::{ParseResult, ParseWarning};
use crate::model::VlanConfig;

pub fn parse_vlans(output: &str) -> ParseResult<Vec<VlanConfig>> {
    let mut vlans = Vec::new();
    let mut warnings = Vec::new();

    for (i, line) in output.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("VLAN") || trimmed.starts_with("---") {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        let id: u16 = match parts[0].parse() {
            Ok(v) => v,
            Err(_) => {
                warnings.push(ParseWarning::new(
                    i + 1,
                    trimmed.to_string(),
                    "VLAN id not numeric".to_string(),
                ));
                continue;
            }
        };
        let name = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
        vlans.push(VlanConfig {
            id,
            name,
            enabled: true,
        });
    }

    ParseResult::new(vlans).with_warnings(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vlan_table() {
        let out = "VLAN Name                             Status    Ports\n1    default                         active    Gi1/0/1\n10   Sales                           active    Gi1/0/2\n";
        let r = parse_vlans(out);
        assert_eq!(r.model.len(), 2);
        assert_eq!(r.model[0].id, 1);
        assert_eq!(r.model[1].name, "Sales");
    }
}
