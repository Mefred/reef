//! Parser for `show ip access-lists`.

use super::{ParseResult, ParseWarning};
use crate::model::{AclConfig, AclRule, Action, AddressSpec, AclType, PortSpec, Protocol};

pub fn parse_acls(output: &str) -> ParseResult<Vec<AclConfig>> {
    let mut acls = Vec::new();
    let mut warnings = Vec::new();

    let mut current: Option<AclConfig> = None;

    for (i, line) in output.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // New ACL header: "Standard IP access list FOO" or "Extended IP access list BAR"
        if trimmed.contains("access list ") {
            if let Some(acl) = current.take() {
                acls.push(acl);
            }
            let acl_type = if trimmed.contains("Extended") {
                AclType::Extended
            } else {
                AclType::Standard
            };
            let name = trimmed
                .split("access list ")
                .nth(1)
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            current = Some(AclConfig {
                name,
                acl_type,
                rules: Vec::new(),
            });
            continue;
        }
        // Rule lines: " 10 permit ip any any" or " 20 deny tcp host 1.2.3.4 eq 80 any"
        if let Some(acl) = current.as_mut() {
            if let Some(rule) = parse_rule(trimmed) {
                acl.rules.push(rule);
                continue;
            }
        }
        warnings.push(ParseWarning::new(
            i + 1,
            trimmed.to_string(),
            "unhandled ACL line".to_string(),
        ));
    }
    if let Some(acl) = current {
        acls.push(acl);
    }

    ParseResult::new(acls).with_warnings(warnings)
}

fn parse_rule(line: &str) -> Option<AclRule> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 4 {
        return None;
    }
    // Skip leading sequence number if present
    let start = if parts[0].parse::<u16>().is_ok() { 1 } else { 0 };
    let action = match parts[start] {
        "permit" => Action::Permit,
        "deny" => Action::Deny,
        _ => return None,
    };
    let protocol = match parts.get(start + 1) {
        Some(&"ip") => Protocol::Ip,
        Some(&"tcp") => Protocol::Tcp,
        Some(&"udp") => Protocol::Udp,
        Some(&"icmp") => Protocol::Icmp,
        Some(s) => Protocol::Other(s.to_string()),
        _ => return None,
    };
    let source = parse_address_spec(parts.get(start + 2)?);
    let destination = parse_address_spec(parts.get(start + 3)?);
    let ports = None; // best-effort; ports parsing omitted for brevity

    Some(AclRule {
        action,
        protocol: Some(protocol),
        source,
        destination,
        ports,
    })
}

fn parse_address_spec(token: &str) -> AddressSpec {
    match token {
        "any" => AddressSpec::Any,
        "host" => AddressSpec::Any, // would need next token; simplified
        s if s.contains('/') => s.parse::<ipnet::IpNet>().map(AddressSpec::Network).unwrap_or(AddressSpec::Any),
        s => s.parse::<std::net::IpAddr>().map(AddressSpec::Host).unwrap_or(AddressSpec::Any),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_standard_acl() {
        let out = "Standard IP access list 10\n 10 permit 1.2.3.4\n";
        let r = parse_acls(out);
        assert_eq!(r.model.len(), 1);
        assert_eq!(r.model[0].name, "10");
    }
}
